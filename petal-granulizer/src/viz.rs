//! Lock-free snapshot of the grain engine, written by the audio thread once
//! per block and read by the editor every frame.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

use crate::engine::{Engine, GrainView, MAX_GRAINS};

#[derive(Default)]
struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Relaxed);
    }
    fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Relaxed))
    }
}

#[derive(Default)]
struct Slot {
    /// 0 = empty slot.
    id: AtomicU32,
    reverse: AtomicBool,
    delay_s: AtomicF32,
    size_s: AtomicF32,
    env: AtomicF32,
    semitones: AtomicF32,
    pan: AtomicF32,
}

pub struct VizShared {
    slots: [Slot; MAX_GRAINS],
    write_pos: AtomicU64,
    sample_rate: AtomicF32,
    /// Peak input level since the editor last read it, as f32 bits.
    peak: AtomicU32,
}

impl Default for VizShared {
    fn default() -> Self {
        let shared = Self {
            slots: std::array::from_fn(|_| Slot::default()),
            write_pos: AtomicU64::new(0),
            sample_rate: AtomicF32::default(),
            peak: AtomicU32::new(0),
        };
        shared.sample_rate.store(48_000.0);
        shared
    }
}

impl VizShared {
    /// Audio thread: publishes the engine state at the end of a block.
    pub fn publish(&self, engine: &Engine, block_peak: f32) {
        let mut live = [false; MAX_GRAINS];
        for g in engine.grain_views() {
            let s = &self.slots[g.slot];
            s.delay_s.store(g.delay_s);
            s.size_s.store(g.size_s);
            s.env.store(g.env);
            s.semitones.store(g.semitones);
            s.pan.store(g.pan);
            s.reverse.store(g.reverse, Relaxed);
            s.id.store(g.id, Relaxed);
            live[g.slot] = true;
        }
        for (slot, alive) in self.slots.iter().zip(live) {
            if !alive {
                slot.id.store(0, Relaxed);
            }
        }
        self.write_pos.store(engine.write_pos(), Relaxed);
        self.sample_rate.store(engine.sample_rate());
        // Positive floats order the same as their bit patterns.
        self.peak.fetch_max(block_peak.abs().to_bits(), Relaxed);
    }

    /// Editor: current grains.
    pub fn grains(&self) -> impl Iterator<Item = GrainView> + '_ {
        self.slots.iter().enumerate().filter_map(|(slot, s)| {
            let id = s.id.load(Relaxed);
            (id != 0).then(|| GrainView {
                slot,
                id,
                delay_s: s.delay_s.load(),
                size_s: s.size_s.load(),
                env: s.env.load(),
                semitones: s.semitones.load(),
                pan: s.pan.load(),
                reverse: s.reverse.load(Relaxed),
            })
        })
    }

    /// Editor: write head position in seconds (stops while frozen).
    pub fn now_s(&self) -> f64 {
        self.write_pos.load(Relaxed) as f64 / self.sample_rate.load().max(1.0) as f64
    }

    /// Editor: input peak since the previous call.
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Relaxed))
    }
}
