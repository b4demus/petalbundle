//! Lo-fi colour chain: noise -> wobble -> distort -> digital -> space ->
//! magnetic, then tone tilt, width and dry/wet. Every module crossfades in
//! with its on/off ramp, so a disabled module is bit-transparent.

use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseType {
    Hiss,
    Vinyl,
    Hum,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistortType {
    Warm,
    Tape,
    Fuzz,
}

/// Per-sample settings. `*_mix` are the smoothed on/off ramps (0..1),
/// amounts are already scaled by Magnitude.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub noise_mix: f32,
    pub noise: f32,
    pub noise_type: NoiseType,
    pub wobble_mix: f32,
    pub wobble: f32,
    pub wobble_rate: f32,
    pub distort_mix: f32,
    pub distort: f32,
    pub distort_type: DistortType,
    pub digital_mix: f32,
    pub digital: f32,
    pub digital_rate: f32,
    pub space_mix: f32,
    pub space: f32,
    pub space_size: f32,
    pub magnetic_mix: f32,
    pub magnetic: f32,
    pub dropout: f32,
    /// -1 (dark) .. 1 (bright).
    pub tone: f32,
    pub width: f32,
    pub mix: f32,
}

impl Settings {
    /// Everything off, neutral tone: the chain must pass audio unchanged.
    pub fn neutral() -> Self {
        Self {
            noise_mix: 0.0,
            noise: 0.0,
            noise_type: NoiseType::Hiss,
            wobble_mix: 0.0,
            wobble: 0.0,
            wobble_rate: 1.0,
            distort_mix: 0.0,
            distort: 0.0,
            distort_type: DistortType::Warm,
            digital_mix: 0.0,
            digital: 0.0,
            digital_rate: 0.0,
            space_mix: 0.0,
            space: 0.0,
            space_size: 0.5,
            magnetic_mix: 0.0,
            magnetic: 0.0,
            dropout: 0.0,
            tone: 0.0,
            width: 1.0,
            mix: 1.0,
        }
    }
}

struct Rng(u32);

impl Rng {
    #[inline]
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    #[inline]
    fn unit(&mut self) -> f32 {
        self.next() * 0.5 + 0.5
    }
}

#[derive(Clone, Copy, Default)]
struct OnePole {
    y: f32,
}

impl OnePole {
    #[inline]
    fn lp(&mut self, x: f32, coef: f32) -> f32 {
        self.y = x + coef * (self.y - x);
        self.y
    }
}

fn pole(hz: f32, sr: f32) -> f32 {
    (-TAU * hz / sr).exp()
}

/// One-pole DC blocker (the asymmetric curves add an offset).
#[derive(Clone, Copy, Default)]
struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + 0.9995 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

struct Line {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
}

impl Line {
    fn new(samples: usize) -> Self {
        let len = (samples + 4).next_power_of_two();
        Self { buf: vec![0.0; len], mask: len - 1, write: 0 }
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.write & self.mask] = x;
        self.write = self.write.wrapping_add(1);
    }

    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let pos = self.write as f32 - delay.max(1.0);
        let i = pos.floor();
        let t = pos - i;
        let i = i as isize as usize;
        let a = self.buf[i & self.mask];
        let b = self.buf[(i + 1) & self.mask];
        a + (b - a) * t
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

pub struct Chain {
    sr: f32,
    rng: Rng,
    // Noise
    hiss_lp: [OnePole; 2],
    hiss_hp: [OnePole; 2],
    crackle: [f32; 2],
    crackle_lp: [OnePole; 2],
    rumble: OnePole,
    hum_phase: f32,
    // Wobble
    wobble_line: [Line; 2],
    wow_phase: f32,
    flutter_phase: f32,
    drift: f32,
    drift_target: f32,
    // Distort
    dist_dc: [DcBlock; 2],
    // Digital
    hold: [f32; 2],
    hold_phase: f32,
    // Space
    space_lines: [Line; 4],
    space_damp: [OnePole; 4],
    // Magnetic
    tape_lp: [OnePole; 2],
    dropout_gain: f32,
    dropout_env: f32,
    dropout_timer: f32,
    // Tone
    tilt: [OnePole; 2],
    /// Last wobble offset (-1..1), for the editor.
    pub wobble_now: f32,
}

const SPACE_MS: [f32; 4] = [23.3, 29.1, 37.7, 43.1];
const WOBBLE_BASE_MS: f32 = 8.0;
const WOBBLE_DEPTH_MS: f32 = 1.6;

