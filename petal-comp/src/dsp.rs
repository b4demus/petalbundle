//! Feed-forward, stereo-linked compressor: soft-knee gain computer followed by
//! attack/release smoothing in the dB domain.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub threshold_db: f32,
    pub ratio: f32,
    pub knee_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

/// Static curve: output level (dB) for an input level (dB), before make-up gain.
pub fn transfer(x: f32, s: &Settings) -> f32 {
    let t = s.threshold_db;
    let w = s.knee_db.max(0.0);
    let slope = 1.0 / s.ratio.max(1.0) - 1.0;
    let over = x - t;
    if 2.0 * over < -w {
        x
    } else if w > 0.0 && 2.0 * over.abs() <= w {
        x + slope * (over + w / 2.0).powi(2) / (2.0 * w)
    } else {
        t + over / s.ratio.max(1.0)
    }
}

pub struct Compressor {
    sample_rate: f32,
    /// Current gain reduction in dB (>= 0).
    reduction: f32,
    attack_coef: f32,
    release_coef: f32,
    last: Option<(f32, f32)>,
}

impl Compressor {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate, reduction: 0.0, attack_coef: 0.0, release_coef: 0.0, last: None }
    }

    pub fn reset(&mut self) {
        self.reduction = 0.0;
    }

    fn coefs(&mut self, s: &Settings) {
        let key = (s.attack_ms, s.release_ms);
        if self.last != Some(key) {
            let coef = |ms: f32| (-1.0 / (ms.max(0.005) * 0.001 * self.sample_rate)).exp();
            self.attack_coef = coef(s.attack_ms);
            self.release_coef = coef(s.release_ms);
            self.last = Some(key);
        }
    }

    /// Takes the stereo-linked detector level (linear peak) and returns the
    /// gain reduction to apply, in dB (>= 0).
    #[inline]
    pub fn process(&mut self, detector: f32, s: &Settings) -> f32 {
        self.coefs(s);
        let x = 20.0 * detector.max(1e-6).log10();
        let target = x - transfer(x, s);
        let coef = if target > self.reduction { self.attack_coef } else { self.release_coef };
        self.reduction = target + coef * (self.reduction - target);
        self.reduction
    }
}

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings { threshold_db: -20.0, ratio: 4.0, knee_db: 0.0, attack_ms: 10.0, release_ms: 100.0 }
    }

    #[test]
    fn hard_knee_curve() {
        let s = settings();
        assert_eq!(transfer(-30.0, &s), -30.0);
        assert!((transfer(-20.0, &s) + 20.0).abs() < 1e-5);
        // 8 dB over threshold at 4:1 -> 2 dB over.
        assert!((transfer(-12.0, &s) + 18.0).abs() < 1e-5);
    }

    #[test]
    fn soft_knee_is_continuous_and_meets_hard_curve_outside() {
        let s = Settings { knee_db: 12.0, ..settings() };
        assert!((transfer(-26.0, &s) + 26.0).abs() < 1e-4);
        assert!((transfer(-14.0, &s) - (-20.0 + 6.0 / 4.0)).abs() < 1e-4);
        let mut prev = transfer(-40.0, &s);
        for i in 1..400 {
            let x = -40.0 + i as f32 * 0.1;
            let y = transfer(x, &s);
            assert!((y - prev).abs() < 0.11, "jump at {x}");
            assert!(y >= prev);
            prev = y;
        }
    }

    #[test]
    fn attack_and_release_follow_their_time_constants() {
        let sr = 48_000.0;
        let s = settings();
        let mut c = Compressor::new(sr);
        // Step to -8 dBFS: target reduction = 12 - 3 = 9 dB.
        let loud = db_to_gain(-8.0);
        let attack_samples = (0.010 * sr) as usize;
        let mut r = 0.0;
        for _ in 0..attack_samples {
            r = c.process(loud, &s);
        }
        assert!((r / 9.0 - 0.632).abs() < 0.02, "after one attack constant: {r}");
        for _ in 0..(sr as usize) {
            r = c.process(loud, &s);
        }
        assert!((r - 9.0).abs() < 1e-3);
        // Release back towards 0 when the signal drops below threshold.
        let quiet = db_to_gain(-40.0);
        for _ in 0..(0.100 * sr) as usize {
            r = c.process(quiet, &s);
        }
        assert!((r / 9.0 - 0.368).abs() < 0.02, "after one release constant: {r}");
    }
}
