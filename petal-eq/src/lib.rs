use nih_plug::prelude::*;
use std::sync::Arc;

pub mod analyzer;
pub mod dsp;
mod editor;

use dsp::{BandSpec, BandState, Design, Kind};

pub const NUM_BANDS: usize = 8;
/// Coefficients are recomputed from the smoothed parameters this often.
const BLOCK: usize = 32;

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum BandKind {
    Bell,
    #[name = "Low Shelf"]
    LowShelf,
    #[name = "High Shelf"]
    HighShelf,
    #[name = "Low Cut"]
    LowCut,
    #[name = "High Cut"]
    HighCut,
    Notch,
    #[name = "Band Pass"]
    BandPass,
}

impl From<BandKind> for Kind {
    fn from(k: BandKind) -> Self {
        match k {
            BandKind::Bell => Kind::Bell,
            BandKind::LowShelf => Kind::LowShelf,
            BandKind::HighShelf => Kind::HighShelf,
            BandKind::LowCut => Kind::LowCut,
            BandKind::HighCut => Kind::HighCut,
            BandKind::Notch => Kind::Notch,
            BandKind::BandPass => Kind::BandPass,
        }
    }
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Slope {
    #[name = "6 dB/oct"]
    S6,
    #[name = "12 dB/oct"]
    S12,
    #[name = "18 dB/oct"]
    S18,
    #[name = "24 dB/oct"]
    S24,
    #[name = "36 dB/oct"]
    S36,
    #[name = "48 dB/oct"]
    S48,
}

impl Slope {
    pub const ALL: [Slope; 6] = [Slope::S6, Slope::S12, Slope::S18, Slope::S24, Slope::S36, Slope::S48];

    pub fn db(self) -> u32 {
        match self {
            Slope::S6 => 6,
            Slope::S12 => 12,
            Slope::S18 => 18,
            Slope::S24 => 24,
            Slope::S36 => 36,
            Slope::S48 => 48,
        }
    }
}

#[derive(Params)]
pub struct BandParams {
    #[id = "on"]
    pub enabled: BoolParam,
    #[id = "type"]
    pub kind: EnumParam<BandKind>,
    #[id = "freq"]
    pub freq: FloatParam,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "q"]
    pub q: FloatParam,
    #[id = "slope"]
    pub slope: EnumParam<Slope>,
}

impl BandParams {
    fn new(index: usize) -> Self {
        const FREQS: [f32; NUM_BANDS] = [40.0, 120.0, 350.0, 1000.0, 2500.0, 5000.0, 9000.0, 16000.0];
        let n = index + 1;
        // The standalone preview starts with a few bands so there is something to see.
        let demo: Option<(BandKind, f32, f32, f32)> = if cfg!(feature = "standalone") {
            [
                Some((BandKind::LowCut, 45.0, 0.0, 0.707)),
                Some((BandKind::Bell, 320.0, -4.5, 1.4)),
                Some((BandKind::Bell, 2400.0, 5.0, 0.9)),
                Some((BandKind::HighShelf, 9000.0, 3.0, 0.707)),
            ]
            .get(index)
            .copied()
            .flatten()
        } else {
            None
        };
        let (kind, freq, gain, q) = demo.unwrap_or((BandKind::Bell, FREQS[index], 0.0, 1.0));

        Self {
            enabled: BoolParam::new(format!("Band {n} On"), demo.is_some()),
            kind: EnumParam::new(format!("Band {n} Type"), kind),
            freq: FloatParam::new(
                format!("Band {n} Freq"),
                freq,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 30_000.0,
                    factor: FloatRange::skew_factor(-2.2),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(25.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(2))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            gain: FloatParam::new(
                format!("Band {n} Gain"),
                gain,
                FloatRange::Linear { min: -30.0, max: 30.0 },
            )
            .with_smoother(SmoothingStyle::Linear(25.0))
            .with_step_size(0.01)
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            q: FloatParam::new(
                format!("Band {n} Q"),
                q,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 30.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(25.0))
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            slope: EnumParam::new(format!("Band {n} Slope"), Slope::S24),
        }
    }

    pub fn spec(&self) -> BandSpec {
        BandSpec {
            kind: self.kind.value().into(),
            freq: self.freq.value(),
            gain_db: self.gain.value(),
            q: self.q.value(),
            slope_db: self.slope.value().db(),
        }
    }

    fn smoothed_spec(&self, steps: u32) -> BandSpec {
        BandSpec {
            kind: self.kind.value().into(),
            freq: self.freq.smoothed.next_step(steps),
            gain_db: self.gain.smoothed.next_step(steps),
            q: self.q.smoothed.next_step(steps),
            slope_db: self.slope.value().db(),
        }
    }
}

#[derive(Params)]
pub struct EqParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<nih_plug_egui::EguiState>,

