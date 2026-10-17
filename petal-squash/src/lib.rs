use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{Amounts, BandSettings, Crossover, Follower, BANDS};

pub const BAND_NAMES: [&str; BANDS] = ["H", "M", "L"];

#[derive(Params)]
pub struct BandParams {
    #[id = "gain"]
    pub gain: FloatParam,
    /// Upward-compression threshold ("below").
    #[id = "up"]
    pub up: FloatParam,
    /// Downward-compression threshold ("above").
    #[id = "down"]
    pub down: FloatParam,
}

fn db_param(name: String, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max })
        .with_smoother(SmoothingStyle::Linear(20.0))
        .with_step_size(0.1)
        .with_unit(" dB")
        .with_value_to_string(formatters::v2s_f32_rounded(1))
}

impl BandParams {
    fn new(b: usize) -> Self {
        let n = BAND_NAMES[b];
        Self {
            gain: db_param(format!("{n} Gain"), 0.0, -24.0, 24.0),
            up: db_param(format!("{n} Upward Threshold"), dsp::DEFAULT_UP_DB[b], -72.0, 0.0),
            down: db_param(format!("{n} Downward Threshold"), dsp::DEFAULT_DOWN_DB[b], -72.0, 0.0),
        }
    }
}

fn percent(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max })
        .with_smoother(SmoothingStyle::Linear(20.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

#[derive(Params)]
pub struct SquashParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "time"]
    pub time: FloatParam,
    #[id = "in_gain"]
    pub in_gain: FloatParam,
    #[id = "out_gain"]
    pub out_gain: FloatParam,
    #[id = "upward"]
    pub upward: FloatParam,
    #[id = "downward"]
    pub downward: FloatParam,

    /// High, mid, low.
    #[nested(array, group = "Bands")]
    pub bands: [BandParams; BANDS],
}

impl Default for SquashParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            depth: percent("Depth", 1.0, 1.0),
            time: FloatParam::new(
                "Time",
                1.0,
                FloatRange::Skewed { min: 0.0, max: 10.0, factor: FloatRange::skew_factor(-1.6) },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            in_gain: db_param("In Gain".into(), 0.0, -24.0, 24.0),
            out_gain: db_param("Out Gain".into(), 0.0, -24.0, 24.0),
            upward: percent("Upward", 1.0, 1.0),
            downward: percent("Downward", 1.0, 1.0),
            bands: std::array::from_fn(BandParams::new),
        }
    }
}

/// Per-band levels for the editor, written once per block.
#[derive(Default)]
pub struct Meters {
    pub input_db: [AtomicU32; BANDS],
    pub output_db: [AtomicU32; BANDS],
    pub gain_db: [AtomicU32; BANDS],
}

impl Meters {
    pub fn load(a: &AtomicU32) -> f32 {
        f32::from_bits(a.load(Relaxed))
    }
}

pub struct PetalSquash {
    params: Arc<SquashParams>,
    meters: Arc<Meters>,
    crossover: Crossover,
    followers: [Follower; BANDS],
    /// Fast peak followers that cap the upward boost on transients.
    peaks: [Follower; BANDS],
    #[cfg(feature = "standalone")]
    demo: demo::Loop,
}

impl Default for PetalSquash {
    fn default() -> Self {
        Self {
            params: Arc::new(SquashParams::default()),
            meters: Arc::new(Meters::default()),
            crossover: Crossover::new(48_000.0),
            followers: std::array::from_fn(|_| Follower::new(48_000.0)),
            peaks: std::array::from_fn(|_| Follower::new(48_000.0)),
            #[cfg(feature = "standalone")]
            demo: demo::Loop::new(48_000.0),
        }
    }
}

