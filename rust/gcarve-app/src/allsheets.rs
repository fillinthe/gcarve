//! All sheets on one screen (View › All Sheets, the button by the sheet tabs; owner 2026-10-19): the boards side by
//! side, left to right in sheet order. Shapes keep their own sheet's coordinates, so nothing in the document moves:
//! the active sheet is drawn where it is and edited as always, every other sheet is drawn shifted by `sheet_rel`
//! (fainter, not editable). A click on another sheet makes it the active one without the view jumping, and a
//! selection dropped on another sheet's board moves to that sheet (its toolpaths go with it).

use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use gcarve_core::geom::{self, pt, Pt};

use crate::canvas::{hex, stroke_path, Canvas};
use crate::theme;

impl Canvas {
    /// all sheets shown (and more than one sheet to show)
    pub fn showing_all_sheets(&self) -> bool {
        self.all_sheets && self.doc.sheets.len() > 1
    }

    /// board size of sheet `j` (the active one's lives in `doc.material` while it is edited)
    fn sheet_size(&self, j: usize) -> (f64, f64) {
        let m = if j as i32 == self.doc.active_sheet { &self.doc.material } else { &self.doc.sheets[j] };
        (m.w, m.h)
    }

    /// Where sheet `j`'s board corner sits, in the active sheet's coordinates: a grid of equal cells (the largest
    /// board plus a gap of a tenth of it), in reading order, with as many columns as suit a wide window.
    pub fn sheet_rel(&self, j: usize) -> Pt {
        let (cols, cw, ch) = self.sheet_grid();
        let at = |k: usize| pt((k % cols) as f64 * cw, -((k / cols) as f64) * ch);
        let a = (self.doc.active_sheet.max(0) as usize).min(self.doc.sheets.len().saturating_sub(1));
        let (p, q) = (at(j), at(a));
        pt(p.x - q.x, p.y - q.y)
    }

    /// (columns, cell width, cell height) of the sheet grid
    fn sheet_grid(&self) -> (usize, f64, f64) {
        let n = self.doc.sheets.len().max(1);
        let (mw, mh) = (0..n).map(|k| self.sheet_size(k)).fold((1.0f64, 1.0f64), |a, b| (a.0.max(b.0), a.1.max(b.1)));
        let gap = mw.max(mh).max(20.0) * 0.1;
        let (cw, ch) = (mw + gap, mh + gap + gap * 0.5); // room for the name above each board
        // the column count whose whole grid is closest to a 16:10 window
        let cols = (1..=n).min_by(|&a, &b| {
            let aspect = |c: usize| (c as f64 * cw) / (n.div_ceil(c) as f64 * ch);
            (aspect(a).ln() - 1.6f64.ln()).abs().total_cmp(&(aspect(b).ln() - 1.6f64.ln()).abs())
        }).unwrap_or(n);
        (cols, cw, ch)
    }

    /// the other sheet whose board is under `w` (active sheet coordinates)
    pub fn other_sheet_at(&self, w: Pt) -> Option<usize> {
        if !self.showing_all_sheets() {
            return None;
        }
        (0..self.doc.sheets.len()).filter(|&j| j as i32 != self.doc.active_sheet).find(|&j| {
            let (o, (bw, bh)) = (self.sheet_rel(j), self.sheet_size(j));
            w.x >= o.x && w.x <= o.x + bw && w.y >= o.y && w.y <= o.y + bh
        })
    }

    /// Makes sheet `j` the active one; the view shifts with it so everything stays where it is on screen. Returns
    /// what to add to a point to have it in the new sheet's coordinates.
    pub fn switch_sheet_keep_view(&mut self, j: usize) -> Pt {
        let r = self.sheet_rel(j);
        if let Some(v) = &mut self.view {
            v.origin += egui::vec2(r.x as f32 * v.zoom, -(r.y as f32) * v.zoom);
        }
        self.doc.set_active_sheet(j as i32);
        self.sel.clear();
        self.sel_changed();
        self.apply_bindings(); // thickness / board-w are this sheet's
        self.mutated();
        pt(-r.x, -r.y)
    }

