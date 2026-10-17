//! Half-time: the input is recorded continuously; at every grid boundary a
//! playhead starts at the boundary and reads the recording at a fraction of
//! normal speed until the next boundary (tape-style: pitch drops with speed).
//! The playhead is always behind the record head, so there is no latency.
//! Boundaries crossfade between the old and the new playhead.

use std::f32::consts::{FRAC_PI_2, PI, SQRT_2};

/// Longest fade-in of a new playhead, ms.
const FADE_IN_MS: f32 = 1.5;
/// Tone at or above this is a bypass.
pub const TONE_BYPASS_HZ: f32 = 19_500.0;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Playback speed, e.g. 0.5 for half time.
    pub speed: f32,
    /// Crossfade at grid boundaries, ms.
    pub smooth_ms: f32,
    /// Low-pass on the slowed signal, Hz.
    pub tone_hz: f32,
}

#[derive(Clone, Copy)]
struct Head {
    /// Absolute record position where this playhead started.
    start: f64,
    /// Samples elapsed since it started.
    elapsed: f64,
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

pub struct HalfTime {
    sr: f32,
    buf: [Vec<f32>; 2],
    mask: usize,
    /// Total samples recorded (unwrapped record head).
    written: u64,
    current: Head,
    previous: Option<Head>,
    /// 0..1 crossfade from `previous` to `current`.
    fade: f32,
    tone: [Svf; 2],
}

impl HalfTime {
    /// `max_segment_s`: the longest grid step that has to fit in the buffer.
    pub fn new(sr: f32, max_segment_s: f32) -> Self {
        let len = ((sr * max_segment_s) as usize + 16).next_power_of_two();
        Self {
            sr,
            buf: [vec![0.0; len], vec![0.0; len]],
            mask: len - 1,
            written: 0,
            current: Head { start: 0.0, elapsed: 0.0 },
            previous: None,
            fade: 1.0,
            tone: Default::default(),
        }
    }

    pub fn reset(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
        self.restart();
    }

    /// Starts a new grid step at the current record position.
    pub fn restart(&mut self) {
        self.previous = Some(self.current);
        self.current = Head { start: self.written as f64, elapsed: 0.0 };
        self.fade = 0.0;
    }

    /// Where the current playhead is, relative to its step (in samples played back).
    pub fn playback_offset(&self) -> f64 {
        self.current.elapsed
    }

    #[inline]
    fn read(&self, ch: usize, pos: f64) -> f32 {
        // Stay a few samples behind the record head: the interpolator looks ahead.
        let pos = pos.min(self.written as f64 - 3.0).max(0.0);
        let i = pos.floor();
        let t = (pos - i) as f32;
        let i = i as usize;
        let b = &self.buf[ch];
        let s = |k: usize| b[i.wrapping_add(k).wrapping_sub(1) & self.mask];
        let (y0, y1, y2, y3) = (s(0), s(1), s(2), s(3));
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * t + c2) * t + c1) * t + y1
    }

    #[inline]
    pub fn process(&mut self, x: [f32; 2], s: &Settings) -> [f32; 2] {
        for ch in 0..2 {
            self.buf[ch][self.written as usize & self.mask] = x[ch];
        }
        self.written += 1;

        let speed = s.speed.clamp(0.05, 1.0) as f64;
        let play = |h: &Head, this: &Self, ch: usize| this.read(ch, h.start + h.elapsed * speed);
        let mut out = [0.0f32; 2];
        let fade_step = 1.0 / (s.smooth_ms.max(0.1) * 0.001 * self.sr);
        // The new playhead comes in fast (so the downbeat transient survives),
        // the old one fades out over the full Smooth time.
        let fade_in_samples = (s.smooth_ms.min(FADE_IN_MS) * 0.001 * self.sr).max(1.0) as f64;
        let g_new = ((self.current.elapsed / fade_in_samples).min(1.0) as f32 * FRAC_PI_2).sin();
        let g_old = if self.fade < 1.0 { (self.fade * FRAC_PI_2).cos() } else { 0.0 };
        for ch in 0..2 {
            let mut y = play(&self.current, self, ch) * g_new;
            if let Some(prev) = self.previous.filter(|_| g_old > 0.0) {
                y += play(&prev, self, ch) * g_old;
            }
            out[ch] = y;
        }
        self.current.elapsed += 1.0;
        if let Some(p) = &mut self.previous {
            p.elapsed += 1.0;
        }
        if self.fade < 1.0 {
            self.fade = (self.fade + fade_step).min(1.0);
            if self.fade >= 1.0 {
                self.previous = None;
            }
        }

        // Tone: fully out of the way at the top of its range.
        let g = (PI * s.tone_hz.min(self.sr * 0.45) / self.sr).tan();
        for ch in 0..2 {
            let lp = self.tone[ch].lowpass(out[ch], g);
            if s.tone_hz < TONE_BYPASS_HZ {
                out[ch] = lp;
            }
        }
        out
    }
}