impl Plugin for PetalSquash {
    const NAME: &'static str = "Petal Squash";
    const VENDOR: &'static str = "b4demus";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.meters.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let sr = buffer_config.sample_rate;
        self.crossover = Crossover::new(sr);
        self.followers = std::array::from_fn(|_| Follower::new(sr));
        self.peaks = std::array::from_fn(|_| Follower::new(sr));
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Loop::new(sr);
        }
        true
    }

    fn reset(&mut self) {
        self.crossover.reset();
        for f in self.followers.iter_mut().chain(&mut self.peaks) {
            f.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let mut in_peak = [0.0f32; BANDS];
        let mut out_peak = [0.0f32; BANDS];
        let mut last_gain = [0.0f32; BANDS];

        for mut frame in buffer.iter_samples() {
            let depth = p.depth.smoothed.next();
            let time = p.time.smoothed.next();
            let in_gain = dsp_gain(p.in_gain.smoothed.next());
            let out_gain = dsp_gain(p.out_gain.smoothed.next());
            let amounts = Amounts { upward: p.upward.smoothed.next(), downward: p.downward.smoothed.next() };

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let (mut l, mut r) = (*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap());
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                l += d;
                r += d;
            }
            let bl = self.crossover.split(0, (l * in_gain) as f64);
            let br = self.crossover.split(1, (r * in_gain) as f64);

            let (mut wet_l, mut wet_r, mut dry_l, mut dry_r) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for b in 0..BANDS {
                let bp = &p.bands[b];
                let settings = BandSettings {
                    down_db: bp.down.smoothed.next(),
                    up_db: bp.up.smoothed.next(),
                    gain_db: bp.gain.smoothed.next(),
                    attack_ms: dsp::ATTACK_MS[b] * time,
                    release_ms: dsp::RELEASE_MS[b] * time,
                };
                let peak = bl[b].abs().max(br[b].abs()) as f32;
                let level = self.followers[b].run(peak, settings.attack_ms, settings.release_ms);
                // Instant attack; releases over the band's attack time.
                let fast = self.peaks[b].run(peak, 0.0, settings.attack_ms.max(1.0));
                let gain_db = dsp::static_gain(level, fast, &settings, &amounts);
                let g = dsp_gain(gain_db + settings.gain_db) as f64;

                wet_l += bl[b] * g;
                wet_r += br[b] * g;
                // The dry path is the un-processed band sum: same phase as the wet path.
                dry_l += bl[b];
                dry_r += br[b];

                in_peak[b] = in_peak[b].max(peak);
                out_peak[b] = out_peak[b].max(peak * g as f32);
                last_gain[b] = gain_db + settings.gain_db;
            }

            let d = depth as f64;
            let og = out_gain as f64;
            *frame.get_mut(0).unwrap() = ((wet_l * d + dry_l * (1.0 - d)) * og) as f32;
            *frame.get_mut(1).unwrap() = ((wet_r * d + dry_r * (1.0 - d)) * og) as f32;
        }

        let db = |x: f32| 20.0 * x.max(1e-6).log10();
        for b in 0..BANDS {
            self.meters.input_db[b].store(db(in_peak[b]).to_bits(), Relaxed);
            self.meters.output_db[b].store(db(out_peak[b]).to_bits(), Relaxed);
            self.meters.gain_db[b].store(last_gain[b].to_bits(), Relaxed);
        }
        ProcessStatus::Normal
    }
}

#[inline]
fn dsp_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

impl ClapPlugin for PetalSquash {
    const CLAP_ID: &'static str = "com.b4demus.petal-squash";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Three-band upward/downward compressor");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Compressor,
        ClapFeature::MultiEffects,
    ];
}

impl Vst3Plugin for PetalSquash {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalSquash1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalSquash);
nih_export_vst3!(PetalSquash);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: drums plus a soft pad.
    pub struct Loop {
        sr: f32,
        t: u64,
        rng: u32,
    }

    impl Loop {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0, rng: 3 }
        }

        pub fn next(&mut self) -> f32 {
            let beat = 60.0 / 96.0;
            let t = self.t as f32 / self.sr;
            self.t += 1;
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let noise = (self.rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
            let pos = t % (beat * 4.0);
            let mut out = 0.0;
            for kick in [0.0, beat * 2.0] {
                let k = pos - kick;
                if (0.0..0.45).contains(&k) {
                    let f = 48.0 + 110.0 * (-k * 28.0).exp();
                    out += (std::f32::consts::TAU * f * k).sin() * (-k * 6.0).exp() * 0.7;
                }
            }
            for snare in [beat, beat * 3.0] {
                let k = pos - snare;
                if (0.0..0.3).contains(&k) {
                    out += noise * (-k * 16.0).exp() * 0.35;
                }
            }
            let hat = pos % (beat / 2.0);
            if hat < 0.06 {
                out += noise * (-hat * 80.0).exp() * 0.08;
            }
            let pad: f32 = [220.0, 261.63, 329.63]
                .iter()
                .map(|f| (std::f32::consts::TAU * f * t).sin())
                .sum();
            out + pad * 0.012
        }
    }
}
