//! Built-in test signal for the standalone preview: soft random plucks.

pub struct Plucks {
    sr: f32,
    t: u64,
    next: u64,
    voices: [(f32, f32, f32); 6], // (phase, freq, amp)
    rng: u32,
}

impl Plucks {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            t: 0,
            next: 0,
            voices: [(0.0, 0.0, 0.0); 6],
            rng: 7,
        }
    }

    pub fn next(&mut self) -> f32 {
        const NOTES: [i32; 8] = [57, 60, 62, 64, 67, 69, 72, 76];
        if self.t >= self.next {
            self.rng = self.rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let note = NOTES[(self.rng >> 16) as usize % NOTES.len()];
            let slot = (self.t / 7) as usize % self.voices.len();
            let f = 440.0 * 2f32.powf((note - 69) as f32 / 12.0);
            self.voices[slot] = (0.0, f, 0.22);
            self.next = self.t + (self.sr * (0.35 + ((self.rng >> 8) & 3) as f32 * 0.15)) as u64;
        }
        self.t += 1;
        let decay = (-3.0 / self.sr).exp();
        let mut out = 0.0;
        for (ph, f, amp) in &mut self.voices {
            if *amp > 1e-4 {
                *ph += *f / self.sr * std::f32::consts::TAU;
                out += (ph.sin() + 0.3 * (2.0 * *ph).sin()) * *amp;
                *amp *= decay;
            }
        }
        out
    }
}
