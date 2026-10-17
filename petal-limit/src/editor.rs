use bundle_ui::palette::*;
use bundle_ui::{card_sized, gradient, header, label_text, panel_frame, pill, Knob, CARD_CHROME};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use crate::{LimitParams, Shared, StyleParam, HISTORY};

const DESIGN: egui::Vec2 = vec2(720.0, 456.0);
const SCALE: f32 = 0.85;
const DISPLAY_H: f32 = 250.0;
const METER_W: f32 = 150.0;
const STYLE_W: f32 = 146.0;
const CARD_H: f32 = 84.0;
/// Gain-reduction scale (dB) of the line hanging from the top of the wave view.
const GR_RANGE: f32 = 12.0;

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN.x as u32, DESIGN.y as u32)
}

fn set_now<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

pub fn create(params: Arc<LimitParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let editor = create_egui_editor(
        params.editor_state.clone(),
        LimitEditor::default(),
        |ctx, _| bundle_ui::apply_style(ctx),
        move |ctx, setter, ed| {
            ctx.request_repaint();
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            ed.update(&shared, dt);
            bundle_ui::root(ctx, |ui| {
                header(ui, "petal limit", "brickwall limiter", |ui| {
                    let text = format!("GR {:.1} dB", -ed.gr);
                    ui.label(egui::RichText::new(text).size(13.0).color(bundle_ui::darken(PINK, 0.25)));
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let wave_w = ui.available_width() - 12.0 - METER_W;
                    ed.wave(ui, wave_w, &params, setter, &shared);
                    ed.meters(ui, &shared);
                });
                ui.add_space(14.0);

                let left = ui.available_width() - 12.0 - STYLE_W - 2.0 * CARD_CHROME;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    card_sized(ui, vec2(left, CARD_H), "limiter", PEACH, |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        Knob::new(&params.gain, setter, "Gain", PEACH).show(ui);
                        Knob::new(&params.ceiling, setter, "Ceiling", PEACH).show(ui);
                        Knob::new(&params.release, setter, "Release", SKY).show(ui);
                        Knob::new(&params.lookahead, setter, "Lookahead", SKY).show(ui);
                        Knob::new(&params.link, setter, "Link", LAVENDER).show(ui);
                    });
                    card_sized(ui, vec2(STYLE_W, CARD_H), "style", LAVENDER, |ui| {
                        egui::Grid::new("styles").spacing([6.0, 5.0]).show(ui, |ui| {
                            let style = params.style.value();
                            let opts = [
                                (StyleParam::Transparent, "transparent"),
                                (StyleParam::Punchy, "punchy"),
                                (StyleParam::Aggressive, "aggressive"),
                                (StyleParam::Safe, "safe"),
                            ];
                            for (i, (s, label)) in opts.into_iter().enumerate() {
                                if pill(ui, label, style == s, LAVENDER, vec2(70.0, 20.0)).clicked() {
                                    set_now(setter, &params.style, s);
                                }
                                if i % 2 == 1 {
                                    ui.end_row();
                                }
                            }
                            let tp = params.true_peak.value();
                            if pill(ui, "true peak", tp, MINT, vec2(70.0, 20.0)).clicked() {
                                set_now(setter, &params.true_peak, !tp);
                            }
                            let unity = params.unity.value();
                            if pill(ui, "unity", unity, SKY, vec2(70.0, 20.0)).clicked() {
                                set_now(setter, &params.unity, !unity);
                            }
                            ui.end_row();
                        });
                    });
                });
            });
        },
    );
    bundle_ui::scaled(editor, SCALE)
}

#[derive(Default)]
struct LimitEditor {
    input: [f32; 2],
    output: [f32; 2],
    gr: f32,
    dragging: bool,
    hist_in: Vec<f32>,
    hist_out: Vec<f32>,
    hist_gr: Vec<f32>,
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-6).log10()
}

impl LimitEditor {
    fn update(&mut self, s: &Shared, dt: f32) {
        let fall = 24.0 * dt;
        for ch in 0..2 {
            self.input[ch] = db(Shared::load(&s.peak_in[ch])).max(self.input[ch] - fall);
            self.output[ch] = db(Shared::load(&s.peak_out[ch])).max(self.output[ch] - fall);
        }
        self.gr = Shared::load(&s.gr).max(self.gr - fall).max(0.0);
    }

