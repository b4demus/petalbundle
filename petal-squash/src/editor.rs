use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, panel_frame, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Rect, Sense, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

use crate::dsp::{BANDS, DEFAULT_DOWN_DB, DEFAULT_UP_DB};
use crate::{Meters, SquashParams, BAND_NAMES};

const DESIGN: egui::Vec2 = vec2(720.0, 480.0);
const SCALE: f32 = 0.85;
const ROW_H: f32 = 76.0;
const GAIN_CARD: f32 = 146.0;
const CARD_H: f32 = 84.0;
/// Level scale of the band bars, dB.
const MIN_DB: f32 = -72.0;
const MAX_DB: f32 = 0.0;
const HANDLE_GRAB: f32 = 10.0;

const BAND_COLORS: [Color32; BANDS] = [SKY, LAVENDER, PEACH];

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

pub fn create(params: Arc<SquashParams>, meters: Arc<Meters>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        SquashEditor::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, ed| {
            ctx.request_repaint();
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            ed.update_meters(&meters, dt);
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal squash", "upward · downward", |_| {});
                ui.add_space(10.0);
                ed.bands(ui, &params, setter);
                ui.add_space(14.0);

                let dyn_w = ui.available_width() - 12.0 - GAIN_CARD - 2.0 * CARD_CHROME;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(dyn_w, CARD_H), "dynamics", MINT, |ui| {
                        Knob::new(&params.depth, setter, "Depth", MINT).show(ui);
                        Knob::new(&params.time, setter, "Time", MINT).show(ui);
                        Knob::new(&params.upward, setter, "Upward", MINT).show(ui);
                        Knob::new(&params.downward, setter, "Downward", PINK).show(ui);
                    });
                    card_sized(ui, vec2(GAIN_CARD, CARD_H), "gain", LAVENDER, |ui| {
                        Knob::new(&params.in_gain, setter, "In", LAVENDER).bipolar().show(ui);
                        Knob::new(&params.out_gain, setter, "Out", LAVENDER).bipolar().show(ui);
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

#[derive(Clone, Copy, PartialEq)]
enum Handle {
    Up,
    Down,
}

#[derive(Default)]
struct SquashEditor {
    input: [f32; BANDS],
    output: [f32; BANDS],
    gain: [f32; BANDS],
    drag: Option<(usize, Handle)>,
}

impl SquashEditor {
    fn update_meters(&mut self, m: &Meters, dt: f32) {
        let fall = 30.0 * dt;
        for b in 0..BANDS {
            self.input[b] = Meters::load(&m.input_db[b]).max(self.input[b] - fall).max(-120.0);
            self.output[b] = Meters::load(&m.output_db[b]).max(self.output[b] - fall).max(-120.0);
            let g = Meters::load(&m.gain_db[b]);
            self.gain[b] += (g - self.gain[b]) * (1.0 - (-dt * 12.0).exp());
        }
    }

    fn bands(&mut self, ui: &mut Ui, params: &SquashParams, setter: &ParamSetter) {
        let height = ROW_H * BANDS as f32 + 12.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);

        let bar_left = rect.left() + 50.0;
        let bar_right = rect.right() - 150.0;
        let x_of = |db: f32| bar_left + (db - MIN_DB) / (MAX_DB - MIN_DB) * (bar_right - bar_left);
        let db_of = |x: f32| MIN_DB + (x - bar_left) / (bar_right - bar_left) * (MAX_DB - MIN_DB);
        let row_center = |b: usize| rect.top() + 6.0 + ROW_H * (b as f32 + 0.5);

        // Which threshold handle is under the pointer?
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let handle_at = |p: egui::Pos2| -> Option<(usize, Handle)> {
            (0..BANDS)
                .filter(|&b| (p.y - row_center(b)).abs() < 22.0)
                .flat_map(|b| {
                    let bp = &params.bands[b];
                    [(b, Handle::Up, bp.up.value()), (b, Handle::Down, bp.down.value())]
                })
                .map(|(b, h, db)| (b, h, (p.x - x_of(db)).abs()))
                .filter(|&(_, _, d)| d < HANDLE_GRAB)
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .map(|(b, h, _)| (b, h))
        };
        let hovered = pointer.and_then(handle_at);
        if hovered.is_some() || self.drag.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }

        let param_of = |b: usize, h: Handle| match h {
            Handle::Up => &params.bands[b].up,
            Handle::Down => &params.bands[b].down,
        };
        if ui.input(|i| i.pointer.primary_pressed()) && resp.hovered() {
            self.drag = hovered;
            if let Some((b, h)) = self.drag {
                setter.begin_set_parameter(param_of(b, h));
            }
        }
        if let (Some((b, h)), Some(p)) = (self.drag, pointer) {
            let bp = &params.bands[b];
            let v = db_of(p.x).clamp(MIN_DB, MAX_DB);
            // Keep the upward threshold at or below the downward one.
            let v = match h {
                Handle::Up => v.min(bp.down.value()),
                Handle::Down => v.max(bp.up.value()),
            };
            setter.set_parameter(param_of(b, h), v);
        }
        if self.drag.is_some() && !ui.input(|i| i.pointer.primary_down()) {
            let (b, h) = self.drag.take().unwrap();
            setter.end_set_parameter(param_of(b, h));
        }
        if resp.double_clicked() {
            if let Some((b, h)) = hovered {
                let default = match h {
                    Handle::Up => DEFAULT_UP_DB[b],
                    Handle::Down => DEFAULT_DOWN_DB[b],
                };
                let p = param_of(b, h);
                setter.begin_set_parameter(p);
                setter.set_parameter(p, default);
                setter.end_set_parameter(p);
            }
        }

        for b in 0..BANDS {
            let bp = &params.bands[b];
            let color = BAND_COLORS[b];
            let cy = row_center(b);
            let up = bp.up.value();
            let down = bp.down.value();

            // Band badge.
            let badge = pos2(rect.left() + 26.0, cy);
            painter.circle_filled(badge, 13.0, color);
            painter.text(badge, Align2::CENTER_CENTER, BAND_NAMES[b], FontId::proportional(13.0), Color32::WHITE);

            // Track with the upward (left) and downward (right) zones.
            let track = Rect::from_x_y_ranges(bar_left..=bar_right, (cy - 15.0)..=(cy + 15.0));
            painter.rect_filled(track, 10.0, CARD.gamma_multiply(0.85));
            let up_zone = Rect::from_x_y_ranges(bar_left..=x_of(up), track.y_range());
            let down_zone = Rect::from_x_y_ranges(x_of(down)..=bar_right, track.y_range());
            painter.rect_filled(up_zone, 10.0, MINT.gamma_multiply(0.3));
            painter.rect_filled(down_zone, 10.0, PINK.gamma_multiply(0.3));
            for db in [-60.0f32, -48.0, -36.0, -24.0, -12.0] {
                let x = x_of(db);
                painter.line_segment(
                    [pos2(x, track.top() + 3.0), pos2(x, track.bottom() - 3.0)],
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(150, 140, 180, 28)),
                );
                if b == BANDS - 1 {
                    label_text(&painter, pos2(x, track.bottom() + 9.0), Align2::CENTER_CENTER, db, 9.0);
                }
            }

            // Levels: thin input bar on top, output bar below it.
            let input_x = x_of(self.input[b].clamp(MIN_DB, MAX_DB));
            let output_x = x_of(self.output[b].clamp(MIN_DB, MAX_DB));
            let input_bar = Rect::from_x_y_ranges(bar_left..=input_x, (cy - 9.0)..=(cy - 4.0));
            let output_bar = Rect::from_x_y_ranges(bar_left..=output_x, (cy - 1.0)..=(cy + 9.0));
            if input_x > bar_left + 1.0 {
                painter.rect_filled(input_bar, 3.0, INK_SOFT.gamma_multiply(0.45));
            }
            if output_x > bar_left + 1.0 {
                painter.rect_filled(output_bar, 4.0, color);
            }

            // Threshold handles.
            for (h, db, c) in [(Handle::Up, up, MINT), (Handle::Down, down, PINK)] {
                let x = x_of(db);
                let hot = hovered == Some((b, h)) || self.drag == Some((b, h));
                let c = bundle_ui::darken(c, if hot { 0.3 } else { 0.15 });
                painter.line_segment([pos2(x, track.top() - 3.0), pos2(x, track.bottom() + 3.0)], Stroke::new(2.0_f32, c));
                painter.circle_filled(pos2(x, track.top() - 4.0), if hot { 5.5 } else { 4.5 }, c);
                if hot {
                    let label = format!("{db:.1} dB");
                    painter.text(pos2(x, track.top() - 13.0), Align2::CENTER_BOTTOM, label, FontId::proportional(10.5), c);
                }
            }

            // Current gain applied to the band.
            let g = self.gain[b];
            let gain_color = if g >= 0.0 { bundle_ui::darken(MINT, 0.35) } else { bundle_ui::darken(PINK, 0.3) };
            painter.text(
                pos2(bar_right + 12.0, cy),
                Align2::LEFT_CENTER,
                format!("{g:+.1} dB"),
                FontId::proportional(12.0),
                gain_color,
            );
        }
        panel_frame(&painter, rect);

        // Band output gain knobs on the right of each row (drawn over the panel).
        for b in 0..BANDS {
            let cy = row_center(b);
            let knob_rect = Rect::from_center_size(pos2(rect.right() - 36.0, cy + 6.0), vec2(56.0, 66.0));
            let color = BAND_COLORS[b];
            ui.scope_builder(egui::UiBuilder::new().max_rect(knob_rect), |ui| {
                Knob::new(&params.bands[b].gain, setter, BAND_NAMES[b], color)
                    .bipolar()
                    .diameter(34.0)
                    .show(ui);
            });
        }
    }
}
