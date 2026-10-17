//! Multi-tap modulated delay (chorus / ensemble / flanger / vibrato / "sky"):
//! up to eight taps read one delay line, each with its own LFO; the summed
//! taps feed back through optional diffusion and saturation.

use std::f32::consts::{PI, SQRT_2, TAU};

pub const MAX_TAPS: usize = 8;
pub const MAX_DELAY_MS: f32 = 1000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Sine,
    Triangle,
    Wander,
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub delay_ms: f32,
    pub taps: usize,
    /// 0..1: how far the taps spread around the delay time.
    pub spread: f32,
    pub rate_hz: f32,
    /// 0..1.
    pub depth: f32,
    pub shape: Shape,
    /// -0.95..0.95.
    pub feedback: f32,
    /// 0..1.
    pub diffusion: f32,
    /// 0..1: saturation in the feedback loop.
    pub warp: f32,
    pub low_cut_hz: f32,
    pub high_cut_hz: f32,
    /// 0..1.
    pub width: f32,
}

/// Modulation reach in ms: proportional to the delay, so short flanger
/// delays sweep a few ms and long ones wobble without going seasick.
pub fn depth_ms(delay_ms: f32, depth: f32) -> f32 {
    depth.clamp(0.0, 1.0) * (2.0 + delay_ms * 0.5).min(12.0)
}

/// Resting delay of tap `k` of `n` (ms), before modulation.
pub fn tap_base_ms(s: &Settings, k: usize) -> f32 {
    let n = s.taps.clamp(1, MAX_TAPS);
    let u = if n == 1 { 0.5 } else { k as f32 / (n - 1) as f32 };
    let base = s.delay_ms * (1.0 + s.spread.clamp(0.0, 1.0) * (u - 0.5) * 1.6);
    // Never let the modulation reach the write head.
    base.max(depth_ms(s.delay_ms, s.depth) + 0.3)
}

/// Pan of tap `k` (-1..1): alternating sides, widening outwards.
pub fn tap_pan(k: usize, n: usize, width: f32) -> f32 {
    if n <= 1 {
        return 0.0;
    }
    let side = if k % 2 == 0 { -1.0 } else { 1.0 };
    let reach = 0.4 + 0.6 * (k as f32 / (n - 1) as f32);
    side * reach * width.clamp(0.0, 1.0)
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], pos: 0 }
    }

    #[inline]
    fn run(&mut self, x: f32, g: f32) -> f32 {
        let d = self.buf[self.pos];
        let v = x - g * d;
        self.buf[self.pos] = v;
        self.pos = (self.pos + 1) % self.buf.len();
        d + g * v
    }
}

#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// Returns (low-pass, high-pass).
    #[inline]
    fn run(&mut self, x: f32, g: f32) -> (f32, f32) {
        let a1 = 1.0 / (1.0 + g * (g + SQRT_2));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, x - SQRT_2 * v1 - v2)
    }
}

pub struct ModDelay {
    sr: f32,
    buf: Vec<f32>,
    mask: usize,
    write: usize,
    phase: f32,
    wander: [f32; MAX_TAPS],
    wander_target: [f32; MAX_TAPS],
    wander_timer: [f32; MAX_TAPS],
    rng: Rng,
    diffusers: [Allpass; 4],
    fb: f32,
    loop_hp: Svf,
    out_lp: [Svf; 2],
    out_hp: [Svf; 2],
    /// Smoothed resting delays (samples): changing Delay or Spread glides.
    base: [f32; MAX_TAPS],
    /// Current modulated delays (ms), for the editor.
    pub current_ms: [f32; MAX_TAPS],
}

