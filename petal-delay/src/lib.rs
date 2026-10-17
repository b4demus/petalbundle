use nih_plug::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

pub mod dsp;
mod editor;

use dsp::{Delay, Mode, Settings};

/// Longest delay: 1/1 dotted at 40 bpm plus offset still fits.
const MAX_SECONDS: f32 = 10.0;

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum FeedbackMode {
    Normal,
    Invert,
    #[name = "Ping-pong"]
    PingPong,
}

impl From<FeedbackMode> for Mode {
    fn from(m: FeedbackMode) -> Self {
        match m {
            FeedbackMode::Normal => Mode::Normal,
            FeedbackMode::Invert => Mode::Invert,
            FeedbackMode::PingPong => Mode::PingPong,
        }
    }
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Note {
    #[name = "1/64"]
    N64,
    #[name = "1/32"]
    N32,
    #[name = "1/16 T"]
    N16T,
    #[name = "1/16"]
    N16,
    #[name = "1/16 D"]
    N16D,
    #[name = "1/8 T"]
    N8T,
    #[name = "1/8"]
    N8,
    #[name = "1/8 D"]
    N8D,
    #[name = "1/4 T"]
    N4T,
    #[name = "1/4"]
    N4,
    #[name = "1/4 D"]
    N4D,
    #[name = "1/2"]
    N2,
    #[name = "1/2 D"]
    N2D,
    #[name = "1/1"]
    N1,
}

impl Note {
    /// Length in quarter notes (beats).
    pub fn beats(self) -> f32 {
        match self {
            Note::N64 => 1.0 / 16.0,
            Note::N32 => 1.0 / 8.0,
            Note::N16T => 1.0 / 6.0,
            Note::N16 => 0.25,
            Note::N16D => 0.375,
            Note::N8T => 1.0 / 3.0,
            Note::N8 => 0.5,
            Note::N8D => 0.75,
            Note::N4T => 2.0 / 3.0,
            Note::N4 => 1.0,
            Note::N4D => 1.5,
            Note::N2 => 2.0,
            Note::N2D => 3.0,
            Note::N1 => 4.0,
        }
    }
}

#[derive(Params)]
pub struct DelayParams {
    #[persist = "editor-state"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    // Input
    #[id = "in_pan"]
    pub in_pan: FloatParam,
    #[id = "in_vol"]
    pub in_vol: FloatParam,
    // Feedback
    #[id = "fb_mode"]
    pub fb_mode: EnumParam<FeedbackMode>,
    #[id = "fb_vol"]
    pub fb_vol: FloatParam,
    #[id = "fb_cut"]
    pub fb_cut: FloatParam,
    // Time
    #[id = "sync"]
    pub sync: BoolParam,
    #[id = "time_note"]
    pub time_note: EnumParam<Note>,
    #[id = "time_ms"]
    pub time_ms: FloatParam,
    #[id = "offset"]
    pub offset: FloatParam,
    // Dry
    #[id = "dry_vol"]
    pub dry_vol: FloatParam,
}

fn percent(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(20.0))
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

fn bipolar_percent(name: &str, default: f32, labels: (&'static str, &'static str)) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: -1.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(20.0))
        .with_value_to_string(Arc::new(move |v| {
            let pct = (v * 100.0).round();
            if pct == 0.0 {
                "C".to_owned()
            } else if pct < 0.0 {
                format!("{}{}", -pct, labels.0)
            } else {
                format!("{}{}", pct, labels.1)
            }
        }))
}

impl Default for DelayParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            // Hard left by default: ping-pong (the default mode) then bounces L -> R -> L.
            in_pan: bipolar_percent("Input Pan", -1.0, ("L", "R")),
            in_vol: percent("Input Vol", 0.8),
            fb_mode: EnumParam::new("Feedback Mode", FeedbackMode::PingPong),
            fb_vol: percent("Feedback Vol", 0.5),
            fb_cut: FloatParam::new(
                "Feedback Cut",
                9000.0,
                FloatRange::Skewed { min: 200.0, max: 20_000.0, factor: FloatRange::skew_factor(-2.0) },
            )
            .with_smoother(SmoothingStyle::Logarithmic(30.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            sync: BoolParam::new("Tempo Sync", true),
            time_note: EnumParam::new("Time (Sync)", Note::N8D),
            time_ms: FloatParam::new(
                "Time",
                375.0,
                FloatRange::Skewed { min: 1.0, max: 4000.0, factor: FloatRange::skew_factor(-1.5) },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            offset: FloatParam::new("Time Offset", 0.0, FloatRange::Linear { min: -1.0, max: 1.0 })
                .with_value_to_string(Arc::new(|v| format!("{:+.0}%", v * 100.0))),
            dry_vol: percent("Dry Vol", 1.0),
        }
    }
}

impl DelayParams {
    /// Base delay time in seconds for the given tempo.
    pub fn time_seconds(&self, bpm: f32) -> f32 {
        if self.sync.value() {
            self.time_note.value().beats() * 60.0 / bpm.max(20.0)
        } else {
            self.time_ms.value() * 0.001
        }
    }

