//! Shared pastel look for every plugin in the bundle.

use nih_plug::prelude::{Editor, GuiContext, ParentWindowHandle};
use nih_plug_egui::egui::{
    self, pos2, vec2, Align2, Color32, Context, FontId, Mesh, Pos2, Rect, RichText, Sense, Shape,
    Stroke, StrokeKind, Ui,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

mod knob;
pub use knob::Knob;

pub mod palette {
    use nih_plug_egui::egui::Color32;

    pub const BG: Color32 = Color32::from_rgb(245, 240, 234);
    pub const CARD: Color32 = Color32::from_rgb(252, 250, 247);
    pub const CARD_BORDER: Color32 = Color32::from_rgb(234, 226, 216);
    pub const INK: Color32 = Color32::from_rgb(79, 74, 94);
    pub const INK_SOFT: Color32 = Color32::from_rgb(142, 135, 156);
    pub const TRACK: Color32 = Color32::from_rgb(232, 225, 233);
    pub const SHADOW: Color32 = Color32::from_rgba_premultiplied(30, 22, 36, 34);
    pub const KNOB: Color32 = Color32::from_rgb(255, 252, 247);
    pub const KNOB_HI: Color32 = Color32::from_rgb(255, 255, 255);
    pub const KNOB_RING: Color32 = Color32::from_rgb(226, 217, 223);

    pub const MINT: Color32 = Color32::from_rgb(146, 210, 186);
    pub const PEACH: Color32 = Color32::from_rgb(245, 180, 146);
    pub const LAVENDER: Color32 = Color32::from_rgb(180, 164, 234);
    pub const PINK: Color32 = Color32::from_rgb(240, 164, 192);
    pub const SKY: Color32 = Color32::from_rgb(150, 194, 236);
    pub const BUTTER: Color32 = Color32::from_rgb(244, 220, 132);
    pub const SAGE: Color32 = Color32::from_rgb(150, 186, 150);
    pub const LILAC: Color32 = Color32::from_rgb(214, 160, 220);

    pub const SKY_TOP: Color32 = Color32::from_rgb(234, 228, 248);
    pub const SKY_BOTTOM: Color32 = Color32::from_rgb(251, 245, 237);
}

use palette::*;

/// Horizontal inner margins plus border of a [`card`].
pub const CARD_CHROME: f32 = 22.0;

/// Light pastel visuals; call from the editor's build closure.
pub fn apply_style(ctx: &Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = BG;
    visuals.override_text_color = Some(INK);
    ctx.set_visuals(visuals);
}

/// Page with the standard background and margins.
pub fn root(ctx: &Context, add: impl FnOnce(&mut Ui)) {
    let frame = egui::Frame::default().fill(BG).inner_margin(18.0);
    egui::CentralPanel::default().frame(frame).show(ctx, add);
}

/// Shows an editor laid out at its `EguiState` size `scale` times smaller.
///
/// egui-baseview tessellates at the window's own pixels-per-point, so egui's
/// zoom factor and layer transforms only resample already-rasterized text
/// (blurry). Instead the host is told the scaled size while the egui window
/// gets `host scale * scale` as its DPI factor: egui then lays out the full
/// design in points and renders it natively at the smaller pixel size.
pub struct ScaledEditor {
    inner: Box<dyn Editor>,
    scale: f32,
    host_scale_set: AtomicBool,
}

pub fn scaled(inner: Option<Box<dyn Editor>>, scale: f32) -> Option<Box<dyn Editor>> {
    inner.map(|inner| {
        Box::new(ScaledEditor {
            inner,
            scale,
            host_scale_set: AtomicBool::new(false),
        }) as Box<dyn Editor>
    })
}

impl Editor for ScaledEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        // Hosts that never report a DPI factor assume 1.0 when sizing the window.
        if !self.host_scale_set.load(Ordering::Relaxed) {
            self.inner.set_scale_factor(self.scale);
        }
        self.inner.spawn(parent, context)
    }

    fn size(&self) -> (u32, u32) {
        let (w, h) = self.inner.size();
        (
            (w as f32 * self.scale).round() as u32,
            (h as f32 * self.scale).round() as u32,
        )
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        let accepted = self.inner.set_scale_factor(factor * self.scale);
        if accepted {
            self.host_scale_set.store(true, Ordering::Relaxed);
        }
        accepted
    }

    fn param_value_changed(&self, id: &str, normalized_value: f32) {
        self.inner.param_value_changed(id, normalized_value);
    }

    fn param_modulation_changed(&self, id: &str, modulation_offset: f32) {
        self.inner.param_modulation_changed(id, modulation_offset);
    }

    fn param_values_changed(&self) {
        self.inner.param_values_changed();
    }
}

