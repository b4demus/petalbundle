use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{pan_gains, Voice, VoiceSettings, VOICES};

#[derive(Params)]
pub struct VoiceParams {
    #[id = "on"]
    pub on: BoolParam,
    #[id = "delay"]
    pub delay: FloatParam,
    #[id = "pitch"]
    pub pitch: FloatParam,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "pan"]
    pub pan: FloatParam,
}

/// Default voice layout: two wide voices on, two inner voices ready to switch on.
const DEFAULTS: [(bool, f32, f32, f32, f32); VOICES] = [
    (true, 12.0, 7.0, -3.0, -0.75),
    (true, 19.0, -9.0, -3.0, 0.75),
    (false, 27.0, 4.0, -6.0, -0.35),
    (false, 35.0, -5.0, -6.0, 0.35),
];

fn pan_param(name: String, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: -1.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(30.0))
        .with_value_to_string(Arc::new(|v| {
            let pct = (v * 100.0).round();
            if pct == 0.0 {
                "C".to_owned()
            } else if pct < 0.0 {
                format!("{}L", -pct)
            } else {
                format!("{pct}R")
            }
        }))
}

fn gain_param(name: String, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max })
        .with_smoother(SmoothingStyle::Linear(30.0))
        .with_step_size(0.1)
        .with_unit(" dB")
        .with_value_to_string(formatters::v2s_f32_rounded(1))
}

impl VoiceParams {
    fn new(i: usize) -> Self {
        let (on, delay, pitch, gain, pan) = DEFAULTS[i];
        let n = i + 1;
        Self {
            on: BoolParam::new(format!("Voice {n} On"), on),
            delay: FloatParam::new(
                format!("Voice {n} Delay"),
                delay,
                FloatRange::Linear { min: 1.0, max: dsp::MAX_DELAY_MS },
            )
            .with_smoother(SmoothingStyle::Linear(40.0))
            .with_step_size(0.1)
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            pitch: FloatParam::new(format!("Voice {n} Pitch"), pitch, FloatRange::Linear { min: -50.0, max: 50.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_step_size(0.1)
                .with_unit(" c")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            gain: gain_param(format!("Voice {n} Gain"), gain, -36.0, 6.0),
            pan: pan_param(format!("Voice {n} Pan"), pan),
        }
    }
}

fn percent(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max })
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

#[derive(Params)]
pub struct DoubleParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[nested(array, group = "Voices")]
    pub voices: [VoiceParams; VOICES],

    #[id = "direct"]
    pub direct: FloatParam,
    #[id = "direct_pan"]
    pub direct_pan: FloatParam,
    #[id = "mod_rate"]
    pub mod_rate: FloatParam,
    #[id = "mod_depth"]
    pub mod_depth: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "low_cut"]
    pub low_cut: FloatParam,
    #[id = "high_cut"]
    pub high_cut: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for DoubleParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            voices: std::array::from_fn(VoiceParams::new),
            direct: gain_param("Direct".into(), 0.0, -36.0, 6.0),
            direct_pan: pan_param("Direct Pan".into(), 0.0),
            mod_rate: FloatParam::new(
                "Mod Rate",
                0.7,
                FloatRange::Skewed { min: 0.05, max: 8.0, factor: FloatRange::skew_factor(-1.8) },
            )
            .with_smoother(SmoothingStyle::Logarithmic(30.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            mod_depth: percent("Mod Depth", 0.35, 1.0),
            feedback: percent("Feedback", 0.0, 0.9),
            low_cut: hz("Low Cut", 120.0, 20.0, 1000.0),
            high_cut: hz("High Cut", 12_000.0, 1000.0, 20_000.0),
            output: gain_param("Output".into(), 0.0, -24.0, 12.0),
        }
    }
}

/// Live modulation values for the editor (one per voice).
#[derive(Default)]
pub struct Shared {
    pub wobble: [AtomicU32; VOICES],
}

pub struct PetalDouble {
    params: Arc<DoubleParams>,
    shared: Arc<Shared>,
    voices: [Voice; VOICES],
    #[cfg(feature = "standalone")]
    demo: demo::Phrase,
}

impl Default for PetalDouble {
    fn default() -> Self {
        Self {
            params: Arc::new(DoubleParams::default()),
            shared: Arc::new(Shared::default()),
            voices: std::array::from_fn(|i| Voice::new(48_000.0, 17 + i as u32 * 7919)),
            #[cfg(feature = "standalone")]
            demo: demo::Phrase::new(48_000.0),
        }
    }
}

#[inline]
fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

impl Plugin for PetalDouble {
    const NAME: &'static str = "Petal Double";
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
        let sr = buffer_config.sample_rate;
        self.voices = std::array::from_fn(|i| Voice::new(sr, 17 + i as u32 * 7919));
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Phrase::new(sr);
        }
        true
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        for mut frame in buffer.iter_samples() {
            let shared = dsp::Shared {
                mod_rate_hz: p.mod_rate.smoothed.next(),
                mod_depth: p.mod_depth.smoothed.next(),
                feedback: p.feedback.smoothed.next(),
                low_cut_hz: p.low_cut.smoothed.next(),
                high_cut_hz: p.high_cut.smoothed.next(),
            };
            let direct = db_to_gain(p.direct.smoothed.next());
            let direct_pan = pan_gains(p.direct_pan.smoothed.next());
            let out_gain = db_to_gain(p.output.smoothed.next());

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let (mut l, mut r) = (*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap());
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                l += d;
                r += d;
            }
            let mono = (l + r) * 0.5;

            let mut out = [l * direct * direct_pan[0], r * direct * direct_pan[1]];
            for (v, vp) in self.voices.iter_mut().zip(&p.voices) {
                let settings = VoiceSettings {
                    delay_ms: vp.delay.smoothed.next(),
                    cents: vp.pitch.smoothed.next(),
                    gain: db_to_gain(vp.gain.smoothed.next()),
                };
                let pan = pan_gains(vp.pan.smoothed.next());
                // Voices keep running while off so switching on is seamless.
                let y = v.process(mono, &settings, &shared);
                if vp.on.value() {
                    out[0] += y * pan[0];
                    out[1] += y * pan[1];
                }
            }
            *frame.get_mut(0).unwrap() = out[0] * out_gain;
            *frame.get_mut(1).unwrap() = out[1] * out_gain;
        }
        for (i, v) in self.voices.iter().enumerate() {
            self.shared.wobble[i].store(v.wobble().to_bits(), Relaxed);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for PetalDouble {
    const CLAP_ID: &'static str = "com.b4demus.petal-double";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Four-voice doubler");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::Chorus];
}

impl Vst3Plugin for PetalDouble {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalDouble1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Modulation, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalDouble);
nih_export_vst3!(PetalDouble);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: a sung-like saw phrase.
    pub struct Phrase {
        sr: f32,
        t: u64,
        phase: f32,
    }

    impl Phrase {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0, phase: 0.0 }
        }

        pub fn next(&mut self) -> f32 {
            let t = self.t as f32 / self.sr;
            self.t += 1;
            let notes = [220.0, 246.94, 261.63, 293.66];
            let f = notes[(t / 0.8) as usize % notes.len()] * (1.0 + 0.004 * (t * 5.5 * std::f32::consts::TAU).sin());
            self.phase = (self.phase + f / self.sr) % 1.0;
            let k = t % 0.8;
            let env = (k * 20.0).min(1.0) * (1.0 - (k - 0.6).max(0.0) * 5.0);
            (2.0 * self.phase - 1.0) * env * 0.15
        }
    }
}
