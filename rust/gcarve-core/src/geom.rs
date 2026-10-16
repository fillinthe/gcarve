//! Vector geometry shared by the editor, importers and toolpath code (C++ `core/geom.*`). Units: inches, Y up.

use std::ops::{Add, Mul, Sub};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

pub const fn pt(x: f64, y: f64) -> Pt {
    Pt { x, y }
}

impl Add for Pt {
    type Output = Pt;
    fn add(self, b: Pt) -> Pt {
        pt(self.x + b.x, self.y + b.y)
    }
}
impl Sub for Pt {
    type Output = Pt;
    fn sub(self, b: Pt) -> Pt {
        pt(self.x - b.x, self.y - b.y)
    }
}
impl Mul<f64> for Pt {
    type Output = Pt;
    fn mul(self, s: f64) -> Pt {
        pt(self.x * s, self.y * s)
    }
}

pub fn dist(a: Pt, b: Pt) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}
pub fn lerp(a: Pt, b: Pt, t: f64) -> Pt {
    a + (b - a) * t
}
/// Even-odd ray cast with half-open edges (a point level with a vertex is counted once); `at(i)` = vertex i of n.
pub fn point_in_poly(p: Pt, n: usize, at: impl Fn(usize) -> (f64, f64)) -> bool {
    let mut inn = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let ((ax, ay), (bx, by)) = (at(i), at(j));
        if (ay > p.y) != (by > p.y) && p.x < (bx - ax) * (p.y - ay) / (by - ay) + ax {
            inn = !inn;
        }
        j = i;
    }
    inn
}

pub fn dist_to_segment(q: Pt, a: Pt, b: Pt) -> f64 {
    let d = b - a;
    let l2 = d.x * d.x + d.y * d.y;
    let t = if l2 > 0.0 { (((q.x - a.x) * d.x + (q.y - a.y) * d.y) / l2).clamp(0.0, 1.0) } else { 0.0 };
    dist(q, a + d * t)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Default for Rect {
    fn default() -> Self {
        Rect { x0: f64::INFINITY, y0: f64::INFINITY, x1: f64::NEG_INFINITY, y1: f64::NEG_INFINITY }
    }
}

impl Rect {
    pub fn is_empty(&self) -> bool {
        self.x0 > self.x1
    }
    pub fn w(&self) -> f64 {
        if self.is_empty() { 0.0 } else { self.x1 - self.x0 }
    }
    pub fn h(&self) -> f64 {
        if self.is_empty() { 0.0 } else { self.y1 - self.y0 }
    }
    pub fn add(&mut self, p: Pt) {
        self.x0 = self.x0.min(p.x);
        self.y0 = self.y0.min(p.y);
        self.x1 = self.x1.max(p.x);
        self.y1 = self.y1.max(p.y);
    }
    pub fn add_rect(&mut self, r: &Rect) {
        if !r.is_empty() {
            self.add(pt(r.x0, r.y0));
            self.add(pt(r.x1, r.y1));
        }
    }
    pub fn intersects(&self, r: &Rect) -> bool {
        !self.is_empty() && !r.is_empty() && self.x0 <= r.x1 && r.x0 <= self.x1 && self.y0 <= r.y1 && r.y0 <= self.y1
    }
    pub fn contains(&self, r: &Rect) -> bool {
        !self.is_empty() && !r.is_empty() && self.x0 <= r.x0 && self.y0 <= r.y0 && self.x1 >= r.x1 && self.y1 >= r.y1
    }
}

/// x' = a*x + c*y + e ; y' = b*x + d*y + f.  (A * B)(p) == A(B(p)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Affine {
    fn default() -> Self {
        Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 }
    }
}

impl Affine {
    pub fn apply(&self, p: Pt) -> Pt {
        pt(self.a * p.x + self.c * p.y + self.e, self.b * p.x + self.d * p.y + self.f)
    }
    pub fn then(&self, r: &Affine) -> Affine {
        // self * r
        Affine {
            a: self.a * r.a + self.c * r.b,
            b: self.b * r.a + self.d * r.b,
            c: self.a * r.c + self.c * r.d,
            d: self.b * r.c + self.d * r.d,
            e: self.a * r.e + self.c * r.f + self.e,
            f: self.b * r.e + self.d * r.f + self.f,
        }
    }
    pub fn translate(x: f64, y: f64) -> Affine {
        Affine { e: x, f: y, ..Default::default() }
    }
    pub fn scale(sx: f64, sy: f64) -> Affine {
        Affine { a: sx, d: sy, ..Default::default() }
    }
    pub fn rotate(rad: f64) -> Affine {
        let (s, c) = rad.sin_cos();
        Affine { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }
}

impl Mul for Affine {
    type Output = Affine;
    fn mul(self, r: Affine) -> Affine {
        self.then(&r)
    }
}

/// Point with optional absolute-position bezier handles. A segment a->b is a cubic if a.has_out || b.has_in.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Node {
    pub p: Pt,
    pub inh: Pt,
    pub out: Pt,
    pub has_in: bool,
    pub has_out: bool,
}

