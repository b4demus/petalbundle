//! Filter design and processing. The same coefficient code feeds both the
//! audio path and the editor's response curve, so what you see is what you hear.

use std::f64::consts::{PI, SQRT_2};

/// Up to 48 dB/oct = 8th order = 4 biquad sections.
pub const MAX_SECTIONS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bell,
    LowShelf,
    HighShelf,
    LowCut,
    HighCut,
    Notch,
    BandPass,
}

impl Kind {
    /// Whether the gain parameter affects this filter type.
    pub fn has_gain(self) -> bool {
        matches!(self, Kind::Bell | Kind::LowShelf | Kind::HighShelf)
    }

    pub fn has_slope(self) -> bool {
        matches!(self, Kind::LowCut | Kind::HighCut)
    }
}

/// Normalized biquad (a0 = 1). First-order sections use b2 = a2 = 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coefs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coefs {
    pub const IDENTITY: Coefs = Coefs { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    /// |H(e^jw)| at `freq` Hz.
    pub fn magnitude(&self, freq: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * PI * freq / sample_rate;
        let (s1, c1) = (-w).sin_cos();
        let (s2, c2) = (-2.0 * w).sin_cos();
        let num_re = self.b0 + self.b1 * c1 + self.b2 * c2;
        let num_im = self.b1 * s1 + self.b2 * s2;
        let den_re = 1.0 + self.a1 * c1 + self.a2 * c2;
        let den_im = self.a1 * s1 + self.a2 * s2;
        ((num_re * num_re + num_im * num_im) / (den_re * den_re + den_im * den_im)).sqrt()
    }
}

