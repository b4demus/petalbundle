use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, lerp_color, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Mesh, PointerButton, Pos2, Rect, RichText, Sense,
    Shape, Stroke, Ui,
};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::analyzer::{Analyzer, Shared};
use crate::dsp::{audition_design, Design, Kind};
use crate::{BandKind, BandParams, EqParams, Slope, NUM_BANDS};

const DESIGN: egui::Vec2 = vec2(720.0, 537.0);
const SCALE: f32 = 0.85;
const GRAPH_HEIGHT: f32 = 300.0;
const OUTPUT_CARD: f32 = 120.0;
/// Content height shared by the bottom cards (fits the tallest band layout).
const CARD_H: f32 = 124.0;

const F_MIN: f32 = 20.0;
const F_MAX: f32 = 20_000.0;
/// EQ curve range (± dB) and analyzer range (dB, after tilt).
const GAIN_RANGE: f32 = 18.0;
const SPEC_TOP: f32 = 6.0;
const SPEC_BOTTOM: f32 = -84.0;
const NODE_R: f32 = 8.0;
/// Display columns for curves and spectrum.
const COLUMNS: usize = 300;

const BAND_COLORS: [Color32; NUM_BANDS] = [MINT, PEACH, LAVENDER, PINK, SKY, BUTTER, SAGE, LILAC];

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

pub fn create(params: Arc<EqParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        EqEditor::new(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, ed| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal eq", "parametric equalizer", |ui| {
                    if pill(ui, "spectrum", ed.show_spectrum, LAVENDER, vec2(86.0, 26.0)).clicked() {
                        ed.show_spectrum = !ed.show_spectrum;
                    }
                });
                ui.add_space(10.0);
                ed.graph(ui, &params, setter, &shared);
                ui.add_space(14.0);

                let band_w = ui.available_width() - 12.0 - OUTPUT_CARD - 2.0 * CARD_CHROME;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ed.band_card(ui, band_w, &params, setter);
                    card_sized(ui, vec2(OUTPUT_CARD, CARD_H), "output", INK_SOFT, |ui| {
                        ui.add_space(22.0);
                        Knob::new(&params.output, setter, "Output", PINK).bipolar().show(ui);
                    });
                });
            });
            // Publish the audition target for the audio thread.
            let solo = ed.solo().map_or(-1, |b| b as i32);
            shared.solo.store(solo, Relaxed);
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

struct Drag {
    band: usize,
    audition: bool,
    /// Node position minus pointer position when grabbed (no jump on pickup).
    grab: egui::Vec2,
}

struct EqEditor {
    analyzer: Analyzer,
    freqs: Vec<f32>,
    selected: Option<usize>,
    drag: Option<Drag>,
    /// Node under the latest primary press, with its grab offset.
    pressed: Option<(usize, egui::Vec2)>,
    /// Band auditioned while the right mouse button is held on it.
    hold: Option<usize>,
    /// Band auditioned via the "listen" toggle.
    latch: Option<usize>,
    show_spectrum: bool,
}

impl EqEditor {
    fn new() -> Self {
        let freqs = (0..COLUMNS)
            .map(|i| F_MIN * (F_MAX / F_MIN).powf(i as f32 / (COLUMNS - 1) as f32))
            .collect();
        let mut ed = Self {
            analyzer: Analyzer::new(COLUMNS),
            freqs,
            selected: None,
            drag: None,
            pressed: None,
            hold: None,
            latch: None,
            show_spectrum: true,
        };
        // Standalone preview: PETAL_PREVIEW="select=<band>,listen=<band>" (1-based).
        if cfg!(feature = "standalone") {
            for part in std::env::var("PETAL_PREVIEW").unwrap_or_default().split(',') {
                let band = |v: &str| v.parse::<usize>().ok().and_then(|n| n.checked_sub(1));
                match part.split_once('=') {
                    Some(("select", v)) => ed.selected = band(v),
                    Some(("listen", v)) => ed.latch = band(v),
                    _ => {}
                }
            }
        }
        ed
    }

