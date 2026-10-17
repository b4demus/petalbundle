//! Doubler voices: each one is a short delay with a small pitch shift
//! (dual-tap rotating delay with crossfades), slow random modulation,
//! feedback and a band-limiting filter.

use std::f32::consts::{PI, SQRT_2, TAU};

pub const VOICES: usize = 4;
pub const MAX_DELAY_MS: f32 = 80.0;
/// Pitch-shifter window. Voice delays shorter than half of it are lifted to it.
pub const WINDOW_MS: f32 = 24.0;
/// Modulation reach at 100 % depth.
const MOD_DELAY_MS: f32 = 3.0;

#[derive(Clone, Copy, Debug)]
pub struct VoiceSettings {
    pub delay_ms: f32,
    pub cents: f32,
    /// Linear gain.
    pub gain: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Shared {
    pub mod_rate_hz: f32,
    /// 0..1.
    pub mod_depth: f32,
    pub feedback: f32,
    pub low_cut_hz: f32,
    pub high_cut_hz: f32,
}

/// Minimum effective delay so the pitch-shift taps never reach the write head.
pub fn effective_delay_ms(delay_ms: f32) -> f32 {
    delay_ms.max(WINDOW_MS * 0.5 + 1.0)
}

/// Zero-delay-feedback SVF giving low-pass and high-pass outputs.
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    #[inline]
    fn run(&mut self, x: f32, g: f32) -> (f32, f32) {
        let k = SQRT_2;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        let lp = v2;
        let hp = x - k * v1 - v2;
        (lp, hp)
    }
}

pub struct Voice {
    sr: f32,
    buf: Vec<f32>,
    mask: usize,
    write: usize,
    /// 0..1 position of the first pitch-shift tap inside the window.
    phase: f32,
    /// Smoothed random modulation in -1..1.
    wobble: f32,
    wobble_target: f32,
    wobble_timer: f32,
    rng: u32,
    last: f32,
    lowcut: Svf,
    highcut: Svf,
    /// Smoothed delay centre (samples) so moving a voice glides.
    center: f32,
}

impl Voice {
    pub fn new(sr: f32, seed: u32) -> Self {
        let max = ((MAX_DELAY_MS + WINDOW_MS + MOD_DELAY_MS + 4.0) * 0.001 * sr) as usize;
        let len = max.next_power_of_two();
        Self {
            sr,
            buf: vec![0.0; len],
            mask: len - 1,
            write: 0,
            phase: 0.5,
            wobble: 0.0,
            wobble_target: 0.0,
            wobble_timer: 0.0,
            rng: seed.max(1),
            last: 0.0,
            lowcut: Svf::default(),
            highcut: Svf::default(),
            center: -1.0,
        }
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.last = 0.0;
        self.lowcut = Svf::default();
        self.highcut = Svf::default();
        self.phase = 0.5;
    }

