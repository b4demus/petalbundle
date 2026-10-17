use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, lerp_color, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::dsp::{note_name, VOICES};
use crate::{OracleParams, Shared};

const DESIGN: egui::Vec2 = vec2(720.0, 572.0);
const SCALE: f32 = 0.85;
const DISPLAY_H: f32 = 240.0;
const CARD_H: f32 = 84.0;
const KNOB_D: f32 = 44.0;
/// Seconds of melody history in the ribbon.
const HISTORY_S: f32 = 8.0;

const VOICE_COLORS: [Color32; VOICES] = [BUTTER, PINK, SKY, MINT, LAVENDER, PEACH];
const PITCH_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

const fn knobs_w(n: usize) -> f32 {
    n as f32 * (KNOB_D + 22.0) + (n as f32 - 1.0) * 2.0
}

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

pub fn create(params: Arc<OracleParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        Sky::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, sky| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal oracle", "generative resonator", |ui| {
                    let on = params.freeze.value();
                    let label = if on { "❄  frozen" } else { "❄  freeze" };
                    if pill(ui, label, on, SKY, vec2(96.0, 28.0)).clicked() {
                        setter.begin_set_parameter(&params.freeze);
                        setter.set_parameter(&params.freeze, !on);
                        setter.end_set_parameter(&params.freeze);
                    }
                    ui.label(egui::RichText::new(format!("{:.0} bpm", Shared::load(&shared.bpm))).size(12.0).color(INK_SOFT));
                });
                ui.add_space(10.0);
                sky.display(ui, &params, &shared);
                ui.add_space(14.0);

                let right = knobs_w(3);
                let left = ui.available_width() - 12.0 - right - 2.0 * CARD_CHROME;
                let p = &params;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(left, CARD_H), "harmony", LAVENDER, |ui| {
                        Knob::new(&p.key, setter, "Key", LAVENDER).show_value().diameter(KNOB_D).show(ui);
                        Knob::new(&p.scale, setter, "Scale", LAVENDER).show_value().diameter(KNOB_D).show(ui);
                        Knob::new(&p.octave, setter, "Octave", LAVENDER).diameter(KNOB_D).show(ui);
                        Knob::new(&p.spread, setter, "Spread", LAVENDER).diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(right, CARD_H), "motion", SKY, |ui| {
                        Knob::new(&p.rate, setter, "Rate", SKY).show_value().diameter(KNOB_D).show(ui);
                        Knob::new(&p.chaos, setter, "Chaos", SKY).diameter(KNOB_D).show(ui);
                        Knob::new(&p.glide, setter, "Glide", SKY).diameter(KNOB_D).show(ui);
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(left, CARD_H), "tone", PINK, |ui| {
                        Knob::new(&p.decay, setter, "Decay", PINK).diameter(KNOB_D).show(ui);
                        Knob::new(&p.brightness, setter, "Brightness", PINK).diameter(KNOB_D).show(ui);
                        Knob::new(&p.bite, setter, "Bite", PEACH).diameter(KNOB_D).show(ui);
                        Knob::new(&p.halo, setter, "Halo", MINT).diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(right, CARD_H), "output", INK_SOFT, |ui| {
                        Knob::new(&p.mix, setter, "Mix", LAVENDER).diameter(KNOB_D).show(ui);
                        Knob::new(&p.output, setter, "Output", PEACH).diameter(KNOB_D).show(ui);
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

#[derive(Default)]
struct Sky {
    /// (seconds since start, notes, energies) per editor frame.
    history: VecDeque<(f32, [f32; VOICES], [f32; VOICES])>,
    clock: f32,
    last_steps: u32,
    pulse: f32,
}

impl Sky {
    fn display(&mut self, ui: &mut Ui, p: &OracleParams, shared: &Shared) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DISPLAY_H), Sense::hover());
        let painter = ui.painter_at(rect);
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        self.clock += dt;

        let notes: [f32; VOICES] = std::array::from_fn(|i| Shared::load(&shared.note[i]));
        let energy: [f32; VOICES] = std::array::from_fn(|i| Shared::load(&shared.energy[i]));
        self.history.push_back((self.clock, notes, energy));
        while self.history.front().is_some_and(|h| self.clock - h.0 > HISTORY_S) {
            self.history.pop_front();
        }
        let steps = shared.steps.load(Relaxed);
        if steps != self.last_steps {
            self.last_steps = steps;
            self.pulse = 1.0;
        }
        self.pulse = (self.pulse - dt * 2.5).max(0.0);

        let h = p.harmony();
        let low = h.note(0) as f32;
        let high = h.note(h.degrees()) as f32;
        let bite = p.bite.value();
        let halo = p.halo.value();
        let freeze = p.freeze.value();

        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);

        // --- Constellation: twelve pitch classes around a circle -----------------
        let center = pos2(rect.left() + 128.0, rect.center().y);
        let radius = 92.0;
        let in_scale: Vec<i32> = h.scale.steps().iter().map(|s| (s + h.key).rem_euclid(12)).collect();
        if self.pulse > 0.0 {
            painter.circle_stroke(center, radius * (1.0 + 0.08 * (1.0 - self.pulse)), Stroke::new(2.0_f32, LAVENDER.gamma_multiply(0.35 * self.pulse)));
        }
        painter.circle_stroke(center, radius, Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.2)));
        painter.circle_stroke(center, radius * 0.35, Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.12)));
        for pc in 0..12 {
            let a = -std::f32::consts::FRAC_PI_2 + pc as f32 / 12.0 * TAU;
            let at = center + vec2(a.cos(), a.sin()) * (radius + 14.0);
            let lit = in_scale.contains(&pc);
            let color = if pc == h.key { bundle_ui::darken(PINK, 0.2) } else if lit { INK } else { INK_SOFT.gamma_multiply(0.35) };
            painter.text(at, Align2::CENTER_CENTER, PITCH_NAMES[pc as usize], FontId::proportional(if lit { 11.0 } else { 9.0 }), color);
            if lit {
                painter.circle_filled(center + vec2(a.cos(), a.sin()) * radius, 2.5, LAVENDER.gamma_multiply(0.6));
            }
        }

        // Voice positions: angle = pitch class (smooth while gliding), radius = height.
        let pos_of = |note: f32| {
            let a = -std::f32::consts::FRAC_PI_2 + (note / 12.0) * TAU;
            let t = ((note - low) / (high - low).max(1.0)).clamp(0.0, 1.0);
            center + vec2(a.cos(), a.sin()) * radius * (0.35 + 0.6 * t)
        };
        let mut points: Vec<(Pos2, usize)> = (0..VOICES).map(|v| (pos_of(notes[v]), v)).collect();
        // Chord shape: connect voices in angular order.
        points.sort_by(|a, b| {
            let ang = |p: Pos2| (p.y - center.y).atan2(p.x - center.x);
            ang(a.0).total_cmp(&ang(b.0))
        });
        let shape: Vec<Pos2> = points.iter().map(|p| p.0).collect();
        painter.add(Shape::closed_line(shape, Stroke::new(1.2_f32, LAVENDER.gamma_multiply(0.45))));
        for &(q, v) in &points {
            let e = energy[v].sqrt().min(1.0);
            let color = if freeze { lerp_color(VOICE_COLORS[v], SKY, 0.4) } else { VOICE_COLORS[v] };
            if halo > 0.0 {
                painter.circle_filled(q, 12.0 + 14.0 * halo, color.gamma_multiply(0.12 * halo + 0.15 * e));
            }
            let r = 7.5 + 8.0 * e;
            bloom(&painter, q, r, self.clock * 0.4 + v as f32, color, bite);
        }

        // --- Melody ribbon ------------------------------------------------------------
        let ribbon = Rect::from_min_max(pos2(rect.left() + 262.0, rect.top() + 16.0), pos2(rect.right() - 16.0, rect.bottom() - 16.0));
        painter.rect_filled(ribbon, 12.0, CARD.gamma_multiply(0.55));
        let y_of = |note: f32| {
            let t = (note - (low - 2.0)) / ((high + 2.0) - (low - 2.0));
            ribbon.bottom() - 10.0 - t.clamp(0.0, 1.0) * (ribbon.height() - 20.0)
        };
        // Octave guide lines with note names.
        let mut n = h.note(0);
        while n as f32 <= high {
            let y = y_of(n as f32);
            painter.line_segment([pos2(ribbon.left() + 34.0, y), pos2(ribbon.right() - 8.0, y)], Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.15)));
            label_text(&painter, pos2(ribbon.left() + 8.0, y), Align2::LEFT_CENTER, note_name(n), 9.0);
            n += 12;
        }
        let x_of = |t: f32| ribbon.right() - 8.0 - (self.clock - t) / HISTORY_S * (ribbon.width() - 42.0);
        for v in 0..VOICES {
            let pts: Vec<Pos2> = self.history.iter().map(|(t, ns, _)| pos2(x_of(*t), y_of(ns[v]))).collect();
            if pts.len() > 1 {
                let e = self.history.back().map_or(0.0, |h| h.2[v]).sqrt().min(1.0);
                painter.add(Shape::line(pts, Stroke::new(1.5_f32 + 2.0 * e, VOICE_COLORS[v].gamma_multiply(0.45 + 0.5 * e))));
            }
        }
        if let Some((_, ns, _)) = self.history.back() {
            for v in 0..VOICES {
                painter.circle_filled(pos2(ribbon.right() - 8.0, y_of(ns[v])), 3.5, VOICE_COLORS[v]);
            }
        }
        panel_frame(&painter, rect);
    }
}