    fn solo(&self) -> Option<usize> {
        self.drag
            .as_ref()
            .filter(|d| d.audition)
            .map(|d| d.band)
            .or(self.hold)
            .or(self.latch)
    }

    // -----------------------------------------------------------------------
    // Graph
    // -----------------------------------------------------------------------

    fn graph(&mut self, ui: &mut Ui, params: &EqParams, setter: &ParamSetter, shared: &Shared) {
        let width = ui.available_width();
        let (rect, resp) =
            ui.allocate_exact_size(vec2(width, GRAPH_HEIGHT), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let map = Map::new(rect);
        let sr = shared.sample_rate() as f64;
        let dt = ui.input(|i| i.stable_dt).min(0.1);

        if self.show_spectrum {
            self.analyzer.update(shared, &self.freqs, dt);
        }

        // Current designs straight from the (unsmoothed) parameters.
        let specs: Vec<_> = params.bands.iter().map(BandParams::spec).collect();
        let enabled: Vec<bool> = params.bands.iter().map(|b| b.enabled.value()).collect();
        let designs: Vec<Design> = specs.iter().map(|s| Design::new(s, sr as f32)).collect();
        let band_db = |b: usize, f: f32| 20.0 * designs[b].magnitude(f as f64, sr).max(1e-6).log10() as f32;
        let total_db: Vec<f32> = self
            .freqs
            .iter()
            .map(|&f| (0..NUM_BANDS).filter(|&b| enabled[b]).map(|b| band_db(b, f)).sum())
            .collect();

        let node_pos = |b: usize| {
            let s = &specs[b];
            let y = if s.kind.has_gain() { map.y_gain(s.gain_db) } else { map.y_gain(0.0) };
            pos2(map.x(s.freq), y)
        };
        let pointer = resp.hover_pos();
        let node_at = |p: Pos2| -> Option<usize> {
            (0..NUM_BANDS)
                .filter(|&b| enabled[b])
                .map(|b| (b, node_pos(b).distance(p)))
                .filter(|&(_, d)| d < NODE_R + 6.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(b, _)| b)
        };
        let hovered = pointer.and_then(node_at);

        self.interact(ui, &resp, &map, params, setter, &enabled, hovered);

        // --- painting -----------------------------------------------------
        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);
        self.grid(&painter, &map);

        if let Some(b) = self.solo().filter(|&b| enabled[b]) {
            self.audition_zone(&painter, &map, &specs[b], b, sr);
        }

        if self.show_spectrum {
            let pre: Vec<Pos2> = self
                .freqs
                .iter()
                .zip(&self.analyzer.pre_db)
                .map(|(&f, &db)| pos2(map.x(f), map.y_spec(db)))
                .collect();
            painter.add(Shape::line(pre, Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.35))));
            let post: Vec<(f32, f32)> = self
                .freqs
                .iter()
                .zip(&self.analyzer.post_db)
                .map(|(&f, &db)| (map.x(f), map.y_spec(db)))
                .collect();
            fill_to(&painter, &post, rect.bottom(), LAVENDER.gamma_multiply(0.28), LAVENDER.gamma_multiply(0.02));
            let line: Vec<Pos2> = post.iter().map(|&(x, y)| pos2(x, y)).collect();
            painter.add(Shape::line(line, Stroke::new(1.0_f32, LAVENDER.gamma_multiply(0.7))));
        }

        // Curve of the selected / hovered band.
        for b in (0..NUM_BANDS).filter(|&b| enabled[b]) {
            let focus = Some(b) == self.selected || Some(b) == hovered;
            if !focus {
                continue;
            }
            let pts: Vec<(f32, f32)> = self
                .freqs
                .iter()
                .map(|&f| (map.x(f), map.y_gain(band_db(b, f))))
                .collect();
            let color = BAND_COLORS[b];
            fill_to(&painter, &pts, map.y_gain(0.0), color.gamma_multiply(0.35), color.gamma_multiply(0.35));
            let line: Vec<Pos2> = pts.iter().map(|&(x, y)| pos2(x, y)).collect();
            painter.add(Shape::line(line, Stroke::new(1.3_f32, color.gamma_multiply(0.95))));
        }

        // Summed curve with a soft halo.
        let total: Vec<Pos2> = self
            .freqs
            .iter()
            .zip(&total_db)
            .map(|(&f, &db)| pos2(map.x(f), map.y_gain(db)))
            .collect();
        painter.add(Shape::line(total.clone(), Stroke::new(6.0_f32, Color32::from_white_alpha(120))));
        painter.add(Shape::line(total, Stroke::new(2.2_f32, INK.gamma_multiply(0.85))));

        // Nodes.
        for b in (0..NUM_BANDS).filter(|&b| enabled[b]) {
            let p = node_pos(b);
            let color = BAND_COLORS[b];
            let active = Some(b) == self.selected;
            let r = if Some(b) == hovered || active { NODE_R + 1.5 } else { NODE_R };
            if Some(b) == self.solo() {
                let pulse = 0.5 + 0.5 * (ui.input(|i| i.time) as f32 * 5.0).sin();
                painter.circle_stroke(p, r + 6.0 + pulse * 3.0, Stroke::new(2.0_f32, color.gamma_multiply(0.6)));
            }
            if active {
                painter.circle_filled(p, r + 5.0, color.gamma_multiply(0.25));
            }
            painter.circle_filled(p + vec2(0.0, 1.5), r, SHADOW);
            painter.circle_filled(p, r, color);
            painter.circle_stroke(p, r, Stroke::new(2.0_f32, Color32::WHITE));
            painter.text(p, Align2::CENTER_CENTER, (b + 1).to_string(), FontId::proportional(10.0), Color32::WHITE);
        }

        // Readout for the band being dragged or hovered.
        if let Some(b) = self.drag.as_ref().map(|d| d.band).or(hovered) {
            readout(&painter, &map, node_pos(b), &specs[b], BAND_COLORS[b]);
        } else if let Some(p) = pointer.filter(|_| self.drag.is_none()) {
            let text = format!("{} Hz", fmt_hz(map.freq(p.x)));
            painter.text(p + vec2(10.0, -10.0), Align2::LEFT_BOTTOM, text, FontId::proportional(10.5), INK_SOFT);
        }

        if !enabled.iter().any(|&e| e) {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "double-click to plant a band",
                FontId::proportional(15.0),
                INK_SOFT.gamma_multiply(0.8),
            );
        }
        if let Some(b) = self.solo().filter(|&b| enabled[b]) {
            painter.text(
                rect.left_top() + vec2(14.0, 12.0),
                Align2::LEFT_TOP,
                format!("🎧 listening to band {}", b + 1),
                FontId::proportional(12.0),
                bundle_ui::darken(BAND_COLORS[b], 0.25),
            );
        }

        panel_frame(&painter, rect);
    }

    #[allow(clippy::too_many_arguments)]
    fn interact(
        &mut self,
        ui: &Ui,
        resp: &egui::Response,
        map: &Map,
        params: &EqParams,
        setter: &ParamSetter,
        enabled: &[bool],
        hovered: Option<usize>,
    ) {
        let (alt, shift, primary_pressed, secondary_pressed, secondary_down, scroll) = ui.input(|i| {
            (
                i.modifiers.alt,
                i.modifiers.shift,
                i.pointer.primary_pressed(),
                i.pointer.secondary_pressed(),
                i.pointer.secondary_down(),
                i.smooth_scroll_delta.y,
            )
        });
        // Positions come from the response (widget space), never raw input,
        // so hit-testing stays right whatever the window scaling is.
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());

        // Remember which node a press landed on; a drag starts a few pixels later.
        if primary_pressed && resp.hovered() {
            self.pressed = hovered.map(|b| {
                let p = &params.bands[b];
                let node = pos2(map.x(p.freq.value()), node_y(map, p));
                (b, node - pointer.unwrap_or(node))
            });
        }

        // Right mouse held on a node: listen while held.
        if secondary_pressed && resp.hovered() {
            self.hold = hovered;
            if hovered.is_some() {
                self.selected = hovered;
            }
        }
        if !secondary_down {
            self.hold = None;
        }

        // Dragging a node: x = frequency, y = gain. Alt = listen while dragging.
        if resp.drag_started_by(PointerButton::Primary) {
            if let Some((b, grab)) = self.pressed {
                let p = &params.bands[b];
                setter.begin_set_parameter(&p.freq);
                setter.begin_set_parameter(&p.gain);
                self.drag = Some(Drag { band: b, audition: alt, grab });
                self.selected = Some(b);
            }
        }
        if let (Some(drag), Some(pos)) = (&self.drag, pointer) {
            let p = &params.bands[drag.band];
            let has_gain = Kind::from(p.kind.value()).has_gain();
            let (x, y) = if shift {
                // Shift: slow, relative movement for precise moves.
                let d = resp.drag_delta() * 0.2;
                (map.x(p.freq.value()) + d.x, map.y_gain(p.gain.value()) + d.y)
            } else {
                let at = pos + drag.grab;
                (at.x, at.y)
            };
            setter.set_parameter(&p.freq, map.freq(x).clamp(10.0, 30_000.0));
            if has_gain {
                setter.set_parameter(&p.gain, map.gain(y).clamp(-30.0, 30.0));
            }
        }
        if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            self.pressed = None;
            if let Some(drag) = self.drag.take() {
                let p = &params.bands[drag.band];
                setter.end_set_parameter(&p.freq);
                setter.end_set_parameter(&p.gain);
            }
        }

        if resp.double_clicked() {
            match hovered {
                // Double-click a node removes it.
                Some(b) => {
                    set_now(setter, &params.bands[b].enabled, false);
                    if self.latch == Some(b) {
                        self.latch = None;
                    }
                    self.selected = None;
                }
                None => {
                    if let Some(pos) = pointer {
                        self.selected = add_band(params, setter, enabled, map.freq(pos.x), map.gain(pos.y));
                    }
                }
            }
        } else if resp.clicked() {
            self.selected = hovered;
        }

        // Scroll: Q of the hovered (or selected) band.
        if resp.hovered() && scroll.abs() > 0.0 {
            if let Some(b) = hovered.or(self.selected).filter(|&b| enabled[b]) {
                let q = &params.bands[b].q;
                let v = (q.value() * (scroll * 0.004).exp()).clamp(0.1, 30.0);
                set_now(setter, q, v);
            }
        }
    }

    fn grid(&self, painter: &egui::Painter, map: &Map) {
        let line = |a: Pos2, b: Pos2, alpha: u8| {
            painter.line_segment([a, b], Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(150, 140, 180, alpha)));
        };
        let r = map.rect;
        for decade in [10.0f32, 100.0, 1000.0, 10_000.0] {
            for m in 1..10 {
                let f = decade * m as f32;
                if !(F_MIN..=F_MAX).contains(&f) {
                    continue;
                }
                let x = map.x(f);
                line(pos2(x, r.top()), pos2(x, r.bottom()), if m == 1 { 40 } else { 9 });
                if matches!(m, 1 | 2 | 5) && f > 20.0 {
                    bundle_ui::label_text(painter, pos2(x + 3.0, r.bottom() - 8.0), Align2::LEFT_CENTER, fmt_hz(f), 9.5);
                }
            }
        }
        for g in [-12.0f32, -6.0, 0.0, 6.0, 12.0] {
            let y = map.y_gain(g);
            line(pos2(r.left(), y), pos2(r.right(), y), if g == 0.0 { 55 } else { 22 });
            let label = if g == 0.0 { "0".to_owned() } else { format!("{g:+}") };
            bundle_ui::label_text(painter, pos2(r.right() - 8.0, y - 7.0), Align2::RIGHT_CENTER, label, 9.5);
        }
    }

    /// Shades the part of the spectrum the auditioned band lets through.
    fn audition_zone(&self, painter: &egui::Painter, map: &Map, spec: &crate::dsp::BandSpec, band: usize, sr: f64) {
        let design = audition_design(spec, sr as f32);
        let color = BAND_COLORS[band];
        let r = map.rect;
        let mut mesh = Mesh::default();
        for (i, &f) in self.freqs.iter().enumerate() {
            let a = design.magnitude(f as f64, sr).min(1.0) as f32;
            let c = color.gamma_multiply(0.28 * a);
            let x = map.x(f);
            mesh.colored_vertex(pos2(x, r.top()), c);
            mesh.colored_vertex(pos2(x, r.bottom()), c);
            if i > 0 {
                let k = i as u32 * 2;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
        }
        painter.add(Shape::mesh(mesh));
    }

    // -----------------------------------------------------------------------
    // Band panel
    // -----------------------------------------------------------------------

    fn band_card(&mut self, ui: &mut Ui, width: f32, params: &EqParams, setter: &ParamSetter) {
        let Some(b) = self.selected.filter(|&b| params.bands[b].enabled.value()) else {
            card_sized(ui, vec2(width, CARD_H), "band", INK_SOFT, |ui| {
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    for line in [
                        "double-click the graph to plant a band",
                        "drag a node to move it, scroll over it for Q",
                        "hold right mouse (or Alt + drag) on a node to listen to it",
                        "double-click a node to remove it",
                    ] {
                        ui.label(RichText::new(line).size(12.5).color(INK_SOFT));
                    }
                    ui.add_space(30.0);
                });
            });
            return;
        };

        let p = &params.bands[b];
        let color = BAND_COLORS[b];
        let kind = p.kind.value();
        let title = format!("band {} · {}", b + 1, kind_label(kind));
        card_sized(ui, vec2(width, CARD_H), &title, color, |ui| {
            Knob::new(&p.freq, setter, "Freq", color).show(ui);
            Knob::new(&p.gain, setter, "Gain", color).bipolar().show(ui);
            Knob::new(&p.q, setter, "Q", color).show(ui);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
                const KINDS: [[BandKind; 4]; 2] = [
                    [BandKind::Bell, BandKind::LowShelf, BandKind::HighShelf, BandKind::Notch],
                    [BandKind::LowCut, BandKind::HighCut, BandKind::BandPass, BandKind::BandPass],
                ];
                for (r, row) in KINDS.iter().enumerate() {
                    ui.horizontal(|ui| {
                        for (i, &k) in row.iter().enumerate() {
                            if r == 1 && i == 3 {
                                break;
                            }
                            if pill(ui, kind_label(k), kind == k, color, vec2(64.0, 22.0)).clicked() {
                                set_now(setter, &p.kind, k);
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if Kind::from(kind).has_slope() {
                        let slope = p.slope.value();
                        for s in Slope::ALL {
                            let label = s.db().to_string();
                            if pill(ui, &label, slope == s, color, vec2(34.0, 22.0)).clicked() {
                                set_now(setter, &p.slope, s);
                            }
                        }
                        ui.label(RichText::new("dB/oct").size(10.5).color(INK_SOFT));
                    } else {
                        ui.add_space(22.0);
                    }
                });
                ui.horizontal(|ui| {
                    let listening = self.latch == Some(b);
                    if pill(ui, "🎧 listen", listening, color, vec2(84.0, 22.0)).clicked() {
                        self.latch = if listening { None } else { Some(b) };
                    }
                    if pill(ui, "remove", false, color, vec2(64.0, 22.0)).clicked() {
                        set_now(setter, &p.enabled, false);
                        if listening {
                            self.latch = None;
                        }
                        self.selected = None;
                    }
                });
            });
        });
    }
}