impl Chain {
    pub fn new(sr: f32) -> Self {
        let ms = |m: f32| (m * 0.001 * sr) as usize;
        Self {
            sr,
            rng: Rng(0x2468_ACE1),
            hiss_lp: Default::default(),
            hiss_hp: Default::default(),
            crackle: [0.0; 2],
            crackle_lp: Default::default(),
            rumble: OnePole::default(),
            hum_phase: 0.0,
            wobble_line: [Line::new(ms(WOBBLE_BASE_MS * 2.0 + WOBBLE_DEPTH_MS * 2.0)), Line::new(ms(WOBBLE_BASE_MS * 2.0 + WOBBLE_DEPTH_MS * 2.0))],
            wow_phase: 0.0,
            flutter_phase: 0.0,
            drift: 0.0,
            drift_target: 0.0,
            dist_dc: Default::default(),
            hold: [0.0; 2],
            hold_phase: 0.0,
            space_lines: std::array::from_fn(|i| Line::new(ms(SPACE_MS[i] * 2.2))),
            space_damp: Default::default(),
            tape_lp: Default::default(),
            dropout_gain: 1.0,
            dropout_env: 0.0,
            dropout_timer: 1.0,
            tilt: Default::default(),
            wobble_now: 0.0,
        }
    }

    pub fn reset(&mut self) {
        for l in self.wobble_line.iter_mut().chain(self.space_lines.iter_mut()) {
            l.clear();
        }
        self.dropout_gain = 1.0;
        self.dropout_env = 0.0;
    }

    /// Current dropout gain (1 = none), for the editor.
    pub fn dropout_gain(&self) -> f32 {
        self.dropout_gain
    }