/// RBJ cookbook second-order section.
fn rbj(kind: Kind, freq: f64, gain_db: f64, q: f64, sr: f64) -> Coefs {
    let w0 = 2.0 * PI * freq / sr;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    let a = 10f64.powf(gain_db / 40.0);
    match kind {
        Kind::Bell => Coefs::normalized(
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        ),
        Kind::LowShelf => {
            let k = 2.0 * a.sqrt() * alpha;
            Coefs::normalized(
                a * ((a + 1.0) - (a - 1.0) * cos + k),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - k),
                (a + 1.0) + (a - 1.0) * cos + k,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - k,
            )
        }
        Kind::HighShelf => {
            let k = 2.0 * a.sqrt() * alpha;
            Coefs::normalized(
                a * ((a + 1.0) + (a - 1.0) * cos + k),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - k),
                (a + 1.0) - (a - 1.0) * cos + k,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - k,
            )
        }
        Kind::HighCut => Coefs::normalized(
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        Kind::LowCut => Coefs::normalized(
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        Kind::Notch => Coefs::normalized(1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        Kind::BandPass => Coefs::normalized(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
    }
}

/// First-order low/high pass (bilinear transform).
fn first_order(high: bool, freq: f64, sr: f64) -> Coefs {
    let k = (PI * freq / sr).tan();
    let a1 = (k - 1.0) / (k + 1.0);
    if high {
        Coefs { b0: 1.0 / (1.0 + k), b1: -1.0 / (1.0 + k), b2: 0.0, a1, a2: 0.0 }
    } else {
        Coefs { b0: k / (1.0 + k), b1: k / (1.0 + k), b2: 0.0, a1, a2: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandSpec {
    pub kind: Kind,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
    /// Cut slope in dB/oct (6..48, multiples of 6).
    pub slope_db: u32,
}

/// Cascade of sections realising one band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Design {
    pub sections: [Coefs; MAX_SECTIONS],
    pub count: usize,
}

impl Design {
    pub fn new(spec: &BandSpec, sample_rate: f32) -> Self {
        let sr = sample_rate as f64;
        let freq = (spec.freq as f64).clamp(5.0, sr * 0.49);
        let q = (spec.q as f64).clamp(0.05, 50.0);
        let mut d = Design { sections: [Coefs::IDENTITY; MAX_SECTIONS], count: 0 };
        let mut push = |c: Coefs| {
            d.sections[d.count] = c;
            d.count += 1;
        };

        if spec.kind.has_slope() {
            // Butterworth cascade; the band's Q adds resonance to the sharpest pair.
            let order = (spec.slope_db / 6).clamp(1, 2 * MAX_SECTIONS as u32) as usize;
            let high = spec.kind == Kind::LowCut;
            let pairs = order / 2;
            let resonance = q / std::f64::consts::FRAC_1_SQRT_2;
            for k in 0..pairs {
                let theta = if order % 2 == 0 {
                    PI * (2 * k + 1) as f64 / (2 * order) as f64
                } else {
                    PI * (k + 1) as f64 / order as f64
                };
                let mut section_q = 1.0 / (2.0 * theta.cos());
                if k == pairs - 1 {
                    section_q *= resonance;
                }
                push(rbj(spec.kind, freq, 0.0, section_q, sr));
            }
            if order % 2 == 1 {
                push(first_order(high, freq, sr));
            }
        } else {
            push(rbj(spec.kind, freq, spec.gain_db as f64, q, sr));
        }
        d
    }

    pub fn magnitude(&self, freq: f64, sample_rate: f64) -> f64 {
        self.sections[..self.count]
            .iter()
            .map(|c| c.magnitude(freq, sample_rate))
            .product()
    }
}

/// Transposed direct form II, in f64 for stable low-frequency filters.
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    s1: f64,
    s2: f64,
}

impl Biquad {
    #[inline]
    pub fn process(&mut self, c: &Coefs, x: f64) -> f64 {
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Stereo state for one band.
#[derive(Clone, Copy, Default)]
pub struct BandState {
    filters: [[Biquad; MAX_SECTIONS]; 2],
}

impl BandState {
    #[inline]
    pub fn process(&mut self, design: &Design, ch: usize, mut x: f64) -> f64 {
        for (f, c) in self.filters[ch].iter_mut().zip(&design.sections[..design.count]) {
            x = f.process(c, x);
        }
        x
    }

    pub fn reset(&mut self) {
        for ch in &mut self.filters {
            for f in ch {
                f.reset();
            }
        }
    }
}

/// Filter used to audition what a band touches: band-pass around bells and
/// notches, low/high pass on the side a shelf or cut acts on (24 dB/oct).
pub fn audition_design(spec: &BandSpec, sample_rate: f32) -> Design {
    let (kind, q) = match spec.kind {
        Kind::Bell | Kind::Notch | Kind::BandPass => (Kind::BandPass, spec.q.clamp(0.4, 12.0)),
        Kind::LowShelf | Kind::LowCut => (Kind::HighCut, 0.0),
        Kind::HighShelf | Kind::HighCut => (Kind::LowCut, 0.0),
    };
    if kind == Kind::BandPass {
        // Two cascaded band-passes give a cleaner, more focused window.
        let one = Design::new(&BandSpec { kind, q, ..*spec }, sample_rate);
        let mut d = one;
        d.sections[1] = one.sections[0];
        d.count = 2;
        return d;
    }
    Design::new(&BandSpec { kind, q: SQRT_2 as f32 * 0.5, slope_db: 24, ..*spec }, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: Kind, freq: f32, gain_db: f32, q: f32, slope_db: u32) -> BandSpec {
        BandSpec { kind, freq, gain_db, q, slope_db }
    }

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn bell_hits_its_gain_at_center() {
        let d = Design::new(&spec(Kind::Bell, 1000.0, 6.0, 1.0, 12), 48_000.0);
        assert!((db(d.magnitude(1000.0, 48_000.0)) - 6.0).abs() < 0.01);
        assert!(db(d.magnitude(50.0, 48_000.0)).abs() < 0.1);
    }

    #[test]
    fn shelves_reach_their_gain() {
        let lo = Design::new(&spec(Kind::LowShelf, 200.0, -9.0, 0.707, 12), 48_000.0);
        assert!((db(lo.magnitude(20.0, 48_000.0)) + 9.0).abs() < 0.2);
        let hi = Design::new(&spec(Kind::HighShelf, 4000.0, 5.0, 0.707, 12), 48_000.0);
        assert!((db(hi.magnitude(20_000.0, 48_000.0)) - 5.0).abs() < 0.3);
    }

    #[test]
    fn cuts_are_minus_3db_at_corner_and_slope_correctly() {
        for slope in [6, 12, 18, 24, 36, 48] {
            let d = Design::new(&spec(Kind::LowCut, 1000.0, 0.0, 0.707, slope), 48_000.0);
            let corner = db(d.magnitude(1000.0, 48_000.0));
            assert!((corner + 3.01).abs() < 0.15, "slope {slope}: {corner}");
            let octave_down = db(d.magnitude(250.0, 48_000.0)) - db(d.magnitude(500.0, 48_000.0));
            assert!((octave_down + slope as f64).abs() < 1.5, "slope {slope}: {octave_down}");
        }
    }

    #[test]
    fn processing_matches_magnitude() {
        let sr = 48_000.0;
        let d = Design::new(&spec(Kind::Bell, 2000.0, -12.0, 2.0, 12), sr);
        let mut st = BandState::default();
        let mut peak = 0.0f64;
        for n in 0..48_000 {
            let x = (2.0 * PI * 2000.0 * n as f64 / sr as f64).sin();
            let y = st.process(&d, 0, x);
            if n > 24_000 {
                peak = peak.max(y.abs());
            }
        }
        assert!((db(peak) + 12.0).abs() < 0.1);
    }
}
