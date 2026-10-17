use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{DryDelay, Geometry, Mode, Shifter};

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum ModeParam {
    Fast,
    Medium,
    Slow,
}

impl From<ModeParam> for Mode {
    fn from(m: ModeParam) -> Self {
        match m {
            ModeParam::Fast => Mode::Fast,
            ModeParam::Medium => Mode::Medium,
            ModeParam::Slow => Mode::Slow,
        }
    }
}

#[derive(Params)]
pub struct ShiftParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "semitones"]
    pub semitones: FloatParam,
    #[id = "cents"]
    pub cents: FloatParam,
    #[id = "spread"]
    pub spread: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<ModeParam>,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "gain"]
    pub gain: FloatParam,
}

impl Default for ShiftParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            // The standalone preview starts a fifth up so the scopes show the shift.
            semitones: FloatParam::new("Semitones", if cfg!(feature = "standalone") { 7.0 } else { 0.0 }, FloatRange::Linear { min: -24.0, max: 24.0 })
                .with_step_size(1.0)
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_unit(" st")
                .with_value_to_string(Arc::new(|v| format!("{:+.0}", v))),
            cents: FloatParam::new("Cents", 0.0, FloatRange::Linear { min: -100.0, max: 100.0 })
                .with_step_size(0.5)
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_unit(" c")
                .with_value_to_string(Arc::new(|v| format!("{:+.1}", v))),
            spread: FloatParam::new("Spread", 0.0, FloatRange::Linear { min: 0.0, max: 50.0 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_unit(" c")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            mode: EnumParam::new("Mode", ModeParam::Medium),
            mix: FloatParam::new("Mix", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_unit(" %")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            gain: FloatParam::new("Gain", 0.0, FloatRange::Linear { min: -24.0, max: 12.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_step_size(0.1)
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }
}

pub const SCOPE_LEN: usize = 1024;

/// Recent input / output (mono) for the editor's scopes.
pub struct Shared {
    pub input: Vec<AtomicU32>,
    pub output: Vec<AtomicU32>,
    pub pos: AtomicUsize,
}

impl Default for Shared {
    fn default() -> Self {
        let ring = || (0..SCOPE_LEN).map(|_| AtomicU32::new(0)).collect();
        Self { input: ring(), output: ring(), pos: AtomicUsize::new(0) }
    }
}

pub struct PetalShift {
    params: Arc<ShiftParams>,
    shared: Arc<Shared>,
    shifters: [Shifter; 2],
    dry: [DryDelay; 2],
    sample_rate: f32,
    mode: Option<Mode>,
    latency: usize,
    #[cfg(feature = "standalone")]
    demo: demo::Melody,
}

impl Default for PetalShift {
    fn default() -> Self {
        Self {
            params: Arc::new(ShiftParams::default()),
            shared: Arc::new(Shared::default()),
            shifters: [Shifter::new(48_000.0), Shifter::new(48_000.0)],
            dry: [DryDelay::new(8192), DryDelay::new(8192)],
            sample_rate: 48_000.0,
            mode: None,
            latency: 0,
            #[cfg(feature = "standalone")]
            demo: demo::Melody::new(48_000.0),
        }
    }
}

impl Plugin for PetalShift {
    const NAME: &'static str = "Petal Shift";
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
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.shifters = [Shifter::new(self.sample_rate), Shifter::new(self.sample_rate)];
        let max_latency = Geometry::new(Mode::Slow, self.sample_rate).latency();
        self.dry = [DryDelay::new(max_latency), DryDelay::new(max_latency)];
        let mode: Mode = self.params.mode.value().into();
        self.apply_mode(mode);
        context.set_latency_samples(self.latency as u32);
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Melody::new(self.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        for s in &mut self.shifters {
            s.reset();
        }
        for d in &mut self.dry {
            d.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let params = self.params.clone();
        let p = &params;
        let mode: Mode = p.mode.value().into();
        if self.mode != Some(mode) {
            self.apply_mode(mode);
            context.set_latency_samples(self.latency as u32);
        }
        let mut pos = self.shared.pos.load(Relaxed);

        for mut frame in buffer.iter_samples() {
            let semis = p.semitones.smoothed.next() + p.cents.smoothed.next() / 100.0;
            let spread = p.spread.smoothed.next() / 100.0 * 0.5;
            let ratios = [2f32.powf((semis - spread) / 12.0), 2f32.powf((semis + spread) / 12.0)];
            let mix = p.mix.smoothed.next();
            let gain = 10f32.powf(p.gain.smoothed.next() / 20.0);

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let mut input = [*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap()];
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                input = [input[0] + d, input[1] + d];
            }
            let mut out = [0.0; 2];
            for ch in 0..2 {
                let wet = self.shifters[ch].process(input[ch], ratios[ch]);
                let dry = self.dry[ch].process(input[ch], self.latency);
                out[ch] = (dry + (wet - dry) * mix) * gain;
            }
            *frame.get_mut(0).unwrap() = out[0];
            *frame.get_mut(1).unwrap() = out[1];

            let i = pos % SCOPE_LEN;
            self.shared.input[i].store(((input[0] + input[1]) * 0.5).to_bits(), Relaxed);
            self.shared.output[i].store(((out[0] + out[1]) * 0.5).to_bits(), Relaxed);
            pos = pos.wrapping_add(1);
        }
        self.shared.pos.store(pos, Relaxed);
        ProcessStatus::Normal
    }
}

impl PetalShift {
    fn apply_mode(&mut self, mode: Mode) {
        let g = Geometry::new(mode, self.sample_rate);
        for s in &mut self.shifters {
            s.set_geometry(g);
        }
        self.latency = g.latency();
        self.mode = Some(mode);
    }
}

impl ClapPlugin for PetalShift {
    const CLAP_ID: &'static str = "com.b4demus.petal-shift";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Pitch shifter");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::PitchShifter];
}

impl Vst3Plugin for PetalShift {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalShift01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::PitchShift, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalShift);
nih_export_vst3!(PetalShift);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: a simple sine melody.
    pub struct Melody {
        sr: f32,
        t: u64,
        phase: f32,
    }

    impl Melody {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0, phase: 0.0 }
        }

        pub fn next(&mut self) -> f32 {
            let t = self.t as f32 / self.sr;
            self.t += 1;
            let notes = [220.0, 261.63, 329.63, 293.66];
            let f = notes[(t / 0.6) as usize % notes.len()];
            self.phase = (self.phase + f / self.sr) % 1.0;
            let k = t % 0.6;
            let env = (k * 30.0).min(1.0) * (-k * 2.0).exp();
            let ph = std::f32::consts::TAU * self.phase;
            (ph.sin() + 0.3 * (2.0 * ph).sin()) * env * 0.3
        }
    }
}
