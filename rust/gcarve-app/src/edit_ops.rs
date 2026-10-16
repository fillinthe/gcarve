//! Document edits on the selection (Qt app: canvas.cpp + canvas_vectors.cpp): mutation bookkeeping, groups,
//! clipboard, delete/duplicate, transforms, booleans, offset, dogbone, join/close, align/distribute, arrays, nesting.

use crate::canvas::{Canvas, Drag};
use crate::units;
use eframe::egui;
use gcarve_core::document::Shape;
use gcarve_core::geom::{bounds, flatten, pt, transformed, Affine, Contour, Pt, Rect};
use gcarve_core::nest::{nest_shapes, NestPart};
use gcarve_core::project::{load_clip, save_clip};
use gcarve_core::vecops::{boolean_op, close_contour, dogbone, fillet_contours, join_open, offset_contours, simplify_contours, BoolOp};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

/// A selection unit: a group's members move as one (Qt Canvas::Unit)
pub struct Unit {
    pub ids: Vec<i32>,
    pub b: Rect,
}

fn contour_area(c: &Contour) -> f64 {
    let p = flatten(c, gcarve_core::geom::FLATTEN_TOL);
    let mut a = 0.0;
    for i in 0..p.len() {
        let j = (i + p.len() - 1) % p.len();
        a += p[j].x * p[i].y - p[i].x * p[j].y;
    }
    a.abs() / 2.0
}

impl Canvas {
    /// After every document change (Qt Canvas::mutated): constraints, orphans, dimensions, selection validity.
    /// Live drags skip the regenerate (set `gen_stale`); the release calls `touch()`.
    pub fn mutated(&mut self) {
        self.dirty = true;
        self.doc.sync_sheet();
        let ids: BTreeSet<i32> = self.doc.shapes.iter().map(|s| s.id).collect();
        self.doc.constraints.retain(|c| ids.contains(&c.anchor) && ids.contains(&c.target) && c.anchor != c.target);
        self.doc.apply_constraints();
        let texts: BTreeSet<i32> = self.doc.shapes.iter().map(|s| s.text).collect();
        self.doc.texts.retain(|t| texts.contains(&t.id));
        self.doc.refresh_dimensions();
        self.doc.apply_driven_dimensions();
        let nl = self.doc.layers.len() as i32 - 1;
        for s in &mut self.doc.shapes {
            s.layer = s.layer.clamp(0, nl);
        }
        self.doc.active_layer = self.doc.active_layer.clamp(0, nl);
        self.sel.retain(|id| self.doc.find(*id).is_some_and(|s| self.doc.shape_editable(s)));
        if self.doc.find(self.node_shape).is_none_or(|s| self.node_idx >= s.c.nodes.len() as i32) {
            self.node_shape = -1;
            self.node_idx = -1;
        }
        if self.sel_dim >= self.doc.dimensions.len() as i32 {
            self.sel_dim = -1;
        }
        self.sel_changed();
        if matches!(self.drag, Drag::Move | Drag::Node | Drag::Scale(_) | Drag::Rotate) {
            self.gen_stale = true;
        } else {
            self.touch();
        }
    }

    /// Drag-box selection from `a` to `b` (document points): dragged to the right = shapes fully inside the box
    /// (window), to the left = anything the box touches (crossing). Adds to the selection.
    pub(crate) fn marquee_select(&mut self, a: Pt, b: Pt) {
        let mut dr = Rect::default();
        dr.add(a);
        dr.add(b);
        let window = b.x >= a.x;
        for s in self.doc.shapes.iter().filter(|s| self.doc.shape_editable(s)) {
            let sb = bounds(&s.c);
            if if window { dr.contains(&sb) } else { dr.intersects(&sb) } {
                self.sel.insert(s.id);
            }
        }
        self.sel_changed();
    }

    /// Selection bounds + whole groups.
    pub fn sel_changed(&mut self) {
        self.expand_groups();
        self.sel_rect = Rect::default();
        for id in &self.sel {
            if let Some(s) = self.doc.find(*id) {
                self.sel_rect.add_rect(&bounds(&s.c));
            }
        }
    }

    fn expand_groups(&mut self) {
        let groups: BTreeSet<i32> = self.sel.iter().filter_map(|id| self.doc.find(*id)).map(|s| s.group).filter(|g| *g != 0).collect();
        if groups.is_empty() {
            return;
        }
        for s in &self.doc.shapes {
            if groups.contains(&s.group) && self.doc.shape_editable(s) {
                self.sel.insert(s.id);
            }
        }
    }

    pub fn selection_units(&self) -> Vec<Unit> {
        let mut units: Vec<Unit> = Vec::new();
        let mut by_group: BTreeMap<i32, usize> = BTreeMap::new();
        for id in &self.sel {
            let Some(s) = self.doc.find(*id) else { continue };
            let b = bounds(&s.c);
            if s.group != 0 {
                match by_group.get(&s.group) {
                    Some(&k) => {
                        units[k].ids.push(*id);
                        units[k].b.add_rect(&b);
                    }
                    None => {
                        by_group.insert(s.group, units.len());
                        units.push(Unit { ids: vec![*id], b });
                    }
                }
            } else {
                units.push(Unit { ids: vec![*id], b });
            }
        }
        units
    }

    pub fn group_selection(&mut self) {
        if self.sel.len() < 2 {
            self.status = "Select two or more shapes to group.".into();
            return;
        }
        self.doc.checkpoint();
        let g = self.doc.new_group_id();
        for id in self.sel.clone() {
            if let Some(s) = self.doc.find_mut(id) {
                s.group = g;
            }
        }
        self.status = format!("Grouped {} shapes", self.sel.len());
        self.mutated();
    }

