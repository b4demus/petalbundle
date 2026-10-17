//! Time-domain pitch shifter: two read taps glide through a delay line at
//! the shifted rate and crossfade with sin^2 windows. Whenever a tap starts
//! a new grain it is re-placed at the offset that best correlates with the
//! tap currently sounding, which removes most of the phasing of a plain
//! two-tap shifter.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Fast,
    Medium,
    Slow,
}

impl Mode {
    pub fn window_ms(self) -> f32 {
        match self {
            Mode::Fast => 22.0,
            Mode::Medium => 40.0,
            Mode::Slow => 75.0,
        }
    }
}

/// Geometry shared by both channels for one mode.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub window: f32,
    pub search: f32,
    pub min_delay: f32,
}

impl Geometry {
    pub fn new(mode: Mode, sr: f32) -> Self {
        let window = mode.window_ms() * 0.001 * sr;
        let search = (window / 3.0).round();
        Self { window, search, min_delay: 4.0 + search }
    }

    /// Latency of the unshifted path (taps rest mid-window), in samples.
    pub fn latency(&self) -> usize {
        (self.min_delay + self.window * 0.5).round() as usize
    }

    fn max_delay(&self) -> f32 {
        self.min_delay + self.window
    }
}

#[derive(Clone, Copy)]
struct Tap {
    delay: f32,
    /// 0..1 through the tap's grain; the crossfade weight is sin^2(pi * progress).
    progress: f32,
}

pub struct Shifter {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
    taps: [Tap; 2],
    geometry: Geometry,
}

impl Shifter {
    pub fn new(sr: f32) -> Self {
        // Room for the slowest mode plus the search margin on both sides.
        let slow = Geometry::new(Mode::Slow, sr);
        let len = ((slow.max_delay() + slow.search * 2.0 + 64.0) as usize).next_power_of_two();
        let g = Geometry::new(Mode::Medium, sr);
        let mut s = Self { buf: vec![0.0; len], mask: len - 1, write: 0, taps: [Tap { delay: 0.0, progress: 0.0 }; 2], geometry: g };
        s.set_geometry(g);
        s
    }

    pub fn set_geometry(&mut self, g: Geometry) {
        self.geometry = g;
        let mid = g.min_delay + g.window * 0.5;
        self.taps = [Tap { delay: mid, progress: 0.5 }, Tap { delay: g.min_delay, progress: 0.0 }];
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.set_geometry(self.geometry);
    }

    #[inline]
    fn at(&self, delay: f32) -> f32 {
        let pos = self.write as f32 - delay;
        let i = pos.floor();
        let t = pos - i;
        let i = i as isize as usize;
        // Cubic Hermite: shifting by large ratios needs a cleaner interpolator.
        let s = |k: usize| self.buf[i.wrapping_add(k).wrapping_sub(1) & self.mask];
        let (y0, y1, y2, y3) = (s(0), s(1), s(2), s(3));
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * t + c2) * t + c1) * t + y1
    }

    /// Best integer offset in [-search, search] for a new grain starting
    /// around `nominal`, matched against the sounding tap at `reference`.
    fn align(&self, nominal: f32, reference: f32) -> f32 {
        let g = self.geometry;
        let len = (g.window * 0.25) as usize;
        let step = 2;
        let mut best = (0.0f32, f32::MIN);
        let mut k = -(g.search as i32);
        while k <= g.search as i32 {
            let d = nominal + k as f32;
            if d >= 3.0 {
                let (mut dot, mut energy) = (0.0f32, 1e-9f32);
                for n in 0..len {
                    let a = self.buf[(self.write as isize - d as isize - n as isize) as usize & self.mask];
                    let b = self.buf[(self.write as isize - reference as isize - n as isize) as usize & self.mask];
                    dot += a * b;
                    energy += a * a;
                }
                let score = dot / energy.sqrt();
                if score > best.1 {
                    best = (k as f32, score);
                }
            }
            k += step;
        }
        best.0
    }

    #[inline]
    pub fn process(&mut self, x: f32, ratio: f32) -> f32 {
        self.buf[self.write & self.mask] = x;
        self.write = self.write.wrapping_add(1);

        let g = self.geometry;
        // A delay that changes by (1 - ratio) per sample plays back at `ratio`.
        let delta = 1.0 - ratio;
        let speed = delta.abs() / g.window;
        if speed > 0.0 {
            for i in 0..2 {
                // Clamped: flipping between up and down mid-grain must never
                // walk a tap past the write head or the end of the buffer.
                self.taps[i].delay = (self.taps[i].delay + delta).clamp(3.0, g.max_delay() + g.search);
                self.taps[i].progress += speed;
                if self.taps[i].progress >= 1.0 {
                    self.taps[i].progress -= 1.0;
                    // Pitch up: delay shrinks, so start at the far end, and vice versa.
                    let nominal = if delta < 0.0 { g.max_delay() } else { g.min_delay };
                    let other = self.taps[1 - i].delay;
                    self.taps[i].delay = nominal + self.align(nominal, other);
                }
            }
        }
        let mut y = 0.0;
        for t in &self.taps {
            let w = (PI * t.progress).sin();
            y += w * w * self.at(t.delay);
        }
        y
    }
}

