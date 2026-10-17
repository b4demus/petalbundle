use bundle_ui::palette::*;
use bundle_ui::{card, darken, gradient, header, lerp_color, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, epaint::QuadraticBezierShape, pos2, vec2, Align2, Color32, FontId, Mesh, Pos2, Rect,
    Sense, Shape, Stroke, Ui,
};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::Arc;

use crate::engine::MAX_GRAINS;
use crate::viz::VizShared;
use crate::{GranulizerParams, PitchModeParam};

const DESIGN: egui::Vec2 = vec2(720.0, 577.0);
const SCALE: f32 = 0.85;
const FIELD_HEIGHT: f32 = 230.0;
/// Seconds of input history kept for the soil line.
const HISTORY_S: f64 = 9.0;

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

pub fn create(params: Arc<GranulizerParams>, viz: Arc<VizShared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        GrainField::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, field| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal granulizer", "granular texture", |ui| {
                    let on = params.freeze.value();
                    let label = if on { "❄  frozen" } else { "❄  freeze" };
                    if pill(ui, label, on, SKY, vec2(96.0, 28.0)).clicked() {
                        setter.begin_set_parameter(&params.freeze);
                        setter.set_parameter(&params.freeze, !on);
                        setter.end_set_parameter(&params.freeze);
                    }
                });
                ui.add_space(10.0);
                field.show(ui, &viz, &params);
                ui.add_space(14.0);
                // Two cards per row, together exactly as wide as the grain field.
                let card_w = (ui.available_width() - 12.0) / 2.0 - CARD_CHROME;

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card(ui, card_w, "grains", MINT, |ui| {
                        Knob::new(&params.density, setter, "Density", MINT).show(ui);
                        Knob::new(&params.size, setter, "Size", MINT).show(ui);
                        Knob::new(&params.jitter, setter, "Jitter", MINT).show(ui);
                        Knob::new(&params.shape, setter, "Shape", MINT).show(ui);
                    });
                    card(ui, card_w, "time", PEACH, |ui| {
                        Knob::new(&params.position, setter, "Position", PEACH).show(ui);
                        Knob::new(&params.spray, setter, "Spray", PEACH).show(ui);
                        Knob::new(&params.reverse, setter, "Reverse", PEACH).show(ui);
                        Knob::new(&params.feedback, setter, "Feedback", PEACH).show(ui);
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card(ui, card_w, "pitch", LAVENDER, |ui| {
                        Knob::new(&params.pitch, setter, "Pitch", LAVENDER)
                            .bipolar()
                            .show(ui);
                        Knob::new(&params.pitch_rand, setter, "Random", LAVENDER).show(ui);
                        pitch_modes(ui, &params, setter);
                    });
                    card(ui, card_w, "output", PINK, |ui| {
                        Knob::new(&params.spread, setter, "Spread", PINK).show(ui);
                        Knob::new(&params.mix, setter, "Mix", PINK).show(ui);
                        Knob::new(&params.gain, setter, "Output", PINK)
                            .bipolar()
                            .show(ui);
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

fn pitch_modes(ui: &mut Ui, params: &GranulizerParams, setter: &ParamSetter) {
    const MODES: [(PitchModeParam, &str); 4] = [
        (PitchModeParam::Free, "free"),
        (PitchModeParam::Octaves, "octaves"),
        (PitchModeParam::Fifths, "fifths"),
        (PitchModeParam::Pentatonic, "penta"),
    ];
    ui.add_space(10.0);
    egui::Grid::new("pitch_modes")
        .spacing([6.0, 6.0])
        .show(ui, |ui| {
            for (i, (mode, label)) in MODES.iter().enumerate() {
                let on = params.pitch_mode.value() == *mode;
                if pill(ui, label, on, LAVENDER, vec2(66.0, 24.0)).clicked() {
                    setter.begin_set_parameter(&params.pitch_mode);
                    setter.set_parameter(&params.pitch_mode, *mode);
                    setter.end_set_parameter(&params.pitch_mode);
                }
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
}

// ---------------------------------------------------------------------------
// Grain field: every grain is a flower growing out of the input "soil".
// x = how far back in time the grain reads, y = its pitch, bloom = its window.
// ---------------------------------------------------------------------------

struct Pollen {
    pos: Pos2,
    vel: egui::Vec2,
    life: f32,
    color: Color32,
}

#[derive(Clone, Copy)]
struct Seen {
    id: u32,
    pos: Pos2,
    color: Color32,
    radius: f32,
    rot: f32,
    reverse: bool,
    delay_s: f32,
    semitones: f32,
}

/// A finished grain wilting away, drifting into the past with the time axis.
struct Wilt {
    seen: Seen,
    /// Write-head time when the grain ended.
    ended_at: f64,
    life: f32,
}

/// How long a finished grain stays visible.
const WILT_S: f32 = 1.4;

pub struct GrainField {
    history: VecDeque<(f64, f32)>,
    view_s: f32,
    seen: [Option<Seen>; MAX_GRAINS],
    pollen: Vec<Pollen>,
    wilting: Vec<Wilt>,
    now: f64,
    rng: u32,
    clock: f32,
}

impl Default for GrainField {
    fn default() -> Self {
        Self {
            history: VecDeque::new(),
            view_s: 1.5,
            seen: [None; MAX_GRAINS],
            pollen: Vec::new(),
            wilting: Vec::new(),
            now: 0.0,
            rng: 0x1234_5678,
            clock: 0.0,
        }
    }
}

impl GrainField {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn show(&mut self, ui: &mut Ui, viz: &VizShared, params: &GranulizerParams) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_HEIGHT), Sense::hover());
        let painter = ui.painter_at(rect);
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        self.clock += dt;

        // Time window follows the settings so the flowers use the whole width.
        // Leaves room on the left for wilting grains to drift into.
        let reach = (params.position.value() + params.spray.value() + params.size.value() * 1.5)
            * 0.001
            + WILT_S * 0.6;
        let target = reach.clamp(1.2, 8.0);
        self.view_s += (target - self.view_s) * (1.0 - (-dt * 3.0).exp());

        // Input history for the soil.
        let now = viz.now_s();
        self.now = now;
        let peak = viz.take_peak();
        if self.history.back().map_or(true, |&(t, _)| now > t) {
            self.history.push_back((now, peak));
        }
        while self
            .history
            .front()
            .is_some_and(|&(t, _)| now - t > HISTORY_S)
        {
            self.history.pop_front();
        }

        // Sky.
        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);
        let x_now = rect.right() - 22.0;
        let x_span = x_now - rect.left() - 14.0;
        let ground = rect.bottom() - 18.0;
        let mid = rect.top() + (ground - rect.top()) * 0.5;
        let half = (ground - rect.top()) * 0.5 - 18.0;
        let view_s = self.view_s;
        let to_x = move |delay: f32| x_now - delay / view_s * x_span;
        let to_y = |st: f32| mid - (st / 24.0).clamp(-1.0, 1.0) * half;

        // Octave guides.
        for st in [-24.0f32, -12.0, 0.0, 12.0, 24.0] {
            let y = to_y(st);
            let alpha = if st == 0.0 { 60 } else { 30 };
            painter.line_segment(
                [pos2(rect.left() + 10.0, y), pos2(x_now, y)],
                Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(150, 140, 180, alpha),
                ),
            );
            let label = if st == 0.0 {
                "0".to_owned()
            } else {
                format!("{st:+}")
            };
            painter.text(
                pos2(rect.left() + 12.0, y - 7.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(9.5),
                INK_SOFT.gamma_multiply(0.7),
            );
        }

        // Soil: input loudness over the same time axis.
        let soil_pts: Vec<(f32, f32)> = self
            .history
            .iter()
            .map(|&(t, level)| (to_x((now - t) as f32), level.sqrt().min(1.0) * 34.0))
            .filter(|&(x, _)| x >= rect.left() - 4.0)
            .collect();
        soil(&painter, &soil_pts, ground, rect);

        // Live head.
        painter.line_segment(
            [pos2(x_now, rect.top() + 12.0), pos2(x_now, ground)],
            Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.35)),
        );
        painter.text(
            pos2(x_now, rect.top() + 8.0),
            Align2::CENTER_CENTER,
            "now",
            FontId::proportional(9.5),
            INK_SOFT,
        );

        // Wilting flowers first so the living ones draw on top.
        for w in &mut self.wilting {
            w.life -= dt / WILT_S;
        }
        self.wilting.retain(|w| w.life > 0.0);
        for w in &self.wilting {
            let s = &w.seen;
            let age = (now - w.ended_at) as f32;
            let pos = pos2(
                to_x(s.delay_s + age),
                to_y(s.semitones) + (1.0 - w.life) * 10.0,
            );
            let radius = s.radius * (0.55 + 0.45 * w.life);
            let rot = s.rot + (1.0 - w.life) * 0.9;
            let fade = w.life * w.life * 0.6;
            stem(&painter, pos, ground, 0.0, fade * 0.5);
            flower(&painter, pos, radius, rot, s.color, fade, s.reverse);
        }

        // Living flowers.
        let mut alive = [false; MAX_GRAINS];
        let mut grains: Vec<_> = viz.grains().collect();
        grains.sort_by_key(|g| g.id);
        for g in &grains {
            let pos = pos2(to_x(g.delay_s), to_y(g.semitones));
            let color = pitch_color(g.semitones);
            let base = 10.0 + 20.0 * (g.size_s / 0.5).sqrt().min(1.3);
            let radius = base * (0.3 + 0.7 * g.env);
            let spin = if g.reverse { -0.8 } else { 0.8 };
            let rot = g.id as f32 * 2.399 + self.clock * spin;

            stem(&painter, pos, ground, g.pan, g.env);
            flower(&painter, pos, radius, rot, color, g.env, g.reverse);

            alive[g.slot] = true;
            match self.seen[g.slot] {
                Some(prev) if prev.id != g.id => self.finish(prev),
                _ => {}
            }
            self.seen[g.slot] = Some(Seen {
                id: g.id,
                pos,
                color,
                radius: base * 0.8,
                rot,
                reverse: g.reverse,
                delay_s: g.delay_s,
                semitones: g.semitones,
            });
        }
        for slot in 0..MAX_GRAINS {
            if !alive[slot] {
                if let Some(prev) = self.seen[slot].take() {
                    self.finish(prev);
                }
            }
        }

        // Pollen drifting away from finished grains.
        for p in &mut self.pollen {
            p.pos += p.vel * dt;
            p.vel = p.vel * (1.0 - dt * 0.8) + vec2(0.0, -14.0 * dt);
            p.life -= dt / 1.6;
        }
        self.pollen.retain(|p| p.life > 0.0 && rect.contains(p.pos));
        for p in &self.pollen {
            let r = 1.2 + 1.8 * p.life;
            painter.circle_filled(p.pos, r, p.color.gamma_multiply(p.life * 0.9));
        }

        if params.freeze.value() {
            painter.rect_filled(
                rect,
                0.0,
                Color32::from_rgba_unmultiplied(170, 200, 240, 40),
            );
            painter.text(
                rect.left_top() + vec2(14.0, 14.0),
                Align2::LEFT_TOP,
                "❄ frozen",
                FontId::proportional(12.0),
                SKY.gamma_multiply(1.0),
            );
        }

        // Rounded frame on top of everything.
        panel_frame(&painter, rect);
    }

    fn finish(&mut self, s: Seen) {
        if self.wilting.len() < 256 {
            self.wilting.push(Wilt {
                seen: s,
                ended_at: self.now,
                life: 1.0,
            });
        }
        self.burst(s);
    }

    fn burst(&mut self, s: Seen) {
        if self.pollen.len() > 600 {
            return;
        }
        let count = 3 + (s.radius / 6.0) as usize;
        for _ in 0..count {
            let a = self.rand() * TAU;
            let speed = 10.0 + self.rand() * 26.0;
            let pos = s.pos + vec2(a.cos(), a.sin()) * s.radius * 0.4;
            let vel = vec2(a.cos() * speed, a.sin() * speed * 0.6 - 12.0);
            let life = 0.6 + self.rand() * 0.4;
            self.pollen.push(Pollen {
                pos,
                vel,
                life,
                color: s.color,
            });
        }
    }
}

fn flower(
    painter: &egui::Painter,
    c: Pos2,
    r: f32,
    rot: f32,
    color: Color32,
    env: f32,
    reverse: bool,
) {
    let alpha = 0.35 + 0.6 * env;
    painter.circle_filled(c, r * 1.7, color.gamma_multiply(0.12 * alpha));

    let petals = 5;
    for i in 0..petals {
        let a = rot + i as f32 * TAU / petals as f32;
        let dir = vec2(a.cos(), a.sin());
        let pts = petal(c + dir * r * 0.55, r * 0.55, r * 0.3, a);
        if reverse {
            // Reversed grains: airy outlined petals.
            painter.add(Shape::convex_polygon(
                pts,
                color.gamma_multiply(0.25 * alpha),
                Stroke::new(1.2_f32, color.gamma_multiply(alpha)),
            ));
        } else {
            painter.add(Shape::convex_polygon(
                pts,
                color.gamma_multiply(0.8 * alpha),
                Stroke::new(0.8_f32, darken(color, 0.15).gamma_multiply(alpha)),
            ));
        }
    }
    painter.circle_filled(c, r * 0.24 + 1.0, BUTTER.gamma_multiply(0.6 + 0.4 * env));
}

/// Ellipse approximated by a convex polygon.
fn petal(c: Pos2, a: f32, b: f32, angle: f32) -> Vec<Pos2> {
    let (s, co) = angle.sin_cos();
    (0..14)
        .map(|i| {
            let t = i as f32 / 14.0 * TAU;
            let (x, y) = (t.cos() * a, t.sin() * b);
            c + vec2(x * co - y * s, x * s + y * co)
        })
        .collect()
}

fn stem(painter: &egui::Painter, top: Pos2, ground: f32, pan: f32, env: f32) {
    let bottom = pos2(top.x - pan * 18.0, ground);
    let ctrl = pos2(top.x + pan * 22.0, (top.y + ground) * 0.5);
    painter.add(QuadraticBezierShape::from_points_stroke(
        [bottom, ctrl, top],
        false,
        Color32::TRANSPARENT,
        Stroke::new(1.6_f32, SAGE.gamma_multiply(0.35 + 0.5 * env)),
    ));
}

fn soil(painter: &egui::Painter, pts: &[(f32, f32)], ground: f32, rect: Rect) {
    // Base strip.
    let base = Rect::from_min_max(pos2(rect.left(), ground), rect.right_bottom());
    painter.rect_filled(base, 0.0, Color32::from_rgb(226, 236, 214));
    if pts.len() < 2 {
        return;
    }
    let mut mesh = Mesh::default();
    let top = Color32::from_rgba_unmultiplied(172, 206, 168, 200);
    let bottom = Color32::from_rgba_unmultiplied(214, 232, 204, 220);
    for (i, &(x, h)) in pts.iter().enumerate() {
        mesh.colored_vertex(pos2(x, ground - h - 1.5), top);
        mesh.colored_vertex(pos2(x, ground + 2.0), bottom);
        if i > 0 {
            let k = (i as u32) * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Pastel colour ramp over the pitch range: low = sky, high = butter.
fn pitch_color(st: f32) -> Color32 {
    const RAMP: [Color32; 6] = [SKY, LAVENDER, PINK, MINT, PEACH, BUTTER];
    let t = ((st + 24.0) / 48.0).clamp(0.0, 1.0) * (RAMP.len() - 1) as f32;
    let i = (t.floor() as usize).min(RAMP.len() - 2);
    lerp_color(RAMP[i], RAMP[i + 1], t - i as f32)
}
