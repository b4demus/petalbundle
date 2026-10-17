use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, lerp_color, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, Pos2, Sense, Shape, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::dsp::{taps, Mode};
use crate::{pan_gains, DelayParams, FeedbackMode, Shared};

const DESIGN: egui::Vec2 = vec2(720.0, 418.0);
const SCALE: f32 = 0.85;
const DISPLAY_H: f32 = 210.0;
const CARD_H: f32 = 84.0;
const KNOB_D: f32 = 44.0;
/// Width of a card holding `n` small knobs.
const fn knobs_w(n: usize) -> f32 {
    n as f32 * (KNOB_D + 22.0) + (n as f32 - 1.0) * 2.0
}

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

pub fn create(params: Arc<DelayParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        DelayEditor::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, ed| {
            ctx.request_repaint();
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            ed.update(&shared, dt);
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal delay", "ping-pong echoes", |ui| {
                    let on = params.sync.value();
                    if pill(ui, "sync", on, SKY, vec2(64.0, 26.0)).clicked() {
                        setter.begin_set_parameter(&params.sync);
                        setter.set_parameter(&params.sync, !on);
                        setter.end_set_parameter(&params.sync);
                    }
                    ui.label(egui::RichText::new(format!("{:.0} bpm", shared.bpm())).size(12.0).color(INK_SOFT));
                });
                ui.add_space(10.0);
                ed.display(ui, &params, &shared);
                ui.add_space(14.0);

                let fixed = knobs_w(2) * 2.0 + knobs_w(1) + 4.0 * CARD_CHROME + 3.0 * 12.0;
                let feedback_w = ui.available_width() - fixed;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(knobs_w(2), CARD_H), "input", MINT, |ui| {
                        Knob::new(&params.in_pan, setter, "Pan", MINT).bipolar().diameter(KNOB_D).show(ui);
                        Knob::new(&params.in_vol, setter, "Vol", MINT).diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(feedback_w, CARD_H), "feedback", PEACH, |ui| {
                        ui.vertical(|ui| {
                            ui.add_space(4.0);
                            ui.spacing_mut().item_spacing.y = 5.0;
                            let mode = params.fb_mode.value();
                            for (m, label) in [
                                (FeedbackMode::Normal, "normal"),
                                (FeedbackMode::Invert, "invert"),
                                (FeedbackMode::PingPong, "ping-pong"),
                            ] {
                                if pill(ui, label, mode == m, PEACH, vec2(76.0, 20.0)).clicked() {
                                    setter.begin_set_parameter(&params.fb_mode);
                                    setter.set_parameter(&params.fb_mode, m);
                                    setter.end_set_parameter(&params.fb_mode);
                                }
                            }
                        });
                        ui.add_space(6.0);
                        Knob::new(&params.fb_vol, setter, "Vol", PEACH).diameter(KNOB_D).show(ui);
                        Knob::new(&params.fb_cut, setter, "Cut", PEACH).diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(knobs_w(2), CARD_H), "time", SKY, |ui| {
                        if params.sync.value() {
                            Knob::new(&params.time_note, setter, "Time", SKY).diameter(KNOB_D).show(ui);
                        } else {
                            Knob::new(&params.time_ms, setter, "Time", SKY).diameter(KNOB_D).show(ui);
                        }
                        Knob::new(&params.offset, setter, "Offset", SKY).bipolar().diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(knobs_w(1), CARD_H), "dry", LAVENDER, |ui| {
                        Knob::new(&params.dry_vol, setter, "Vol", LAVENDER).diameter(KNOB_D).show(ui);
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

#[derive(Default)]
struct DelayEditor {
    /// Seconds since the last detected hit on the input (drives the tap glow).
    since_hit: f32,
    prev_input: f32,
    level: [f32; 2],
    clock: f32,
}

impl DelayEditor {
    fn update(&mut self, shared: &Shared, dt: f32) {
        let input = f32::from_bits(shared.input.load(Relaxed));
        if input > 0.02 && input > self.prev_input * 1.8 && self.since_hit > 0.08 {
            self.since_hit = 0.0;
        } else {
            self.since_hit += dt;
        }
        self.prev_input = input.max(self.prev_input * (-dt * 8.0).exp());
        for ch in 0..2 {
            let l = f32::from_bits(shared.level[ch].load(Relaxed));
            self.level[ch] = l.max(self.level[ch] * (-dt * 6.0).exp());
        }
        self.clock += dt;
    }

    fn display(&mut self, ui: &mut Ui, params: &DelayParams, shared: &Shared) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DISPLAY_H), Sense::hover());
        let painter = ui.painter_at(rect);
        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);

        let bpm = shared.bpm();
        let times = params.stereo_times(bpm);
        let mode: Mode = params.fb_mode.value().into();
        let pan = pan_gains(params.in_pan.value());
        let input = [params.in_vol.value() * pan[0] * 0.7, params.in_vol.value() * pan[1] * 0.7];
        let pattern = taps(input, times, params.fb_vol.value(), mode, 24);

        let left = rect.left() + 46.0;
        let right = rect.right() - 54.0;
        let span = pattern.last().map_or(1.0, |t| t.time_s).max(times[0].max(times[1]) * 4.0) * 1.08;
        let x = |t: f32| left + t / span * (right - left);
        let mid = rect.center().y;
        let reach = DISPLAY_H * 0.5 - 30.0;

        // Beat (synced) or time grid.
        let step = if params.sync.value() { 60.0 / bpm.max(20.0) } else { nice_step(span) };
        let mut t = step;
        let mut n = 1;
        while t < span {
            let xx = x(t);
            painter.line_segment(
                [pos2(xx, rect.top() + 14.0), pos2(xx, rect.bottom() - 14.0)],
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(150, 140, 180, 20)),
            );
            let label = if params.sync.value() { format!("{n}") } else { format!("{:.0} ms", t * 1000.0) };
            label_text(&painter, pos2(xx, rect.bottom() - 8.0), Align2::CENTER_CENTER, label, 9.0);
            t += step;
            n += 1;
        }
        painter.line_segment([pos2(left - 10.0, mid), pos2(right, mid)], Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.3)));
        label_text(&painter, pos2(rect.left() + 16.0, mid - reach * 0.5), Align2::CENTER_CENTER, "L", 11.0);
        label_text(&painter, pos2(rect.left() + 16.0, mid + reach * 0.5), Align2::CENTER_CENTER, "R", 11.0);

        // The dry hit itself.
        let seed_glow = (-self.since_hit * 6.0).exp();
        painter.circle_filled(pos2(left, mid), 6.0 + 6.0 * seed_glow, BUTTER.gamma_multiply(0.25 + 0.4 * seed_glow));
        painter.circle_filled(pos2(left, mid), 4.0, bundle_ui::darken(BUTTER, 0.1));

        // How much the cut filter darkens each repeat.
        let cut = params.fb_cut.value();
        let dull = 1.0 - ((cut / 200.0).ln() / (20_000f32 / 200.0).ln()).clamp(0.0, 1.0);
        for tap in &pattern {
            let side_sign = if tap.side == 0 { -1.0 } else { 1.0 };
            let a = tap.amp.abs().min(1.0).sqrt();
            let top = pos2(x(tap.time_s), mid + side_sign * (10.0 + a * reach));
            let base = if tap.side == 0 { MINT } else { PINK };
            let fade = (dull * 0.18 * tap.repeat as f32).min(0.8);
            let color = lerp_color(base, LAVENDER, fade);
            // Light up as the echo actually sounds after a hit.
            let age = self.since_hit - tap.time_s;
            let glow = if age >= 0.0 { (-age * 5.0).exp() } else { 0.0 };

            painter.line_segment(
                [pos2(top.x, mid), top],
                Stroke::new(1.4_f32, SAGE.gamma_multiply(0.35 + 0.4 * a)),
            );
            let r = 4.0 + 10.0 * a;
            if glow > 0.02 {
                painter.circle_filled(top, r * (1.6 + glow), color.gamma_multiply(0.3 * glow));
            }
            flower(&painter, top, r * (1.0 + 0.25 * glow), tap.repeat as f32 * 0.9 + self.clock * 0.3, color, tap.amp < 0.0);
        }

        // Wet output meters.
        for ch in 0..2 {
            let db = 20.0 * self.level[ch].max(1e-5).log10();
            let t = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            let x0 = rect.right() - 34.0 + ch as f32 * 12.0;
            let track = egui::Rect::from_x_y_ranges(x0..=(x0 + 6.0), (rect.top() + 18.0)..=(rect.bottom() - 18.0));
            painter.rect_filled(track, 3.0, TRACK);
            let fill = egui::Rect::from_x_y_ranges(track.x_range(), (track.bottom() - t * track.height())..=track.bottom());
            painter.rect_filled(fill, 3.0, if ch == 0 { MINT } else { PINK });
        }
        panel_frame(&painter, rect);
    }
}