    pub fn ungroup_selection(&mut self) {
        if !self.selection_has_group() {
            return;
        }
        self.doc.checkpoint();
        for id in self.sel.clone() {
            if let Some(s) = self.doc.find_mut(id) {
                s.group = 0;
            }
        }
        self.status = "Ungrouped".into();
        self.mutated();
    }

    /// A target moved on its own (its anchor not moving too) is no longer constrained.
    pub(crate) fn break_constraints_on_move(&mut self) {
        let before = self.doc.constraints.len();
        let sel = &self.sel;
        self.doc.constraints.retain(|c| !(sel.contains(&c.target) && !sel.contains(&c.anchor)));
        if self.doc.constraints.len() != before {
            self.status = format!("Constraint released: shape moved by hand ({})", before - self.doc.constraints.len());
        }
        for d in &mut self.doc.dimensions {
            if !d.drive.is_empty() && d.shape_a != d.shape_b && sel.contains(&d.shape_b) && !sel.contains(&d.shape_a) {
                d.drive.clear();
                self.status = "Driving dimension released: shape moved by hand".into();
            }
        }
    }

    pub fn transform_selection(&mut self, m: &Affine) {
        if self.sel.is_empty() {
            return;
        }
        self.doc.checkpoint();
        self.break_constraints_on_move();
        for id in self.sel.clone() {
            if let Some(s) = self.doc.find_mut(id) {
                s.c = transformed(&s.c, m);
            }
        }
        self.texts_follow(m);
        self.mutated();
    }

    pub fn delete_selected(&mut self) {
        if self.sel_dim >= 0 {
            let i = self.sel_dim as usize;
            self.remove_dimension(i);
            return;
        }
        if self.tool == crate::canvas::Tool::Nodes && self.node_idx >= 0 {
            let (shape, node) = (self.node_shape, self.node_idx);
            self.remove_node(shape, node);
            return;
        }
        if self.sel.is_empty() {
            return;
        }
        self.doc.checkpoint();
        let sel = std::mem::take(&mut self.sel);
        self.doc.shapes.retain(|s| !sel.contains(&s.id));
        // toolpaths follow their shapes: drop the deleted ids, and drop toolpaths left with no shapes
        for op in &mut self.doc.ops {
            op.shape_ids.retain(|id| !sel.contains(id));
        }
        self.doc.ops.retain(|op| !(op.shape_ids.is_empty() && op.layer_scope < 0 && op.group_scope == 0));
        self.mutated();
    }

    pub fn remove_dimension(&mut self, i: usize) {
        if i >= self.doc.dimensions.len() {
            return;
        }
        self.doc.checkpoint();
        self.doc.dimensions.remove(i);
        self.sel_dim = -1;
        self.mutated();
    }

    /// Dimension popup: label ("{v}" inserts the value) and a driving length expression.
    pub fn set_dimension(&mut self, i: usize, label: &str, drive: &str) {
        if i >= self.doc.dimensions.len() {
            return;
        }
        let dr = drive.trim();
        if !dr.is_empty() {
            if let Err(e) = units::parse(&self.doc, dr) {
                self.status = format!("Can't evaluate \"{dr}\": {e}");
                return;
            }
        }
        self.doc.checkpoint();
        let d = &mut self.doc.dimensions[i];
        d.label = label.trim().to_string();
        d.drive = dr.to_string();
        if !dr.is_empty() && (d.shape_a < 0 || d.shape_b < 0) {
            self.status = "Both ends must snap to shape nodes for a dimension to drive geometry.".into();
        }
        self.mutated();
    }

    pub fn duplicate(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.doc.checkpoint();
        let layer = self.selected().and_then(|id| self.doc.find(id)).map(|s| s.layer).unwrap_or(self.doc.active_layer);
        let clip = save_clip(&self.doc, &self.sel);
        let keep = self.doc.active_layer;
        self.doc.active_layer = layer;
        let off = 12.0 / self.zoom();
        self.sel = load_clip(&clip, &mut self.doc, pt(off, -off));
        self.doc.active_layer = keep;
        self.mutated();
    }

    /// Copies as GCarve clip JSON: kept internally and put on the OS clipboard (Qt: application/x-gcarve-clip + text).
    pub fn copy(&mut self, ctx: &egui::Context) {
        if self.sel.is_empty() {
            return;
        }
        self.clip = save_clip(&self.doc, &self.sel);
        ctx.copy_text(self.clip.clone());
        let ops = self.doc.ops.iter().filter(|op| !op.shape_ids.is_empty() && op.shape_ids.iter().all(|id| self.sel.contains(id))).count();
        self.status = format!("Copied {} shape(s){}", self.sel.len(), if ops > 0 { format!(" with {ops} toolpath(s)") } else { String::new() });
    }

    pub fn cut(&mut self, ctx: &egui::Context) {
        if self.sel.is_empty() {
            return;
        }
        self.copy(ctx);
        self.delete_selected();
    }

    /// Pastes clip JSON (the OS clipboard text when it is one, else the internal clipboard) in place.
    pub fn paste(&mut self, text: &str) -> bool {
        let clip = if text.contains("\"clip\"") { text.to_string() } else { self.clip.clone() };
        if clip.is_empty() {
            return false;
        }
        self.doc.checkpoint();
        let ops_before = self.doc.ops.len();
        let ids = load_clip(&clip, &mut self.doc, Pt::default());
        if ids.is_empty() {
            self.doc.undo();
            self.status = "Nothing to paste.".into();
            return false;
        }
        self.sel = ids;
        let n = self.doc.ops.len() - ops_before;
        self.status = format!("Pasted {} shape(s){}", self.sel.len(), if n > 0 { format!(" with {n} toolpath(s)") } else { String::new() });
        self.mutated();
        true
    }

