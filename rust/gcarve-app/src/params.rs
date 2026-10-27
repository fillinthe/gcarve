//! Parametrics on the canvas (Qt app: canvas_param.cpp): variables, bindings of the selection / nodes, constraints.
//! Values typed here are project-unit expressions; anything naming a variable stays bound to it.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::canvas::Canvas;
use crate::units;
use gcarve_core::document::{Bind, Constraint, Document, Variable};
use gcarve_core::expr::{identifiers, rename_ident};
use gcarve_core::geom::{dist, pt, transformed, Affine};

/// Node popup data (Qt Canvas::nodeAt): position, length to the next node, bound expressions ("" = none).
pub struct NodeInfo {
    pub x: f64,
    pub y: f64,
    pub len: f64,
    pub has_next: bool,
    pub bx: String,
    pub by: String,
    pub blen: String,
}

fn axis_of(kind: &str) -> char {
    if matches!(kind, "above" | "below" | "align-top" | "align-bottom" | "center-y") { 'y' } else { 'x' }
}

/// Human name of a bind key (Qt variableUsage fieldNames)
fn field_name(k: &str) -> String {
    let fixed = [("x", "X"), ("y", "Y"), ("w", "W"), ("h", "H"), ("depth", "depth"), ("stepdown", i18n::t("pass depth")), ("tabWidth", i18n::t("tab width")), ("tabHeight", i18n::t("tab height")), ("allowance", "allowance"), ("peck", "peck"), ("flatDepth", i18n::t("flat depth")), ("t", "thickness"), ("safeZ", i18n::t("safe Z")), ("retractZ", i18n::t("retract Z"))];
    if let Some((_, n)) = fixed.iter().find(|(key, _)| *key == k) {
        return n.to_string();
    }
    match (k.as_bytes().first(), k.split_once('.')) {
        (Some(b'n'), Some((i, a))) => format!("node {} {}", &i[1..], a.to_uppercase()),
        (Some(b's'), Some((i, _))) => format!("segment {} length", &i[1..]),
        _ => k.to_string(),
    }
}

/// Where a variable is used: "W of shape 3", "depth of Pocket 1", "board thickness", "variable gap"…
pub fn variable_usage(doc: &Document, name: &str) -> Vec<String> {
    let refs = |e: &str| identifiers(e).iter().any(|i| i == name);
    let mut uses = Vec::new();
    for s in &doc.shapes {
        uses.extend(s.bind.iter().filter(|(_, e)| refs(e)).map(|(k, _)| format!("{} of shape {}", field_name(k), s.id)));
    }
    for op in &doc.ops {
        uses.extend(op.bind.iter().filter(|(_, e)| refs(e)).map(|(k, _)| format!("{} of {}", field_name(k), op.name)));
    }
    uses.extend(doc.material.bind.iter().filter(|(_, e)| refs(e)).map(|(k, _)| format!("board {}", field_name(k))));
    uses.extend(doc.constraints.iter().filter(|c| refs(&c.expr)).map(|c| format!("constraint {} → {}", c.target, c.anchor)));
    uses.extend(doc.dimensions.iter().enumerate().filter(|(_, d)| refs(&d.drive)).map(|(i, _)| format!("dimension {}", i + 1)));
    uses.extend(doc.variables.iter().filter(|v| v.name != name && refs(&v.expr)).map(|v| format!("variable {}", v.name)));
    uses
}

impl Canvas {
    /// Re-evaluates every binding (after variable, board or sheet changes and on load).
    pub fn apply_bindings(&mut self) {
        self.doc.apply_bindings();
    }

    fn eval_or_status(&mut self, text: &str) -> Option<f64> {
        match units::parse(&self.doc, text) {
            Ok(v) => Some(v),
            Err(e) => {
                self.status = format!("Can't evaluate \"{}\": {e}", text.trim());
                None
            }
        }
    }

    /// x / y / w / h bindings every selected shape shares (inspector shows the expression instead of the value).
    pub fn selection_bindings(&self) -> Bind {
        let mut it = self.sel.iter().filter_map(|id| self.doc.find(*id));
        let Some(first) = it.next() else { return Bind::new() };
        let mut m = first.bind.clone();
        for s in it {
            m.retain(|k, e| s.bind.get(k) == Some(e));
        }
        m
    }

    /// Inspector X/Y/W/H (Qt setSelectionField): a variable expression binds the field on every selected shape,
    /// a plain value unbinds and places / resizes (W/H scale the other axis too with `lock_aspect`).
    /// the anchor every selected shape shares; 0 (bottom-left) when they differ
    pub fn selection_anchor(&self) -> i32 {
        let mut it = self.sel.iter().filter_map(|id| self.doc.find(*id)).map(|s| s.anchor);
        let first = it.next().unwrap_or(0);
        if it.all(|a| a == first) { first } else { 0 }
    }