fn nice_step(span: f32) -> f32 {
    [0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0]
        .into_iter()
        .find(|s| span / s <= 10.0)
        .unwrap_or(2.0)
}

/// Five-petal flower; inverted echoes get airy outlined petals.
fn flower(painter: &egui::Painter, c: Pos2, r: f32, rot: f32, color: Color32, hollow: bool) {
    for i in 0..5 {
        let a = rot + i as f32 * TAU / 5.0;
        let dir = vec2(a.cos(), a.sin());
        let (s, co) = a.sin_cos();
        let center = c + dir * r * 0.55;
        let pts: Vec<Pos2> = (0..12)
            .map(|k| {
                let t = k as f32 / 12.0 * TAU;
                let (x, y) = (t.cos() * r * 0.55, t.sin() * r * 0.3);
                center + vec2(x * co - y * s, x * s + y * co)
            })
            .collect();
        if hollow {
            painter.add(Shape::convex_polygon(pts, color.gamma_multiply(0.2), Stroke::new(1.2_f32, color)));
        } else {
            painter.add(Shape::convex_polygon(pts, color.gamma_multiply(0.85), Stroke::new(0.8_f32, bundle_ui::darken(color, 0.15))));
        }
    }
    painter.circle_filled(c, r * 0.24 + 1.0, BUTTER);
}

