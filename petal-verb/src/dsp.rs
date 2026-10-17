//! Algorithmic stereo reverb: pre-delay, input diffusion (all-pass chains)
//! and an 8-line feedback delay network with a Hadamard mixing matrix,
//! per-line damping and slow delay modulation.

use std::f32::consts::TAU;

const LINES: usize = 8;
/// Base FDN line lengths at size = 1 (ms), mutually prime-ish.
const LINE_MS: [f32; LINES] = [31.7, 37.1, 41.3, 43.9, 47.3, 53.1, 59.9, 67.7];
/// Size maps to this range of line-length scale.
const SCALE_MIN: f32 = 0.25;
const SCALE_MAX: f32 = 2.0;
/// Input diffuser lengths (ms) per channel.
const DIFFUSER_MS: [[f32; 4]; 2] = [[4.71, 3.59, 12.73, 9.31], [5.13, 3.17, 11.89, 8.77]];
const DIFFUSION: f32 = 0.7;
pub const MAX_PREDELAY_MS: f32 = 500.0;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub predelay_ms: f32,
    /// 0..1.
    pub size: f32,
    /// RT60 in seconds.
    pub decay_s: f32,
    /// 0..1.
    pub modulation: f32,
    pub damp_hz: f32,
    pub low_cut_hz: f32,
    /// 0 = mono, 1 = full stereo.
    pub width: f32,
    pub freeze: bool,
}

struct Line {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
}

impl Line {
    fn new(max_samples: usize) -> Self {
        let len = (max_samples + 4).next_power_of_two();
        Self { buf: vec![0.0; len], mask: len - 1, write: 0 }
    }

    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let pos = self.write as f32 - delay.clamp(1.0, (self.mask - 2) as f32);
        let i = pos.floor();
        let t = pos - i;
        let i = i as isize as usize;
        let a = self.buf[i & self.mask];
        let b = self.buf[(i + 1) & self.mask];
        a + (b - a) * t
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.write & self.mask] = x;
        self.write = self.write.wrapping_add(1);
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// Schroeder all-pass with an integer delay.
struct Allpass {
    line: Line,
    delay: f32,
}

impl Allpass {
    #[inline]
    fn run(&mut self, x: f32, g: f32) -> f32 {
        let d = self.line.read(self.delay);
        let v = x - g * d;
        self.line.push(v);
        d + g * v
    }
}

/// In-place fast Walsh-Hadamard transform, normalized (orthogonal, lossless).
#[inline]
fn hadamard(x: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        for i in (0..LINES).step_by(h * 2) {
            for j in i..i + h {
                let (a, b) = (x[j], x[j + h]);
                x[j] = a + b;
                x[j + h] = a - b;
            }
        }
        h *= 2;
    }
    let norm = 1.0 / (LINES as f32).sqrt();
    for v in x.iter_mut() {
        *v *= norm;
    }
}