    #[inline]
    pub fn process(&mut self, input: [f32; 2], s: &Settings) -> [f32; 2] {
        let sr = self.sr;
        let mut x = input;

        // --- Noise -------------------------------------------------------
        if s.noise_mix > 0.0 {
            let n = self.noise(s);
            for ch in 0..2 {
                x[ch] += n[ch] * s.noise_mix;
            }
        }

        // --- Wobble: wow + flutter + slow random drift on a short delay ----
        {
            let rate = s.wobble_rate.max(0.05);
            self.wow_phase = (self.wow_phase + rate / sr) % 1.0;
            self.flutter_phase = (self.flutter_phase + rate * 7.3 / sr) % 1.0;
            if self.rng.unit() < rate * 0.5 / sr {
                self.drift_target = self.rng.next();
            }
            self.drift += (self.drift_target - self.drift) * (1.0 / (0.4 * sr));
            let wob = 0.7 * (TAU * self.wow_phase).sin() + 0.2 * self.drift + 0.1 * (TAU * self.flutter_phase).sin();
            self.wobble_now = wob * s.wobble * s.wobble_mix;
            let delay = (WOBBLE_BASE_MS + WOBBLE_DEPTH_MS * s.wobble * wob) * 0.001 * sr;
            for ch in 0..2 {
                self.wobble_line[ch].push(x[ch]);
                if s.wobble_mix > 0.0 {
                    let y = self.wobble_line[ch].read(delay);
                    x[ch] += (y - x[ch]) * s.wobble_mix;
                }
            }
        }

        // --- Distort -----------------------------------------------------
        if s.distort_mix > 0.0 {
            let drive = 1.0 + s.distort * 7.0;
            let curve = |d: f32| match s.distort_type {
                // Asymmetric tube-ish curve (even harmonics), zero at zero.
                DistortType::Warm => (d + 0.3).tanh() - 0.3f32.tanh(),
                DistortType::Tape => (2.0 / PI) * (d * PI / 2.0).atan(),
                DistortType::Fuzz => (d * 2.0).clamp(-1.0, 1.0) * 0.8 + d.tanh() * 0.2,
            };
            // A 0.5-peak signal keeps its level at any drive.
            let norm = 0.5 / curve(0.5 * drive).abs().max(1e-3);
            for ch in 0..2 {
                let y = self.dist_dc[ch].run(curve(x[ch] * drive) * norm);
                x[ch] += (y - x[ch]) * s.distort_mix;
            }
        }

        // --- Digital: bit depth + sample-and-hold rate reduction --------------
        if s.digital_mix > 0.0 {
            let bits = 16.0 - 8.0 * s.digital;
            let steps = 2f32.powf(bits - 1.0);
            let target_sr = sr * (1.0 - 0.75 * s.digital_rate.clamp(0.0, 1.0));
            self.hold_phase += target_sr / sr;
            let refresh = self.hold_phase >= 1.0;
            if refresh {
                self.hold_phase -= 1.0;
            }
            for ch in 0..2 {
                if refresh {
                    self.hold[ch] = (x[ch] * steps).round() / steps;
                }
                x[ch] += (self.hold[ch] - x[ch]) * s.digital_mix;
            }
        }

        // --- Space: small 4-line FDN room ------------------------------------
        if s.space_mix > 0.0 {
            let scale = 0.4 + 1.6 * s.space_size;
            let rt60 = 0.3 + 1.1 * s.space_size;
            let mut taps = [0.0; 4];
            for (i, line) in self.space_lines.iter().enumerate() {
                taps[i] = line.read(SPACE_MS[i] * 0.001 * sr * scale);
            }
            let coef = pole(6000.0, sr);
            let mut fb = [0.0; 4];
            for i in 0..4 {
                let len = SPACE_MS[i] * 0.001 * sr * scale;
                let g = 10f32.powf(-3.0 * len / (rt60 * sr));
                fb[i] = self.space_damp[i].lp(taps[i], coef) * g;
            }
            // 4x4 Hadamard.
            let h = [
                fb[0] + fb[1] + fb[2] + fb[3],
                fb[0] - fb[1] + fb[2] - fb[3],
                fb[0] + fb[1] - fb[2] - fb[3],
                fb[0] - fb[1] - fb[2] + fb[3],
            ];
            for i in 0..4 {
                let inject = if i % 2 == 0 { x[0] } else { x[1] };
                self.space_lines[i].push(h[i] * 0.5 + inject * 0.4);
            }
            let wet = [(taps[0] + taps[2]) * 0.35, (taps[1] + taps[3]) * 0.35];
            for ch in 0..2 {
                x[ch] += wet[ch] * s.space * s.space_mix;
            }
        }

        // --- Magnetic: saturation, high loss, dropouts -------------------------
        if s.magnetic_mix > 0.0 {
            // Random dropouts: brief dips in level.
            self.dropout_timer -= (0.3 + 3.0 * s.dropout) / sr;
            if self.dropout_timer <= 0.0 {
                self.dropout_timer = 0.3 + self.rng.unit() * 1.5;
                if self.rng.unit() < s.dropout {
                    self.dropout_env = 0.4 + 0.6 * self.rng.unit();
                }
            }
            self.dropout_env *= (-1.0 / (0.09 * sr)).exp();
            let target = 1.0 - self.dropout_env * s.dropout * 0.5;
            self.dropout_gain += (target - self.dropout_gain) * (1.0 / (0.008 * sr));

            let cut = pole(18_000.0 - 7_000.0 * s.magnetic, sr);
            let drive = 1.0 + 1.2 * s.magnetic;
            for ch in 0..2 {
                let sat = (x[ch] * drive).tanh() / drive.tanh();
                let y = self.tape_lp[ch].lp(sat, cut) * self.dropout_gain;
                x[ch] += (y - x[ch]) * s.magnetic_mix;
            }
        } else {
            self.dropout_gain = 1.0;
        }

        // --- Tone tilt around 800 Hz (+-6 dB), width, mix ---------------------
        let coef = pole(800.0, sr);
        let g_low = 10f32.powf(-s.tone * 6.0 / 20.0);
        let g_high = 10f32.powf(s.tone * 6.0 / 20.0);
        for ch in 0..2 {
            let low = self.tilt[ch].lp(x[ch], coef);
            let high = x[ch] - low;
            x[ch] = low * g_low + high * g_high;
        }
        let mid = (x[0] + x[1]) * 0.5;
        let side = (x[0] - x[1]) * 0.5 * s.width;
        let wet = [mid + side, mid - side];
        [
            input[0] + (wet[0] - input[0]) * s.mix,
            input[1] + (wet[1] - input[1]) * s.mix,
        ]
    }

