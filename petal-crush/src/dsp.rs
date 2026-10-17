//! Level-crusher in the spirit of the Shure Level-Loc: heavy input drive into
//! an instant-attack limiter with saturation, a "darkness" low-pass and a
//! "shut up" expander that pushes down the noise the limiter pulls up.

use std::f32::consts::{PI, SQRT_2};

/// Level the limiter clamps to (dBFS) before make-up.
pub const CEILING_DB: f32 = -12.0;
/// Fixed make-up after the limiter.
pub const MAKEUP_DB: f32 = 7.0;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// 0..10.
    pub crush: f32,
    /// 0..10.
    pub darkness: f32,
    pub release_ms: f32,
    /// 0..10.
    pub shut_up: f32,
    pub mix: f32,
}

impl Settings {
    pub fn drive_db(&self) -> f32 {
        self.crush.clamp(0.0, 10.0) * 4.0
    }

    pub fn darkness_hz(&self) -> f32 {
        20_000.0 * (1_500.0f32 / 20_000.0).powf(self.darkness.clamp(0.0, 10.0) / 10.0)
    }
}

#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    #[inline]
    fn lowpass(&mut self, x: f32, g: f32) -> f32 {
        let a1 = 1.0 / (1.0 + g * (g + SQRT_2));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v2
    }
}

/// Live values for the editor.
#[derive(Clone, Copy, Debug, Default)]
pub struct Meter {
    pub reduction_db: f32,
    pub input_db: f32,
    /// 1 = open, lower = "shut up" is pushing the level down.
    pub shut_gain: f32,
}

pub struct Crusher {
    sr: f32,
    reduction: f32,
    release: f32,
    last_release: f32,
    input_env: f32,
    shut_gain: f32,
    dark: [Svf; 2],
}

impl Crusher {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            reduction: 0.0,
            release: 0.0,
            last_release: -1.0,
            input_env: -120.0,
            shut_gain: 1.0,
            dark: Default::default(),
        }
    }

    pub fn reset(&mut self) {
        self.reduction = 0.0;
        self.input_env = -120.0;
        self.shut_gain = 1.0;
        self.dark = Default::default();
    }

    pub fn meter(&self) -> Meter {
        Meter { reduction_db: self.reduction, input_db: self.input_env, shut_gain: self.shut_gain }
    }

    #[inline]
    pub fn process(&mut self, x: [f32; 2], s: &Settings) -> [f32; 2] {
        let sr = self.sr;
        if s.release_ms != self.last_release {
            self.release = (-1.0 / (s.release_ms.max(1.0) * 0.001 * sr)).exp();
            self.last_release = s.release_ms;
        }

        // Input follower for "shut up" (before the drive, so it tracks the real dynamics).
        let peak_in = x[0].abs().max(x[1].abs());
        let in_db = 20.0 * peak_in.max(1e-6).log10();
        let coef = if in_db > self.input_env { (-1.0 / (0.002 * sr)).exp() } else { (-1.0 / (0.08 * sr)).exp() };
        self.input_env = in_db + coef * (self.input_env - in_db);

        // Drive into an instant limiter: everything over the ceiling is held there.
        let drive = 10f32.powf(s.drive_db() / 20.0);
        let driven = [x[0] * drive, x[1] * drive];
        let level = 20.0 * driven[0].abs().max(driven[1].abs()).max(1e-6).log10();
        // Instant attack (no overshoot past the ceiling), smoothed release.
        let target = (level - CEILING_DB).max(0.0);
        self.reduction = if target > self.reduction { target } else { target + self.release * (self.reduction - target) };
        let g = 10f32.powf((MAKEUP_DB - self.reduction) / 20.0);

        // "Shut up": the quieter the input, the more the output is pulled down.
        let open = ((self.input_env + 50.0) / 24.0).clamp(0.0, 1.0);
        let shut_target = 1.0 - (s.shut_up / 10.0).clamp(0.0, 1.0) * (1.0 - open) * 0.97;
        self.shut_gain += (shut_target - self.shut_gain) * (1.0 / (0.01 * sr));

        let gd = (PI * s.darkness_hz().min(sr * 0.45) / sr).tan();
        let mut out = [0.0; 2];
        for ch in 0..2 {
            // Gritty saturation on top of the limiting.
            let y = driven[ch] * g;
            let sat = (y * 1.4).tanh() / 1.4f32.tanh() * 0.9 + y * 0.1;
            let dark = self.dark[ch].lowpass(sat, gd);
            let wet = dark * self.shut_gain;
            out[ch] = x[ch] + (wet - x[ch]) * s.mix;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn settings() -> Settings {
        Settings { crush: 5.0, darkness: 0.0, release_ms: 100.0, shut_up: 0.0, mix: 1.0 }
    }

    fn run(s: &Settings, amp: f32, f: f32, secs: f32) -> Vec<[f32; 2]> {
        let sr = 48_000.0;
        let mut c = Crusher::new(sr);
        (0..(sr * secs) as usize)
            .map(|n| {
                let v = (TAU * f * n as f32 / sr).sin() * amp;
                c.process([v, v], s)
            })
            .collect()
    }

    fn rms(v: &[[f32; 2]]) -> f32 {
        (v.iter().map(|f| f[0] * f[0]).sum::<f32>() / v.len() as f32).sqrt()
    }

    #[test]
    fn crush_flattens_dynamics() {
        let s = settings();
        // A 30 dB difference in input level ends up within a few dB at the output.
        let loud = rms(&run(&s, 0.5, 220.0, 0.5)[12_000..]);
        let quiet = rms(&run(&s, 0.016, 220.0, 0.5)[12_000..]);
        let diff = 20.0 * (loud / quiet).log10();
        assert!(diff < 8.0, "level difference {diff:.1} dB");
        let peak = run(&s, 1.0, 220.0, 0.5).iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        assert!(peak < 1.0, "peak {peak}");
    }

    #[test]
    fn darkness_removes_highs() {
        let bright = rms(&run(&settings(), 0.3, 6000.0, 0.3)[4000..]);
        let dark = rms(&run(&Settings { darkness: 10.0, ..settings() }, 0.3, 6000.0, 0.3)[4000..]);
        assert!(dark < bright * 0.25, "bright {bright} dark {dark}");
    }

    #[test]
    fn shut_up_pushes_down_quiet_input() {
        let open = rms(&run(&settings(), 0.002, 220.0, 0.5)[12_000..]);
        let shut = rms(&run(&Settings { shut_up: 10.0, ..settings() }, 0.002, 220.0, 0.5)[12_000..]);
        assert!(shut < open * 0.2, "open {open} shut {shut}");
        // Loud material is untouched by it.
        let loud_open = rms(&run(&settings(), 0.5, 220.0, 0.5)[12_000..]);
        let loud_shut = rms(&run(&Settings { shut_up: 10.0, ..settings() }, 0.5, 220.0, 0.5)[12_000..]);
        assert!((loud_shut / loud_open - 1.0).abs() < 0.02);
    }

    #[test]
    fn zero_mix_is_dry() {
        let s = Settings { mix: 0.0, ..settings() };
        let sr = 48_000.0;
        let mut c = Crusher::new(sr);
        for n in 0..4800 {
            let v = (TAU * 300.0 * n as f32 / sr).sin() * 0.4;
            let y = c.process([v, v], &s);
            assert!((y[0] - v).abs() < 1e-6);
        }
    }
}
