use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, lerp_color, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, epaint::QuadraticBezierShape, pos2, vec2, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense,
    Shape, Stroke, Ui,
};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::dsp::{effective_delay_ms, MAX_DELAY_MS, VOICES};
use crate::{DoubleParams, Shared, VoiceParams};

const DESIGN: egui::Vec2 = vec2(720.0, 528.0);
const SCALE: f32 = 0.85;
const MAP_H: f32 = 200.0;
const CARD_H: f32 = 84.0;
const KNOB_D: f32 = 42.0;
const NODE_HIT: f32 = 16.0;

const fn knobs_w(n: usize) -> f32 {
    n as f32 * (KNOB_D + 22.0) + (n as f32 - 1.0) * 2.0
}

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

pub fn create(params: Arc<DoubleParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        DoubleEditor::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, ed| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal double", "four-voice doubler", |ui| {
                    // Right-to-left: voice chips 4..1.
                    for v in (0..VOICES).rev() {
                        let vp = &params.voices[v];
                        let color = if vp.on.value() { voice_color(vp.pitch.value()) } else { TRACK };
                        if pill(ui, &(v + 1).to_string(), ed.selected == v, color, vec2(30.0, 26.0)).clicked() {
                            ed.selected = v;
                        }
                    }
                });
                ui.add_space(10.0);
                ed.map(ui, &params, setter, &shared);
                ui.add_space(14.0);

                let right_w = knobs_w(3);
                let left_w = ui.available_width() - 12.0 - right_w - 2.0 * CARD_CHROME;
                let vp = &params.voices[ed.selected];
                let color = voice_color(vp.pitch.value());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let title = format!("voice {}", ed.selected + 1);
                    card_sized(ui, vec2(left_w, CARD_H), &title, color, |ui| {
                        Knob::new(&vp.delay, setter, "Delay", color).diameter(KNOB_D).show(ui);
                        Knob::new(&vp.pitch, setter, "Pitch", color).bipolar().diameter(KNOB_D).show(ui);
                        Knob::new(&vp.gain, setter, "Gain", color).diameter(KNOB_D).show(ui);
                        Knob::new(&vp.pan, setter, "Pan", color).bipolar().diameter(KNOB_D).show(ui);
                        ui.add_space(10.0);
                        ui.vertical(|ui| {
                            ui.add_space(14.0);
                            let on = vp.on.value();
                            if pill(ui, if on { "on" } else { "off" }, on, color, vec2(60.0, 24.0)).clicked() {
                                set_now(setter, &vp.on, !on);
                            }
                        });
                    });
                    card_sized(ui, vec2(right_w, CARD_H), "direct", BUTTER, |ui| {
                        Knob::new(&params.direct, setter, "Direct", BUTTER).diameter(KNOB_D).show(ui);
                        Knob::new(&params.direct_pan, setter, "Pan", BUTTER).bipolar().diameter(KNOB_D).show(ui);
                        Knob::new(&params.output, setter, "Output", PINK).diameter(KNOB_D).show(ui);
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(left_w, CARD_H), "modulation", LAVENDER, |ui| {
                        Knob::new(&params.mod_rate, setter, "Rate", LAVENDER).diameter(KNOB_D).show(ui);
                        Knob::new(&params.mod_depth, setter, "Depth", LAVENDER).diameter(KNOB_D).show(ui);
                        Knob::new(&params.feedback, setter, "Feedback", LAVENDER).diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(right_w, CARD_H), "tone", MINT, |ui| {
                        Knob::new(&params.low_cut, setter, "Low Cut", MINT).diameter(KNOB_D).show(ui);
                        Knob::new(&params.high_cut, setter, "High Cut", MINT).diameter(KNOB_D).show(ui);
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

#[derive(Clone, Copy, PartialEq)]
enum Grab {
    Voice(usize),
    Direct,
}

#[derive(Default)]
struct DoubleEditor {
    selected: usize,
    pressed: Option<(Grab, egui::Vec2)>,
    dragging: Option<(Grab, egui::Vec2)>,
}

/// Flat voices are cool, sharp voices warm.
fn voice_color(cents: f32) -> Color32 {
    let t = (cents / 50.0).clamp(-1.0, 1.0);
    if t < 0.0 {
        lerp_color(MINT, SKY, -t)
    } else {
        lerp_color(MINT, PEACH, t)
    }
}

fn set_now<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

impl DoubleEditor {
    fn map(&mut self, ui: &mut Ui, params: &DoubleParams, setter: &ParamSetter, shared: &Shared) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), MAP_H), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let time = ui.input(|i| i.time) as f32;

        let left = rect.left() + 56.0;
        let right = rect.right() - 40.0;
        let top = rect.top() + 30.0;
        let bottom = rect.bottom() - 22.0;
        let x_of = |pan: f32| left + (pan + 1.0) * 0.5 * (right - left);
        let pan_of = |x: f32| ((x - left) / (right - left) * 2.0 - 1.0).clamp(-1.0, 1.0);
        let y_of = |ms: f32| top + ms / MAX_DELAY_MS * (bottom - top);
        let ms_of = |y: f32| ((y - top) / (bottom - top) * MAX_DELAY_MS).clamp(1.0, MAX_DELAY_MS);

        let depth = params.mod_depth.value();
        let voice_pos = |v: usize, vp: &VoiceParams| {
            let wobble = f32::from_bits(shared.wobble[v].load(Relaxed));
            // The node drifts with the live modulation (up to ~3 ms).
            let ms = effective_delay_ms(vp.delay.value()) + wobble * depth * 3.0;
            pos2(x_of(vp.pan.value()), y_of(ms))
        };
        let direct_pos = pos2(x_of(params.direct_pan.value()), top - 12.0);

        // Hit testing in widget space.
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let grab_at = |p: Pos2| -> Option<(Grab, Pos2)> {
            let mut best: Option<(Grab, Pos2, f32)> = None;
            for v in 0..VOICES {
                let q = voice_pos(v, &params.voices[v]);
                let d = q.distance(p);
                if d < NODE_HIT && best.is_none_or(|b| d < b.2) {
                    best = Some((Grab::Voice(v), q, d));
                }
            }
            let d = direct_pos.distance(p);
            if d < NODE_HIT && best.is_none_or(|b| d < b.2) {
                best = Some((Grab::Direct, direct_pos, d));
            }
            best.map(|(g, q, _)| (g, q))
        };
        let hovered = pointer.and_then(grab_at);
        if hovered.is_some() || self.dragging.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }

        if ui.input(|i| i.pointer.primary_pressed()) && resp.hovered() {
            self.pressed = hovered.map(|(g, q)| (g, q - pointer.unwrap_or(q)));
            if let Some((Grab::Voice(v), _)) = self.pressed {
                self.selected = v;
            }
        }
        if resp.drag_started() {
            if let Some((g, grab)) = self.pressed {
                match g {
                    Grab::Voice(v) => {
                        setter.begin_set_parameter(&params.voices[v].pan);
                        setter.begin_set_parameter(&params.voices[v].delay);
                    }
                    Grab::Direct => setter.begin_set_parameter(&params.direct_pan),
                }
                self.dragging = Some((g, grab));
            }
        }
        if let (Some((g, grab)), Some(p)) = (self.dragging, pointer) {
            let at = p + grab;
            match g {
                Grab::Voice(v) => {
                    setter.set_parameter(&params.voices[v].pan, pan_of(at.x));
                    setter.set_parameter(&params.voices[v].delay, ms_of(at.y));
                }
                Grab::Direct => setter.set_parameter(&params.direct_pan, pan_of(at.x)),
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            self.pressed = None;
            if let Some((g, _)) = self.dragging.take() {
                match g {
                    Grab::Voice(v) => {
                        setter.end_set_parameter(&params.voices[v].pan);
                        setter.end_set_parameter(&params.voices[v].delay);
                    }
                    Grab::Direct => setter.end_set_parameter(&params.direct_pan),
                }
            }
        }
        if resp.double_clicked() {
            if let Some((Grab::Voice(v), _)) = hovered {
                let on = params.voices[v].on.value();
                set_now(setter, &params.voices[v].on, !on);
            }
        }
        // Scroll over a voice: detune it.
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if resp.hovered() && scroll.abs() > 0.0 {
            if let Some((Grab::Voice(v), _)) = hovered {
                let p = &params.voices[v].pitch;
                set_now(setter, p, (p.value() + scroll * 0.03).clamp(-50.0, 50.0));
            }
        }

        // --- painting -------------------------------------------------------
        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);
        let grid = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(150, 140, 180, 22));
        for ms in (10..=70).step_by(10) {
            let y = y_of(ms as f32);
            painter.line_segment([pos2(left, y), pos2(right, y)], grid);
            label_text(&painter, pos2(rect.left() + 14.0, y), Align2::LEFT_CENTER, format!("{ms} ms"), 9.5);
        }
        for (pan, label) in [(-1.0f32, "L"), (0.0, "C"), (1.0, "R")] {
            let x = x_of(pan);
            painter.line_segment([pos2(x, top - 4.0), pos2(x, bottom)], grid);
            label_text(&painter, pos2(x, rect.bottom() - 10.0), Align2::CENTER_CENTER, label, 10.0);
        }

        // Stems from the direct sound to each voice.
        for v in 0..VOICES {
            let vp = &params.voices[v];
            let q = voice_pos(v, vp);
            let ctrl = pos2((direct_pos.x + q.x) * 0.5, direct_pos.y + (q.y - direct_pos.y) * 0.15);
            let alpha = if vp.on.value() { 0.45 } else { 0.15 };
            painter.add(QuadraticBezierShape::from_points_stroke(
                [direct_pos, ctrl, q],
                false,
                Color32::TRANSPARENT,
                Stroke::new(1.4_f32, SAGE.gamma_multiply(alpha)),
            ));
        }

        // Direct sound: a butter-yellow seed you can slide left/right.
        let hot = |g: Grab| hovered.map(|h| h.0) == Some(g) || self.dragging.map(|d| d.0) == Some(g);
        let r = if hot(Grab::Direct) { 8.0 } else { 6.5 };
        painter.circle_filled(direct_pos, r + 5.0, BUTTER.gamma_multiply(0.25));
        painter.circle_filled(direct_pos, r, bundle_ui::darken(BUTTER, 0.05));
        painter.circle_stroke(direct_pos, r, Stroke::new(1.5_f32, Color32::WHITE));

        for v in 0..VOICES {
            let vp = &params.voices[v];
            let q = voice_pos(v, vp);
            let on = vp.on.value();
            let gain = ((vp.gain.value() + 36.0) / 42.0).clamp(0.0, 1.0);
            let radius = 7.0 + 9.0 * gain + if hot(Grab::Voice(v)) { 2.0 } else { 0.0 };
            let color = if on { voice_color(vp.pitch.value()) } else { TRACK };
            if v == self.selected {
                painter.circle_filled(q, radius + 7.0, color.gamma_multiply(0.22));
            }
            let spin = time * (0.2 + 0.25 * params.mod_rate.value().min(4.0)) * if v % 2 == 0 { 1.0 } else { -1.0 };
            flower(&painter, q, radius, spin + v as f32, color, on);
            painter.text(q, Align2::CENTER_CENTER, (v + 1).to_string(), FontId::proportional(10.0), if on { Color32::WHITE } else { INK_SOFT });
        }

        // Readout for the hovered / dragged node.
        let focus = self.dragging.map(|d| d.0).or(hovered.map(|h| h.0));
        if let Some(Grab::Voice(v)) = focus {
            let vp = &params.voices[v];
            let text = format!(
                "{:.1} ms · {:+.1} c · {:.1} dB · {}",
                vp.delay.value(),
                vp.pitch.value(),
                vp.gain.value(),
                vp.pan.normalized_value_to_string(vp.pan.unmodulated_normalized_value(), false)
            );
            readout(&painter, rect, voice_pos(v, vp), text, voice_color(vp.pitch.value()));
        } else if let Some(Grab::Direct) = focus {
            let text = format!(
                "direct · {}",
                params.direct_pan.normalized_value_to_string(params.direct_pan.unmodulated_normalized_value(), false)
            );
            readout(&painter, rect, direct_pos, text, BUTTER);
        }
        panel_frame(&painter, rect);
    }
}

