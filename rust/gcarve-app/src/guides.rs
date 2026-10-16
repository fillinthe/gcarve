//! Guide lines: drag out of the top ruler for a horizontal guide, out of the left ruler for a vertical one. Each
//! guide shows a marker in the ruler that measures it; drag the marker to move the guide, drop it back on a ruler
//! to remove it. Drawing and moving snap to guides (always: that is what they are for). Saved with the project.

use crate::canvas::Canvas;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke};
use gcarve_core::geom::{pt, Pt};

const GRAB_PX: f32 = 6.0;
const COLOR: Color32 = Color32::from_rgb(0x2f, 0xd0, 0xc8);

impl Canvas {
    /// p with each coordinate pulled onto a guide within `r` inches; true when one took it
    pub(crate) fn guide_snap(&self, p: Pt, r: f64) -> (Pt, bool) {
        let mut q = p;
        for &(vertical, at) in &self.doc.guides {
            if vertical && (p.x - at).abs() <= r && (q.x == p.x || (p.x - at).abs() < (p.x - q.x).abs()) {
                q.x = at;
            } else if !vertical && (p.y - at).abs() <= r && (q.y == p.y || (p.y - at).abs() < (p.y - q.y).abs()) {
                q.y = at;
            }
        }
        (q, q.x != p.x || q.y != p.y)
    }

    /// Ruler strips: start, move and drop guides; paint them over the canvas. `full` = canvas with the rulers.
    pub(crate) fn guides_ui(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, full: Rect) {
        let r = self.ruler_px();
        if r <= 0.0 || self.view.is_none() {
            return;
        }
        let area = self.view_area(full);
        let top = Rect::from_min_max(egui::pos2(area.left(), full.top()), egui::pos2(full.right(), area.top()));
        let left = Rect::from_min_max(egui::pos2(full.left(), area.top()), egui::pos2(area.left(), full.bottom()));
        let screen = |c: &Canvas, vertical: bool, at: f64| { let s = c.view.as_ref().unwrap().to_screen(pt(at, at)); if vertical { s.x } else { s.y } };
        for (strip, id, along_x) in [(top, "guide-top", true), (left, "guide-left", false)] {
            let resp = ui.interact(strip, ui.id().with(id), Sense::drag()).on_hover_text(if along_x { "Drag down for a horizontal guide" } else { "Drag right for a vertical guide" });
            let Some(pos) = resp.interact_pointer_pos() else { continue };
            if resp.drag_started() {
                // on a marker: move that guide (the top ruler carries the vertical guides' markers); else a new one
                let coord = if along_x { pos.x } else { pos.y };
                let hit = self.doc.guides.iter().position(|&(v, at)| v == along_x && (screen(self, v, at) - coord).abs() <= GRAB_PX);
                self.guide_drag = Some(hit.unwrap_or_else(|| {
                    self.doc.guides.push((!along_x, 0.0));
                    self.doc.guides.len() - 1
                }));
            }
            if let Some(i) = self.guide_drag.filter(|&i| i < self.doc.guides.len()) {
                let vertical = self.doc.guides[i].0;
                if resp.dragged() {
                    let d = self.view.as_ref().unwrap().to_doc(pos);
                    // lands on round ruler values with Snap on (the grid step), exactly under the pointer without
                    let g = self.grid_step();
                    let v = if vertical { d.x } else { d.y };
                    self.doc.guides[i].1 = if self.snap { (v / g).round() * g } else { v };
                    ui.ctx().set_cursor_icon(if vertical { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
                }
                if resp.drag_stopped() {
                    if !area.contains(pos) {
                        self.doc.guides.remove(i); // dropped on a ruler (or outside): gone
                    }
                    self.guide_drag = None;
                    self.touch();
                }
            }
        }
        let stroke = Stroke::new(1.0, COLOR.gamma_multiply(0.8));
        for &(vertical, at) in &self.doc.guides {
            let s = screen(self, vertical, at);
            if vertical && s >= area.left() && s <= area.right() {
                painter.vline(s, area.top()..=area.bottom(), stroke);
            } else if !vertical && s >= area.top() && s <= area.bottom() {
                painter.hline(area.left()..=area.right(), s, stroke);
            }
        }
    }

    /// markers in the rulers, painted after them
    pub(crate) fn paint_guide_markers(&self, painter: &egui::Painter, full: Rect) {
        let r = self.ruler_px();
        if r <= 0.0 || self.view.is_none() {
            return;
        }
        let area = self.view_area(full);
        let view = self.view.as_ref().unwrap();
        for &(vertical, at) in &self.doc.guides {
            let s = view.to_screen(pt(at, at));
            let tri = |a: Pos2, b: Pos2, c: Pos2| painter.add(egui::Shape::convex_polygon(vec![a, b, c], COLOR, Stroke::NONE));
            if vertical && s.x >= area.left() {
                tri(egui::pos2(s.x - 5.0, area.top() - 7.0), egui::pos2(s.x + 5.0, area.top() - 7.0), egui::pos2(s.x, area.top()));
            } else if !vertical && s.y >= area.top() {
                tri(egui::pos2(area.left() - 7.0, s.y - 5.0), egui::pos2(area.left() - 7.0, s.y + 5.0), egui::pos2(area.left(), s.y));
            }
        }
    }
}
