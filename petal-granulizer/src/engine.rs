//! Host-independent granular engine: records the incoming audio into a ring
//! buffer and plays overlapping windowed grains out of it.

use std::f32::consts::{FRAC_PI_4, PI, SQRT_2};

pub const MAX_GRAINS: usize = 128;
/// Ring buffer length in seconds (rounded up to a power of two in samples).
const BUFFER_SECONDS: f32 = 8.0;
/// Safety distance between a read head and the write head, in samples.
const HEAD_MARGIN: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PitchMode {
    Free,
    Octaves,
    Fifths,
    Pentatonic,
}

impl PitchMode {
    /// Snaps a random pitch offset (semitones) to the allowed intervals.
    fn quantize(self, st: f32) -> f32 {
        let snap = |steps: &[f32]| {
            let octave = (st / 12.0).floor();
            let within = st - octave * 12.0;
            let mut best = steps[0];
            for &s in steps.iter().chain(std::iter::once(&12.0)) {
                if (s - within).abs() < (best - within).abs() {
                    best = s;
                }
            }
            octave * 12.0 + best
        };
        match self {
            PitchMode::Free => st,
            PitchMode::Octaves => (st / 12.0).round() * 12.0,
            PitchMode::Fifths => snap(&[0.0, 7.0]),
            PitchMode::Pentatonic => snap(&[0.0, 2.0, 4.0, 7.0, 9.0]),
        }
    }
}

/// Per-block grain parameters, all in user-facing units.
#[derive(Clone, Copy, Debug)]
pub struct GrainSettings {
    /// Grains per second.
    pub density: f32,
    /// Random deviation of the spawn interval, 0..1.
    pub jitter: f32,
    pub size_ms: f32,
    /// How far behind the live input grains start reading.
    pub position_ms: f32,
    /// Random extra delay added per grain.
    pub spray_ms: f32,
    pub pitch_st: f32,
    /// Range of the random per-grain pitch offset (± semitones).
    pub pitch_rand_st: f32,
    pub pitch_mode: PitchMode,
    /// Probability that a grain plays backwards, 0..1.
    pub reverse: f32,
    /// Stereo scatter, 0..1.
    pub spread: f32,
    /// Window shape: 0 = Hann (soft), 1 = nearly rectangular (hard).
    pub shape: f32,
    /// Amount of grain output fed back into the recording, 0..~0.95.
    pub feedback: f32,
    /// Stops recording so grains loop over what is already in the buffer.
    pub freeze: bool,
}

