use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, panel_frame, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke,
    Ui,
};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

use crate::dsp::transfer;
use crate::meter::{Frame, Shared};
use crate::CompParams;

const DESIGN: egui::Vec2 = vec2(720.0, 482.0);
const SCALE: f32 = 0.85;
const DISPLAY_H: f32 = 270.0;
const SIDE_W: f32 = 170.0;
const OUTPUT_CARD: f32 = 150.0;
const CARD_H: f32 = 84.0;

/// Display range in dB.
const TOP_DB: f32 = 0.0;
const BOTTOM_DB: f32 = -60.0;

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

pub fn create(params: Arc<CompParams>, meter: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        CompEditor::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, ed| {
            ctx.request_repaint();
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            ed.update_meters(&meter, dt);
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal comp", "soft dynamics", |ui| {
                    let text = format!("GR {:.1} dB", -ed.gr);
                    ui.label(egui::RichText::new(text).size(13.0).color(bundle_ui::darken(PINK, 0.25)));
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let display_w = ui.available_width() - 12.0 - SIDE_W;
                    ed.display(ui, display_w, &params, setter, &meter);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        ed.transfer_panel(ui, &params);
                        ed.meters(ui);
                    });
                });
                ui.add_space(14.0);

                let dyn_w = ui.available_width() - 12.0 - OUTPUT_CARD - 2.0 * CARD_CHROME;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(dyn_w, CARD_H), "dynamics", PEACH, |ui| {
                        Knob::new(&params.threshold, setter, "Threshold", PEACH).show(ui);
                        Knob::new(&params.ratio, setter, "Ratio", PEACH).show(ui);
                        Knob::new(&params.knee, setter, "Knee", PEACH).show(ui);
                        Knob::new(&params.attack, setter, "Attack", MINT).show(ui);
                        Knob::new(&params.release, setter, "Release", MINT).show(ui);
                    });
                    card_sized(ui, vec2(OUTPUT_CARD, CARD_H), "output", PINK, |ui| {
                        Knob::new(&params.gain, setter, "Gain", PINK).bipolar().show(ui);
                        Knob::new(&params.dry, setter, "Dry", PINK).show(ui);
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

#[derive(Default)]
struct CompEditor {
    frames: Vec<Frame>,
    dragging_threshold: bool,
    /// Meter ballistics (dB): input / output peaks and gain reduction.
    input: f32,
    output: f32,
    gr: f32,
}

impl CompEditor {
    fn update_meters(&mut self, meter: &Shared, dt: f32) {
        let mut last = [Frame::default(); 6];
        meter.latest(&mut last);
        let peak_in = last.iter().map(|f| f.input_db).fold(-120.0, f32::max);
        let peak_out = last.iter().map(|f| f.output_db).fold(-120.0, f32::max);
        let gr = last.iter().map(|f| f.reduction_db).fold(0.0, f32::max);
        let fall = 30.0 * dt;
        self.input = peak_in.max(self.input - fall);
        self.output = peak_out.max(self.output - fall);
        self.gr = gr.max(self.gr - fall).max(0.0);
    }

    // -----------------------------------------------------------------------
    // Scrolling level display
    // -----------------------------------------------------------------------

    fn display(&mut self, ui: &mut Ui, width: f32, params: &CompParams, setter: &ParamSetter, meter: &Shared) {
        let (rect, resp) = ui.allocate_exact_size(vec2(width, DISPLAY_H), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let y = |db: f32| {
            let t = (TOP_DB - db) / (TOP_DB - BOTTOM_DB);
            rect.top() + 10.0 + t.clamp(0.0, 1.0) * (rect.height() - 20.0)
        };
        let db_at = |py: f32| TOP_DB - (py - rect.top() - 10.0) / (rect.height() - 20.0) * (TOP_DB - BOTTOM_DB);

        // Threshold: drag the line, or scroll anywhere on the display.
        let thr = params.threshold.value();
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let near_line = pointer.is_some_and(|p| (p.y - y(thr)).abs() < 9.0);
        if near_line || self.dragging_threshold {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
        }
        if resp.drag_started() && near_line {
            self.dragging_threshold = true;
            setter.begin_set_parameter(&params.threshold);
        }
        if self.dragging_threshold {
            if let Some(p) = pointer {
                setter.set_parameter(&params.threshold, db_at(p.y).clamp(-60.0, 0.0));
            }
            if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
                self.dragging_threshold = false;
                setter.end_set_parameter(&params.threshold);
            }
        }
        if resp.double_clicked() && near_line {
            setter.begin_set_parameter(&params.threshold);
            setter.set_parameter(&params.threshold, -18.0);
            setter.end_set_parameter(&params.threshold);
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if resp.hovered() && scroll.abs() > 0.0 {
            setter.begin_set_parameter(&params.threshold);
            setter.set_parameter(&params.threshold, (thr + scroll * 0.02).clamp(-60.0, 0.0));
            setter.end_set_parameter(&params.threshold);
        }

        // History, one frame per point, newest at the right edge.
        let columns = rect.width().round() as usize;
        self.frames.resize(columns, Frame { input_db: -120.0, output_db: -120.0, reduction_db: 0.0 });
        meter.latest(&mut self.frames);
        let frames = &self.frames;
        let x = |i: usize| rect.left() + i as f32 * rect.width() / (columns.max(2) - 1) as f32;

        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);
        for db in (-54..=-6).step_by(6) {
            let yy = y(db as f32);
            painter.line_segment(
                [pos2(rect.left(), yy), pos2(rect.right(), yy)],
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(150, 140, 180, 22)),
            );
            label_text(&painter, pos2(rect.left() + 8.0, yy - 7.0), Align2::LEFT_CENTER, db, 9.5);
        }

        let input: Vec<(f32, f32)> = frames.iter().enumerate().map(|(i, f)| (x(i), y(f.input_db))).collect();
        let output: Vec<(f32, f32)> = frames.iter().enumerate().map(|(i, f)| (x(i), y(f.output_db))).collect();
        let gr: Vec<(f32, f32)> = frames
            .iter()
            .enumerate()
            .map(|(i, f)| (x(i), y(TOP_DB - f.reduction_db)))
            .collect();
        fill_between(&painter, &input, rect.bottom(), SKY.gamma_multiply(0.45), SKY.gamma_multiply(0.18));
        fill_between(&painter, &output, rect.bottom(), LAVENDER.gamma_multiply(0.42), LAVENDER.gamma_multiply(0.12));
        polyline(&painter, &output, Stroke::new(1.2_f32, bundle_ui::darken(LAVENDER, 0.15)));
        fill_between(&painter, &gr, y(TOP_DB), PINK.gamma_multiply(0.45), PINK.gamma_multiply(0.15));
        polyline(&painter, &gr, Stroke::new(1.6_f32, bundle_ui::darken(PINK, 0.1)));

        // Threshold line with a handle.
        let ty = y(thr);
        let active = near_line || self.dragging_threshold;
        let color = if active { bundle_ui::darken(PEACH, 0.15) } else { PEACH };
        dashed(&painter, rect.left(), rect.right(), ty, Stroke::new(if active { 2.0_f32 } else { 1.4 }, color));
        let label = format!("threshold {thr:.1} dB");
        let galley = painter.layout_no_wrap(label, FontId::proportional(11.0), Color32::WHITE);
        let size = galley.size() + vec2(14.0, 6.0);
        let mut at = pos2(rect.right() - size.x - 10.0, ty - size.y - 4.0);
        if at.y < rect.top() + 4.0 {
            at.y = ty + 4.0;
        }
        let handle = Rect::from_min_size(at, size);
        painter.rect_filled(handle, size.y * 0.5, color);
        painter.galley(handle.min + vec2(7.0, 3.0), galley, Color32::WHITE);

        panel_frame(&painter, rect);
    }

    // -----------------------------------------------------------------------
    // Side panels
    // -----------------------------------------------------------------------

    fn transfer_panel(&self, ui: &mut Ui, params: &CompParams) {
        let (rect, _) = ui.allocate_exact_size(vec2(SIDE_W, SIDE_W), Sense::hover());
        let painter = ui.painter_at(rect);
        let inner = rect.shrink(12.0);
        let map = |x_db: f32, y_db: f32| {
            let tx = (x_db - BOTTOM_DB) / (TOP_DB - BOTTOM_DB);
            let ty = (y_db - BOTTOM_DB) / (TOP_DB - BOTTOM_DB);
            pos2(inner.left() + tx * inner.width(), inner.bottom() - ty.clamp(-0.1, 1.1) * inner.height())
        };
        let s = params.settings();
        let makeup = params.gain.value();

        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);
        for db in [-48.0f32, -36.0, -24.0, -12.0] {
            let a = map(db, BOTTOM_DB);
            let grid = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(150, 140, 180, 22));
            painter.line_segment([pos2(a.x, inner.top()), pos2(a.x, inner.bottom())], grid);
            painter.line_segment([pos2(inner.left(), map(BOTTOM_DB, db).y), pos2(inner.right(), map(BOTTOM_DB, db).y)], grid);
        }
        painter.line_segment(
            [map(BOTTOM_DB, BOTTOM_DB), map(TOP_DB, TOP_DB)],
            Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.35)),
        );
        // Knee region.
        let k0 = map(s.threshold_db - s.knee_db / 2.0, BOTTOM_DB).x;
        let k1 = map(s.threshold_db + s.knee_db / 2.0, BOTTOM_DB).x;
        if k1 > k0 {
            painter.rect_filled(
                Rect::from_x_y_ranges(k0..=k1, inner.y_range()),
                0.0,
                PEACH.gamma_multiply(0.12),
            );
        }
        let curve: Vec<Pos2> = (0..=120)
            .map(|i| {
                let x = BOTTOM_DB + i as f32 * 0.5;
                map(x, transfer(x, &s) + makeup)
            })
            .collect();
        painter.add(Shape::line(curve, Stroke::new(2.0_f32, INK.gamma_multiply(0.85))));

        if self.input > BOTTOM_DB {
            let x = self.input.min(TOP_DB);
            let dot = map(x, transfer(x, &s) + makeup);
            if inner.expand(4.0).contains(dot) {
                painter.circle_filled(dot, 9.0, PINK.gamma_multiply(0.25));
                painter.circle_filled(dot, 4.5, PINK);
                painter.circle_stroke(dot, 4.5, Stroke::new(1.5_f32, Color32::WHITE));
            }
        }
        label_text(&painter, rect.left_top() + vec2(10.0, 9.0), Align2::LEFT_TOP, ratio_label(params), 10.5);
        panel_frame(&painter, rect);
    }

    fn meters(&self, ui: &mut Ui) {
        let height = DISPLAY_H - SIDE_W - 10.0;
        let (rect, _) = ui.allocate_exact_size(vec2(SIDE_W, height), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, CARD);

        let rows = [
            ("in", self.input, SKY, false),
            ("gr", -self.gr, PINK, true),
            ("out", self.output, LAVENDER, false),
        ];
        let bar_left = rect.left() + 34.0;
        let bar_right = rect.right() - 46.0;
        let row_h = (rect.height() - 16.0) / rows.len() as f32;
        for (i, (name, db, color, from_right)) in rows.into_iter().enumerate() {
            let cy = rect.top() + 8.0 + row_h * (i as f32 + 0.5);
            let track = Rect::from_x_y_ranges(bar_left..=bar_right, (cy - 4.0)..=(cy + 4.0));
            painter.rect_filled(track, 4.0, TRACK);
            let (t, text) = if from_right {
                // Gain reduction grows leftwards from 0 dB over a 24 dB scale.
                ((-db / 24.0).clamp(0.0, 1.0), format!("{:.1}", db))
            } else {
                (((db - BOTTOM_DB) / (TOP_DB - BOTTOM_DB)).clamp(0.0, 1.0), fmt_db(db))
            };
            if t > 0.0 {
                let fill = if from_right {
                    Rect::from_x_y_ranges((bar_right - t * track.width())..=bar_right, track.y_range())
                } else {
                    Rect::from_x_y_ranges(bar_left..=(bar_left + t * track.width()), track.y_range())
                };
                painter.rect_filled(fill, 4.0, color);
            }
            label_text(&painter, pos2(rect.left() + 12.0, cy), Align2::LEFT_CENTER, name, 11.0);
            painter.text(
                pos2(rect.right() - 10.0, cy),
                Align2::RIGHT_CENTER,
                text,
                FontId::proportional(11.0),
                if from_right { bundle_ui::darken(PINK, 0.25) } else { INK },
            );
        }
        panel_frame(&painter, rect);
    }
}

fn ratio_label(params: &CompParams) -> String {
    params.ratio.normalized_value_to_string(params.ratio.unmodulated_normalized_value(), false)
}

fn fmt_db(db: f32) -> String {
    if db <= BOTTOM_DB {
        "-inf".to_owned()
    } else {
        format!("{db:.1}")
    }
}

/// Area between a polyline and a horizontal baseline.
fn fill_between(painter: &egui::Painter, pts: &[(f32, f32)], base: f32, near: Color32, far: Color32) {
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

fn polyline(painter: &egui::Painter, pts: &[(f32, f32)], stroke: Stroke) {
    painter.add(Shape::line(pts.iter().map(|&(x, y)| pos2(x, y)).collect(), stroke));
}

fn dashed(painter: &egui::Painter, x0: f32, x1: f32, y: f32, stroke: Stroke) {
    let mut x = x0;
    while x < x1 {
        painter.line_segment([pos2(x, y), pos2((x + 7.0).min(x1), y)], stroke);
        x += 11.0;
    }
}
