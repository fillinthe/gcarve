//! A small drawing of a box as it is being defined (owner 2026-11-13: "show a representation of the box they are
//! defining in the window as they make the adjustments"): an oblique view, front face square on, the top and the
//! right side going back at 45°. The front's side edges show the finger joints (odd count, as `boxjoint` cuts
//! them), an open box shows its walls' thickness from above, and W / D / H are named on the edges. Painted fresh
//! each frame from the numbers in the dialog: nothing cached, nothing stored.

use eframe::egui::{self, pos2, vec2, Color32, Pos2, Sense, Shape, Stroke};

const FRONT: Color32 = Color32::from_rgb(0xc4, 0x9a, 0x6c);
const SIDE: Color32 = Color32::from_rgb(0xa0, 0x78, 0x50);
const TOP: Color32 = Color32::from_rgb(0xd6, 0xb2, 0x84);
const INSIDE: Color32 = Color32::from_rgb(0x5a, 0x42, 0x2c);
const GRAIN: Color32 = Color32::from_rgb(0x8c, 0x64, 0x40);
const EDGE: Color32 = Color32::from_rgb(0x2a, 0x1e, 0x14);

/// The box `b` describes (outside sizes), in `size` of room.
pub fn paint(ui: &mut egui::Ui, size: egui::Vec2, b: &gcarve_core::boxjoint::BoxSpec) {
    let (w, d, h, t, finger, lid) = (b.w, b.d, b.h, b.t, b.finger, b.lid);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    let ok = w > 0.0 && d > 0.0 && h > 0.0 && t > 0.0 && 2.0 * t < w.min(d) && t < h;
    if !ok {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, gcarve_ui::i18n::t("The box will show here"), egui::FontId::proportional(12.0), Color32::GRAY);
        return;
    }
    // oblique: depth goes up and right at 45°, half length
    let k = 0.5 * std::f64::consts::FRAC_1_SQRT_2;
    let (dw, dh) = (w + d * k, h + d * k);
    let margin = 26.0_f64;
    let s = ((f64::from(rect.width()) - 2.0 * margin) / dw).min((f64::from(rect.height()) - 2.0 * margin) / dh) as f32;
    let o = pos2(rect.center().x - (dw as f32 * s) / 2.0, rect.center().y + (dh as f32 * s) / 2.0);
    // x along the width, y into the depth, z up
    let at = |x: f64, y: f64, z: f64| -> Pos2 { o + vec2((x + y * k) as f32 * s, -((z + y * k) as f32) * s) };
    let face = |pts: Vec<Pos2>, fill: Color32| Shape::convex_polygon(pts, fill, Stroke::new(1.0, EDGE));
    // right side, then top, then front (back to front)
    p.add(face(vec![at(w, 0.0, 0.0), at(w, d, 0.0), at(w, d, h), at(w, 0.0, h)], SIDE));
    p.add(face(vec![at(0.0, 0.0, h), at(w, 0.0, h), at(w, d, h), at(0.0, d, h)], TOP));
    if !lid {
        // open: the hollow inside the walls, as deep as the box less its bottom
        p.add(face(vec![at(t, t, h), at(w - t, t, h), at(w - t, d - t, h), at(t, d - t, h)], INSIDE));
    }
    p.add(face(vec![at(0.0, 0.0, 0.0), at(w, 0.0, 0.0), at(w, 0.0, h), at(0.0, 0.0, h)], FRONT));
    // the fingers up both side edges of the front: every other one is the side panel's end grain
    let n = gcarve_core::boxjoint::count(h, finger);
    let step = h / n as f64;
    for i in (1..n).step_by(2) {
        let (z0, z1) = (step * i as f64, step * (i + 1) as f64);
        for x0 in [0.0, w - t] {
            p.add(face(vec![at(x0, 0.0, z0), at(x0 + t, 0.0, z0), at(x0 + t, 0.0, z1), at(x0, 0.0, z1)], GRAIN));
        }
    }
    // and along the front's bottom (and the lid's) edge: the bottom and lid own the corners, so their fingers are
    // the even ones, and the front panel's end grain shows in the odd ones
    let m = gcarve_core::boxjoint::count(w, finger);
    let step = w / m as f64;
    for i in (1..m).step_by(2) {
        let (x0, x1) = (step * i as f64, step * (i + 1) as f64);
        let mut zs = vec![0.0];
        if lid {
            zs.push(h - t);
        }
        for z0 in zs {
            p.add(face(vec![at(x0, 0.0, z0), at(x1, 0.0, z0), at(x1, 0.0, z0 + t), at(x0, 0.0, z0 + t)], GRAIN));
        }
    }
    // names of the edges
    let font = egui::FontId::proportional(12.0);
    let dim = ui.visuals().text_color();
    p.text(at(w / 2.0, 0.0, 0.0) + vec2(0.0, 10.0), egui::Align2::CENTER_CENTER, "W", font.clone(), dim);
    p.text(at(0.0, 0.0, h / 2.0) - vec2(10.0, 0.0), egui::Align2::CENTER_CENTER, "H", font.clone(), dim);
    p.text(at(w, d / 2.0, 0.0) + vec2(10.0, 4.0), egui::Align2::CENTER_CENTER, "D", font, dim);
    let fingers = gcarve_ui::i18n::tf("{n} fingers up each corner", &[("n", &n)]);
    resp.on_hover_text(fingers);
}