    pub fn select_all(&mut self) {
        self.sel = self.doc.shapes.iter().filter(|s| self.doc.shape_editable(s)).map(|s| s.id).collect();
        self.sel_changed();
    }

    fn sel_centre(&self) -> Option<Pt> {
        let r = self.sel_rect;
        (!r.is_empty()).then(|| pt((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0))
    }

    pub fn flip(&mut self, horizontal: bool) {
        let Some(c) = self.sel_centre() else { return };
        let s = if horizontal { Affine::scale(-1.0, 1.0) } else { Affine::scale(1.0, -1.0) };
        self.transform_selection(&(Affine::translate(c.x, c.y) * s * Affine::translate(-c.x, -c.y)));
    }

    /// Mirror copy: a flipped copy placed against the selection's right edge (`horizontal`) or its top edge, so the
    /// two halves share that edge (symmetric parts: draw half, mirror, weld). The copies end up selected.
    pub fn mirror_copy(&mut self, horizontal: bool) {
        let r = self.sel_rect;
        if self.sel.is_empty() || r.is_empty() {
            return;
        }
        let m = if horizontal { Affine::translate(2.0 * r.x1, 0.0) * Affine::scale(-1.0, 1.0) } else { Affine::translate(0.0, 2.0 * r.y1) * Affine::scale(1.0, -1.0) };
        self.doc.checkpoint();
        let src: Vec<Shape> = self.sel.iter().filter_map(|id| self.doc.find(*id).cloned()).collect();
        self.sel = src.iter().map(|s| self.doc.add(transformed(&s.c, &m), Some(s.layer))).collect();
        self.status = format!("Mirror copy: {} vector(s). Weld the two halves to make one outline.", src.len());
        self.mutated();
    }

    pub fn rotate90(&mut self) {
        let Some(c) = self.sel_centre() else { return };
        self.transform_selection(&(Affine::translate(c.x, c.y) * Affine::rotate(PI / 2.0) * Affine::translate(-c.x, -c.y)));
    }

    /// Replaces the selected shapes with `result` (same layer); toolpaths that used the old shapes move to the new ones.
    pub fn replace_selection(&mut self, result: Vec<Contour>, what: &str, keep_original: bool) {
        if result.is_empty() {
            self.status = format!("{what}: no result (shapes don't overlap, or the offset is too large).");
            return;
        }
        // booleans and offsets leave every arc as hundreds of points: give the curves back so nodes stay editable
        let result: Vec<Contour> = result.iter().map(|c| gcarve_core::vecops::refit_arcs(c, 0.001)).collect();
        self.doc.checkpoint();
        let layer = self.selected().and_then(|id| self.doc.find(id)).map(|s| s.layer).unwrap_or(self.doc.active_layer);
        let old = std::mem::take(&mut self.sel);
        if !keep_original {
            self.doc.shapes.retain(|s| !old.contains(&s.id));
        }
        let n = result.len();
        for c in result {
            let id = self.doc.add(c, Some(layer));
            self.sel.insert(id);
        }
        if !keep_original {
            let new_ids: Vec<i32> = self.sel.iter().copied().collect();
            for op in &mut self.doc.ops {
                let before = op.shape_ids.len();
                op.shape_ids.retain(|id| !old.contains(id));
                if op.shape_ids.len() != before {
                    op.shape_ids.extend(&new_ids);
                }
            }
        }
        self.status = format!("{what}: {n} shape(s)");
        self.mutated();
    }

    fn sel_contours(&self) -> Vec<Contour> {
        let ids: Vec<i32> = self.sel.iter().copied().collect();
        self.doc.contours_of(&ids)
    }

    pub fn weld_selection(&mut self) {
        if self.sel.len() < 2 {
            self.status = "Select two or more closed shapes to weld.".into();
            return;
        }
        let r = boolean_op(BoolOp::Weld, &self.sel_contours(), &[]);
        self.replace_selection(r, "Weld", false);
    }

    /// The largest shape keeps, the others are cut out of it.
    pub fn subtract_selection(&mut self) {
        if self.sel.len() < 2 {
            self.status = "Select the shape to keep plus the shapes to cut out.".into();
            return;
        }
        let mut all = self.sel_contours();
        let big = (0..all.len()).max_by(|&a, &b| contour_area(&all[a]).total_cmp(&contour_area(&all[b]))).unwrap();
        let subj = all.remove(big);
        let r = boolean_op(BoolOp::Subtract, &[subj], &all);
        self.replace_selection(r, "Subtract", false);
    }

    pub fn intersect_selection(&mut self) {
        if self.sel.len() < 2 {
            self.status = "Select two or more closed shapes to intersect.".into();
            return;
        }
        let all = self.sel_contours();
        let mut acc = vec![all[0].clone()];
        for c in &all[1..] {
            if acc.is_empty() {
                break;
            }
            acc = boolean_op(BoolOp::Intersect, &acc, std::slice::from_ref(c));
        }
        self.replace_selection(acc, "Intersect", false);
    }

    /// delta in inches (+ out, - in)
    pub fn offset_selection(&mut self, delta: f64, round_corners: bool, keep_original: bool) {
        if self.sel.is_empty() || delta == 0.0 {
            return;
        }
        let r = offset_contours(&gcarve_core::vecops::same_direction(&self.sel_contours()), delta, round_corners);
        self.replace_selection(r, "Offset", keep_original);
    }

    /// tool_dia in inches
    pub fn dogbone_selection(&mut self, tool_dia: f64, inside: bool, tbone: bool) {
        if self.sel.is_empty() || tool_dia <= 0.0 {
            return;
        }
        let r = tool_dia / 2.0 + 0.0005; // a hair over the bit radius so it fits the lobe
        let out: Vec<Contour> = self.sel_contours().iter().map(|c| dogbone(c, r, inside, tbone)).collect();
        self.replace_selection(out, if tbone { "T-bone" } else { "Dogbone" }, false);
    }

    /// radius in inches; which corners: outer (stick out), inner (go in)
    pub fn fillet_selection(&mut self, radius: f64, outer: bool, inner: bool) {
        if self.sel.is_empty() || radius <= 0.0 {
            return;
        }
        let out = fillet_contours(&gcarve_core::vecops::same_direction(&self.sel_contours()), radius, outer, inner);
        if out.is_empty() {
            self.status = "Fillet: nothing left (select closed vectors; the radius must fit inside them).".into();
            return;
        }
        self.replace_selection(out, "Fillet", false);
    }

    /// Right-click ▸ Trim Here: the piece of the vector under the pointer, between the crossings either side of
    /// it, is removed. Join / Weld afterwards to build new outlines from the pieces.
    pub fn trim_here(&mut self) {
        let at = self.ctx_at;
        let Some(id) = self.hit_shape(at) else {
            self.status = "Trim: right-click on the part of a vector you want removed.".into();
            return;
        };
        let shapes: Vec<&gcarve_core::document::Shape> = self.doc.shapes.iter().filter(|s| self.doc.shape_visible(s)).collect();
        let all: Vec<&Contour> = shapes.iter().map(|s| &s.c).collect();
        let Some(k) = shapes.iter().position(|s| s.id == id) else { return };
        let Some(pieces) = gcarve_core::vecops::trim_at(&all, k, at) else {
            self.status = "Trim: this vector isn't crossed by another one on both sides here.".into();
            return;
        };
        self.sel = [id].into();
        if pieces.is_empty() {
            self.doc.checkpoint();
            self.doc.shapes.retain(|s| s.id != id);
            self.sel.clear();
            self.mutated();
        } else {
            self.replace_selection(pieces, "Trim", false);
        }
        self.status = "Trimmed. Join Open Vectors (or Weld) builds new outlines from the pieces.".into();
    }

    /// Vectors ▸ Simplify: fewer nodes, within `tol` inches of the original
    pub fn simplify_selection(&mut self, tol: f64) {
        if self.sel.is_empty() {
            return;
        }
        let input = self.sel_contours();
        let before: usize = input.iter().map(|c| c.nodes.len()).sum();
        let out = simplify_contours(&input, tol);
        let after: usize = out.iter().map(|c| c.nodes.len()).sum();
        if after >= before {
            self.status = format!("Simplify: nothing to remove at this tolerance ({before} nodes).");
            return;
        }
        self.replace_selection(out, "Simplify", false);
        self.status = format!("Simplify: {before} → {after} nodes.");
    }

    /// Vectors ▸ Check Vectors: selects the open, duplicate and tiny vectors of this sheet and says what to do
    pub fn check_vectors(&mut self) {
        let items: Vec<(i32, &Contour)> = self.doc.shapes.iter().filter(|s| self.doc.shape_editable(s)).map(|s| (s.id, &s.c)).collect();
        let n = items.len();
        let r = gcarve_core::vecops::check_vectors(&items);
        let found: BTreeSet<i32> = r.open.iter().chain(&r.duplicates).chain(&r.tiny).copied().collect();
        if found.is_empty() {
            self.status = format!("Check Vectors: {n} vector(s), no open, duplicate or tiny ones.");
            return;
        }
        let mut parts = Vec::new();
        for (list, what, fix) in [(&r.open, "open", "Join or Close them"), (&r.duplicates, "duplicate", "Delete them"), (&r.tiny, "tiny", "Delete them")] {
            if !list.is_empty() {
                parts.push(format!("{} {what} ({fix})", list.len()));
            }
        }
        self.sel = found;
        self.sel_changed();
        self.status = format!("Check Vectors: {}. They are selected.", parts.join(", "));
    }

    /// Vectors ▸ Extend Ends: the selected open vectors run on until they meet another vector of the sheet
    pub fn extend_selection(&mut self) {
        let ids: Vec<i32> = self.doc.shapes.iter().filter(|s| self.sel.contains(&s.id) && !s.c.closed).map(|s| s.id).collect();
        if ids.is_empty() {
            self.status = "Extend: select open vectors (lines, curves) to run on to the next vector.".into();
            return;
        }
        let mut changed = Vec::new();
        for s in self.doc.shapes.iter().filter(|s| ids.contains(&s.id)) {
            // the vector itself is no target: it would only meet its own far end
            let rest: Vec<&Contour> = self.doc.shapes.iter().filter(|o| o.id != s.id && self.doc.shape_visible(o)).map(|o| &o.c).collect();
            if let Some(c) = gcarve_core::vecops::extend_ends(&s.c, &rest) {
                changed.push((s.id, c));
            }
        }
        if changed.is_empty() {
            self.status = "Extend: no end of the selected vectors runs into another vector.".into();
            return;
        }
        self.doc.checkpoint();
        let n = changed.len();
        for (id, c) in changed {
            if let Some(s) = self.doc.shapes.iter_mut().find(|s| s.id == id) {
                s.c = c;
            }
        }
        self.mutated();
        self.status = format!("Extended {n} vector(s) to the next vector. Trim Here and Join finish the corner.");
    }

    pub fn join_selection(&mut self) {
        if self.sel.len() < 2 {
            self.status = "Select the open vectors to join.".into();
            return;
        }
        let input = self.sel_contours();
        let out = join_open(&input, 0.02);
        if out.len() == input.len() {
            self.status = "Join: no ends close enough to each other.".into();
            return;
        }
        self.replace_selection(out, "Join", false);
    }

    pub fn close_selection(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        let out: Vec<Contour> = self.sel_contours().into_iter().map(|c| if c.closed { c } else { close_contour(c) }).collect();
        self.replace_selection(out, "Close", false);
    }

    /// Align the selection (as one block) to the board: left/right/top/bottom/centerH/centerV/center.
    pub fn align_selection(&mut self, mode: &str) {
        let r = self.sel_rect;
        if r.is_empty() {
            return;
        }
        let m = &self.doc.material;
        let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        let (mut dx, mut dy) = (0.0, 0.0);
        match mode {
            "center" => {
                dx = m.w / 2.0 - cx;
                dy = m.h / 2.0 - cy;
            }
            "centerH" => dx = m.w / 2.0 - cx,
            "centerV" => dy = m.h / 2.0 - cy,
            "left" => dx = -r.x0,
            "right" => dx = m.w - r.x1,
            "bottom" => dy = -r.y0,
            "top" => dy = m.h - r.y1,
            _ => {}
        }
        self.transform_selection(&Affine::translate(dx, dy));
    }

    /// Align the selected shapes to each other (selection bounds): left/right/centerH/top/bottom/centerV.
    pub fn align_objects(&mut self, mode: &str) {
        let r = self.sel_rect;
        if self.sel.len() < 2 || r.is_empty() {
            self.status = "Select two or more shapes to align them to each other.".into();
            return;
        }
        self.doc.checkpoint();
        self.break_constraints_on_move();
        for un in self.selection_units() {
            let b = un.b;
            let (dx, dy) = match mode {
                "left" => (r.x0 - b.x0, 0.0),
                "right" => (r.x1 - b.x1, 0.0),
                "centerH" => ((r.x0 + r.x1) / 2.0 - (b.x0 + b.x1) / 2.0, 0.0),
                "bottom" => (0.0, r.y0 - b.y0),
                "top" => (0.0, r.y1 - b.y1),
                "centerV" => (0.0, (r.y0 + r.y1) / 2.0 - (b.y0 + b.y1) / 2.0),
                _ => (0.0, 0.0),
            };
            if dx != 0.0 || dy != 0.0 {
                for id in un.ids {
                    let s = self.doc.find_mut(id).unwrap();
                    s.c = transformed(&s.c, &Affine::translate(dx, dy));
                }
            }
        }
        self.status = format!("Aligned {} shapes ({mode})", self.sel.len());
        self.mutated();
    }

    /// gap < 0: even spacing between the outer two (which stay); else a fixed gap in inches.
    pub fn distribute_objects(&mut self, horizontal: bool, gap: f64) {
        if self.sel.len() < 3 && gap < 0.0 {
            self.status = "Select three or more shapes to distribute evenly.".into();
            return;
        }
        if self.sel.len() < 2 {
            return;
        }
        let h = horizontal;
        let mut items = self.selection_units();
        items.sort_by(|a, b| if h { (a.b.x0 + a.b.x1).total_cmp(&(b.b.x0 + b.b.x1)) } else { (a.b.y0 + a.b.y1).total_cmp(&(b.b.y0 + b.b.y1)) });
        self.doc.checkpoint();
        self.break_constraints_on_move();
        let size = |b: &Rect| if h { b.w() } else { b.h() };
        let start = |b: &Rect| if h { b.x0 } else { b.y0 };
        let g = if gap < 0.0 {
            let total: f64 = items.iter().map(|it| size(&it.b)).sum();
            let span = if h { items.last().unwrap().b.x1 - items[0].b.x0 } else { items.last().unwrap().b.y1 - items[0].b.y0 };
            (span - total) / (items.len() - 1) as f64
        } else {
            gap
        };
        let mut cursor = start(&items[0].b);
        for it in &items {
            let d = cursor - start(&it.b);
            if d.abs() > 1e-12 {
                for id in &it.ids {
                    let s = self.doc.find_mut(*id).unwrap();
                    s.c = transformed(&s.c, &if h { Affine::translate(d, 0.0) } else { Affine::translate(0.0, d) });
                }
            }
            cursor += size(&it.b) + g;
        }
        self.status = if gap < 0.0 { format!("Distributed {} shapes evenly", items.len()) } else { format!("Spaced {} shapes {} apart", items.len(), units::fmt(&self.doc, gap)) };
        self.mutated();
    }

    /// Grid array: rows x cols copies with gaps (inches) between the selection's bounding boxes.
    pub fn array_copy(&mut self, rows: i32, cols: i32, gap_x: f64, gap_y: f64) {
        let r = self.sel_rect;
        if self.sel.is_empty() || r.is_empty() || rows * cols <= 1 {
            return;
        }
        let (dx, dy) = (r.w() + gap_x, r.h() + gap_y);
        self.doc.checkpoint();
        let src: Vec<Shape> = self.sel.iter().filter_map(|id| self.doc.find(*id).cloned()).collect();
        for row in 0..rows {
            for col in 0..cols {
                if row == 0 && col == 0 {
                    continue;
                }
                for s in &src {
                    let id = self.doc.add(transformed(&s.c, &Affine::translate(col as f64 * dx, row as f64 * dy)), Some(s.layer));
                    self.sel.insert(id);
                }
            }
        }
        self.status = format!("Array: {cols} × {rows}");
        self.mutated();
    }

    /// Circular array: `count` copies (the original included) around `centre`, optionally rotating each copy.
    pub fn array_circular(&mut self, count: i32, centre: Pt, rotate_copies: bool) {
        if self.sel.is_empty() || count < 2 {
            return;
        }
        self.doc.checkpoint();
        let src: Vec<Shape> = self.sel.iter().filter_map(|id| self.doc.find(*id).cloned()).collect();
        let sc = self.sel_centre().unwrap_or(centre);
        for k in 1..count {
            let ang = 2.0 * PI * k as f64 / count as f64;
            let m = if rotate_copies {
                Affine::translate(centre.x, centre.y) * Affine::rotate(ang) * Affine::translate(-centre.x, -centre.y)
            } else {
                let p = Affine::translate(centre.x, centre.y) * Affine::rotate(ang) * Affine::translate(-centre.x, -centre.y);
                let np = p.apply(sc);
                Affine::translate(np.x - sc.x, np.y - sc.y)
            };
            for s in &src {
                let id = self.doc.add(transformed(&s.c, &m), Some(s.layer));
                self.sel.insert(id);
            }
        }
        self.status = format!("Circular array: {count} copies");
        self.mutated();
    }

    /// Copy along a vector (VCarve "Copy along vectors"): the longest selected vector is the path, everything else
    /// selected is copied `count` times evenly along it, centred on the path; `turn` rotates each copy to the
    /// path's direction there. The originals stay; the copies end up selected.
    pub fn array_along(&mut self, count: i32, turn: bool) {
        use gcarve_core::geom::dist;
        let length = |c: &Contour| { let p = gcarve_core::geom::flatten(c, 0.001); p.windows(2).map(|w| dist(w[0], w[1])).sum::<f64>() + if c.closed && p.len() > 1 { dist(p[p.len() - 1], p[0]) } else { 0.0 } };
        let sel: Vec<Shape> = self.sel.iter().filter_map(|id| self.doc.find(*id).cloned()).collect();
        let Some(path) = sel.iter().max_by(|a, b| length(&a.c).total_cmp(&length(&b.c))).filter(|_| sel.len() > 1 && count >= 2).cloned() else {
            self.status = "Copy along a vector: select the object(s) to copy and the vector to follow.".into();
            return;
        };
        let src: Vec<&Shape> = sel.iter().filter(|s| s.id != path.id).collect();
        let mut r = Rect::default();
        for s in &src {
            r.add_rect(&bounds(&s.c));
        }
        let centre = pt((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        let mut p = gcarve_core::geom::flatten(&path.c, 0.001);
        if path.c.closed && p.len() > 1 && dist(p[0], p[p.len() - 1]) > 1e-9 {
            p.push(p[0]);
        }
        let mut lens = vec![0.0];
        for w in p.windows(2) {
            lens.push(lens[lens.len() - 1] + dist(w[0], w[1]));
        }
        let total = lens[lens.len() - 1];
        if total <= 0.0 {
            return;
        }
        self.doc.checkpoint();
        self.sel.clear();
        let steps = if path.c.closed { count } else { count - 1 };
        for k in 0..count {
            let s = (total * k as f64 / steps as f64).min(total);
            let i = lens.partition_point(|&l| l <= s).clamp(1, p.len() - 1);
            let t = if lens[i] > lens[i - 1] { (s - lens[i - 1]) / (lens[i] - lens[i - 1]) } else { 0.0 };
            let at = pt(p[i - 1].x + (p[i].x - p[i - 1].x) * t, p[i - 1].y + (p[i].y - p[i - 1].y) * t);
            let ang = if turn { (p[i].y - p[i - 1].y).atan2(p[i].x - p[i - 1].x) } else { 0.0 };
            let m = Affine::translate(at.x, at.y) * Affine::rotate(ang) * Affine::translate(-centre.x, -centre.y);
            for sh in &src {
                let id = self.doc.add(transformed(&sh.c, &m), Some(sh.layer));
                self.sel.insert(id);
            }
        }
        self.status = format!("{count} copies along the vector");
        self.mutated();
    }

    /// True-shape nesting of the selection on the active board. Inner shapes (box inside another selected shape)
    /// and group members travel with their parent. Returns (placed, ids that did not fit).
    pub fn nest_selection(&mut self, gap: f64, margin: f64, rot_step_deg: i32) -> (usize, Vec<i32>) {
        if self.sel.is_empty() {
            return (0, Vec::new());
        }
        let ids: Vec<i32> = self.sel.iter().copied().collect();
        let n = ids.len();
        let boxes: Vec<Rect> = ids.iter().map(|id| bounds(&self.doc.find(*id).unwrap().c)).collect();
        let area = |b: &Rect| b.w() * b.h();
        let mut parent: Vec<i32> = vec![-1; n];
        for i in 0..n {
            let mut best_a = 1e300;
            for j in 0..n {
                if i == j || !boxes[j].contains(&boxes[i]) || area(&boxes[j]) <= area(&boxes[i]) {
                    continue;
                }
                if area(&boxes[j]) < best_a {
                    best_a = area(&boxes[j]);
                    parent[i] = j as i32;
                }
            }
        }
        // grouped shapes travel with the group's largest member
        let mut group_root: BTreeMap<i32, usize> = BTreeMap::new();
        for i in 0..n {
            let g = self.doc.find(ids[i]).unwrap().group;
            if g == 0 {
                continue;
            }
            match group_root.get(&g) {
                Some(&r) if area(&boxes[i]) <= area(&boxes[r]) => {}
                _ => {
                    group_root.insert(g, i);
                }
            }
        }
        for i in 0..n {
            let g = self.doc.find(ids[i]).unwrap().group;
            if g != 0 && group_root[&g] != i {
                parent[i] = group_root[&g] as i32;
            }
        }
        for r in group_root.values() {
            parent[*r] = -1;
        }
        let top = |mut i: usize| {
            while parent[i] >= 0 {
                i = parent[i] as usize;
            }
            i
        };
        let roots: Vec<usize> = (0..n).filter(|&i| parent[i] < 0).collect();
        let mut parts = Vec::new();
        for &r in &roots {
            let sh = self.doc.find(ids[r]).unwrap();
            let mut outline = if sh.c.closed { flatten(&sh.c, 0.002) } else { Vec::new() };
            outline.pop();
            let rect_outline = |b: &Rect| vec![pt(b.x0, b.y0), pt(b.x1, b.y0), pt(b.x1, b.y1), pt(b.x0, b.y1)];
            if outline.len() < 3 {
                outline = rect_outline(&boxes[r]); // open shapes: their box
            }
            if sh.group != 0 {
                // a group: use the box of all members unless they all sit inside the root
                let mut gb = boxes[r];
                for (i, b) in boxes.iter().enumerate() {
                    if top(i) == r {
                        gb.add_rect(b);
                    }
                }
                if !boxes[r].contains(&gb) {
                    outline = rect_outline(&gb);
                }
            }
            parts.push(NestPart { outline });
        }
        let places = nest_shapes(&parts, self.doc.material.w, self.doc.material.h, gap, margin, rot_step_deg);
        self.doc.checkpoint();
        let mut placed = 0;
        let mut unplaced = Vec::new();
        for (k, &root) in roots.iter().enumerate() {
            if !places[k].placed {
                unplaced.extend((0..n).filter(|&i| top(i) == root).map(|i| ids[i]));
                continue;
            }
            let m = Affine::translate(places[k].offset.x, places[k].offset.y) * Affine::rotate(places[k].rotation);
            for (i, id) in ids.iter().enumerate() {
                if top(i) == root {
                    let s = self.doc.find_mut(*id).unwrap();
                    s.c = transformed(&s.c, &m);
                }
            }
            placed += 1;
        }
        self.status = if placed == roots.len() { format!("Nested {placed} part(s) on the board") } else { format!("Nested {placed} of {} part(s); the rest don't fit on this board", roots.len()) };
        self.mutated();
        (placed, unplaced)
    }

    /// Nest, moving leftovers onto the next sheet (a new one with the same board when needed), until all fit.
    pub fn nest_selection_sheets(&mut self, gap: f64, margin: f64, rot_step_deg: i32) -> usize {
        let mut total = 0;
        let mut last_unplaced = usize::MAX;
        for _ in 0..50 {
            let (placed, rest) = self.nest_selection(gap, margin, rot_step_deg);
            total += placed;
            if rest.is_empty() {
                break;
            }
            if rest.len() == last_unplaced {
                self.status = format!("{} part(s) are bigger than the board and were left on sheet {}", rest.len(), self.doc.active_sheet + 1);
                self.mutated();
                return total;
            }
            last_unplaced = rest.len();
            let next = self.doc.active_sheet + 1;
            if next >= self.doc.sheets.len() as i32 {
                self.doc.sync_sheet();
                let m = self.doc.material.clone();
                self.doc.sheets.push(m);
            }
            for id in &rest {
                if let Some(s) = self.doc.find_mut(*id) {
                    s.sheet = next;
                }
            }
            self.doc.set_active_sheet(next);
            self.sel = rest.into_iter().collect();
            self.sel_changed();
        }
        self.status = format!("Nested {total} part(s) across {} sheet(s)", self.doc.active_sheet + 1);
        self.mutated();
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::View;
    use gcarve_core::document::{Op, OpType};
    use gcarve_core::geom::{make_ellipse, make_rect};

    fn canvas() -> Canvas {
        Canvas { view: Some(View { zoom: 20.0, origin: egui::Pos2::ZERO }), ..Default::default() }
    }

    #[test]
    fn clipboard_round_trip_keeps_toolpaths_and_tools() {
        let mut c = canvas();
        let a = c.add_shape(make_rect(0.0, 0.0, 2.0, 1.0));
        c.doc.ops.push(Op { kind: OpType::Pocket, shape_ids: vec![a], tool: 1, ..Default::default() });
        c.sel = BTreeSet::from([a]);
        let ctx = egui::Context::default();
        c.copy(&ctx);
        assert!(c.clip.contains("\"clip\"") && c.status.contains("1 toolpath"));
        let mut d = canvas();
        d.doc.tools.clear();
        let json = c.clip.clone();
        assert!(d.paste(&json));
        assert_eq!(d.doc.shapes.len(), 1);
        assert_eq!(d.doc.ops.len(), 1);
        assert_eq!(d.doc.ops[0].shape_ids, vec![d.doc.shapes[0].id]);
        assert_eq!(d.doc.tools.len(), 1, "the toolpath's tool is appended");
        assert!((bounds(&d.doc.shapes[0].c).w() - 2.0).abs() < 1e-9);
        // not a clip: the internal clipboard is used
        d.clip = json;
        assert!(d.paste("hello") && d.doc.shapes.len() == 2);
        d.sel = d.doc.shapes.iter().map(|s| s.id).collect();
        d.duplicate();
        assert_eq!(d.doc.shapes.len(), 4);
        d.delete_selected();
        assert_eq!(d.doc.shapes.len(), 2);
        assert!(d.doc.ops.iter().all(|op| !op.shape_ids.is_empty()));
    }

    #[test]
    fn booleans_replace_selection_and_repoint_ops() {
        let mut c = canvas();
        let a = c.add_shape(make_rect(0.0, 0.0, 2.0, 2.0));
        let b = c.add_shape(make_rect(1.0, 1.0, 3.0, 3.0));
        c.doc.ops.push(Op { shape_ids: vec![a, b], ..Default::default() });
        c.sel = BTreeSet::from([a, b]);
        c.sel_changed();
        c.weld_selection();
        assert_eq!(c.doc.shapes.len(), 1);
        let id = c.doc.shapes[0].id;
        assert!(id != a && id != b && c.sel == BTreeSet::from([id]));
        assert_eq!(c.doc.ops[0].shape_ids, vec![id]);
        let bb = bounds(&c.doc.shapes[0].c);
        assert!((bb.w() - 3.0).abs() < 1e-3);
        c.offset_selection(0.5, true, true);
        assert_eq!(c.doc.shapes.len(), 2);
        c.doc.undo();
        c.mutated();
        c.sel = BTreeSet::from([id]);
        c.flip(true);
        c.rotate90();
        assert_eq!(c.doc.shapes.len(), 1);
    }

    #[test]
    fn align_array_group_nest() {
        let mut c = canvas();
        let a = c.add_shape(make_rect(0.0, 0.0, 1.0, 1.0));
        let b = c.add_shape(make_ellipse(5.0, 5.0, 1.0, 0.5));
        c.sel = BTreeSet::from([a, b]);
        c.sel_changed();
        c.align_objects("left");
        assert!((bounds(&c.doc.find(b).unwrap().c).x0).abs() < 1e-6);
        c.group_selection();
        assert!(c.selection_has_group());
        c.sel = BTreeSet::from([a]);
        c.sel_changed();
        assert_eq!(c.sel.len(), 2, "groups select as one");
        c.array_copy(2, 3, 0.5, 0.5);
        assert_eq!(c.doc.shapes.len(), 12);
        c.select_all();
        c.doc.material.w = 4.0;
        c.doc.material.h = 6.0;
        let placed = c.nest_selection_sheets(0.1, 0.1, 90);
        assert!(placed == 11 && c.doc.sheets.len() >= 2, "{placed} parts, {} sheets", c.doc.sheets.len());
        c.ungroup_selection();
        assert!(!c.selection_has_group());
    }
}

#[cfg(test)]
mod marquee_tests {
    use super::*;
    use gcarve_core::geom::make_rect;

    #[test]
    fn window_and_crossing() {
        let mut c = Canvas::default();
        let a = c.doc.add(make_rect(0.0, 0.0, 1.0, 1.0), None);
        let b = c.doc.add(make_rect(2.0, 0.0, 3.0, 1.0), None);
        // left to right around the first square and half of the second: only the first (fully inside)
        c.marquee_select(pt(-0.5, -0.5), pt(2.5, 1.5));
        assert_eq!(c.sel, BTreeSet::from([a]));
        // right to left: both (touched)
        c.sel.clear();
        c.marquee_select(pt(2.5, 1.5), pt(-0.5, -0.5));
        assert_eq!(c.sel, BTreeSet::from([a, b]));
    }
}

#[cfg(test)]
mod vector_tool_tests {
    use super::*;
    use gcarve_core::geom::{make_polyline, make_rect};

    #[test]
    fn copies_along_a_vector_trim_and_check() {
        let mut c = Canvas::default();
        let hole = c.doc.add(make_rect(20.0, 20.0, 20.5, 20.5), None);
        let path = c.doc.add(make_polyline(&[pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 10.0)], false), None);
        c.sel = BTreeSet::from([hole, path]);
        c.array_along(5, true);
        // 5 copies, 5 in apart along the 20 in path, centred on it; the original hole and the path stay
        assert_eq!(c.sel.len(), 5);
        assert_eq!(c.doc.shapes.len(), 7);
        let centres: Vec<Pt> = c.sel.iter().map(|id| { let b = bounds(&c.doc.find(*id).unwrap().c); pt((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0) }).collect();
        for (got, want) in centres.iter().zip([pt(0.0, 0.0), pt(5.0, 0.0), pt(10.0, 0.0), pt(10.0, 5.0), pt(10.0, 10.0)]) {
            assert!((got.x - want.x).abs() < 1e-6 && (got.y - want.y).abs() < 1e-6, "{got:?} vs {want:?}");
        }
        // Check Vectors finds the open path
        c.check_vectors();
        assert!(c.sel.contains(&path) && c.status.contains("1 open"), "{}", c.status);
        // Trim Here on the path's first leg between two of the copies
        c.ctx_at = pt(2.5, 0.0);
        c.trim_here();
        assert!(c.doc.find(path).is_none() && c.status.starts_with("Trimmed"), "{}", c.status);
        // Simplify: a polyline with a needless middle point loses it
        let wiggly = c.doc.add(make_polyline(&[pt(30.0, 0.0), pt(35.0, 0.0001), pt(40.0, 0.0)], false), None);
        c.sel = BTreeSet::from([wiggly]);
        c.simplify_selection(0.005);
        assert!(c.status.contains("3 → 2"), "{}", c.status);
        // Mirror copy: the flipped copy touches the original's right edge
        let half = c.doc.add(make_polyline(&[pt(50.0, 0.0), pt(52.0, 0.0), pt(52.0, 3.0)], false), None);
        c.sel = BTreeSet::from([half]);
        c.sel_changed();
        c.mirror_copy(true);
        let b = bounds(&c.doc.find(*c.sel.iter().next().unwrap()).unwrap().c);
        assert!((b.x0 - 52.0).abs() < 1e-9 && (b.x1 - 54.0).abs() < 1e-9 && (b.y1 - 3.0).abs() < 1e-9, "{b:?}");
    }
}
