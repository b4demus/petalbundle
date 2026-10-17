use nih_plug::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{step_index, HalfTime, Settings};

/// Slowest tempo we size the buffer for.
const MIN_BPM: f32 = 30.0;

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Speed {
    #[name = "1/2"]
    Half,
    #[name = "1/3"]
    Third,
    #[name = "1/4"]
    Quarter,
    #[name = "1/8"]
    Eighth,
    #[name = "2/3"]
    TwoThirds,
}

impl Speed {
    pub const ALL: [Speed; 5] = [Speed::Half, Speed::Third, Speed::Quarter, Speed::Eighth, Speed::TwoThirds];

    pub fn ratio(self) -> f32 {
        match self {
            Speed::Half => 0.5,
            Speed::Third => 1.0 / 3.0,
            Speed::Quarter => 0.25,
            Speed::Eighth => 0.125,
            Speed::TwoThirds => 2.0 / 3.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Speed::Half => "1/2",
            Speed::Third => "1/3",
            Speed::Quarter => "1/4",
            Speed::Eighth => "1/8",
            Speed::TwoThirds => "2/3",
        }
    }
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Grid {
    #[name = "1/16"]
    Sixteenth,
    #[name = "1/8"]
    Eighth,
    #[name = "1/4"]
    Beat,
    #[name = "1/2"]
    TwoBeats,
    #[name = "1 bar"]
    Bar,
    #[name = "2 bars"]
    TwoBars,
}

impl Grid {
    pub const ALL: [Grid; 6] = [Grid::Sixteenth, Grid::Eighth, Grid::Beat, Grid::TwoBeats, Grid::Bar, Grid::TwoBars];

    pub fn beats(self) -> f64 {
        match self {
            Grid::Sixteenth => 0.25,
            Grid::Eighth => 0.5,
            Grid::Beat => 1.0,
            Grid::TwoBeats => 2.0,
            Grid::Bar => 4.0,
            Grid::TwoBars => 8.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Grid::Sixteenth => "1/16",
            Grid::Eighth => "1/8",
            Grid::Beat => "1/4",
            Grid::TwoBeats => "1/2",
            Grid::Bar => "1 bar",
            Grid::TwoBars => "2 bars",
        }
    }
}

#[derive(Params)]
pub struct HalfParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[id = "speed"]
    pub speed: EnumParam<Speed>,
    #[id = "grid"]
    pub grid: EnumParam<Grid>,
    #[id = "smooth"]
    pub smooth: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for HalfParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            speed: EnumParam::new("Speed", Speed::Half),
            grid: EnumParam::new("Grid", Grid::Bar),
            smooth: FloatParam::new("Smooth", 8.0, FloatRange::Skewed { min: 0.5, max: 60.0, factor: FloatRange::skew_factor(-1.5) })
                .with_unit(" ms")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            tone: FloatParam::new("Tone", 20_000.0, FloatRange::Skewed { min: 500.0, max: 20_000.0, factor: FloatRange::skew_factor(-2.0) })
                .with_smoother(SmoothingStyle::Logarithmic(30.0))
                .with_unit(" Hz")
                .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
                .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
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

pub const BINS: usize = 192;

/// Live values for the editor.
pub struct Shared {
    /// Position of the record head inside the current step, 0..1.
    pub progress: AtomicU32,
    /// Position of the playhead inside the recorded step, 0..1.
    pub playhead: AtomicU32,
    /// Peak envelope of the current step's input, BINS bins.
    pub bins: Vec<AtomicU32>,
    pub synced: AtomicBool,
    pub bpm: AtomicU32,
    pub step: AtomicU32,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            progress: AtomicU32::new(0),
            playhead: AtomicU32::new(0),
            bins: (0..BINS).map(|_| AtomicU32::new(0)).collect(),
            synced: AtomicBool::new(false),
            bpm: AtomicU32::new(120f32.to_bits()),
            step: AtomicU32::new(0),
        }
    }
}

impl Shared {
    pub fn load(a: &AtomicU32) -> f32 {
        f32::from_bits(a.load(Relaxed))
    }
}

pub struct PetalHalf {
    params: Arc<HalfParams>,
    shared: Arc<Shared>,
    half: HalfTime,
    sample_rate: f32,
    /// Free-running song position (beats) used when the host is stopped.
    free_beats: f64,
    last_step: Option<i64>,
    last_bin: usize,
    bin_peak: f32,
    #[cfg(feature = "standalone")]
    demo: demo::Drums,
}

