use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{Reverb, Settings};

#[derive(Params)]
pub struct VerbParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "predelay"]
    pub predelay: FloatParam,
    #[id = "size"]
    pub size: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "mod"]
    pub modulation: FloatParam,
    #[id = "damp"]
    pub damp: FloatParam,
    #[id = "low_cut"]
    pub low_cut: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "dry"]
    pub dry: FloatParam,
    #[id = "wet"]
    pub wet: FloatParam,
    #[id = "freeze"]
    pub freeze: BoolParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(30.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

fn hz(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) })
        .with_smoother(SmoothingStyle::Logarithmic(30.0))
        .with_unit(" Hz")
        .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
        .with_string_to_value(formatters::s2v_f32_hz_then_khz())
}

impl Default for VerbParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            predelay: FloatParam::new(
                "Predelay",
                20.0,
                FloatRange::Skewed { min: 0.0, max: dsp::MAX_PREDELAY_MS, factor: FloatRange::skew_factor(-1.5) },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            size: percent("Size", 0.6),
            decay: FloatParam::new(
                "Decay",
                2.5,
                FloatRange::Skewed { min: 0.2, max: 20.0, factor: FloatRange::skew_factor(-1.8) },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            modulation: percent("Mod", 0.3),
            damp: hz("Damp", 7000.0, 1000.0, 20_000.0),
            low_cut: hz("Low Cut", 120.0, 20.0, 1000.0),
            width: percent("Width", 1.0),
            dry: percent("Dry", 1.0),
            wet: percent("Wet", 0.35),
            freeze: BoolParam::new("Freeze", false),
        }
    }
}

/// Values the editor needs from the audio thread (block peaks).
#[derive(Default)]
pub struct Shared {
    pub input: AtomicU32,
    pub wet: AtomicU32,
}

impl Shared {
    pub fn load(a: &AtomicU32) -> f32 {
        f32::from_bits(a.load(Relaxed))
    }
}

pub struct PetalVerb {
    params: Arc<VerbParams>,
    shared: Arc<Shared>,
    reverb: Reverb,
    #[cfg(feature = "standalone")]
    demo: demo::Chimes,
}

impl Default for PetalVerb {
    fn default() -> Self {
        Self {
            params: Arc::new(VerbParams::default()),
            shared: Arc::new(Shared::default()),
            reverb: Reverb::new(48_000.0),
            #[cfg(feature = "standalone")]
            demo: demo::Chimes::new(48_000.0),
        }
    }
}

impl Plugin for PetalVerb {
    const NAME: &'static str = "Petal Verb";
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
        editor::create(self.params.clone(), self.shared.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.reverb = Reverb::new(buffer_config.sample_rate);
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Chimes::new(buffer_config.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        self.reverb.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let freeze = p.freeze.value();
        let (mut in_peak, mut wet_peak) = (0.0f32, 0.0f32);

        for mut frame in buffer.iter_samples() {
            let settings = Settings {
                predelay_ms: p.predelay.smoothed.next(),
                size: p.size.smoothed.next(),
                decay_s: p.decay.smoothed.next(),
                modulation: p.modulation.smoothed.next(),
                damp_hz: p.damp.smoothed.next(),
                low_cut_hz: p.low_cut.smoothed.next(),
                width: p.width.smoothed.next(),
                freeze,
            };
            let dry = p.dry.smoothed.next();
            let wet_gain = p.wet.smoothed.next();

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let (mut l, mut r) = (*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap());
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                l += d;
                r += d;
            }

            let wet = self.reverb.process([l, r], &settings);
            *frame.get_mut(0).unwrap() = l * dry + wet[0] * wet_gain;
            *frame.get_mut(1).unwrap() = r * dry + wet[1] * wet_gain;
            in_peak = in_peak.max(l.abs()).max(r.abs());
            wet_peak = wet_peak.max(wet[0].abs()).max(wet[1].abs());
        }
        self.shared.input.store(in_peak.to_bits(), Relaxed);
        self.shared.wet.store(wet_peak.to_bits(), Relaxed);
        // The tail keeps ringing after the input stops.
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for PetalVerb {
    const CLAP_ID: &'static str = "com.b4demus.petal-verb";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Algorithmic stereo reverb");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::Reverb];
}

impl Vst3Plugin for PetalVerb {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalVerb001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Reverb, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalVerb);
nih_export_vst3!(PetalVerb);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: sparse bell-like chimes.
    pub struct Chimes {
        sr: f32,
        t: u64,
    }

    impl Chimes {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0 }
        }

        pub fn next(&mut self) -> f32 {
            let t = self.t as f32 / self.sr;
            self.t += 1;
            let period = 1.4;
            let k = t % period;
            let n = (t / period) as usize;
            let f = [880.0, 1174.66, 987.77, 1318.51][n % 4];
            let env = (-k * 6.0).exp() * (1.0 - (-k * 900.0).exp());
            let ph = std::f32::consts::TAU * f * k;
            (ph.sin() + 0.4 * (2.76 * ph).sin()) * env * 0.3
        }
    }
}
