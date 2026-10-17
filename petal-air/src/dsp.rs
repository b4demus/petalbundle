//! Dynamic high-frequency enhancer: a presence band ("mid air", 4-10 kHz)
//! and an air band ("high air", above 10 kHz) are boosted and lightly
//! excited. The boost backs off while a band is loud, so quiet passages
//! open up and loud or sibilant ones do not turn harsh.

use std::f32::consts::{PI, SQRT_2};

/// Presence centre and air corner. Both boosts are minimum-phase shapes
/// (1 + g * band-pass is a bell, 1 + g * first-order high-pass a shelf), so
/// adding them back to the dry signal never digs a notch.
const MID_CENTER: f32 = 6_500.0;
const MID_Q: f32 = 0.8;
const HIGH: f32 = 10_000.0;
/// Boost (linear amount of band added back) at 100 %: about +6 dB mid, +10 dB high.
const MID_MAX: f32 = 1.0;
const HIGH_MAX: f32 = 2.2;
/// Band level at which the dynamic boost is halved.
const DYN_THRESHOLD: f32 = 0.06;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// 0..1.
    pub mid_air: f32,
    /// 0..1.
    pub high_air: f32,
}

#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// (low-pass, high-pass), Butterworth Q.
    #[inline]
    fn run(&mut self, x: f32, g: f32) -> (f32, f32) {
        let (lp, _, hp) = self.run_q(x, g, SQRT_2);
        (lp, hp)
    }

    /// (low-pass, unity-peak band-pass, high-pass) with damping k = 1/Q.
    #[inline]
    fn run_q(&mut self, x: f32, g: f32, k: f32) -> (f32, f32, f32) {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, k * v1, x - k * v1 - v2)
    }
}

#[derive(Clone, Copy, Default)]
struct Channel {
    mid_bp: Svf,
    /// Zero-delay one-pole state; x - lp is the first-order high-pass.
    high_lp: f32,
    mid_exc_hp: Svf,
    high_exc_hp: Svf,
}

/// Live values for the editor.
#[derive(Clone, Copy, Debug, Default)]
pub struct Meter {
    /// Boost currently applied, in dB-ish units (20*log10(1 + amount)).
    pub mid_db: f32,
    pub high_db: f32,
    /// Peak of the added (air) signal over the last block.
    pub added: f32,
}

pub struct Air {
    sr: f32,
    ch: [Channel; 2],
    env_mid: f32,
    env_high: f32,
    g_mid: f32,
    g_high: f32,
    added: f32,
    coefs: [f32; 3],
    /// TPT one-pole gain g/(1+g) for the air corner.
    high_g: f32,
}

fn coef(hz: f32, sr: f32) -> f32 {
    (PI * hz.min(sr * 0.45) / sr).tan()
}

