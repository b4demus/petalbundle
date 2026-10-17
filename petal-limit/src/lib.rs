use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{Limiter, Loudness, Settings, Style};

pub const MAX_LOOKAHEAD_MS: f32 = 10.0;

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum StyleParam {
    Transparent,
    Punchy,
    Aggressive,
    Safe,
}

impl From<StyleParam> for Style {
    fn from(s: StyleParam) -> Self {
        match s {
            StyleParam::Transparent => Style::Transparent,
            StyleParam::Punchy => Style::Punchy,
            StyleParam::Aggressive => Style::Aggressive,
            StyleParam::Safe => Style::Safe,
        }
    }
}

#[derive(Params)]
pub struct LimitParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "ceiling"]
    pub ceiling: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "lookahead"]
    pub lookahead: FloatParam,
    #[id = "link"]
    pub link: FloatParam,
    #[id = "style"]
    pub style: EnumParam<StyleParam>,
    #[id = "true_peak"]
    pub true_peak: BoolParam,
    #[id = "unity"]
    pub unity: BoolParam,
}

impl Default for LimitParams {
    fn default() -> Self {
        let db = |name: &str, default: f32, min: f32, max: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min, max })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_step_size(0.1)
                .with_unit(" dB")
                .with_value_to_string(formatters::v2s_f32_rounded(1))
        };
        Self {
            editor_state: editor::default_state(),
            // The standalone preview starts driven so the limiting is visible.
            gain: db("Gain", if cfg!(feature = "standalone") { 6.0 } else { 0.0 }, 0.0, 30.0),
            ceiling: db("Ceiling", -0.3, -24.0, 0.0),
            release: FloatParam::new("Release", 80.0, FloatRange::Skewed { min: 1.0, max: 1000.0, factor: FloatRange::skew_factor(-1.8) })
                .with_smoother(SmoothingStyle::Logarithmic(30.0))
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            // Changing it changes the latency, so no smoothing: it applies per block.
            lookahead: FloatParam::new("Lookahead", 2.0, FloatRange::Skewed { min: 0.1, max: MAX_LOOKAHEAD_MS, factor: FloatRange::skew_factor(-1.2) })
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            link: FloatParam::new("Link", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(30.0))
                .with_unit(" %")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            style: EnumParam::new("Style", StyleParam::Transparent),
            true_peak: BoolParam::new("True Peak", true),
            unity: BoolParam::new("Unity Gain", false),
        }
    }
}

pub const HISTORY: usize = 1024;

/// Live values for the editor: level history plus meters.
pub struct Shared {
    pub hist_in: Vec<AtomicU32>,
    pub hist_out: Vec<AtomicU32>,
    pub hist_gr: Vec<AtomicU32>,
    pub hist_pos: AtomicUsize,
    pub peak_in: [AtomicU32; 2],
    pub peak_out: [AtomicU32; 2],
    pub gr: AtomicU32,
    pub lufs_m: AtomicU32,
    pub lufs_s: AtomicU32,
}

impl Default for Shared {
    fn default() -> Self {
        let ring = || (0..HISTORY).map(|_| AtomicU32::new(0)).collect();
        Self {
            hist_in: ring(),
            hist_out: ring(),
            hist_gr: ring(),
            hist_pos: AtomicUsize::new(0),
            peak_in: Default::default(),
            peak_out: Default::default(),
            gr: AtomicU32::new(0),
            lufs_m: AtomicU32::new((-70f32).to_bits()),
            lufs_s: AtomicU32::new((-70f32).to_bits()),
        }
    }
}

impl Shared {
    pub fn load(a: &AtomicU32) -> f32 {
        f32::from_bits(a.load(Relaxed))
    }
}

pub struct PetalLimit {
    params: Arc<LimitParams>,
    shared: Arc<Shared>,
    limiter: Limiter,
    loudness: Loudness,
    sample_rate: f32,
    latency: usize,
    frame_len: usize,
    frame: (usize, f32, f32, f32),
    #[cfg(feature = "standalone")]
    demo: demo::Beat,
}

impl Default for PetalLimit {
    fn default() -> Self {
        Self {
            params: Arc::new(LimitParams::default()),
            shared: Arc::new(Shared::default()),
            limiter: Limiter::new(48_000.0, MAX_LOOKAHEAD_MS),
            loudness: Loudness::new(48_000.0),
            sample_rate: 48_000.0,
            latency: 0,
            frame_len: 400,
            frame: (0, 0.0, 0.0, 0.0),
            #[cfg(feature = "standalone")]
            demo: demo::Beat::new(48_000.0),
        }
    }
}

