//! Offline check of the engine without a host:
//!   cargo run --release --example render -- [input.wav] [output.wav]
//! Without an input it synthesizes a short plucked-chord phrase.

use petal_granulizer::engine::{Engine, GrainSettings};

const SR: u32 = 48_000;

fn test_input() -> Vec<(f32, f32)> {
    // A few decaying plucks (A minor-ish), roughly piano/kalimba-like.
    let notes = [
        (0.0, 57),
        (0.5, 60),
        (1.0, 64),
        (1.5, 69),
        (2.5, 67),
        (3.0, 64),
    ];
    let len = SR as usize * 5;
    let mut out = vec![(0.0, 0.0); len];
    for &(start, midi) in &notes {
        let f = 440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0);
        let s0 = (start * SR as f32) as usize;
        for n in 0..(SR as usize * 2).min(len - s0) {
            let t = n as f32 / SR as f32;
            let env = (-t * 3.0).exp() * (1.0 - (-t * 400.0).exp());
            let ph = std::f32::consts::TAU * f * t;
            let x = (ph.sin() + 0.3 * (2.0 * ph).sin() + 0.1 * (3.0 * ph).sin()) * env * 0.25;
            out[s0 + n].0 += x;
            out[s0 + n].1 += x;
        }
    }
    out
}

fn read_wav(path: &str) -> (Vec<(f32, f32)>, u32) {
    let mut r = hound::WavReader::open(path).expect("open input");
    let spec = r.spec();
    let scale = match spec.sample_format {
        hound::SampleFormat::Float => 1.0,
        hound::SampleFormat::Int => 1.0 / (1u32 << (spec.bits_per_sample - 1)) as f32,
    };
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => r
            .samples::<i32>()
            .map(|s| s.unwrap() as f32 * scale)
            .collect(),
    };
    let ch = spec.channels as usize;
    let frames = samples
        .chunks(ch)
        .map(|c| (c[0], *c.get(1).unwrap_or(&c[0])))
        .collect();
    (frames, spec.sample_rate)
}

fn write_wav(path: &str, frames: &[(f32, f32)], sr: u32) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: sr,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for &(l, r) in frames {
        w.write_sample(l).unwrap();
        w.write_sample(r).unwrap();
    }
    w.finalize().unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, sr) = match args.first() {
        Some(p) => read_wav(p),
        None => {
            let input = test_input();
            write_wav("render_in.wav", &input, SR);
            (input, SR)
        }
    };
    let out_path = args.get(1).map(String::as_str).unwrap_or("render_out.wav");

    let mut engine = Engine::new(sr as f32);
    let settings = GrainSettings::default();
    let mix = 0.6;
    let tail = sr as usize * 4;

    let mut peak: f32 = 0.0;
    let mut sum_sq = 0.0f64;
    let out: Vec<(f32, f32)> = input
        .iter()
        .copied()
        .chain(std::iter::repeat((0.0, 0.0)).take(tail))
        .map(|(l, r)| {
            let (wl, wr) = engine.process(l, r, &settings);
            let o = (l * (1.0 - mix) + wl * mix, r * (1.0 - mix) + wr * mix);
            peak = peak.max(o.0.abs()).max(o.1.abs());
            sum_sq += (o.0 * o.0 + o.1 * o.1) as f64 * 0.5;
            o
        })
        .collect();

    write_wav(out_path, &out, sr);
    let rms = (sum_sq / out.len() as f64).sqrt();
    println!(
        "wrote {out_path}: {:.2}s, peak {:.1} dBFS, rms {:.1} dBFS",
        out.len() as f32 / sr as f32,
        20.0 * peak.log10(),
        20.0 * rms.log10()
    );
}