    #[nested(array, group = "Bands")]
    pub bands: [BandParams; NUM_BANDS],

    #[id = "output"]
    pub output: FloatParam,
}

impl Default for EqParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            bands: std::array::from_fn(BandParams::new),
            output: FloatParam::new(
                "Output",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-24.0),
                    max: util::db_to_gain(24.0),
                    factor: FloatRange::gain_skew_factor(-24.0, 24.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(30.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

pub struct PetalEq {
    params: Arc<EqParams>,
    shared: Arc<analyzer::Shared>,
    sample_rate: f32,
    designs: [Design; NUM_BANDS],
    specs: [Option<BandSpec>; NUM_BANDS],
    states: [BandState; NUM_BANDS],
    active: [bool; NUM_BANDS],
    // Audition path.
    audition_band: Option<usize>,
    audition_spec: Option<BandSpec>,
    audition_design: Design,
    audition_state: BandState,
    audition_mix: f32,
    #[cfg(feature = "standalone")]
    demo: demo::Source,
}

impl Default for PetalEq {
    fn default() -> Self {
        let blank = Design::new(
            &BandSpec { kind: Kind::Bell, freq: 1000.0, gain_db: 0.0, q: 1.0, slope_db: 12 },
            48_000.0,
        );
        Self {
            params: Arc::new(EqParams::default()),
            shared: Arc::new(analyzer::Shared::default()),
            sample_rate: 48_000.0,
            designs: [blank; NUM_BANDS],
            specs: [None; NUM_BANDS],
            states: [BandState::default(); NUM_BANDS],
            active: [false; NUM_BANDS],
            audition_band: None,
            audition_spec: None,
            audition_design: blank,
            audition_state: BandState::default(),
            audition_mix: 0.0,
            #[cfg(feature = "standalone")]
            demo: demo::Source::new(48_000.0),
        }
    }
}

impl PetalEq {
    fn update_filters(&mut self, steps: usize) {
        for b in 0..NUM_BANDS {
            let p = &self.params.bands[b];
            let enabled = p.enabled.value();
            if enabled && !self.active[b] {
                self.states[b].reset();
                self.specs[b] = None;
            }
            self.active[b] = enabled;
            let spec = p.smoothed_spec(steps as u32);
            if self.specs[b] != Some(spec) {
                self.designs[b] = Design::new(&spec, self.sample_rate);
                self.specs[b] = Some(spec);
            }
        }

        // Keep the last auditioned band while fading out.
        if let Some(b) = self.shared.solo().filter(|&b| b < NUM_BANDS && self.active[b]) {
            if self.audition_band != Some(b) {
                self.audition_state.reset();
                self.audition_band = Some(b);
                self.audition_spec = None;
            }
        }
        if let Some(b) = self.audition_band {
            let spec = self.specs[b].unwrap_or_else(|| self.params.bands[b].spec());
            if self.audition_spec != Some(spec) {
                self.audition_design = dsp::audition_design(&spec, self.sample_rate);
                self.audition_spec = Some(spec);
            }
        }
    }
}

impl Plugin for PetalEq {
    const NAME: &'static str = "Petal EQ";
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
        self.shared.set_sample_rate(self.sample_rate);
        self.specs = [None; NUM_BANDS];
        self.audition_spec = None;
        #[cfg(feature = "standalone")]
        {
            self.demo = demo::Source::new(self.sample_rate);
        }
        true
    }

    fn reset(&mut self) {
        for s in &mut self.states {
            s.reset();
        }
        self.audition_state.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let samples = buffer.samples();
        let channels = buffer.as_slice();
        let fade_step = 1.0 / (0.015 * self.sample_rate);

        let mut start = 0;
        while start < samples {
            let len = (samples - start).min(BLOCK);
            self.update_filters(len);
            let soloing = self.shared.solo().is_some_and(|b| b < NUM_BANDS && self.active[b]);

            for i in start..start + len {
                #[cfg_attr(not(feature = "standalone"), allow(unused_mut))]
                let mut x = [channels[0][i] as f64, channels[1][i] as f64];
                #[cfg(feature = "standalone")]
                {
                    let d = self.demo.next() as f64;
                    x[0] += d;
                    x[1] += d;
                }

                let mut y = x;
                for b in 0..NUM_BANDS {
                    if self.active[b] {
                        for ch in 0..2 {
                            y[ch] = self.states[b].process(&self.designs[b], ch, y[ch]);
                        }
                    }
                }

                // Crossfade to the audition filter (fed from the dry input).
                let target = if soloing { 1.0 } else { 0.0 };
                if self.audition_mix != target {
                    self.audition_mix = if target > self.audition_mix {
                        (self.audition_mix + fade_step).min(1.0)
                    } else {
                        (self.audition_mix - fade_step).max(0.0)
                    };
                }
                if self.audition_mix > 0.0 {
                    let m = self.audition_mix as f64;
                    for ch in 0..2 {
                        let a = self.audition_state.process(&self.audition_design, ch, x[ch]);
                        y[ch] = y[ch] * (1.0 - m) + a * m;
                    }
                } else if self.audition_band.is_some() && !soloing {
                    self.audition_band = None;
                }

                let gain = self.params.output.smoothed.next() as f64;
                channels[0][i] = (y[0] * gain) as f32;
                channels[1][i] = (y[1] * gain) as f32;
                self.shared.push(
                    ((x[0] + x[1]) * 0.5) as f32,
                    ((channels[0][i] + channels[1][i]) * 0.5) as f32,
                );
            }
            start += len;
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for PetalEq {
    const CLAP_ID: &'static str = "com.b4demus.petal-eq";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Parametric EQ with band audition");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Equalizer,
        ClapFeature::Filter,
    ];
}

impl Vst3Plugin for PetalEq {
    const VST3_CLASS_ID: [u8; 16] = *b"ExtwPetalEqPro01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Eq, Vst3SubCategory::Stereo];
}

nih_export_clap!(PetalEq);
nih_export_vst3!(PetalEq);

#[cfg(feature = "standalone")]
mod demo {
    //! Test signal for the standalone preview: pink-ish noise plus soft chords.
    pub struct Source {
        sr: f32,
        t: u64,
        rng: u32,
        pink: [f32; 3],
    }

    impl Source {
        pub fn new(sr: f32) -> Self {
            Self { sr, t: 0, rng: 1, pink: [0.0; 3] }
        }

        pub fn next(&mut self) -> f32 {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let white = (self.rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
            // Paul Kellet's economy pink filter.
            self.pink[0] = 0.99765 * self.pink[0] + white * 0.0990460;
            self.pink[1] = 0.96300 * self.pink[1] + white * 0.2965164;
            self.pink[2] = 0.57000 * self.pink[2] + white * 1.0526913;
            let pink = (self.pink[0] + self.pink[1] + self.pink[2] + white * 0.1848) * 0.05;

            let t = self.t as f32 / self.sr;
            self.t += 1;
            let chord: f32 = [110.0, 220.0, 277.18, 329.63, 440.0]
                .iter()
                .map(|f| (std::f32::consts::TAU * f * t).sin())
                .sum();
            pink + chord * 0.04
        }
    }
}
