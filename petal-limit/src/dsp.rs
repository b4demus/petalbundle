//! Look-ahead brickwall limiter.
//!
//! For every sample the gain needed to keep it under the ceiling is computed.
//! A sliding minimum over the look-ahead window followed by a moving average
//! (no longer than that window) gives a smooth gain curve that is provably at
//! or below the needed gain of the sample leaving the delay line, so nothing
//! passes above the ceiling. Release is then smoothed only while the gain rises.

use std::collections::VecDeque;
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Transparent,
    Punchy,
    Aggressive,
    Safe,
}

impl Style {
    /// (attack as a fraction of the look-ahead, release multiplier, pre-clip amount).
    fn shape(self) -> (f32, f32, f32) {
        match self {
            Style::Transparent => (1.0, 1.5, 0.0),
            Style::Punchy => (0.35, 0.7, 0.0),
            Style::Aggressive => (0.2, 0.4, 0.6),
            Style::Safe => (1.0, 2.0, 0.0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub gain_db: f32,
    pub ceiling_db: f32,
    pub release_ms: f32,
    /// 0..1 stereo linking.
    pub link: f32,
    pub style: Style,
    pub true_peak: bool,
}

/// Sliding-window minimum (monotonic deque), O(1) amortized.
struct SlidingMin {
    window: usize,
    deque: VecDeque<(u64, f32)>,
    index: u64,
}

impl SlidingMin {
    fn new(capacity: usize) -> Self {
        Self { window: 1, deque: VecDeque::with_capacity(capacity + 2), index: 0 }
    }

    fn set_window(&mut self, w: usize) {
        self.window = w.max(1);
    }

    #[inline]
    fn push(&mut self, v: f32) -> f32 {
        while self.deque.back().is_some_and(|&(_, b)| b >= v) {
            self.deque.pop_back();
        }
        self.deque.push_back((self.index, v));
        let oldest = self.index + 1 - self.window.min(self.index as usize + 1) as u64;
        while self.deque.front().is_some_and(|&(i, _)| i < oldest) {
            self.deque.pop_front();
        }
        self.index += 1;
        self.deque.front().map_or(v, |&(_, m)| m)
    }

    fn reset(&mut self) {
        self.deque.clear();
        self.index = 0;
    }
}

/// Moving average over a fixed ring (f64 sum, refreshed to avoid drift).
struct BoxFilter {
    ring: Vec<f32>,
    len: usize,
    pos: usize,
    sum: f64,
    since_refresh: usize,
}

impl BoxFilter {
    fn new(capacity: usize) -> Self {
        Self { ring: vec![1.0; capacity.max(1)], len: 1, pos: 0, sum: 1.0, since_refresh: 0 }
    }

    fn set_len(&mut self, len: usize) {
        let len = len.clamp(1, self.ring.len());
        if len != self.len {
            self.len = len;
            self.ring[..len].fill(1.0);
            self.pos = 0;
            self.sum = len as f64;
        }
    }

    #[inline]
    fn push(&mut self, v: f32) -> f32 {
        self.sum += v as f64 - self.ring[self.pos] as f64;
        self.ring[self.pos] = v;
        self.pos = (self.pos + 1) % self.len;
        self.since_refresh += 1;
        if self.since_refresh > 4096 {
            self.sum = self.ring[..self.len].iter().map(|&x| x as f64).sum();
            self.since_refresh = 0;
        }
        (self.sum / self.len as f64) as f32
    }

    fn reset(&mut self) {
        self.ring[..self.len].fill(1.0);
        self.sum = self.len as f64;
    }
}

struct Channel {
    delay: Vec<f32>,
    write: usize,
    /// Last four input samples for the inter-sample peak estimate.
    hist: [f32; 4],
    min: SlidingMin,
    avg: BoxFilter,
    gain: f32,
}

pub struct Limiter {
    sr: f32,
    lookahead: usize,
    max_lookahead: usize,
    ch: [Channel; 2],
    /// Current gain reduction in dB (>= 0), for the meters.
    pub reduction_db: f32,
}

/// 4-point Hermite interpolation between y1 and y2.
#[inline]
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}

impl Limiter {
    pub fn new(sr: f32, max_lookahead_ms: f32) -> Self {
        let max = (max_lookahead_ms * 0.001 * sr).ceil() as usize + 4;
        let ch = || Channel {
            delay: vec![0.0; max + 4],
            write: 0,
            hist: [0.0; 4],
            min: SlidingMin::new(max + 4),
            avg: BoxFilter::new(max + 4),
            gain: 1.0,
        };
        let mut l = Self { sr, lookahead: 1, max_lookahead: max, ch: [ch(), ch()], reduction_db: 0.0 };
        l.set_lookahead_ms(1.0);
        l
    }

    /// Latency in samples for a look-ahead in ms (what the host must compensate).
    pub fn latency_for(&self, ms: f32) -> usize {
        ((ms * 0.001 * self.sr).round() as usize).clamp(1, self.max_lookahead)
    }

    pub fn set_lookahead_ms(&mut self, ms: f32) {
        self.lookahead = self.latency_for(ms);
        for c in &mut self.ch {
            c.min.set_window(self.lookahead);
        }
    }

    pub fn latency(&self) -> usize {
        self.lookahead
    }

    pub fn reset(&mut self) {
        for c in &mut self.ch {
            c.delay.fill(0.0);
            c.hist = [0.0; 4];
            c.min.reset();
            c.avg.reset();
            c.gain = 1.0;
        }
        self.reduction_db = 0.0;
    }

    #[inline]
    pub fn process(&mut self, x: [f32; 2], s: &Settings) -> [f32; 2] {
        let (attack_frac, release_mult, clip) = s.style.shape();
        let ceiling = 10f32.powf(s.ceiling_db / 20.0);
        let drive = 10f32.powf(s.gain_db / 20.0);
        let true_peak = s.true_peak || s.style == Style::Safe;
        let attack_len = ((self.lookahead as f32 * attack_frac).round() as usize).clamp(1, self.lookahead);
        let release = (-1.0 / ((s.release_ms * release_mult).max(0.5) * 0.001 * self.sr)).exp();

        // Driven (and, for aggressive styles, softly pre-clipped) input.
        let mut driven = [x[0] * drive, x[1] * drive];
        if clip > 0.0 {
            let knee = ceiling * 1.4;
            for v in &mut driven {
                *v += (knee * (*v / knee).tanh() - *v) * clip;
            }
        }

        // Needed gain for the sample one step back (y2): the peak estimate
        // looks one sample ahead so it can see the interval after it too.
        let mut need = [1.0f32; 2];
        for (k, (c, &v)) in self.ch.iter_mut().zip(&driven).enumerate() {
            c.hist = [c.hist[1], c.hist[2], c.hist[3], v];
            let [y0, y1, y2, y3] = c.hist;
            let mut peak = y2.abs();
            if true_peak {
                // Inter-sample peaks on both sides of y2 (4x oversampled estimate).
                for t in [0.25, 0.5, 0.75] {
                    peak = peak.max(hermite(y0, y1, y2, y3, t).abs());
                    peak = peak.max(hermite(y1, y2, y3, y3, t).abs());
                }
            }
            need[k] = if peak > ceiling { ceiling / peak } else { 1.0 };
        }
        // Linking: blend each channel towards the common (smaller) gain; still safe.
        let linked = need[0].min(need[1]);
        let link = s.link.clamp(0.0, 1.0);

        let n = self.lookahead;
        let mut out = [0.0f32; 2];
        let mut min_gain = 1.0f32;
        for (k, c) in self.ch.iter_mut().enumerate() {
            let target = need[k] + (linked - need[k]) * link;
            c.min.set_window(n);
            c.avg.set_len(attack_len);
            let held = c.min.push(target);
            let smooth = c.avg.push(held);
            // Instant down, released up.
            c.gain = if smooth < c.gain { smooth } else { smooth + release * (c.gain - smooth) };

            // The delayed sample: the one whose gain need entered the window n samples ago.
            let len = c.delay.len();
            c.delay[c.write] = c.hist[2];
            let read = (c.write + len - (n - 1)) % len;
            out[k] = c.delay[read] * c.gain;
            c.write = (c.write + 1) % len;
            min_gain = min_gain.min(c.gain);
        }
        self.reduction_db = -20.0 * min_gain.max(1e-6).log10();
        out
    }
}

/// Approximate BS.1770 loudness meter (K-weighting + mean square).
pub struct Loudness {
    sr: f32,
    k: [[Biquad; 2]; 2],
    momentary: Window,
    short: Window,
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    s: [f64; 2],
}

impl Biquad {
    #[inline]
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.s[0];
        self.s[0] = self.b[1] * x - self.a[0] * y + self.s[1];
        self.s[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Mean-square over a sliding window, decimated into 100 ms blocks.
struct Window {
    blocks: VecDeque<f64>,
    count: usize,
    acc: f64,
    n: usize,
    block_len: usize,
}

impl Window {
    fn new(sr: f32, seconds: f32) -> Self {
        let block_len = (sr * 0.1) as usize;
        Self { blocks: VecDeque::new(), count: (seconds * 10.0) as usize, acc: 0.0, n: 0, block_len }
    }

    fn push(&mut self, ms: f64) {
        self.acc += ms;
        self.n += 1;
        if self.n >= self.block_len {
            self.blocks.push_back(self.acc / self.n as f64);
            if self.blocks.len() > self.count {
                self.blocks.pop_front();
            }
            self.acc = 0.0;
            self.n = 0;
        }
    }

    fn lufs(&self) -> f32 {
        if self.blocks.is_empty() {
            return -70.0;
        }
        let mean = self.blocks.iter().sum::<f64>() / self.blocks.len() as f64;
        (-0.691 + 10.0 * mean.max(1e-12).log10()) as f32
    }
}

impl Loudness {
    pub fn new(sr: f32) -> Self {
        let fs = sr as f64;
        // K-weighting after Brecht De Man's formulation, which reproduces the
        // BS.1770 48 kHz coefficients exactly and re-derives them for other rates.
        // Stage 1: high shelf (+4 dB above ~1.7 kHz). Stage 2: high pass ~38 Hz.
        let shelf = {
            let (fc, gain_db, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
            let k = (PI * fc / fs).tan();
            let vh = 10f64.powf(gain_db / 20.0);
            let vb = vh.powf(0.4996667741545416);
            let a0 = 1.0 + k / q + k * k;
            Biquad {
                b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
                a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
                s: [0.0; 2],
            }
        };
        let hp = {
            let (fc, q) = (38.13547087602444, 0.5003270373238773);
            let k = (PI * fc / fs).tan();
            let a0 = 1.0 + k / q + k * k;
            Biquad {
                b: [1.0, -2.0, 1.0],
                a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
                s: [0.0; 2],
            }
        };
        Self { sr, k: [[shelf, hp], [shelf, hp]], momentary: Window::new(sr, 0.4), short: Window::new(sr, 3.0) }
    }

    #[inline]
    pub fn push(&mut self, x: [f32; 2]) {
        let mut sum = 0.0;
        for (ch, &v) in self.k.iter_mut().zip(&x) {
            let shelved = ch[0].run(v as f64);
            let y = ch[1].run(shelved);
            sum += y * y;
        }
        self.momentary.push(sum);
        self.short.push(sum);
    }

    pub fn momentary(&self) -> f32 {
        self.momentary.lufs()
    }

    pub fn short_term(&self) -> f32 {
        self.short.lufs()
    }

    pub fn sample_rate(&self) -> f32 {
        self.sr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn settings() -> Settings {
        Settings { gain_db: 12.0, ceiling_db: -1.0, release_ms: 50.0, link: 1.0, style: Style::Transparent, true_peak: false }
    }

    struct Noise(u32);
    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        }
    }

    #[test]
    fn never_exceeds_the_ceiling() {
        for style in [Style::Transparent, Style::Punchy, Style::Aggressive, Style::Safe] {
            let sr = 48_000.0;
            let mut l = Limiter::new(sr, 10.0);
            l.set_lookahead_ms(2.0);
            let s = Settings { style, ..settings() };
            let ceiling = 10f32.powf(-1.0 / 20.0);
            let mut rng = Noise(3);
            for n in 0..(sr as usize * 2) {
                // Bursty material: quiet noise with loud hits.
                let hit = if n % 7000 < 300 { 1.0 } else { 0.1 };
                let v = rng.next() * hit;
                let y = l.process([v, -v * 0.7], &s);
                assert!(y[0].abs() <= ceiling * 1.0001 && y[1].abs() <= ceiling * 1.0001, "{style:?} sample {n}: {y:?}");
            }
        }
    }

    #[test]
    fn quiet_signal_passes_unchanged_after_the_latency() {
        let sr = 48_000.0;
        let mut l = Limiter::new(sr, 10.0);
        l.set_lookahead_ms(1.5);
        let s = Settings { gain_db: 0.0, ..settings() };
        let lat = l.latency();
        let input: Vec<f32> = (0..4000).map(|n| (TAU * 440.0 * n as f32 / sr).sin() * 0.3).collect();
        let out: Vec<f32> = input.iter().map(|&v| l.process([v, v], &s)[0]).collect();
        for n in lat + 10..4000 {
            assert!((out[n] - input[n - lat]).abs() < 1e-6, "sample {n}");
        }
    }

    #[test]
    fn true_peak_holds_inter_sample_peaks_near_the_ceiling() {
        let sr = 48_000.0;
        let mut l = Limiter::new(sr, 10.0);
        l.set_lookahead_ms(2.0);
        let s = Settings { true_peak: true, gain_db: 6.0, ..settings() };
        let out: Vec<f32> = (0..20_000).map(|n| l.process([(TAU * 11_025.0 * n as f32 / sr + 0.4).sin(), 0.0], &s)[0]).collect();
        // Reconstruct the waveform at 4x and look at the true peak.
        let mut tp = 0.0f32;
        for w in out[2000..].windows(4) {
            for t in [0.0, 0.25, 0.5, 0.75] {
                tp = tp.max(hermite(w[0], w[1], w[2], w[3], t).abs());
            }
        }
        let tp_db = 20.0 * tp.log10();
        assert!(tp_db < -0.8, "true peak {tp_db:.2} dBTP");
    }

    #[test]
    fn loudness_of_a_sine_matches_the_standard() {
        // BS.1770: a 0 dBFS 1 kHz sine in ONE channel reads -3.01 LUFS, so a
        // -20 dBFS sine in both channels reads -20.0 LUFS.
        let sr = 48_000.0;
        let mut m = Loudness::new(sr);
        for n in 0..(sr as usize * 4) {
            let v = (TAU * 1000.0 * n as f32 / sr).sin() * 0.1;
            m.push([v, v]);
        }
        let k = &m.k[0];
        let mag = |b: &Biquad, f: f64| {
            let w = 2.0 * PI * f / 48_000.0;
            let z1 = (w.cos(), -w.sin());
            let z2 = ((2.0 * w).cos(), -(2.0 * w).sin());
            let num = (b.b[0] + b.b[1] * z1.0 + b.b[2] * z2.0, b.b[1] * z1.1 + b.b[2] * z2.1);
            let den = (1.0 + b.a[0] * z1.0 + b.a[1] * z2.0, b.a[0] * z1.1 + b.a[1] * z2.1);
            ((num.0 * num.0 + num.1 * num.1) / (den.0 * den.0 + den.1 * den.1)).sqrt()
        };
        let k_db = 20.0 * (mag(&k[0], 1000.0) * mag(&k[1], 1000.0)).log10();
        assert!((k_db - 0.691).abs() < 0.05, "K-weighting at 1 kHz: {k_db:.3} dB");
        assert!((m.short_term() + 20.0).abs() < 0.3, "{}", m.short_term());
        assert!((m.momentary() + 20.0).abs() < 0.3, "{}", m.momentary());
    }
}
