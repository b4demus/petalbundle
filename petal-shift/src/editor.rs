use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, lerp_color, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::dsp::interval_name;
use crate::{ModeParam, Shared, ShiftParams, SCOPE_LEN};

const DESIGN: egui::Vec2 = vec2(720.0, 427.0);
const SCALE: f32 = 0.85;
const DISPLAY_H: f32 = 220.0;
const CARD_H: f32 = 84.0;
const KNOB_D: f32 = 44.0;
/// The wheel spans +-150 degrees for +-24 semitones.
const SWEEP: f32 = 150.0 * std::f32::consts::PI / 180.0;

const fn knobs_w(n: usize) -> f32 {
    n as f32 * (KNOB_D + 22.0) + (n as f32 - 1.0) * 2.0
}

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

fn set_now<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

pub fn create(params: Arc<ShiftParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        ShiftEditor::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, ed| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal shift", "pitch shifter", |_| {});
                ui.add_space(10.0);
                ed.display(ui, &params, setter, &shared);
                ui.add_space(14.0);

                let mode_w = 92.0;
                let pitch_w = ui.available_width() - mode_w - knobs_w(2) - 3.0 * CARD_CHROME - 2.0 * 12.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(pitch_w, CARD_H), "pitch", PEACH, |ui| {
                        Knob::new(&params.semitones, setter, "Semitones", PEACH).bipolar().diameter(KNOB_D).show(ui);
                        Knob::new(&params.cents, setter, "Cents", PEACH).bipolar().diameter(KNOB_D).show(ui);
                        Knob::new(&params.spread, setter, "Spread", SKY).diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(mode_w, CARD_H), "mode", LAVENDER, |ui| {
                        ui.vertical(|ui| {
                            ui.add_space(2.0);
                            ui.spacing_mut().item_spacing.y = 4.0;
                            let mode = params.mode.value();
                            for (m, label) in [(ModeParam::Fast, "fast"), (ModeParam::Medium, "medium"), (ModeParam::Slow, "slow")] {
                                if pill(ui, label, mode == m, LAVENDER, vec2(86.0, 20.0)).clicked() {
                                    set_now(setter, &params.mode, m);
                                }
                            }
                        });
                    });
                    card_sized(ui, vec2(knobs_w(2), CARD_H), "output", PINK, |ui| {
                        Knob::new(&params.mix, setter, "Mix", PINK).diameter(KNOB_D).show(ui);
                        Knob::new(&params.gain, setter, "Gain", PINK).diameter(KNOB_D).show(ui);
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

#[derive(Default)]
struct ShiftEditor {
    /// Semitones when the wheel drag began.
    drag_start: Option<f32>,
    input: Vec<f32>,
    output: Vec<f32>,
}

/// Up is warm, down is cool.
fn pitch_color(semis: f32) -> Color32 {
    let t = (semis / 24.0).clamp(-1.0, 1.0);
    if t >= 0.0 {
        lerp_color(LAVENDER, PEACH, t.sqrt())
    } else {
        lerp_color(LAVENDER, SKY, (-t).sqrt())
    }
}

impl ShiftEditor {
    fn display(&mut self, ui: &mut Ui, params: &ShiftParams, setter: &ParamSetter, shared: &Shared) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), DISPLAY_H), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);

        let center = pos2(rect.left() + 130.0, rect.center().y + 14.0);
        let radius = 86.0;
        let on_wheel = resp.hover_pos().or(resp.interact_pointer_pos()).is_some_and(|p| p.distance(center) < radius + 14.0);

        // Drag the wheel up/down: 6 px per semitone, Shift for cents. Scroll = cents.
        let semis = params.semitones.value();
        let cents = params.cents.value();
        if on_wheel {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
        }
        if resp.drag_started() && on_wheel {
            self.drag_start = Some(semis);
            setter.begin_set_parameter(&params.semitones);
            setter.begin_set_parameter(&params.cents);
        }
        if let Some(s0) = self.drag_start {
            let dy = -resp.drag_delta().y;
            if ui.input(|i| i.modifiers.shift) {
                setter.set_parameter(&params.cents, (params.cents.value() + dy * 0.5).clamp(-100.0, 100.0));
            } else {
                // Accumulate in the remembered start value so steps feel even.
                let total = ui.ctx().data_mut(|d| {
                    let acc = d.get_temp_mut_or_insert_with(egui::Id::new("wheel_acc"), || 0.0f32);
                    *acc += dy;
                    *acc
                });
                setter.set_parameter(&params.semitones, (s0 + (total / 6.0).round()).clamp(-24.0, 24.0));
            }
            if !ui.input(|i| i.pointer.primary_down()) {
                self.drag_start = None;
                ui.ctx().data_mut(|d| d.remove::<f32>(egui::Id::new("wheel_acc")));
                setter.end_set_parameter(&params.semitones);
                setter.end_set_parameter(&params.cents);
            }
        }
        if resp.double_clicked() && on_wheel {
            set_now(setter, &params.semitones, 0.0);
            set_now(setter, &params.cents, 0.0);
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if on_wheel && scroll.abs() > 0.0 {
            set_now(setter, &params.cents, (cents + scroll * 0.05).clamp(-100.0, 100.0));
        }

        // --- Wheel -------------------------------------------------------------
        let total = semis + cents / 100.0;
        let angle_of = |st: f32| -std::f32::consts::FRAC_PI_2 + st / 24.0 * SWEEP;
        let point = |a: f32, r: f32| center + vec2(a.cos(), a.sin()) * r;
        painter.circle_filled(center + vec2(0.0, 3.0), radius + 2.0, SHADOW);
        painter.circle_filled(center, radius, CARD);
        painter.circle_stroke(center, radius, Stroke::new(1.0_f32, CARD_BORDER));
        for st in -24..=24 {
            let a = angle_of(st as f32);
            let (len, w, c) = if st % 12 == 0 {
                (14.0, 2.0_f32, INK_SOFT)
            } else if st % 12 == 7 || st % 12 == -5 || st % 12 == 5 || st % 12 == -7 {
                (9.0, 1.5_f32, INK_SOFT.gamma_multiply(0.7))
            } else {
                (6.0, 1.0_f32, INK_SOFT.gamma_multiply(0.4))
            };
            painter.line_segment([point(a, radius - 6.0 - len), point(a, radius - 6.0)], Stroke::new(w, c));
            if st % 12 == 0 {
                let label = if st == 0 { "0".to_owned() } else { format!("{st:+}") };
                label_text(&painter, point(a, radius - 27.0), Align2::CENTER_CENTER, label, 9.5);
            }
        }
        // Value arc from 0 to the current pitch.
        let color = pitch_color(total);
        let (a0, a1) = (angle_of(0.0), angle_of(total.clamp(-24.0, 24.0)));
        let n = (((a1 - a0).abs() / 0.04).ceil() as usize).max(2);
        let arc: Vec<Pos2> = (0..=n).map(|i| point(a0 + (a1 - a0) * i as f32 / n as f32, radius - 3.0)).collect();
        painter.add(Shape::line(arc, Stroke::new(5.0_f32, color)));
        painter.circle_filled(point(a1, radius - 3.0), 6.0, color);
        painter.circle_stroke(point(a1, radius - 3.0), 6.0, Stroke::new(2.0_f32, Color32::WHITE));

        // Centre readout.
        let st_text = format!("{:+.0}", semis);
        painter.text(center + vec2(0.0, -16.0), Align2::CENTER_CENTER, st_text, FontId::proportional(30.0), INK);
        painter.text(center + vec2(0.0, 7.0), Align2::CENTER_CENTER, format!("{:+.1} cents", cents), FontId::proportional(11.0), INK_SOFT);
        let name = interval_name(semis.round() as i32);
        let dir = if total > 0.01 { " up" } else if total < -0.01 { " down" } else { "" };
        painter.text(center + vec2(0.0, 22.0), Align2::CENTER_CENTER, format!("{name}{dir}"), FontId::proportional(10.5), bundle_ui::darken(color, 0.35));

        // --- Scopes ---------------------------------------------------------------
        let end = shared.pos.load(Relaxed);
        self.input.resize(SCOPE_LEN, 0.0);
        self.output.resize(SCOPE_LEN, 0.0);
        for k in 0..SCOPE_LEN {
            let i = end.wrapping_sub(SCOPE_LEN - k) % SCOPE_LEN;
            self.input[k] = f32::from_bits(shared.input[i].load(Relaxed));
            self.output[k] = f32::from_bits(shared.output[i].load(Relaxed));
        }
        let left = rect.left() + 260.0;
        let scope_h = (rect.height() - 44.0) / 2.0;
        let rows = [("in", &self.input, LAVENDER), ("out", &self.output, color)];
        for (k, (label, data, c)) in rows.into_iter().enumerate() {
            let top = rect.top() + 16.0 + k as f32 * (scope_h + 12.0);
            let r = Rect::from_min_size(pos2(left, top), vec2(rect.right() - 16.0 - left, scope_h));
            painter.rect_filled(r, 12.0, CARD.gamma_multiply(0.55));
            let mid = r.center().y;
            painter.line_segment([pos2(r.left() + 8.0, mid), pos2(r.right() - 8.0, mid)], Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.2)));
            // Show the newest third so the cycles are readable.
            let shown = &data[SCOPE_LEN * 2 / 3..];
            let pts: Vec<Pos2> = shown
                .iter()
                .enumerate()
                .map(|(i, &v)| pos2(r.left() + 8.0 + i as f32 / (shown.len() - 1) as f32 * (r.width() - 16.0), mid - v.clamp(-1.0, 1.0) * scope_h * 0.42))
                .collect();
            painter.add(Shape::line(pts.clone(), Stroke::new(4.0_f32, c.gamma_multiply(0.25))));
            painter.add(Shape::line(pts, Stroke::new(1.5_f32, bundle_ui::darken(c, 0.1))));
            label_text(&painter, pos2(r.left() + 10.0, r.top() + 10.0), Align2::LEFT_CENTER, label, 10.0);
        }
        panel_frame(&painter, rect);
    }
}