    fn read(&self, delay: f32) -> f32 {
        let pos = self.write as f32 - delay;
        let i = pos.floor();
        let t = pos - i;
        let i = i as isize as usize;
        let a = self.buf[i & self.mask];
        let b = self.buf[(i + 1) & self.mask];
        a + (b - a) * t
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    /// Current modulation value (-1..1), for the editor.
    pub fn wobble(&self) -> f32 {
        self.wobble
    }

    #[inline]
    pub fn process(&mut self, x: f32, v: &VoiceSettings, s: &Shared) -> f32 {
        let sr = self.sr;
        // Slow random walk: new target every 1/rate seconds, glided towards.
        self.wobble_timer -= s.mod_rate_hz.max(0.01) / sr;
        if self.wobble_timer <= 0.0 {
            self.wobble_timer += 1.0;
            self.wobble_target = self.rand();
        }
        let glide = 1.0 - (-TAU * s.mod_rate_hz.max(0.01) / sr).exp();
        self.wobble += (self.wobble_target - self.wobble) * glide;

        let target = effective_delay_ms(v.delay_ms) * 0.001 * sr;
        self.center = if self.center < 0.0 { target } else { self.center + (target - self.center) * (1.0 / (0.05 * sr)) };
        let window = WINDOW_MS * 0.001 * sr;
        let modulation = self.wobble * s.mod_depth * MOD_DELAY_MS * 0.001 * sr;

        // Rotating taps: a delay that changes by (1 - ratio) per sample plays
        // back at `ratio`; two taps half a window apart are crossfaded with
        // sin^2 windows that sum to one.
        let ratio = 2f32.powf(v.cents / 1200.0);
        if v.cents.abs() > 1e-3 {
            self.phase = (self.phase + (1.0 - ratio) / window).rem_euclid(1.0);
        }
        let p1 = self.phase;
        let p2 = (self.phase + 0.5) % 1.0;
        let base = self.center + modulation - window * 0.5;
        let y = (PI * p1).sin().powi(2) * self.read(base + p1 * window)
            + (PI * p2).sin().powi(2) * self.read(base + p2 * window);

        let g_lo = (PI * s.low_cut_hz.clamp(10.0, sr * 0.45) / sr).tan();
        let g_hi = (PI * s.high_cut_hz.clamp(100.0, sr * 0.45) / sr).tan();
        let (_, y) = self.lowcut.run(y, g_lo);
        let (y, _) = self.highcut.run(y, g_hi);

        // Feedback with a soft clip so high settings cannot run away.
        self.buf[self.write & self.mask] = x + (self.last * s.feedback).tanh();
        self.write = self.write.wrapping_add(1);
        self.last = y;
        y * v.gain
    }
}

/// Equal-power pan gains for -1..1.
pub fn pan_gains(pan: f32) -> [f32; 2] {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    [a.cos() * SQRT_2, a.sin() * SQRT_2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Shared {
        Shared { mod_rate_hz: 1.0, mod_depth: 0.0, feedback: 0.0, low_cut_hz: 10.0, high_cut_hz: 22_000.0 }
    }

    #[test]
    fn unshifted_voice_arrives_at_its_delay() {
        let sr = 48_000.0;
        let mut v = Voice::new(sr, 1);
        let vs = VoiceSettings { delay_ms: 20.0, cents: 0.0, gain: 1.0 };
        let out: Vec<f32> = (0..4000).map(|n| v.process(if n == 0 { 1.0 } else { 0.0 }, &vs, &shared())).collect();
        let peak = out.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert!((peak as f32 - 0.020 * sr).abs() <= 2.0, "peak at {peak}");
    }

    #[test]
    fn pitch_shift_moves_frequency_by_the_cents() {
        let sr = 48_000.0;
        for cents in [-30.0f32, 25.0, 50.0] {
            let mut v = Voice::new(sr, 7);
            let vs = VoiceSettings { delay_ms: 20.0, cents, gain: 1.0 };
            let n = sr as usize * 2;
            let out: Vec<f32> = (0..n)
                .map(|i| v.process((TAU * 1000.0 * i as f32 / sr).sin(), &vs, &shared()))
                .collect();
            // Count rising zero crossings over the second half.
            let half = &out[n / 2..];
            let crossings = half.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32;
            let freq = crossings / (half.len() as f32 / sr);
            let expected = 1000.0 * 2f32.powf(cents / 1200.0);
            assert!((freq - expected).abs() < 3.0, "{cents} c: {freq} vs {expected}");
        }
    }

    #[test]
    fn feedback_stays_bounded() {
        let sr = 48_000.0;
        let mut v = Voice::new(sr, 3);
        let vs = VoiceSettings { delay_ms: 15.0, cents: 10.0, gain: 1.0 };
        let s = Shared { feedback: 0.95, ..shared() };
        for n in 0..(sr as usize * 3) {
            let y = v.process(if n < 100 { 1.0 } else { 0.0 }, &vs, &s);
            assert!(y.is_finite() && y.abs() < 4.0);
        }
    }
}