    fn wave(&mut self, ui: &mut Ui, width: f32, p: &LimitParams, setter: &ParamSetter, s: &Shared) {
        let (rect, resp) = ui.allocate_exact_size(vec2(width, DISPLAY_H), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        gradient(&painter, rect, SKY_TOP, SKY_BOTTOM);

        let mid = rect.center().y;
        let half = rect.height() * 0.5 - 14.0;
        let ceiling = 10f32.powf(p.ceiling.value() / 20.0);

        // Drag either ceiling line (or scroll) to set the ceiling.
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let near = pointer.is_some_and(|q| ((q.y - mid).abs() - ceiling * half).abs() < 8.0);
        if near || self.dragging {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
        }
        if ui.input(|i| i.pointer.primary_pressed()) && resp.hovered() && near {
            self.dragging = true;
            setter.begin_set_parameter(&p.ceiling);
        }
        if self.dragging {
            if let Some(q) = pointer {
                let amp = ((q.y - mid).abs() / half).clamp(0.063, 1.0);
                setter.set_parameter(&p.ceiling, db(amp).clamp(-24.0, 0.0));
            }
            if !ui.input(|i| i.pointer.primary_down()) {
                self.dragging = false;
                setter.end_set_parameter(&p.ceiling);
            }
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if resp.hovered() && scroll.abs() > 0.0 {
            set_now(setter, &p.ceiling, (p.ceiling.value() + scroll * 0.01).clamp(-24.0, 0.0));
        }

        // History, newest at the right.
        let n = (rect.width() as usize).min(HISTORY);
        let end = s.hist_pos.load(Relaxed);
        self.hist_in.resize(n, 0.0);
        self.hist_out.resize(n, 0.0);
        self.hist_gr.resize(n, 0.0);
        for k in 0..n {
            let i = end.wrapping_sub(n - k) % HISTORY;
            self.hist_in[k] = Shared::load(&s.hist_in[i]);
            self.hist_out[k] = Shared::load(&s.hist_out[i]);
            self.hist_gr[k] = Shared::load(&s.hist_gr[i]);
        }
        let x = |k: usize| rect.left() + k as f32 / (n.max(2) - 1) as f32 * rect.width();
        // Input above the ceiling shows as the part that gets cut.
        mirrored(&painter, &self.hist_in, &x, mid, half, PEACH.gamma_multiply(0.45));
        mirrored(&painter, &self.hist_out, &x, mid, half, LAVENDER.gamma_multiply(0.75));
        painter.line_segment([pos2(rect.left(), mid), pos2(rect.right(), mid)], Stroke::new(1.0_f32, INK_SOFT.gamma_multiply(0.25)));

        // Gain reduction hanging from the top.
        let gr_y = |gr: f32| rect.top() + 6.0 + (gr / GR_RANGE).clamp(0.0, 1.0) * (half * 0.8);
        let line: Vec<Pos2> = self.hist_gr.iter().enumerate().map(|(k, &g)| pos2(x(k), gr_y(g))).collect();
        painter.add(Shape::line(line, Stroke::new(1.6_f32, bundle_ui::darken(PINK, 0.1))));

        // Ceiling lines and their handle.
        let hot = near || self.dragging;
        let color = if hot { bundle_ui::darken(PEACH, 0.25) } else { bundle_ui::darken(PEACH, 0.1) };
        for sign in [-1.0f32, 1.0] {
            let y = mid + sign * ceiling * half;
            let mut xx = rect.left();
            while xx < rect.right() {
                painter.line_segment([pos2(xx, y), pos2((xx + 7.0).min(rect.right()), y)], Stroke::new(if hot { 2.0_f32 } else { 1.3 }, color));
                xx += 11.0;
            }
        }
        let label = format!("ceiling {:.1} dB", p.ceiling.value());
        let galley = painter.layout_no_wrap(label, FontId::proportional(11.0), Color32::WHITE);
        let size = galley.size() + vec2(14.0, 6.0);
        let at = pos2(rect.right() - size.x - 10.0, (mid - ceiling * half - size.y - 4.0).max(rect.top() + 4.0));
        let handle = Rect::from_min_size(at, size);
        painter.rect_filled(handle, size.y * 0.5, color);
        painter.galley(handle.min + vec2(7.0, 3.0), galley, Color32::WHITE);
        label_text(&painter, pos2(rect.left() + 10.0, rect.bottom() - 10.0), Align2::LEFT_CENTER, "pink: gain reduction", 9.5);
        panel_frame(&painter, rect);
    }

    fn meters(&self, ui: &mut Ui, s: &Shared) {
        let (rect, _) = ui.allocate_exact_size(vec2(METER_W, DISPLAY_H), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, CARD);

        let top = rect.top() + 26.0;
        let bottom = rect.bottom() - 58.0;
        let y_of = |d: f32| top + ((0.0 - d) / 48.0).clamp(0.0, 1.0) * (bottom - top);
        let bar = |x: f32, d: f32, color: Color32| {
            let track = Rect::from_x_y_ranges(x..=(x + 9.0), top..=bottom);
            painter.rect_filled(track, 4.0, TRACK);
            let fill = Rect::from_x_y_ranges(track.x_range(), y_of(d)..=bottom);
            if d > -48.0 {
                painter.rect_filled(fill, 4.0, color);
            }
        };
        let x0 = rect.left() + 16.0;
        bar(x0, self.input[0], SKY);
        bar(x0 + 12.0, self.input[1], SKY);
        // Gain reduction grows downwards from the top (24 dB scale).
        let gr_track = Rect::from_x_y_ranges((x0 + 44.0)..=(x0 + 53.0), top..=bottom);
        painter.rect_filled(gr_track, 4.0, TRACK);
        let gr_bottom = top + (self.gr / 24.0).clamp(0.0, 1.0) * (bottom - top);
        if self.gr > 0.05 {
            painter.rect_filled(Rect::from_x_y_ranges(gr_track.x_range(), top..=gr_bottom), 4.0, PINK);
        }
        bar(x0 + 76.0, self.output[0], LAVENDER);
        bar(x0 + 88.0, self.output[1], LAVENDER);
        for (x, label) in [(x0 + 10.0, "in"), (x0 + 48.5, "gr"), (x0 + 86.0, "out")] {
            label_text(&painter, pos2(x, rect.top() + 13.0), Align2::CENTER_CENTER, label, 10.0);
        }
        for d in [-6.0f32, -12.0, -24.0, -36.0] {
            label_text(&painter, pos2(rect.right() - 10.0, y_of(d)), Align2::RIGHT_CENTER, format!("{d:.0}"), 8.5);
        }

        // Loudness readout.
        let m = Shared::load(&s.lufs_m);
        let st = Shared::load(&s.lufs_s);
        let fmt = |v: f32| if v <= -69.0 { "-inf".to_owned() } else { format!("{v:.1}") };
        painter.text(pos2(rect.left() + 14.0, rect.bottom() - 38.0), Align2::LEFT_CENTER, format!("M  {}", fmt(m)), FontId::proportional(13.0), INK);
        painter.text(pos2(rect.left() + 14.0, rect.bottom() - 18.0), Align2::LEFT_CENTER, format!("S  {}", fmt(st)), FontId::proportional(13.0), INK);
        label_text(&painter, pos2(rect.right() - 12.0, rect.bottom() - 28.0), Align2::RIGHT_CENTER, "LUFS", 10.0);
        panel_frame(&painter, rect);
    }
}

/// Peak envelope drawn symmetrically around the centre line.
fn mirrored(painter: &egui::Painter, data: &[f32], x: &dyn Fn(usize) -> f32, mid: f32, half: f32, color: Color32) {
    if data.len() < 2 {
        return;
    }
    let mut mesh = Mesh::default();
    for (k, &a) in data.iter().enumerate() {
        let h = a.min(1.2) * half;
        mesh.colored_vertex(pos2(x(k), mid - h), color);
        mesh.colored_vertex(pos2(x(k), mid + h), color);
        if k > 0 {
            let i = k as u32 * 2;
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i + 1, i);
        }
    }
    painter.add(Shape::mesh(mesh));
}