    /// End of a move drag: a selection whose centre is over another sheet's board goes to that sheet (shifted into
    /// its coordinates), and that sheet becomes the active one. Returns the sheet it went to.
    pub fn drop_on_other_sheet(&mut self) -> Option<usize> {
        if self.sel.is_empty() {
            return None;
        }
        let r = self.sel_rect;
        let j = self.other_sheet_at(pt((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0))?;
        let o = self.sheet_rel(j);
        let shift = geom::Affine::translate(-o.x, -o.y);
        let ids: Vec<i32> = self.sel.iter().copied().collect();
        for id in &ids {
            if let Some(s) = self.doc.find_mut(*id) {
                s.c = geom::transformed(&s.c, &shift);
                s.sheet = j as i32;
            }
        }
        self.switch_sheet_keep_view(j);
        self.sel = ids.into_iter().collect();
        self.sel_changed();
        self.mutated();
        Some(j)
    }

    /// A back side (Material::back_of): the front sheet's vectors, mirrored the way the board is flipped, as faint
    /// dashed lines under this sheet's own, so holes and cut-outs line up from both faces.
    pub fn paint_front_side(&self, painter: &egui::Painter) {
        let Some((front, lr)) = self.doc.material.back_of else { return };
        let Some(view) = &self.view else { return };
        if self.showing_all_sheets() || front < 0 || front as usize >= self.doc.sheets.len() || front == self.doc.active_sheet {
            return;
        }
        let (w, h) = (self.doc.material.w, self.doc.material.h);
        let flip = if lr { geom::Affine { a: -1.0, d: 1.0, e: w, ..Default::default() } } else { geom::Affine { a: 1.0, d: -1.0, f: h, ..Default::default() } };
        let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 170, 230, 110));
        for s in self.doc.shapes.iter().filter(|s| s.sheet == front) {
            let pts = self.screen_pts(view, &geom::transformed(&s.c, &flip));
            for w in pts.windows(2) {
                painter.add(egui::Shape::dashed_line(&[w[0], w[1]], stroke, 4.0, 4.0));
            }
        }
    }

    /// The other sheets' boards, names and shapes (fainter), the active sheet's name and the drop target while a
    /// selection is dragged over another board. Drawn under the active sheet.
    pub fn paint_other_sheets(&self, painter: &egui::Painter, dragging: bool, hover: Option<Pt>) {
        if !self.showing_all_sheets() {
            return;
        }
        let Some(view) = &self.view else { return };
        let r = self.sel_rect;
        let target = if dragging && !self.sel.is_empty() { self.other_sheet_at(pt((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0)) } else { None };
        let hovered = if dragging { None } else { hover.and_then(|w| self.other_sheet_at(w)) };
        // the longest form of the name that fits over its board: full, then shorter
        let label = |at: Pos2, width: f32, forms: &[String], col: Color32| {
            let font = theme::font(12.0);
            let fits = forms.iter().find(|t| painter.layout_no_wrap((*t).clone(), font.clone(), col).size().x <= width).or(forms.last());
            if let Some(t) = fits {
                painter.text(at + egui::vec2(0.0, -6.0), egui::Align2::LEFT_BOTTOM, t, font, col);
            }
        };
        for j in 0..self.doc.sheets.len() {
            let (o, (bw, bh)) = (self.sheet_rel(j), self.sheet_size(j));
            let board = Rect::from_two_pos(view.to_screen(o), view.to_screen(pt(o.x + bw, o.y + bh)));
            let count = self.doc.shapes.iter().filter(|s| s.sheet as usize == j).count();
            let name = format!("{}  ·  {} × {}  ·  {count} vector{}", self.doc.sheet_name(j), crate::units::text(&self.doc, bw), crate::units::text(&self.doc, bh), if count == 1 { "" } else { "s" });
            let short = format!("{}  ·  {count}", self.doc.sheet_name(j));
            let tiny = format!("{}", j + 1);
            if j as i32 == self.doc.active_sheet {
                // the sheet being worked on: its name in the accent colour and a bar over the board
                painter.rect_filled(Rect::from_min_size(board.left_top() - egui::vec2(0.0, 3.0), egui::vec2(board.width(), 3.0)), 1.0, theme::ACCENT);
                label(board.left_top() - egui::vec2(0.0, 3.0), board.width(), &[format!("{name}  ·  editing"), name, short, tiny], theme::ACCENT);
                continue; // the active board and its shapes are drawn by `ui` as always
            }
            let (hot, over) = (target == Some(j), hovered == Some(j));
            let (fill, stroke) = if hot {
                (60, Stroke::new(2.0, theme::ACCENT))
            } else if over {
                (34, Stroke::new(1.5, theme::TEXT_DIM))
            } else {
                (16, Stroke::new(1.0, Color32::from_rgb(0x6a, 0x5f, 0x48)))
            };
            painter.rect(board, 0.0, Color32::from_rgba_unmultiplied(138, 122, 90, fill), stroke, egui::StrokeKind::Middle);
            let forms = if hot {
                vec![format!("Drop to move to {}", self.doc.sheet_name(j)), format!("→ {}", j + 1), tiny]
            } else if over {
                vec![format!("{name}  ·  click to work on it"), format!("{short}  ·  click to edit"), short, tiny]
            } else {
                vec![name, short, tiny]
            };
            label(board.left_top(), board.width(), &forms, if hot { theme::ACCENT } else if over { theme::TEXT } else { theme::TEXT_DIM });
            for s in self.doc.shapes.iter().filter(|s| s.sheet as usize == j && self.doc.layers.get(s.layer as usize).is_none_or(|l| l.visible)) {
                let pts: Vec<Pos2> = geom::flatten(&s.c, 0.5 / view.zoom as f64).into_iter().map(|p| view.to_screen(pt(p.x + o.x, p.y + o.y))).collect();
                let col = self.doc.layers.get(s.layer as usize).map(|l| hex(&l.color)).unwrap_or(Color32::from_rgb(0xd7, 0xda, 0xe0));
                stroke_path(painter, pts, s.c.closed, Stroke::new(1.0, col.gamma_multiply(if over { 0.8 } else { 0.55 })));
            }
        }
    }

    /// the boards of every sheet, for Fit in the all-sheets view (active sheet coordinates)
    pub fn all_boards(&self) -> Option<geom::Rect> {
        if !self.showing_all_sheets() {
            return None;
        }
        let mut r: Option<geom::Rect> = None;
        for j in 0..self.doc.sheets.len() {
            let (o, (w, h)) = (self.sheet_rel(j), self.sheet_size(j));
            let b = geom::Rect { x0: o.x, y0: o.y, x1: o.x + w, y1: o.y + h };
            r = Some(r.map_or(b, |a| geom::Rect { x0: a.x0.min(b.x0), y0: a.y0.min(b.y0), x1: a.x1.max(b.x1), y1: a.y1.max(b.y1) }));
        }
        r
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_back_side_remembers_its_front_and_survives_save_and_removal() {
        let mut c = Canvas::default();
        c.doc.material.w = 10.0;
        c.doc.add(gcarve_core::geom::make_polyline(&[gcarve_core::geom::pt(4.0, 0.5), gcarve_core::geom::pt(6.5, 0.5), gcarve_core::geom::pt(4.0, 2.5)], true), None); // a right triangle across the middle: its mirror on the back points the other way
        c.doc.sync_sheet();
        let mut back = gcarve_core::document::Material { back_of: Some((0, true)), ..c.doc.material.clone() };
        back.name = "Back".into();
        c.doc.sheets.push(back);
        let text = gcarve_core::project::save_project(&c.doc);
        let d = gcarve_core::project::load_project(&text).unwrap();
        assert_eq!(d.sheets[1].back_of, Some((0, true)));
        assert_eq!(d.sheets[0].back_of, None);
        if let Ok(out) = std::env::var("GCARVE_BACKSIDE_OUT") {
            std::fs::write(out, text).unwrap(); // a two-sided project for a screenshot (GCARVE_SHEET=1)
        }
    }

    use super::*;
    use gcarve_core::geom::{bounds, make_rect};
    use std::collections::BTreeSet;

    #[test]
    fn a_selection_dropped_on_another_sheet_moves_there_in_its_coordinates() {
        let mut c = Canvas::default();
        c.doc.sheets = vec![c.doc.material.clone(), c.doc.material.clone()];
        c.all_sheets = true;
        let id = c.doc.add(make_rect(1.0, 1.0, 3.0, 2.0), None);
        // dragged right over sheet 2's board (drawn at sheet_rel(1) in sheet 1's coordinates)
        let o = c.sheet_rel(1);
        assert!(o.x >= c.doc.material.w || -o.y >= c.doc.material.h, "sheet 2 sits beside or below sheet 1, clear of it: {o:?}");
        let moved = geom::transformed(&c.doc.find(id).unwrap().c, &geom::Affine::translate(o.x, o.y));
        c.doc.find_mut(id).unwrap().c = moved;
        c.sel = BTreeSet::from([id]);
        c.sel_changed();
        assert_eq!(c.drop_on_other_sheet(), Some(1));
        let s = c.doc.find(id).unwrap();
        let b = bounds(&s.c);
        assert_eq!((s.sheet, c.doc.active_sheet), (1, 1), "on sheet 2, now the active one");
        assert!((b.x0 - 1.0).abs() < 1e-9 && (b.y0 - 1.0).abs() < 1e-9, "back in board coordinates: {b:?}");
        assert!(c.sel.contains(&id), "still selected");
        // a press on sheet 1's board from sheet 2 switches back; points convert by the offset
        let back = c.sheet_rel(0);
        assert_eq!(c.other_sheet_at(pt(back.x + 2.0, back.y + 2.0)), Some(0));
    }
}
