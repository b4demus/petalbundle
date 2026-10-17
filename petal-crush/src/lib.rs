use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{Crusher, Settings};

fn tenths(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 10.0 })
        .with_smoother(SmoothingStyle::Linear(30.0))
        .with_step_size(0.1)
        .with_value_to_string(formatters::v2s_f32_rounded(1))
}

#[derive(Params)]
pub struct CrushParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "crush"]
    pub crush: FloatParam,
    #[id = "darkness"]
    pub darkness: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "shut_up"]
    pub shut_up: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for CrushParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            crush: tenths("Crush", 4.0),
            darkness: tenths("Darkness", 3.0),
            release: FloatParam::new(
                "Release",
                120.0,
                FloatRange::Skewed { min: 10.0, max: 1000.0, factor: FloatRange::skew_factor(-1.6) },
            )
            .with_smoother(SmoothingStyle::Logarithmic(30.0))
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            shut_up: tenths("Shut Up", 0.0),
            mix: FloatParam::new("Mix", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_unit(" %")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            output: FloatParam::new("Output", 0.0, FloatRange::Linear { min: -24.0, max: 12.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_step_size(0.1)
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }
}

pub const HISTORY: usize = 512;

/// Live values for the editor: the latest meter plus a gain-reduction history.
pub struct Shared {
    pub reduction: AtomicU32,
    pub input: AtomicU32,
    pub output: AtomicU32,
    pub shut: AtomicU32,
    pub history: Vec<AtomicU32>,
    pub history_pos: AtomicUsize,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            reduction: AtomicU32::new(0),
            input: AtomicU32::new((-120f32).to_bits()),
            output: AtomicU32::new(0),
            shut: AtomicU32::new(1f32.to_bits()),
            history: (0..HISTORY).map(|_| AtomicU32::new(0)).collect(),
            history_pos: AtomicUsize::new(0),
        }
    }
}

impl Shared {
    pub fn load(a: &AtomicU32) -> f32 {
        f32::from_bits(a.load(Relaxed))
    }
}

pub struct PetalCrush {
    params: Arc<CrushParams>,
    shared: Arc<Shared>,
    crusher: Crusher,
    sample_rate: f32,
    /// Samples per history frame and the running max within it.
    frame_len: usize,
    frame_n: usize,
    frame_max: f32,
    #[cfg(feature = "standalone")]
    demo: demo::Drums,
}

impl Default for PetalCrush {
    fn default() -> Self {
        Self {
            params: Arc::new(CrushParams::default()),
            shared: Arc::new(Shared::default()),
            crusher: Crusher::new(48_000.0),
            sample_rate: 48_000.0,
            frame_len: 400,
            frame_n: 0,
            frame_max: 0.0,
            #[cfg(feature = "standalone")]
            demo: demo::Drums::new(48_000.0),
        }
    }
}

impl Plugin for PetalCrush {
    const NAME: &'static str = "Petal Crush";
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
        self.sample_rate = buffer_config.sample_rate;
        self.crusher = Crusher::new(self.sample_rate);
        // ~120 history frames per second.
        self.frame_len = (self.sample_rate / 120.0) as usize;
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Drums::new(self.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        self.crusher.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let mut out_peak = 0.0f32;
        for mut frame in buffer.iter_samples() {
            let s = Settings {
                crush: p.crush.smoothed.next(),
                darkness: p.darkness.smoothed.next(),
                release_ms: p.release.smoothed.next(),
                shut_up: p.shut_up.smoothed.next(),
                mix: p.mix.smoothed.next(),
            };
            let gain = 10f32.powf(p.output.smoothed.next() / 20.0);

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let mut x = [*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap()];
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                x = [x[0] + d, x[1] + d];
            }
            let y = self.crusher.process(x, &s);
            let out = [y[0] * gain, y[1] * gain];
            *frame.get_mut(0).unwrap() = out[0];
            *frame.get_mut(1).unwrap() = out[1];
            out_peak = out_peak.max(out[0].abs()).max(out[1].abs());

            // Gain-reduction history: the max of each short frame.
            self.frame_max = self.frame_max.max(self.crusher.meter().reduction_db);
            self.frame_n += 1;
            if self.frame_n >= self.frame_len {
                let i = self.shared.history_pos.load(Relaxed);
                self.shared.history[i % HISTORY].store(self.frame_max.to_bits(), Relaxed);
                self.shared.history_pos.store(i.wrapping_add(1), Relaxed);
                self.frame_n = 0;
                self.frame_max = 0.0;
            }
        }
        let m = self.crusher.meter();
        self.shared.reduction.store(m.reduction_db.to_bits(), Relaxed);
        self.shared.input.store(m.input_db.to_bits(), Relaxed);
        self.shared.shut.store(m.shut_gain.to_bits(), Relaxed);
        self.shared.output.store(out_peak.to_bits(), Relaxed);
        ProcessStatus::Normal
    }
}

impl ClapPlugin for PetalCrush {
    const CLAP_ID: &'static str = "com.b4demus.petal-crush";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Level crusher: heavy limiting and grit");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::Compressor, ClapFeature::Distortion];
}

impl Vst3Plugin for PetalCrush {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalCrush01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics, Vst3SubCategory::Distortion];
}

nih_export_clap!(PetalCrush);
nih_export_vst3!(PetalCrush);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: a sparse drum loop with room tails.
    pub struct Drums {
        sr: f32,
        t: u64,
        rng: u32,
    }

    impl Drums {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0, rng: 5 }
        }

        pub fn next(&mut self) -> f32 {
            let beat = 60.0 / 92.0;
            let t = self.t as f32 / self.sr;
            self.t += 1;
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let noise = (self.rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
            let pos = t % (beat * 2.0);
            let mut out = 0.0;
            let k = pos;
            if k < 0.5 {
                let f = 50.0 + 100.0 * (-k * 25.0).exp();
                out += (std::f32::consts::TAU * f * k).sin() * (-k * 5.0).exp() * 0.6;
            }
            let s = pos - beat;
            if (0.0..0.6).contains(&s) {
                out += noise * ((-s * 20.0).exp() * 0.35 + (-s * 4.0).exp() * 0.02);
            }
            out
        }
    }
}
