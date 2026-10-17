//! Spectrum analyzer. The audio thread only appends samples to lock-free
//! rings; the editor runs the FFT.

use std::sync::atomic::{AtomicI32, AtomicU32, AtomicUsize, Ordering::Relaxed};

pub const FFT_SIZE: usize = 4096;
const RING: usize = FFT_SIZE * 2;

pub struct Shared {
    pre: Vec<AtomicU32>,
    post: Vec<AtomicU32>,
    write: AtomicUsize,
    sample_rate: AtomicU32,
    /// Band being auditioned, or -1. Written by the editor.
    pub solo: AtomicI32,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            pre: (0..RING).map(|_| AtomicU32::new(0)).collect(),
            post: (0..RING).map(|_| AtomicU32::new(0)).collect(),
            write: AtomicUsize::new(0),
            sample_rate: AtomicU32::new(48_000f32.to_bits()),
            solo: AtomicI32::new(-1),
        }
    }
}

impl Shared {
    pub fn set_sample_rate(&self, sr: f32) {
        self.sample_rate.store(sr.to_bits(), Relaxed);
    }

    pub fn sample_rate(&self) -> f32 {
        f32::from_bits(self.sample_rate.load(Relaxed))
    }

    /// Audio thread: append one mono sample of input and output.
    #[inline]
    pub fn push(&self, pre: f32, post: f32) {
        let i = self.write.load(Relaxed);
        self.pre[i % RING].store(pre.to_bits(), Relaxed);
        self.post[i % RING].store(post.to_bits(), Relaxed);
        self.write.store(i.wrapping_add(1), Relaxed);
    }

    pub fn solo(&self) -> Option<usize> {
        usize::try_from(self.solo.load(Relaxed)).ok()
    }

    /// Editor: copies the newest FFT_SIZE samples of both rings.
    fn snapshot(&self, pre: &mut [f32], post: &mut [f32]) {
        let end = self.write.load(Relaxed);
        for k in 0..FFT_SIZE {
            let i = end.wrapping_sub(FFT_SIZE - k) % RING;
            pre[k] = f32::from_bits(self.pre[i].load(Relaxed));
            post[k] = f32::from_bits(self.post[i].load(Relaxed));
        }
    }
}

/// Editor-side analyzer state: FFT scratch and smoothed curves.
pub struct Analyzer {
    window: Vec<f32>,
    window_gain: f32,
    raw_pre: Vec<f32>,
    raw_post: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Smoothed dB per display column.
    pub pre_db: Vec<f32>,
    pub post_db: Vec<f32>,
}

impl Analyzer {
    pub fn new(columns: usize) -> Self {
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|n| 0.5 - 0.5 * (std::f32::consts::TAU * n as f32 / FFT_SIZE as f32).cos())
            .collect();
        let window_gain = window.iter().sum::<f32>();
        Self {
            window,
            window_gain,
            raw_pre: vec![0.0; FFT_SIZE],
            raw_post: vec![0.0; FFT_SIZE],
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            pre_db: vec![-120.0; columns],
            post_db: vec![-120.0; columns],
        }
    }

    /// Recomputes both spectra. `freqs` gives the centre frequency of each display column.
    pub fn update(&mut self, shared: &Shared, freqs: &[f32], dt: f32) {
        shared.snapshot(&mut self.raw_pre, &mut self.raw_post);
        let sr = shared.sample_rate();
        let raw_pre = std::mem::take(&mut self.raw_pre);
        let raw_post = std::mem::take(&mut self.raw_post);
        let mut pre_db = std::mem::take(&mut self.pre_db);
        let mut post_db = std::mem::take(&mut self.post_db);
        self.spectrum_into(&raw_pre, freqs, sr, dt, &mut pre_db);
        self.spectrum_into(&raw_post, freqs, sr, dt, &mut post_db);
        self.raw_pre = raw_pre;
        self.raw_post = raw_post;
        self.pre_db = pre_db;
        self.post_db = post_db;
    }

    fn spectrum_into(&mut self, input: &[f32], freqs: &[f32], sr: f32, dt: f32, out: &mut [f32]) {
        for k in 0..FFT_SIZE {
            self.re[k] = input[k] * self.window[k];
            self.im[k] = 0.0;
        }
        fft(&mut self.re, &mut self.im);

        let bin_hz = sr / FFT_SIZE as f32;
        let scale = 2.0 / self.window_gain;
        let fall = 40.0 * dt; // dB per frame of release
        let attack = 1.0 - (-dt * 25.0).exp();
        for (col, &f) in freqs.iter().enumerate() {
            // Peak of the bins under this column (+/- 1/24 octave): tones read
            // at their true level and the top end is not a jagged comb.
            let lo = ((f * 0.971 / bin_hz).floor() as usize).max(1);
            let hi = ((f * 1.030 / bin_hz).ceil() as usize).clamp(lo + 1, FFT_SIZE / 2);
            let mut power = 0.0f32;
            for b in lo..hi {
                power = power.max(self.re[b] * self.re[b] + self.im[b] * self.im[b]);
            }
            let mag = power.sqrt() * scale;
            // +4.5 dB/oct tilt around 1 kHz, like most mixing analyzers.
            let tilt = 4.5 * (f / 1000.0).log2();
            let db = 20.0 * mag.max(1e-9).log10() + tilt;

            let prev = out[col];
            out[col] = if db > prev { prev + (db - prev) * attack } else { (prev - fall).max(db) };
        }
    }
}

/// In-place iterative radix-2 FFT.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f32::consts::TAU / len as f32;
        let (w_im, w_re) = ang.sin_cos();
        for start in (0..n).step_by(len) {
            let (mut c_re, mut c_im) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let a = start + k;
                let b = a + len / 2;
                let t_re = re[b] * c_re - im[b] * c_im;
                let t_im = re[b] * c_im + im[b] * c_re;
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
                let next = c_re * w_re - c_im * w_im;
                c_im = c_re * w_im + c_im * w_re;
                c_re = next;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_sine_reads_near_zero_db() {
        let shared = Shared::default();
        let sr = 48_000.0;
        for n in 0..FFT_SIZE * 2 {
            let x = (std::f32::consts::TAU * 1000.0 * n as f32 / sr).sin();
            shared.push(x, x);
        }
        let mut a = Analyzer::new(1);
        // Let the smoothing settle.
        for _ in 0..200 {
            a.update(&shared, &[1000.0], 1.0 / 60.0);
        }
        // Hann scalloping costs at most ~1.4 dB; the tilt is 0 dB at 1 kHz.
        assert!(a.post_db[0] < 0.5 && a.post_db[0] > -2.0, "{}", a.post_db[0]);
    }
}