    /// Properties › Grows from: the anchor of every selected shape
    pub fn set_selection_anchor(&mut self, anchor: i32) {
        if self.sel.is_empty() || self.selection_anchor() == anchor {
            return;
        }
        self.doc.checkpoint();
        for id in self.sel.clone() {
            if let Some(s) = self.doc.find_mut(id) {
                s.anchor = anchor.clamp(0, 8);
            }
        }
        self.status = format!("Sizes now grow from the {}", gcarve_core::document::ANCHOR_NAMES[anchor.clamp(0, 8) as usize]);
        self.mutated();
    }

    pub fn set_selection_field(&mut self, prop: &str, text: &str, lock_aspect: bool) {
        let r = self.sel_rect;
        if self.sel.is_empty() || r.is_empty() || !matches!(prop, "x" | "y" | "w" | "h") {
            return;
        }
        let expr = text.trim();
        let Some(v) = self.eval_or_status(expr) else { return };
        self.doc.checkpoint();
        if !identifiers(expr).is_empty() {
            for id in self.sel.clone() {
                if let Some(s) = self.doc.find_mut(id) {
                    s.bind.insert(prop.into(), expr.into());
                }
            }
            self.apply_bindings();
            self.status = format!("{} bound to \"{expr}\"", prop.to_uppercase());
        } else {
            for id in self.sel.clone() {
                if let Some(s) = self.doc.find_mut(id) {
                    s.bind.remove(prop);
                }
            }
            let (mut x, mut y, mut w, mut h) = (r.x0, r.y0, r.w(), r.h());
            match prop {
                "x" => x = v,
                "y" => y = v,
                "w" => w = v,
                _ => h = v,
            }
            if lock_aspect && prop == "w" && r.w() > 0.0 {
                h = r.h() * w / r.w();
            }
            if lock_aspect && prop == "h" && r.h() > 0.0 {
                w = r.w() * h / r.h();
            }
            // a typed size grows away from the selection's anchor (the shapes' shared one; mixed = bottom-left)
            let (fx, fy) = gcarve_core::document::anchor_frac(self.selection_anchor());
            x += fx * (r.w() - w);
            y += fy * (r.h() - h);
            let sx = if r.w() > 0.0 && w > 0.0 { w / r.w() } else { 1.0 };
            let sy = if r.h() > 0.0 && h > 0.0 { h / r.h() } else { 1.0 };
            let m = Affine::translate(x, y) * Affine::scale(sx, sy) * Affine::translate(-r.x0, -r.y0);
            for id in self.sel.clone() {
                if let Some(s) = self.doc.find_mut(id) {
                    s.c = transformed(&s.c, &m);
                }
            }
        }
        self.mutated();
    }

    pub fn node_info(&self, shape: i32, node: i32) -> Option<NodeInfo> {
        let s = self.doc.find(shape)?;
        let i = usize::try_from(node).ok().filter(|&i| i < s.c.nodes.len())?;
        let n = s.c.nodes.len();
        let has_next = s.c.closed || i + 1 < n;
        let b = |k: String| s.bind.get(&k).cloned().unwrap_or_default();
        let p = s.c.nodes[i].p;
        Some(NodeInfo { x: p.x, y: p.y, len: if has_next { dist(p, s.c.nodes[(i + 1) % n].p) } else { 0.0 }, has_next, bx: b(format!("n{i}.x")), by: b(format!("n{i}.y")), blen: b(format!("s{i}.len")) })
    }

