//! Stereo delay with normal / inverted / ping-pong feedback and a low-pass
//! "cut" filter inside the feedback loop.

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Invert,
    PingPong,
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Delay time per channel in samples (fractional).
    pub time_l: f32,
    pub time_r: f32,
    pub feedback: f32,
    pub cut_hz: f32,
    pub mode: Mode,
}

/// Zero-delay-feedback state-variable low-pass (Simper), Q = 1/sqrt 2.
#[derive(Clone, Copy, Default)]
struct Lowpass {
    ic1: f32,
    ic2: f32,
}

impl Lowpass {
    #[inline]
    fn run(&mut self, x: f32, g: f32) -> f32 {
        let k = std::f32::consts::SQRT_2;
        let a1 = 1.0 / (1.0 + g * (g + k));
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

/// One-pole DC blocker so feedback can never build up an offset.
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

pub struct Delay {
    sample_rate: f32,
    buf: [Vec<f32>; 2],
    mask: usize,
    write: usize,
    /// Smoothed delay times (samples): changes glide like tape instead of clicking.
    time: [f32; 2],
    glide: f32,
    lowpass: [Lowpass; 2],
    dc: [DcBlock; 2],
}

impl Delay {
    pub fn new(sample_rate: f32, max_seconds: f32) -> Self {
        let len = ((sample_rate * max_seconds) as usize + 8).next_power_of_two();
        Self {
            sample_rate,
            buf: [vec![0.0; len], vec![0.0; len]],
            mask: len - 1,
            write: 0,
            time: [0.0; 2],
            glide: 1.0 - (-1.0 / (0.06 * sample_rate)).exp(),
            lowpass: [Lowpass::default(); 2],
            dc: [DcBlock::default(); 2],
        }
    }

    pub fn max_delay(&self) -> f32 {
        (self.mask - 4) as f32
    }

    pub fn reset(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
        self.lowpass = [Lowpass::default(); 2];
        self.dc = [DcBlock::default(); 2];
        self.time = [0.0; 2];
    }

    fn read(&self, ch: usize, delay: f32) -> f32 {
        let pos = self.write as f32 - delay;
        let i = pos.floor();
        let t = pos - i;
        let i = i as isize;
        let at = |k: isize| self.buf[ch][(i + k) as usize & self.mask];
        hermite(at(-1), at(0), at(1), at(2), t)
    }

    /// Processes one frame of the (already panned / scaled) delay input and
    /// returns the wet output.
    #[inline]
    pub fn process(&mut self, input: [f32; 2], s: &Settings) -> [f32; 2] {
        let targets = [s.time_l, s.time_r];
        for ch in 0..2 {
            let target = targets[ch].clamp(1.0, self.max_delay());
            // Jump straight to the first time ever set, glide afterwards.
            self.time[ch] = if self.time[ch] == 0.0 {
                target
            } else {
                self.time[ch] + (target - self.time[ch]) * self.glide
            };
        }
        let wet = [self.read(0, self.time[0]), self.read(1, self.time[1])];

        let g = (PI * s.cut_hz.min(self.sample_rate * 0.45) / self.sample_rate).tan();
        let mut fb = [0.0; 2];
        for ch in 0..2 {
            let filtered = self.lowpass[ch].run(wet[ch], g);
            // Soft clip so 100 % feedback saturates instead of exploding.
            fb[ch] = soft_clip(self.dc[ch].run(filtered) * s.feedback);
        }
        let back = match s.mode {
            Mode::Normal => fb,
            Mode::Invert => [-fb[0], -fb[1]],
            Mode::PingPong => [fb[1], fb[0]],
        };
        let w = self.write & self.mask;
        self.buf[0][w] = input[0] + back[0];
        self.buf[1][w] = input[1] + back[1];
        self.write = self.write.wrapping_add(1);
        wet
    }
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    // Transparent below ~0.5, smoothly limits towards +/-1.5.
    if x.abs() < 0.5 {
        x
    } else {
        x.signum() * (0.5 + (x.abs() - 0.5).tanh())
    }
}

fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}

/// One echo for the visualizer: when it arrives, on which side and how loud.
#[derive(Clone, Copy, Debug)]
pub struct Tap {
    pub time_s: f32,
    /// 0 = left, 1 = right.
    pub side: usize,
    /// Signed amplitude relative to the delay input (negative = inverted).
    pub amp: f32,
    pub repeat: usize,
}

/// Predicts the echo pattern for an impulse panned with `input` gains.
pub fn taps(
    input: [f32; 2],
    time_s: [f32; 2],
    feedback: f32,
    mode: Mode,
    max_taps: usize,
) -> Vec<Tap> {
    let mut out = Vec::new();
    // Each line keeps (arrival time, amplitude) of the echo it will emit next.
    let mut pending: Vec<(f32, usize, f32, usize)> = vec![
        (time_s[0], 0, input[0], 1),
        (time_s[1], 1, input[1], 1),
    ];
    while out.len() < max_taps {
        let Some(idx) = pending
            .iter()
            .enumerate()
            .filter(|(_, p)| p.2.abs() > 0.004)
            .min_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
            .map(|(i, _)| i)
        else {
            break;
        };
        let (t, side, amp, repeat) = pending.remove(idx);
        out.push(Tap { time_s: t, side, amp, repeat });
        let next_amp = amp * feedback.min(1.0);
        let (next_side, sign) = match mode {
            Mode::Normal => (side, 1.0),
            Mode::Invert => (side, -1.0),
            Mode::PingPong => (1 - side, 1.0),
        };
        pending.push((t + time_s[next_side], next_side, next_amp * sign, repeat + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mode: Mode, feedback: f32, input: [f32; 2], n: usize) -> Vec<[f32; 2]> {
        let mut d = Delay::new(48_000.0, 2.0);
        let s = Settings { time_l: 100.0, time_r: 100.0, feedback, cut_hz: 24_000.0, mode };
        (0..n)
            .map(|i| d.process(if i == 0 { input } else { [0.0; 2] }, &s))
            .collect()
    }

    /// Area of the echo around t: robust to the cut filter smearing the impulse.
    fn peak_near(out: &[[f32; 2]], at: usize, ch: usize) -> f32 {
        out[at - 4..at + 12].iter().map(|f| f[ch]).sum()
    }

    #[test]
    fn echo_arrives_at_the_delay_time() {
        let out = run(Mode::Normal, 0.0, [0.4, 0.4], 400);
        assert!(peak_near(&out, 100, 0) > 0.35);
        assert!(out[150][0].abs() < 1e-3 && out[250][0].abs() < 1e-3);
    }

    #[test]
    fn feedback_modes() {
        // Normal: second echo on the same side, scaled by the feedback.
        let n = run(Mode::Normal, 0.5, [0.4, 0.0], 400);
        assert!((peak_near(&n, 200, 0) - 0.2).abs() < 0.03);
        // Invert: second echo flips polarity.
        let i = run(Mode::Invert, 0.5, [0.4, 0.0], 400);
        assert!((peak_near(&i, 200, 0) + 0.2).abs() < 0.03);
        // Ping-pong: second echo moves to the other side.
        let p = run(Mode::PingPong, 0.5, [0.4, 0.0], 400);
        assert!(peak_near(&p, 200, 0).abs() < 0.02);
        assert!((peak_near(&p, 200, 1) - 0.2).abs() < 0.03);
    }

    #[test]
    fn full_feedback_stays_bounded() {
        let out = run(Mode::Normal, 1.0, [1.0, 1.0], 48_000);
        assert!(out.iter().all(|f| f[0].abs() < 2.0 && f[1].abs() < 2.0));
    }

    #[test]
    fn tap_prediction_matches_ping_pong() {
        let t = taps([1.0, 0.0], [0.25, 0.25], 0.5, Mode::PingPong, 4);
        let sides: Vec<_> = t.iter().map(|t| t.side).collect();
        assert_eq!(sides, [0, 1, 0, 1]);
        assert!((t[1].time_s - 0.5).abs() < 1e-6 && (t[1].amp - 0.5).abs() < 1e-6);
    }
}
