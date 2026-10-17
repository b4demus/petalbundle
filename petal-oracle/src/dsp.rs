//! Generative resonator: the input excites six tuned string-like resonators
//! (Karplus-Strong style comb loops). A small "brain" walks their notes
//! through a scale over time, with glide, optional wavefolding inside the
//! loops ("bite") and a long diffuse "halo".

use std::f32::consts::{PI, TAU};

pub const VOICES: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Major,
    Minor,
    Dorian,
    Phrygian,
    Lydian,
    Pentatonic,
    MinorPentatonic,
    Hirajoshi,
    WholeTone,
    Chromatic,
}

impl Scale {
    pub fn steps(self) -> &'static [i32] {
        match self {
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Scale::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Scale::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Scale::Pentatonic => &[0, 2, 4, 7, 9],
            Scale::MinorPentatonic => &[0, 3, 5, 7, 10],
            Scale::Hirajoshi => &[0, 2, 3, 7, 8],
            Scale::WholeTone => &[0, 2, 4, 6, 8, 10],
            Scale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }
}

/// Harmony settings: which notes the brain may use.
#[derive(Clone, Copy, Debug)]
pub struct Harmony {
    /// 0 = C .. 11 = B.
    pub key: i32,
    pub scale: Scale,
    /// Octave of the root (C3 = octave 3).
    pub octave: i32,
    /// How many octaves the voices may spread over (1..4).
    pub spread: i32,
}

impl Harmony {
    pub fn degrees(&self) -> i32 {
        self.scale.steps().len() as i32 * self.spread.clamp(1, 4)
    }

