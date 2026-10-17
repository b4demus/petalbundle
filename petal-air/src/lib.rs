use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{Air, Settings};

#[derive(Params)]
pub struct AirParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "mid_air"]
    pub mid_air: FloatParam,
    #[id = "high_air"]
    pub high_air: FloatParam,
    #[id = "trim"]
    pub trim: FloatParam,
}

impl Default for AirParams {
    fn default() -> Self {
        let amount = |name: &str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_unit(" %")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        Self {
            editor_state: editor::default_state(),
            mid_air: amount("Mid Air", 0.3),
            high_air: amount("High Air", 0.4),
            trim: FloatParam::new("Trim", 0.0, FloatRange::Linear { min: -12.0, max: 12.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_step_size(0.1)
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }
}

/// Live values for the editor.
#[derive(Default)]
pub struct Shared {
    pub mid_db: AtomicU32,
    pub high_db: AtomicU32,
    pub added: AtomicU32,
    pub level: AtomicU32,
}

impl Shared {
    pub fn load(a: &AtomicU32) -> f32 {
        f32::from_bits(a.load(Relaxed))
    }
}

pub struct PetalAir {
    params: Arc<AirParams>,
    shared: Arc<Shared>,
    air: Air,
    #[cfg(feature = "standalone")]
    demo: demo::Vocalish,
}

impl Default for PetalAir {
    fn default() -> Self {
        Self {
            params: Arc::new(AirParams::default()),
            shared: Arc::new(Shared::default()),
            air: Air::new(48_000.0),
            #[cfg(feature = "standalone")]
            demo: demo::Vocalish::new(48_000.0),
        }
    }
}

impl Plugin for PetalAir {
    const NAME: &'static str = "Petal Air";
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
        self.air = Air::new(buffer_config.sample_rate);
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Vocalish::new(buffer_config.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        self.air.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let mut level = 0.0f32;
        for mut frame in buffer.iter_samples() {
            let s = Settings { mid_air: p.mid_air.smoothed.next(), high_air: p.high_air.smoothed.next() };
            let trim = 10f32.powf(p.trim.smoothed.next() / 20.0);
            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let mut x = [*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap()];
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                x = [x[0] + d, x[1] + d];
            }
            let y = self.air.process(x, &s);
            *frame.get_mut(0).unwrap() = y[0] * trim;
            *frame.get_mut(1).unwrap() = y[1] * trim;
            level = level.max(x[0].abs()).max(x[1].abs());
        }
        let m = self.air.take_meter();
        self.shared.mid_db.store(m.mid_db.to_bits(), Relaxed);
        self.shared.high_db.store(m.high_db.to_bits(), Relaxed);
        self.shared.added.store(m.added.to_bits(), Relaxed);
        self.shared.level.store(level.to_bits(), Relaxed);
        ProcessStatus::Normal
    }
}

impl ClapPlugin for PetalAir {
    const CLAP_ID: &'static str = "com.b4demus.petal-air";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Dynamic high-frequency enhancer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::Equalizer];
}

impl Vst3Plugin for PetalAir {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalAir0001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Eq, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalAir);
nih_export_vst3!(PetalAir);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: a breathy "vocal" with sibilants.
    pub struct Vocalish {
        sr: f32,
        t: u64,
        phase: f32,
        rng: u32,
        hp: f32,
    }

    impl Vocalish {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0, phase: 0.0, rng: 13, hp: 0.0 }
        }

        pub fn next(&mut self) -> f32 {
            let t = self.t as f32 / self.sr;
            self.t += 1;
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let noise = (self.rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
            self.hp = noise + 0.6 * (self.hp - noise);
            let hiss = noise - self.hp;
            let f = [196.0, 220.0, 246.94, 220.0][(t / 0.7) as usize % 4];
            self.phase = (self.phase + f / self.sr) % 1.0;
            let k = t % 0.7;
            let vowel = (2.0 * self.phase - 1.0) * (k * 12.0).min(1.0) * (-k * 1.5).exp() * 0.25;
            // A short "s" at the end of every phrase.
            let s = if k > 0.55 { hiss * 0.12 } else { hiss * 0.01 };
            vowel + s
        }
    }
}