/// Plain delay used to keep the dry signal in time with the shifted one.
pub struct DryDelay {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
}

impl DryDelay {
    pub fn new(max: usize) -> Self {
        let len = (max + 2).next_power_of_two();
        Self { buf: vec![0.0; len], mask: len - 1, write: 0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32, delay: usize) -> f32 {
        self.buf[self.write & self.mask] = x;
        let y = self.buf[self.write.wrapping_sub(delay) & self.mask];
        self.write = self.write.wrapping_add(1);
        y
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
    }
}

/// Interval name for a semitone count, for the editor.
pub fn interval_name(semitones: i32) -> &'static str {
    const NAMES: [&str; 13] = [
        "unison",
        "minor second",
        "major second",
        "minor third",
        "major third",
        "perfect fourth",
        "tritone",
        "perfect fifth",
        "minor sixth",
        "major sixth",
        "minor seventh",
        "major seventh",
        "octave",
    ];
    let s = semitones.unsigned_abs() as usize;
    if s > 12 && s % 12 == 0 {
        "octaves"
    } else if s > 12 {
        NAMES[s % 12]
    } else {
        NAMES[s]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn measured_freq(mode: Mode, semitones: f32, f: f32) -> f32 {
        let sr = 48_000.0;
        let mut s = Shifter::new(sr);
        s.set_geometry(Geometry::new(mode, sr));
        let ratio = 2f32.powf(semitones / 12.0);
        let n = sr as usize * 2;
        let out: Vec<f32> = (0..n).map(|i| s.process((TAU * f * i as f32 / sr).sin(), ratio)).collect();
        let half = &out[n / 2..];
        let crossings = half.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32;
        crossings / (half.len() as f32 / sr)
    }

    #[test]
    fn shifts_to_the_requested_pitch() {
        for (mode, st) in [(Mode::Medium, 7.0), (Mode::Medium, -5.0), (Mode::Slow, 12.0), (Mode::Fast, -12.0), (Mode::Fast, 3.0)] {
            let got = measured_freq(mode, st, 440.0);
            let want = 440.0 * 2f32.powf(st / 12.0);
            assert!((got / want - 1.0).abs() < 0.01, "{mode:?} {st:+}: {got:.1} vs {want:.1}");
        }
    }

    #[test]
    fn unshifted_path_is_a_pure_delay_of_the_reported_latency() {
        let sr = 48_000.0;
        let g = Geometry::new(Mode::Medium, sr);
        let mut s = Shifter::new(sr);
        s.set_geometry(g);
        let out: Vec<f32> = (0..g.latency() + 100).map(|n| s.process(if n == 0 { 1.0 } else { 0.0 }, 1.0)).collect();
        let peak = out.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert!((peak as i64 - g.latency() as i64).abs() <= 1, "peak {peak}, latency {}", g.latency());
    }

    #[test]
    fn output_level_stays_close_to_input() {
        let sr = 48_000.0;
        let mut s = Shifter::new(sr);
        let ratio = 2f32.powf(5.0 / 12.0);
        let mut energy = 0.0;
        let n = sr as usize;
        for i in 0..n {
            let y = s.process((TAU * 330.0 * i as f32 / sr).sin() * 0.5, ratio);
            if i > n / 4 {
                energy += y * y;
            }
        }
        let rms = (energy / (n - n / 4) as f32).sqrt();
        // A 0.5 sine has an RMS of 0.354; allow a couple of dB of crossfade loss.
        assert!(rms > 0.25 && rms < 0.42, "rms {rms}");
    }
}