    /// Left/right delay times: the offset spreads them apart by up to ±25 %.
    pub fn stereo_times(&self, bpm: f32) -> [f32; 2] {
        let t = self.time_seconds(bpm);
        let o = self.offset.value() * 0.25;
        [t * (1.0 - o), t * (1.0 + o)]
    }
}

/// Equal-power pan gains for -1..1.
pub fn pan_gains(pan: f32) -> [f32; 2] {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    [a.cos() * std::f32::consts::SQRT_2, a.sin() * std::f32::consts::SQRT_2]
}

/// Values the editor needs from the audio thread.
pub struct Shared {
    pub bpm: AtomicU32,
    /// Block peaks of the wet output (L, R) and of the dry input.
    pub level: [AtomicU32; 2],
    pub input: AtomicU32,
}

impl Default for Shared {
    fn default() -> Self {
        Self { bpm: AtomicU32::new(120f32.to_bits()), level: Default::default(), input: AtomicU32::new(0) }
    }
}

impl Shared {
    pub fn bpm(&self) -> f32 {
        f32::from_bits(self.bpm.load(Relaxed))
    }
}

pub struct PetalDelay {
    params: Arc<DelayParams>,
    shared: Arc<Shared>,
    delay: Delay,
    sample_rate: f32,
    #[cfg(feature = "standalone")]
    demo: demo::Blips,
}

impl Default for PetalDelay {
    fn default() -> Self {
        Self {
            params: Arc::new(DelayParams::default()),
            shared: Arc::new(Shared::default()),
            delay: Delay::new(48_000.0, MAX_SECONDS),
            sample_rate: 48_000.0,
            #[cfg(feature = "standalone")]
            demo: demo::Blips::new(48_000.0),
        }
    }
}

impl Plugin for PetalDelay {
    const NAME: &'static str = "Petal Delay";
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
        self.delay = Delay::new(self.sample_rate, MAX_SECONDS);
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Blips::new(self.sample_rate);
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
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let bpm = context.transport().tempo.unwrap_or(120.0) as f32;
        self.shared.bpm.store(bpm.to_bits(), Relaxed);

        let times = p.stereo_times(bpm);
        let mode: Mode = p.fb_mode.value().into();
        let mut level = [0.0f32; 2];
        let mut input_peak = 0.0f32;

        for mut frame in buffer.iter_samples() {
            let pan = pan_gains(p.in_pan.smoothed.next());
            let in_vol = p.in_vol.smoothed.next();
            let settings = Settings {
                time_l: times[0] * self.sample_rate,
                time_r: times[1] * self.sample_rate,
                feedback: p.fb_vol.smoothed.next(),
                cut_hz: p.fb_cut.smoothed.next(),
                mode,
            };
            let dry = p.dry_vol.smoothed.next();

            #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
            let (mut l, mut r) = (*frame.get_mut(0).unwrap(), *frame.get_mut(1).unwrap());
            #[cfg(feature = "standalone")]
            {
                let d = self.demo.next();
                l += d;
                r += d;
            }

            // Ping-pong starts from a mono sum so the bounce is clean; the
            // other modes keep the stereo image and pan acts as a balance.
            let input = if mode == Mode::PingPong {
                let m = (l + r) * 0.5 * in_vol;
                [m * pan[0], m * pan[1]]
            } else {
                [l * in_vol * pan[0], r * in_vol * pan[1]]
            };
            input_peak = input_peak.max(l.abs()).max(r.abs());
            let wet = self.delay.process(input, &settings);
            let out = [l * dry + wet[0], r * dry + wet[1]];
            *frame.get_mut(0).unwrap() = out[0];
            *frame.get_mut(1).unwrap() = out[1];
            level[0] = level[0].max(wet[0].abs());
            level[1] = level[1].max(wet[1].abs());
        }
        for ch in 0..2 {
            self.shared.level[ch].store(level[ch].to_bits(), Relaxed);
        }
        self.shared.input.store(input_peak.to_bits(), Relaxed);
        // Echoes keep ringing after the input stops.
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for PetalDelay {
    const CLAP_ID: &'static str = "com.b4demus.petal-delay";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Stereo delay with ping-pong feedback");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::Delay];
}

impl Vst3Plugin for PetalDelay {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalDelay01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Delay, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalDelay);
nih_export_vst3!(PetalDelay);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: short plucks every two beats.
    pub struct Blips {
        sr: f32,
        t: u64,
    }

    impl Blips {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0 }
        }

        pub fn next(&mut self) -> f32 {
            let t = self.t as f32 / self.sr;
            self.t += 1;
            let k = t % 1.0;
            let f = [523.25, 659.25, 783.99][(t as usize) % 3];
            (std::f32::consts::TAU * f * k).sin() * (-k * 14.0).exp() * 0.4
        }
    }
}
