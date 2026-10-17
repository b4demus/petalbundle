use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{ModDelay, Settings, Shape, MAX_TAPS};

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum ShapeParam {
    Sine,
    Triangle,
    Wander,
}

impl From<ShapeParam> for Shape {
    fn from(s: ShapeParam) -> Self {
        match s {
            ShapeParam::Sine => Shape::Sine,
            ShapeParam::Triangle => Shape::Triangle,
            ShapeParam::Wander => Shape::Wander,
        }
    }
}

fn unit(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(40.0))
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

#[derive(Params)]
pub struct ModParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "delay"]
    pub delay: FloatParam,
    #[id = "taps"]
    pub taps: IntParam,
    #[id = "spread"]
    pub spread: FloatParam,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "shape"]
    pub shape: EnumParam<ShapeParam>,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "diffusion"]
    pub diffusion: FloatParam,
    #[id = "warp"]
    pub warp: FloatParam,
    #[id = "low_cut"]
    pub low_cut: FloatParam,
    #[id = "high_cut"]
    pub high_cut: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for ModParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            delay: FloatParam::new(
                "Delay",
                12.0,
                FloatRange::Skewed { min: 0.5, max: dsp::MAX_DELAY_MS, factor: FloatRange::skew_factor(-2.2) },
            )
            .with_smoother(SmoothingStyle::Logarithmic(60.0))
            .with_unit(" ms")
            .with_value_to_string(Arc::new(|v| if v < 10.0 { format!("{v:.2}") } else { format!("{v:.1}") })),
            taps: IntParam::new("Taps", 3, IntRange::Linear { min: 1, max: MAX_TAPS as i32 }),
            spread: unit("Spread", 0.4),
            rate: FloatParam::new("Rate", 0.6, FloatRange::Skewed { min: 0.01, max: 10.0, factor: FloatRange::skew_factor(-2.0) })
                .with_smoother(SmoothingStyle::Logarithmic(40.0))
                .with_unit(" Hz")
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            depth: unit("Depth", 0.35),
            shape: EnumParam::new("Shape", ShapeParam::Sine),
            feedback: FloatParam::new("Feedback", 0.0, FloatRange::Linear { min: -0.95, max: 0.95 })
                .with_smoother(SmoothingStyle::Linear(40.0))
                .with_value_to_string(Arc::new(|v| format!("{:+.0}%", v * 100.0))),
            diffusion: unit("Diffusion", 0.0),
            warp: unit("Warp", 0.0),
            low_cut: hz("Low Cut", 80.0, 20.0, 2000.0),
            high_cut: hz("High Cut", 12_000.0, 1000.0, 20_000.0),
            width: unit("Width", 0.8),
            mix: unit("Mix", 0.5),
        }
    }
}

impl ModParams {
    pub fn settings(&self) -> Settings {
        Settings {
            delay_ms: self.delay.value(),
            taps: self.taps.value() as usize,
            spread: self.spread.value(),
            rate_hz: self.rate.value(),
            depth: self.depth.value(),
            shape: self.shape.value().into(),
            feedback: self.feedback.value(),
            diffusion: self.diffusion.value(),
            warp: self.warp.value(),
            low_cut_hz: self.low_cut.value(),
            high_cut_hz: self.high_cut.value(),
            width: self.width.value(),
        }
    }
}

/// One-click styles, like UberMod's modes.
pub struct Style {
    pub name: &'static str,
    pub taps: i32,
    pub delay: f32,
    pub spread: f32,
    pub rate: f32,
    pub depth: f32,
    pub shape: ShapeParam,
    pub feedback: f32,
    pub diffusion: f32,
    pub warp: f32,
    pub width: f32,
    pub mix: f32,
    pub high_cut: f32,
}

