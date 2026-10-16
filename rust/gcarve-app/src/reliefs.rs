//! 3D models on the board (Qt canvas_view.cpp reliefImages_ + canvas_input.cpp Drag::ReliefMove): a shaded top-down
//! preview in each model's rectangle (cached per model), a dashed outline, and Select-tool dragging to place it.

use crate::canvas::{Canvas, Drag};
use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, Stroke, TextureHandle, TextureOptions};
use gcarve_core::document::Relief;
use gcarve_core::geom::{pt, Pt};
use gcarve_core::sim::{shade, HeightField};
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

/// Preview textures by relief id, with the key of the height data they were shaded from.
#[derive(Default)]
pub struct ReliefTextures(std::collections::HashMap<i32, (u64, TextureHandle)>);

/// ponytail: sampled hash (every 61st height + size + depth), not the whole field; a re-import that changes only
/// unsampled cells keeps the old preview until the next size/depth change
fn key(r: &Relief) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (r.nx, r.ny, r.depth.to_bits(), r.mirror, r.height.len()).hash(&mut h);
    for v in r.height.iter().step_by(61) {
        v.to_bits().hash(&mut h);
    }
    h.finish()
}

/// Shaded RGBA (row 0 = top) of a model as carved: top at the surface, base at -depth, never "through".
pub fn preview(r: &Relief, m: &gcarve_core::document::Material) -> Vec<u8> {
    let hf = HeightField { nx: r.nx, ny: r.ny, res: r.w / r.nx.max(1) as f64, x0: r.x, y0: r.y, depth: r.height.iter().map(|h| (r.depth * (1.0 - *h as f64)) as f32).collect() };
    let mut mm = m.clone();
    mm.t = mm.t.max(r.depth * 1.5);
    shade(&hf, &mm)
}

impl Canvas {
    pub(crate) fn paint_reliefs(&mut self, ctx: &egui::Context, painter: &egui::Painter) {
        let live: BTreeSet<i32> = self.doc.reliefs.iter().map(|r| r.id).collect();
        self.relief_tex.0.retain(|id, _| live.contains(id));
        for r in self.doc.reliefs.iter().filter(|r| r.nx > 0 && r.ny > 0 && r.height.len() == (r.nx * r.ny) as usize) {
            let k = key(r);
            if self.relief_tex.0.get(&r.id).is_none_or(|(old, _)| *old != k) {
                let img = ColorImage::from_rgba_unmultiplied([r.nx as usize, r.ny as usize], &preview(r, &self.doc.material));
                self.relief_tex.0.insert(r.id, (k, ctx.load_texture(format!("relief-{}", r.id), img, TextureOptions::LINEAR)));
            }
        }
        let Some(view) = self.view.as_ref() else { return };
        for r in &self.doc.reliefs {
            let rect = Rect::from_two_pos(view.to_screen(pt(r.x, r.y + r.h)), view.to_screen(pt(r.x + r.w, r.y)));
            if let Some((_, tex)) = self.relief_tex.0.get(&r.id) {
                painter.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE.gamma_multiply(0.9));
            }
            let c = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
            painter.extend(egui::Shape::dashed_line(&c, Stroke::new(1.0, Color32::from_rgb(0xc9, 0xa9, 0xff)), 4.0, 2.0));
        }
    }

    /// Topmost model whose rectangle contains `w`.
    pub fn hit_relief(&self, w: Pt) -> Option<i32> {
        self.doc.reliefs.iter().rev().find(|r| w.x >= r.x && w.x <= r.x + r.w && w.y >= r.y && w.y <= r.y + r.h).map(|r| r.id)
    }

    /// Select tool pressed on empty space over a model: start moving it (true), else the caller starts a marquee.
    pub(crate) fn start_relief_drag(&mut self, w: Pt) -> bool {
        let Some(id) = self.hit_relief(w) else { return false };
        let r = self.doc.reliefs.iter().find(|r| r.id == id).unwrap();
        self.relief_drag = (id, pt(r.x, r.y));
        self.doc.checkpoint();
        self.drag = Drag::ReliefMove;
        self.status = "Drag to move the 3D model; set exact size and position in Toolpaths › 3D Models".into();
        true
    }

    /// The model's corner follows the pointer, snapped like a shape corner; toolpaths regenerate on release.
    pub(crate) fn relief_drag_to(&mut self, w: Pt) {
        let (id, o) = self.relief_drag;
        let to = self.snap_at(o + (w - self.press_w), &BTreeSet::new()).to;
        if let Some(r) = self.doc.reliefs.iter_mut().find(|r| r.id == id) {
            (r.x, r.y) = (to.x, to.y);
            self.gen_stale = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_and_drag_place_the_model() {
        let mut c = Canvas::default();
        c.doc.reliefs.push(Relief { id: 7, x: 1.0, y: 1.0, w: 2.0, h: 1.0, depth: 0.25, nx: 4, ny: 2, height: vec![0.0, 0.5, 1.0, 0.5, 0.0, 0.5, 1.0, 0.5], ..Default::default() });
        let img = preview(&c.doc.reliefs[0], &c.doc.material);
        assert_eq!(img.len(), 4 * 2 * 4);
        assert!(img.chunks(4).all(|p| p[3] == 255), "a model preview is never cut through");
        assert_eq!(c.hit_relief(pt(2.0, 1.5)), Some(7));
        assert_eq!(c.hit_relief(pt(0.5, 1.5)), None);
        c.press_w = pt(2.0, 1.5);
        assert!(c.start_relief_drag(pt(2.0, 1.5)));
        c.relief_drag_to(pt(3.0, 2.0));
        let r = &c.doc.reliefs[0];
        assert!((r.x - 2.0).abs() < 1e-9 && (r.y - 1.5).abs() < 1e-9);
        c.undo();
        assert!((c.doc.reliefs[0].x - 1.0).abs() < 1e-9, "one undo step");
    }
}
