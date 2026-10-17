use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, Pos2, Sense, Shape, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::dsp::{tap_base_ms, tap_pan, MAX_TAPS};
use crate::{ModParams, Shared, ShapeParam, Style, STYLES};

const DESIGN: egui::Vec2 = vec2(720.0, 548.0);
const SCALE: f32 = 0.85;
const DISPLAY_H: f32 = 200.0;
const CARD_H: f32 = 84.0;
const KNOB_D: f32 = 44.0;

const TAP_COLORS: [Color32; MAX_TAPS] = [MINT, PINK, SKY, PEACH, LAVENDER, BUTTER, SAGE, LILAC];

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

fn apply(style: &Style, p: &ModParams, setter: &ParamSetter) {
    set_now(setter, &p.taps, style.taps);
    set_now(setter, &p.delay, style.delay);
    set_now(setter, &p.spread, style.spread);
    set_now(setter, &p.rate, style.rate);
    set_now(setter, &p.depth, style.depth);
    set_now(setter, &p.shape, style.shape);
    set_now(setter, &p.feedback, style.feedback);
    set_now(setter, &p.diffusion, style.diffusion);
    set_now(setter, &p.warp, style.warp);
    set_now(setter, &p.width, style.width);
    set_now(setter, &p.mix, style.mix);
    set_now(setter, &p.high_cut, style.high_cut);
}

/// A style is "on" while the defining settings still match it.
fn matches(style: &Style, p: &ModParams) -> bool {
    p.taps.value() == style.taps
        && (p.delay.value() - style.delay).abs() < 0.05
        && p.shape.value() == style.shape
        && (p.feedback.value() - style.feedback).abs() < 0.01
}

