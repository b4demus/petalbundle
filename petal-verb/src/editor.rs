use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, lerp_color, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::f32::consts::TAU;
use std::sync::Arc;

use crate::{Shared, VerbParams};

const DESIGN: egui::Vec2 = vec2(720.0, 418.0);
const SCALE: f32 = 0.85;
const POND_H: f32 = 210.0;
const CARD_H: f32 = 84.0;
const KNOB_D: f32 = 42.0;
/// Perspective squash of the ripples.
const TILT: f32 = 0.36;

const fn knobs_w(n: usize) -> f32 {
    n as f32 * (KNOB_D + 22.0) + (n as f32 - 1.0) * 2.0
}

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

pub fn create(params: Arc<VerbParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        Pond::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, pond| {
            ctx.request_repaint();
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal verb", "rain pond", |ui| {
                    let on = params.freeze.value();
                    let label = if on { "❄  frozen" } else { "❄  freeze" };
                    if pill(ui, label, on, SKY, vec2(96.0, 28.0)).clicked() {
                        setter.begin_set_parameter(&params.freeze);
                        setter.set_parameter(&params.freeze, !on);
                        setter.end_set_parameter(&params.freeze);
                    }
                });
                ui.add_space(10.0);
                pond.show(ui, &params, &shared);
                ui.add_space(14.0);

                let fixed = knobs_w(3) + knobs_w(2) + 3.0 * CARD_CHROME + 2.0 * 12.0;
                let space_w = ui.available_width() - fixed;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(space_w, CARD_H), "space", SKY, |ui| {
                        Knob::new(&params.predelay, setter, "Predelay", SKY).diameter(KNOB_D).show(ui);
                        Knob::new(&params.size, setter, "Size", SKY).diameter(KNOB_D).show(ui);
                        Knob::new(&params.decay, setter, "Decay", SKY).diameter(KNOB_D).show(ui);
                        Knob::new(&params.modulation, setter, "Mod", SKY).diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(knobs_w(3), CARD_H), "tone", MINT, |ui| {
                        Knob::new(&params.damp, setter, "Damp", MINT).diameter(KNOB_D).show(ui);
                        Knob::new(&params.low_cut, setter, "Low Cut", MINT).diameter(KNOB_D).show(ui);
                        Knob::new(&params.width, setter, "Width", MINT).diameter(KNOB_D).show(ui);
                    });
                    card_sized(ui, vec2(knobs_w(2), CARD_H), "mix", LAVENDER, |ui| {
                        Knob::new(&params.dry, setter, "Dry", LAVENDER).diameter(KNOB_D).show(ui);
                        Knob::new(&params.wet, setter, "Wet", LAVENDER).diameter(KNOB_D).show(ui);
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

struct Ripple {
    /// Position as a fraction of the pond.
    at: (f32, f32),
    /// Seconds since it started spreading (negative while in the pre-delay).
    age: f32,
    strength: f32,
}

pub struct Pond {
    ripples: Vec<Ripple>,
    prev_input: f32,
    since_hit: f32,
    wet: f32,
    rng: u32,
}

impl Default for Pond {
    fn default() -> Self {
        Self { ripples: Vec::new(), prev_input: 0.0, since_hit: 1.0, wet: 0.0, rng: 0x51ED_27A1 }
    }
}

impl Pond {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn show(&mut self, ui: &mut Ui, params: &VerbParams, shared: &Shared) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), POND_H), Sense::hover());
        let painter = ui.painter_at(rect);
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        let time = ui.input(|i| i.time) as f32;

        let size = params.size.value();
        let decay = params.decay.value();
        let freeze = params.freeze.value();
        let predelay = params.predelay.value() * 0.001;
        // Damping darkens the water's reflections.
        let damp = params.damp.value();
        let dull = 1.0 - ((damp / 1000.0).ln() / 20f32.ln()).clamp(0.0, 1.0);

        // A new hit on the input drops a stone (after the pre-delay).
        let input = Shared::load(&shared.input);
        if !freeze && input > 0.02 && input > self.prev_input * 1.8 && self.since_hit > 0.08 {
            self.since_hit = 0.0;
            let at = (0.18 + 0.64 * self.rand(), 0.42 + 0.3 * self.rand());
            if self.ripples.len() < 32 {
                self.ripples.push(Ripple { at, age: -predelay, strength: input.sqrt().min(1.0) });
            }
        } else {
            self.since_hit += dt;
        }
        self.prev_input = input.max(self.prev_input * (-dt * 8.0).exp());
        self.wet = Shared::load(&shared.wet).max(self.wet * (-dt * 4.0).exp());

        // Water.
        gradient(&painter, rect, SKY_TOP, lerp_color(SKY_BOTTOM, MINT, 0.18));
        for k in 0..14 {
            // Short drifting glints on the water surface.
            let seed = k as f32 * 12.9898;
            let fx = (seed.sin() * 43758.545).fract().abs();
            let fy = 0.18 + 0.7 * ((seed * 1.7).cos() * 24634.634).fract().abs();
            let x = rect.left() + 30.0 + fx * (rect.width() - 120.0) + (time * 0.35 + k as f32).sin() * 10.0;
            let y = rect.top() + fy * rect.height();
            let len = 10.0 + 26.0 * fx;
            let alpha = (40.0 * (0.5 + 0.5 * (time * 0.8 + seed).sin())) as u8;
            painter.line_segment([pos2(x, y), pos2(x + len, y)], Stroke::new(1.2_f32, Color32::from_white_alpha(alpha)));
        }
        lily_pads(&painter, rect, time);

        // Ripples: speed follows Size, fading follows Decay, colour follows Damp.
        let speed = 40.0 + 170.0 * size;
        let ring_color = bundle_ui::darken(lerp_color(SKY, LAVENDER, dull * 0.8), 0.12);
        for r in &mut self.ripples {
            if !freeze {
                r.age += dt;
            }
        }
        let max_r = rect.width();
        self.ripples.retain(|r| r.age < 0.0 || freeze || (10f32.powf(-1.5 * r.age / decay) * r.strength > 0.02 && r.age * speed < max_r));
        for r in &self.ripples {
            if r.age < 0.0 {
                // Still in the pre-delay: a small glowing drop.
                let c = pos2(rect.left() + r.at.0 * rect.width(), rect.top() + r.at.1 * rect.height());
                painter.circle_filled(c, 3.0, BUTTER.gamma_multiply(0.8));
                continue;
            }
            let center = pos2(rect.left() + r.at.0 * rect.width(), rect.top() + r.at.1 * rect.height());
            let fade = if freeze { 0.6 } else { 10f32.powf(-1.5 * r.age / decay) } * r.strength;
            for ring in 0..4 {
                let ring_age = r.age - ring as f32 * 0.14;
                if ring_age <= 0.0 {
                    continue;
                }
                let radius = ring_age * speed;
                let a = (fade * (1.0 - ring as f32 * 0.2)).clamp(0.0, 1.0);
                // sqrt keeps the colour readable through most of the tail.
                ellipse(&painter, center, radius, Stroke::new(2.4_f32, ring_color.gamma_multiply(a.sqrt() * 0.9)));
                ellipse(&painter, center + vec2(0.0, -2.0), radius, Stroke::new(0.8_f32, Color32::from_white_alpha((60.0 * a) as u8)));
            }
            if r.age < 0.25 {
                painter.circle_filled(center, 4.0 * (1.0 - r.age * 4.0), BUTTER.gamma_multiply(0.9));
            }
        }

        // Wet level on the right.
        let db = 20.0 * self.wet.max(1e-5).log10();
        let t = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
        let track = Rect::from_x_y_ranges((rect.right() - 26.0)..=(rect.right() - 18.0), (rect.top() + 18.0)..=(rect.bottom() - 18.0));
        painter.rect_filled(track, 4.0, TRACK);
        painter.rect_filled(
            Rect::from_x_y_ranges(track.x_range(), (track.bottom() - t * track.height())..=track.bottom()),
            4.0,
            LAVENDER,
        );

        let info = format!(
            "decay {:.1} s · predelay {:.0} ms · size {:.0}%",
            decay,
            predelay * 1000.0,
            size * 100.0
        );
        label_text(&painter, pos2(rect.left() + 14.0, rect.bottom() - 12.0), Align2::LEFT_CENTER, info, 10.5);
        if freeze {
            painter.text(
                rect.left_top() + vec2(14.0, 12.0),
                Align2::LEFT_TOP,
                "❄ frozen",
                egui::FontId::proportional(12.0),
                bundle_ui::darken(SKY, 0.2),
            );
        }
        panel_frame(&painter, rect);
    }
}