/// Grid step index for a song position in beats.
pub fn step_index(beats: f64, grid_beats: f64) -> i64 {
    (beats / grid_beats.max(1e-6)).floor() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn settings(speed: f32) -> Settings {
        Settings { speed, smooth_ms: 5.0, tone_hz: 22_000.0 }
    }

    /// Runs a 440 Hz sine through, restarting every `step` samples.
    fn run(speed: f32, step: usize, total: usize) -> Vec<f32> {
        let sr = 48_000.0;
        let mut h = HalfTime::new(sr, 4.0);
        let s = settings(speed);
        (0..total)
            .map(|n| {
                if n % step == 0 {
                    h.restart();
                }
                let v = (TAU * 440.0 * n as f32 / sr).sin() * 0.5;
                h.process([v, v], &s)[0]
            })
            .collect()
    }

    fn freq(v: &[f32]) -> f32 {
        let c = v.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32;
        c / (v.len() as f32 / 48_000.0)
    }

    #[test]
    fn half_speed_drops_an_octave() {
        let step = 24_000;
        let out = run(0.5, step, step * 3);
        // Measure inside a step, away from the crossfades.
        let f = freq(&out[step + 2_000..2 * step - 1_000]);
        assert!((f - 220.0).abs() < 2.0, "{f} Hz");
        let out = run(0.25, step, step * 3);
        let f = freq(&out[step + 2_000..2 * step - 1_000]);
        assert!((f - 110.0).abs() < 2.0, "{f} Hz");
    }

    #[test]
    fn each_step_replays_its_own_downbeat() {
        // A drum hit on every step boundary must come back right at the start
        // of the step (the playhead begins at the record head, no latency) and
        // keep most of its punch through the quick fade-in.
        let sr = 48_000.0;
        let mut h = HalfTime::new(sr, 4.0);
        let s = Settings { smooth_ms: 10.0, ..settings(0.5) };
        let step = 12_000;
        let mut out = Vec::new();
        for n in 0..step * 3 {
            if n % step == 0 {
                h.restart();
            }
            let k = (n % step) as f32 / sr;
            let v = (TAU * 120.0 * k).sin() * (-k * 30.0).exp();
            out.push(h.process([v, v], &s)[0]);
        }
        let hit = out[step..step + (0.02 * sr) as usize].iter().map(|v| v.abs()).fold(0.0, f32::max);
        assert!(hit > 0.5, "downbeat peak {hit}");
    }

    #[test]
    fn full_speed_is_transparent_inside_a_step() {
        let out = run(1.0, 24_000, 48_000);
        for n in 25_000..47_000 {
            // The playhead trails the record head by two samples (interpolator margin).
            let want = (TAU * 440.0 * (n as f32 - 2.0) / 48_000.0).sin() * 0.5;
            assert!((out[n] - want).abs() < 2e-3, "sample {n}: {} vs {want}", out[n]);
        }
    }

    #[test]
    fn boundaries_do_not_click() {
        let out = run(0.5, 6_000, 48_000);
        // A 0.5 sine at <= 440 Hz never moves more than ~0.03 per sample.
        let worst = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(worst < 0.06, "largest jump {worst}");
    }
}