pub fn create(params: Arc<ModParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        (),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, _| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal mod", "multi-tap modulation", |ui| {
                    // Right-to-left layout: walk the styles backwards.
                    for style in STYLES.iter().rev() {
                        let on = matches(style, &params);
                        if pill(ui, style.name, on, LAVENDER, vec2(70.0, 26.0)).clicked() {
                            apply(style, &params, setter);
                        }
                    }
                });
                ui.add_space(10.0);
                display(ui, &params, &shared);
                ui.add_space(14.0);

                let right = knobs_w(4);
                let left = ui.available_width() - 12.0 - right - 2.0 * CARD_CHROME;
                let p = &params;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(left, CARD_H), "time", SKY, |ui| {
                        Knob::new(&p.delay, setter, "Delay", SKY).diameter(KNOB_D).show(ui);
                        Knob::new(&p.taps, setter, "Taps", SKY).diameter(KNOB_D).show(ui);
                        Knob::new(&p.spread, setter, "Spread", SKY).diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(right, CARD_H), "mod", LAVENDER, |ui| {
                        Knob::new(&p.rate, setter, "Rate", LAVENDER).diameter(KNOB_D).show(ui);
                        Knob::new(&p.depth, setter, "Depth", LAVENDER).diameter(KNOB_D).show(ui);
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.add_space(2.0);
                            ui.spacing_mut().item_spacing.y = 4.0;
                            let shape = p.shape.value();
                            for (s, label) in [(ShapeParam::Sine, "sine"), (ShapeParam::Triangle, "triangle"), (ShapeParam::Wander, "wander")] {
                                if pill(ui, label, shape == s, LAVENDER, vec2(76.0, 20.0)).clicked() {
                                    set_now(setter, &p.shape, s);
                                }
                            }
                        });
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(left, CARD_H), "feedback", PEACH, |ui| {
                        Knob::new(&p.feedback, setter, "Feedback", PEACH).bipolar().diameter(KNOB_D).show(ui);
                        Knob::new(&p.diffusion, setter, "Diffusion", PEACH).diameter(KNOB_D).show(ui);
                        Knob::new(&p.warp, setter, "Warp", PEACH).diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(right, CARD_H), "tone", MINT, |ui| {
                        Knob::new(&p.low_cut, setter, "Low Cut", MINT).diameter(KNOB_D).show(ui);
                        Knob::new(&p.high_cut, setter, "High Cut", MINT).diameter(KNOB_D).show(ui);
                        Knob::new(&p.width, setter, "Width", MINT).diameter(KNOB_D).show(ui);
                        Knob::new(&p.mix, setter, "Mix", PINK).diameter(KNOB_D).show(ui);
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

fn display(ui: &mut Ui, p: &ModParams, shared: &Shared) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DISPLAY_H), Sense::hover());
    let painter = ui.painter_at(rect);
    let time = ui.input(|i| i.time) as f32;
    gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);

    let s = p.settings();
    let n = s.taps.clamp(1, MAX_TAPS);
    let reach = crate::dsp::depth_ms(s.delay_ms, s.depth);
    let longest = (0..n).map(|k| tap_base_ms(&s, k)).fold(0.0, f32::max) + reach;
    let repeats = if s.feedback.abs() > 0.05 { 3 } else { 0 };
    let span = (longest * (1 + repeats) as f32 * 1.1).max(4.0);

    let left = rect.left() + 40.0;
    let right = rect.right() - 20.0;
    let x = |ms: f32| left + ms / span * (right - left);
    let mid = rect.center().y - 6.0;

    // Time grid.
    let step = [1.0f32, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0].into_iter().find(|s| span / s <= 8.0).unwrap_or(1000.0);
    let mut t = step;
    while t < span {
        let xx = x(t);
        painter.line_segment([pos2(xx, rect.top() + 14.0), pos2(xx, rect.bottom() - 34.0)], Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(150, 140, 180, 20)));
        label_text(&painter, pos2(xx, rect.bottom() - 26.0), Align2::CENTER_CENTER, format!("{t:.0} ms"), 9.0);
        t += step;
    }
    painter.line_segment([pos2(left - 10.0, mid), pos2(right, mid)], Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.3)));
    label_text(&painter, pos2(rect.left() + 16.0, mid - 36.0), Align2::CENTER_CENTER, "L", 11.0);
    label_text(&painter, pos2(rect.left() + 16.0, mid + 36.0), Align2::CENTER_CENTER, "R", 11.0);
    painter.circle_filled(pos2(left, mid), 4.0, bundle_ui::darken(BUTTER, 0.1));

    // Taps: flowers swaying with their live delay.
    for k in 0..n {
        let ms = f32::from_bits(shared.tap_ms[k].load(Relaxed));
        let ms = if ms > 0.0 { ms } else { tap_base_ms(&s, k) };
        let pan = tap_pan(k, n, s.width);
        // Left (and centred) taps grow upwards, right taps downwards.
        let offset = 22.0 + 42.0 * pan.abs();
        let top = pos2(x(ms), if pan <= 0.0 { mid - offset } else { mid + offset });
        let color = TAP_COLORS[k];
        // The swing range of this tap.
        let base = tap_base_ms(&s, k);
        painter.line_segment(
            [pos2(x(base - reach), top.y), pos2(x(base + reach), top.y)],
            Stroke::new(4.0_f32, color.gamma_multiply(0.18)),
        );
        painter.line_segment([pos2(top.x, mid), top], Stroke::new(1.4_f32, SAGE.gamma_multiply(0.6)));
        flower(&painter, top, 8.0, time * 0.5 + k as f32, color);
        // Feedback repeats: fading ghosts of this tap.
        for r in 1..=repeats {
            let a = s.feedback.abs().powi(r as i32);
            let g = pos2(x(ms + r as f32 * base), top.y);
            if g.x < right {
                painter.circle_filled(g, 6.0 * a + 2.0, color.gamma_multiply(0.5 * a + 0.1));
                if s.feedback < 0.0 && r % 2 == 1 {
                    // Inverted repeats get an outline instead of a fill.
                    painter.circle_stroke(g, 6.0 * a + 4.0, Stroke::new(1.0_f32, color));
                }
            }
        }
    }

    // LFO shape preview along the bottom.
    let lfo_y = rect.bottom() - 12.0;
    let pts: Vec<Pos2> = (0..=120)
        .map(|i| {
            let u = i as f32 / 120.0;
            let ph = (u * 2.0 + time * s.rate_hz.min(4.0) * 0.25) % 1.0;
            let v = match s.shape {
                crate::dsp::Shape::Sine => (TAU * ph).sin(),
                crate::dsp::Shape::Triangle => 1.0 - 4.0 * (ph - 0.5).abs(),
                crate::dsp::Shape::Wander => (u * 9.0 + time * 0.3).sin() * 0.6 + (u * 23.0 - time * 0.2).sin() * 0.4,
            };
            pos2(left + u * (right - left), lfo_y - v * 5.0)
        })
        .collect();
    painter.add(Shape::line(pts, Stroke::new(1.2_f32, LAVENDER.gamma_multiply(0.7))));
    panel_frame(&painter, rect);
}

fn flower(painter: &egui::Painter, c: Pos2, r: f32, rot: f32, color: Color32) {
    for i in 0..5 {
        let a = rot + i as f32 * TAU / 5.0;
        let (s, co) = a.sin_cos();
        let center = c + vec2(co, s) * r * 0.55;
        let pts: Vec<Pos2> = (0..12)
            .map(|k| {
                let t = k as f32 / 12.0 * TAU;
                let (px, py) = (t.cos() * r * 0.55, t.sin() * r * 0.3);
                center + vec2(px * co - py * s, px * s + py * co)
            })
            .collect();
        painter.add(Shape::convex_polygon(pts, color.gamma_multiply(0.9), Stroke::new(0.8_f32, bundle_ui::darken(color, 0.15))));
    }
    painter.circle_filled(c, r * 0.28 + 1.0, BUTTER);
}
