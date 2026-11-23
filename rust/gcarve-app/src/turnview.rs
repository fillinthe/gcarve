//! A small side view of a turned part as it is being set up (Pocket › Turning on a rotary job): the blank lying
//! along X, the part the picked profile gives (mirrored about the axis, as turning makes it), and the lengths the
//! chuck and tailstock hold. Painted fresh each frame from the drawing: nothing cached, nothing stored.

use eframe::egui::{self, pos2, Color32, Pos2, Sense, Shape, Stroke};
use gcarve_core::document::Material;
use gcarve_core::geom::Contour;

const WOOD: Color32 = Color32::from_rgb(0xc4, 0x9a, 0x6c);
const EDGE: Color32 = Color32::from_rgb(0x2a, 0x1e, 0x14);
const HELD: Color32 = Color32::from_rgb(0x70, 0x74, 0x7c);

/// The part `profile` (the picked vectors) turns out of the job's blank, in `size` of room.
pub fn paint(ui: &mut egui::Ui, size: egui::Vec2, profile: &[Contour], m: &Material) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    let shape = m.rotary.map(|dia| (dia, gcarve_core::turning::radius_along(profile, m, 160)));
    let Some((dia, radii)) = shape.filter(|(_, r)| r.len() >= 2 && m.w > 0.0) else {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, gcarve_ui::i18n::t("Pick the side profile (an open line along the blank) to see the part here"), egui::FontId::proportional(12.0), Color32::GRAY);
        return;
    };
    let margin = 10.0;
    let s = ((rect.width() - 2.0 * margin) / m.w as f32).min((rect.height() - 2.0 * margin) / dia as f32);
    let at = |x: f64, r: f64| -> Pos2 { pos2(rect.center().x + (x - m.w / 2.0) as f32 * s, rect.center().y - r as f32 * s) };
    // the blank, faint, then the turned part over it
    let half = dia / 2.0;
    p.rect_stroke(egui::Rect::from_two_pos(at(0.0, half), at(m.w, -half)), 0.0, Stroke::new(1.0, Color32::from_gray(110)), egui::StrokeKind::Inside);
    let top: Vec<Pos2> = radii.iter().map(|(x, r)| at(*x, *r)).collect();
    // strips, not one polygon: a waisted shape is not convex
    for (a, b) in radii.iter().zip(radii.iter().skip(1)) {
        p.add(Shape::convex_polygon(vec![at(a.0, a.1), at(b.0, b.1), at(b.0, -b.1), at(a.0, -a.1)], WOOD, Stroke::NONE));
    }
    p.add(Shape::line(top.clone(), Stroke::new(1.2, EDGE)));
    p.add(Shape::line(radii.iter().map(|(x, r)| at(*x, -*r)).collect(), Stroke::new(1.2, EDGE)));
    // what holds the blank
    for (x0, x1) in [(0.0, m.chuck), (m.w - m.tail, m.w)] {
        if x1 > x0 {
            p.rect_filled(egui::Rect::from_two_pos(at(x0, half * 1.15), at(x1, -half * 1.15)), 2.0, HELD.gamma_multiply(0.75));
        }
    }
    p.line_segment([at(0.0, 0.0), at(m.w, 0.0)], Stroke::new(1.0, Color32::from_gray(90)));
}