    /// Node popup (Qt setNodeField): prop x | y | len. A variable expression binds (n<i>.x, s<i>.len), a value moves
    /// the node (a length moves the next node along the segment once).
    pub fn set_node_field(&mut self, shape: i32, node: i32, prop: &str, text: &str) {
        let Some(info) = self.node_info(shape, node) else { return };
        let expr = text.trim();
        let Some(v) = self.eval_or_status(expr) else { return };
        let cur = match prop { "x" => info.x, "y" => info.y, _ => info.len };
        let bound = match prop { "x" => &info.bx, "y" => &info.by, _ => &info.blen };
        if bound.is_empty() && identifiers(expr).is_empty() && (v - cur).abs() < 1e-9 {
            return; // unchanged value
        }
        let key = if prop == "len" { format!("s{node}.len") } else { format!("n{node}.{prop}") };
        self.doc.checkpoint();
        let s = self.doc.find_mut(shape).unwrap();
        if !identifiers(expr).is_empty() {
            s.bind.insert(key, expr.into());
            self.status = format!("Node {node} {} bound to \"{expr}\"", prop.to_uppercase());
        } else if prop == "len" {
            s.bind.insert(key.clone(), expr.into()); // a one-off: apply, then drop the binding
            self.apply_bindings();
            self.doc.find_mut(shape).unwrap().bind.remove(&key);
            self.status = format!("Node {node} LEN = {expr}");
        } else {
            s.bind.remove(&key);
            let nd = &mut s.c.nodes[node as usize];
            let d = if prop == "x" { pt(v - nd.p.x, 0.0) } else { pt(0.0, v - nd.p.y) };
            nd.p = nd.p + d;
            nd.inh = nd.inh + d;
            nd.out = nd.out + d;
            self.status = format!("Node {node} {} = {expr}", prop.to_uppercase());
        }
        self.node_shape = shape;
        self.node_idx = node;
        self.apply_bindings();
        self.mutated();
    }

    pub fn set_variable(&mut self, name: &str, expr: &str) {
        let n = name.trim();
        if n.is_empty() {
            return;
        }
        self.doc.checkpoint();
        match self.doc.find_variable_mut(n) {
            Some(v) => v.expr = expr.trim().into(),
            None => self.doc.variables.push(Variable { name: n.into(), expr: expr.trim().into() }),
        }
        self.apply_bindings();
        self.mutated();
    }

    /// Renames the variable and every expression that uses it.
    pub fn rename_variable(&mut self, from: &str, to: &str) {
        let to = to.trim();
        if from == to || to.is_empty() || self.doc.find_variable(from).is_none() || self.doc.find_variable(to).is_some() {
            return;
        }
        self.doc.checkpoint();
        let d = &mut self.doc;
        d.find_variable_mut(from).unwrap().name = to.into();
        let ren = |e: &mut String| *e = rename_ident(e, from, to);
        d.variables.iter_mut().for_each(|v| ren(&mut v.expr));
        d.shapes.iter_mut().flat_map(|s| s.bind.values_mut()).for_each(ren);
        d.ops.iter_mut().flat_map(|o| o.bind.values_mut()).for_each(ren);
        d.material.bind.values_mut().for_each(ren);
        d.constraints.iter_mut().for_each(|c| ren(&mut c.expr));
        d.dimensions.iter_mut().for_each(|dm| ren(&mut dm.drive));
        self.mutated();
    }

    pub fn remove_variable(&mut self, name: &str) {
        self.doc.checkpoint();
        self.doc.variables.retain(|v| v.name != name);
        self.mutated();
    }

    /// "var1 = 1", "var2 = 1"…; returns the name.
    pub fn add_variable(&mut self) -> String {
        let mut k = self.doc.variables.len() + 1;
        while self.doc.find_variable(&format!("var{k}")).is_some() {
            k += 1;
        }
        let n = format!("var{k}");
        self.doc.checkpoint();
        self.doc.variables.push(Variable { name: n.clone(), expr: "1".into() });
        self.mutated();
        n
    }

    /// One positional constraint per axis per target: a new one replaces the old one on the same axis.
    pub fn add_constraint(&mut self, anchor: i32, target: i32, kind: &str, expr: &str) -> bool {
        if self.doc.find(anchor).is_none() || self.doc.find(target).is_none() || anchor == target {
            self.status = i18n::t("Select two shapes to constrain.").into();
            return false;
        }
        if !Document::constraint_types().iter().any(|(k, _)| *k == kind) {
            return false;
        }
        let e = expr.trim();
        if !e.is_empty() && self.eval_or_status(e).is_none() {
            return false;
        }
        self.doc.checkpoint();
        self.doc.constraints.retain(|c| !(c.target == target && axis_of(&c.kind) == axis_of(kind)));
        self.doc.constraints.push(Constraint { anchor, target, kind: kind.into(), expr: e.into() });
        self.status = format!("Constrained: shape {target} {kind} shape {anchor}");
        self.mutated();
        true
    }

    /// kind "" keeps the relation.
    pub fn update_constraint(&mut self, i: usize, kind: &str, expr: &str) {
        if i >= self.doc.constraints.len() {
            return;
        }
        let e = expr.trim();
        if !e.is_empty() && self.eval_or_status(e).is_none() {
            return;
        }
        self.doc.checkpoint();
        let c = &mut self.doc.constraints[i];
        if !kind.is_empty() {
            c.kind = kind.into();
        }
        c.expr = e.into();
        self.mutated();
    }

