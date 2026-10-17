use nih_plug::prelude::*;
use std::sync::Arc;

#[cfg(feature = "standalone")]
mod demo;
mod editor;
pub mod engine;
mod viz;

use engine::{Engine, GrainSettings, PitchMode};

pub struct PetalGranulizer {
    params: Arc<GranulizerParams>,
    engine: Engine,
    viz: Arc<viz::VizShared>,
    #[cfg(feature = "standalone")]
    demo: demo::Plucks,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum PitchModeParam {
    Free,
    Octaves,
    Fifths,
    Pentatonic,
}

impl From<PitchModeParam> for PitchMode {
    fn from(p: PitchModeParam) -> Self {
        match p {
            PitchModeParam::Free => PitchMode::Free,
            PitchModeParam::Octaves => PitchMode::Octaves,
            PitchModeParam::Fifths => PitchMode::Fifths,
            PitchModeParam::Pentatonic => PitchMode::Pentatonic,
        }
    }
}

#[derive(Params)]
pub struct GranulizerParams {
    // Bumped when the window size changes so old projects don't restore the old size.
    #[persist = "editor-state-v3"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "density"]
    pub density: FloatParam,
    #[id = "jitter"]
    pub jitter: FloatParam,
    #[id = "size"]
    pub size: FloatParam,
    #[id = "position"]
    pub position: FloatParam,
    #[id = "spray"]
    pub spray: FloatParam,
    #[id = "pitch"]
    pub pitch: FloatParam,
    #[id = "pitch_rand"]
    pub pitch_rand: FloatParam,
    #[id = "pitch_mode"]
    pub pitch_mode: EnumParam<PitchModeParam>,
    #[id = "reverse"]
    pub reverse: FloatParam,
    #[id = "spread"]
    pub spread: FloatParam,
    #[id = "shape"]
    pub shape: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "freeze"]
    pub freeze: BoolParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "gain"]
    pub gain: FloatParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

fn millis(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: FloatRange::skew_factor(-1.5),
        },
    )
    .with_unit(" ms")
    .with_value_to_string(formatters::v2s_f32_rounded(0))
}

impl Default for GranulizerParams {
    fn default() -> Self {
        let d = GrainSettings::default();
        Self {
            editor_state: editor::default_state(),
            density: FloatParam::new(
                "Density",
                d.density,
                FloatRange::Skewed {
                    min: 0.5,
                    max: 100.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" /s")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            jitter: percent("Jitter", d.jitter),
            size: millis("Size", d.size_ms, 10.0, 1000.0),
            position: millis("Position", d.position_ms, 0.0, 2000.0),
            spray: millis("Spray", d.spray_ms, 0.0, 2000.0),
            pitch: FloatParam::new(
                "Pitch",
                d.pitch_st,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .with_step_size(0.01)
            .with_unit(" st")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            pitch_rand: FloatParam::new(
                "Pitch Random",
                d.pitch_rand_st,
                FloatRange::Linear {
                    min: 0.0,
                    max: 24.0,
                },
            )
            .with_unit(" st")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            pitch_mode: EnumParam::new("Pitch Mode", PitchModeParam::Octaves),
            reverse: percent("Reverse", d.reverse),
            spread: percent("Spread", d.spread),
            shape: percent("Shape", d.shape),
            feedback: FloatParam::new(
                "Feedback",
                d.feedback,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.95,
                },
            )
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            freeze: BoolParam::new("Freeze", false),
            mix: percent("Mix", 0.6).with_smoother(SmoothingStyle::Linear(30.0)),
            gain: FloatParam::new(
                "Output",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-30.0),
                    max: util::db_to_gain(12.0),
                    factor: FloatRange::gain_skew_factor(-30.0, 12.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(30.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

impl GranulizerParams {
    fn settings(&self) -> GrainSettings {
        GrainSettings {
            density: self.density.value(),
            jitter: self.jitter.value(),
            size_ms: self.size.value(),
            position_ms: self.position.value(),
            spray_ms: self.spray.value(),
            pitch_st: self.pitch.value(),
            pitch_rand_st: self.pitch_rand.value(),
            pitch_mode: self.pitch_mode.value().into(),
            reverse: self.reverse.value(),
            spread: self.spread.value(),
            shape: self.shape.value(),
            feedback: self.feedback.value(),
            freeze: self.freeze.value(),
        }
    }
}

impl Default for PetalGranulizer {
    fn default() -> Self {
        Self {
            params: Arc::new(GranulizerParams::default()),
            engine: Engine::new(48_000.0),
            viz: Arc::new(viz::VizShared::default()),
            #[cfg(feature = "standalone")]
            demo: demo::Plucks::new(48_000.0),
        }
    }
}

impl Plugin for PetalGranulizer {
    const NAME: &'static str = "Petal Granulizer";
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
        editor::create(self.params.clone(), self.viz.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.engine = Engine::new(buffer_config.sample_rate);
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Plucks::new(buffer_config.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let settings = self.params.settings();
        let mut peak = 0.0f32;

        for mut frame in buffer.iter_samples() {
            let mix = self.params.mix.smoothed.next();
            let gain = self.params.gain.smoothed.next();
            let in_l = *frame.get_mut(0).unwrap();
            let in_r = *frame.get_mut(1).unwrap();
            #[cfg(feature = "standalone")]
            let (in_l, in_r) = {
                let x = self.demo.next();
                (in_l + x, in_r + x)
            };
            peak = peak.max(in_l.abs()).max(in_r.abs());

            let (wet_l, wet_r) = self.engine.process(in_l, in_r, &settings);

            *frame.get_mut(0).unwrap() = (in_l * (1.0 - mix) + wet_l * mix) * gain;
            *frame.get_mut(1).unwrap() = (in_r * (1.0 - mix) + wet_r * mix) * gain;
        }
        self.viz.publish(&self.engine, peak);

        // Grains keep sounding after the input goes quiet (and forever when frozen).
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for PetalGranulizer {
    const CLAP_ID: &'static str = "com.b4demus.petal-granulizer";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Granular texture effect");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Granular,
        ClapFeature::Glitch,
    ];
}

impl Vst3Plugin for PetalGranulizer {
    // Kept from the plugin's first name so existing FL projects still load it.
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwBotanicaGrn1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Delay,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(PetalGranulizer);
nih_export_vst3!(PetalGranulizer);
