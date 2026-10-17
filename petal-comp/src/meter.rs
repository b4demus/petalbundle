//! Level history for the editor: the audio thread appends one frame of
//! (input peak, output peak, gain reduction) every few milliseconds.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};

/// Frames per second of history.
pub const RATE: f32 = 120.0;
const RING: usize = 2048;

#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    pub input_db: f32,
    pub output_db: f32,
    pub reduction_db: f32,
}

pub struct Shared {
    input: Vec<AtomicU32>,
    output: Vec<AtomicU32>,
    reduction: Vec<AtomicU32>,
    count: AtomicUsize,
}

impl Default for Shared {
    fn default() -> Self {
        let ring = || (0..RING).map(|_| AtomicU32::new((-120f32).to_bits())).collect();
        Self {
            input: ring(),
            output: ring(),
            reduction: (0..RING).map(|_| AtomicU32::new(0)).collect(),
            count: AtomicUsize::new(0),
        }
    }
}

impl Shared {
    pub fn push(&self, f: Frame) {
        let i = self.count.load(Relaxed);
        self.input[i % RING].store(f.input_db.to_bits(), Relaxed);
        self.output[i % RING].store(f.output_db.to_bits(), Relaxed);
        self.reduction[i % RING].store(f.reduction_db.to_bits(), Relaxed);
        self.count.store(i.wrapping_add(1), Relaxed);
    }

    /// Total frames pushed so far (lets the editor scroll smoothly).
    pub fn count(&self) -> usize {
        self.count.load(Relaxed)
    }

    /// The newest `out.len()` frames, oldest first.
    pub fn latest(&self, out: &mut [Frame]) {
        let end = self.count();
        let n = out.len().min(RING);
        for (k, slot) in out.iter_mut().enumerate().take(n) {
            let i = end.wrapping_sub(n - k) % RING;
            *slot = Frame {
                input_db: f32::from_bits(self.input[i].load(Relaxed)),
                output_db: f32::from_bits(self.output[i].load(Relaxed)),
                reduction_db: f32::from_bits(self.reduction[i].load(Relaxed)),
            };
        }
    }
}

/// Audio-thread accumulator that turns samples into history frames.
pub struct Accumulator {
    step: usize,
    n: usize,
    input: f32,
    output: f32,
    reduction: f32,
}

impl Accumulator {
    pub fn new(sample_rate: f32) -> Self {
        Self { step: (sample_rate / RATE).round().max(1.0) as usize, n: 0, input: 0.0, output: 0.0, reduction: 0.0 }
    }

    #[inline]
    pub fn add(&mut self, shared: &Shared, input: f32, output: f32, reduction_db: f32) {
        self.input = self.input.max(input);
        self.output = self.output.max(output);
        self.reduction = self.reduction.max(reduction_db);
        self.n += 1;
        if self.n >= self.step {
            let db = |x: f32| 20.0 * x.max(1e-6).log10();
            shared.push(Frame {
                input_db: db(self.input),
                output_db: db(self.output),
                reduction_db: self.reduction,
            });
            *self = Self { step: self.step, n: 0, input: 0.0, output: 0.0, reduction: 0.0 };
        }
    }
}