/// Screen mapping for the graph.
struct Map {
    rect: Rect,
}

impl Map {
    fn new(rect: Rect) -> Self {
        Self { rect }
    }

    fn x(&self, f: f32) -> f32 {
        let t = (f.max(1.0) / F_MIN).ln() / (F_MAX / F_MIN).ln();
        self.rect.left() + t * self.rect.width()
    }

    fn freq(&self, x: f32) -> f32 {
        let t = (x - self.rect.left()) / self.rect.width();
        F_MIN * (F_MAX / F_MIN).powf(t)
    }

    fn y_gain(&self, db: f32) -> f32 {
        let half = self.rect.height() * 0.5 - 16.0;
        self.rect.center().y - db / GAIN_RANGE * half
    }

    fn gain(&self, y: f32) -> f32 {
        let half = self.rect.height() * 0.5 - 16.0;
        (self.rect.center().y - y) / half * GAIN_RANGE
    }

    fn y_spec(&self, db: f32) -> f32 {
        let t = (SPEC_TOP - db) / (SPEC_TOP - SPEC_BOTTOM);
        self.rect.top() + t.clamp(0.0, 1.05) * self.rect.height()
    }
}

/// Vertical node position: its gain, or the 0 dB line for gainless types.
fn node_y(map: &Map, p: &BandParams) -> f32 {
    if Kind::from(p.kind.value()).has_gain() {
        map.y_gain(p.gain.value())
    } else {
        map.y_gain(0.0)
    }
}