    pub fn remove_constraint(&mut self, i: usize) {
        if i < self.doc.constraints.len() {
            self.doc.checkpoint();
            self.doc.constraints.remove(i);
            self.mutated();
        }
    }

    pub fn select_constraint_shapes(&mut self, i: usize) {
        if let Some(c) = self.doc.constraints.get(i) {
            self.sel = [c.anchor, c.target].into();
            self.sel_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_typed_width_grows_from_the_selection_anchor() {
        let mut c = Canvas::default();
        let id = c.doc.add(gcarve_core::geom::make_rect(2.0, 2.0, 4.0, 3.0), None);
        c.sel = std::collections::BTreeSet::from([id]);
        c.sel_changed();
        c.set_selection_anchor(2); // bottom-right
        c.set_selection_field("w", "4", false);
        let b = gcarve_core::geom::bounds(&c.doc.find(id).unwrap().c);
        assert!((b.x1 - 4.0).abs() < 1e-9 && (b.x0 - 0.0).abs() < 1e-9, "{b:?}");
        c.undo();
        c.undo();
        assert_eq!(c.doc.find(id).unwrap().anchor, 0);
    }

    use super::*;
    use gcarve_core::geom::{bounds, make_rect};

    #[test]
    fn selection_fields_bind_and_variables_drive_them() {
        let mut c = Canvas::default();
        let a = c.add_shape(make_rect(0.0, 0.0, 2.0, 1.0));
        c.set_variable("box-w", "4in");
        c.set_selection_field("w", "box-w", false);
        assert_eq!(c.selection_bindings().get("w").map(String::as_str), Some("box-w"));
        assert!((bounds(&c.doc.find(a).unwrap().c).w() - 4.0).abs() < 1e-9);
        c.set_variable("box-w", "5");
        assert!((bounds(&c.doc.find(a).unwrap().c).w() - 5.0).abs() < 1e-9);
        c.rename_variable("box-w", "bw");
        assert_eq!(c.doc.find(a).unwrap().bind.get("w").map(String::as_str), Some("bw"));
        assert_eq!(variable_usage(&c.doc, "bw"), vec!["W of shape 1".to_string()]);
        // a number unbinds; keep proportions scales H too
        c.set_selection_field("w", "10", true);
        let b = bounds(&c.doc.find(a).unwrap().c);
        assert!(c.selection_bindings().is_empty() && (b.w() - 10.0).abs() < 1e-9 && (b.h() - 2.0).abs() < 1e-9, "{b:?}");
        // nodes: bound x, one-off length
        c.set_node_field(a, 0, "len", "3"); // moves node 1
        let n = &c.doc.find(a).unwrap().c.nodes;
        assert!((dist(n[0].p, n[1].p) - 3.0).abs() < 1e-9 && c.doc.find(a).unwrap().bind.is_empty(), "{:?} {:?}", n.iter().map(|x| x.p).collect::<Vec<_>>(), c.doc.find(a).unwrap().bind);
        c.set_node_field(a, 0, "y", "bw / 10");
        assert!((c.doc.find(a).unwrap().c.nodes[0].p.y - 0.5).abs() < 1e-9);
        assert_eq!(c.node_info(a, 0).unwrap().by, "bw / 10");
        assert_eq!(c.add_variable(), "var2");
        c.remove_variable("var2");
        assert_eq!(c.doc.variables.len(), 1);
    }

    #[test]
    fn constraints_one_per_axis() {
        let mut c = Canvas::default();
        let a = c.add_shape(make_rect(0.0, 0.0, 2.0, 1.0));
        let b = c.add_shape(make_rect(5.0, 5.0, 6.0, 6.0));
        assert!(c.add_constraint(a, b, "right-of", "0.5"));
        assert!(c.add_constraint(a, b, "left-of", ""));
        assert!(c.add_constraint(a, b, "align-bottom", ""));
        assert_eq!(c.doc.constraints.len(), 2, "left-of replaced right-of (same axis)");
        let rb = bounds(&c.doc.find(b).unwrap().c);
        assert!(rb.x1.abs() < 1e-9 && rb.y0.abs() < 1e-9);
        c.update_constraint(0, "right-of", "1");
        assert!((bounds(&c.doc.find(b).unwrap().c).x0 - 3.0).abs() < 1e-9);
        assert!(!c.add_constraint(a, b, "right-of", "nope"));
        c.select_constraint_shapes(0);
        assert_eq!(c.sel.len(), 2);
        c.remove_constraint(0);
        assert_eq!(c.doc.constraints.len(), 1);
    }
}
