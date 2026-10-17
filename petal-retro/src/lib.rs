use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{Chain, DistortType, NoiseType, Settings};

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum NoiseKind {
    Hiss,
    Vinyl,
    Hum,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum DistortKind {
    Warm,
    Tape,
    Fuzz,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(30.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

#[derive(Params)]
pub struct RetroParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "noise_on"]
    pub noise_on: BoolParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "noise_type"]
    pub noise_type: EnumParam<NoiseKind>,

    #[id = "wobble_on"]
    pub wobble_on: BoolParam,
    #[id = "wobble"]
    pub wobble: FloatParam,
    #[id = "wobble_rate"]
    pub wobble_rate: FloatParam,

    #[id = "distort_on"]
    pub distort_on: BoolParam,
    #[id = "distort"]
    pub distort: FloatParam,
    #[id = "distort_type"]
    pub distort_type: EnumParam<DistortKind>,

    #[id = "digital_on"]
    pub digital_on: BoolParam,
    #[id = "digital"]
    pub digital: FloatParam,
    #[id = "digital_rate"]
    pub digital_rate: FloatParam,

    #[id = "space_on"]
    pub space_on: BoolParam,
    #[id = "space"]
    pub space: FloatParam,
    #[id = "space_size"]
    pub space_size: FloatParam,

    #[id = "magnetic_on"]
    pub magnetic_on: BoolParam,
    #[id = "magnetic"]
    pub magnetic: FloatParam,
    #[id = "dropout"]
    pub dropout: FloatParam,

    #[id = "magnitude"]
    pub magnitude: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for RetroParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            noise_on: BoolParam::new("Noise On", true),
            noise: percent("Noise", 0.3),
            noise_type: EnumParam::new("Noise Type", NoiseKind::Vinyl),
            wobble_on: BoolParam::new("Wobble On", true),
            wobble: percent("Wobble", 0.3),
            wobble_rate: FloatParam::new(
                "Wobble Rate",
                0.8,
                FloatRange::Skewed { min: 0.1, max: 8.0, factor: FloatRange::skew_factor(-1.8) },
            )
            .with_smoother(SmoothingStyle::Logarithmic(30.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            distort_on: BoolParam::new("Distort On", true),
            distort: percent("Distort", 0.25),
            distort_type: EnumParam::new("Distort Type", DistortKind::Warm),
            digital_on: BoolParam::new("Digital On", false),
            digital: percent("Digital", 0.3),
            digital_rate: percent("Digital Rate", 0.3),
            space_on: BoolParam::new("Space On", false),
            space: percent("Space", 0.25),
            space_size: percent("Space Size", 0.5),
            magnetic_on: BoolParam::new("Magnetic On", true),
            magnetic: percent("Magnetic", 0.3),
            dropout: percent("Dropout", 0.2),
            magnitude: percent("Magnitude", 1.0),
            tone: FloatParam::new("Tone", 0.0, FloatRange::Linear { min: -1.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_value_to_string(Arc::new(|v| {
                    if v.abs() < 0.005 {
                        "flat".to_owned()
                    } else if v < 0.0 {
                        format!("dark {:.0}%", -v * 100.0)
                    } else {
                        format!("bright {:.0}%", v * 100.0)
                    }
                })),
            width: FloatParam::new("Width", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_unit(" %")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            mix: percent("Mix", 1.0),
            output: FloatParam::new("Output", 0.0, FloatRange::Linear { min: -24.0, max: 12.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_step_size(0.1)
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }
}

pub const SCOPE_LEN: usize = 1024;

/// Live values for the editor.
pub struct Shared {
    pub wobble: AtomicU32,
    pub dropout: AtomicU32,
    pub scope: Vec<AtomicU32>,
    pub scope_pos: AtomicUsize,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            wobble: AtomicU32::new(0),
            dropout: AtomicU32::new(1f32.to_bits()),
            scope: (0..SCOPE_LEN).map(|_| AtomicU32::new(0)).collect(),
            scope_pos: AtomicUsize::new(0),
        }
    }
}

impl Shared {
    pub fn load(a: &AtomicU32) -> f32 {
        f32::from_bits(a.load(Relaxed))
    }
}

/// On/off ramps so toggling a module never clicks.
#[derive(Default)]
struct Ramps([f32; 6]);

impl Ramps {
    fn step(&mut self, on: [bool; 6], step: f32) -> [f32; 6] {
        for (r, &on) in self.0.iter_mut().zip(&on) {
            let target = if on { 1.0 } else { 0.0 };
            *r = if *r < target { (*r + step).min(1.0) } else { (*r - step).max(0.0) };
        }
        self.0
    }
}

pub struct PetalRetro {
    params: Arc<RetroParams>,
    shared: Arc<Shared>,
    chain: Chain,
    ramps: Ramps,
    sample_rate: f32,
    #[cfg(feature = "standalone")]
    demo: demo::Keys,
}

impl Default for PetalRetro {
    fn default() -> Self {
        Self {
            params: Arc::new(RetroParams::default()),
            shared: Arc::new(Shared::default()),
            chain: Chain::new(48_000.0),
            ramps: Ramps::default(),
            sample_rate: 48_000.0,
            #[cfg(feature = "standalone")]
            demo: demo::Keys::new(48_000.0),
        }
    }
}

impl Plugin for PetalRetro {
    const NAME: &'static str = "Petal Retro";
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
        self.chain = Chain::new(self.sample_rate);
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Keys::new(self.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        self.chain.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let ramp_step = 1.0 / (0.02 * self.sample_rate);
        let on = [
            p.noise_on.value(),
            p.wobble_on.value(),
            p.distort_on.value(),
            p.digital_on.value(),
            p.space_on.value(),
            p.magnetic_on.value(),
        ];
        let noise_type = match p.noise_type.value() {
            NoiseKind::Hiss => NoiseType::Hiss,
            NoiseKind::Vinyl => NoiseType::Vinyl,
            NoiseKind::Hum => NoiseType::Hum,
        };
        let distort_type = match p.distort_type.value() {
            DistortKind::Warm => DistortType::Warm,
            DistortKind::Tape => DistortType::Tape,
            DistortKind::Fuzz => DistortType::Fuzz,
        };
        let mut pos = self.shared.scope_pos.load(Relaxed);

        for mut frame in buffer.iter_samples() {
            let r = self.ramps.step(on, ramp_step);
            let m = p.magnitude.smoothed.next();
            let s = Settings {
                noise_mix: r[0],
                noise: p.noise.smoothed.next() * m,
                noise_type,
                wobble_mix: r[1],
                wobble: p.wobble.smoothed.next() * m,
                wobble_rate: p.wobble_rate.smoothed.next(),
                distort_mix: r[2],
                distort: p.distort.smoothed.next() * m,
                distort_type,
                digital_mix: r[3],
                digital: p.digital.smoothed.next() * m,
                digital_rate: p.digital_rate.smoothed.next() * m,
                space_mix: r[4],
                space: p.space.smoothed.next() * m,
                space_size: p.space_size.smoothed.next(),
                magnetic_mix: r[5],
                magnetic: p.magnetic.smoothed.next() * m,
                dropout: p.dropout.smoothed.next() * m,
                tone: p.tone.smoothed.next(),
                width: p.width.smoothed.next(),
                mix: p.mix.smoothed.next(),
            };
            let out_gain = 10f32.powf(p.output.smoothed.next() / 20.0);

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let (mut l, mut rr) = (*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap());
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                l += d;
                rr += d;
            }
            let y = self.chain.process([l, rr], &s);
            let out = [y[0] * out_gain, y[1] * out_gain];
            *frame.get_mut(0).unwrap() = out[0];
            *frame.get_mut(1).unwrap() = out[1];

            self.shared.scope[pos % SCOPE_LEN].store(((out[0] + out[1]) * 0.5).to_bits(), Relaxed);
            pos = pos.wrapping_add(1);
        }
        self.shared.scope_pos.store(pos, Relaxed);
        self.shared.wobble.store(self.chain.wobble_now.to_bits(), Relaxed);
        self.shared.dropout.store(self.chain.dropout_gain().to_bits(), Relaxed);
        // Noise and the room tail keep going without input.
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for PetalRetro {
    const CLAP_ID: &'static str = "com.b4demus.petal-retro";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Lo-fi colour: noise, wobble, distortion, bitcrush, room, tape");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Distortion,
        ClapFeature::MultiEffects,
    ];
}

impl Vst3Plugin for PetalRetro {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalRetro01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalRetro);
nih_export_vst3!(PetalRetro);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: soft electric-piano chords.
    pub struct Keys {
        sr: f32,
        t: u64,
    }

    impl Keys {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0 }
        }

        pub fn next(&mut self) -> f32 {
            let t = self.t as f32 / self.sr;
            self.t += 1;
            let chords = [[261.63, 329.63, 392.0], [220.0, 261.63, 329.63], [174.61, 220.0, 261.63], [196.0, 246.94, 293.66]];
            let c = chords[(t / 1.6) as usize % 4];
            let k = t % 1.6;
            let env = (-k * 2.2).exp() * (1.0 - (-k * 300.0).exp());
            let s: f32 = c
                .iter()
                .map(|f| {
                    let ph = std::f32::consts::TAU * f * t;
                    ph.sin() + 0.2 * (2.0 * ph).sin()
                })
                .sum();
            s * env * 0.12
        }
    }
}