pub const STYLES: [Style; 5] = [
    Style { name: "chorus", taps: 3, delay: 12.0, spread: 0.4, rate: 0.6, depth: 0.35, shape: ShapeParam::Sine, feedback: 0.0, diffusion: 0.0, warp: 0.0, width: 0.8, mix: 0.5, high_cut: 12_000.0 },
    Style { name: "ensemble", taps: 6, delay: 9.0, spread: 0.6, rate: 1.1, depth: 0.5, shape: ShapeParam::Wander, feedback: 0.0, diffusion: 0.0, warp: 0.0, width: 1.0, mix: 0.5, high_cut: 11_000.0 },
    Style { name: "flanger", taps: 1, delay: 2.0, spread: 0.0, rate: 0.2, depth: 0.6, shape: ShapeParam::Triangle, feedback: -0.7, diffusion: 0.0, warp: 0.1, width: 0.5, mix: 0.5, high_cut: 16_000.0 },
    Style { name: "vibrato", taps: 1, delay: 5.0, spread: 0.0, rate: 5.0, depth: 0.3, shape: ShapeParam::Sine, feedback: 0.0, diffusion: 0.0, warp: 0.0, width: 0.0, mix: 1.0, high_cut: 16_000.0 },
    Style { name: "sky", taps: 8, delay: 180.0, spread: 0.9, rate: 0.15, depth: 0.6, shape: ShapeParam::Wander, feedback: 0.6, diffusion: 0.8, warp: 0.2, width: 1.0, mix: 0.4, high_cut: 7000.0 },
];

/// Live tap delays (ms) for the editor.
#[derive(Default)]
pub struct Shared {
    pub tap_ms: [AtomicU32; MAX_TAPS],
}

pub struct PetalMod {
    params: Arc<ModParams>,
    shared: Arc<Shared>,
    delay: ModDelay,
    #[cfg(feature = "standalone")]
    demo: demo::Pad,
}

impl Default for PetalMod {
    fn default() -> Self {
        Self {
            params: Arc::new(ModParams::default()),
            shared: Arc::new(Shared::default()),
            delay: ModDelay::new(48_000.0),
            #[cfg(feature = "standalone")]
            demo: demo::Pad::new(48_000.0),
        }
    }
}

impl Plugin for PetalMod {
    const NAME: &'static str = "Petal Mod";
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
        self.delay = ModDelay::new(buffer_config.sample_rate);
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Pad::new(buffer_config.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        self.delay.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let base = p.settings();
        for mut frame in buffer.iter_samples() {
            let s = Settings {
                delay_ms: p.delay.smoothed.next(),
                spread: p.spread.smoothed.next(),
                rate_hz: p.rate.smoothed.next(),
                depth: p.depth.smoothed.next(),
                feedback: p.feedback.smoothed.next(),
                diffusion: p.diffusion.smoothed.next(),
                warp: p.warp.smoothed.next(),
                low_cut_hz: p.low_cut.smoothed.next(),
                high_cut_hz: p.high_cut.smoothed.next(),
                width: p.width.smoothed.next(),
                ..base
            };
            let mix = p.mix.smoothed.next();

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let mut x = [*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap()];
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                x = [x[0] + d, x[1] + d];
            }
            let wet = self.delay.process(x, &s);
            *frame.get_mut(0).unwrap() = x[0] + (wet[0] - x[0]) * mix;
            *frame.get_mut(1).unwrap() = x[1] + (wet[1] - x[1]) * mix;
        }
        for (k, ms) in self.delay.current_ms.iter().enumerate() {
            self.shared.tap_ms[k].store(ms.to_bits(), Relaxed);
        }
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for PetalMod {
    const CLAP_ID: &'static str = "com.b4demus.petal-mod";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Multi-tap modulated delay");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Chorus,
        ClapFeature::Flanger,
        ClapFeature::Delay,
    ];
}

impl Vst3Plugin for PetalMod {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalMod0001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Modulation, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalMod);
nih_export_vst3!(PetalMod);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: a plucked saw arpeggio.
    pub struct Pad {
        sr: f32,
        t: u64,
        phase: f32,
    }

    impl Pad {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0, phase: 0.0 }
        }

        pub fn next(&mut self) -> f32 {
            let t = self.t as f32 / self.sr;
            self.t += 1;
            let notes = [220.0, 277.18, 329.63, 440.0];
            let f = notes[(t / 0.4) as usize % 4];
            self.phase = (self.phase + f / self.sr) % 1.0;
            let k = t % 0.4;
            (2.0 * self.phase - 1.0) * (-k * 5.0).exp() * 0.2
        }
    }
}
