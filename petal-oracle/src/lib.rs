use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{Harmony, Oracle, Scale, Tone, VOICES};

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Key {
    C,
    #[name = "C#"]
    Cs,
    D,
    #[name = "D#"]
    Ds,
    E,
    F,
    #[name = "F#"]
    Fs,
    G,
    #[name = "G#"]
    Gs,
    A,
    #[name = "A#"]
    As,
    B,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum ScaleParam {
    Major,
    Minor,
    Dorian,
    Phrygian,
    Lydian,
    Pentatonic,
    #[name = "Minor Penta"]
    MinorPentatonic,
    Hirajoshi,
    #[name = "Whole Tone"]
    WholeTone,
    Chromatic,
}

impl From<ScaleParam> for Scale {
    fn from(s: ScaleParam) -> Self {
        match s {
            ScaleParam::Major => Scale::Major,
            ScaleParam::Minor => Scale::Minor,
            ScaleParam::Dorian => Scale::Dorian,
            ScaleParam::Phrygian => Scale::Phrygian,
            ScaleParam::Lydian => Scale::Lydian,
            ScaleParam::Pentatonic => Scale::Pentatonic,
            ScaleParam::MinorPentatonic => Scale::MinorPentatonic,
            ScaleParam::Hirajoshi => Scale::Hirajoshi,
            ScaleParam::WholeTone => Scale::WholeTone,
            ScaleParam::Chromatic => Scale::Chromatic,
        }
    }
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Rate {
    #[name = "1/16"]
    N16,
    #[name = "1/8"]
    N8,
    #[name = "1/4"]
    N4,
    #[name = "1/2"]
    N2,
    #[name = "1 bar"]
    Bar1,
    #[name = "2 bars"]
    Bar2,
    #[name = "4 bars"]
    Bar4,
}

impl Rate {
    pub fn beats(self) -> f32 {
        match self {
            Rate::N16 => 0.25,
            Rate::N8 => 0.5,
            Rate::N4 => 1.0,
            Rate::N2 => 2.0,
            Rate::Bar1 => 4.0,
            Rate::Bar2 => 8.0,
            Rate::Bar4 => 16.0,
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

#[derive(Params)]
pub struct OracleParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "key"]
    pub key: EnumParam<Key>,
    #[id = "scale"]
    pub scale: EnumParam<ScaleParam>,
    #[id = "octave"]
    pub octave: IntParam,
    #[id = "spread"]
    pub spread: IntParam,

    #[id = "rate"]
    pub rate: EnumParam<Rate>,
    #[id = "chaos"]
    pub chaos: FloatParam,
    #[id = "glide"]
    pub glide: FloatParam,

    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "brightness"]
    pub brightness: FloatParam,
    #[id = "bite"]
    pub bite: FloatParam,
    #[id = "halo"]
    pub halo: FloatParam,
    #[id = "freeze"]
    pub freeze: BoolParam,

    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for OracleParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            key: EnumParam::new("Key", Key::A),
            scale: EnumParam::new("Scale", ScaleParam::Minor),
            octave: IntParam::new("Octave", 3, IntRange::Linear { min: 1, max: 5 }),
            spread: IntParam::new("Spread", 2, IntRange::Linear { min: 1, max: 4 })
                .with_unit(" oct"),
            rate: EnumParam::new("Rate", Rate::N2),
            chaos: unit("Chaos", 0.35),
            glide: FloatParam::new("Glide", 0.12, FloatRange::Skewed { min: 0.0, max: 2.0, factor: FloatRange::skew_factor(-1.5) })
                .with_unit(" s")
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            decay: FloatParam::new("Decay", 3.0, FloatRange::Skewed { min: 0.1, max: 20.0, factor: FloatRange::skew_factor(-1.8) })
                .with_smoother(SmoothingStyle::Logarithmic(40.0))
                .with_unit(" s")
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            brightness: unit("Brightness", 0.6),
            bite: unit("Bite", 0.0),
            halo: unit("Halo", 0.4),
            freeze: BoolParam::new("Freeze", false),
            mix: unit("Mix", 0.8),
            output: FloatParam::new("Output", 0.0, FloatRange::Linear { min: -24.0, max: 12.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_step_size(0.1)
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
        }
    }
}

impl OracleParams {
    pub fn harmony(&self) -> Harmony {
        Harmony {
            key: self.key.value() as i32,
            scale: self.scale.value().into(),
            octave: self.octave.value(),
            spread: self.spread.value(),
        }
    }
}

/// Live values for the editor.
#[derive(Default)]
pub struct Shared {
    pub note: [AtomicU32; VOICES],
    pub energy: [AtomicU32; VOICES],
    /// Incremented on every melody step.
    pub steps: AtomicU32,
    pub bpm: AtomicU32,
}

impl Shared {
    pub fn load(a: &AtomicU32) -> f32 {
        f32::from_bits(a.load(Relaxed))
    }
}

pub struct PetalOracle {
    params: Arc<OracleParams>,
    shared: Arc<Shared>,
    oracle: Oracle,
    sample_rate: f32,
    until_step: f32,
    #[cfg(feature = "standalone")]
    demo: demo::Taps,
}

impl Default for PetalOracle {
    fn default() -> Self {
        Self {
            params: Arc::new(OracleParams::default()),
            shared: Arc::new(Shared::default()),
            oracle: Oracle::new(48_000.0),
            sample_rate: 48_000.0,
            until_step: 0.0,
            #[cfg(feature = "standalone")]
            demo: demo::Taps::new(48_000.0),
        }
    }
}

impl Plugin for PetalOracle {
    const NAME: &'static str = "Petal Oracle";
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
        self.oracle = Oracle::new(self.sample_rate);
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Taps::new(self.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        self.oracle.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let bpm = context.transport().tempo.unwrap_or(120.0) as f32;
        self.shared.bpm.store(bpm.to_bits(), Relaxed);
        let harmony = p.harmony();
        let step_len = p.rate.value().beats() * 60.0 / bpm.max(20.0) * self.sample_rate;
        let freeze = p.freeze.value();

        for mut frame in buffer.iter_samples() {
            // The melody brain moves the notes once per Rate (keeps moving while frozen).
            self.until_step -= 1.0;
            if self.until_step <= 0.0 {
                self.until_step += step_len;
                self.oracle.step(&harmony, p.chaos.value());
                self.shared.steps.fetch_add(1, Relaxed);
            }
            let tone = Tone {
                decay_s: p.decay.smoothed.next(),
                brightness: p.brightness.smoothed.next(),
                bite: p.bite.smoothed.next(),
                halo: p.halo.smoothed.next(),
                glide_s: p.glide.value(),
                freeze,
            };
            let mix = p.mix.smoothed.next();
            let gain = 10f32.powf(p.output.smoothed.next() / 20.0);

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let mut x = [*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap()];
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                x = [x[0] + d, x[1] + d];
            }
            let wet = self.oracle.process(x, &harmony, &tone);
            *frame.get_mut(0).unwrap() = (x[0] + (wet[0] - x[0]) * mix) * gain;
            *frame.get_mut(1).unwrap() = (x[1] + (wet[1] - x[1]) * mix) * gain;
        }
        for (i, v) in self.oracle.views().iter().enumerate() {
            self.shared.note[i].store(v.note.to_bits(), Relaxed);
            self.shared.energy[i].store(v.energy.to_bits(), Relaxed);
        }
        // Resonators ring on after the input stops.
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for PetalOracle {
    const CLAP_ID: &'static str = "com.b4demus.petal-oracle";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Generative resonator that turns any input into melodies");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::Filter, ClapFeature::Glitch];
}

impl Vst3Plugin for PetalOracle {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalOracle1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Filter, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalOracle);
nih_export_vst3!(PetalOracle);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: noise ticks on eighth notes.
    pub struct Taps {
        sr: f32,
        t: u64,
        rng: u32,
    }

    impl Taps {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0, rng: 11 }
        }

        pub fn next(&mut self) -> f32 {
            let t = self.t as f32 / self.sr;
            self.t += 1;
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let noise = (self.rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
            let k = t % 0.25;
            noise * (-k * 160.0).exp() * 0.5
        }
    }
}