impl ModDelay {
    pub fn new(sr: f32) -> Self {
        let max = ((MAX_DELAY_MS * 1.9 + 20.0) * 0.001 * sr) as usize;
        let len = max.next_power_of_two();
        let ms = |m: f32| (m * 0.001 * sr) as usize;
        Self {
            sr,
            buf: vec![0.0; len],
            mask: len - 1,
            write: 0,
            phase: 0.0,
            wander: [0.0; MAX_TAPS],
            wander_target: [0.0; MAX_TAPS],
            wander_timer: std::array::from_fn(|k| k as f32 / MAX_TAPS as f32),
            rng: Rng(0x7A11_C0DE),
            diffusers: [Allpass::new(ms(4.3)), Allpass::new(ms(7.9)), Allpass::new(ms(11.3)), Allpass::new(ms(16.7))],
            fb: 0.0,
            loop_hp: Svf::default(),
            out_lp: Default::default(),
            out_hp: Default::default(),
            base: [-1.0; MAX_TAPS],
            current_ms: [0.0; MAX_TAPS],
        }
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.fb = 0.0;
        for d in &mut self.diffusers {
            d.buf.fill(0.0);
        }
    }

    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let pos = self.write as f32 - delay;
        let i = pos.floor();
        let t = pos - i;
        let i = i as isize as usize;
        let s = |k: usize| self.buf[i.wrapping_add(k).wrapping_sub(1) & self.mask];
        let (y0, y1, y2, y3) = (s(0), s(1), s(2), s(3));
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * t + c2) * t + c1) * t + y1
    }

    fn lfo(&mut self, k: usize, n: usize, s: &Settings) -> f32 {
        let offset = k as f32 / n as f32;
        let p = (self.phase + offset) % 1.0;
        match s.shape {
            Shape::Sine => (TAU * p).sin(),
            Shape::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
            Shape::Wander => {
                self.wander_timer[k] -= s.rate_hz.max(0.01) / self.sr;
                if self.wander_timer[k] <= 0.0 {
                    self.wander_timer[k] += 1.0;
                    self.wander_target[k] = self.rng.next();
                }
                let glide = 1.0 - (-TAU * s.rate_hz.max(0.01) / self.sr).exp();
                self.wander[k] += (self.wander_target[k] - self.wander[k]) * glide;
                self.wander[k]
            }
        }
    }

    #[inline]
    pub fn process(&mut self, input: [f32; 2], s: &Settings) -> [f32; 2] {
        let sr = self.sr;
        let n = s.taps.clamp(1, MAX_TAPS);
        self.phase = (self.phase + s.rate_hz / sr) % 1.0;

        // Write: input plus the (diffused, saturated, high-passed) feedback.
        let mono = (input[0] + input[1]) * 0.5;
        let mut fb = self.fb * s.feedback.clamp(-0.95, 0.95);
        let g = 0.7 * s.diffusion.clamp(0.0, 1.0);
        if g > 0.0 {
            for d in &mut self.diffusers {
                fb = d.run(fb, g);
            }
        }
        if s.warp > 0.0 {
            let drive = 1.0 + 4.0 * s.warp;
            fb = fb + ((fb * drive).tanh() / drive.tanh() - fb) * s.warp;
        }
        let (_, fb) = self.loop_hp.run(fb, (PI * 30.0 / sr).tan());
        self.buf[self.write & self.mask] = mono + fb;
        self.write = self.write.wrapping_add(1);

        // Read the taps.
        let reach = depth_ms(s.delay_ms, s.depth) * 0.001 * sr;
        let gain = 1.0 / (n as f32).sqrt();
        let mut out = [0.0f32; 2];
        let mut sum = 0.0;
        for k in 0..n {
            let target = tap_base_ms(s, k) * 0.001 * sr;
            self.base[k] = if self.base[k] < 0.0 { target } else { self.base[k] + (target - self.base[k]) * (1.0 / (0.06 * sr)) };
            let m = self.lfo(k, n, s);
            let delay = (self.base[k] + m * reach).clamp(1.0, (self.mask - 4) as f32);
            self.current_ms[k] = delay / sr * 1000.0;
            let y = self.read(delay) * gain;
            sum += y;
            let a = (tap_pan(k, n, s.width) + 1.0) * std::f32::consts::FRAC_PI_4;
            out[0] += y * a.cos() * SQRT_2;
            out[1] += y * a.sin() * SQRT_2;
        }
        self.fb = sum * gain.recip() / n as f32;

        // Tone of the wet signal.
        let g_lo = (PI * s.low_cut_hz.clamp(10.0, sr * 0.45) / sr).tan();
        let g_hi = (PI * s.high_cut_hz.clamp(200.0, sr * 0.45) / sr).tan();
        for ch in 0..2 {
            let (_, hp) = self.out_hp[ch].run(out[ch], g_lo);
            let (lp, _) = self.out_lp[ch].run(hp, g_hi);
            out[ch] = lp;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            delay_ms: 20.0,
            taps: 1,
            spread: 0.0,
            rate_hz: 0.5,
            depth: 0.0,
            shape: Shape::Sine,
            feedback: 0.0,
            diffusion: 0.0,
            warp: 0.0,
            low_cut_hz: 10.0,
            high_cut_hz: 22_000.0,
            width: 1.0,
        }
    }

    fn impulse(s: &Settings, len: usize) -> Vec<f32> {
        let mut d = ModDelay::new(48_000.0);
        (0..len)
            .map(|n| {
                let y = d.process(if n == 0 { [1.0, 1.0] } else { [0.0, 0.0] }, s);
                y[0] + y[1]
            })
            .collect()
    }

    fn peaks(v: &[f32], threshold: f32) -> Vec<usize> {
        (1..v.len() - 1).filter(|&i| v[i].abs() > threshold && v[i].abs() >= v[i - 1].abs() && v[i].abs() >= v[i + 1].abs()).collect()
    }

    #[test]
    fn taps_land_where_expected() {
        let sr = 48_000.0;
        let s = Settings { taps: 3, spread: 0.5, ..settings() };
        let ir = impulse(&s, 4000);
        let found = peaks(&ir, 0.1);
        let expected: Vec<usize> = (0..3).map(|k| (tap_base_ms(&s, k) * 0.001 * sr).round() as usize).collect();
        assert_eq!(found.len(), 3, "peaks at {found:?}");
        for (f, e) in found.iter().zip(&expected) {
            assert!((*f as i64 - *e as i64).abs() <= 1, "found {found:?}, expected {expected:?}");
        }
    }

    #[test]
    fn modulation_moves_the_taps() {
        let sr = 48_000.0;
        let mut d = ModDelay::new(sr);
        let s = Settings { depth: 1.0, rate_hz: 2.0, ..settings() };
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for _ in 0..(sr as usize) {
            d.process([0.0, 0.0], &s);
            lo = lo.min(d.current_ms[0]);
            hi = hi.max(d.current_ms[0]);
        }
        let swing = hi - lo;
        let want = 2.0 * depth_ms(20.0, 1.0);
        assert!((swing - want).abs() < 0.2, "swing {swing:.2} ms, want {want:.2}");
    }

    #[test]
    fn negative_feedback_flips_the_repeats() {
        let s = Settings { feedback: -0.5, ..settings() };
        let ir = impulse(&s, 3000);
        let at = |ms: f32| ir[(ms * 48.0) as usize - 3..(ms * 48.0) as usize + 4].iter().sum::<f32>();
        assert!(at(20.0) > 0.5);
        assert!(at(40.0) < -0.2, "second repeat {}", at(40.0));
    }

    #[test]
    fn heavy_settings_stay_bounded() {
        let mut d = ModDelay::new(48_000.0);
        let s = Settings { taps: 8, spread: 1.0, depth: 1.0, rate_hz: 3.0, shape: Shape::Wander, feedback: 0.95, diffusion: 1.0, warp: 1.0, delay_ms: 300.0, ..settings() };
        for n in 0..96_000 {
            let x = if n % 12_000 < 200 { 0.9 } else { 0.0 };
            let y = d.process([x, x], &s);
            assert!(y[0].is_finite() && y[0].abs() < 6.0);
        }
    }
}
