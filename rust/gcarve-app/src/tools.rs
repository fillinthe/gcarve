//! Pointer tools (Qt app: app/canvas_input.cpp): select / move / marquee, node editing, the shape tools, polyline,
//! pen, arc, measure, dimension, text; snapping and hit testing.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::canvas::{Canvas, CtxMenu, Drag, Request, Snap, Tool, View, HIT_PX, NODE_PX, SNAP_PX};
use crate::dialogs::prefs::{SNAP_CENTRES, SNAP_EDGE, SNAP_GRID, SNAP_MIDPOINTS, SNAP_POINTS, SNAP_QUADRANTS};
use crate::widgets::TipExt;
use crate::popups::SizePopup;
use crate::theme;
use crate::units;
use eframe::egui::{self, Pos2, Shape, Stroke};
use gcarve_core::document::Dimension;
use gcarve_core::geom::{self, bounds, dist, make_ellipse, make_polyline, make_rect, nearest, pt, seg_count, split_segment, transformed, Affine, Builder, Contour, Node, Pt};
use gcarve_core::vecops::{make_polygon, make_star};
use std::collections::BTreeSet;
use std::f64::consts::PI;

/// Arc from a through m to b (a straight line if the points are collinear).
pub fn arc_through(a: Pt, m: Pt, b: Pt) -> Contour {
    let d = 2.0 * (a.x * (m.y - b.y) + m.x * (b.y - a.y) + b.x * (a.y - m.y));
    if d.abs() < 1e-12 || dist(a, b) < 1e-12 {
        return make_polyline(&[a, b], false);
    }
    let (a2, m2, b2) = (a.x * a.x + a.y * a.y, m.x * m.x + m.y * m.y, b.x * b.x + b.y * b.y);
    let c = pt((a2 * (m.y - b.y) + m2 * (b.y - a.y) + b2 * (a.y - m.y)) / d, (a2 * (b.x - m.x) + m2 * (a.x - b.x) + b2 * (m.x - a.x)) / d);
    let r = dist(c, a);
    let norm = |v: f64| v.rem_euclid(2.0 * PI);
    let a0 = (a.y - c.y).atan2(a.x - c.x);
    let sb = norm((b.y - c.y).atan2(b.x - c.x) - a0);
    let sm = norm((m.y - c.y).atan2(m.x - c.x) - a0);
    let sweep = if sm < sb { sb } else { sb - 2.0 * PI }; // go the way that passes through m
    let mut bld = Builder::default();
    bld.start(a);
    bld.arc(c.x, c.y, r, r, 0.0, a0, sweep);
    bld.c.nodes.last_mut().unwrap().p = b;
    bld.c
}

fn len(p: Pt) -> f64 {
    p.x.hypot(p.y)
}

/// Rail spin box for the Polygon (sides) / Star (points) tool, 3–64 (Qt ModeRail child column).
pub fn rail_count(ui: &mut eframe::egui::Ui, canvas: &mut Canvas) {
    let (label, v) = match canvas.tool {
        Tool::Polygon => (i18n::t("Sides"), &mut canvas.polygon_sides),
        Tool::Star => (i18n::t("Points"), &mut canvas.star_points),
        _ => return,
    };
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        ui.label(eframe::egui::RichText::new(label).size(11.0));
        let w = ui.available_width();
        ui.add(crate::widgets::SpinBox::new(v, 3..=64).width(w)).tip(format!("{label} of the next shape"));
    });
}

impl Canvas {
    /// A hit / snap / node radius in screen pixels with the Larger handles preference applied.
    pub(crate) fn px(&self, base: f64) -> f64 {
        base * self.hit_scale
    }

