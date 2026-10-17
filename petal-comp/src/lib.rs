use nih_plug::prelude::*;
use std::sync::Arc;

pub mod dsp;
mod editor;
pub mod meter;

use dsp::{db_to_gain, Compressor, Settings};

#[derive(Params)]
pub struct CompParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "threshold"]
    pub threshold: FloatParam,
    #[id = "ratio"]
    pub ratio: FloatParam,
    #[id = "knee"]
    pub knee: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "dry"]
    pub dry: FloatParam,
}

fn millis(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) },
    )
    .with_smoother(SmoothingStyle::Logarithmic(20.0))
    .with_unit(" ms")
    .with_value_to_string(Arc::new(|v| {
        if v < 1.0 {
            format!("{v:.2}")
        } else if v < 100.0 {
            format!("{v:.1}")
        } else {
            format!("{v:.0}")
        }
    }))
}

impl Default for CompParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            threshold: FloatParam::new("Threshold", -18.0, FloatRange::Linear { min: -60.0, max: 0.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_step_size(0.1)
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            ratio: FloatParam::new(
                "Ratio",
                4.0,
                FloatRange::Skewed { min: 1.0, max: 30.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_value_to_string(Arc::new(|v| {
                if v >= 29.9 {
                    "∞:1".to_owned()
                } else {
                    format!("{v:.1}:1")
                }
            })),
            knee: FloatParam::new("Knee", 6.0, FloatRange::Linear { min: 0.0, max: 24.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            attack: millis("Attack", 10.0, 0.05, 250.0),
            release: millis("Release", 120.0, 5.0, 2500.0),
            gain: FloatParam::new("Gain", 0.0, FloatRange::Linear { min: -24.0, max: 24.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_step_size(0.1)
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            dry: FloatParam::new("Dry", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_unit(" %")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
        }
    }
}

impl CompParams {
    pub fn settings(&self) -> Settings {
        Settings {
            threshold_db: self.threshold.value(),
            ratio: ratio_value(self.ratio.value()),
            knee_db: self.knee.value(),
            attack_ms: self.attack.value(),
            release_ms: self.release.value(),
        }
    }
}

/// The top of the ratio range acts as a limiter.
pub fn ratio_value(r: f32) -> f32 {
    if r >= 29.9 {
        1000.0
    } else {
        r
    }
}

pub struct PetalComp {
    params: Arc<CompParams>,
    meter: Arc<meter::Shared>,
    comp: Compressor,
    acc: meter::Accumulator,
    #[cfg(feature = "standalone")]
    demo: demo::Drums,
}

impl Default for PetalComp {
    fn default() -> Self {
        Self {
            params: Arc::new(CompParams::default()),
            meter: Arc::new(meter::Shared::default()),
            comp: Compressor::new(48_000.0),
            acc: meter::Accumulator::new(48_000.0),
            #[cfg(feature = "standalone")]
            demo: demo::Drums::new(48_000.0),
        }
    }
}

impl Plugin for PetalComp {
    const NAME: &'static str = "Petal Comp";
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
        editor::create(self.params.clone(), self.meter.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let sr = buffer_config.sample_rate;
        self.comp = Compressor::new(sr);
        self.acc = meter::Accumulator::new(sr);
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Drums::new(sr);
        }
        true
    }

    fn reset(&mut self) {
        self.comp.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        for mut frame in buffer.iter_samples() {
            let settings = Settings {
                threshold_db: p.threshold.smoothed.next(),
                ratio: ratio_value(p.ratio.smoothed.next()),
                knee_db: p.knee.smoothed.next(),
                attack_ms: p.attack.smoothed.next(),
                release_ms: p.release.smoothed.next(),
            };
            let makeup = p.gain.smoothed.next();
            let dry = p.dry.smoothed.next();

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let (mut l, mut r) = (*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap());
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                l += d;
                r += d;
            }

            let detector = l.abs().max(r.abs());
            let reduction = self.comp.process(detector, &settings);
            let g = db_to_gain(makeup - reduction);
            let out_l = l * g * (1.0 - dry) + l * dry;
            let out_r = r * g * (1.0 - dry) + r * dry;
            *frame.get_mut(0).unwrap() = out_l;
            *frame.get_mut(1).unwrap() = out_r;

            self.acc.add(&self.meter, detector, out_l.abs().max(out_r.abs()), reduction);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for PetalComp {
    const CLAP_ID: &'static str = "com.b4demus.petal-comp";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Compressor with level history display");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::Compressor];
}

impl Vst3Plugin for PetalComp {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalComp001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalComp);
nih_export_vst3!(PetalComp);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: a simple kick/snare/hat loop.
    pub struct Drums {
        sr: f32,
        t: u64,
        rng: u32,
    }

    impl Drums {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0, rng: 9 }
        }

        pub fn next(&mut self) -> f32 {
            let beat = 60.0 / 100.0; // 100 bpm, quarter notes
            let t = self.t as f32 / self.sr;
            self.t += 1;
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let noise = (self.rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0;

            let pos = t % (beat * 4.0);
            let mut out = 0.0;
            for kick in [0.0, beat * 2.0, beat * 2.5] {
                let k = pos - kick;
                if (0.0..0.4).contains(&k) {
                    let f = 45.0 + 120.0 * (-k * 30.0).exp();
                    out += (std::f32::consts::TAU * f * k).sin() * (-k * 7.0).exp() * 0.9;
                }
            }
            for snare in [beat, beat * 3.0] {
                let k = pos - snare;
                if (0.0..0.3).contains(&k) {
                    out += noise * (-k * 18.0).exp() * 0.5;
                }
            }
            let hat = pos % (beat / 2.0);
            if hat < 0.05 {
                out += noise * (-hat * 90.0).exp() * 0.12;
            }
            out
        }
    }
}