/// A voice flower: soft petals, turning into spikes as Bite rises.
fn bloom(painter: &egui::Painter, c: Pos2, r: f32, rot: f32, color: Color32, bite: f32) {
    let petals = 6;
    for i in 0..petals {
        let a = rot + i as f32 * TAU / petals as f32;
        let (s, co) = a.sin_cos();
        let len = r * (1.0 + 0.8 * bite);
        let width = r * 0.45 * (1.0 - 0.7 * bite);
        // Lens-shaped petal from the centre outwards.
        let pts: Vec<Pos2> = (0..=10)
            .map(|k| {
                let t = k as f32 / 10.0;
                let w = (t * std::f32::consts::PI).sin() * width;
                c + vec2(co, s) * (t * len) + vec2(-s, co) * w
            })
            .chain((1..10).rev().map(|k| {
                let t = k as f32 / 10.0;
                let w = (t * std::f32::consts::PI).sin() * width;
                c + vec2(co, s) * (t * len) - vec2(-s, co) * w
            }))
            .collect();
        painter.add(Shape::convex_polygon(pts, color.gamma_multiply(0.85), Stroke::new(0.8_f32, bundle_ui::darken(color, 0.2))));
    }
    painter.circle_filled(c, r * 0.32 + 1.0, Color32::WHITE.gamma_multiply(0.9));
}
