//! Document edits on the selection (Qt app: canvas.cpp + canvas_vectors.cpp): mutation bookkeeping, groups,
//! clipboard, delete/duplicate, transforms, booleans, offset, dogbone, join/close, align/distribute, arrays, nesting.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::canvas::{Canvas, Drag};
use crate::units;
use eframe::egui;
use gcarve_core::document::Shape;
use gcarve_core::geom::{bounds, flatten, make_rect, pt, transformed, Affine, Contour, Pt, Rect};
use gcarve_core::nest::NestPart;
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

    /// Toolpaths ▸ Surface the Board: a rectangle the size of the board on the "Surface" layer, selected, for a
    /// raster pocket with a big bit that planes the top flat (spoilboard or rough stock). Returns its id.
    pub fn add_surface_rect(&mut self) -> i32 {
        self.doc.checkpoint();
        let (w, h) = (self.doc.material.w, self.doc.material.h);
        let layer = self.doc.layer_named(i18n::t("Surface"));
        let id = self.doc.add(make_rect(0.0, 0.0, w, h), Some(layer));
        self.sel = BTreeSet::from([id]);
        self.mutated();
        id
    }

    pub fn group_selection(&mut self) {
        if self.sel.len() < 2 {
            self.status = i18n::t("Select two or more shapes to group.").into();
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
        self.status = i18n::t("Ungrouped").into();
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
                self.status = i18n::t("Driving dimension released: shape moved by hand").into();
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
        self.doc.ops.retain(|op| !(op.shape_ids.is_empty() && op.layer_scope < 0 && op.group_scope == 0 && !op.all_vectors));
        self.mutated();
    }

    /// right-click a dimension › Make Horizontal / Vertical / Aligned
    pub fn set_dimension_axis(&mut self, i: usize, axis: i32) {
        if i < self.doc.dimensions.len() {
            self.doc.checkpoint();
            self.doc.dimensions[i].axis = axis;
            self.doc.apply_driven_dimensions();
            self.mutated();
        }
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
            self.status = i18n::t("Both ends must snap to shape nodes for a dimension to drive geometry.").into();
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
            self.status = i18n::t("Nothing to paste.").into();
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

    /// Edit › Vector Selector (VCarve's): selects the editable vectors of this sheet that pass every filter
    pub fn select_matching(&mut self, f: &VecFilter) {
        self.sel = self.doc.shapes.iter().filter(|s| self.doc.shape_editable(s) && f.matches(s)).map(|s| s.id).collect();
        self.sel_changed();
        self.status = format!("Vector Selector: {} vector(s) selected.", self.sel.len());
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
            self.status = i18n::t("Select two or more closed shapes to weld.").into();
            return;
        }
        let r = boolean_op(BoolOp::Weld, &self.sel_contours(), &[]);
        self.replace_selection(r, i18n::t("Weld"), false);
    }

    /// The largest shape keeps, the others are cut out of it.
    pub fn subtract_selection(&mut self) {
        if self.sel.len() < 2 {
            self.status = i18n::t("Select the shape to keep plus the shapes to cut out.").into();
            return;
        }
        let mut all = self.sel_contours();
        let big = (0..all.len()).max_by(|&a, &b| contour_area(&all[a]).total_cmp(&contour_area(&all[b]))).unwrap();
        let subj = all.remove(big);
        let r = boolean_op(BoolOp::Subtract, &[subj], &all);
        self.replace_selection(r, i18n::t("Subtract"), false);
    }

    pub fn intersect_selection(&mut self) {
        if self.sel.len() < 2 {
            self.status = i18n::t("Select two or more closed shapes to intersect.").into();
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
        self.replace_selection(acc, i18n::t("Intersect"), false);
    }

    /// delta in inches (+ out, - in)
    pub fn offset_selection(&mut self, delta: f64, round_corners: bool, keep_original: bool) {
        if self.sel.is_empty() || delta == 0.0 {
            return;
        }
        let r = offset_contours(&gcarve_core::vecops::same_direction(&self.sel_contours()), delta, round_corners);
        self.replace_selection(r, i18n::t("Offset"), keep_original);
    }

    /// tool_dia in inches
    pub fn dogbone_selection(&mut self, tool_dia: f64, inside: bool, tbone: bool) {
        if self.sel.is_empty() || tool_dia <= 0.0 {
            return;
        }
        let r = tool_dia / 2.0 + 0.0005; // a hair over the bit radius so it fits the lobe
        let out: Vec<Contour> = self.sel_contours().iter().map(|c| dogbone(c, r, inside, tbone)).collect();
        self.replace_selection(out, if tbone { i18n::t("T-bone") } else { i18n::t("Dogbone") }, false);
    }

    /// radius in inches; which corners: outer (stick out), inner (go in)
    pub fn fillet_selection(&mut self, radius: f64, outer: bool, inner: bool) {
        if self.sel.is_empty() || radius <= 0.0 {
            return;
        }
        let out = fillet_contours(&gcarve_core::vecops::same_direction(&self.sel_contours()), radius, outer, inner);
        if out.is_empty() {
            self.status = i18n::t("Fillet: nothing left (select closed vectors; the radius must fit inside them).").into();
            return;
        }
        self.replace_selection(out, i18n::t("Fillet"), false);
    }

    /// Right-click ▸ Trim Here: the piece of the vector under the pointer, between the crossings either side of
    /// it, is removed. Join / Weld afterwards to build new outlines from the pieces.
    pub fn trim_here(&mut self) {
        let at = self.ctx_at;
        let Some(id) = self.hit_shape(at) else {
            self.status = i18n::t("Trim: right-click on the part of a vector you want removed.").into();
            return;
        };
        let shapes: Vec<&gcarve_core::document::Shape> = self.doc.shapes.iter().filter(|s| self.doc.shape_visible(s)).collect();
        let all: Vec<&Contour> = shapes.iter().map(|s| &s.c).collect();
        let Some(k) = shapes.iter().position(|s| s.id == id) else { return };
        let Some(pieces) = gcarve_core::vecops::trim_at(&all, k, at) else {
            self.status = i18n::t("Trim: this vector isn't crossed by another one on both sides here.").into();
            return;
        };
        self.sel = [id].into();
        if pieces.is_empty() {
            self.doc.checkpoint();
            self.doc.shapes.retain(|s| s.id != id);
            self.sel.clear();
            self.mutated();
        } else {
            self.replace_selection(pieces, i18n::t("Trim"), false);
        }
        self.status = i18n::t("Trimmed. Join Open Vectors (or Weld) builds new outlines from the pieces.").into();
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
        self.replace_selection(out, i18n::t("Simplify"), false);
        self.status = format!("Simplify: {before} → {after} nodes.");
    }

    /// Vectors › Distort › Bend Along Curve (`two` = false: the one selected open vector is the curve) / Fit Between
    /// Two Curves (the two selected open vectors: the higher one is the top). The other selected shapes are mapped
    /// from their bounding box onto the curve(s); the curves stay. (Below.)
    ///
    /// Vectors › Distort › Fit Into Four Corners: one selected closed shape with exactly four corners (straight
    /// sides, drawn or node-edited into any skew) is the box; the other selected shapes are stretched from their
    /// bounding box onto it (corner pin). When several selected shapes have four corners, the biggest is the box.
    pub fn distort_four_corners(&mut self) {
        let quad = self
            .sel
            .iter()
            .filter_map(|id| self.doc.find(*id))
            .filter(|s| s.c.closed && s.c.nodes.len() == 4 && s.c.nodes.iter().all(|n| !n.has_in && !n.has_out))
            .max_by(|a, b| { let (ra, rb) = (bounds(&a.c), bounds(&b.c)); (ra.w() * ra.h()).total_cmp(&(rb.w() * rb.h())) })
            .map(|s| s.id);
        let (Some(q), true) = (quad, self.sel.len() > 1) else {
            self.status = i18n::t("Fit Into Four Corners: select the shapes and a four-corner box (straight sides) to fit them into.").into();
            return;
        };
        // corners: the lower two are the bottom edge, the upper two the top, each left to right
        let mut p: Vec<Pt> = self.doc.find(q).unwrap().c.nodes.iter().map(|n| n.p).collect();
        p.sort_by(|a, b| a.y.total_cmp(&b.y));
        let (mut lo, mut hi) = ([p[0], p[1]], [p[2], p[3]]);
        lo.sort_by(|a, b| a.x.total_cmp(&b.x));
        hi.sort_by(|a, b| a.x.total_cmp(&b.x));
        let ids: Vec<i32> = self.sel.iter().copied().filter(|id| *id != q).collect();
        let mut frame = Rect::default();
        for id in &ids {
            frame.add_rect(&bounds(&self.doc.find(*id).unwrap().c));
        }
        let shapes: Vec<Contour> = ids.iter().map(|id| self.doc.find(*id).unwrap().c.clone()).collect();
        let out = gcarve_core::vecops::distort(&shapes, frame, &lo, Some(&hi));
        self.doc.checkpoint();
        for (id, c) in ids.iter().zip(out) {
            self.doc.find_mut(*id).unwrap().c = c;
        }
        self.status = format!("Distort: {} shape(s) fitted into the four corners", ids.len());
        self.mutated();
    }

    pub fn distort_selection(&mut self, two: bool) {
        let open: Vec<i32> = self.sel.iter().copied().filter(|id| self.doc.find(*id).is_some_and(|s| !s.c.closed)).collect();
        let want = if two { 2 } else { 1 };
        if open.len() != want {
            self.status = if two { i18n::t("Fit Between Two Curves: select the shapes and exactly two open curves (top and bottom).") } else { i18n::t("Bend Along Curve: select the shapes and one open curve.") }.into();
            return;
        }
        let mut curves: Vec<(Vec<Pt>, f64)> = open.iter().map(|id| {
            let c = &self.doc.find(*id).unwrap().c;
            let mut p = flatten(c, 0.002);
            if p.len() >= 2 && p[0].x > p[p.len() - 1].x {
                p.reverse(); // left to right, so the shapes read the right way round
            }
            let b = bounds(c);
            (p, (b.y0 + b.y1) / 2.0)
        }).collect();
        curves.sort_by(|a, b| a.1.total_cmp(&b.1));
        let ids: Vec<i32> = self.sel.iter().copied().filter(|id| !open.contains(id)).collect();
        let mut frame = Rect::default();
        for id in &ids {
            frame.add_rect(&bounds(&self.doc.find(*id).unwrap().c));
        }
        if ids.is_empty() || frame.is_empty() {
            self.status = i18n::t("Distort: select the shapes to distort as well as the curve(s).").into();
            return;
        }
        let shapes: Vec<Contour> = ids.iter().map(|id| self.doc.find(*id).unwrap().c.clone()).collect();
        let out = gcarve_core::vecops::distort(&shapes, frame, &curves[0].0, curves.get(1).map(|c| c.0.as_slice()));
        self.doc.checkpoint();
        for (id, c) in ids.iter().zip(out) {
            self.doc.find_mut(*id).unwrap().c = c;
        }
        self.status = format!("Distort: {} shape(s) {}", ids.len(), if two { i18n::t("fitted between the curves") } else { i18n::t("bent along the curve") });
        self.mutated();
    }

    /// Vectors › Shelf-Pin Holes… (VCarve's drill bank, the system-32 kind): in every selected closed part, two rows
    /// of `dia` holes `pitch` apart along its long side, `inset` in from both long edges, the first and last at least
    /// `margin` from the ends (the run centred). Circles on the "Shelf pins" layer, for a Drill toolpath.
    pub fn shelf_pin_holes(&mut self, dia: f64, pitch: f64, inset: f64, margin: f64) {
        let parts: Vec<Rect> = self.sel.iter().filter_map(|id| self.doc.find(*id)).filter(|s| s.c.closed).map(|s| bounds(&s.c)).collect();
        if parts.is_empty() || pitch <= 0.0 || dia <= 0.0 {
            self.status = i18n::t("Shelf-pin holes: select the cabinet sides (closed shapes) first.").into();
            return;
        }
        self.doc.checkpoint();
        let layer = self.doc.layer_named(i18n::t("Shelf pins"));
        let mut ids = BTreeSet::new();
        let mut skipped = 0;
        for r in parts {
            let along_x = r.w() >= r.h();
            let (len, wid) = if along_x { (r.w(), r.h()) } else { (r.h(), r.w()) };
            let run = len - 2.0 * margin;
            if run < 0.0 || wid < 2.0 * inset + dia {
                skipped += 1;
                continue;
            }
            let n = ((run / pitch).floor() as usize + 1).min(2_000); // a typed 1e-6 pitch made millions of circles
            let start = (len - (n - 1) as f64 * pitch) / 2.0; // centred along the part
            for row in [inset, wid - inset] {
                for k in 0..n {
                    let a = start + k as f64 * pitch;
                    let (cx, cy) = if along_x { (r.x0 + a, r.y0 + row) } else { (r.x0 + row, r.y0 + a) };
                    ids.insert(self.doc.add(gcarve_core::geom::make_ellipse(cx, cy, dia / 2.0, dia / 2.0), Some(layer)));
                }
            }
        }
        let n = ids.len();
        self.sel = ids;
        self.sel_changed();
        self.status = if skipped > 0 { format!("Shelf-pin holes: {n} holes; {skipped} part(s) too small for the margins and inset") } else { format!("Shelf-pin holes: {n} holes, selected: add a Drill toolpath") };
        self.mutated();
    }

    /// Vectors › Rotary Layout… (VCarve's Wrapped Spiral / Wrapped Fluting, rotary jobs): `starts` lines evenly round
    /// the blank from X `x0` to `x1` on the unrolled board. `pitch` = X travel per turn (a spiral, negative = left
    /// hand), 0 = straight flutes along the blank. A spiral is cut into pieces at the seam (Y = 0 / once round), as
    /// it crosses the board's top and comes back in at the bottom. Open lines on "Rotary layout", selected, for a
    /// Flute / V-Carve / Profile on the line.
    pub fn rotary_layout(&mut self, starts: usize, pitch: f64, x0: f64, x1: f64) {
        let Some(_) = self.doc.material.rotary else {
            self.status = i18n::t("Rotary layout: turn on Job Setup › Rotary first.").into();
            return;
        };
        let circ = self.doc.material.h;
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        if starts == 0 || starts > 64 || x1 - x0 < 1e-6 {
            self.status = i18n::t("Rotary layout: 1 to 64 starts over a length above zero.").into();
            return;
        }
        let mut lines: Vec<Vec<Pt>> = Vec::new();
        for k in 0..starts {
            let mut y = circ * k as f64 / starts as f64;
            if pitch.abs() < 1e-9 {
                lines.push(vec![pt(x0, y), pt(x1, y)]);
                continue;
            }
            let s = circ / pitch; // Y per X
            let mut x = x0;
            while x < x1 - 1e-9 && lines.len() < 20_000 {
                let to_seam = if s > 0.0 { (circ - y) / s } else { -y / s };
                let xe = (x + to_seam.max(0.0)).min(x1);
                let ye = y + s * (xe - x);
                if xe - x > 1e-9 {
                    lines.push(vec![pt(x, y), pt(xe, ye)]);
                }
                x = xe;
                y = if s > 0.0 && (ye - circ).abs() < 1e-9 { 0.0 } else if s < 0.0 && ye.abs() < 1e-9 { circ } else { ye };
            }
        }
        self.doc.checkpoint();
        let layer = self.doc.layer_named(i18n::t("Rotary layout"));
        let ids: BTreeSet<i32> = lines.iter().map(|l| self.doc.add(gcarve_core::geom::make_polyline(l, false), Some(layer))).collect();
        let n = ids.len();
        self.sel = ids;
        self.sel_changed();
        self.status = format!("Rotary layout: {n} line(s) on \"Rotary layout\", selected: add a Flute or V-Carve on them");
        self.mutated();
    }

    /// Vectors › QR Code…: `text` as a QR code `size` wide (quartile error correction: a chipped square still reads),
    /// centred on the board on the "QR code" layer as one group, selected. The dark squares become welded outlines
    /// (pocket or V-carve them); `invert` gives the light squares instead (pocket them so the dark ones stand up).
    pub fn qr_code(&mut self, text: &str, size: f64, invert: bool) {
        use gcarve_core::geom::make_rect;
        use gcarve_core::vecops::{boolean_op, BoolOp};
        use qrcodegen::{QrCode, QrCodeEcc};
        let Ok(qr) = QrCode::encode_text(text.trim(), QrCodeEcc::Quartile) else {
            self.status = i18n::t("QR Code: too much text for a QR code (keep it under about 1,600 characters).").into();
            return;
        };
        if text.trim().is_empty() || size <= 0.0 {
            self.status = i18n::t("QR Code: type the text or web address, and a size.").into();
            return;
        }
        let n = qr.size();
        let m = size / n as f64;
        let m_ = &self.doc.material;
        let (x0, y0) = ((m_.w - size) / 2.0, (m_.h - size) / 2.0);
        // one rectangle per run of squares along a row, then welded into outlines (holes kept)
        let mut rects = Vec::new();
        for r in 0..n {
            let mut c = 0;
            while c < n {
                if qr.get_module(c, r) != invert {
                    let s = c;
                    while c < n && qr.get_module(c, r) != invert {
                        c += 1;
                    }
                    let y = y0 + (n - 1 - r) as f64 * m; // QR rows go down, the drawing's Y goes up
                    rects.push(make_rect(x0 + s as f64 * m, y, x0 + c as f64 * m, y + m));
                }
                c += 1;
            }
        }
        let shapes = boolean_op(BoolOp::Weld, &rects, &[]);
        self.doc.checkpoint();
        let layer = self.doc.layer_named(i18n::t("QR code"));
        let group = self.doc.new_group_id();
        let mut ids = BTreeSet::new();
        for c in shapes {
            let id = self.doc.add(c, Some(layer));
            if let Some(s) = self.doc.find_mut(id) {
                s.group = group;
            }
            ids.insert(id);
        }
        self.sel = ids;
        self.sel_changed();
        self.status = format!("QR Code: {n} × {n} squares of {} on the \"QR code\" layer, selected. Pocket them (an end mill under {} wide, or a V-bit engraving fill) and leave {} of plain board all round so phones can read it.", crate::units::text(&self.doc, m), crate::units::text(&self.doc, m * 0.8), crate::units::text(&self.doc, 4.0 * m));
        self.mutated();
    }

    /// Vectors › Jigsaw Puzzle…: the cut lines (`jigsaw::puzzle`) centred on the board, on the "Puzzle" layer as one
    /// group, selected: cut them with a Profile on the line and a bit narrower than the knob necks.
    pub fn jigsaw(&mut self, w: f64, h: f64, cols: usize, rows: usize, seed: u64) {
        use gcarve_core::geom::{transformed, Affine};
        if w <= 0.0 || h <= 0.0 || cols * rows < 2 {
            self.status = i18n::t("Jigsaw Puzzle: a size and at least two pieces.").into();
            return;
        }
        let m = &self.doc.material;
        let at = Affine::translate(((m.w - w) / 2.0).max(0.0), ((m.h - h) / 2.0).max(0.0));
        self.doc.checkpoint();
        let layer = self.doc.layer_named(i18n::t("Puzzle"));
        let group = self.doc.new_group_id();
        let mut ids = BTreeSet::new();
        for c in gcarve_core::jigsaw::puzzle(w, h, cols, rows, seed) {
            let id = self.doc.add(transformed(&c, &at), Some(layer));
            if let Some(s) = self.doc.find_mut(id) {
                s.group = group;
            }
            ids.insert(id);
        }
        self.sel = ids;
        self.sel_changed();
        // the knob neck is a fifth of the shorter piece side: the bit has to fit through it with wood to spare
        let neck = 0.2 * (w / cols as f64).min(h / rows as f64);
        self.status = format!("Jigsaw Puzzle: {} pieces on the \"Puzzle\" layer, selected. Cut with a Profile on the line, with a bit under {} wide (the knob necks are {}).", cols * rows, crate::units::text(&self.doc, neck / 3.0), crate::units::text(&self.doc, neck));
        self.mutated();
    }

    /// Toolpaths › Inlay Wizard…: the selected closed shapes become an inlay. This sheet gets the female toolpath; a new
    /// sheet "Inlay plug" (a board just big enough) gets the plug, mirrored because it goes in face down.
    /// V-bit (`flat` false): female V-carve to a flat floor `depth`; plug = V-carve of the margin box minus the design
    /// starting `gap` down (the glue gap), floors cleared by an end mill on both. Flat: the design's corners are rounded
    /// to the end mill both ways (`fillet_contours` outer + inner, so the pocket and the plug's outside profile can
    /// each cut that shape exactly); female = that shape pocketed; plug = it shrunk by `gap` (the fit, per side), cut
    /// round on the outside 1/16 in deeper than the pocket so it stands proud to be planed flush.
    pub fn inlay(&mut self, flat: bool, depth: f64, gap: f64, margin: f64) {
        use crate::dialogs::sign::bit;
        use gcarve_core::document::{Material, Op, OpType, Side, ToolType};
        let picked: Vec<Contour> = self.doc.shapes.iter().filter(|s| self.sel.contains(&s.id) && s.c.closed).map(|s| s.c.clone()).collect();
        if picked.is_empty() || depth <= 0.0 || gap < 0.0 || (!flat && gap >= depth) {
            self.status = if picked.is_empty() { i18n::t("Inlay Wizard: select the closed shapes of the design first.").into() } else { i18n::t("Inlay Wizard: a depth, and a gap smaller than it.").into() };
            return;
        }
        self.doc.checkpoint();
        let vbit = (!flat).then(|| bit(self, ToolType::VBit, 0.5, i18n::t("90° V-Bit (0.5 inch)")));
        let mill = bit(self, ToolType::EndMill, if flat { 0.125 } else { 0.25 }, if flat { i18n::t("1/8 End Mill (0.125 inch)") } else { i18n::t("1/4 End Mill (0.25 inch)") });
        let r = self.doc.tools[mill as usize].diameter / 2.0;
        let (female, plug): (Vec<Contour>, Vec<Contour>) = if flat {
            let t = fillet_contours(&picked, r + 0.001, true, true);
            let p = if gap > 0.0 { offset_contours(&t, -gap, true) } else { t.clone() };
            (t, p)
        } else {
            (picked.clone(), picked.clone())
        };
        if female.is_empty() || plug.is_empty() {
            self.doc.undo();
            self.status = format!("Inlay Wizard: the design is too thin for a {} end mill.", units::text(&self.doc, 2.0 * r));
            return;
        }
        let mut b = Rect::default();
        plug.iter().for_each(|c| b.add_rect(&bounds(c)));
        let front = self.doc.active_sheet;
        let layer = self.doc.layer_named(i18n::t("Inlay"));
        let female_ids: Vec<i32> = if flat { female.into_iter().map(|c| self.doc.add(c, Some(layer))).collect() } else { self.sel.iter().copied().collect() };
        // the plug board: the design (+ the V-bit's margin box) with 1/2 in of board round it, mirrored left to right
        let room = if flat { 0.5 } else { margin + 0.5 };
        let mirror = Affine { a: -1.0, d: 1.0, e: room + b.x1, f: room - b.y0, ..Default::default() };
        self.doc.sync_sheet();
        let t = depth + 0.25;
        self.doc.sheets.push(Material { name: i18n::t("Inlay plug").into(), back_of: None, w: b.w() + 2.0 * room, h: b.h() + 2.0 * room, t, ..self.doc.material.clone() });
        self.doc.set_active_sheet(self.doc.sheets.len() as i32 - 1);
        let mut plug_ids: Vec<i32> = plug.iter().map(|c| self.doc.add(transformed(c, &mirror), Some(layer))).collect();
        if !flat {
            plug_ids.push(self.doc.add(make_rect(0.5, 0.5, b.w() + 2.0 * room - 0.5, b.h() + 2.0 * room - 0.5), Some(layer)));
        }
        let plug_sheet = self.doc.active_sheet;
        self.doc.set_active_sheet(front);
        let ops = if let Some(v) = vbit {
            [
                Op { kind: OpType::VCarve, name: i18n::t("Inlay pocket").into(), tool: v, clear_tool: mill, shape_ids: female_ids, depth, flat_depth: depth, sheet_scope: front, ..Default::default() },
                Op { kind: OpType::VCarve, name: i18n::t("Inlay plug").into(), tool: v, clear_tool: mill, shape_ids: plug_ids, depth, flat_depth: depth, start_depth: gap, sheet_scope: plug_sheet, ..Default::default() },
            ]
        } else {
            [
                Op { kind: OpType::Pocket, name: i18n::t("Inlay pocket").into(), tool: mill, shape_ids: female_ids, depth, sheet_scope: front, ..Default::default() },
                Op { kind: OpType::Profile, name: i18n::t("Inlay plug").into(), tool: mill, shape_ids: plug_ids, depth: depth + 0.0625, side: Side::Outside, sheet_scope: plug_sheet, ..Default::default() },
            ]
        };
        self.doc.ops.extend(ops);
        self.mutated();
        self.status = if flat {
            format!("Inlay Wizard: \"Inlay pocket\" here (corners rounded to the {} bit), the plug on sheet \"Inlay plug\", mirrored, {} smaller all round. Glue it in face down, plane it flush.", units::text(&self.doc, 2.0 * r), units::text(&self.doc, gap))
        } else {
            format!("Inlay Wizard: \"Inlay pocket\" V-carved here, the plug on sheet \"Inlay plug\", mirrored, starting {} down (glue gap). Glue it in face down, then saw or plane the plug board off.", units::text(&self.doc, gap))
        };
    }

    /// Vectors › Living Hinge…: staggered slots (`vecops::living_hinge`) over the selection's box (or the board, 1 in
    /// in from its edges), on the "Hinge" layer as one group, selected. `along_y` = slots run up the board (it bends
    /// about an up-down line), else across.
    pub fn living_hinge(&mut self, spacing: f64, slot: f64, bridge: f64, along_y: bool) {
        let sel = self.sel_rect;
        let m = &self.doc.material;
        let area = if sel.is_empty() || self.sel.is_empty() { Rect { x0: 1.0, y0: 1.0, x1: m.w - 1.0, y1: m.h - 1.0 } } else { sel };
        // across: work in the mirror image about the diagonal (x <-> y), then mirror back
        let swap = Affine { a: 0.0, b: 1.0, c: 1.0, d: 0.0, e: 0.0, f: 0.0 };
        let frame = if along_y { area } else { Rect { x0: area.y0, y0: area.x0, x1: area.y1, y1: area.x1 } };
        let lines = gcarve_core::vecops::living_hinge(frame, spacing, slot, bridge);
        if lines.is_empty() {
            self.status = i18n::t("Living Hinge: the area is smaller than one column of slots.").into();
            return;
        }
        self.doc.checkpoint();
        let layer = self.doc.layer_named(i18n::t("Hinge"));
        let group = self.doc.new_group_id();
        let mut ids = BTreeSet::new();
        for c in lines {
            let id = self.doc.add(if along_y { c } else { transformed(&c, &swap) }, Some(layer));
            if let Some(s) = self.doc.find_mut(id) {
                s.group = group;
            }
            ids.insert(id);
        }
        let n = ids.len();
        self.sel = ids;
        self.sel_changed();
        self.status = format!("Living Hinge: {n} slots on the \"Hinge\" layer, selected. Cut them with a Profile on the line, right through, with a bit no wider than {} (thin board, 1/4 in or less, bends best).", units::text(&self.doc, (spacing / 2.0).min(0.125)));
        self.mutated();
    }

    /// Vectors › Box Maker…: the finger-jointed panels (`boxjoint::panels`) laid flat in rows across the board, on the
    /// "Box" layer, each panel its own group, selected: Profile (outside) them through the board.
    pub fn box_maker(&mut self, spec: gcarve_core::boxjoint::BoxSpec) {
        use gcarve_core::geom::{transformed, Affine};
        if spec.t <= 0.0 || [spec.w, spec.d, spec.h].iter().any(|&v| v < 4.0 * spec.t) {
            self.status = i18n::t("Box Maker: each size must be at least 4 × the board thickness.").into();
            return;
        }
        let gap = (spec.bit * 2.0).max(0.5);
        let max_w = self.doc.material.w.max(spec.w.max(spec.d) + gap);
        self.doc.checkpoint();
        let layer = self.doc.layer_named(i18n::t("Box"));
        let (mut x, mut y, mut row) = (gap / 2.0, gap / 2.0, 0.0f64);
        let mut ids = BTreeSet::new();
        let panels = gcarve_core::boxjoint::panels(&spec);
        for (_, a, b, cs) in &panels {
            if x + a > max_w && x > gap {
                (x, y, row) = (gap / 2.0, y + row + gap, 0.0);
            }
            let group = self.doc.new_group_id();
            for c in cs {
                let id = self.doc.add(transformed(c, &Affine::translate(x, y)), Some(layer));
                if let Some(s) = self.doc.find_mut(id) {
                    s.group = group;
                }
                ids.insert(id);
            }
            (x, row) = (x + a + gap, row.max(*b));
        }
        self.sel = ids;
        self.sel_changed();
        self.status = format!("Box Maker: {} panels on the \"Box\" layer, selected: add a Profile (outside, through the board). Nest them to use less board.", panels.len());
        self.mutated();
    }

    /// Vectors › Vector Texture…: wavy lines filling the selected closed shapes, added next to them (same layer)
    pub fn texture_selection(&mut self, spacing: f64, amplitude: f64, wavelength: f64, angle: f64, vary: f64) {
        let shapes = self.sel_contours();
        let lines = gcarve_core::vecops::vector_texture(&shapes, spacing, amplitude, wavelength, angle, vary);
        let n = lines.len();
        if n == 0 {
            self.status = i18n::t("Vector Texture: select one or more closed shapes to fill.").into();
            return;
        }
        let layer = self.selected().and_then(|id| self.doc.find(id)).map(|s| s.layer);
        self.doc.checkpoint();
        let group = self.doc.new_group_id();
        let mut ids = BTreeSet::new();
        for c in lines {
            let id = self.doc.add(c, layer);
            if let Some(s) = self.doc.find_mut(id) {
                s.group = group;
            }
            ids.insert(id);
        }
        self.sel = ids;
        self.sel_changed();
        self.status = format!("Vector Texture: {n} lines added as one group. Cut them with a V-bit or ball nose (Profile on the line, or Flute).");
        self.mutated();
    }

    /// Vectors › Simplify › Fit Arcs and Curves (VCarve "Fit curves to vectors"): runs of points that lie on an arc
    /// become arcs (Bézier nodes with handles) within `tol` inches; straight runs lose their extra points
    pub fn fit_curves_selection(&mut self, tol: f64) {
        if self.sel.is_empty() {
            return;
        }
        let input = self.sel_contours();
        let before: usize = input.iter().map(|c| c.nodes.len()).sum();
        // straight runs first (simplify flattens curves), then arcs through what is left
        let out: Vec<Contour> = simplify_contours(&input, tol / 4.0).iter().map(|c| gcarve_core::vecops::refit_arcs(c, tol)).collect();
        let after: usize = out.iter().map(|c| c.nodes.len()).sum();
        if after >= before {
            self.status = format!("Fit curves: nothing to fit at this tolerance ({before} nodes).");
            return;
        }
        self.replace_selection(out, i18n::t("Fit curves"), false);
        self.status = format!("Fit curves: {before} → {after} nodes.");
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
        for (list, what, fix) in [(&r.open, "open", i18n::t("Join or Close them")), (&r.duplicates, "duplicate", i18n::t("Delete them")), (&r.tiny, "tiny", i18n::t("Delete them"))] {
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
            self.status = i18n::t("Extend: select open vectors (lines, curves) to run on to the next vector.").into();
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
            self.status = i18n::t("Extend: no end of the selected vectors runs into another vector.").into();
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
            self.status = i18n::t("Select the open vectors to join.").into();
            return;
        }
        let input = self.sel_contours();
        let out = join_open(&input, 0.02);
        if out.len() == input.len() {
            self.status = i18n::t("Join: no ends close enough to each other.").into();
            return;
        }
        self.replace_selection(out, i18n::t("Join"), false);
    }

    pub fn close_selection(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        let out: Vec<Contour> = self.sel_contours().into_iter().map(|c| if c.closed { c } else { close_contour(c) }).collect();
        self.replace_selection(out, i18n::t("Close"), false);
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
            self.status = i18n::t("Select two or more shapes to align them to each other.").into();
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
            self.status = i18n::t("Select three or more shapes to distribute evenly.").into();
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
            self.status = i18n::t("Copy along a vector: select the object(s) to copy and the vector to follow.").into();
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
    /// `corner` = where the packing starts (VCarve "Nest From"): 0 bottom-left, 1 bottom-right, 2 top-left, 3 top-right
    pub fn nest_selection(&mut self, gap: f64, margin: f64, rot_step_deg: i32, corner: u8) -> (usize, Vec<i32>) {
        let mirror = self.nest_mirror;
        if self.sel.is_empty() {
            return (0, Vec::new());
        }
        self.doc.checkpoint();
        let mut ids: Vec<i32> = self.sel.iter().copied().collect();
        // nest boundary: the largest selected closed shape (an offcut's outline) stays put and holds the rest
        let boundary_id = self.nest_boundary.then(|| ids.iter().copied().filter(|id| self.doc.find(*id).is_some_and(|s| s.c.closed)).max_by(|a, b| {
            let ar = |id: &i32| { let r = bounds(&self.doc.find(*id).unwrap().c); r.w() * r.h() };
            ar(a).total_cmp(&ar(b))
        })).flatten();
        if let Some(b) = boundary_id {
            ids.retain(|id| *id != b);
            if ids.is_empty() {
                return (0, Vec::new());
            }
        }
        // another corner: mirror the parts so that corner is bottom-left, nest, mirror back (shapes keep their look)
        let (w, h) = (self.doc.material.w, self.doc.material.h);
        let flip = Affine::translate(if corner & 1 == 1 { w } else { 0.0 }, if corner & 2 == 2 { h } else { 0.0 }) * Affine::scale(if corner & 1 == 1 { -1.0 } else { 1.0 }, if corner & 2 == 2 { -1.0 } else { 1.0 });
        let mirror_all = |doc: &mut gcarve_core::document::Document| {
            if corner != 0 {
                for id in &ids {
                    if let Some(s) = doc.find_mut(*id) {
                        s.c = transformed(&s.c, &flip);
                    }
                }
            }
        };
        mirror_all(&mut self.doc);
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
        let fence: Option<Vec<Pt>> = boundary_id.and_then(|b| self.doc.find(b)).map(|s| {
            let mut p = flatten(&s.c, 0.002);
            p.pop();
            p.into_iter().map(|q| flip.apply(q)).collect()
        });
        // filler part (VCarve's): copies of the smallest part fill what space is left; they come last (smallest)
        let n_parts = parts.len();
        let filler = self.nest_filler.then(|| (0..n_parts).min_by(|a, b| poly_area(&parts[*a].outline).total_cmp(&poly_area(&parts[*b].outline)))).flatten();
        let mut family: Vec<Shape> = Vec::new();
        if let Some(k) = filler {
            let a = poly_area(&parts[k].outline).max(1e-6);
            let spare = (self.doc.material.w * self.doc.material.h - parts.iter().map(|p| poly_area(&p.outline)).sum::<f64>()).max(0.0);
            for _ in 0..((spare / a) as usize).min(300) {
                parts.push(parts[k].clone());
            }
            family = ids.iter().enumerate().filter(|(i, _)| top(*i) == roots[k]).filter_map(|(_, id)| self.doc.find(*id).cloned()).collect();
        }
        let places = gcarve_core::nest::nest_shapes_with(&parts, self.doc.material.w, self.doc.material.h, gap, margin, rot_step_deg, mirror, fence.as_deref());
        let mut fillers = 0;
        for p in places.iter().skip(n_parts).filter(|p| p.placed) {
            let m = flip * Affine::translate(p.offset.x, p.offset.y) * Affine::rotate(p.rotation) * Affine::scale(if p.mirror { -1.0 } else { 1.0 }, 1.0);
            let group = if family.iter().any(|s| s.group != 0) { self.doc.new_group_id() } else { 0 };
            for s in &family {
                let id = self.doc.add(transformed(&s.c, &m), Some(s.layer));
                self.doc.find_mut(id).unwrap().group = group;
            }
            fillers += 1;
        }
        let mut placed = 0;
        let mut unplaced = Vec::new();
        let mut moved: Vec<(Vec<Pt>, Affine)> = Vec::new(); // each placed part: its old outline and its move (board coordinates)
        for (k, &root) in roots.iter().enumerate() {
            if !places[k].placed {
                unplaced.extend((0..n).filter(|&i| top(i) == root).map(|i| ids[i]));
                continue;
            }
            let m = Affine::translate(places[k].offset.x, places[k].offset.y) * Affine::rotate(places[k].rotation) * Affine::scale(if places[k].mirror { -1.0 } else { 1.0 }, 1.0);
            moved.push((parts[k].outline.iter().map(|q| flip.apply(*q)).collect(), flip * m * flip));
            for (i, id) in ids.iter().enumerate() {
                if top(i) == root {
                    let s = self.doc.find_mut(*id).unwrap();
                    s.c = transformed(&s.c, &m);
                }
            }
            placed += 1;
        }
        mirror_all(&mut self.doc);
        let backs = self.follow_on_back_sides(&moved);
        self.status = if placed == roots.len() { format!("Nested {placed} part(s) on the board") } else { format!("Nested {placed} of {} part(s); the rest don't fit on this board", roots.len()) };
        if fillers > 0 {
            self.status += &format!(", plus {fillers} filler cop{} of the smallest part", if fillers == 1 { "y" } else { "ies" });
        }
        if backs > 0 {
            self.status += &format!("; {backs} back-side vector{} moved with {}", if backs == 1 { "" } else { "s" }, if backs == 1 { i18n::t("its part") } else { i18n::t("their parts") });
        }
        self.mutated();
        (placed, unplaced)
    }

    /// Two-sided jobs: the vectors on this sheet's back side (Vectors › New Back Side) that lie under a nested part
    /// (seen through the flip) get the same move, mirrored, so both faces still meet. Returns how many moved.
    /// ponytail: parts that overflow onto another sheet leave their back-side vectors where they were.
    fn follow_on_back_sides(&mut self, moved: &[(Vec<Pt>, Affine)]) -> usize {
        let front = self.doc.active_sheet;
        let (w, h) = (self.doc.material.w, self.doc.material.h);
        let backs: Vec<(i32, bool)> = (0..self.doc.sheets.len()).filter_map(|j| self.doc.sheets[j].back_of.filter(|b| b.0 == front).map(|b| (j as i32, b.1))).collect();
        let mut n = 0;
        for (sheet, lr) in backs {
            let f = if lr { Affine::translate(w, 0.0) * Affine::scale(-1.0, 1.0) } else { Affine::translate(0.0, h) * Affine::scale(1.0, -1.0) };
            for s in self.doc.shapes.iter_mut().filter(|s| s.sheet == sheet) {
                let b = bounds(&s.c);
                let c = f.apply(pt((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0));
                let inside = |o: &Vec<Pt>| gcarve_core::geom::point_in_poly(c, o.len(), |i| (o[i].x, o[i].y));
                let area = |o: &Vec<Pt>| { let r = bounds(&gcarve_core::geom::make_polyline(o, true)); r.w() * r.h() };
                if let Some((_, t)) = moved.iter().filter(|(o, _)| inside(o)).min_by(|a, b| area(&a.0).total_cmp(&area(&b.0))) {
                    s.c = transformed(&s.c, &(f * *t * f));
                    n += 1;
                }
            }
        }
        n
    }

    /// Nest, moving leftovers onto the next sheet (a new one with the same board when needed), until all fit.
    pub fn nest_selection_sheets(&mut self, gap: f64, margin: f64, rot_step_deg: i32, corner: u8) -> usize {
        let mut total = 0;
        let mut last_unplaced = usize::MAX;
        for _ in 0..50 {
            let (placed, rest) = self.nest_selection(gap, margin, rot_step_deg, corner);
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
                let m = gcarve_core::document::Material { name: String::new(), back_of: None, ..self.doc.material.clone() };
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

/// area of a closed outline (shoelace), always positive
fn poly_area(p: &[Pt]) -> f64 {
    (0..p.len()).map(|i| { let (a, b) = (p[i], p[(i + 1) % p.len()]); a.x * b.y - b.x * a.y }).sum::<f64>().abs() / 2.0
}

/// Vector Selector filters; lengths in inches, `None` = no limit
#[derive(Clone, Default)]
pub struct VecFilter {
    /// 0 any, 1 open, 2 closed, 3 circles
    pub kind: u8,
    /// longest side of the bounding box
    pub larger: Option<f64>,
    pub smaller: Option<f64>,
    /// layer index; None = any
    pub layer: Option<i32>,
}

impl VecFilter {
    pub fn matches(&self, s: &Shape) -> bool {
        let b = bounds(&s.c);
        let size = (b.x1 - b.x0).max(b.y1 - b.y0);
        let kind = match self.kind {
            1 => !s.c.closed,
            2 => s.c.closed,
            3 => is_circle(&s.c),
            _ => true,
        };
        kind && self.larger.is_none_or(|v| size > v) && self.smaller.is_none_or(|v| size < v) && self.layer.is_none_or(|l| s.layer == l)
    }
}

/// closed, as wide as tall, every outline point within 1 % of the radius from the centre
fn is_circle(c: &Contour) -> bool {
    let b = bounds(c);
    let (w, h) = (b.x1 - b.x0, b.y1 - b.y0);
    if !c.closed || w <= 0.0 || (w - h).abs() > 0.01 * w {
        return false;
    }
    let (cx, cy, r) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0, w / 2.0);
    flatten(c, r * 0.001).iter().all(|p| ((p.x - cx).hypot(p.y - cy) - r).abs() < 0.01 * r)
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
    fn surfacing_the_board_plans_rows_over_all_of_it_with_the_widest_end_mill() {
        let mut c = canvas();
        c.doc.material.w = 20.0;
        c.doc.material.h = 10.0;
        c.doc.tools.clear();
        let mut small = gcarve_core::document::Tool::default();
        small.diameter = 0.25;
        let mut big = small.clone();
        big.diameter = 1.5;
        c.doc.tools.push(small);
        c.doc.tools.push(big);
        let id = c.add_surface_rect();
        let (_, op) = crate::dialogs::op::OpDialog::surface(&c.doc, id).draft(&c.doc).unwrap();
        assert!(op.raster && op.tool == 1 && op.shape_ids == vec![id]);
        c.doc.ops.push(op);
        let gen = gcarve_core::pipeline::regenerate(&c.doc);
        let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for tp in gen.paths.iter().flatten() {
            for m in &tp.moves {
                x0 = x0.min(m.x.unwrap_or(x0));
                x1 = x1.max(m.x.unwrap_or(x1));
                y0 = y0.min(m.y.unwrap_or(y0));
                y1 = y1.max(m.y.unwrap_or(y1));
            }
        }
        // the bit centre reaches the walls' inset by its radius: the whole board gets cut
        assert!(x0 <= 0.76 && x1 >= 19.24 && y0 <= 0.76 && y1 >= 9.24, "{x0} {x1} {y0} {y1}");
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
        let placed = c.nest_selection_sheets(0.1, 0.1, 90, 0);
        assert!(placed == 11 && c.doc.sheets.len() >= 2, "{placed} parts, {} sheets", c.doc.sheets.len());
        c.ungroup_selection();
        assert!(!c.selection_has_group());
    }

    #[test]
    fn nesting_a_two_sided_part_moves_its_back_side_too() {
        let mut c = canvas();
        c.doc.material.w = 20.0;
        c.doc.material.h = 10.0;
        let part = c.add_shape(make_rect(10.0, 6.0, 12.0, 8.0));
        c.doc.sync_sheet();
        c.doc.sheets.push(gcarve_core::document::Material { back_of: Some((0, true)), ..c.doc.material.clone() });
        // on the back (flipped left to right) a hole under the part's middle (front 11, 7 → back 9, 7), and one elsewhere
        let hole = c.doc.add(make_ellipse(9.0, 7.0, 0.3, 0.3), None);
        let other = c.doc.add(make_ellipse(3.0, 2.0, 0.3, 0.3), None);
        for id in [hole, other] {
            c.doc.find_mut(id).unwrap().sheet = 1;
        }
        c.sel = BTreeSet::from([part]);
        c.sel_changed();
        c.nest_selection(0.0, 0.5, 90, 0);
        let pb = bounds(&c.doc.find(part).unwrap().c);
        assert!(pb.x0 < 1.0 && pb.y0 < 1.0, "{pb:?}");
        let (mx, my) = ((pb.x0 + pb.x1) / 2.0, (pb.y0 + pb.y1) / 2.0);
        let hb = bounds(&c.doc.find(hole).unwrap().c);
        assert!(((hb.x0 + hb.x1) / 2.0 - (20.0 - mx)).abs() < 1e-6 && ((hb.y0 + hb.y1) / 2.0 - my).abs() < 1e-6, "{hb:?} for part at {mx}, {my}");
        let ob = bounds(&c.doc.find(other).unwrap().c);
        assert!(((ob.x0 + ob.x1) / 2.0 - 3.0).abs() < 1e-9, "a back vector under no part stays");
        assert!(c.status.contains("back-side"), "{}", c.status);
    }
}

#[cfg(test)]
mod nest_corner_tests {
    use super::*;
    use gcarve_core::geom::make_rect;

    #[test]
    fn nesting_from_the_top_right_packs_into_that_corner() {
        let mut c = Canvas::default();
        c.doc.material.w = 20.0;
        c.doc.material.h = 10.0;
        let a = c.doc.add(make_rect(1.0, 1.0, 3.0, 2.0), None);
        c.sel = BTreeSet::from([a]);
        let undo_to = bounds(&c.doc.find(a).unwrap().c);
        c.nest_selection(0.0, 0.5, 0, 3);
        let b = bounds(&c.doc.find(a).unwrap().c);
        assert!((b.x1 - 19.5).abs() < 0.01 && (b.y1 - 9.5).abs() < 0.01 && (b.w() - 2.0).abs() < 1e-6, "{b:?}");
        c.undo();
        assert_eq!(bounds(&c.doc.find(a).unwrap().c), undo_to);
        // distort: three bars bent along an arch keep their order and end up on the arch
        let mut d = Canvas::default();
        let bars: Vec<i32> = (0..3).map(|k| d.doc.add(make_rect(k as f64 * 2.0, 0.0, k as f64 * 2.0 + 1.5, 1.0), None)).collect();
        let arch: Vec<Pt> = (0..=40).map(|i| { let a = PI - PI * i as f64 / 40.0; pt(20.0 + 6.0 * a.cos(), 6.0 * a.sin()) }).collect();
        let curve = d.doc.add(gcarve_core::geom::make_polyline(&arch, false), None);
        d.sel = bars.iter().copied().chain([curve]).collect();
        d.distort_selection(false);
        assert!(d.status.contains("bent"), "{}", d.status);
        let mid = bounds(&d.doc.find(bars[1]).unwrap().c);
        assert!(mid.y0 > 5.3 && mid.y1 > 6.9 && (((mid.x0 + mid.x1) / 2.0) - 20.0).abs() < 0.5, "{mid:?}"); // on the arch, standing 1 in tall at its top
        if let Ok(out) = std::env::var("GCARVE_DISTORT_SVG") {
            let items: Vec<_> = d.doc.shapes.iter().map(|s| (&s.c, "")).collect();
            std::fs::write(out, gcarve_core::svg::export_svg(&items, 30.0, 10.0)).unwrap();
        }
        // four corners: a 2 x 1 bar into a skewed box keeps to its corners
        let mut q = Canvas::default();
        let bar = q.doc.add(make_rect(0.0, 0.0, 2.0, 1.0), None);
        let quad = q.doc.add(gcarve_core::geom::make_polyline(&[pt(10.0, 0.0), pt(16.0, 1.0), pt(15.0, 5.0), pt(11.0, 4.0)], true), None);
        q.sel = BTreeSet::from([bar, quad]);
        q.distort_four_corners();
        assert!(q.status.contains("four corners"), "{}", q.status);
        let b = bounds(&q.doc.find(bar).unwrap().c);
        assert!((b.x0 - 10.0).abs() < 0.01 && (b.x1 - 16.0).abs() < 0.01 && b.y0.abs() < 0.01 && (b.y1 - 5.0).abs() < 0.01, "{b:?}");
        // rotary layout: a 2-start spiral, 4 in per turn, over 8 in of a 2 in blank = 2 × 2 turns, each piece once round
        let mut rl = Canvas::default();
        rl.doc.material.rotary = Some(2.0);
        rl.doc.material.h = PI * 2.0;
        rl.rotary_layout(2, 4.0, 0.0, 8.0);
        let pieces: Vec<Rect> = rl.sel.iter().map(|id| bounds(&rl.doc.find(*id).unwrap().c)).collect();
        assert_eq!(pieces.len(), 2 * 2 + 1, "{}", rl.status); // the half-turn-offset start is split once more
        assert!(pieces.iter().all(|b| b.y0 >= -1e-9 && b.y1 <= PI * 2.0 + 1e-9 && b.x1 <= 8.0 + 1e-9));
        let along: f64 = pieces.iter().map(|b| b.w()).sum();
        assert!((along - 16.0).abs() < 1e-6, "{along}");
        rl.rotary_layout(6, 0.0, 1.0, 5.0);
        assert_eq!(rl.sel.len(), 6);
        // shelf pins: a 30 x 11.25 side gets two rows centred along its length, 1.26 in apart
        let mut sp = Canvas::default();
        let side = sp.doc.add(make_rect(0.0, 0.0, 30.0, 11.25), None);
        sp.sel = BTreeSet::from([side]);
        sp.shelf_pin_holes(5.0 / 25.4, 32.0 / 25.4, 37.0 / 25.4, 3.0);
        let holes: Vec<Rect> = sp.sel.iter().map(|id| bounds(&sp.doc.find(*id).unwrap().c)).collect();
        assert_eq!(holes.len(), 2 * 20, "{}", sp.status); // 24 in of run between the 3 in margins: 20 holes 32 mm apart
        assert!(holes.iter().all(|b| ((b.y0 + b.y1) / 2.0 - 37.0 / 25.4).abs() < 1e-6 || ((b.y0 + b.y1) / 2.0 - (11.25 - 37.0 / 25.4)).abs() < 1e-6));
        let xs: Vec<f64> = holes.iter().map(|b| (b.x0 + b.x1) / 2.0).collect();
        assert!((xs.iter().cloned().fold(f64::MAX, f64::min) + xs.iter().cloned().fold(f64::MIN, f64::max) - 30.0).abs() < 1e-6, "centred");
        // filler: one 4 x 4 part and a 1 x 1 part on a 6 x 6 board: the gaps take more 1 x 1 copies
        let mut f = Canvas::default();
        f.doc.material.w = 6.0;
        f.doc.material.h = 6.0;
        let big = f.doc.add(make_rect(0.0, 0.0, 4.0, 4.0), None);
        let small = f.doc.add(make_rect(10.0, 0.0, 11.0, 1.0), None);
        f.sel = BTreeSet::from([big, small]);
        f.nest_filler = true;
        let (placed, _) = f.nest_selection(0.0, 0.0, 0, 0);
        assert_eq!(placed, 2);
        // ponytail: exact fits (zero gap, parts flush to each other) are not always found, so not all 19 spots fill
        assert!(f.doc.shapes.len() >= 2 + 8 && f.status.contains("filler"), "{} shapes, {}", f.doc.shapes.len(), f.status);
        assert!(f.doc.shapes.iter().all(|s| { let b = bounds(&s.c); b.x0 >= -1e-6 && b.y0 >= -1e-6 && b.x1 <= 6.0 + 1e-6 && b.y1 <= 6.0 + 1e-6 }));
        let bb = bounds(&f.doc.find(big).unwrap().c);
        assert!(f.doc.shapes.iter().filter(|s| s.id != big).all(|s| { let b = bounds(&s.c); b.x0 >= bb.x1 - 1e-6 || b.x1 <= bb.x0 + 1e-6 || b.y0 >= bb.y1 - 1e-6 || b.y1 <= bb.y0 + 1e-6 }), "a filler sits on the big part");
        // inside an offcut: the triangle stays, the squares land inside it (from the top-right corner too)
        let tri = c.doc.add(gcarve_core::geom::make_polyline(&[pt(10.0, 0.0), pt(20.0, 0.0), pt(20.0, 10.0)], true), None);
        let sq: Vec<i32> = (0..3).map(|k| c.doc.add(make_rect(k as f64 * 2.0, 0.0, k as f64 * 2.0 + 1.0, 1.0), None)).collect();
        let before = bounds(&c.doc.find(tri).unwrap().c);
        c.sel = sq.iter().copied().chain([tri]).collect();
        c.nest_boundary = true;
        for corner in [0, 3] {
            let (placed, _) = c.nest_selection(0.1, 0.0, 0, corner);
            assert_eq!(placed, 3);
            assert_eq!(bounds(&c.doc.find(tri).unwrap().c), before);
            for id in &sq {
                let b = bounds(&c.doc.find(*id).unwrap().c);
                // inside the triangle (10,0) (20,0) (20,10): on the board strip and under its slope y = x - 10
                assert!(b.x0 >= 10.0 - 1e-6 && b.y0 >= -1e-6 && b.x1 <= 20.0 + 1e-6 && b.y1 <= b.x0 - 10.0 + 1e-6, "corner {corner}: {b:?}");
            }
        }
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
    fn a_qr_code_is_its_dark_squares_welded_and_the_inverse_is_the_rest() {
        // shoelace with holes: welded outlines are anticlockwise, their holes clockwise
        let area = |c: &Canvas| c.sel.iter().map(|id| {
            let p = gcarve_core::geom::flatten(&c.doc.find(*id).unwrap().c, 0.001);
            (0..p.len()).map(|i| { let (a, b) = (p[i], p[(i + 1) % p.len()]); a.x * b.y - b.x * a.y }).sum::<f64>() / 2.0
        }).sum::<f64>();
        let qr = qrcodegen::QrCode::encode_text("https://gcarve.app", qrcodegen::QrCodeEcc::Quartile).unwrap();
        let n = qr.size();
        let dark = (0..n).flat_map(|y| (0..n).map(move |x| (x, y))).filter(|&(x, y)| qr.get_module(x, y)).count() as f64;
        let m = 4.0 / n as f64;
        let mut c = Canvas::default();
        c.qr_code("https://gcarve.app", 4.0, false);
        if let Ok(out) = std::env::var("GCARVE_QR_SVG") {
            let b = c.doc.material.clone();
            let paths: String = c.sel.iter().map(|id| {
                let p = gcarve_core::geom::flatten(&c.doc.find(*id).unwrap().c, 0.001);
                format!("M{}Z", p.iter().map(|q| format!("{},{}", q.x, b.h - q.y)).collect::<Vec<_>>().join("L"))
            }).collect();
            std::fs::write(out, format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {} {}' width='1600' height='800'><rect width='100%' height='100%' fill='white'/><path d='{paths}' fill='black' fill-rule='nonzero'/></svg>", b.w, b.h)).unwrap();
        }
        assert!((area(&c) - dark * m * m).abs() < 1e-3, "{} vs {}", area(&c), dark * m * m);
        let mut inv = Canvas::default();
        inv.qr_code("https://gcarve.app", 4.0, true);
        assert!((area(&inv) - (n * n) as f64 * m * m + dark * m * m).abs() < 1e-3, "light squares are the rest");
    }

    #[test]
    fn a_jigsaw_puzzle_lands_centred_on_the_board_and_cuts_cleanly_on_the_line() {
        use gcarve_core::document::{Op, OpType, Side, Tool, ToolType};
        let mut c = Canvas::default();
        (c.doc.material.w, c.doc.material.h, c.doc.material.t) = (14.0, 11.0, 0.25);
        c.jigsaw(12.0, 9.0, 4, 3, 7);
        assert_eq!(c.sel.len(), 2 + 3 + 1);
        let b = c.sel.iter().map(|id| bounds(&c.doc.find(*id).unwrap().c)).fold(gcarve_core::geom::Rect::default(), |mut r, b| { r.add_rect(&b); r });
        assert!((b.x0 - 1.0).abs() < 1e-6 && (b.y1 - 10.0).abs() < 1e-6, "centred: {b:?}");
        c.doc.tools = vec![Tool { name: "1/16".into(), kind: ToolType::EndMill, diameter: 0.0625, stepdown: 0.0625, feed_xy: 60.0, feed_z: 20.0, rpm: 18000, ..Default::default() }];
        c.doc.ops = vec![Op { kind: OpType::Profile, side: Side::On, tool: 0, depth: 0.25, shape_ids: c.sel.iter().copied().collect(), ..Default::default() }];
        let gen = gcarve_core::pipeline::regenerate(&c.doc);
        assert!(gen.warnings.is_empty(), "{:?}", gen.warnings);
        assert!(!gen.paths[0].is_empty());
    }

    #[test]
    fn the_inlay_wizard_puts_the_plug_mirrored_on_its_own_sheet_and_it_fits_the_pocket() {
        // an L: one inside corner, five outside ones
        let l = gcarve_core::geom::make_polyline(&[pt(2.0, 2.0), pt(6.0, 2.0), pt(6.0, 3.0), pt(3.0, 3.0), pt(3.0, 6.0), pt(2.0, 6.0)], true);
        let area = |cs: &[Contour]| cs.iter().map(|c| { let p = flatten(c, 0.001); (0..p.len()).map(|i| { let (a, b) = (p[i], p[(i + 1) % p.len()]); a.x * b.y - b.x * a.y }).sum::<f64>().abs() / 2.0 }).sum::<f64>();
        for flat in [true, false] {
            let mut c = Canvas::default();
            let id = c.doc.add(l.clone(), None);
            c.sel = BTreeSet::from([id]);
            c.inlay(flat, 0.2, if flat { 0.005 } else { 0.1 }, 0.25);
            assert_eq!(c.doc.sheets.len(), 2, "{}", c.status);
            assert_eq!(c.doc.sheet_name(1), "Inlay plug");
            assert_eq!(c.doc.active_sheet, 0, "back on the design's sheet");
            let (pocket, plug) = (c.doc.ops[0].clone(), c.doc.ops[1].clone());
            assert!(pocket.shape_ids.iter().all(|i| c.doc.find(*i).unwrap().sheet == 0) && plug.shape_ids.iter().all(|i| c.doc.find(*i).unwrap().sheet == 1));
            for j in 0..2 {
                c.doc.set_active_sheet(j);
                let gen = gcarve_core::pipeline::regenerate(&c.doc);
                assert!(gen.warnings.is_empty(), "flat {flat} sheet {j}: {:?}", gen.warnings);
                assert!(gen.paths[j as usize].iter().map(|p| p.moves.len()).sum::<usize>() > 0, "flat {flat}: sheet {j} cuts");
            }
            c.doc.set_active_sheet(0);
            if flat {
                let female: Vec<Contour> = pocket.shape_ids.iter().map(|i| c.doc.find(*i).unwrap().c.clone()).collect();
                let plug_c: Vec<Contour> = plug.shape_ids.iter().map(|i| c.doc.find(*i).unwrap().c.clone()).collect();
                // mirrored: the L's foot points left on the plug board; flipped back it lies inside the pocket
                let pb = bounds(&plug_c[0]);
                let fb = bounds(&female[0]);
                let back = Affine { a: -1.0, d: 1.0, e: pb.x0 + pb.x1, ..Default::default() };
                let flipped: Vec<Contour> = plug_c.iter().map(|k| transformed(k, &back)).collect();
                let shift = Affine::translate((fb.x0 + fb.x1) / 2.0 - (pb.x0 + pb.x1) / 2.0, (fb.y0 + fb.y1) / 2.0 - (pb.y0 + pb.y1) / 2.0);
                let flipped: Vec<Contour> = flipped.iter().map(|k| transformed(k, &shift)).collect();
                assert!(area(&boolean_op(BoolOp::Subtract, &flipped, &female)) < 1e-4, "the plug fits inside the pocket");
                let slack = area(&female) - area(&flipped);
                assert!(slack > 0.005 * 15.0 && slack < 0.005 * 25.0, "a fit gap all round: {slack}");
                assert!(plug.depth > pocket.depth && plug.side == gcarve_core::document::Side::Outside);
            } else {
                assert_eq!((plug.start_depth, plug.flat_depth), (0.1, 0.2));
                assert_eq!(plug.shape_ids.len(), 2, "the design and the box round it");
            }
        }
    }

    #[test]
    fn a_living_hinge_fills_the_selected_box_across_and_cuts_on_the_line() {
        use gcarve_core::document::{Op, OpType, Side, Tool, ToolType};
        let mut c = Canvas::default();
        let id = c.doc.add(make_rect(2.0, 2.0, 10.0, 6.0), None);
        c.sel = BTreeSet::from([id]);
        c.sel_changed();
        c.living_hinge(0.25, 1.5, 0.25, false);
        assert!(c.sel.len() > 40, "{}", c.status);
        for id in &c.sel {
            let b = bounds(&c.doc.find(*id).unwrap().c);
            assert!((b.y1 - b.y0).abs() < 1e-9, "across = horizontal slots");
            assert!(b.x0 >= 2.0 - 1e-9 && b.x1 <= 10.0 + 1e-9 && b.y0 > 2.0 && b.y0 < 6.0);
        }
        c.doc.tools = vec![Tool { name: "1/16".into(), kind: ToolType::EndMill, diameter: 0.0625, stepdown: 0.0625, feed_xy: 60.0, feed_z: 20.0, rpm: 18000, ..Default::default() }];
        c.doc.material.t = 0.25;
        c.doc.ops = vec![Op { kind: OpType::Profile, side: Side::On, tool: 0, depth: 0.25, shape_ids: c.sel.iter().copied().collect(), ..Default::default() }];
        let gen = gcarve_core::pipeline::regenerate(&c.doc);
        assert!(gen.warnings.is_empty(), "{:?}", gen.warnings);
    }

    #[test]
    fn the_box_maker_lays_its_panels_flat_apart_on_the_box_layer() {
        let mut c = Canvas::default();
        let spec = gcarve_core::boxjoint::BoxSpec { w: 10.0, d: 6.0, h: 4.0, t: 0.75, finger: 1.5, play: 0.005, lid: true, bit: 0.25 };
        c.box_maker(spec);
        assert_eq!(c.sel.len(), 6);
        let boxes: Vec<_> = c.sel.iter().map(|id| bounds(&c.doc.find(*id).unwrap().c)).collect();
        for (i, a) in boxes.iter().enumerate() {
            assert!(a.x0 >= 0.0 && a.y0 >= 0.0, "on the board");
            for b in &boxes[i + 1..] {
                assert!(a.x1 <= b.x0 || b.x1 <= a.x0 || a.y1 <= b.y0 || b.y1 <= a.y0, "{a:?} overlaps {b:?}");
            }
        }
        let layer = c.doc.find(*c.sel.first().unwrap()).unwrap().layer;
        assert_eq!(c.doc.layers[layer as usize].name, "Box");
        // too small for its board: refused, nothing added
        let n = c.doc.shapes.len();
        c.box_maker(gcarve_core::boxjoint::BoxSpec { h: 1.0, ..spec });
        assert_eq!(c.doc.shapes.len(), n);
    }

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
        // Vector Selector: closed / open / circles / size
        let ring = c.doc.add(gcarve_core::geom::make_ellipse(60.0, 0.0, 2.0, 2.0), None);
        let big = c.doc.add(make_rect(70.0, 0.0, 80.0, 10.0), None);
        c.select_matching(&VecFilter { kind: 3, ..Default::default() });
        assert_eq!(c.sel, BTreeSet::from([ring]));
        c.select_matching(&VecFilter { kind: 2, larger: Some(5.0), ..Default::default() });
        assert_eq!(c.sel, BTreeSet::from([big]));
        c.select_matching(&VecFilter { kind: 1, ..Default::default() });
        assert!(!c.sel.is_empty() && c.sel.iter().all(|id| !c.doc.find(*id).unwrap().c.closed));
        // Fit curves: a 72-point polyline circle becomes a few arc nodes that stay on the circle
        let dense: Vec<Pt> = (0..72).map(|i| { let a = i as f64 * PI / 36.0; pt(90.0 + 3.0 * a.cos(), 3.0 * a.sin()) }).collect();
        let circ = c.doc.add(gcarve_core::geom::make_polyline(&dense, true), None);
        c.sel = BTreeSet::from([circ]);
        c.fit_curves_selection(0.005);
        let fitted = &c.doc.find(*c.sel.iter().next().unwrap()).unwrap().c;
        assert!(fitted.nodes.len() <= 12 && c.status.starts_with("Fit curves: 72"), "{} nodes, {}", fitted.nodes.len(), c.status);
        assert!(flatten(fitted, 0.001).iter().all(|p| ((p.x - 90.0).hypot(p.y) - 3.0).abs() < 0.01));
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

#[cfg(test)]
mod rotary_sample {
    /// GCARVE_ROTARY_OUT=x.tlp: a 4-start barley twist V-fluted on a 3 in blank (rotary 3D view screenshots)
    #[test]
    #[ignore]
    fn rotary_sample() {
        let Ok(out) = std::env::var("GCARVE_ROTARY_OUT") else { return };
        let mut c = crate::canvas::Canvas::default();
        let m = &mut c.doc.material;
        m.rotary = Some(3.0);
        m.w = 12.0;
        m.h = std::f64::consts::PI * 3.0;
        m.t = 1.5;
        c.rotary_layout(4, 5.0, 1.0, 11.0);
        let ids: Vec<i32> = c.sel.iter().copied().collect();
        c.doc.tools.push(gcarve_core::document::Tool { name: "V90".into(), kind: gcarve_core::document::ToolType::VBit, angle: 90.0, diameter: 0.5, stepdown: 0.1, feed_xy: 100.0, feed_z: 30.0, rpm: 18000, ..Default::default() });
        let tool = (c.doc.tools.len() - 1) as i32;
        let mut op = crate::dialogs::op::new_op(&c.doc, gcarve_core::document::OpType::Flute, &ids);
        (op.depth, op.tool) = (0.25, tool);
        c.doc.ops.push(op);
        std::fs::write(out, gcarve_core::project::save_project(&c.doc)).unwrap();
    }
}
