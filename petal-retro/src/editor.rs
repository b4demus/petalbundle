use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, lerp_color, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::{DistortKind, NoiseKind, RetroParams, Shared, SCOPE_LEN};

const DESIGN: egui::Vec2 = vec2(720.0, 624.0);
const SCALE: f32 = 0.85;
const DISPLAY_H: f32 = 170.0;
const CARD_H: f32 = 84.0;
const KNOB_D: f32 = 42.0;

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

fn set_now<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

/// Module card: on/off pill, amount knob, then `second` (a knob or type pills).
fn module(
    ui: &mut Ui,
    width: f32,
    title: &str,
    color: Color32,
    on: &BoolParam,
    amount: &FloatParam,
    setter: &ParamSetter,
    second: impl FnOnce(&mut Ui),
) {
    let lit = on.value();
    let accent = if lit { color } else { TRACK };
    card_sized(ui, vec2(width, CARD_H), title, accent, |ui| {
        ui.vertical(|ui| {
            ui.add_space(14.0);
            if pill(ui, if lit { "on" } else { "off" }, lit, color, vec2(44.0, 22.0)).clicked() {
                set_now(setter, on, !lit);
            }
        });
        ui.add_space(4.0);
        Knob::new(amount, setter, "Amount", accent).diameter(KNOB_D).show(ui);
        second(ui);
    });
}