    /// MIDI note of a scale degree.
    pub fn note(&self, degree: i32) -> i32 {
        let steps = self.scale.steps();
        let n = steps.len() as i32;
        let d = degree.max(0);
        12 * (self.octave + 1) + self.key + steps[(d % n) as usize] + 12 * (d / n)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tone {
    pub decay_s: f32,
    /// 0..1.
    pub brightness: f32,
    /// 0..1: wavefolding inside the loops.
    pub bite: f32,
    /// 0..1: amount of the long diffuse tail.
    pub halo: f32,
    /// Glide time in seconds.
    pub glide_s: f32,
    pub freeze: bool,
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
}

struct Line {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
}

impl Line {
    fn new(len: usize) -> Self {
        let len = len.next_power_of_two();
        Self { buf: vec![0.0; len], mask: len - 1, write: 0 }
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

    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.write & self.mask] = x;
        self.write = self.write.wrapping_add(1);
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

struct Voice {
    line: Line,
    /// Current (gliding) loop length in samples.
    length: f32,
    damp: f32,
    energy: f32,
    degree: i32,
}

/// Fixed stereo positions of the six voices.
const PAN: [f32; VOICES] = [0.0, -0.75, 0.75, -0.4, 0.4, -0.15];

/// Live values for the editor.
#[derive(Clone, Copy, Debug, Default)]
pub struct VoiceView {
    pub note: f32,
    pub energy: f32,
}

pub struct Oracle {
    sr: f32,
    voices: [Voice; VOICES],
    rng: Rng,
    halo: [Line; 4],
    halo_damp: [f32; 4],
    excite_hp: [f32; 2],
    started: bool,
}

const HALO_MS: [f32; 4] = [50.3, 61.7, 73.1, 89.9];

impl Oracle {
    pub fn new(sr: f32) -> Self {
        // Room for the lowest note we allow (~20 Hz).
        let max_len = (sr / 20.0) as usize + 8;
        Self {
            sr,
            voices: std::array::from_fn(|_| Voice { line: Line::new(max_len), length: 0.0, damp: 0.0, energy: 0.0, degree: 0 }),
            rng: Rng(0x0BAD_5EED),
            halo: std::array::from_fn(|i| Line::new((HALO_MS[i] * 0.001 * sr) as usize + 8)),
            halo_damp: [0.0; 4],
            excite_hp: [0.0; 2],
            started: false,
        }
    }

    pub fn reset(&mut self) {
        for v in &mut self.voices {
            v.line.clear();
            v.damp = 0.0;
            v.energy = 0.0;
        }
        for l in &mut self.halo {
            l.clear();
        }
    }

    /// Spreads the voices over the scale (root, third, fifth across two octaves).
    pub fn seed(&mut self, h: &Harmony) {
        let n = h.scale.steps().len() as i32;
        let max = h.degrees();
        let start = [0, 2, 4, n, n + 2, n + 4];
        for (v, d) in self.voices.iter_mut().zip(start) {
            v.degree = d.min(max);
        }
        self.started = true;
    }

    /// One step of the melody brain. Voice 0 keeps the root as an anchor;
    /// the others wander through the scale, further and more often with chaos.
    pub fn step(&mut self, h: &Harmony, chaos: f32) {
        if !self.started {
            self.seed(h);
        }
        let chaos = chaos.clamp(0.0, 1.0);
        let max = h.degrees();
        let p_move = 0.15 + 0.6 * chaos;
        let leap = 1 + (chaos * 4.0).round() as i32;
        for v in 1..VOICES {
            if self.rng.next() < p_move {
                let mut step = (self.rng.next() * (2 * leap + 1) as f32) as i32 - leap;
                if step == 0 {
                    step = if self.rng.next() < 0.5 { -1 } else { 1 };
                }
                let d = self.voices[v].degree + step;
                // Reflect at the edges so voices do not pile up there.
                self.voices[v].degree = if d < 0 { -d } else if d > max { 2 * max - d } else { d }.clamp(0, max);
            }
        }
        self.voices[0].degree = 0;
    }

    pub fn views(&self) -> [VoiceView; VOICES] {
        std::array::from_fn(|i| {
            let v = &self.voices[i];
            let note = if v.length > 0.0 { 69.0 + 12.0 * (self.sr / v.length / 440.0).log2() } else { 0.0 };
            VoiceView { note, energy: v.energy }
        })
    }

    #[inline]
    pub fn process(&mut self, x: [f32; 2], h: &Harmony, t: &Tone) -> [f32; 2] {
        let sr = self.sr;
        if !self.started {
            self.seed(h);
        }
        // Excitation: mono, high-passed so lows do not just boom.
        let hp_coef = (-TAU * 120.0 / sr).exp();
        let mono = (x[0] + x[1]) * 0.5;
        self.excite_hp[0] = mono + hp_coef * (self.excite_hp[0] - mono);
        let excite = if t.freeze { 0.0 } else { (mono - self.excite_hp[0]) * 0.6 };

        let damp_hz = 800.0 * (16_000f32 / 800.0).powf(t.brightness.clamp(0.0, 1.0));
        // Frozen loops skip the damping, otherwise the upper voices would melt away.
        let damp_coef = if t.freeze { 0.0 } else { (-TAU * damp_hz / sr).exp() };
        // One-pole low-pass delays the loop by about c/(1-c) samples at low
        // frequencies; subtracting it keeps the resonators in tune.
        let lp_delay = damp_coef / (1.0 - damp_coef);
        let glide = if t.glide_s <= 0.001 { 1.0 } else { 1.0 - (-1.0 / (t.glide_s * sr)).exp() };
        let fold = t.bite.clamp(0.0, 1.0);

        let mut out = [0.0f32; 2];
        for (i, v) in self.voices.iter_mut().enumerate() {
            let note = h.note(v.degree) as f32;
            let freq = 440.0 * 2f32.powf((note - 69.0) / 12.0);
            let target = (sr / freq - lp_delay).clamp(4.0, (v.line.mask - 4) as f32);
            v.length = if v.length <= 0.0 { target } else { v.length + (target - v.length) * glide };

            let y = v.line.read(v.length);
            v.damp = y + damp_coef * (v.damp - y);
            let mut s = v.damp;
            if fold > 0.0 {
                // Wavefolder: bright, metallic, harsh as it is pushed.
                let folded = (s * (1.0 + 7.0 * fold) * PI * 0.5).sin();
                s += (folded * 0.6 - s) * fold;
            }
            let g = if t.freeze { 0.99995 } else { 10f32.powf(-3.0 / (t.decay_s.max(0.05) * freq)) };
            let fb = loop_clip(s * g);
            v.line.push(excite + fb);

            v.energy = y.abs().max(v.energy * 0.9995);
            let a = (PAN[i] + 1.0) * std::f32::consts::FRAC_PI_4;
            out[0] += y * a.cos();
            out[1] += y * a.sin();
        }
        let voices = [out[0] * 0.45, out[1] * 0.45];

        // Halo: small FDN with a long tail.
        let mut halo_out = [0.0; 2];
        if t.halo > 0.0 {
            let coef = (-TAU * 5000.0 / sr).exp();
            let mut taps = [0.0; 4];
            for (k, l) in self.halo.iter().enumerate() {
                taps[k] = l.read(HALO_MS[k] * 0.001 * sr);
            }
            let mut fb = [0.0; 4];
            for k in 0..4 {
                let len = HALO_MS[k] * 0.001 * sr;
                let g = 10f32.powf(-3.0 * len / (7.0 * sr));
                self.halo_damp[k] = taps[k] + coef * (self.halo_damp[k] - taps[k]);
                fb[k] = self.halo_damp[k] * g;
            }
            let mix = [fb[0] + fb[1] + fb[2] + fb[3], fb[0] - fb[1] + fb[2] - fb[3], fb[0] + fb[1] - fb[2] - fb[3], fb[0] - fb[1] - fb[2] + fb[3]];
            for k in 0..4 {
                self.halo[k].push(mix[k] * 0.5 + voices[k % 2] * 0.3);
            }
            halo_out = [(taps[0] + taps[2]) * t.halo, (taps[1] + taps[3]) * t.halo];
        }
        [voices[0] + halo_out[0], voices[1] + halo_out[1]]
    }
}

/// Safety limiter for the resonator loops. It must be exactly linear at
/// normal levels: even tanh's tiny compression, applied hundreds of times
/// a second, would eat the decay (and break freeze).
#[inline]
fn loop_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.5 {
        x
    } else {
        x.signum() * (0.5 + 0.5 * ((a - 0.5) * 2.0).tanh())
    }
}

pub fn note_name(midi: i32) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[midi.rem_euclid(12) as usize], midi / 12 - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harmony() -> Harmony {
        Harmony { key: 9, scale: Scale::Minor, octave: 3, spread: 2 }
    }

    fn tone() -> Tone {
        Tone { decay_s: 4.0, brightness: 0.7, bite: 0.0, halo: 0.0, glide_s: 0.0, freeze: false }
    }

    #[test]
    fn root_voice_rings_at_its_note() {
        // A3 = 220 Hz. Excite once and look at the root voice alone.
        let sr = 48_000.0;
        let mut o = Oracle::new(sr);
        let h = harmony();
        o.seed(&h);
        let t = tone();
        let mut root = Vec::new();
        for n in 0..(sr as usize) {
            let x = if n < 64 { 1.0 } else { 0.0 };
            o.process([x, x], &h, &t);
            root.push(o.voices[0].line.read(1.0));
        }
        let tail = &root[4800..];
        let crossings = tail.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32;
        let freq = crossings / (tail.len() as f32 / sr);
        assert!((freq / 220.0 - 1.0).abs() < 0.01, "root at {freq:.1} Hz");
    }

    #[test]
    fn root_voice_decays_over_the_decay_time() {
        let sr = 48_000.0;
        let mut o = Oracle::new(sr);
        let h = harmony();
        o.seed(&h);
        let t = Tone { decay_s: 1.0, brightness: 1.0, ..tone() };
        let mut root = Vec::new();
        for n in 0..(sr as usize * 2) {
            let x = if n < 64 { 1.0 } else { 0.0 };
            o.process([x, x], &h, &t);
            root.push(o.voices[0].line.read(1.0));
        }
        let peak = |from: f32| root[(from * sr) as usize..((from + 0.05) * sr) as usize].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let drop = 20.0 * (peak(0.2) / peak(1.2)).log10();
        // Brightness at max still damps the upper harmonics a little, so allow some extra.
        assert!(drop > 52.0 && drop < 72.0, "dropped {drop:.1} dB in 1 s");
    }

    #[test]
    fn brain_stays_in_the_scale() {
        let sr = 48_000.0;
        let mut o = Oracle::new(sr);
        let h = Harmony { key: 2, scale: Scale::Hirajoshi, octave: 2, spread: 3 };
        let allowed: Vec<i32> = Scale::Hirajoshi.steps().iter().map(|s| (s + 2) % 12).collect();
        for _ in 0..500 {
            o.step(&h, 1.0);
            for v in &o.voices {
                let note = h.note(v.degree);
                assert!(allowed.contains(&note.rem_euclid(12)), "note {note} out of scale");
                assert!(v.degree >= 0 && v.degree <= h.degrees());
            }
        }
    }

    #[test]
    fn freeze_sustains() {
        let sr = 48_000.0;
        let mut o = Oracle::new(sr);
        let h = harmony();
        let mut t = tone();
        for n in 0..(sr as usize / 2) {
            let x = if n % 4800 < 50 { 0.8 } else { 0.0 };
            o.process([x, x], &h, &t);
        }
        t.freeze = true;
        let energy = |o: &mut Oracle, t: &Tone| {
            let mut e = 0.0;
            for _ in 0..4800 {
                let y = o.process([0.0, 0.0], &h, t);
                e += y[0] * y[0];
            }
            e
        };
        // Let the loops settle out of their soft clipping first.
        for _ in 0..(sr as usize / 2) {
            o.process([0.0, 0.0], &h, &t);
        }
        let first = energy(&mut o, &t);
        for _ in 0..(sr as usize * 2) {
            o.process([0.0, 0.0], &h, &t);
        }
        let later = energy(&mut o, &t);
        assert!(later > first * 0.6, "first {first} later {later}");
    }

    #[test]
    fn harsh_settings_stay_bounded() {
        let sr = 48_000.0;
        let mut o = Oracle::new(sr);
        let h = harmony();
        let t = Tone { decay_s: 20.0, brightness: 1.0, bite: 1.0, halo: 1.0, glide_s: 0.2, freeze: false };
        for n in 0..(sr as usize * 2) {
            if n % 6000 == 0 {
                o.step(&h, 1.0);
            }
            let x = ((n as f32 * 0.37).sin() * 0.9) * if n % 9000 < 400 { 1.0 } else { 0.0 };
            let y = o.process([x, x], &h, &t);
            assert!(y[0].is_finite() && y[0].abs() < 8.0);
        }
    }
}