fn set_now<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

/// Enables the first free band at the given spot; returns its index.
fn add_band(params: &EqParams, setter: &ParamSetter, enabled: &[bool], freq: f32, gain: f32) -> Option<usize> {
    let b = (0..NUM_BANDS).find(|&b| !enabled[b])?;
    let p = &params.bands[b];
    let (kind, gain) = if freq < 30.0 {
        (BandKind::LowCut, 0.0)
    } else if freq > 15_000.0 {
        (BandKind::HighCut, 0.0)
    } else {
        (BandKind::Bell, gain)
    };
    set_now(setter, &p.kind, kind);
    set_now(setter, &p.freq, freq.clamp(10.0, 30_000.0));
    set_now(setter, &p.gain, gain.clamp(-30.0, 30.0));
    set_now(setter, &p.q, if kind == BandKind::Bell { 1.0 } else { 0.707 });
    set_now(setter, &p.slope, Slope::S24);
    set_now(setter, &p.enabled, true);
    Some(b)
}

/// Fills the area between a polyline and a horizontal baseline.
fn fill_to(painter: &egui::Painter, pts: &[(f32, f32)], base: f32, near: Color32, far: Color32) {
    if pts.len() < 2 {
        return;
    }
    let mut mesh = Mesh::default();
    for (i, &(x, y)) in pts.iter().enumerate() {
        mesh.colored_vertex(pos2(x, y), near);
        mesh.colored_vertex(pos2(x, base), far);
        if i > 0 {
            let k = i as u32 * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(Shape::mesh(mesh));
}

fn readout(painter: &egui::Painter, map: &Map, at: Pos2, spec: &crate::dsp::BandSpec, color: Color32) {
    let mut text = format!("{} Hz", fmt_hz(spec.freq));
    if spec.kind.has_gain() {
        text += &format!("  {:+.1} dB", spec.gain_db);
    }
    text += &format!("  Q {:.2}", spec.q);
    if spec.kind.has_slope() {
        text += &format!("  {} dB/oct", spec.slope_db);
    }
    let galley = painter.layout_no_wrap(text, FontId::proportional(11.0), INK);
    let size = galley.size() + vec2(14.0, 8.0);
    let mut pos = at + vec2(-size.x * 0.5, -NODE_R - 12.0 - size.y);
    if pos.y < map.rect.top() + 4.0 {
        pos.y = at.y + NODE_R + 12.0;
    }
    pos.x = pos.x.clamp(map.rect.left() + 4.0, map.rect.right() - size.x - 4.0);
    let bg = Rect::from_min_size(pos, size);
    painter.rect_filled(bg, 8.0, lerp_color(CARD, color, 0.18));
    painter.galley(bg.min + vec2(7.0, 4.0), galley, INK);
}

fn kind_label(k: BandKind) -> &'static str {
    match k {
        BandKind::Bell => "bell",
        BandKind::LowShelf => "low shelf",
        BandKind::HighShelf => "high shelf",
        BandKind::LowCut => "low cut",
        BandKind::HighCut => "high cut",
        BandKind::Notch => "notch",
        BandKind::BandPass => "band pass",
    }
}

fn fmt_hz(f: f32) -> String {
    if f >= 1000.0 {
        let k = f / 1000.0;
        if k >= 10.0 || (k - k.round()).abs() < 0.05 {
            format!("{:.0}k", k)
        } else {
            format!("{:.1}k", k)
        }
    } else {
        format!("{:.0}", f)
    }
}