fn type_pills<T: Enum + PartialEq + Copy + 'static>(
    ui: &mut Ui,
    param: &EnumParam<T>,
    options: &[(T, &str)],
    color: Color32,
    setter: &ParamSetter,
) {
    ui.vertical(|ui| {
        ui.add_space(2.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        let current = param.value();
        for &(v, label) in options {
            if pill(ui, label, current == v, color, vec2(62.0, 20.0)).clicked() {
                set_now(setter, param, v);
            }
        }
    });
}

pub fn create(params: Arc<RetroParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        Record::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, rec| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal retro", "vintage colour", |_| {});
                ui.add_space(10.0);
                rec.display(ui, &params, &shared);
                ui.add_space(14.0);

                let w = (ui.available_width() - 2.0 * 12.0) / 3.0 - CARD_CHROME;
                let p = &params;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    module(ui, w, "noise", BUTTER, &p.noise_on, &p.noise, setter, |ui| {
                        type_pills(
                            ui,
                            &p.noise_type,
                            &[(NoiseKind::Hiss, "hiss"), (NoiseKind::Vinyl, "vinyl"), (NoiseKind::Hum, "hum")],
                            BUTTER,
                            setter,
                        );
                    });
                    module(ui, w, "wobble", SKY, &p.wobble_on, &p.wobble, setter, |ui| {
                        Knob::new(&p.wobble_rate, setter, "Rate", SKY).diameter(KNOB_D).show(ui);
                    });
                    module(ui, w, "distort", PEACH, &p.distort_on, &p.distort, setter, |ui| {
                        type_pills(
                            ui,
                            &p.distort_type,
                            &[(DistortKind::Warm, "warm"), (DistortKind::Tape, "tape"), (DistortKind::Fuzz, "fuzz")],
                            PEACH,
                            setter,
                        );
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    module(ui, w, "digital", LAVENDER, &p.digital_on, &p.digital, setter, |ui| {
                        Knob::new(&p.digital_rate, setter, "Rate", LAVENDER).diameter(KNOB_D).show(ui);
                    });
                    module(ui, w, "space", MINT, &p.space_on, &p.space, setter, |ui| {
                        Knob::new(&p.space_size, setter, "Size", MINT).diameter(KNOB_D).show(ui);
                    });
                    module(ui, w, "magnetic", PINK, &p.magnetic_on, &p.magnetic, setter, |ui| {
                        Knob::new(&p.dropout, setter, "Dropout", PINK).diameter(KNOB_D).show(ui);
                    });
                });
                ui.add_space(12.0);
                let full = ui.available_width() - CARD_CHROME;
                card_sized(ui, vec2(full, CARD_H), "master", INK_SOFT, |ui| {
                    // Five knobs, centred in the card.
                    let gap = 18.0;
                    let knobs = 5.0 * (KNOB_D + 22.0) + 4.0 * gap;
                    ui.add_space(((full - knobs) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = gap;
                    Knob::new(&p.magnitude, setter, "Magnitude", PINK).diameter(KNOB_D).show(ui);
                    Knob::new(&p.tone, setter, "Tone", SKY).bipolar().diameter(KNOB_D).show(ui);
                    Knob::new(&p.width, setter, "Width", MINT).diameter(KNOB_D).show(ui);
                    Knob::new(&p.mix, setter, "Mix", LAVENDER).diameter(KNOB_D).show(ui);
                    Knob::new(&p.output, setter, "Output", PEACH).diameter(KNOB_D).show(ui);
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

#[derive(Default)]
struct Record {
    angle: f32,
    scope: Vec<f32>,
    rng: u32,
}

impl Record {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn display(&mut self, ui: &mut Ui, p: &RetroParams, shared: &Shared) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DISPLAY_H), Sense::hover());
        let painter = ui.painter_at(rect);
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let m = p.magnitude.value();
        let amt = |on: &BoolParam, a: &FloatParam| if on.value() { a.value() * m } else { 0.0 };
        let noise = amt(&p.noise_on, &p.noise);
        let distort = amt(&p.distort_on, &p.distort);
        let digital = amt(&p.digital_on, &p.digital);
        let space = amt(&p.space_on, &p.space);
        let wobble = Shared::load(&shared.wobble);
        let dropout = Shared::load(&shared.dropout);

        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);

        // --- Record ------------------------------------------------------------
        let radius = 66.0;
        let center = pos2(rect.left() + 100.0, rect.center().y) + vec2(wobble * 5.0, wobble * 2.0);
        // 33 1/3 rpm, sped up / slowed down by the wobble.
        self.angle = (self.angle + dt * TAU * 0.555 * (1.0 + wobble * 0.08)) % TAU;
        let fade = dropout.clamp(0.0, 1.0);
        // Digital makes the record visibly polygonal.
        let sides = (64.0 - 56.0 * digital).round().max(6.0) as usize;

        for k in 0..3 {
            let a = space * (0.22 - k as f32 * 0.06);
            if a > 0.0 {
                painter.circle_filled(center, radius + 10.0 + k as f32 * 9.0, LAVENDER.gamma_multiply(a));
            }
        }
        painter.circle_filled(center + vec2(0.0, 4.0), radius + 1.0, SHADOW);
        let vinyl = lerp_color(Color32::from_rgb(92, 84, 110), Color32::from_rgb(110, 96, 120), distort * 0.5);
        painter.add(Shape::convex_polygon(polygon(center, radius, sides, self.angle), vinyl.gamma_multiply(0.35 + 0.65 * fade), Stroke::NONE));
        for g in 0..9 {
            let r = 30.0 + g as f32 * 4.0;
            let pts = polygon(center, r, sides, self.angle);
            painter.add(Shape::closed_line(pts, Stroke::new(0.8_f32, Color32::from_white_alpha(18))));
        }
        // Sheen.
        for k in [-1.0f32, 1.0] {
            let a = -0.8 + k * 0.12;
            painter.line_segment(
                [center + vec2(a.cos(), a.sin()) * 32.0, center + vec2(a.cos(), a.sin()) * (radius - 4.0)],
                Stroke::new(3.0_f32, Color32::from_white_alpha(22)),
            );
        }
        // Label warms up with the distortion.
        let label = lerp_color(PINK, PEACH, distort);
        painter.add(Shape::convex_polygon(polygon(center, 24.0, sides.min(48), self.angle), label, Stroke::NONE));
        let mark = center + vec2(self.angle.cos(), self.angle.sin()) * 15.0;
        painter.circle_filled(mark, 3.0, BUTTER);
        painter.circle_filled(center, 3.0, CARD);

        // Dust specks for the noise.
        let specks = (noise * 40.0) as usize;
        for _ in 0..specks {
            let a = self.rand() * TAU;
            let r = 28.0 + self.rand() * (radius - 28.0);
            let c = center + vec2(a.cos(), a.sin()) * r;
            painter.circle_filled(c, 0.8 + self.rand() * 1.2, Color32::from_white_alpha((60.0 + 140.0 * self.rand()) as u8));
        }

        // Tone arm.
        let pivot = pos2(center.x + radius + 22.0, rect.top() + 24.0);
        let stylus = center + vec2(30.0, -44.0);
        painter.line_segment([pivot, stylus], Stroke::new(3.0_f32, INK_SOFT.gamma_multiply(0.8)));
        painter.circle_filled(pivot, 6.0, CARD);
        painter.circle_stroke(pivot, 6.0, Stroke::new(1.5_f32, INK_SOFT));
        painter.circle_filled(stylus, 3.5, PEACH);

        // --- Scope ---------------------------------------------------------------
        let scope_rect = Rect::from_min_max(pos2(rect.left() + 220.0, rect.top() + 16.0), pos2(rect.right() - 16.0, rect.bottom() - 16.0));
        painter.rect_filled(scope_rect, 12.0, CARD.gamma_multiply(0.55));
        painter.line_segment(
            [pos2(scope_rect.left() + 8.0, scope_rect.center().y), pos2(scope_rect.right() - 8.0, scope_rect.center().y)],
            Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.25)),
        );
        let end = shared.scope_pos.load(Relaxed);
        self.scope.resize(SCOPE_LEN, 0.0);
        for k in 0..SCOPE_LEN {
            let i = end.wrapping_sub(SCOPE_LEN - k) % SCOPE_LEN;
            self.scope[k] = Shared::load(&shared.scope[i]);
        }
        let n = SCOPE_LEN;
        let half = scope_rect.height() * 0.42;
        let pts: Vec<Pos2> = self
            .scope
            .iter()
            .enumerate()
            .map(|(k, &v)| {
                pos2(
                    scope_rect.left() + 8.0 + k as f32 / (n - 1) as f32 * (scope_rect.width() - 16.0),
                    scope_rect.center().y - v.clamp(-1.2, 1.2) * half,
                )
            })
            .collect();
        let wave = lerp_color(LAVENDER, PEACH, distort * 0.7);
        painter.add(Shape::line(pts.clone(), Stroke::new(4.0_f32, wave.gamma_multiply(0.25))));
        painter.add(Shape::line(pts, Stroke::new(1.5_f32, bundle_ui::darken(wave, 0.1))));

        panel_frame(&painter, rect);
    }
}

fn polygon(c: Pos2, r: f32, sides: usize, rot: f32) -> Vec<Pos2> {
    (0..sides)
        .map(|i| {
            let a = rot + i as f32 / sides as f32 * TAU;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}