fn ellipse(painter: &egui::Painter, c: Pos2, r: f32, stroke: Stroke) {
    if r < 0.5 || stroke.color.a() == 0 {
        return;
    }
    let n = ((r / 3.0) as usize).clamp(24, 96);
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = i as f32 / n as f32 * TAU;
            c + vec2(a.cos() * r, a.sin() * r * TILT)
        })
        .collect();
    painter.add(Shape::line(pts, stroke));
}

/// A few decorative lily pads drifting slightly.
fn lily_pads(painter: &egui::Painter, rect: Rect, time: f32) {
    const PADS: [(f32, f32, f32); 3] = [(0.12, 0.78, 22.0), (0.74, 0.3, 16.0), (0.86, 0.74, 19.0)];
    for (i, &(fx, fy, r)) in PADS.iter().enumerate() {
        let drift = vec2((time * 0.3 + i as f32).sin() * 3.0, (time * 0.23 + i as f32).cos() * 1.5);
        let c = pos2(rect.left() + fx * rect.width(), rect.top() + fy * rect.height()) + drift;
        // Pad with a notch, squashed like the ripples.
        let notch = 0.5 + i as f32;
        let mut pts = vec![c];
        for k in 0..=40 {
            let a = notch + 0.35 + k as f32 / 40.0 * (TAU - 0.7);
            pts.push(c + vec2(a.cos() * r, a.sin() * r * TILT * 1.2));
        }
        let fill = SAGE.gamma_multiply(0.55);
        // Fan triangles from the centre (the notch makes it non-convex).
        let mut mesh = egui::Mesh::default();
        for p in &pts {
            mesh.colored_vertex(*p, fill);
        }
        for k in 1..pts.len() as u32 - 1 {
            mesh.add_triangle(0, k, k + 1);
        }
        painter.add(Shape::mesh(mesh));
        if i == 0 {
            painter.circle_filled(c + vec2(-4.0, -5.0), 4.5, PINK);
            painter.circle_filled(c + vec2(-4.0, -5.0), 1.8, BUTTER);
        }
    }
}
