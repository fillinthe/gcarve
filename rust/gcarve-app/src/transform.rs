//! Transform frame on the selection (Select tool): 8 scale handles (corners keep proportions, Shift = free; edges
//! scale one axis; the opposite side stays put), a rotate handle above the top edge (any angle, Shift = 15° steps)
//! around the pivot, and the pivot itself (drag it anywhere; it starts at the selection's centre). `impl Canvas`.

use crate::canvas::{Canvas, Drag, View, HIT_PX};
use crate::theme;
use eframe::egui::{self, Color32, Pos2, Stroke};
use gcarve_core::geom::{pt, transformed, Affine, Pt};

/// Screen pixels from the top edge to the rotate handle.
const ROT_PX: f32 = 26.0;
const FRAME: Color32 = Color32::from_rgb(0x6c, 0xb6, 0xff);

/// Which part of the frame: 0..8 scale handles (corners 0 BL, 2 BR, 4 TR, 6 TL; edges between), 8 rotate, 9 pivot.
pub(crate) type Handle = u8;
pub(crate) const ROTATE: Handle = 8;
pub(crate) const PIVOT: Handle = 9;

impl Canvas {
    /// The pivot for rotation: the user's point while the selection stays the same, else the selection's centre.
    pub(crate) fn pivot(&self) -> Pt {
        match self.pivot {
            Some((p, ref ids)) if *ids == self.sel => p,
            _ => {
                let r = self.sel_rect;
                pt((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0)
            }
        }
    }

    fn frame_visible(&self) -> bool {
        self.tool == crate::canvas::Tool::Select && !self.sel.is_empty() && !self.sel_rect.is_empty() && self.view.is_some()
    }

    /// Handle positions in document coordinates (index = Handle 0..8).
    fn handle_points(&self) -> [Pt; 8] {
        let r = self.sel_rect;
        let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        [pt(r.x0, r.y0), pt(cx, r.y0), pt(r.x1, r.y0), pt(r.x1, cy), pt(r.x1, r.y1), pt(cx, r.y1), pt(r.x0, r.y1), pt(r.x0, cy)]
    }

    fn rotate_screen(&self, view: &View) -> Pos2 {
        let r = self.sel_rect;
        view.to_screen(pt((r.x0 + r.x1) / 2.0, r.y1)) - egui::vec2(0.0, ROT_PX)
    }

    /// The frame part under a screen position, if any (checked before shapes so handles win).
    pub(crate) fn hit_handle(&self, s: Pos2) -> Option<Handle> {
        if !self.frame_visible() {
            return None;
        }
        let view = self.view.as_ref().unwrap();
        let tol = (HIT_PX * self.hit_scale) as f32 + 1.0;
        if view.to_screen(self.pivot()).distance(s) <= tol {
            return Some(PIVOT);
        }
        if self.rotate_screen(view).distance(s) <= tol + 1.0 {
            return Some(ROTATE);
        }
        let hp = self.handle_points();
        // tiny selections: only the corners (edge handles would sit on top of them)
        let small = { let a = view.to_screen(hp[0]); let b = view.to_screen(hp[4]); (a.x - b.x).abs() < 24.0 || (a.y - b.y).abs() < 24.0 };
        (0..8u8).filter(|h| !small || h % 2 == 0).find(|&h| {
            let p = view.to_screen(hp[h as usize]);
            (p.x - s.x).abs() <= tol && (p.y - s.y).abs() <= tol
        })
    }

    pub(crate) fn handle_cursor(h: Handle) -> egui::CursorIcon {
        use egui::CursorIcon::*;
        match h {
            0 | 4 => ResizeNeSw,
            2 | 6 => ResizeNwSe,
            1 | 5 => ResizeVertical,
            3 | 7 => ResizeHorizontal,
            ROTATE => Alias,
            _ => Move,
        }
    }

    /// Press on a handle: remember the shapes as they are (each drag frame transforms these originals).
    pub(crate) fn start_handle_drag(&mut self, h: Handle, press_w: Pt) {
        self.press_w = press_w;
        if h == PIVOT {
            self.drag = Drag::Pivot;
            return;
        }
        self.doc.checkpoint();
        self.break_constraints_on_move();
        self.orig = self.sel.iter().filter_map(|i| self.doc.find(*i).cloned()).collect();
        self.move_rect = self.sel_rect;
        self.xf_pivot = self.pivot();
        self.xf_angle = 0.0;
        self.xf_last = Affine::default();
        self.drag = if h == ROTATE { Drag::Rotate } else { Drag::Scale(h) };
    }

    /// One drag frame. Returns the status text (angle or scale).
    pub(crate) fn apply_handle_drag(&mut self, w: Pt, shift: bool) -> String {
        let m = match self.drag {
            Drag::Pivot => {
                self.pivot = Some((self.snapped(w), self.sel.clone()));
                return "Pivot: rotation centre".into();
            }
            Drag::Rotate => {
                let c = self.xf_pivot;
                let a0 = (self.press_w.y - c.y).atan2(self.press_w.x - c.x);
                let mut a = (w.y - c.y).atan2(w.x - c.x) - a0;
                let step = if shift { 15f64.to_radians() } else { 0.1f64.to_radians() };
                a = (a / step).round() * step;
                self.xf_angle = a;
                Affine::translate(c.x, c.y) * Affine::rotate(a) * Affine::translate(-c.x, -c.y)
            }
            Drag::Scale(h) => {
                let r = self.move_rect;
                // the anchor = the opposite handle; edges only change their own axis
                let ax = if matches!(h, 0 | 6 | 7) { r.x1 } else { r.x0 };
                let ay = if matches!(h, 0..=2) { r.y1 } else { r.y0 };
                let (w0, h0) = (r.w().max(1e-9), r.h().max(1e-9));
                let mut sx = if matches!(h, 1 | 5) { 1.0 } else { (w.x - ax).abs() / w0 };
                let mut sy = if matches!(h, 3 | 7) { 1.0 } else { (w.y - ay).abs() / h0 };
                if h % 2 == 0 && !shift {
                    let s = sx.max(sy); // corners keep proportions
                    (sx, sy) = (s, s);
                }
                (sx, sy) = (sx.max(1e-3), sy.max(1e-3));
                self.xf_angle = 0.0;
                self.xf_scale = (sx, sy);
                Affine::translate(ax, ay) * Affine::scale(sx, sy) * Affine::translate(-ax, -ay)
            }
            _ => return String::new(),
        };
        for o in &self.orig {
            if let Some(s) = self.doc.find_mut(o.id) {
                s.c = transformed(&o.c, &m);
            }
        }
        self.xf_last = m;
        self.mutated();
        match self.drag {
            Drag::Rotate => format!("Rotate {:.1}°  (Shift: 15° steps)", self.xf_angle.to_degrees()),
            _ => {
                let r = self.sel_rect;
                format!("W {}  H {}  ({:.0}% × {:.0}%; corners keep proportions, Shift = free)", crate::units::text(&self.doc, r.w()), crate::units::text(&self.doc, r.h()), self.xf_scale.0 * 100.0, self.xf_scale.1 * 100.0)
            }
        }
    }

    /// Rotates the selection by `deg` (counter-clockwise) around the pivot: the inspector's exact-angle field.
    pub fn rotate_selection(&mut self, deg: f64) {
        let c = self.pivot();
        let pivot = self.pivot.clone();
        self.transform_selection(&(Affine::translate(c.x, c.y) * Affine::rotate(deg.to_radians()) * Affine::translate(-c.x, -c.y)));
        self.pivot = pivot; // the selection is the same: keep the user's pivot
    }

    pub(crate) fn paint_transform_frame(&self, painter: &egui::Painter) {
        if !self.frame_visible() {
            return;
        }
        let view = self.view.as_ref().unwrap();
        let stroke = Stroke::new(1.0, FRAME);
        if self.drag == Drag::Rotate {
            // the original box turning with the shapes (the axis-aligned bounds would wobble)
            let r = self.move_rect;
            let c = self.xf_pivot;
            let m = Affine::translate(c.x, c.y) * Affine::rotate(self.xf_angle) * Affine::translate(-c.x, -c.y);
            let pts: Vec<Pos2> = [pt(r.x0, r.y0), pt(r.x1, r.y0), pt(r.x1, r.y1), pt(r.x0, r.y1)].iter().map(|p| view.to_screen(m.apply(*p))).collect();
            painter.extend(egui::Shape::dashed_line(&[pts.clone(), vec![pts[0]]].concat(), stroke, 5.0, 3.0));
            let pc = view.to_screen(c);
            painter.line_segment([pc, view.to_screen(m.apply(pt((r.x0 + r.x1) / 2.0, r.y1)))], stroke);
        } else {
            let hp = self.handle_points();
            let corners: Vec<Pos2> = [hp[0], hp[2], hp[4], hp[6], hp[0]].iter().map(|p| view.to_screen(*p)).collect();
            painter.extend(egui::Shape::dashed_line(&corners, stroke, 5.0, 3.0));
            let small = (corners[0].x - corners[2].x).abs() < 24.0 || (corners[0].y - corners[2].y).abs() < 24.0;
            for (i, p) in hp.iter().enumerate() {
                if small && i % 2 == 1 {
                    continue;
                }
                let s = view.to_screen(*p);
                let r = egui::Rect::from_center_size(s, egui::vec2(8.0, 8.0));
                painter.rect(r, 1.0, theme::CANVAS_BG, stroke, egui::StrokeKind::Middle);
            }
            let top = view.to_screen(hp[5]);
            let rot = self.rotate_screen(view);
            painter.line_segment([top, rot], stroke);
            painter.circle(rot, 5.0, FRAME, Stroke::new(1.5, theme::CANVAS_BG));
        }
        // pivot: a small crosshair in a circle
        let p = view.to_screen(if self.drag == Drag::Rotate { self.xf_pivot } else { self.pivot() });
        painter.circle_stroke(p, 5.0, stroke);
        painter.line_segment([p - egui::vec2(8.0, 0.0), p + egui::vec2(8.0, 0.0)], stroke);
        painter.line_segment([p - egui::vec2(0.0, 8.0), p + egui::vec2(0.0, 8.0)], stroke);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::geom::{bounds, make_rect};

    fn canvas_with_square() -> Canvas {
        let mut c = Canvas::default();
        let id = c.doc.add(make_rect(0.0, 0.0, 2.0, 1.0), None);
        c.tool = crate::canvas::Tool::Select;
        c.sel.insert(id);
        c.sel_changed();
        c.view = Some(View { zoom: 100.0, origin: Pos2::new(0.0, 500.0) });
        c
    }

    #[test]
    fn corner_scale_keeps_proportions_shift_frees() {
        let mut c = canvas_with_square();
        // drag the top-right corner (handle 4) from (2,1) to (4,1.2): proportional = x2 from the bottom-left
        c.start_handle_drag(4, pt(2.0, 1.0));
        c.apply_handle_drag(pt(4.0, 1.2), false);
        let b = bounds(&c.doc.shapes[0].c);
        assert!((b.x0, b.y0) == (0.0, 0.0) && (b.w() - 4.0).abs() < 1e-9 && (b.h() - 2.0).abs() < 1e-9, "{b:?}");
        c.apply_handle_drag(pt(4.0, 1.2), true);
        let b = bounds(&c.doc.shapes[0].c);
        assert!((b.w() - 4.0).abs() < 1e-9 && (b.h() - 1.2).abs() < 1e-9, "{b:?}");
        // edge handle 7 (left): only X, the right side stays
        c.drag = Drag::None;
        c.sel_changed();
        c.start_handle_drag(7, pt(0.0, 0.6));
        c.apply_handle_drag(pt(-4.0, 9.0), false);
        let b = bounds(&c.doc.shapes[0].c);
        assert!((b.x1 - 4.0).abs() < 1e-9 && (b.x0 + 4.0).abs() < 1e-9 && (b.h() - 1.2).abs() < 1e-9, "{b:?}");
        assert!(c.doc.undo());
    }

    #[test]
    fn rotate_snaps_and_pivot_moves() {
        let mut c = canvas_with_square();
        assert_eq!(c.pivot(), pt(1.0, 0.5));
        // rotate a quarter turn around the centre with Shift (15° steps): 2 x 1 becomes 1 x 2
        c.start_handle_drag(ROTATE, pt(1.0, 2.0));
        let msg = c.apply_handle_drag(pt(-0.03, 0.52), true);
        assert!(msg.starts_with("Rotate 90.0"), "{msg}");
        let b = bounds(&c.doc.shapes[0].c);
        assert!((b.w() - 1.0).abs() < 1e-6 && (b.h() - 2.0).abs() < 1e-6, "{b:?}");
        c.drag = Drag::None;
        // pivot dragged to the origin; exact-angle rotate turns around it
        c.start_handle_drag(PIVOT, c.pivot());
        c.apply_handle_drag(pt(0.0, 0.0), false);
        c.drag = Drag::None;
        assert_eq!(c.pivot(), pt(0.0, 0.0));
        c.rotate_selection(180.0);
        let b = bounds(&c.doc.shapes[0].c);
        assert!((b.x1 + 0.5).abs() < 1e-6 && (b.y1 - 0.5).abs() < 1e-6, "{b:?}");
        assert_eq!(c.pivot(), pt(0.0, 0.0), "pivot kept for the same selection");
    }
}
