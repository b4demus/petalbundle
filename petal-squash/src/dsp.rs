//! Three-band upward/downward compressor in the spirit of OTT.

use std::f64::consts::PI;

/// Crossover frequencies (Hz): low|mid and mid|high.
pub const SPLIT_LOW: f64 = 88.3;
pub const SPLIT_HIGH: f64 = 2500.0;

/// Band order everywhere: 0 = high, 1 = mid, 2 = low (top to bottom in the UI).
pub const BANDS: usize = 3;

/// Approximate OTT defaults per band (high, mid, low).
pub const DEFAULT_DOWN_DB: [f32; BANDS] = [-35.5, -30.2, -33.8];
pub const DEFAULT_UP_DB: [f32; BANDS] = [-40.8, -41.8, -40.8];
pub const ATTACK_MS: [f32; BANDS] = [13.5, 22.4, 47.8];
pub const RELEASE_MS: [f32; BANDS] = [132.0, 282.0, 282.0];
/// Downward ~66:1, upward ~4.17:1 at 100 %.
pub const DOWN_RATIO: f32 = 66.0;
pub const UP_RATIO: f32 = 4.17;
/// Upward gain is capped so near-silence does not explode.
pub const MAX_BOOST_DB: f32 = 30.0;

#[derive(Clone, Copy)]
struct Coefs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

/// Butterworth (Q = 1/sqrt 2) low or high pass, RBJ cookbook.
fn butter(high: bool, freq: f64, sr: f64) -> Coefs {
    let w0 = 2.0 * PI * freq / sr;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / std::f64::consts::SQRT_2;
    let a0 = 1.0 + alpha;
    let (b0, b1) = if high { ((1.0 + cos) / 2.0, -(1.0 + cos)) } else { ((1.0 - cos) / 2.0, 1.0 - cos) };
    Coefs { b0: b0 / a0, b1: b1 / a0, b2: b0 / a0, a1: -2.0 * cos / a0, a2: (1.0 - alpha) / a0 }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    s1: f64,
    s2: f64,
}

impl Biquad {
    #[inline]
    fn run(&mut self, c: &Coefs, x: f64) -> f64 {
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }
}

/// Linkwitz-Riley 4th order = two cascaded Butterworth sections.
#[derive(Clone, Copy, Default)]
struct Lr4 {
    a: Biquad,
    b: Biquad,
}

impl Lr4 {
    #[inline]
    fn run(&mut self, c: &Coefs, x: f64) -> f64 {
        let y = self.a.run(c, x);
        self.b.run(c, y)
    }
}

struct Split {
    lp: Coefs,
    hp: Coefs,
}

impl Split {
    fn new(freq: f64, sr: f64) -> Self {
        Self { lp: butter(false, freq, sr), hp: butter(true, freq, sr) }
    }
}

/// Per-channel crossover state.
#[derive(Clone, Copy, Default)]
struct ChannelSplit {
    low_lp: Lr4,
    low_hp: Lr4,
    high_lp: Lr4,
    high_hp: Lr4,
    // LR4 all-pass at the upper split for the low band, so all bands share phase.
    ap_lp: Lr4,
    ap_hp: Lr4,
}

/// Three-way LR4 crossover whose bands sum back to an all-pass (flat magnitude).
pub struct Crossover {
    low: Split,
    high: Split,
    ch: [ChannelSplit; 2],
}