impl Plugin for PetalLimit {
    const NAME: &'static str = "Petal Limit";
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
        self.limiter = Limiter::new(self.sample_rate, MAX_LOOKAHEAD_MS);
        self.limiter.set_lookahead_ms(self.params.lookahead.value());
        self.loudness = Loudness::new(self.sample_rate);
        self.latency = self.limiter.latency();
        context.set_latency_samples(self.latency as u32);
        self.frame_len = (self.sample_rate / 120.0) as usize;
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Beat::new(self.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        self.limiter.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let want = self.limiter.latency_for(p.lookahead.value());
        if want != self.latency {
            self.limiter.set_lookahead_ms(p.lookahead.value());
            self.latency = self.limiter.latency();
            context.set_latency_samples(self.latency as u32);
        }
        let style: Style = p.style.value().into();
        let true_peak = p.true_peak.value();
        let unity = p.unity.value();
        let mut peak_in = [0.0f32; 2];
        let mut peak_out = [0.0f32; 2];

        for mut frame in buffer.iter_samples() {
            let s = Settings {
                gain_db: p.gain.smoothed.next(),
                ceiling_db: p.ceiling.smoothed.next(),
                release_ms: p.release.smoothed.next(),
                link: p.link.smoothed.next(),
                style,
                true_peak,
            };
            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let mut x = [*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap()];
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                x = [x[0] + d, x[1] + d];
            }
            let mut y = self.limiter.process(x, &s);
            if unity {
                // Listen at matched level: undo the input gain after limiting.
                let g = 10f32.powf(-s.gain_db / 20.0);
                y = [y[0] * g, y[1] * g];
            }
            *frame.get_mut(0).unwrap() = y[0];
            *frame.get_mut(1).unwrap() = y[1];
            self.loudness.push(y);

            let drive = 10f32.powf(s.gain_db / 20.0);
            for ch in 0..2 {
                peak_in[ch] = peak_in[ch].max((x[ch] * drive).abs());
                peak_out[ch] = peak_out[ch].max(y[ch].abs());
            }
            // History frames for the scrolling display.
            let f = &mut self.frame;
            f.0 += 1;
            f.1 = f.1.max((x[0] * drive).abs().max((x[1] * drive).abs()));
            f.2 = f.2.max(y[0].abs().max(y[1].abs()));
            f.3 = f.3.max(self.limiter.reduction_db);
            if f.0 >= self.frame_len {
                let i = self.shared.hist_pos.load(Relaxed) % HISTORY;
                self.shared.hist_in[i].store(f.1.to_bits(), Relaxed);
                self.shared.hist_out[i].store(f.2.to_bits(), Relaxed);
                self.shared.hist_gr[i].store(f.3.to_bits(), Relaxed);
                self.shared.hist_pos.fetch_add(1, Relaxed);
                *f = (0, 0.0, 0.0, 0.0);
            }
        }
        for ch in 0..2 {
            self.shared.peak_in[ch].store(peak_in[ch].to_bits(), Relaxed);
            self.shared.peak_out[ch].store(peak_out[ch].to_bits(), Relaxed);
        }
        self.shared.gr.store(self.limiter.reduction_db.to_bits(), Relaxed);
        self.shared.lufs_m.store(self.loudness.momentary().to_bits(), Relaxed);
        self.shared.lufs_s.store(self.loudness.short_term().to_bits(), Relaxed);
        ProcessStatus::Normal
    }
}

impl ClapPlugin for PetalLimit {
    const CLAP_ID: &'static str = "com.b4demus.petal-limit";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Look-ahead brickwall limiter");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::Limiter, ClapFeature::Mastering];
}

impl Vst3Plugin for PetalLimit {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalLimit01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Dynamics, Vst3SubCategory::Mastering];
}

nih_export_clap!(PetalLimit);
nih_export_vst3!(PetalLimit);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: a loud kick + chord loop.
    pub struct Beat {
        sr: f32,
        t: u64,
    }

    impl Beat {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0 }
        }

        pub fn next(&mut self) -> f32 {
            let t = self.t as f32 / self.sr;
            self.t += 1;
            let beat = 60.0 / 120.0;
            let k = t % beat;
            let kick = if k < 0.3 {
                let f = 50.0 + 120.0 * (-k * 30.0).exp();
                (std::f32::consts::TAU * f * k).sin() * (-k * 8.0).exp()
            } else {
                0.0
            };
            let chord: f32 = [196.0, 246.94, 293.66]
                .iter()
                .map(|f| (std::f32::consts::TAU * f * t).sin())
                .sum();
            kick * 0.9 + chord * 0.12
        }
    }
}
