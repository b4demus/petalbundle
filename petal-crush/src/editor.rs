use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, lerp_color, panel_frame, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::{CrushParams, Shared, HISTORY};

const DESIGN: egui::Vec2 = vec2(720.0, 432.0);
const SCALE: f32 = 0.85;
const DISPLAY_H: f32 = 220.0;
const CARD_H: f32 = 84.0;
/// Gain-reduction scale of the history graph (dB).
const GR_RANGE: f32 = 36.0;

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

pub fn create(params: Arc<CrushParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        Blob::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, blob| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal crush", "level squisher", |ui| {
                    let text = format!("GR {:.1} dB", -blob.gr);
                    ui.label(egui::RichText::new(text).size(13.0).color(bundle_ui::darken(PINK, 0.25)));
                });
                ui.add_space(10.0);
                blob.display(ui, &params, &shared);
                ui.add_space(14.0);

                let out_w = 2.0 * 72.0 + 2.0;
                let crush_w = ui.available_width() - 12.0 - out_w - 2.0 * CARD_CHROME;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(crush_w, CARD_H), "crush", PINK, |ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        Knob::new(&params.crush, setter, "Crush", PINK).show(ui);
                        Knob::new(&params.darkness, setter, "Darkness", LAVENDER).show(ui);
                        Knob::new(&params.release, setter, "Release", SKY).show(ui);
                        Knob::new(&params.shut_up, setter, "Shut Up", MINT).show(ui);
                    });
                    card_sized(ui, vec2(out_w, CARD_H), "output", PEACH, |ui| {
                        Knob::new(&params.mix, setter, "Mix", PEACH).show(ui);
                        Knob::new(&params.output, setter, "Output", PEACH).show(ui);
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

#[derive(Default)]
struct Blob {
    gr: f32,
    shut: f32,
    level: f32,
    history: Vec<f32>,
}

impl Blob {
    fn display(&mut self, ui: &mut Ui, params: &CrushParams, shared: &Shared) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DISPLAY_H), Sense::hover());
        let painter = ui.painter_at(rect);
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let time = ui.input(|i| i.time) as f32;

        // Ballistics: fast attack, gentle fall so the squish is readable.
        let gr = Shared::load(&shared.reduction);
        self.gr = gr.max(self.gr - 40.0 * dt).max(0.0);
        self.shut += (Shared::load(&shared.shut) - self.shut) * (1.0 - (-dt * 10.0).exp());
        self.level = Shared::load(&shared.output).max(self.level * (-dt * 6.0).exp());

        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);
        let ground = rect.bottom() - 30.0;
        painter.line_segment(
            [pos2(rect.left() + 30.0, ground), pos2(rect.left() + 250.0, ground)],
            Stroke::new(2.0_f32, SAGE.gamma_multiply(0.5)),
        );

        // --- The blob: squashed by gain reduction --------------------------------
        let crush = params.crush.value() / 10.0;
        let darkness = params.darkness.value() / 10.0;
        let squash = (self.gr / 30.0).clamp(0.0, 0.55);
        let (sx, sy) = (1.0 + squash * 0.75, 1.0 - squash);
        let r = 62.0;
        let center = pos2(rect.left() + 140.0, ground - r * sy);
        let body = lerp_color(PINK, LAVENDER, darkness);

        painter.add(ellipse_shape(pos2(center.x, ground + 2.0), r * sx * 0.9, 7.0, SHADOW, 0.0, 0.0));
        painter.add(ellipse_shape(center, r * sx, r * sy, body, time, 1.6));
        painter.add(ellipse_shape(center + vec2(-r * sx * 0.38, -r * sy * 0.48), r * 0.15, r * 0.08 * sy, Color32::from_white_alpha(60), 0.0, 0.0));

        // Face.
        let eye_y = center.y - 12.0 * sy;
        let ink = bundle_ui::darken(body, 0.55);
        for side in [-1.0f32, 1.0] {
            let e = pos2(center.x + side * 21.0 * sx, eye_y);
            if self.shut < 0.5 {
                // "Shut up" is pushing the level down: eyes closed.
                painter.line_segment([e + vec2(-6.0, 0.0), e + vec2(6.0, 0.0)], Stroke::new(2.5_f32, ink));
            } else if self.gr > 18.0 {
                // Squeezed hard: > < eyes.
                let k = side * -1.0;
                painter.line_segment([e + vec2(-5.0 * k, -5.0), e + vec2(5.0 * k, 0.0)], Stroke::new(2.5_f32, ink));
                painter.line_segment([e + vec2(5.0 * k, 0.0), e + vec2(-5.0 * k, 5.0)], Stroke::new(2.5_f32, ink));
            } else {
                painter.circle_filled(e, 5.5, ink);
                painter.circle_filled(e + vec2(-1.8, -1.8), 1.8, Color32::WHITE);
            }
            // Cheeks blush with the crush amount.
            painter.circle_filled(pos2(center.x + side * 36.0 * sx, center.y + 4.0 * sy), 7.0, PEACH.gamma_multiply(0.25 + 0.6 * crush));
        }
        let mouth = pos2(center.x, center.y + 14.0 * sy);
        let open = (self.level.sqrt() * 9.0).clamp(1.5, 9.0) * sy;
        painter.add(ellipse_shape(mouth, 7.0 * sx, open, ink, 0.0, 0.0));

        // Sparks when it is being crushed hard.
        let sparks = ((self.gr - 8.0) / 20.0).clamp(0.0, 1.0);
        if sparks > 0.0 {
            for k in 0..7 {
                let a = time * 1.3 + k as f32 * TAU / 7.0;
                let p = center + vec2(a.cos() * (r * sx + 16.0), a.sin() * (r * sy + 12.0));
                star(&painter, p, 4.0 + 2.0 * (time * 4.0 + k as f32).sin().abs(), BUTTER.gamma_multiply(sparks));
            }
        }

        // --- Gain-reduction history ---------------------------------------------
        let graph = Rect::from_min_max(pos2(rect.left() + 290.0, rect.top() + 16.0), pos2(rect.right() - 16.0, rect.bottom() - 16.0));
        painter.rect_filled(graph, 12.0, CARD.gamma_multiply(0.55));
        let gy = |db: f32| graph.top() + 10.0 + (db / GR_RANGE).clamp(0.0, 1.0) * (graph.height() - 20.0);
        for db in [6.0f32, 12.0, 18.0, 24.0, 30.0] {
            let y = gy(db);
            painter.line_segment([pos2(graph.left() + 30.0, y), pos2(graph.right() - 8.0, y)], Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.15)));
            label_text(&painter, pos2(graph.left() + 8.0, y), Align2::LEFT_CENTER, format!("-{db:.0}"), 9.0);
        }
        let end = shared.history_pos.load(Relaxed);
        self.history.resize(HISTORY, 0.0);
        for k in 0..HISTORY {
            self.history[k] = Shared::load(&shared.history[end.wrapping_sub(HISTORY - k) % HISTORY]);
        }
        let shown = &self.history[HISTORY - 360..];
        let x = |i: usize| graph.left() + 30.0 + i as f32 / (shown.len() - 1) as f32 * (graph.width() - 38.0);
        let mut mesh = Mesh::default();
        for (i, &db) in shown.iter().enumerate() {
            mesh.colored_vertex(pos2(x(i), gy(0.0)), PINK.gamma_multiply(0.15));
            mesh.colored_vertex(pos2(x(i), gy(db)), PINK.gamma_multiply(0.55));
            if i > 0 {
                let k = i as u32 * 2;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
        }
        painter.add(Shape::mesh(mesh));
        let line: Vec<Pos2> = shown.iter().enumerate().map(|(i, &db)| pos2(x(i), gy(db))).collect();
        painter.add(Shape::line(line, Stroke::new(1.6_f32, bundle_ui::darken(PINK, 0.15))));
        painter.text(
            pos2(graph.right() - 12.0, graph.bottom() - 12.0),
            Align2::RIGHT_CENTER,
            "gain reduction",
            FontId::proportional(10.0),
            INK_SOFT,
        );
        panel_frame(&painter, rect);
    }
}

/// Filled ellipse; `wobble` adds a jelly ripple on the outline.
fn ellipse_shape(c: Pos2, rx: f32, ry: f32, fill: Color32, time: f32, wobble: f32) -> Shape {
    let pts: Vec<Pos2> = (0..64)
        .map(|i| {
            let a = i as f32 / 64.0 * TAU;
            let w = 1.0 + wobble * 0.012 * (a * 3.0 + time * 3.0).sin();
            c + vec2(a.cos() * rx * w, a.sin() * ry * w)
        })
        .collect();
    Shape::convex_polygon(pts, fill, Stroke::NONE)
}

fn star(painter: &egui::Painter, c: Pos2, r: f32, color: Color32) {
    for k in 0..4 {
        let a = k as f32 * TAU / 8.0;
        let d = vec2(a.cos(), a.sin()) * r;
        painter.line_segment([c - d, c + d], Stroke::new(1.6_f32, color));
    }
}