    /// Nearest shape node within SNAP_PX, else the grid point (Qt Canvas::snapAt). kind 0 when snapping is off.
    pub fn snap_at(&self, p: Pt, exclude: &BTreeSet<i32>) -> Snap {
        // object snaps: always for Measure / Dimension (they need exact points), else with the Snap switch;
        // the per-kind checkboxes (Snap settings) apply to both
        let objects = self.snap || matches!(self.tool, Tool::Measure | Tool::Dimension);
        let r = self.px(self.snap_prefs.px) / self.zoom();
        // guides pull whenever nothing closer (a point, an edge) takes the pointer, also with Snap off
        let guided = |q: Pt| match self.guide_snap(q, r) {
            (g, true) => Snap { kind: 1, to: g, d: dist(q, g), shape: -1, node: -1, what: "guide" },
            _ => Snap { to: q, ..Default::default() },
        };
        if !objects {
            return guided(p);
        }
        let k = self.snap_prefs.kinds;
        let mut best = Snap { kind: 0, to: p, d: r, shape: -1, node: -1, what: "" };
        let mut edge = best;
        let mut take = |q: Pt, shape: i32, node: i32, what: &'static str| {
            let bit = match what { "point" => SNAP_POINTS, "midpoint" => SNAP_MIDPOINTS, "centre" => SNAP_CENTRES, _ => SNAP_QUADRANTS };
            if k & bit == 0 {
                return;
            }
            let d = dist(q, p);
            if d < best.d || (best.kind == 0 && d <= best.d) {
                // ties keep the first: a node beats the midpoint / quadrant at the same spot
                best = Snap { kind: 2, to: q, d, shape, node, what };
            }
        };
        for s in &self.doc.shapes {
            if exclude.contains(&s.id) || !self.doc.shape_visible(s) {
                continue;
            }
            let b = bounds(&s.c);
            let c = pt((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
            if p.x < b.x0 - r || p.x > b.x1 + r || p.y < b.y0 - r || p.y > b.y1 + r {
                if s.c.closed {
                    take(c, s.id, -1, "centre"); // only the centre can still be near
                }
                continue;
            }
            for (i, n) in s.c.nodes.iter().enumerate() {
                take(n.p, s.id, i as i32, "point");
            }
            for i in 0..seg_count(&s.c) {
                let g = geom::segment(&s.c, i);
                take(if g.curve { geom::cubic_at(&g, 0.5) } else { pt((g.p0.x + g.p1.x) / 2.0, (g.p0.y + g.p1.y) / 2.0) }, s.id, -1, "midpoint");
            }
            if s.c.closed {
                // ponytail: bounds centre / quadrants: exact for circles, ellipses, rectangles; a fair guess otherwise
                take(c, s.id, -1, "centre");
                for q in [pt(b.x1, c.y), pt(c.x, b.y1), pt(b.x0, c.y), pt(c.x, b.y0)] {
                    if nearest(&s.c, q).d < 1e-6 {
                        take(q, s.id, -1, "quadrant"); // only where the outline really passes
                    }
                }
            }
            if k & SNAP_EDGE == 0 {
                continue;
            }
            let h = nearest(&s.c, p);
            if h.d <= edge.d {
                let g = geom::segment(&s.c, h.seg);
                let q = if g.curve { geom::cubic_at(&g, h.t) } else { pt(g.p0.x + (g.p1.x - g.p0.x) * h.t, g.p0.y + (g.p1.y - g.p0.y) * h.t) };
                edge = Snap { kind: 2, to: q, d: h.d, shape: s.id, node: -1, what: "on edge" };
            }
        }
        if best.kind == 2 {
            return best;
        }
        if edge.kind == 2 {
            return edge;
        }
        if !self.snap || k & SNAP_GRID == 0 {
            return guided(p);
        }
        let g = self.grid_step();
        let q = pt((p.x / g).round() * g, (p.y / g).round() * g);
        // a guide near the pointer wins over the grid line on its axis
        let (gq, on_guide) = self.guide_snap(p, r);
        let q = pt(if gq.x != p.x { gq.x } else { q.x }, if gq.y != p.y { gq.y } else { q.y });
        Snap { kind: 1, to: q, d: dist(p, q), shape: -1, node: -1, what: if on_guide { "guide" } else { "" } }
    }
    pub(crate) fn snapped(&self, p: Pt) -> Pt {
        self.snap_at(p, &BTreeSet::new()).to
    }

    /// Topmost editable shape within HIT_PX of a document point.
    pub fn hit_shape(&self, q: Pt) -> Option<i32> {
        let tol = self.px(HIT_PX) / self.zoom();
        let mut bd = tol + 1e-12;
        let mut best = None;
        for s in self.doc.shapes.iter().rev().filter(|s| self.doc.shape_editable(s)) {
            let b = bounds(&s.c);
            if q.x < b.x0 - tol || q.x > b.x1 + tol || q.y < b.y0 - tol || q.y > b.y1 + tol {
                continue;
            }
            let d = nearest(&s.c, q).d;
            if d < bd {
                bd = d;
                best = Some(s.id);
            }
        }
        best
    }

    /// (shape, node, part) under a screen point: part 0 = the point, 1 = in handle, 2 = out handle.
    pub(crate) fn hit_node(&self, s: Pos2) -> Option<(i32, usize, u8)> {
        let view = self.view.as_ref()?;
        let mut bd = self.px(NODE_PX);
        let mut best = None;
        for id in &self.sel {
            let Some(sh) = self.doc.find(*id) else { continue };
            for (i, n) in sh.c.nodes.iter().enumerate() {
                let mut test = |p: Pt, part: u8, bias: f64| {
                    let q = view.to_screen(p);
                    let d = (q - s).length() as f64 - bias;
                    if d <= bd {
                        bd = d;
                        best = Some((*id, i, part));
                    }
                };
                if self.node_shown(*id, i) {
                    if n.has_in {
                        test(n.inh, 1, 2.0);
                    }
                    if n.has_out {
                        test(n.out, 2, 2.0);
                    }
                }
                test(n.p, 0, 0.0);
            }
        }
        best
    }

    pub(crate) fn hit_dimension(&self, s: Pos2) -> Option<usize> {
        let view = self.view.as_ref()?;
        let mut best = self.px(HIT_PX) as f32 + 2.0;
        let mut hit = None;
        for (i, dm) in self.doc.dimensions.iter().enumerate() {
            if dm.axis == 3 {
                // on the arc: right distance from the corner, inside the sweep
                let w = view.to_doc(s);
                let (a0, sweep) = dm.arc();
                let t = (w.y - dm.vertex.y).atan2(w.x - dm.vertex.x);
                let rel = (t - a0).rem_euclid(std::f64::consts::TAU);
                let within = if sweep >= 0.0 { rel <= sweep } else { rel >= std::f64::consts::TAU + sweep };
                let dd = ((dist(w, dm.vertex) - dm.offset.abs()).abs() * f64::from(view.zoom)) as f32;
                if within && dd < best {
                    best = dd;
                    hit = Some(i);
                }
                continue;
            }
            let (ea, eb) = dm.ends();
            let d = eb - ea;
            let l = dist(ea, eb);
            if l < 1e-9 {
                continue;
            }
            let n = pt(-d.y / l, d.x / l);
            let (a2, b2) = (view.to_screen(ea + n * dm.offset), view.to_screen(eb + n * dm.offset));
            let v = b2 - a2;
            let len2 = v.length_sq();
            let t = if len2 > 0.0 { ((s - a2).dot(v) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let dd = (a2 + v * t - s).length();
            if dd < best {
                best = dd;
                hit = Some(i);
            }
        }
        hit
    }

    /// Adds a shape, selects it, records undo.
    pub fn add_shape(&mut self, c: Contour) -> i32 {
        self.doc.checkpoint();
        let id = self.doc.add(c, None);
        self.sel = BTreeSet::from([id]);
        self.mutated();
        id
    }

    /// Size popup result (Qt addSizedShape): a shape centred at (cx, cy), inches.
    pub fn add_sized_shape(&mut self, tool: Tool, cx: f64, cy: f64, w: f64, h: f64) {
        if w <= 0.0 || h <= 0.0 {
            self.status = i18n::t("Width and height must be > 0").into();
            return;
        }
        let c = match tool {
            Tool::Rect => make_rect(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0),
            Tool::Ellipse => make_ellipse(cx, cy, w / 2.0, h / 2.0),
            Tool::Polygon => make_polygon(pt(cx, cy), w / 2.0, self.polygon_sides, 0.0),
            Tool::Star => make_star(pt(cx, cy), w / 2.0, w / 2.0 * self.star_ratio, self.star_points, 0.0),
            _ => return,
        };
        self.add_shape(c);
        self.status = i18n::t("Added shape; use the inspector or Variables to constrain it").into();
    }

    pub(crate) fn open_size_popup(&mut self, at: Pos2, centre: Pt) {
        let w = units::text(&self.doc, 1.0);
        let (cx, cy) = (units::text(&self.doc, centre.x), units::text(&self.doc, centre.y));
        self.size_popup = Some(SizePopup { tool: self.tool, at, w: w.clone(), h: w, cx, cy, lock: self.tool == Tool::Ellipse, fresh: true });
    }

    pub(crate) fn finish_poly(&mut self, close: bool) {
        if self.poly.len() >= 2 {
            let c = make_polyline(&self.poly, close && self.poly.len() >= 3);
            self.add_shape(c);
        }
        self.poly.clear();
    }

    pub(crate) fn finish_pen(&mut self, close: bool) {
        if self.pen.nodes.len() >= 2 {
            let mut c = std::mem::take(&mut self.pen);
            if close && c.nodes.len() >= 3 {
                c.closed = true;
            }
            self.add_shape(c);
        }
        self.pen = Contour::default();
    }

    /// Move the selection by `raw` with the selection's corners and centre snapping (node snaps beat grid snaps).
    fn apply_move(&mut self, raw: Pt) {
        let r = self.move_rect;
        let refs = [pt(r.x0, r.y0), pt(r.x1, r.y0), pt(r.x0, r.y1), pt(r.x1, r.y1), pt((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0)];
        let mut best = Snap::default();
        let mut delta = raw;
        for rf in refs {
            let target = rf + raw;
            let s = self.snap_at(target, &self.sel);
            if s.kind == 0 {
                continue;
            }
            if best.kind == 0 || s.kind > best.kind || (s.kind == best.kind && s.d < best.d) {
                best = s;
                delta = raw + (s.to - target);
            }
        }
        self.snap_mark = (best.kind == 2).then_some(best);
        for o in &self.orig {
            if let Some(s) = self.doc.find_mut(o.id) {
                s.c = transformed(&o.c, &Affine::translate(delta.x, delta.y));
            }
        }
        self.mutated();
    }

    fn apply_node_drag(&mut self, raw: Pt) {
        let mut c = self.orig_c.clone();
        let idx = self.node_idx as usize;
        let o = self.orig_c.nodes[idx];
        let n = &mut c.nodes[idx];
        if self.node_part == 0 {
            let s = self.snap_at(o.p + raw, &BTreeSet::from([self.node_shape]));
            self.snap_mark = (s.kind == 2).then_some(s);
            let d = if s.kind != 0 { s.to - o.p } else { raw };
            n.p = o.p + d;
            n.inh = o.inh + d;
            n.out = o.out + d;
        } else {
            let (h, other) = if self.node_part == 1 { (&mut n.inh, &mut n.out) } else { (&mut n.out, &mut n.inh) };
            *h = (if self.node_part == 1 { o.inh } else { o.out }) + raw;
            let (a, b) = (o.inh - o.p, o.out - o.p);
            let (la, lb) = (len(a), len(b));
            // dragging one handle of a smooth node keeps the other one collinear
            if o.has_in && o.has_out && la > 1e-9 && lb > 1e-9 && (a.x * b.y - a.y * b.x).abs() / (la * lb) < 0.02 && a.x * b.x + a.y * b.y < 0.0 {
                let dir = *h - n.p;
                if len(dir) > 1e-9 {
                    *other = n.p - dir * ((if self.node_part == 1 { lb } else { la }) / len(dir));
                }
            }
        }
        if let Some(s) = self.doc.find_mut(self.node_shape) {
            s.c = c;
        }
        self.mutated();
    }

    fn press_select(&mut self, s: Pos2, mut w: Pt, shift: bool, drag: bool) {
        // all sheets shown: a press on another sheet's board (not on a shape of this one) makes that sheet active
        if self.hit_shape(w).is_none() {
            if let Some(j) = self.other_sheet_at(w) {
                let d = self.switch_sheet_keep_view(j);
                w = w + d;
                if drag {
                    self.press_w = w;
                }
            }
        }
        if let Some(dh) = self.hit_dimension(s) {
            self.sel_dim = dh as i32;
            self.sel.clear();
            self.sel_changed();
            return;
        }
        self.sel_dim = -1;
        match self.hit_shape(w) {
            Some(id) => {
                if shift && self.sel.contains(&id) {
                    let g = self.doc.find(id).map(|s| s.group).unwrap_or(0);
                    self.sel.remove(&id);
                    if g != 0 {
                        let members: Vec<i32> = self.doc.shapes.iter().filter(|sh| sh.group == g).map(|sh| sh.id).collect();
                        for m in members {
                            self.sel.remove(&m);
                        }
                    }
                } else if shift {
                    self.sel.insert(id);
                } else if !self.sel.contains(&id) {
                    self.sel = BTreeSet::from([id]);
                }
                self.sel_changed();
                if drag && self.sel.contains(&id) {
                    self.doc.checkpoint();
                    self.break_constraints_on_move();
                    self.orig = self.sel.iter().filter_map(|i| self.doc.find(*i).cloned()).collect();
                    self.move_rect = self.sel_rect;
                    self.drag = Drag::Move;
                }
            }
            None => {
                if !shift {
                    self.sel.clear();
                }
                // a finger on empty canvas pans (phones and tablets have no right button and no space bar)
                let touch = drag && self.touching;
                if touch {
                    self.touch_pan = true;
                } else if drag && !self.start_relief_drag(w) {
                    self.drag = Drag::Marquee; // a 3D model under the pointer is dragged into place instead
                }
                self.sel_changed();
            }
        }
    }

    fn press_nodes(&mut self, s: Pos2, w: Pt, shift: bool, drag: bool) {
        if let Some((shape, idx, part)) = self.hit_node(s) {
            self.node_shape = shape;
            self.node_idx = idx as i32;
            self.node_part = part;
            if drag {
                self.orig_c = self.doc.find(shape).map(|s| s.c.clone()).unwrap_or_default();
                self.doc.checkpoint();
                self.drag = Drag::Node;
            }
        } else {
            self.node_idx = -1;
            match self.hit_shape(w) {
                Some(id) => {
                    if !shift {
                        self.sel.clear();
                    }
                    self.sel.insert(id);
                    self.node_shape = id;
                }
                None => self.sel.clear(),
            }
            self.sel_changed();
        }
    }

    /// Double-click in the Nodes tool: toggle corner/smooth on a node, or insert a node on a segment.
    fn double_click_nodes(&mut self, s: Pos2, w: Pt) {
        if let Some((shape, idx, 0)) = self.hit_node(s) {
            self.doc.checkpoint();
            let c = &mut self.doc.find_mut(shape).unwrap().c;
            let cnt = c.nodes.len();
            let n = c.nodes[idx];
            if n.has_in || n.has_out {
                c.nodes[idx].has_in = false;
                c.nodes[idx].has_out = false;
            } else if c.closed || (idx > 0 && idx < cnt - 1) {
                let pv = c.nodes[(idx + cnt - 1) % cnt].p;
                let nx = c.nodes[(idx + 1) % cnt].p;
                let dir = nx - pv;
                let dl = len(dir);
                if dl > 1e-12 {
                    let dir = dir * (1.0 / dl);
                    let m = &mut c.nodes[idx];
                    m.inh = m.p - dir * (dist(m.p, pv) / 3.0);
                    m.has_in = true;
                    m.out = m.p + dir * (dist(m.p, nx) / 3.0);
                    m.has_out = true;
                }
            }
            self.node_shape = shape;
            self.node_idx = idx as i32;
            self.mutated();
            return;
        }
        let tol = self.px(HIT_PX) / self.zoom();
        for id in self.sel.clone() {
            let Some(sh) = self.doc.find(id) else { continue };
            let hit = nearest(&sh.c, w);
            if hit.d <= tol && seg_count(&sh.c) > 0 {
                self.doc.checkpoint();
                let sh = self.doc.find_mut(id).unwrap();
                split_segment(&mut sh.c, hit.seg, hit.t.clamp(0.02, 0.98));
                shift_node_binds(&mut sh.bind, hit.seg as i32 + 1, 1);
                self.doc.shift_dimension_nodes(id, hit.seg as i32 + 1, 1);
                self.node_shape = id;
                self.node_idx = hit.seg as i32 + 1;
                self.mutated();
                return;
            }
        }
    }

    /// One frame of pointer input for the current tool; draws previews; returns the status text.
    pub(crate) fn input(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: egui::Rect, painter: &egui::Painter) -> String {
        let view = self.view.as_ref().unwrap();
        let (zoom, origin) = (view.zoom, view.origin);
        let view = View { zoom, origin };
        let (shift, press_origin, hover) = ui.input(|i| (i.modifiers.shift, i.pointer.press_origin(), i.pointer.hover_pos()));
        let pointer = resp.interact_pointer_pos().or(hover);
        let hover_w = pointer.map(|p| view.to_doc(p));
        let hover_snap = hover_w.map(|w| if self.tool.draws() && self.drag == Drag::None { self.snap_at(w, &BTreeSet::new()) } else { Snap { to: w, ..Default::default() } });
        let mut status = self.cursor_text.clone();
        if let Some(sn) = hover_snap {
            // blank until the pointer has been over the laid-out canvas (no "X inf"); the last reading stays after it leaves
            if pointer.is_some_and(|p| rect.contains(p)) && sn.to.x.is_finite() && sn.to.y.is_finite() {
                self.cursor_text = format!("X {}  Y {}", units::text(&self.doc, sn.to.x), units::fmt(&self.doc, sn.to.y));
            }
            status = self.cursor_text.clone();
            if sn.kind == 2 && self.drag == Drag::None {
                self.snap_mark = Some(sn);
            } else if self.drag == Drag::None {
                self.snap_mark = None;
            }
        }
        let primary = egui::PointerButton::Primary;
        let dbl = resp.double_clicked_by(primary);
        let clicked = resp.clicked_by(primary) && !dbl;
        let drag_started = resp.drag_started_by(primary) && !ui.input(|i| i.key_down(egui::Key::Space));
        let press_w = press_origin.map(|p| view.to_doc(p)).unwrap_or_default();
        if drag_started {
            self.press_w = press_w;
        }
        let screen = pointer.unwrap_or_default();

        // right-click without dragging: context menus (popups.rs draws them)
        if resp.secondary_clicked() {
            self.ctx_menu = CtxMenu::Shape;
            if let Some(w) = hover_w {
                self.ctx_at = w;
                if self.tool == Tool::Nodes {
                    if let Some((shape, idx, 0)) = self.hit_node(screen) {
                        self.node_shape = shape;
                        self.node_idx = idx as i32;
                        self.ctx_menu = CtxMenu::Node(shape, idx);
                    }
                }
                if self.ctx_menu == CtxMenu::Shape {
                    if let Some(dh) = self.tool.eq(&Tool::Select).then(|| self.hit_dimension(screen)).flatten() {
                        self.sel_dim = dh as i32;
                        self.sel.clear();
                        self.sel_changed();
                        self.ctx_menu = CtxMenu::Dimension(dh);
                    } else if matches!(self.tool, Tool::Rect | Tool::Ellipse | Tool::Polygon | Tool::Star) {
                        self.ctx_menu = CtxMenu::None;
                        let c = self.snapped(w);
                        self.open_size_popup(screen, c);
                    } else if let Some(id) = self.hit_shape(w) {
                        if !self.sel.contains(&id) {
                            self.sel = BTreeSet::from([id]);
                            self.sel_changed();
                        }
                    }
                }
            }
        }

        // placing tabs by hand (the toolpath dialog is open): a click goes to the dialog, nothing else reacts
        if self.tab_pick > 0 {
            self.tab_pick -= 1;
            if clicked {
                if let Some(w) = hover_w {
                    self.tab_clicks.push(w);
                }
            }
            // drag a tab along the outline (the dialog moves it while the pointer moves)
            if drag_started {
                self.tab_drag = Some((press_w, press_w, false));
            }
            if let (Some(d), Some(w)) = (self.tab_drag.as_mut(), hover_w) {
                d.1 = w;
            }
            if resp.drag_stopped() {
                if let Some(d) = self.tab_drag.as_mut() {
                    d.2 = true;
                }
            }
            return format!("{status}   click the part's outline to add or remove a tab, drag a tab to move it");
        }
        match self.tool {
            Tool::Select => {
                if dbl {
                    if let Some(w) = hover_w {
                        if let Some(dh) = self.hit_dimension(screen) {
                            self.sel_dim = dh as i32;
                            self.sel.clear();
                            self.sel_changed();
                            self.open_dim_popup(dh, screen);
                        } else if let Some(id) = self.hit_shape(w) {
                            let text = self.doc.find(id).map(|s| s.text).unwrap_or(0);
                            self.sel = BTreeSet::from([id]);
                            self.sel_changed();
                            if text != 0 {
                                self.requests.push(Request::EditText(text));
                            } else {
                                self.node_shape = id;
                                self.node_idx = -1;
                                self.set_tool(Tool::Nodes);
                            }
                        }
                    }
                } else if clicked {
                    if let Some(w) = hover_w {
                        self.press_select(screen, w, shift, false);
                    }
                } else if drag_started {
                    match self.hit_handle(press_origin.unwrap_or_default()) {
                        Some(h) => self.start_handle_drag(h, press_w),
                        None => self.press_select(press_origin.unwrap_or_default(), press_w, shift, true),
                    }
                } else if self.drag == Drag::None {
                    if let Some(h) = pointer.filter(|_| resp.hovered()).and_then(|p| self.hit_handle(p)) {
                        ui.ctx().set_cursor_icon(Canvas::handle_cursor(h));
                    }
                }
                match self.drag {
                    Drag::Scale(_) | Drag::Rotate | Drag::Pivot => {
                        if let Some(w) = hover_w {
                            status = self.apply_handle_drag(w, shift);
                        }
                    }
                    Drag::Move => {
                        if let Some(w) = hover_w {
                            self.apply_move(w - self.press_w);
                        }
                    }
                    Drag::ReliefMove => {
                        if let Some(w) = hover_w {
                            self.relief_drag_to(w);
                        }
                    }
                    Drag::Marquee => {
                        // egui forgets the press position on the release frame: the start is kept in press_w
                        if let Some(b) = pointer {
                            let a = view.to_screen(self.press_w);
                            let r = egui::Rect::from_two_pos(a, b);
                            let window = b.x >= a.x;
                            let col = if window { theme::ACCENT } else { theme::ORANGE };
                            painter.rect_stroke(r, 0.0, Stroke::new(1.0, col), egui::StrokeKind::Inside);
                            painter.rect_filled(r, 0.0, col.gamma_multiply(0.15));
                            if resp.drag_stopped() {
                                self.marquee_select(self.press_w, view.to_doc(b));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Tool::Nodes => {
                if dbl {
                    if let Some(w) = hover_w {
                        self.double_click_nodes(screen, w);
                    }
                } else if clicked {
                    if let Some(w) = hover_w {
                        self.press_nodes(screen, w, shift, false);
                    }
                } else if drag_started {
                    self.press_nodes(press_origin.unwrap_or_default(), press_w, shift, true);
                }
                if self.drag == Drag::Node {
                    if let Some(w) = hover_w {
                        self.apply_node_drag(w - self.press_w);
                    }
                }
            }
            Tool::Rect | Tool::Ellipse | Tool::Polygon | Tool::Star => {
                let radial = matches!(self.tool, Tool::Polygon | Tool::Star);
                if clicked {
                    if let Some(w) = hover_w {
                        let c = self.snapped(w);
                        self.open_size_popup(screen, c);
                    }
                } else if drag_started {
                    self.box_a = self.snapped(press_w);
                    self.box_b = self.box_a;
                    self.drag = if radial { Drag::DrawRadial } else { Drag::DrawBox };
                }
                if matches!(self.drag, Drag::DrawBox | Drag::DrawRadial) {
                    if let Some(w) = hover_w {
                        let mut b = self.snapped(w);
                        let a = self.box_a;
                        if shift && !radial {
                            let (dx, dy) = (b.x - a.x, b.y - a.y);
                            let m = dx.abs().max(dy.abs());
                            b = pt(a.x + if dx < 0.0 { -m } else { m }, a.y + if dy < 0.0 { -m } else { m });
                        } else if shift {
                            let r = dist(a, b);
                            let ang = ((b.y - a.y).atan2(b.x - a.x) / (PI / 12.0)).round() * (PI / 12.0);
                            b = pt(a.x + r * ang.cos(), a.y + r * ang.sin());
                        }
                        self.box_b = b;
                    }
                    let preview = self.preview_shape();
                    if let Some(c) = &preview {
                        let pts = self.screen_pts(&view, c);
                        painter.add(Shape::closed_line(pts, Stroke::new(1.0, theme::ACCENT)));
                    }
                    let (a, b) = (self.box_a, self.box_b);
                    status = if radial {
                        format!("R {}", units::fmt(&self.doc, dist(a, b)))
                    } else {
                        format!("W {}  H {}", units::text(&self.doc, (b.x - a.x).abs()), units::fmt(&self.doc, (b.y - a.y).abs()))
                    };
                    if resp.drag_stopped() {
                        self.drag = Drag::None;
                        if let Some(c) = preview {
                            self.add_shape(c);
                        } else {
                            self.open_size_popup(screen, a);
                        }
                    }
                }
            }
            Tool::Polyline => {
                if dbl {
                    self.finish_poly(false);
                } else if clicked {
                    if let Some(w) = hover_w {
                        if self.poly.len() >= 3 && (view.to_screen(self.poly[0]) - screen).length() < self.px(SNAP_PX) as f32 {
                            self.finish_poly(true);
                        } else {
                            let p = self.snapped(w);
                            if self.poly.last().is_none_or(|l| dist(*l, p) > 1e-9) {
                                self.poly.push(p);
                            }
                        }
                    }
                }
                if !self.poly.is_empty() {
                    let mut pts: Vec<Pos2> = self.poly.iter().map(|p| view.to_screen(*p)).collect();
                    if let Some(sn) = hover_snap {
                        pts.push(view.to_screen(sn.to));
                    }
                    painter.add(Shape::line(pts, Stroke::new(1.0, theme::ACCENT)));
                    status = format!("{status}   {} points · click the start or double-click to finish, Delete takes back the last point, Esc cancels", self.poly.len());
                }
            }
            Tool::Pen => {
                if dbl {
                    self.finish_pen(false);
                } else if clicked || drag_started {
                    let w = if drag_started { press_w } else { hover_w.unwrap_or_default() };
                    let near_start = self.pen.nodes.len() >= 2 && (view.to_screen(self.pen.nodes[0].p) - screen).length() < self.px(SNAP_PX) as f32;
                    if near_start && clicked {
                        self.finish_pen(true);
                    } else {
                        self.pen.nodes.push(Node::at(self.snapped(w)));
                        if drag_started {
                            self.drag = Drag::Pen;
                        }
                    }
                }
                if self.drag == Drag::Pen {
                    if let (Some(w), Some(n)) = (hover_w, self.pen.nodes.last_mut()) {
                        n.out = w; // dragging pulls out symmetric handles -> smooth node
                        n.inh = n.p * 2.0 - w;
                        n.has_in = true;
                        n.has_out = true;
                    }
                    if resp.drag_stopped() {
                        self.drag = Drag::None;
                        if self.pen.nodes.len() == 1 {
                            self.pen.nodes[0].has_in = false; // first node: only an out handle matters
                        }
                    }
                }
                if !self.pen.nodes.is_empty() {
                    let mut c = self.pen.clone();
                    if let Some(sn) = hover_snap {
                        if self.drag != Drag::Pen {
                            c.nodes.push(Node::at(sn.to));
                        }
                    }
                    let pts = self.screen_pts(&view, &c);
                    painter.add(Shape::line(pts, Stroke::new(1.0, theme::ACCENT)));
                    for n in &self.pen.nodes {
                        let p = view.to_screen(n.p);
                        painter.circle_stroke(p, 3.0, Stroke::new(1.0, theme::ACCENT));
                        for (has, h) in [(n.has_in, n.inh), (n.has_out, n.out)] {
                            if has {
                                painter.line_segment([p, view.to_screen(h)], Stroke::new(1.0, theme::TEXT_DIM));
                            }
                        }
                    }
                    status = format!("{status}   click = corner, drag = curve; click the start to close, double-click/Enter to finish");
                }
            }
            Tool::Arc => {
                if clicked {
                    if let Some(w) = hover_w {
                        let p = self.snapped(w);
                        if self.arc_pts.len() < 2 {
                            if self.arc_pts.first().is_none_or(|f| dist(*f, p) > 1e-9) {
                                self.arc_pts.push(p);
                            }
                        } else {
                            let c = arc_through(self.arc_pts[0], p, self.arc_pts[1]);
                            self.arc_pts.clear();
                            self.add_shape(c);
                        }
                    }
                }
                if let (Some(sn), false) = (hover_snap, self.arc_pts.is_empty()) {
                    let c = if self.arc_pts.len() == 1 { make_polyline(&[self.arc_pts[0], sn.to], false) } else { arc_through(self.arc_pts[0], sn.to, self.arc_pts[1]) };
                    let pts = self.screen_pts(&view, &c);
                    painter.add(Shape::line(pts, Stroke::new(1.0, theme::ACCENT)));
                    status = format!("{status}   {}", if self.arc_pts.len() == 1 { i18n::t("click the arc's end") } else { i18n::t("click a point the arc passes through") });
                }
            }
            Tool::Freehand => {
                if drag_started {
                    self.freehand = vec![press_w];
                }
                if !self.freehand.is_empty() {
                    if let Some(w) = hover_w {
                        // a point every 2 screen px is plenty: the fit smooths the rest
                        if self.freehand.last().is_some_and(|l| dist(*l, w) * self.zoom() >= 2.0) {
                            self.freehand.push(w);
                        }
                    }
                    let pts: Vec<egui::Pos2> = self.freehand.iter().map(|p| view.to_screen(*p)).collect();
                    painter.add(Shape::line(pts, Stroke::new(1.5, theme::ACCENT)));
                    status = format!("{status}   draw; end where you started to close the shape");
                    if resp.drag_stopped() {
                        let stroke = std::mem::take(&mut self.freehand);
                        if let Some(c) = freehand_contour(&stroke, self.px(SNAP_PX) / self.zoom()) {
                            self.add_shape(c);
                        }
                    }
                }
            }
            Tool::Measure => {
                if clicked {
                    if let Some(w) = hover_w {
                        if self.measure.len() >= 2 {
                            self.measure.clear();
                            self.measure_text.clear();
                        }
                        self.measure.push(self.snapped(w));
                        if self.measure.len() == 2 {
                            let (a, b) = (self.measure[0], self.measure[1]);
                            let d = &self.doc;
                            self.measure_text = format!("Distance {}, ΔX {}, ΔY {}", units::fmt(d, dist(a, b)), units::fmt(d, (b.x - a.x).abs()), units::fmt(d, (b.y - a.y).abs()));
                        }
                    }
                }
                self.measure_seg = None;
                if let Some(&a) = self.measure.first() {
                    let b = self.measure.get(1).copied().or(hover_snap.map(|s| s.to)).unwrap_or(a);
                    self.measure_seg = Some((a, b));
                    painter.add(Shape::dashed_line(&[view.to_screen(a), view.to_screen(b)], Stroke::new(1.0, theme::ORANGE), 4.0, 3.0));
                    painter.circle_filled(view.to_screen(a), 3.0, theme::ORANGE);
                    painter.circle_filled(view.to_screen(b), 3.0, theme::ORANGE);
                    let d = &self.doc;
                    status = if self.measure.len() == 2 { self.measure_text.clone() } else { format!("{status}   Distance {}", units::fmt(d, dist(a, b))) };
                }
            }
            Tool::Dimension => {
                if clicked {
                    if let Some(w) = hover_w {
                        let sn = self.snap_at(w, &BTreeSet::new());
                        if self.dim_pts.len() < 2 {
                            if self.dim_pts.first().is_none_or(|f| dist(*f, sn.to) > 1e-9) {
                                self.dim_pts.push(sn.to);
                                self.dim_anchors.push(if sn.node >= 0 { (sn.shape, sn.node) } else { (-1, -1) });
                            }
                        } else {
                            let (a, b) = (self.dim_pts[0], self.dim_pts[1]);
                            let off = dim_offset(a, b, w);
                            self.doc.checkpoint();
                            let (sa, sb) = (self.dim_anchors[0], self.dim_anchors[1]);
                            self.doc.dimensions.push(Dimension { a, b, offset: off, shape_a: sa.0, node_a: sa.1, shape_b: sb.0, node_b: sb.1, ..Default::default() });
                            self.dim_pts.clear();
                            self.dim_anchors.clear();
                            self.status = format!("Dimension added: {}", units::fmt(&self.doc, dist(a, b)));
                            self.mutated();
                        }
                    }
                }
                if let (Some(&a), Some(hw)) = (self.dim_pts.first(), hover_w) {
                    let b = self.dim_pts.get(1).copied().unwrap_or(hover_snap.map(|s| s.to).unwrap_or(hw));
                    let off = if self.dim_pts.len() == 2 { dim_offset(a, b, hw) } else { 0.5 };
                    self.paint_dimension(painter, &view, &Dimension { a, b, offset: off, ..Default::default() }, true, true);
                    status = format!("{status}   {}", if self.dim_pts.len() == 1 { i18n::t("click the second point") } else { i18n::t("click where the dimension line goes") });
                }
            }
            Tool::Angle => {
                if clicked {
                    if let Some(w) = hover_w {
                        let sn = self.snap_at(w, &BTreeSet::new());
                        if self.dim_pts.len() < 2 {
                            if self.dim_pts.last().is_none_or(|f| dist(*f, sn.to) > 1e-9) {
                                self.dim_pts.push(sn.to);
                                self.dim_anchors.push(if sn.node >= 0 { (sn.shape, sn.node) } else { (-1, -1) });
                            }
                        } else {
                            // corner, first arm, second arm: the arc sits at the first arm's distance (at most 2 in)
                            let (v, a, b) = (self.dim_pts[0], self.dim_pts[1], sn.to);
                            let (sa, sb) = (self.dim_anchors[1], if sn.node >= 0 { (sn.shape, sn.node) } else { (-1, -1) });
                            let r = (dist(v, a).min(dist(v, b)) * 0.6).clamp(0.05, 2.0);
                            self.doc.checkpoint();
                            let d = Dimension { a, b, vertex: v, offset: r, axis: 3, shape_a: sa.0, node_a: sa.1, shape_b: sb.0, node_b: sb.1, ..Default::default() };
                            self.status = format!("Angle added: {:.1}°", d.degrees());
                            self.doc.dimensions.push(d);
                            self.dim_pts.clear();
                            self.dim_anchors.clear();
                            self.mutated();
                        }
                    }
                }
                if let (Some(&v), Some(hw)) = (self.dim_pts.first(), hover_w) {
                    let to = hover_snap.map(|s| s.to).unwrap_or(hw);
                    if self.dim_pts.len() == 1 {
                        painter.add(Shape::dashed_line(&[view.to_screen(v), view.to_screen(to)], Stroke::new(1.0, theme::ACCENT), 4.0, 3.0));
                    } else {
                        let a = self.dim_pts[1];
                        let r = (dist(v, a).min(dist(v, to)) * 0.6).clamp(0.05, 2.0);
                        self.paint_dimension(painter, &view, &Dimension { a, b: to, vertex: v, offset: r, axis: 3, ..Default::default() }, true, true);
                    }
                    status = format!("{status}   {}", if self.dim_pts.len() == 1 { i18n::t("click a point on the first arm") } else { i18n::t("click a point on the second arm") });
                } else {
                    status = format!("{status}   click the corner of the angle");
                }
            }
            Tool::Text => {
                if clicked {
                    self.requests.push(Request::Text(false));
                }
                status = format!("{status}   click to place text");
            }
        }
        if resp.drag_stopped() {
            self.touch_pan = false;
        }
        if resp.drag_stopped() && matches!(self.drag, Drag::Move | Drag::Node | Drag::Marquee | Drag::ReliefMove | Drag::Scale(_) | Drag::Rotate | Drag::Pivot) {
            if matches!(self.drag, Drag::Scale(_) | Drag::Rotate) {
                let m = self.xf_last;
                self.texts_follow(&m);
            }
            // all sheets shown: a selection dropped on another sheet's board moves to that sheet
            if self.drag == Drag::Move && self.drop_on_other_sheet().is_some() {
                self.gen_stale = false;
            }
            self.drag = Drag::None;
            self.snap_mark = None;
            if self.gen_stale {
                self.gen_stale = false;
                self.touch();
            }
        }
        if !matches!(self.tool, Tool::Select | Tool::Nodes) && resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let _ = rect;
        status // Qt cursorText: the pointer only (the inspector shows the selection size)
    }

    /// The shape being dragged out (None while too small to count as a drag, i.e. a click).
    fn preview_shape(&self) -> Option<Contour> {
        let (a, b) = (self.box_a, self.box_b);
        let zoom = self.zoom();
        match self.tool {
            Tool::Polygon | Tool::Star => {
                let r = dist(a, b);
                let rot = (b.y - a.y).atan2(b.x - a.x) - PI / 2.0;
                (r * zoom >= 3.0).then(|| if self.tool == Tool::Polygon { make_polygon(a, r, self.polygon_sides, rot) } else { make_star(a, r, r * self.star_ratio, self.star_points, rot) })
            }
            _ => {
                let (w, h) = ((b.x - a.x).abs(), (b.y - a.y).abs());
                (w * zoom >= 3.0 && h * zoom >= 3.0).then(|| if self.tool == Tool::Rect { make_rect(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)) } else { make_ellipse((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, w / 2.0, h / 2.0) })
            }
        }
    }

    /// Inspector / node popup: node position in inches.
    pub fn node_at(&self, shape: i32, node: i32) -> Option<Pt> {
        self.doc.find(shape).and_then(|s| usize::try_from(node).ok().and_then(|i| s.c.nodes.get(i))).map(|n| n.p)
    }

    /// Node tool context menu: remove the node, or toggle corner/smooth (`convert`).
    pub fn remove_node(&mut self, shape: i32, node: i32) {
        let Some(s) = self.doc.find(shape) else { return };
        if node < 0 || node as usize >= s.c.nodes.len() {
            return;
        }
        self.doc.checkpoint();
        let s = self.doc.find_mut(shape).unwrap();
        geom::remove_node(&mut s.c, node as usize);
        shift_node_binds(&mut s.bind, node, -1);
        let n = s.c.nodes.len();
        if n == 2 {
            s.c.closed = false;
        }
        self.doc.shift_dimension_nodes(shape, node, -1);
        if n < 2 {
            self.doc.shapes.retain(|s| s.id != shape);
            self.sel.remove(&shape);
        }
        self.node_idx = -1;
        self.mutated();
    }

    pub fn insert_node_after(&mut self, shape: i32, node: i32) {
        let Some(s) = self.doc.find(shape) else { return };
        if node < 0 || node as usize >= seg_count(&s.c) {
            return;
        }
        self.doc.checkpoint();
        let s = self.doc.find_mut(shape).unwrap();
        split_segment(&mut s.c, node as usize, 0.5);
        shift_node_binds(&mut s.bind, node + 1, 1);
        self.doc.shift_dimension_nodes(shape, node + 1, 1);
        self.node_shape = shape;
        self.node_idx = node + 1;
        self.mutated();
    }

    /// Node tool ▸ right-click ▸ Make Start Point: the closed shape's node order is rotated so this node is the
    /// first one (profiles with "Start at the vector's first node" begin their cut there).
    pub fn make_start_point(&mut self, shape: i32, node: i32) {
        let Some(s) = self.doc.find(shape) else { return };
        if !s.c.closed || node <= 0 || node as usize >= s.c.nodes.len() {
            self.status = if s.c.closed && node == 0 { i18n::t("That node already is the start point.").into() } else { i18n::t("Start points are for closed shapes: pick a node of one.").into() };
            return;
        }
        // node-level variables and dimensions are numbered by node: rotating would point them at other nodes
        if s.bind.keys().any(|k| k.as_bytes().first().is_some_and(|b| matches!(b, b'n' | b's')) && k.contains('.')) || self.doc.dimensions.iter().any(|d| d.shape_a == shape || d.shape_b == shape) {
            self.status = i18n::t("This shape has node bindings or dimensions on its nodes: remove them first, then set the start point.").into();
            return;
        }
        self.doc.checkpoint();
        self.doc.find_mut(shape).unwrap().c.nodes.rotate_left(node as usize);
        self.node_idx = 0;
        self.mutated();
        self.status = i18n::t("Start point set: toolpaths with \"Start at the vector's first node\" begin here.").into();
    }

    pub fn toggle_node_smooth(&mut self, shape: i32, node: i32) {
        let Some(p) = self.node_at(shape, node) else { return };
        let view = self.view.as_ref().unwrap();
        let s = view.to_screen(p);
        self.sel.insert(shape);
        self.double_click_nodes(s, p);
    }
}

/// signed distance of q from the line a-b along the left normal
fn dim_offset(a: Pt, b: Pt, q: Pt) -> f64 {
    let d = b - a;
    let l = dist(a, b);
    if l < 1e-12 {
        return 0.5;
    }
    let n = pt(-d.y / l, d.x / l);
    (q.x - a.x) * n.x + (q.y - a.y) * n.y
}

/// Keeps `n<i>.x` / `s<i>.len` bindings pointing at the same nodes after an insert (+1) or remove (-1) at `at`.
pub fn shift_node_binds(bind: &mut gcarve_core::document::Bind, at: i32, delta: i32) {
    let old = std::mem::take(bind);
    for (k, e) in old {
        let kind = k.as_bytes().first().copied();
        let Some(dot) = k.find('.') else { bind.insert(k, e); continue };
        if !matches!(kind, Some(b'n') | Some(b's')) || dot < 2 || !k.as_bytes()[1].is_ascii_digit() {
            bind.insert(k, e);
            continue;
        }
        let Ok(mut i) = k[1..dot].parse::<i32>() else { bind.insert(k, e); continue };
        if delta < 0 && i == at {
            continue; // the node was removed: drop its bindings
        }
        if i >= at {
            i += delta;
        }
        bind.insert(format!("{}{}{}", k.as_bytes()[0] as char, i, &k[dot..]), e);
    }
}

/// a freehand stroke as a vector: closed when it ends within `close` of its start, straight runs thinned, round
/// runs fitted with arcs (0.01 in); None for a stroke too short to keep
pub(crate) fn freehand_contour(stroke: &[Pt], close: f64) -> Option<gcarve_core::geom::Contour> {
    if stroke.len() < 3 {
        return None;
    }
    let closed = stroke.len() > 8 && dist(stroke[0], stroke[stroke.len() - 1]) <= close;
    let raw = make_polyline(if closed { &stroke[..stroke.len() - 1] } else { stroke }, closed);
    let thin = gcarve_core::vecops::simplify_contours(&[raw], 0.0025).pop()?;
    let c = gcarve_core::vecops::refit_arcs(&thin, 0.01);
    (gcarve_core::geom::bounds(&c).w().max(gcarve_core::geom::bounds(&c).h()) > 0.01).then_some(c)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_freehand_loop_closes_into_a_few_curves() {
        use gcarve_core::geom::{bounds, pt};
        // a hand-drawn circle of 2 in radius, 200 points with a little wobble, ending just short of the start
        let stroke: Vec<_> = (0..200).map(|i| { let a = i as f64 / 199.0 * 6.2; let r = 2.0 + 0.003 * (i as f64 * 1.7).sin(); pt(r * a.cos(), r * a.sin()) }).collect();
        let c = super::freehand_contour(&stroke, 0.3).unwrap();
        let b = bounds(&c);
        assert!(c.closed && c.nodes.len() < 40 && (b.w() - 4.0).abs() < 0.1, "{} nodes, {b:?}", c.nodes.len());
        // an open wiggle stays open; a click (two points) is no shape
        let line: Vec<_> = (0..50).map(|i| pt(i as f64 * 0.1, 0.0)).collect();
        assert!(!super::freehand_contour(&line, 0.3).unwrap().closed);
        assert!(super::freehand_contour(&line[..2], 0.3).is_none());
    }

    use super::*;
    use gcarve_core::geom::make_rect;

    #[test]
    fn snapping_nodes_then_grid() {
        let mut c = Canvas { view: Some(View { zoom: 200.0, origin: Pos2::ZERO }), ..Default::default() };
        c.doc.add(make_rect(1.0, 1.0, 3.0, 2.0), None);
        assert_eq!(c.snap_at(pt(1.01, 1.01), &BTreeSet::new()).kind, 0); // snapping off
        c.snap = true;
        let s = c.snap_at(pt(1.01, 1.01), &BTreeSet::new());
        assert!(s.kind == 2 && s.to == pt(1.0, 1.0) && s.shape == 1 && s.node == 0);
        let s = c.snap_at(pt(2.43, 1.52), &BTreeSet::new());
        assert!(s.kind == 1 && (s.to.x - 2.4).abs() < 1e-12 && (s.to.y - 1.5).abs() < 1e-12, "{s:?}"); // 0.1 in grid at 200 px/in
        assert_eq!(c.snap_at(pt(1.01, 1.01), &BTreeSet::from([1])).kind, 1); // excluded shape
        // object snaps: midpoint, centre, on an edge
        let s = c.snap_at(pt(2.01, 1.99), &BTreeSet::new());
        assert!(s.what == "midpoint" && s.to == pt(2.0, 2.0) && s.node == -1, "{s:?}");
        let s = c.snap_at(pt(2.02, 1.51), &BTreeSet::new());
        assert!(s.what == "centre" && s.to == pt(2.0, 1.5), "{s:?}");
        let s = c.snap_at(pt(1.4, 1.01), &BTreeSet::new());
        assert!(s.what == "on edge" && (s.to.y - 1.0).abs() < 1e-12 && (s.to.x - 1.4).abs() < 1e-9, "{s:?}");
        // circle: quadrants and centre, and always on for the dimension tool even with Snap off
        c.doc.add(make_ellipse(5.0, 5.0, 1.0, 1.0), None);
        c.snap = false;
        c.tool = Tool::Dimension;
        let s = c.snap_at(pt(6.01, 5.02), &BTreeSet::new());
        assert!(s.kind == 2 && (s.to.x - 6.0).abs() < 1e-9 && (s.to.y - 5.0).abs() < 1e-9, "{s:?}");
        let s = c.snap_at(pt(5.01, 4.99), &BTreeSet::new());
        assert!(s.what == "centre" && (s.to.x - 5.0).abs() < 1e-9, "{s:?}");
        c.tool = Tool::Select;
        c.snap = true;
        assert_eq!(c.hit_shape(pt(2.0, 1.02)), Some(1));
        assert_eq!(c.hit_shape(pt(2.0, 1.5)), None);
    }

    #[test]
    fn snap_kinds_distance_and_fixed_grid() {
        use crate::dialogs::prefs::*;
        let mut c = Canvas { view: Some(View { zoom: 200.0, origin: Pos2::ZERO }), snap: true, ..Default::default() };
        c.doc.add(make_rect(1.0, 1.0, 3.0, 2.0), None);
        let at = |c: &Canvas, x, y| c.snap_at(pt(x, y), &BTreeSet::new());
        // points off: the corner falls through to the edge, midpoints off: to the edge too
        c.snap_prefs.kinds = SNAP_ALL & !SNAP_POINTS;
        assert_eq!(at(&c, 1.01, 1.01).what, "on edge");
        c.snap_prefs.kinds = SNAP_ALL & !SNAP_MIDPOINTS;
        assert_eq!(at(&c, 2.01, 1.99).what, "quadrant", "a rectangle's edge midpoint is also a quadrant");
        c.snap_prefs.kinds = SNAP_ALL & !SNAP_MIDPOINTS & !SNAP_QUADRANTS;
        assert_eq!(at(&c, 2.01, 1.99).what, "on edge");
        c.snap_prefs.kinds = SNAP_ALL & !SNAP_CENTRES;
        assert_eq!(at(&c, 2.02, 1.51).kind, 1, "centre off: grid");
        c.snap_prefs.kinds = SNAP_ALL & !SNAP_EDGE;
        assert_eq!(at(&c, 1.4, 1.01).kind, 1, "edge off: grid");
        c.snap_prefs.kinds = SNAP_ALL & !SNAP_GRID;
        assert_eq!(at(&c, 2.43, 1.52).kind, 0, "grid off: free point");
        // Measure keeps object snaps with Snap off, but never the grid; the boxes still apply
        c.snap = false;
        c.tool = Tool::Measure;
        c.snap_prefs.kinds = SNAP_ALL;
        assert_eq!(at(&c, 1.01, 1.01).what, "point");
        assert_eq!(at(&c, 2.43, 1.52).kind, 0);
        c.snap_prefs.kinds = SNAP_ALL & !SNAP_POINTS & !SNAP_EDGE;
        assert_eq!(at(&c, 1.01, 1.01).kind, 0);
        // snap distance in px: 8 px = 0.04 in at 200 px/in, 2 px = 0.01 in
        c.snap_prefs.kinds = SNAP_ALL;
        assert_eq!(at(&c, 1.03, 1.0).what, "point");
        c.snap_prefs.px = 2.0;
        assert_eq!(at(&c, 1.03, 1.0).what, "on edge");
        // fixed grid spacing (inches)
        c.tool = Tool::Select;
        c.snap = true;
        c.grid.spacing = 0.25;
        let s = at(&c, 2.4, 1.6);
        assert!(s.kind == 1 && (s.to.x - 2.5).abs() < 1e-12 && (s.to.y - 1.5).abs() < 1e-12, "{s:?}");
    }

    #[test]
    fn arc_and_node_binds() {
        let c = arc_through(pt(0.0, 0.0), pt(1.0, 1.0), pt(2.0, 0.0));
        let b = bounds(&c);
        assert!((b.y1 - 1.0).abs() < 1e-3 && (b.w() - 2.0).abs() < 1e-6);
        let mut bind = gcarve_core::document::Bind::new();
        bind.insert("n2.x".into(), "a".into());
        bind.insert("s3.len".into(), "b".into());
        bind.insert("w".into(), "c".into());
        shift_node_binds(&mut bind, 2, 1);
        assert!(bind.contains_key("n3.x") && bind.contains_key("s4.len") && bind.contains_key("w"));
        shift_node_binds(&mut bind, 3, -1);
        assert!(!bind.contains_key("n3.x") && bind.contains_key("s3.len"));
    }
}