fn readout(painter: &egui::Painter, area: Rect, at: Pos2, text: String, color: Color32) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(11.0), INK);
    let size = galley.size() + vec2(14.0, 8.0);
    let mut pos = at + vec2(16.0, -size.y * 0.5);
    if pos.x + size.x > area.right() - 6.0 {
        pos.x = at.x - 16.0 - size.x;
    }
    pos.y = pos.y.clamp(area.top() + 4.0, area.bottom() - size.y - 4.0);
    let bg = Rect::from_min_size(pos, size);
    painter.rect_filled(bg, 8.0, lerp_color(CARD, color, 0.2));
    painter.galley(bg.min + vec2(7.0, 4.0), galley, INK);
}

/// Six-petal flower; switched-off voices are drawn as pale outlines.
fn flower(painter: &egui::Painter, c: Pos2, r: f32, rot: f32, color: Color32, on: bool) {
    for i in 0..6 {
        let a = rot + i as f32 * TAU / 6.0;
        let (s, co) = a.sin_cos();
        let center = c + vec2(co, s) * r * 0.6;
        let pts: Vec<Pos2> = (0..12)
            .map(|k| {
                let t = k as f32 / 12.0 * TAU;
                let (x, y) = (t.cos() * r * 0.55, t.sin() * r * 0.32);
                center + vec2(x * co - y * s, x * s + y * co)
            })
            .collect();
        if on {
            painter.add(Shape::convex_polygon(pts, color.gamma_multiply(0.9), Stroke::new(0.8_f32, bundle_ui::darken(color, 0.15))));
        } else {
            painter.add(Shape::convex_polygon(pts, CARD.gamma_multiply(0.6), Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.5))));
        }
    }
    painter.circle_filled(c, r * 0.42, if on { bundle_ui::darken(color, 0.25) } else { TRACK });
}