impl Crossover {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate as f64;
        Self {
            low: Split::new(SPLIT_LOW, sr),
            high: Split::new(SPLIT_HIGH, sr),
            ch: [ChannelSplit::default(); 2],
        }
    }

    pub fn reset(&mut self) {
        self.ch = [ChannelSplit::default(); 2];
    }

    /// Returns [high, mid, low].
    #[inline]
    pub fn split(&mut self, ch: usize, x: f64) -> [f64; BANDS] {
        let s = &mut self.ch[ch];
        let low = s.low_lp.run(&self.low.lp, x);
        let rest = s.low_hp.run(&self.low.hp, x);
        let mid = s.high_lp.run(&self.high.lp, rest);
        let high = s.high_hp.run(&self.high.hp, rest);
        let low = s.ap_lp.run(&self.high.lp, low) + s.ap_hp.run(&self.high.hp, low);
        [high, mid, low]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BandSettings {
    pub down_db: f32,
    pub up_db: f32,
    /// Band output gain in dB.
    pub gain_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Amounts {
    /// 0..1 scaling of the downward / upward slopes.
    pub downward: f32,
    pub upward: f32,
}

/// Gain (dB) for a detected level: boost below `up_db`, cut above `down_db`.
///
/// `peak_db` comes from a fast peak follower. The upward boost never lifts
/// that peak above the upward threshold, so a transient hitting a band that
/// was quiet a moment ago is not thrown up by the full (slow) boost.
pub fn static_gain(level_db: f32, peak_db: f32, b: &BandSettings, a: &Amounts) -> f32 {
    let down_slope = (1.0 - 1.0 / DOWN_RATIO) * a.downward;
    let up_slope = (1.0 - 1.0 / UP_RATIO) * a.upward;
    let mut g = 0.0;
    if level_db > b.down_db {
        g -= (level_db - b.down_db) * down_slope;
    }
    if level_db < b.up_db {
        let boost = ((b.up_db - level_db) * up_slope).min(MAX_BOOST_DB);
        g += boost.min((b.up_db - peak_db).max(0.0));
    }
    g
}

/// Level follower in dB with separate attack and release.
pub struct Follower {
    sample_rate: f32,
    level_db: f32,
    attack: f32,
    release: f32,
    last: Option<(f32, f32)>,
}

impl Follower {
    pub fn new(sample_rate: f32) -> Self {
        Self { sample_rate, level_db: -120.0, attack: 0.0, release: 0.0, last: None }
    }

    pub fn reset(&mut self) {
        self.level_db = -120.0;
    }

    #[inline]
    pub fn run(&mut self, peak: f32, attack_ms: f32, release_ms: f32) -> f32 {
        if self.last != Some((attack_ms, release_ms)) {
            let coef = |ms: f32| (-1.0 / (ms.max(0.02) * 0.001 * self.sample_rate)).exp();
            self.attack = coef(attack_ms);
            self.release = coef(release_ms);
            self.last = Some((attack_ms, release_ms));
        }
        let x = 20.0 * peak.max(1e-6).log10();
        let coef = if x > self.level_db { self.attack } else { self.release };
        self.level_db = x + coef * (self.level_db - x);
        self.level_db
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossover_bands_sum_flat() {
        let sr = 48_000.0;
        for f in [30.0, 88.3, 300.0, 1000.0, 2500.0, 8000.0, 16000.0] {
            // RMS, not sample peak: at high frequencies the samples miss the
            // true peak once the all-pass has shifted the phase.
            let mut x = Crossover::new(sr);
            let (mut sum_in, mut sum_out) = (0.0f64, 0.0f64);
            for n in 0..(sr as usize) {
                let s = (2.0 * PI * f * n as f64 / sr as f64).sin();
                let y: f64 = x.split(0, s).iter().sum();
                if n > sr as usize / 2 {
                    sum_in += s * s;
                    sum_out += y * y;
                }
            }
            let ratio = (sum_out / sum_in).sqrt();
            assert!((ratio - 1.0).abs() < 0.005, "{f} Hz: {ratio}");
        }
    }

    #[test]
    fn static_curve_boosts_quiet_and_cuts_loud() {
        let b = BandSettings { down_db: -30.0, up_db: -40.0, gain_db: 0.0, attack_ms: 10.0, release_ms: 100.0 };
        let full = Amounts { downward: 1.0, upward: 1.0 };
        // Steady signals: the fast peak equals the slow level.
        let g = |level: f32, a: &Amounts| static_gain(level, level, &b, a);
        assert_eq!(g(-35.0, &full), 0.0);
        // 10 dB over the downward threshold at ~66:1 -> almost all of it removed.
        assert!((g(-20.0, &full) + 10.0 * (1.0 - 1.0 / 66.0)).abs() < 1e-4);
        // 8 dB under the upward threshold at ~4.17:1 -> ~6.1 dB boost.
        assert!((g(-48.0, &full) - 8.0 * (1.0 - 1.0 / 4.17)).abs() < 1e-4);
        assert_eq!(g(-200.0, &full), MAX_BOOST_DB);
        let none = Amounts { downward: 0.0, upward: 0.0 };
        assert_eq!(g(-10.0, &none), 0.0);
        assert_eq!(g(-60.0, &none), 0.0);
    }

    #[test]
    fn transients_are_not_thrown_over_the_upward_threshold() {
        let b = BandSettings { down_db: -30.0, up_db: -40.0, gain_db: 0.0, attack_ms: 10.0, release_ms: 100.0 };
        let full = Amounts { downward: 1.0, upward: 1.0 };
        // Slow level still says "silence" (-90) but a -45 dB hit just arrived.
        let boost = static_gain(-90.0, -45.0, &b, &full);
        assert!((boost - 5.0).abs() < 1e-4, "{boost}");
        // A hit already above the upward threshold gets no boost at all.
        assert_eq!(static_gain(-90.0, -20.0, &b, &full), 0.0);
    }
}
