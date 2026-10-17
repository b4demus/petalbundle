use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::{Grid, HalfParams, Shared, Speed, BINS};

const DESIGN: egui::Vec2 = vec2(720.0, 488.0);
const SCALE: f32 = 0.85;
const DISPLAY_H: f32 = 200.0;
const PILL_H: f32 = 30.0;
const CARD_H: f32 = 84.0;

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

fn set_now<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

pub fn create(params: Arc<HalfParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        (),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, _| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal half", "tempo half-time", |ui| {
                    let synced = shared.synced.load(Relaxed);
                    let text = if synced { format!("synced · {:.0} bpm", Shared::load(&shared.bpm)) } else { "free running (press play)".to_owned() };
                    ui.label(egui::RichText::new(text).size(12.0).color(if synced { bundle_ui::darken(MINT, 0.35) } else { INK_SOFT }));
                });
                ui.add_space(10.0);
                display(ui, &params, &shared);
                ui.add_space(14.0);

                let speed_w = 5.0 * 52.0 + 4.0 * 6.0;
                let grid_w = ui.available_width() - 12.0 - speed_w - 2.0 * CARD_CHROME;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(speed_w, PILL_H), "speed", PEACH, |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let speed = params.speed.value();
                        for s in Speed::ALL {
                            if pill(ui, s.label(), speed == s, PEACH, vec2(52.0, 24.0)).clicked() {
                                set_now(setter, &params.speed, s);
                            }
                        }
                    });
                    card_sized(ui, vec2(grid_w, PILL_H), "grid", SKY, |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let grid = params.grid.value();
                        let w = (grid_w - 5.0 * 6.0) / 6.0;
                        for g in Grid::ALL {
                            if pill(ui, g.label(), grid == g, SKY, vec2(w, 24.0)).clicked() {
                                set_now(setter, &params.grid, g);
                            }
                        }
                    });
                });
                ui.add_space(12.0);
                let full = ui.available_width() - CARD_CHROME;
                card_sized(ui, vec2(full, CARD_H), "sound", LAVENDER, |ui| {
                    let gap = 24.0;
                    let knobs = 4.0 * 72.0 + 3.0 * gap;
                    ui.add_space(((full - knobs) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = gap;
                    Knob::new(&params.smooth, setter, "Smooth", MINT).show(ui);
                    Knob::new(&params.tone, setter, "Tone", SKY).show(ui);
                    Knob::new(&params.mix, setter, "Mix", LAVENDER).show(ui);
                    Knob::new(&params.output, setter, "Output", PINK).show(ui);
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

/// One grid step as a strip: the recording fills in at full speed, the
/// highlighted part is what gets heard, and the playhead crawls through it.
fn display(ui: &mut Ui, p: &HalfParams, s: &Shared) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DISPLAY_H), Sense::hover());
    let painter = ui.painter_at(rect);
    let time = ui.input(|i| i.time) as f32;
    gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);

    let speed = p.speed.value().ratio();
    let grid_beats = p.grid.value().beats() as f32;
    let progress = Shared::load(&s.progress).clamp(0.0, 1.0);
    let playhead = Shared::load(&s.playhead).clamp(0.0, 1.0);

    let strip = Rect::from_min_max(pos2(rect.left() + 24.0, rect.top() + 40.0), pos2(rect.right() - 24.0, rect.bottom() - 44.0));
    let x = |t: f32| strip.left() + t * strip.width();
    let mid = strip.center().y;
    let half = strip.height() * 0.5;

    // The part of the step that will be heard (its first `speed` fraction).
    painter.rect_filled(Rect::from_x_y_ranges(strip.left()..=x(speed), strip.y_range()), 10.0, PEACH.gamma_multiply(0.18));
    painter.rect_stroke(strip, 10.0, Stroke::new(1.0_f32, CARD_BORDER), egui::StrokeKind::Inside);

    // Beat ticks inside the step.
    let ticks = (grid_beats * 4.0).round().max(1.0) as usize; // sixteenths
    for k in 1..ticks {
        let t = k as f32 / ticks as f32;
        let major = k % 4 == 0;
        painter.line_segment(
            [pos2(x(t), strip.top() + if major { 4.0 } else { 12.0 }), pos2(x(t), strip.bottom() - if major { 4.0 } else { 12.0 })],
            Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(if major { 0.25 } else { 0.1 })),
        );
    }

    // Recorded envelope so far (mirrored), brighter where it will be heard.
    let mut mesh = Mesh::default();
    let filled = ((progress * BINS as f32) as usize).min(BINS);
    for k in 0..filled {
        let a = f32::from_bits(s.bins[k].load(Relaxed)).min(1.0).sqrt() * half * 0.9;
        let t0 = k as f32 / BINS as f32;
        let t1 = (k + 1) as f32 / BINS as f32;
        let c = if t0 < speed { LAVENDER.gamma_multiply(0.85) } else { LAVENDER.gamma_multiply(0.3) };
        let i = mesh.vertices.len() as u32;
        for (px, py) in [(x(t0), mid - a), (x(t1), mid - a), (x(t1), mid + a), (x(t0), mid + a)] {
            mesh.colored_vertex(pos2(px, py), c);
        }
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    }
    painter.add(Shape::mesh(mesh));

    // Record head (full speed) and playhead (slowed).
    let rec_x = x(progress);
    painter.line_segment([pos2(rec_x, strip.top() - 6.0), pos2(rec_x, strip.bottom() + 6.0)], Stroke::new(1.5_f32, INK_SOFT.gamma_multiply(0.6)));
    label_text(&painter, pos2(rec_x, strip.top() - 14.0), Align2::CENTER_CENTER, "rec", 9.5);
    let play_x = x(playhead);
    painter.line_segment([pos2(play_x, strip.top() - 6.0), pos2(play_x, strip.bottom() + 6.0)], Stroke::new(2.0_f32, bundle_ui::darken(PEACH, 0.15)));
    flower(&painter, pos2(play_x, strip.bottom() + 16.0), 7.0, time * 1.5 * speed, PEACH);
    label_text(&painter, pos2(rect.left() + 24.0, rect.bottom() - 14.0), Align2::LEFT_CENTER, "peach: the part you hear, played back slower", 9.5);

    painter.text(
        pos2(rect.right() - 24.0, rect.top() + 20.0),
        Align2::RIGHT_CENTER,
        format!("{} speed · every {}", p.speed.value().label(), p.grid.value().label()),
        FontId::proportional(12.0),
        INK,
    );
    panel_frame(&painter, rect);
}

fn flower(painter: &egui::Painter, c: Pos2, r: f32, rot: f32, color: Color32) {
    for i in 0..5 {
        let a = rot + i as f32 * TAU / 5.0;
        let (sn, co) = a.sin_cos();
        let center = c + vec2(co, sn) * r * 0.55;
        let pts: Vec<Pos2> = (0..12)
            .map(|k| {
                let t = k as f32 / 12.0 * TAU;
                let (px, py) = (t.cos() * r * 0.55, t.sin() * r * 0.3);
                center + vec2(px * co - py * sn, px * sn + py * co)
            })
            .collect();
        painter.add(Shape::convex_polygon(pts, color.gamma_multiply(0.9), Stroke::new(0.8_f32, bundle_ui::darken(color, 0.15))));
    }
    painter.circle_filled(c, r * 0.28 + 1.0, BUTTER);
}
