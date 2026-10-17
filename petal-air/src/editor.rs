use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, lerp_color, panel_frame, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::f32::consts::TAU;
use std::sync::Arc;

use crate::{AirParams, Shared};

const DESIGN: egui::Vec2 = vec2(720.0, 466.0);
const SCALE: f32 = 0.85;
const DISPLAY_H: f32 = 240.0;
const BIG_KNOB: f32 = 64.0;
/// Shape constants mirrored from the DSP (analog prototypes for the curve).
const MID_CENTER: f32 = 6_500.0;
const MID_Q: f32 = 0.8;
const HIGH: f32 = 10_000.0;
const MID_MAX: f32 = 1.0;
const HIGH_MAX: f32 = 2.2;

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

pub fn create(params: Arc<AirParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        Breeze::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, breeze| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal air", "fresh breeze", |_| {});
                ui.add_space(10.0);
                breeze.display(ui, &params, &shared);
                ui.add_space(14.0);
                let full = ui.available_width() - CARD_CHROME;
                card_sized(ui, vec2(full, BIG_KNOB + 30.0), "air", SKY, |ui| {
                    let gap = 56.0;
                    let knobs = 3.0 * (BIG_KNOB + 22.0) + 2.0 * gap;
                    ui.add_space(((full - knobs) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = gap;
                    Knob::new(&params.mid_air, setter, "Mid Air", PEACH).diameter(BIG_KNOB).show(ui);
                    Knob::new(&params.high_air, setter, "High Air", SKY).diameter(BIG_KNOB).show(ui);
                    Knob::new(&params.trim, setter, "Trim", LAVENDER).bipolar().diameter(BIG_KNOB).show(ui);
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

struct Seed {
    pos: Pos2,
    vel: egui::Vec2,
    life: f32,
    color: Color32,
    phase: f32,
}

#[derive(Default)]
struct Breeze {
    seeds: Vec<Seed>,
    rng: u32,
    spawn: f32,
    mid_db: f32,
    high_db: f32,
}

impl Breeze {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn display(&mut self, ui: &mut Ui, p: &AirParams, s: &Shared) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DISPLAY_H), Sense::hover());
        let painter = ui.painter_at(rect);
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let time = ui.input(|i| i.time) as f32;
        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);

        let smooth = 1.0 - (-dt * 10.0).exp();
        self.mid_db += (Shared::load(&s.mid_db) - self.mid_db) * smooth;
        self.high_db += (Shared::load(&s.high_db) - self.high_db) * smooth;
        let added = Shared::load(&s.added);

        // --- Dandelion: seeds blow away with the air being added -------------------
        let base = pos2(rect.left() + 130.0, rect.bottom() - 18.0);
        let head = pos2(rect.left() + 130.0 + (time * 0.7).sin() * 3.0, rect.top() + 92.0);
        let ctrl = pos2(base.x - 14.0, (base.y + head.y) * 0.5);
        painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
            [base, ctrl, head],
            false,
            Color32::TRANSPARENT,
            Stroke::new(2.5_f32, SAGE.gamma_multiply(0.8)),
        ));
        painter.circle_filled(head, 7.0, lerp_color(SAGE, BUTTER, 0.4));
        for k in 0..36 {
            let a = k as f32 / 36.0 * TAU + (time * 0.15).sin() * 0.05;
            let tip = head + vec2(a.cos(), a.sin()) * 42.0;
            painter.line_segment([head, tip], Stroke::new(0.8_f32, Color32::from_white_alpha(170)));
            painter.circle_filled(tip, 2.4, Color32::from_white_alpha(200));
        }

        let total = self.mid_db + self.high_db;
        self.spawn += dt * (added.sqrt() * 60.0 + total * 1.5);
        while self.spawn >= 1.0 && self.seeds.len() < 220 {
            self.spawn -= 1.0;
            let a = self.rand() * TAU;
            let start = head + vec2(a.cos(), a.sin()) * 40.0;
            let mid_share = self.mid_db / total.max(1e-3);
            let color = if self.rand() < mid_share { PEACH } else { SKY };
            let vel = vec2(18.0 + 30.0 * self.rand(), -14.0 - 22.0 * self.rand());
            let phase = self.rand() * TAU;
            self.seeds.push(Seed { pos: start, vel, life: 1.0, color, phase });
        }
        self.spawn = self.spawn.min(4.0);
        for seed in &mut self.seeds {
            seed.pos += (seed.vel + vec2(0.0, (time * 2.0 + seed.phase).sin() * 10.0)) * dt;
            seed.life -= dt / 3.5;
        }
        self.seeds.retain(|s| s.life > 0.0 && rect.expand(10.0).contains(s.pos));
        for seed in &self.seeds {
            let a = seed.life.min(1.0);
            // A tiny parachute: stalk + tuft.
            let tuft = seed.pos + vec2(0.0, -5.0);
            painter.line_segment([seed.pos, tuft], Stroke::new(0.8_f32, seed.color.gamma_multiply(a)));
            for k in -2..=2 {
                let d = vec2(k as f32 * 1.6, -2.5 + (k as f32).abs() * 0.6);
                painter.line_segment([tuft, tuft + d], Stroke::new(0.8_f32, seed.color.gamma_multiply(a)));
            }
            painter.circle_filled(seed.pos, 1.3, bundle_ui::darken(seed.color, 0.2).gamma_multiply(a));
        }

        // --- Live boost curve ---------------------------------------------------------
        let graph = Rect::from_min_max(pos2(rect.left() + 290.0, rect.top() + 16.0), pos2(rect.right() - 16.0, rect.bottom() - 16.0));
        painter.rect_filled(graph, 12.0, CARD.gamma_multiply(0.55));
        let (f_lo, f_hi) = (500.0f32, 20_000.0f32);
        let x_of = |f: f32| graph.left() + 34.0 + (f / f_lo).ln() / (f_hi / f_lo).ln() * (graph.width() - 44.0);
        let y_of = |db: f32| graph.bottom() - 18.0 - (db / 12.0).clamp(0.0, 1.0) * (graph.height() - 34.0);
        for db in [3.0f32, 6.0, 9.0, 12.0] {
            let y = y_of(db);
            painter.line_segment([pos2(graph.left() + 34.0, y), pos2(graph.right() - 10.0, y)], Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.12)));
            label_text(&painter, pos2(graph.left() + 8.0, y), Align2::LEFT_CENTER, format!("+{db:.0}"), 9.0);
        }
        for (f, l) in [(1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10_000.0, "10k"), (20_000.0, "20k")] {
            label_text(&painter, pos2(x_of(f), graph.bottom() - 8.0), Align2::CENTER_CENTER, l, 9.0);
        }
        let gm_live = 10f32.powf(self.mid_db / 20.0) - 1.0;
        let gh_live = 10f32.powf(self.high_db / 20.0) - 1.0;
        let gm_max = p.mid_air.value() * MID_MAX;
        let gh_max = p.high_air.value() * HIGH_MAX;
        let freqs: Vec<f32> = (0..=120).map(|i| f_lo * (f_hi / f_lo).powf(i as f32 / 120.0)).collect();
        let curve = |gm: f32, gh: f32| -> Vec<Pos2> { freqs.iter().map(|&f| pos2(x_of(f), y_of(boost_db(f, gm, gh)))).collect() };
        // The ceiling of what the knobs allow, then the live (dynamic) boost.
        painter.add(Shape::line(curve(gm_max, gh_max), Stroke::new(1.2_f32, INK_SOFT.gamma_multiply(0.35))));
        let live = curve(gm_live, gh_live);
        let mut mesh = Mesh::default();
        for (i, q) in live.iter().enumerate() {
            let t = i as f32 / (live.len() - 1) as f32;
            let c = lerp_color(PEACH, SKY, t).gamma_multiply(0.45);
            mesh.colored_vertex(*q, c);
            mesh.colored_vertex(pos2(q.x, y_of(0.0)), c.gamma_multiply(0.2));
            if i > 0 {
                let k = i as u32 * 2;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
        }
        painter.add(Shape::mesh(mesh));
        painter.add(Shape::line(live, Stroke::new(2.0_f32, INK.gamma_multiply(0.75))));
        label_text(&painter, pos2(x_of(MID_CENTER), graph.top() + 12.0), Align2::CENTER_CENTER, format!("mid +{:.1}", self.mid_db), 10.0);
        label_text(&painter, pos2(x_of(15_000.0), graph.top() + 12.0), Align2::CENTER_CENTER, format!("high +{:.1}", self.high_db), 10.0);
        panel_frame(&painter, rect);
    }
}

/// Boost in dB at `f` for the bell (mid) + first-order shelf (high), analog prototypes.
fn boost_db(f: f32, gm: f32, gh: f32) -> f32 {
    // Bell: 1 + gm * BP(jw), BP(s) = (s/(Q w0)) / (s^2/w0^2 + s/(Q w0) + 1).
    let u = f / MID_CENTER;
    let (bp_re, bp_im) = {
        let den = (1.0 - u * u, u / MID_Q);
        let num = (0.0, u / MID_Q);
        let d2 = den.0 * den.0 + den.1 * den.1;
        ((num.0 * den.0 + num.1 * den.1) / d2, (num.1 * den.0 - num.0 * den.1) / d2)
    };
    // Shelf: HP1(jw) = jv / (1 + jv), v = f / fc.
    let v = f / HIGH;
    let (hp_re, hp_im) = (v * v / (1.0 + v * v), v / (1.0 + v * v));
    let re = 1.0 + gm * bp_re + gh * hp_re;
    let im = gm * bp_im + gh * hp_im;
    10.0 * (re * re + im * im).log10()
}