/// Plugin name + subtitle on the left, `right` widgets right-aligned.
pub fn header(ui: &mut Ui, title: &str, subtitle: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(28.0).color(INK));
        ui.add_space(6.0);
        ui.label(RichText::new(subtitle).size(13.0).color(INK_SOFT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

/// Rounded card with a coloured dot and title above its contents (laid out in a row).
pub fn card(ui: &mut Ui, width: f32, title: &str, accent: Color32, add: impl FnOnce(&mut Ui)) {
    card_sized(ui, vec2(width, 0.0), title, accent, add);
}

/// [`card`] with a minimum content height, so cards in a row line up.
pub fn card_sized(
    ui: &mut Ui,
    size: egui::Vec2,
    title: &str,
    accent: Color32,
    add: impl FnOnce(&mut Ui),
) {
    let width = size.x;
    egui::Frame::default()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, CARD_BORDER))
        .corner_radius(16.0)
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 8,
            bottom: 6,
        })
        .show(ui, |ui| {
            // The parent row is often horizontal; a card stacks title over contents.
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.set_min_height(size.y);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, accent);
                    ui.label(RichText::new(title).size(12.0).color(INK_SOFT));
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    add(ui);
                });
            });
        });
}

/// Rounded toggle/selector button.
pub fn pill(ui: &mut Ui, text: &str, on: bool, accent: Color32, size: egui::Vec2) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let radius = size.y * 0.5;
    if on {
        painter.rect_filled(rect, radius, accent);
    } else {
        let fill = if response.hovered() {
            lerp_color(CARD, accent, 0.2)
        } else {
            CARD
        };
        painter.rect_filled(rect, radius, fill);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0_f32, CARD_BORDER),
            StrokeKind::Inside,
        );
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(12.5),
        if on { Color32::WHITE } else { INK },
    );
    response
}

/// Paints `color` over the parts of `rect` outside its rounded corners.
pub fn corner_mask(painter: &egui::Painter, rect: Rect, r: f32, color: Color32) {
    use std::f32::consts::{FRAC_PI_2, PI};
    let corners = [
        (rect.left_top(), pos2(rect.left() + r, rect.top() + r), PI),
        (rect.right_top(), pos2(rect.right() - r, rect.top() + r), 1.5 * PI),
        (rect.right_bottom(), pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (rect.left_bottom(), pos2(rect.left() + r, rect.bottom() - r), FRAC_PI_2),
    ];
    let mut mesh = Mesh::default();
    for (corner, center, a0) in corners {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(corner, color);
        let steps = 10;
        for i in 0..=steps {
            let a = a0 + i as f32 / steps as f32 * FRAC_PI_2;
            mesh.colored_vertex(center + vec2(a.cos(), a.sin()) * r, color);
            if i > 0 {
                mesh.add_triangle(base, base + i, base + i + 1);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Rounded panel frame drawn over custom-painted content (masks the corners).
pub fn panel_frame(painter: &egui::Painter, rect: Rect) {
    let radius = 18.0;
    corner_mask(painter, rect, radius, BG);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0_f32, CARD_BORDER),
        StrokeKind::Inside,
    );
}

/// Vertical two-colour gradient.
pub fn gradient(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(Shape::mesh(mesh));
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn darken(c: Color32, k: f32) -> Color32 {
    let d = |x: u8| (x as f32 * (1.0 - k)) as u8;
    Color32::from_rgb(d(c.r()), d(c.g()), d(c.b()))
}

/// Small text helper used for axis labels.
pub fn label_text(painter: &egui::Painter, pos: Pos2, align: Align2, text: impl ToString, size: f32) {
    painter.text(pos, align, text.to_string(), FontId::proportional(size), INK_SOFT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Probe {
        scale: Mutex<Option<f32>>,
    }

    struct ProbeEditor(Arc<Probe>);

    impl Editor for ProbeEditor {
        fn spawn(&self, _: ParentWindowHandle, _: Arc<dyn GuiContext>) -> Box<dyn std::any::Any + Send> {
            Box::new(())
        }
        fn size(&self) -> (u32, u32) {
            (720, 560)
        }
        fn set_scale_factor(&self, factor: f32) -> bool {
            *self.0.scale.lock().unwrap() = Some(factor);
            true
        }
        fn param_value_changed(&self, _: &str, _: f32) {}
        fn param_modulation_changed(&self, _: &str, _: f32) {}
        fn param_values_changed(&self) {}
    }

    #[test]
    fn host_sees_scaled_size_and_window_gets_scaled_dpi() {
        let probe = Arc::new(Probe::default());
        let ed = scaled(Some(Box::new(ProbeEditor(probe.clone()))), 0.85).unwrap();
        assert_eq!(ed.size(), (612, 476));
        // A 125% host: the egui window renders the 720x560 layout at 1.0625 px/pt,
        // i.e. 765x595 physical pixels = 612x476 * 1.25, exactly the host's window.
        assert!(ed.set_scale_factor(1.25));
        assert!((probe.scale.lock().unwrap().unwrap() - 1.0625).abs() < 1e-6);
    }
}