impl Air {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            ch: [Channel::default(); 2],
            env_mid: 0.0,
            env_high: 0.0,
            g_mid: 0.0,
            g_high: 0.0,
            added: 0.0,
            coefs: [coef(MID_CENTER, sr), coef(5_000.0, sr), coef(12_000.0, sr)],
            high_g: {
                let g = coef(HIGH, sr);
                g / (1.0 + g)
            },
        }
    }

    pub fn reset(&mut self) {
        self.ch = [Channel::default(); 2];
        self.env_mid = 0.0;
        self.env_high = 0.0;
    }

    /// Takes the peak of the added signal since the last call.
    pub fn take_meter(&mut self) -> Meter {
        let m = Meter {
            mid_db: 20.0 * (1.0 + self.g_mid).log10(),
            high_db: 20.0 * (1.0 + self.g_high).log10(),
            added: self.added,
        };
        self.added = 0.0;
        m
    }

    #[inline]
    pub fn process(&mut self, x: [f32; 2], s: &Settings) -> [f32; 2] {
        let [c_mid, c_mid_exc, c_high_exc] = self.coefs;
        let mut mid = [0.0f32; 2];
        let mut high = [0.0f32; 2];
        for (k, c) in self.ch.iter_mut().enumerate() {
            let (_, bp, _) = c.mid_bp.run_q(x[k], c_mid, 1.0 / MID_Q);
            mid[k] = bp;
            // Topology-preserving one-pole: matches the analog shelf shape up top.
            let v = (x[k] - c.high_lp) * self.high_g;
            let lp = v + c.high_lp;
            c.high_lp = lp + v;
            high[k] = x[k] - lp;
        }

        // Band envelopes (stereo-linked): fast attack, gentle release.
        let att = (-1.0 / (0.001 * self.sr)).exp();
        let rel = (-1.0 / (0.08 * self.sr)).exp();
        let follow = |env: f32, v: f32| {
            let c = if v > env { att } else { rel };
            v + c * (env - v)
        };
        self.env_mid = follow(self.env_mid, mid[0].abs().max(mid[1].abs()));
        self.env_high = follow(self.env_high, high[0].abs().max(high[1].abs()));

        // The louder a band already is, the less it is lifted.
        let dyn_gain = |env: f32| 1.0 / (1.0 + env / DYN_THRESHOLD);
        let target_mid = s.mid_air.clamp(0.0, 1.0) * MID_MAX * dyn_gain(self.env_mid);
        let target_high = s.high_air.clamp(0.0, 1.0) * HIGH_MAX * dyn_gain(self.env_high);
        let smooth = 1.0 - (-1.0 / (0.005 * self.sr)).exp();
        self.g_mid += (target_mid - self.g_mid) * smooth;
        self.g_high += (target_high - self.g_high) * smooth;

        let mut out = x;
        for (k, c) in self.ch.iter_mut().enumerate() {
            // Exciters: soft saturation of each band, high-passed so only the
            // new upper harmonics are added.
            let (_, mid_exc) = c.mid_exc_hp.run((mid[k] * 4.0).tanh() * 0.25 - mid[k], c_mid_exc);
            let (_, high_exc) = c.high_exc_hp.run((high[k] * 6.0).tanh() / 6.0 - high[k], c_high_exc);
            let added = mid[k] * self.g_mid + mid_exc * self.g_mid * 0.6 + high[k] * self.g_high + high_exc * self.g_high * 0.5;
            out[k] += added;
            self.added = self.added.max(added.abs());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn rms_out(s: &Settings, f: f32, amp: f32) -> f32 {
        let sr = 48_000.0;
        let mut a = Air::new(sr);
        let n = sr as usize;
        let mut sum = 0.0;
        for i in 0..n {
            let v = (TAU * f * i as f32 / sr).sin() * amp;
            let y = a.process([v, v], s);
            if i > n / 2 {
                sum += y[0] * y[0];
            }
        }
        (sum / (n / 2) as f32).sqrt() / (amp * std::f32::consts::FRAC_1_SQRT_2)
    }

    fn db(x: f32) -> f32 {
        20.0 * x.log10()
    }

    #[test]
    fn zero_amounts_are_transparent() {
        let mut a = Air::new(48_000.0);
        let s = Settings { mid_air: 0.0, high_air: 0.0 };
        for i in 0..4800 {
            let v = (TAU * 7000.0 * i as f32 / 48_000.0).sin() * 0.5;
            assert_eq!(a.process([v, -v], &s), [v, -v]);
        }
    }

    #[test]
    fn high_air_lifts_the_top_and_leaves_the_body() {
        let s = Settings { mid_air: 0.0, high_air: 1.0 };
        let top = db(rms_out(&s, 15_000.0, 0.01));
        let body = db(rms_out(&s, 800.0, 0.01));
        assert!(top > 6.0, "15 kHz lifted by {top:.1} dB");
        assert!(body.abs() < 0.5, "800 Hz moved by {body:.2} dB");
    }

    #[test]
    fn loud_highs_get_less_air_than_quiet_ones() {
        let s = Settings { mid_air: 1.0, high_air: 1.0 };
        let quiet = db(rms_out(&s, 12_000.0, 0.01));
        let loud = db(rms_out(&s, 12_000.0, 0.5));
        assert!(quiet > loud + 3.0, "quiet +{quiet:.1} dB, loud +{loud:.1} dB");
    }

    #[test]
    fn mid_air_targets_the_presence_range() {
        let s = Settings { mid_air: 1.0, high_air: 0.0 };
        let presence = db(rms_out(&s, 6_000.0, 0.01));
        let low = db(rms_out(&s, 300.0, 0.01));
        assert!(presence > 3.0, "6 kHz lifted by {presence:.1} dB");
        assert!(low.abs() < 0.5, "300 Hz moved by {low:.2} dB");
    }
}