impl Default for PetalHalf {
    fn default() -> Self {
        Self {
            params: Arc::new(HalfParams::default()),
            shared: Arc::new(Shared::default()),
            half: HalfTime::new(48_000.0, 8.0 * 60.0 / MIN_BPM + 1.0),
            sample_rate: 48_000.0,
            free_beats: 0.0,
            last_step: None,
            last_bin: 0,
            bin_peak: 0.0,
            #[cfg(feature = "standalone")]
            demo: demo::Drums::new(48_000.0),
        }
    }
}

impl Plugin for PetalHalf {
    const NAME: &'static str = "Petal Half";
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
        // Room for the longest step (2 bars) at the slowest tempo.
        self.half = HalfTime::new(self.sample_rate, 8.0 * 60.0 / MIN_BPM + 1.0);
        self.last_step = None;
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Drums::new(self.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        self.half.reset();
        self.last_step = None;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let transport = context.transport();
        let bpm = (transport.tempo.unwrap_or(120.0) as f32).max(MIN_BPM);
        let host_beats = transport.pos_beats().filter(|_| transport.playing);
        let synced = host_beats.is_some();
        self.shared.synced.store(synced, Relaxed);
        self.shared.bpm.store(bpm.to_bits(), Relaxed);

        let grid = p.grid.value().beats();
        let speed = p.speed.value().ratio();
        let beats_per_sample = bpm as f64 / 60.0 / self.sample_rate as f64;
        let step_samples = grid / beats_per_sample;
        let beats = host_beats.unwrap_or(self.free_beats);

        for (n, mut frame) in buffer.iter_samples().enumerate() {
            let pos = beats + n as f64 * beats_per_sample;
            let step = step_index(pos, grid);
            if self.last_step != Some(step) {
                self.last_step = Some(step);
                self.half.restart();
                self.shared.step.fetch_add(1, Relaxed);
                for b in &self.shared.bins {
                    b.store(0, Relaxed);
                }
                self.last_bin = 0;
                self.bin_peak = 0.0;
            }

            let s = Settings { speed, smooth_ms: p.smooth.value(), tone_hz: p.tone.smoothed.next() };
            let mix = p.mix.smoothed.next();
            let gain = 10f32.powf(p.output.smoothed.next() / 20.0);

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let mut x = [*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap()];
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next(pos as f32 * 60.0 / bpm);
                x = [x[0] + d, x[1] + d];
            }
            let wet = self.half.process(x, &s);
            *frame.get_mut(0).unwrap() = (x[0] + (wet[0] - x[0]) * mix) * gain;
            *frame.get_mut(1).unwrap() = (x[1] + (wet[1] - x[1]) * mix) * gain;

            // Envelope of the step being recorded, for the editor.
            let progress = ((pos / grid).fract()) as f32;
            let bin = ((progress * BINS as f32) as usize).min(BINS - 1);
            if bin != self.last_bin {
                self.shared.bins[self.last_bin].store(self.bin_peak.to_bits(), Relaxed);
                self.last_bin = bin;
                self.bin_peak = 0.0;
            }
            self.bin_peak = self.bin_peak.max(x[0].abs().max(x[1].abs()));
            self.shared.progress.store(progress.to_bits(), Relaxed);
        }
        let played = (self.half.playback_offset() * speed as f64 / step_samples) as f32;
        self.shared.playhead.store(played.clamp(0.0, 1.0).to_bits(), Relaxed);
        if !synced {
            self.free_beats = beats + buffer.samples() as f64 * beats_per_sample;
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for PetalHalf {
    const CLAP_ID: &'static str = "com.b4demus.petal-half";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Tempo-synced half-time effect");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::Glitch];
}

impl Vst3Plugin for PetalHalf {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalHalf001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::PitchShift, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalHalf);
nih_export_vst3!(PetalHalf);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: a busy drum groove.
    pub struct Drums {
        rng: u32,
    }

    impl Drums {
        pub fn new(_sr: f32) -> Self {
            Self { rng: 21 }
        }

        /// `t` is the song time in seconds.
        pub fn next(&mut self, t: f32) -> f32 {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let noise = (self.rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
            let beat = 0.5;
            let pos = t % (beat * 4.0);
            let mut out = 0.0;
            for kick in [0.0, beat * 1.5, beat * 2.5] {
                let k = pos - kick;
                if (0.0..0.3).contains(&k) {
                    let f = 50.0 + 110.0 * (-k * 30.0).exp();
                    out += (std::f32::consts::TAU * f * k).sin() * (-k * 9.0).exp() * 0.7;
                }
            }
            for snare in [beat, beat * 3.0] {
                let k = pos - snare;
                if (0.0..0.25).contains(&k) {
                    out += noise * (-k * 20.0).exp() * 0.4;
                }
            }
            let hat = pos % (beat / 2.0);
            if hat < 0.04 {
                out += noise * (-hat * 120.0).exp() * 0.15;
            }
            out
        }
    }
}