    fn noise(&mut self, s: &Settings) -> [f32; 2] {
        let sr = self.sr;
        let a = s.noise;
        match s.noise_type {
            NoiseType::Hiss => {
                let lp = pole(9000.0, sr);
                let hp = pole(400.0, sr);
                let mut out = [0.0; 2];
                for ch in 0..2 {
                    let w = self.rng.next();
                    let l = self.hiss_lp[ch].lp(w, lp);
                    let h = l - self.hiss_hp[ch].lp(l, hp);
                    out[ch] = h * a * 0.045;
                }
                out
            }
            NoiseType::Vinyl => {
                // Sparse clicks (more and louder with the amount) + low rumble.
                let rate = 3.0 + 30.0 * a;
                if self.rng.unit() < rate / sr {
                    let amp = (0.2 + 0.8 * self.rng.unit().powi(3)) * a * 0.25;
                    let pan = self.rng.unit();
                    self.crackle[0] += amp * (1.0 - pan) * self.rng.next().signum();
                    self.crackle[1] += amp * pan * self.rng.next().signum();
                }
                let decay = (-1.0 / (0.0006 * sr)).exp();
                let lp = pole(5000.0, sr);
                let rumble = self.rumble.lp(self.rng.next(), pole(40.0, sr)) * a * 0.15;
                let mut out = [0.0; 2];
                for ch in 0..2 {
                    self.crackle[ch] *= decay;
                    let soft = self.hiss_lp[ch].lp(self.rng.next(), lp) * a * 0.005;
                    out[ch] = self.crackle_lp[ch].lp(self.crackle[ch], pole(7000.0, sr)) + soft + rumble;
                }
                out
            }
            NoiseType::Hum => {
                self.hum_phase = (self.hum_phase + 50.0 / sr) % 1.0;
                let p = TAU * self.hum_phase;
                let hum = (p.sin() + 0.5 * (2.0 * p).sin() + 0.3 * (3.0 * p).sin()) * a * 0.012;
                [hum, hum]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize, sr: f32) -> f32 {
        (TAU * 440.0 * n as f32 / sr).sin() * 0.5
    }

    #[test]
    fn everything_off_is_transparent() {
        let sr = 48_000.0;
        let mut c = Chain::new(sr);
        let s = Settings::neutral();
        for n in 0..10_000 {
            let x = sine(n, sr);
            let y = c.process([x, -x], &s);
            assert!((y[0] - x).abs() < 1e-5 && (y[1] + x).abs() < 1e-5, "sample {n}");
        }
    }

    #[test]
    fn noise_types_add_noise_into_silence() {
        let sr = 48_000.0;
        for t in [NoiseType::Hiss, NoiseType::Vinyl, NoiseType::Hum] {
            let mut c = Chain::new(sr);
            let s = Settings { noise_mix: 1.0, noise: 1.0, noise_type: t, ..Settings::neutral() };
            let mut sum = 0.0;
            for _ in 0..sr as usize {
                let y = c.process([0.0, 0.0], &s);
                sum += y[0] * y[0];
            }
            let rms_db = 10.0 * (sum / sr).log10();
            assert!(rms_db > -60.0 && rms_db < -12.0, "{t:?}: {rms_db:.1} dB");
        }
    }

    #[test]
    fn digital_reduces_resolution() {
        let sr = 48_000.0;
        let mut c = Chain::new(sr);
        let s = Settings { digital_mix: 1.0, digital: 1.0, digital_rate: 0.0, ..Settings::neutral() };
        let mut levels = std::collections::BTreeSet::new();
        for n in 0..4800 {
            let y = c.process([sine(n, sr), 0.0], &s);
            levels.insert((y[0] * 1e4).round() as i32);
        }
        // 8 bits at full amount -> at most 2 * 128 + 1 levels (16-bit audio would give thousands).
        assert!(levels.len() <= 257, "{} distinct levels", levels.len());
    }

    #[test]
    fn everything_on_stays_bounded() {
        let sr = 48_000.0;
        let mut c = Chain::new(sr);
        let s = Settings {
            noise_mix: 1.0,
            noise: 1.0,
            noise_type: NoiseType::Vinyl,
            wobble_mix: 1.0,
            wobble: 1.0,
            wobble_rate: 6.0,
            distort_mix: 1.0,
            distort: 1.0,
            distort_type: DistortType::Fuzz,
            digital_mix: 1.0,
            digital: 1.0,
            digital_rate: 1.0,
            space_mix: 1.0,
            space: 1.0,
            space_size: 1.0,
            magnetic_mix: 1.0,
            magnetic: 1.0,
            dropout: 1.0,
            tone: 1.0,
            width: 2.0,
            mix: 1.0,
        };
        for n in 0..(sr as usize * 2) {
            let x = sine(n, sr) * 2.0;
            let y = c.process([x, x], &s);
            assert!(y[0].is_finite() && y[0].abs() < 6.0 && y[1].abs() < 6.0);
        }
    }
}