impl Node {
    pub fn at(p: Pt) -> Node {
        Node { p, ..Default::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Contour {
    pub nodes: Vec<Node>,
    pub closed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub p0: Pt,
    pub c1: Pt,
    pub c2: Pt,
    pub p1: Pt,
    pub curve: bool,
}

pub fn seg_count(c: &Contour) -> usize {
    let n = c.nodes.len();
    if n < 2 { 0 } else if c.closed { n } else { n - 1 }
}

/// segment i runs nodes[i] -> nodes[(i+1) % n]
pub fn segment(c: &Contour, i: usize) -> Seg {
    let a = &c.nodes[i];
    let b = &c.nodes[(i + 1) % c.nodes.len()];
    Seg { p0: a.p, c1: if a.has_out { a.out } else { a.p }, c2: if b.has_in { b.inh } else { b.p }, p1: b.p, curve: a.has_out || b.has_in }
}

pub fn cubic_at(s: &Seg, t: f64) -> Pt {
    let q0 = lerp(s.p0, s.c1, t);
    let q1 = lerp(s.c1, s.c2, t);
    let q2 = lerp(s.c2, s.p1, t);
    lerp(lerp(q0, q1, t), lerp(q1, q2, t), t)
}

pub fn flatten(c: &Contour, tol: f64) -> Vec<Pt> {
    let mut pts = Vec::new();
    if c.nodes.is_empty() {
        return pts;
    }
    pts.push(c.nodes[0].p);
    for i in 0..seg_count(c) {
        let s = segment(c, i);
        if !s.curve {
            pts.push(s.p1);
            continue;
        }
        let d1 = s.p0 - s.c1 * 2.0 + s.c2;
        let d2 = s.c1 - s.c2 * 2.0 + s.p1;
        let m = d1.x.hypot(d1.y).max(d2.x.hypot(d2.y));
        let n = ((0.75 * m / tol).sqrt().ceil() as i64).clamp(2, 200);
        for k in 1..=n {
            pts.push(cubic_at(&s, k as f64 / n as f64));
        }
    }
    pts
}

pub const FLATTEN_TOL: f64 = 0.001;

pub fn bounds(c: &Contour) -> Rect {
    let mut r = Rect::default();
    for p in flatten(c, FLATTEN_TOL) {
        r.add(p);
    }
    r
}

pub fn transformed(c: &Contour, m: &Affine) -> Contour {
    let mut o = c.clone();
    for n in &mut o.nodes {
        n.p = m.apply(n.p);
        n.inh = m.apply(n.inh);
        n.out = m.apply(n.out);
    }
    o
}

pub fn reverse(c: &mut Contour) {
    c.nodes.reverse();
    for n in &mut c.nodes {
        std::mem::swap(&mut n.inh, &mut n.out);
        std::mem::swap(&mut n.has_in, &mut n.has_out);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub d: f64,
    pub seg: usize,
    pub t: f64,
}

/// the point of the outline a `nearest` hit stands for
pub fn point_on(c: &Contour, h: &Hit) -> Pt {
    let s = segment(c, h.seg);
    if s.curve { cubic_at(&s, h.t) } else { lerp(s.p0, s.p1, h.t) }
}

/// closest point on the outline
pub fn nearest(c: &Contour, q: Pt) -> Hit {
    let mut best = Hit { d: f64::INFINITY, seg: 0, t: 0.0 };
    for i in 0..seg_count(c) {
        let s = segment(c, i);
        let n = if s.curve { 32 } else { 1 };
        let mut prev = s.p0;
        for k in 1..=n {
            let cur = if k == n { s.p1 } else { cubic_at(&s, k as f64 / n as f64) };
            let d = cur - prev;
            let len2 = d.x * d.x + d.y * d.y;
            let u = if len2 > 0.0 { (((q.x - prev.x) * d.x + (q.y - prev.y) * d.y) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let dd = dist(q, prev + d * u);
            if dd < best.d {
                best = Hit { d: dd, seg: i, t: ((k - 1) as f64 + u) / n as f64 };
            }
            prev = cur;
        }
    }
    if best.d == f64::INFINITY && !c.nodes.is_empty() {
        best.d = dist(q, c.nodes[0].p); // single-node contour
    }
    best
}

/// insert a node, shape unchanged
pub fn split_segment(c: &mut Contour, i: usize, t: f64) {
    let n = c.nodes.len();
    let j = (i + 1) % n;
    let s = segment(c, i);
    let m = if !s.curve {
        Node::at(lerp(s.p0, s.p1, t))
    } else {
        let q0 = lerp(s.p0, s.c1, t);
        let q1 = lerp(s.c1, s.c2, t);
        let q2 = lerp(s.c2, s.p1, t);
        let r0 = lerp(q0, q1, t);
        let r1 = lerp(q1, q2, t);
        c.nodes[j].inh = q2;
        c.nodes[j].has_in = true;
        c.nodes[i].out = q0;
        c.nodes[i].has_out = true;
        Node { p: lerp(r0, r1, t), inh: r0, out: r1, has_in: true, has_out: true }
    };
    c.nodes.insert(i + 1, m);
}

pub fn remove_node(c: &mut Contour, i: usize) {
    c.nodes.remove(i);
}

/// Contour builder: lines, cubics, elliptical arcs.
#[derive(Default)]
pub struct Builder {
    pub c: Contour,
}

impl Builder {
    pub fn start(&mut self, p: Pt) {
        self.c = Contour { nodes: vec![Node::at(p)], closed: false };
    }
    pub fn cur(&self) -> Pt {
        self.c.nodes.last().map(|n| n.p).unwrap_or_default()
    }
    pub fn line(&mut self, p: Pt) {
        self.c.nodes.push(Node::at(p));
    }
    pub fn cubic(&mut self, c1: Pt, c2: Pt, p: Pt) {
        if let Some(last) = self.c.nodes.last_mut() {
            last.out = c1;
            last.has_out = true;
        }
        self.c.nodes.push(Node { p, inh: c2, has_in: true, ..Default::default() });
    }
    /// Elliptical arc around (cx,cy), x-axis rotated by phi, from param angle t1 sweeping dt (CCW positive).
    #[allow(clippy::too_many_arguments)]
    pub fn arc(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, phi: f64, t1: f64, dt: f64) {
        let n = ((dt.abs() / (std::f64::consts::PI / 2.0) - 1e-9).ceil() as i64).max(1);
        let d = dt / n as f64;
        let k = 4.0 / 3.0 * (d / 4.0).tan();
        let (sp, cp) = phi.sin_cos();
        let p = |t: f64| {
            let (x, y) = (rx * t.cos(), ry * t.sin());
            pt(cx + cp * x - sp * y, cy + sp * x + cp * y)
        };
        let dd = |t: f64| {
            let (x, y) = (-rx * t.sin(), ry * t.cos());
            pt(cp * x - sp * y, sp * x + cp * y)
        };
        for i in 0..n {
            let a = t1 + i as f64 * d;
            let b = a + d;
            self.cubic(p(a) + dd(a) * k, p(b) - dd(b) * k, p(b));
        }
    }
    /// merges a final node that coincides with the first
    pub fn close(&mut self) {
        self.c.closed = true;
        let n = self.c.nodes.len();
        if n > 1 && dist(self.c.nodes[n - 1].p, self.c.nodes[0].p) < 1e-9 {
            let last = self.c.nodes.pop().unwrap();
            self.c.nodes[0].inh = last.inh;
            self.c.nodes[0].has_in = last.has_in;
        }
    }
}

pub fn make_polyline(pts: &[Pt], closed: bool) -> Contour {
    Contour { nodes: pts.iter().map(|&p| Node::at(p)).collect(), closed }
}

pub fn make_rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Contour {
    make_polyline(&[pt(x0, y0), pt(x1, y0), pt(x1, y1), pt(x0, y1)], true)
}

pub fn make_ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> Contour {
    let mut b = Builder::default();
    b.start(pt(cx + rx, cy));
    b.arc(cx, cy, rx, ry, 0.0, 0.0, 2.0 * std::f64::consts::PI);
    b.close();
    b.c
}

/// Chain open contours whose endpoints coincide within tol. Already-closed contours pass through.
/// ponytail: O(n^2) endpoint search; add a spatial hash if DXF files with >10k loose segments get slow.
pub fn join_contours(input: Vec<Contour>, tol: f64) -> Vec<Contour> {
    let mut out = Vec::new();
    let mut open = Vec::new();
    for c in input {
        if c.closed || c.nodes.len() < 2 { out.push(c) } else { open.push(c) }
    }
    while let Some(mut chain) = open.pop() {
        let mut grew = true;
        while grew && !chain.closed {
            grew = false;
            if dist(chain.nodes[0].p, chain.nodes.last().unwrap().p) <= tol && chain.nodes.len() > 2 {
                break;
            }
            let mut k = 0;
            while k < open.len() && !grew {
                let end = chain.nodes.last().unwrap().p;
                let beg = chain.nodes[0].p;
                let o = &mut open[k];
                let at_end = if dist(end, o.nodes[0].p) <= tol {
                    true
                } else if dist(end, o.nodes.last().unwrap().p) <= tol {
                    reverse(o);
                    true
                } else if dist(beg, o.nodes.last().unwrap().p) <= tol {
                    false
                } else if dist(beg, o.nodes[0].p) <= tol {
                    reverse(o);
                    false
                } else {
                    k += 1;
                    continue;
                };
                let mut o = open.remove(k);
                if at_end {
                    let last = chain.nodes.last_mut().unwrap();
                    last.out = o.nodes[0].out;
                    last.has_out = o.nodes[0].has_out;
                    chain.nodes.extend_from_slice(&o.nodes[1..]);
                } else {
                    let ol = o.nodes.last_mut().unwrap();
                    ol.out = chain.nodes[0].out;
                    ol.has_out = chain.nodes[0].has_out;
                    o.nodes.extend_from_slice(&chain.nodes[1..]);
                    chain = o;
                }
                grew = true;
            }
        }
        let mut b = Builder { c: chain };
        if b.c.nodes.len() > 2 && dist(b.c.nodes[0].p, b.c.nodes.last().unwrap().p) <= tol {
            b.close();
        }
        out.push(b.c);
    }
    out
}

/// Same, but only joins contours with equal tags (e.g. layer names); returns contours with their tags.
pub fn join_contours_tagged(input: Vec<Contour>, tags: Vec<String>, tol: f64) -> Vec<(Contour, String)> {
    let mut order: Vec<String> = Vec::new();
    for t in &tags {
        if !order.contains(t) {
            order.push(t.clone());
        }
    }
    let mut out = Vec::new();
    for t in order {
        let group: Vec<Contour> = input.iter().zip(&tags).filter(|(_, g)| **g == t).map(|(c, _)| c.clone()).collect();
        for c in join_contours(group, tol) {
            out.push((c, t.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn rect_ellipse_bounds_and_flatten() {
        let r = bounds(&make_rect(0.0, 0.0, 2.0, 1.0));
        assert!(near(r.w(), 2.0) && near(r.h(), 1.0));
        let e = make_ellipse(1.0, 1.0, 1.0, 0.5);
        assert!(e.closed && e.nodes.len() == 4); // four quarter arcs, closing node merged
        let b = bounds(&e);
        assert!((b.w() - 2.0).abs() < 1e-3 && (b.h() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn affine_composes_right_to_left() {
        let m = Affine::translate(1.0, 0.0) * Affine::scale(2.0, 2.0);
        assert_eq!(m.apply(pt(1.0, 1.0)), pt(3.0, 2.0));
        let r = Affine::rotate(std::f64::consts::FRAC_PI_2).apply(pt(1.0, 0.0));
        assert!(near(r.x, 0.0) && near(r.y, 1.0));
    }

    #[test]
    fn split_keeps_shape_and_nearest_finds_outline() {
        let mut e = make_ellipse(0.0, 0.0, 1.0, 1.0);
        let before = bounds(&e);
        split_segment(&mut e, 0, 0.5);
        assert_eq!(e.nodes.len(), 5);
        let after = bounds(&e);
        assert!((before.w() - after.w()).abs() < 1e-6);
        let h = nearest(&make_rect(0.0, 0.0, 2.0, 2.0), pt(1.0, -0.5));
        assert!(near(h.d, 0.5) && h.seg == 0 && near(h.t, 0.5));
    }

    #[test]
    fn join_chains_and_closes() {
        let a = make_polyline(&[pt(0.0, 0.0), pt(1.0, 0.0)], false);
        let b = make_polyline(&[pt(1.0, 1.0), pt(1.0, 0.0)], false); // reversed on join
        let c = make_polyline(&[pt(1.0, 1.0), pt(0.0, 0.0)], false);
        let j = join_contours(vec![a, b, c], 1e-4);
        assert_eq!(j.len(), 1);
        assert!(j[0].closed && j[0].nodes.len() == 3);
        let t = join_contours_tagged(
            vec![make_polyline(&[pt(0.0, 0.0), pt(1.0, 0.0)], false), make_polyline(&[pt(1.0, 0.0), pt(2.0, 0.0)], false)],
            vec!["A".into(), "B".into()],
            1e-4,
        );
        assert_eq!(t.len(), 2); // different layers never join
    }
}