impl Default for GrainSettings {
    fn default() -> Self {
        Self {
            density: 12.0,
            jitter: 0.5,
            size_ms: 180.0,
            position_ms: 120.0,
            spray_ms: 250.0,
            pitch_st: 0.0,
            pitch_rand_st: 12.0,
            pitch_mode: PitchMode::Octaves,
            reverse: 0.3,
            spread: 0.7,
            shape: 0.2,
            feedback: 0.2,
            freeze: false,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Grain {
    active: bool,
    /// Absolute (unwrapped) read position in the ring buffer.
    pos: f64,
    /// Read increment per output sample; negative for reversed grains.
    inc: f64,
    age: u32,
    len: u32,
    taper: f32,
    gain_l: f32,
    gain_r: f32,
    // Kept only for the visualizer.
    id: u32,
    semitones: f32,
    pan: f32,
}

/// Snapshot of one sounding grain, for the editor.
#[derive(Clone, Copy, Debug)]
pub struct GrainView {
    pub slot: usize,
    pub id: u32,
    /// Seconds between the live input and the grain's read head.
    pub delay_s: f32,
    pub size_s: f32,
    /// Current window gain, 0..1.
    pub env: f32,
    pub semitones: f32,
    pub pan: f32,
    pub reverse: bool,
}

/// Small, allocation-free xorshift RNG for the audio thread.
struct Rng(u32);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in -1..1.
    fn bipolar(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

pub struct Engine {
    sample_rate: f32,
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    mask: usize,
    /// Total number of samples written (unwrapped write head).
    write_pos: u64,
    grains: [Grain; MAX_GRAINS],
    samples_to_next: f32,
    rng: Rng,
    last_wet: (f32, f32),
    next_id: u32,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        let len = ((sample_rate * BUFFER_SECONDS) as usize).next_power_of_two();
        Self {
            sample_rate,
            buf_l: vec![0.0; len],
            buf_r: vec![0.0; len],
            mask: len - 1,
            write_pos: len as u64, // keeps `write_pos - delay` positive
            grains: [Grain::default(); MAX_GRAINS],
            samples_to_next: 0.0,
            rng: Rng(0x9E37_79B9),
            last_wet: (0.0, 0.0),
            next_id: 1,
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Unwrapped write head; stops advancing while frozen.
    pub fn write_pos(&self) -> u64 {
        self.write_pos
    }

    pub fn grain_views(&self) -> impl Iterator<Item = GrainView> + '_ {
        let sr = self.sample_rate;
        self.grains
            .iter()
            .enumerate()
            .filter(|(_, g)| g.active)
            .map(move |(slot, g)| GrainView {
                slot,
                id: g.id,
                delay_s: ((self.write_pos as f64 - g.pos) / sr as f64) as f32,
                size_s: g.len as f32 / sr,
                env: window(g.age as f32 / g.len as f32, g.taper),
                semitones: g.semitones,
                pan: g.pan,
                reverse: g.inc < 0.0,
            })
    }

    pub fn reset(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.grains = [Grain::default(); MAX_GRAINS];
        self.samples_to_next = 0.0;
        self.last_wet = (0.0, 0.0);
    }

    pub fn active_grains(&self) -> usize {
        self.grains.iter().filter(|g| g.active).count()
    }

    /// Processes one stereo frame and returns the wet (grains only) signal.
    pub fn process(&mut self, in_l: f32, in_r: f32, s: &GrainSettings) -> (f32, f32) {
        if !s.freeze {
            // Feedback goes through a soft clipper so it can never run away.
            let (fl, fr) = self.last_wet;
            let idx = self.write_pos as usize & self.mask;
            self.buf_l[idx] = in_l + (fl * s.feedback).tanh();
            self.buf_r[idx] = in_r + (fr * s.feedback).tanh();
            self.write_pos += 1;
        }

        self.samples_to_next -= 1.0;
        if self.samples_to_next <= 0.0 {
            self.spawn(s);
            let interval = self.sample_rate / s.density.max(0.1);
            let jitter = 1.0 + s.jitter.clamp(0.0, 1.0) * self.rng.bipolar() * 0.9;
            self.samples_to_next += interval * jitter;
        }

        // Keep the level roughly constant as grains overlap more or less.
        let overlap = s.density * s.size_ms * 0.001;
        let norm = 1.0 / overlap.max(1.0).sqrt();

        let mut out_l = 0.0;
        let mut out_r = 0.0;
        for g in self.grains.iter_mut().filter(|g| g.active) {
            let x = g.age as f32 / g.len as f32;
            let w = window(x, g.taper);
            let (l, r) = read_hermite(&self.buf_l, &self.buf_r, self.mask, g.pos);
            out_l += l * w * g.gain_l;
            out_r += r * w * g.gain_r;

            g.pos += g.inc;
            g.age += 1;
            if g.age >= g.len {
                g.active = false;
            }
        }

        let wet = (out_l * norm, out_r * norm);
        self.last_wet = wet;
        wet
    }

    fn spawn(&mut self, s: &GrainSettings) {
        let Some(slot) = self.grains.iter().position(|g| !g.active) else {
            return;
        };

        let sr = self.sample_rate as f64;
        let len = (s.size_ms.max(1.0) as f64 * 0.001 * sr).max(16.0);

        let rand_st = s.pitch_mode.quantize(self.rng.bipolar() * s.pitch_rand_st);
        let ratio = 2f64.powf((s.pitch_st + rand_st) as f64 / 12.0);
        let reverse = self.rng.next_f32() < s.reverse;

        let delay_ms = s.position_ms + self.rng.next_f32() * s.spray_ms;
        let mut delay = delay_ms as f64 * 0.001 * sr;
        // A forward grain must not overtake the write head while it plays.
        if !reverse {
            let head_speed = if s.freeze { 0.0 } else { 1.0 };
            delay = delay.max(len * (ratio - head_speed) + HEAD_MARGIN);
        }
        delay = delay.max(HEAD_MARGIN);
        // ...and it must not run off the old end of the buffer either.
        let reach = if reverse { len * ratio } else { 0.0 };
        let max_delay = self.mask as f64 - reach - HEAD_MARGIN;
        let delay = delay.min(max_delay.max(HEAD_MARGIN));

        let pan = self.rng.bipolar() * s.spread.clamp(0.0, 1.0);
        let angle = (pan + 1.0) * FRAC_PI_4;

        self.grains[slot] = Grain {
            active: true,
            pos: self.write_pos as f64 - delay,
            inc: if reverse { -ratio } else { ratio },
            age: 0,
            len: len as u32,
            taper: 1.0 - s.shape.clamp(0.0, 1.0) * 0.92,
            gain_l: angle.cos() * SQRT_2,
            gain_r: angle.sin() * SQRT_2,
            id: self.next_id,
            semitones: s.pitch_st + rand_st,
            pan,
        };
        self.next_id = self.next_id.wrapping_add(1).max(1);
    }
}

/// Tukey window: `taper` = 1 is a full Hann, smaller values flatten the top.
fn window(x: f32, taper: f32) -> f32 {
    let half = taper * 0.5;
    if x < half {
        0.5 - 0.5 * (PI * x / half).cos()
    } else if x > 1.0 - half {
        0.5 - 0.5 * (PI * (1.0 - x) / half).cos()
    } else {
        1.0
    }
}

fn read_hermite(bl: &[f32], br: &[f32], mask: usize, pos: f64) -> (f32, f32) {
    let i = pos.floor();
    let t = (pos - i) as f32;
    let i = i as i64;
    let at = |k: i64| (i + k) as usize & mask;
    let (a, b, c, d) = (at(-1), at(0), at(1), at(2));
    (
        hermite(bl[a], bl[b], bl[c], bl[d], t),
        hermite(br[a], br[b], br[c], br[d], t),
    )
}

fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_snaps_to_intervals() {
        assert_eq!(PitchMode::Octaves.quantize(7.0), 12.0);
        assert_eq!(PitchMode::Fifths.quantize(5.0), 7.0);
        assert_eq!(PitchMode::Fifths.quantize(-4.0), -5.0);
        assert_eq!(PitchMode::Pentatonic.quantize(3.2), 4.0);
    }

    #[test]
    fn output_is_finite_and_nonzero() {
        let mut e = Engine::new(48_000.0);
        let s = GrainSettings {
            feedback: 0.9,
            ..Default::default()
        };
        let mut energy = 0.0;
        for n in 0..48_000 * 3 {
            let x = (n as f32 * 0.05).sin() * 0.5;
            let (l, r) = e.process(x, x, &s);
            assert!(l.is_finite() && r.is_finite());
            energy += l * l + r * r;
        }
        assert!(energy > 1.0);
    }

    #[test]
    fn freeze_keeps_playing_after_input_stops() {
        let mut e = Engine::new(48_000.0);
        let mut s = GrainSettings::default();
        for n in 0..48_000 {
            let x = (n as f32 * 0.05).sin() * 0.5;
            e.process(x, x, &s);
        }
        s.freeze = true;
        let mut energy = 0.0;
        for _ in 0..48_000 * 2 {
            let (l, r) = e.process(0.0, 0.0, &s);
            energy += l * l + r * r;
        }
        assert!(energy > 1.0);
    }
}
