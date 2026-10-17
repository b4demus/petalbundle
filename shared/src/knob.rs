//! Pastel rotary knob bound to a nih-plug parameter.

use nih_plug::prelude::{Param, ParamSetter};
use nih_plug_egui::egui::{
    pos2, vec2, Align2, Color32, FontId, Pos2, Response, Sense, Shape, Stroke, Ui,
};
use std::f32::consts::PI;

use crate::palette;

const START: f32 = PI * 0.75; // 7:30 o'clock
const SWEEP: f32 = PI * 1.5; // 270 degrees

pub struct Knob<'a, P: Param> {
    param: &'a P,
    setter: &'a ParamSetter<'a>,
    accent: Color32,
    label: &'a str,
    /// Fill the arc from 12 o'clock (for ± parameters such as pitch).
    bipolar: bool,
    diameter: f32,
    /// Show the value instead of the label when idle (for choice knobs).
    show_value: bool,
}

impl<'a, P: Param> Knob<'a, P> {
    pub fn new(param: &'a P, setter: &'a ParamSetter<'a>, label: &'a str, accent: Color32) -> Self {
        Self {
            param,
            setter,
            accent,
            label,
            bipolar: false,
            diameter: 50.0,
            show_value: false,
        }
    }

    pub fn bipolar(mut self) -> Self {
        self.bipolar = true;
        self
    }

    pub fn show_value(mut self) -> Self {
        self.show_value = true;
        self
    }

    /// Knob size (default 50). The widget is `d + 22` wide and `d + 30` tall.
    pub fn diameter(mut self, d: f32) -> Self {
        self.diameter = d;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let width = self.diameter + 22.0;
        let (rect, response) =
            ui.allocate_exact_size(vec2(width, self.diameter + 30.0), Sense::click_and_drag());
        self.interact(ui, &response);

        let value = self.param.unmodulated_normalized_value();
        let center = pos2(rect.center().x, rect.top() + self.diameter * 0.5 + 4.0);
        let r = self.diameter * 0.5;
        let active = response.hovered() || response.dragged();
        let painter = ui.painter_at(rect.expand(4.0));

        // Track and value arc.
        let arc_r = r - 1.0;
        painter.add(arc(
            center,
            arc_r,
            START,
            START + SWEEP,
            Stroke::new(4.0_f32, palette::TRACK),
        ));
        let (a0, a1) = if self.bipolar {
            let mid = START + SWEEP * 0.5;
            let v = START + SWEEP * value;
            (mid.min(v), mid.max(v))
        } else {
            (START, START + SWEEP * value)
        };
        if a1 - a0 > 0.01 {
            painter.add(arc(
                center,
                arc_r,
                a0,
                a1,
                Stroke::new(4.0_f32, self.accent),
            ));
        }

        // Body: soft drop shadow, cream cap and a lighter highlight.
        // Proportional so small knobs keep their look (equals r - 8 at d = 50).
        let body_r = r * 0.68;
        painter.circle_filled(center + vec2(0.0, 2.5), body_r + 1.0, palette::SHADOW);
        painter.circle_filled(center, body_r, palette::KNOB);
        painter.circle_filled(
            center - vec2(0.0, body_r * 0.18),
            body_r * 0.72,
            palette::KNOB_HI,
        );
        let ring = if active {
            self.accent
        } else {
            palette::KNOB_RING
        };
        painter.circle_stroke(center, body_r, Stroke::new(1.2_f32, ring));

        // Pointer dot.
        let angle = START + SWEEP * value;
        let dir = vec2(angle.cos(), angle.sin());
        let dot = (3.0 * self.diameter / 50.0).max(2.0);
        painter.circle_filled(center + dir * body_r * 0.65, dot, palette::INK);

        // Label, or the value while touched.
        let text = if active || self.show_value {
            self.param.normalized_value_to_string(value, true)
        } else {
            self.label.to_owned()
        };
        painter.text(
            pos2(rect.center().x, rect.bottom() - 9.0),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(12.0),
            if active {
                palette::INK
            } else {
                palette::INK_SOFT
            },
        );
        response
    }

    fn interact(&self, ui: &Ui, response: &Response) {
        let drag_id = response.id.with("drag_value");
        if response.double_clicked() {
            self.setter.begin_set_parameter(self.param);
            self.setter
                .set_parameter_normalized(self.param, self.param.default_normalized_value());
            self.setter.end_set_parameter(self.param);
            return;
        }
        if response.drag_started() {
            self.setter.begin_set_parameter(self.param);
            let v = self.param.unmodulated_normalized_value();
            ui.ctx().data_mut(|d| d.insert_temp(drag_id, v));
        }
        if response.dragged() {
            let fine = ui.input(|i| i.modifiers.shift);
            let speed = if fine { 0.0008 } else { 0.005 };
            let delta = -response.drag_delta().y * speed + response.drag_delta().x * speed * 0.3;
            let v = ui.ctx().data(|d| d.get_temp::<f32>(drag_id)).unwrap_or(0.0);
            let v = (v + delta).clamp(0.0, 1.0);
            ui.ctx().data_mut(|d| d.insert_temp(drag_id, v));
            self.setter.set_parameter_normalized(self.param, v);
        }
        if response.drag_stopped() {
            self.setter.end_set_parameter(self.param);
        }
        if response.hovered() && !response.dragged() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                let v =
                    (self.param.unmodulated_normalized_value() + scroll * 0.0015).clamp(0.0, 1.0);
                self.setter.begin_set_parameter(self.param);
                self.setter.set_parameter_normalized(self.param, v);
                self.setter.end_set_parameter(self.param);
            }
        }
    }
}

fn arc(center: Pos2, r: f32, a0: f32, a1: f32, stroke: Stroke) -> Shape {
    let n = ((a1 - a0) / 0.06).ceil().max(2.0) as usize;
    let points = (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    Shape::line(points, stroke)
}