pub struct Reverb {
    sr: f32,
    predelay: [Line; 2],
    diffusers: [[Allpass; 4]; 2],
    lines: [Line; LINES],
    damp: [f32; LINES],
    /// Two cascaded one-pole high-passes per channel on the wet output.
    hp: [[(f32, f32); 2]; 2],
    lfo: [f32; LINES],
    /// Smoothed line-length scale (changing Size glides instead of clicking).
    scale: f32,
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let ms = |m: f32| (m * 0.001 * sample_rate) as usize;
        let max_line = ms(LINE_MS[LINES - 1] * SCALE_MAX + 4.0);
        let diffuser = |ch: usize, k: usize| Allpass {
            line: Line::new(ms(DIFFUSER_MS[ch][k]) + 2),
            delay: DIFFUSER_MS[ch][k] * 0.001 * sample_rate,
        };
        Self {
            sr: sample_rate,
            predelay: [Line::new(ms(MAX_PREDELAY_MS) + 2), Line::new(ms(MAX_PREDELAY_MS) + 2)],
            diffusers: std::array::from_fn(|ch| std::array::from_fn(|k| diffuser(ch, k))),
            lines: std::array::from_fn(|_| Line::new(max_line)),
            damp: [0.0; LINES],
            hp: [[(0.0, 0.0); 2]; 2],
            lfo: std::array::from_fn(|i| i as f32 * 0.37),
            scale: -1.0,
        }
    }

    pub fn reset(&mut self) {
        for l in self.predelay.iter_mut().chain(self.lines.iter_mut()) {
            l.clear();
        }
        for ch in &mut self.diffusers {
            for a in ch {
                a.line.clear();
            }
        }
        self.damp = [0.0; LINES];
        self.hp = [[(0.0, 0.0); 2]; 2];
    }

    /// Processes one stereo frame and returns the wet signal.
    #[inline]
    pub fn process(&mut self, input: [f32; 2], s: &Settings) -> [f32; 2] {
        let sr = self.sr;
        let target = SCALE_MIN + (SCALE_MAX - SCALE_MIN) * s.size.clamp(0.0, 1.0);
        self.scale = if self.scale < 0.0 { target } else { self.scale + (target - self.scale) * (1.0 / (0.08 * sr)) };

        // Pre-delay and input diffusion.
        let pre = s.predelay_ms.clamp(0.0, MAX_PREDELAY_MS) * 0.001 * sr;
        let mut diffused = [0.0; 2];
        for ch in 0..2 {
            let x = if s.freeze { 0.0 } else { input[ch] };
            self.predelay[ch].push(x);
            let mut v = if pre < 1.0 { x } else { self.predelay[ch].read(pre) };
            for a in &mut self.diffusers[ch] {
                v = a.run(v, DIFFUSION);
            }
            diffused[ch] = v;
        }

        // FDN: read, damp, mix, feed back.
        let damp_coef = if s.freeze { 0.0 } else { (-TAU * s.damp_hz.min(sr * 0.45) / sr).exp() };
        let mod_depth = s.modulation * 0.0012 * sr; // up to ~1.2 ms of wobble
        let mut taps = [0.0; LINES];
        let mut fb = [0.0; LINES];
        for i in 0..LINES {
            let rate = 0.21 + 0.13 * i as f32;
            self.lfo[i] = (self.lfo[i] + rate / sr) % 1.0;
            let len = LINE_MS[i] * 0.001 * sr * self.scale;
            let delay = len + mod_depth * (1.0 + (TAU * self.lfo[i]).sin());
            let y = self.lines[i].read(delay);
            taps[i] = y;
            // One-pole low-pass in the loop: highs die faster than lows.
            self.damp[i] = y + damp_coef * (self.damp[i] - y);
            let gain = if s.freeze { 1.0 } else { 10f32.powf(-3.0 * delay / (s.decay_s.max(0.05) * sr)) };
            fb[i] = self.damp[i] * gain;
        }
        hadamard(&mut fb);
        for i in 0..LINES {
            let inject = if i % 2 == 0 { diffused[0] } else { diffused[1] };
            self.lines[i].push(fb[i] + inject * 0.5);
        }

        // Even lines feed the left output, odd lines the right, with alternating signs.
        let mut out = [0.0; 2];
        for i in 0..LINES {
            let sign = if (i / 2) % 2 == 0 { 1.0 } else { -1.0 };
            out[i % 2] += taps[i] * sign * 0.5;
        }

        // Low cut on the wet signal.
        let hp = (-TAU * s.low_cut_hz.clamp(10.0, sr * 0.45) / sr).exp();
        for ch in 0..2 {
            for stage in &mut self.hp[ch] {
                let (x1, y1) = *stage;
                let x = out[ch];
                let y = hp * (y1 + x - x1);
                *stage = (x, y);
                out[ch] = y;
            }
        }

        // Width via mid/side.
        let mid = (out[0] + out[1]) * 0.5;
        let side = (out[0] - out[1]) * 0.5 * s.width;
        [mid + side, mid - side]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(decay_s: f32) -> Settings {
        Settings {
            predelay_ms: 0.0,
            size: 0.5,
            decay_s,
            modulation: 0.0,
            damp_hz: 20_000.0,
            low_cut_hz: 10.0,
            width: 1.0,
            freeze: false,
        }
    }

    fn rms(v: &[[f32; 2]]) -> f32 {
        (v.iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum::<f32>() / (2 * v.len()) as f32).sqrt()
    }

    #[test]
    fn tail_decays_about_60db_over_the_decay_time() {
        let sr = 48_000.0;
        let s = settings(1.0);
        let mut r = Reverb::new(sr);
        let out: Vec<[f32; 2]> = (0..(sr as usize * 2))
            .map(|n| r.process(if n == 0 { [1.0, 1.0] } else { [0.0, 0.0] }, &s))
            .collect();
        let w = (0.05 * sr) as usize;
        let early = rms(&out[(0.2 * sr) as usize..(0.2 * sr) as usize + w]);
        let late = rms(&out[(1.2 * sr) as usize..(1.2 * sr) as usize + w]);
        let drop = 20.0 * (early / late).log10();
        assert!((drop - 60.0).abs() < 8.0, "dropped {drop:.1} dB over 1 s");
    }

    #[test]
    fn stereo_output_is_decorrelated() {
        let sr = 48_000.0;
        let s = settings(2.0);
        let mut r = Reverb::new(sr);
        let mut rng = 1u32;
        let (mut ll, mut rr, mut lr) = (0.0f64, 0.0f64, 0.0f64);
        for n in 0..(sr as usize) {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let x = if n < 2000 { (rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0 } else { 0.0 };
            let o = r.process([x, x], &s);
            if n > 4000 {
                ll += (o[0] * o[0]) as f64;
                rr += (o[1] * o[1]) as f64;
                lr += (o[0] * o[1]) as f64;
            }
        }
        let corr = lr / (ll * rr).sqrt();
        assert!(corr.abs() < 0.5, "correlation {corr:.2}");
    }

    #[test]
    fn freeze_holds_the_tail_without_blowing_up() {
        let sr = 48_000.0;
        let mut r = Reverb::new(sr);
        let mut s = settings(2.0);
        for n in 0..(sr as usize / 2) {
            let x = if n < 500 { 0.5 } else { 0.0 };
            r.process([x, -x], &s);
        }
        s.freeze = true;
        let out: Vec<[f32; 2]> = (0..(sr as usize * 3)).map(|_| r.process([1.0, 1.0], &s)).collect();
        let first = rms(&out[..4800]);
        let last = rms(&out[out.len() - 4800..]);
        assert!(last > first * 0.7 && last < first * 1.3, "first {first} last {last}");
        assert!(out.iter().all(|f| f[0].is_finite() && f[0].abs() < 4.0));
    }
}
