//! Vector editing operations (C++ core/vecops.*, VCarve "Edit Vectors"): booleans, offset, joining, polygons/stars,
//! dogbones, box nesting.

use std::f64::consts::PI;

use clipper2_rust::{
    difference_d, inflate_paths_d, intersect_d, simplify_paths, union_subjects_d, xor_d, EndType, FillRule, JoinType, PathD, PathsD,
    PointD,
};

use crate::geom::{dist, flatten, join_contours, make_polyline, pt, reverse, Builder, Contour, Pt};

const PREC: i32 = 5;
const TOL: f64 = 0.0005;

/// Contour -> Clipper path (curves flattened, closing point dropped).
fn to_path(c: &Contour) -> PathD {
    let mut p: PathD = flatten(c, TOL).into_iter().map(|q| PointD::new(q.x, q.y)).collect();
    if c.closed && p.len() > 1 {
        p.pop();
    }
    p
}

fn from_paths(paths: &PathsD) -> Vec<Contour> {
    paths
        .iter()
        .filter(|p| p.len() >= 3)
        .map(|p| make_polyline(&p.iter().map(|q| pt(q.x, q.y)).collect::<Vec<_>>(), true))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Weld,
    Subtract,
    Intersect,
    Xor,
}

/// Closed contours only. For Subtract, `subjects` minus `clips`; otherwise all shapes are combined.
pub fn boolean_op(op: BoolOp, subjects: &[Contour], clips: &[Contour]) -> Vec<Contour> {
    // every shape counts as solid whichever way round it was drawn (imported files mix directions: with the
    // non-zero rule two overlapping shapes of opposite direction would cancel instead of merging)
    let solid = |k: &Contour| {
        let mut p = to_path(k);
        if !clipper2_rust::is_positive(&p) {
            p.reverse();
        }
        p
    };
    let s: PathsD = subjects.iter().filter(|k| k.closed).map(solid).collect();
    let c: PathsD = clips.iter().filter(|k| k.closed).map(solid).collect();
    let nz = FillRule::NonZero;
    let r = match op {
        BoolOp::Weld => union_subjects_d(&[s, c].concat(), nz, PREC),
        BoolOp::Subtract => difference_d(&s, &c, nz, PREC),
        BoolOp::Intersect => intersect_d(&s, &c, nz, PREC),
        BoolOp::Xor => xor_d(&s, &c, nz, PREC),
    };
    // ponytail: results are polylines (curves flattened at 0.0005"); refit arcs/beziers if hand-editing them matters.
    from_paths(&simplify_paths(&r, TOL / 2.0, true))
}

/// One outline from overlapping shapes that keeps their holes (letters of a script font): a non-zero union without
/// the solid-everything orientation fix `boolean_op` does, so counters (the hole in an "o") stay holes.
/// ponytail: results are polylines, like the other booleans.
pub fn weld_keep_holes(input: &[Contour]) -> Vec<Contour> {
    let p: PathsD = input.iter().filter(|k| k.closed).map(to_path).collect();
    from_paths(&simplify_paths(&union_subjects_d(&p, FillRule::NonZero, PREC), TOL / 2.0, true))
}

/// Offset closed contours outward (+) or inward (-). Open contours get a closed outline around the stroke.
pub fn offset_contours(input: &[Contour], delta: f64, round_corners: bool) -> Vec<Contour> {
    let jt = if round_corners { JoinType::Round } else { JoinType::Miter };
    let (closed, open): (Vec<&Contour>, Vec<&Contour>) = input.iter().partition(|c| c.closed);
    let mut out = Vec::new();
    if !closed.is_empty() {
        let p: PathsD = closed.into_iter().map(to_path).collect();
        let r = inflate_paths_d(&p, delta, jt, EndType::Polygon, 2.0, PREC, TOL);
        out.extend(from_paths(&simplify_paths(&r, TOL / 2.0, true)));
    }
    if !open.is_empty() {
        let p: PathsD = open.into_iter().map(to_path).collect();
        let et = if round_corners { EndType::Round } else { EndType::Square };
        let r = inflate_paths_d(&p, delta.abs(), jt, et, 2.0, PREC, TOL);
        out.extend(from_paths(&simplify_paths(&r, TOL / 2.0, true)));
    }
    out
}

/// Closed contours turned counter-clockwise (open ones untouched): offsets grow / shrink by the sign of the
/// distance only when every shape runs the same way, and imported drawings mix directions.
pub fn same_direction(input: &[Contour]) -> Vec<Contour> {
    input
        .iter()
        .map(|c| {
            let mut c = c.clone();
            if c.closed && !clipper2_rust::is_positive(&to_path(&c)) {
                crate::geom::reverse(&mut c);
            }
            c
        })
        .collect()
}

/// Curves back from a dense polyline: runs of points that lie on a circle (what booleans and offsets leave of every
/// arc) become smooth arc segments, straight runs stay lines. Reuses the G-code arc fitter (`arcfit::fit_arcs`), so
/// the same tolerance rules apply. Contours that already have curves, or only a few points, come back unchanged.
pub fn refit_arcs(c: &Contour, tol: f64) -> Contour {
    use crate::job::{Move, MoveType, Toolpath};
    if c.nodes.len() < 8 || c.nodes.iter().any(|n| n.has_in || n.has_out) {
        return c.clone();
    }
    let mut tp = Toolpath { moves: vec![Move::rapid(Some(c.nodes[0].p.x), Some(c.nodes[0].p.y), Some(0.0))], ..Default::default() };
    for n in c.nodes.iter().skip(1).chain(c.closed.then_some(&c.nodes[0])) {
        tp.moves.push(Move::linear(Some(n.p.x), Some(n.p.y), Some(0.0)));
    }
    let fitted = crate::arcfit::fit_arcs(&tp, tol, std::f64::consts::FRAC_PI_2);
    if fitted.moves.len() >= tp.moves.len() {
        return c.clone(); // nothing round in it
    }
    let mut b = crate::geom::Builder::default();
    b.start(c.nodes[0].p);
    let mut cur = c.nodes[0].p;
    for m in fitted.moves.iter().skip(1) {
        let to = pt(m.x.unwrap_or(cur.x), m.y.unwrap_or(cur.y));
        if m.kind == MoveType::Linear || m.kind == MoveType::Rapid {
            b.line(to);
        } else {
            let ctr = pt(cur.x + m.i, cur.y + m.j);
            let r = crate::geom::dist(cur, ctr);
            let (a0, a1) = ((cur.y - ctr.y).atan2(cur.x - ctr.x), (to.y - ctr.y).atan2(to.x - ctr.x));
            let tau = std::f64::consts::TAU;
            let dt = if m.kind == MoveType::ArcCw { -((a0 - a1).rem_euclid(tau)) } else { (a1 - a0).rem_euclid(tau) };
            b.arc(ctr.x, ctr.y, r, r, 0.0, a0, dt);
            if let Some(last) = b.c.nodes.last_mut() {
                last.p = to; // exactly the original point: neighbours keep meeting
            }
        }
        cur = to;
    }
    if c.closed {
        b.close();
    }
    b.c
}

/// Extend (VCarve "extend vectors to meet"): each end of the open vector `c` runs on straight along its last
/// direction until it meets the nearest of `others` (flattened). None when no end reaches anything.
pub fn extend_ends(c: &Contour, others: &[&Contour]) -> Option<Contour> {
    use crate::geom::{dist, flatten, Node};
    if c.closed || c.nodes.len() < 2 {
        return None;
    }
    let segs: Vec<(Pt, Pt)> = others
        .iter()
        .flat_map(|o| {
            let mut q = flatten(o, 0.001);
            if o.closed && q.len() > 2 {
                q.push(q[0]);
            }
            q.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>()
        })
        .collect();
    let hit = |from: Pt, dir: Pt| -> Option<Pt> {
        let len = dist(pt(0.0, 0.0), dir);
        if len < 1e-12 {
            return None;
        }
        let d = pt(dir.x / len, dir.y / len);
        let mut best: Option<f64> = None;
        for &(u, v) in &segs {
            let (ex, ey) = (v.x - u.x, v.y - u.y);
            let den = d.x * ey - d.y * ex;
            if den.abs() < 1e-14 {
                continue;
            }
            let (wx, wy) = (u.x - from.x, u.y - from.y);
            let t = (wx * ey - wy * ex) / den; // along the ray
            let s = (wx * d.y - wy * d.x) / den; // along the other segment
            if t > 1e-6 && (-1e-9..=1.0 + 1e-9).contains(&s) && best.is_none_or(|b| t < b) {
                best = Some(t);
            }
        }
        best.map(|t| pt(from.x + d.x * t, from.y + d.y * t))
    };
    let n = c.nodes.len();
    let (first, last) = (&c.nodes[0], &c.nodes[n - 1]);
    // direction the end is heading: from the handle when it is curved, else from its neighbour
    let away = |end: &Node, neighbour: &Node, handle: Option<Pt>| {
        let from = handle.unwrap_or(neighbour.p);
        pt(end.p.x - from.x, end.p.y - from.y)
    };
    let a = hit(first.p, away(first, &c.nodes[1], first.has_out.then_some(first.out)));
    let b = hit(last.p, away(last, &c.nodes[n - 2], last.has_in.then_some(last.inh)));
    if a.is_none() && b.is_none() {
        return None;
    }
    let mut out = c.clone();
    if let Some(p) = b {
        out.nodes.push(Node::at(p));
    }
    if let Some(p) = a {
        out.nodes.insert(0, Node::at(p));
    }
    Some(out)
}

/// Trim (VCarve scissors): remove the piece of `all[target]` under `at` between the crossings on either side of
/// it (crossings with any vector in `all`, itself included). A closed vector opens up; an open one loses its end or
/// splits in two. None when the vector isn't crossed there (nothing to trim to).
/// ponytail: works on the flattened outline, so the pieces come back as polylines (like the booleans).
pub fn trim_at(all: &[&Contour], target: usize, at: Pt) -> Option<Vec<Contour>> {
    use crate::geom::{bounds, dist, flatten};
    let c = all.get(target)?;
    let mut p = flatten(c, 0.001);
    if c.closed && p.len() > 2 && dist(p[0], p[p.len() - 1]) > 1e-9 {
        p.push(p[0]);
    }
    if p.len() < 2 {
        return None;
    }
    let mut lens = vec![0.0];
    for w in p.windows(2) {
        lens.push(lens[lens.len() - 1] + dist(w[0], w[1]));
    }
    let total = lens[lens.len() - 1];
    // crossings along the target, as distance from its start
    let tb = bounds(c);
    let mut cuts: Vec<f64> = Vec::new();
    for (k, o) in all.iter().enumerate() {
        let ob = bounds(o);
        if ob.x1 < tb.x0 - 1e-9 || ob.x0 > tb.x1 + 1e-9 || ob.y1 < tb.y0 - 1e-9 || ob.y0 > tb.y1 + 1e-9 {
            continue;
        }
        let mut q = flatten(o, 0.001);
        if o.closed && q.len() > 2 && dist(q[0], q[q.len() - 1]) > 1e-9 {
            q.push(q[0]);
        }
        for i in 0..p.len() - 1 {
            let (a, b) = (p[i], p[i + 1]);
            for j in 0..q.len().saturating_sub(1) {
                if k == target && (i == j || i + 1 == j || j + 1 == i || (c.closed && (i.min(j) == 0 && i.max(j) == p.len() - 2))) {
                    continue; // a segment and its neighbours always touch
                }
                let (u, v) = (q[j], q[j + 1]);
                let den = (b.x - a.x) * (v.y - u.y) - (b.y - a.y) * (v.x - u.x);
                if den.abs() < 1e-14 {
                    continue;
                }
                let t = ((u.x - a.x) * (v.y - u.y) - (u.y - a.y) * (v.x - u.x)) / den;
                let s = ((u.x - a.x) * (b.y - a.y) - (u.y - a.y) * (b.x - a.x)) / den;
                if (-1e-9..=1.0 + 1e-9).contains(&t) && (-1e-9..=1.0 + 1e-9).contains(&s) {
                    cuts.push(lens[i] + t.clamp(0.0, 1.0) * (lens[i + 1] - lens[i]));
                }
            }
        }
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    // where the click is along the target
    let (mut sc, mut best) = (0.0, f64::MAX);
    for i in 0..p.len() - 1 {
        let (a, b) = (p[i], p[i + 1]);
        let l2 = (b.x - a.x).powi(2) + (b.y - a.y).powi(2);
        let t = if l2 > 0.0 { (((at.x - a.x) * (b.x - a.x) + (at.y - a.y) * (b.y - a.y)) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let d = dist(at, pt(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
        if d < best {
            (best, sc) = (d, lens[i] + t * (lens[i + 1] - lens[i]));
        }
    }
    // the polyline from distance s0 to s1 (s1 may run past the end of a closed outline)
    let piece = |s0: f64, s1: f64| -> Contour {
        let at_len = |s: f64| {
            let s = s.clamp(0.0, total);
            let i = lens.partition_point(|&l| l <= s).clamp(1, p.len() - 1);
            let t = if lens[i] > lens[i - 1] { (s - lens[i - 1]) / (lens[i] - lens[i - 1]) } else { 0.0 };
            pt(p[i - 1].x + (p[i].x - p[i - 1].x) * t, p[i - 1].y + (p[i].y - p[i - 1].y) * t)
        };
        let mut out = vec![at_len(if s0 >= total { s0 - total } else { s0 })];
        let walk = |from: f64, to: f64, out: &mut Vec<Pt>| {
            for i in 0..p.len() {
                if lens[i] > from + 1e-9 && lens[i] < to - 1e-9 {
                    out.push(p[i]);
                }
            }
            out.push(at_len(to));
        };
        if s1 > total {
            walk(s0, total, &mut out);
            walk(0.0, s1 - total, &mut out);
        } else {
            walk(s0, s1, &mut out);
        }
        out.dedup_by(|a, b| dist(*a, *b) < 1e-9);
        make_polyline(&out, false)
    };
    if c.closed {
        if cuts.len() < 2 {
            return None;
        }
        let a = cuts.iter().rev().find(|&&s| s <= sc).copied().unwrap_or(cuts[cuts.len() - 1] - total); // crossing before the click
        let b = cuts.iter().find(|&&s| s >= sc).copied().unwrap_or(cuts[0] + total); // and after it
        // keep the rest of the loop: from b round to a
        let (from, to) = (b.rem_euclid(total), a.rem_euclid(total));
        Some(vec![piece(from, if to > from { to } else { to + total })])
    } else {
        let a = cuts.iter().rev().find(|&&s| s < sc).copied();
        let b = cuts.iter().find(|&&s| s > sc).copied();
        if a.is_none() && b.is_none() {
            return None;
        }
        let mut out = Vec::new();
        if let Some(a) = a.filter(|&a| a > 1e-6) {
            out.push(piece(0.0, a));
        }
        if let Some(b) = b.filter(|&b| b < total - 1e-6) {
            out.push(piece(b, total));
        }
        Some(out)
    }
}

/// Fewer nodes (VCarve "Fit curves to vectors" for the straight-line case): each contour becomes a polyline that
/// stays within `tol` of the original (Douglas-Peucker, the tracer's). Open contours stay open, ends kept.
pub fn simplify_contours(input: &[Contour], tol: f64) -> Vec<Contour> {
    input
        .iter()
        .map(|c| {
            let p = crate::geom::flatten(c, (tol / 4.0).max(0.0002));
            if p.len() < 3 {
                return c.clone();
            }
            if c.closed {
                let mut q = p;
                if q.len() > 3 && crate::geom::dist(q[0], q[q.len() - 1]) < 1e-9 {
                    q.pop();
                }
                make_polyline(&crate::trace::simplify_closed(&q, tol), true)
            } else {
                let mut out = Vec::new();
                crate::trace::dp(&p, tol, &mut out);
                make_polyline(&out, false)
            }
        })
        .collect()
}

/// What `check_vectors` found, as shape ids (VCarve "Vector Validator").
#[derive(Debug, Default, PartialEq)]
pub struct VectorCheck {
    /// not closed: can't be pocketed or cut inside / outside
    pub open: Vec<i32>,
    /// a copy lying on an earlier vector (the earlier one is kept out of the list): would be cut twice
    pub duplicates: Vec<i32>,
    /// smaller than 0.01 in both ways: import specks
    pub tiny: Vec<i32>,
}

/// Find the vectors that usually stop an imported drawing from machining well.
/// ponytail: duplicates are matched by bounding box + length + closedness (1/1000 in), which catches stacked copies
/// from DXF/SVG exports; partial overlaps and self-crossings are not looked for.
pub fn check_vectors(items: &[(i32, &Contour)]) -> VectorCheck {
    let mut out = VectorCheck::default();
    let mut seen = std::collections::HashSet::new();
    let q = |v: f64| (v * 1000.0).round() as i64;
    for &(id, c) in items {
        let b = crate::geom::bounds(c);
        let pts = crate::geom::flatten(c, 0.001);
        let mut len: f64 = pts.windows(2).map(|w| crate::geom::dist(w[0], w[1])).sum();
        if c.closed && pts.len() > 1 {
            len += crate::geom::dist(pts[pts.len() - 1], pts[0]);
        }
        if (b.x1 - b.x0).max(b.y1 - b.y0) < 0.01 {
            out.tiny.push(id);
        } else if !seen.insert((c.closed, q(b.x0), q(b.y0), q(b.x1), q(b.y1), q(len))) {
            out.duplicates.push(id);
        } else if !c.closed {
            out.open.push(id);
        }
    }
    out
}

/// Fillet (round) the corners of closed contours with radius r: `outer` = the corners that stick out (convex),
/// `inner` = the ones that go in (concave). Rolling a disc of radius r inside the shape rounds the outer corners
/// (in by r, out by r); rolling it outside rounds the inner ones (out by r, in by r). Parts narrower than 2r vanish
/// when rounding outer corners, as with any fillet that doesn't fit.
pub fn fillet_contours(input: &[Contour], r: f64, outer: bool, inner: bool) -> Vec<Contour> {
    let mut c: Vec<Contour> = input.iter().filter(|c| c.closed).cloned().collect();
    if r <= 0.0 {
        return c;
    }
    if outer {
        c = offset_contours(&offset_contours(&c, -r, true), r, true);
    }
    if inner {
        c = offset_contours(&offset_contours(&c, r, true), -r, true);
    }
    c
}

/// Open contours whose ends meet within tol are joined; a single open contour whose ends meet is closed.
pub fn join_open(input: &[Contour], tol: f64) -> Vec<Contour> {
    join_contours(input.to_vec(), tol)
}

/// Force-close (adds the closing segment).
pub fn close_contour(c: Contour) -> Contour {
    if c.nodes.len() < 3 {
        return c;
    }
    let ends_meet = dist(c.nodes[0].p, c.nodes[c.nodes.len() - 1].p) < 1e-9;
    let mut b = Builder { c };
    if ends_meet {
        b.close();
    } else {
        b.c.closed = true;
    }
    b.c
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NestItem {
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct NestPlace {
    pub x: f64,
    pub y: f64,
    pub rotated: bool,
    pub placed: bool,
}

/// Bounding-box nesting (shelf packing): x/y of each box's bottom-left and whether it was rotated 90 degrees.
/// Items that don't fit get placed = false.
pub fn nest_boxes(items: &[NestItem], sheet_w: f64, sheet_h: f64, gap: f64, margin: f64, allow_rotate: bool) -> Vec<NestPlace> {
    let mut out = vec![NestPlace::default(); items.len()];
    // tallest first (after optional rotation so the short side is the height), left to right in shelves
    let eff = |i: usize| {
        let it = items[i];
        if allow_rotate && it.w < it.h {
            (NestItem { w: it.h, h: it.w }, true)
        } else {
            (it, false)
        }
    };
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| eff(b).0.h.total_cmp(&eff(a).0.h));
    let (usable_w, usable_h) = (sheet_w - 2.0 * margin, sheet_h - 2.0 * margin);
    let (mut x, mut y, mut shelf_h) = (0.0, 0.0, 0.0f64);
    for i in order {
        let (mut it, mut rot) = eff(i);
        let swap = |it: &mut NestItem, rot: &mut bool| {
            std::mem::swap(&mut it.w, &mut it.h);
            *rot = !*rot;
        };
        // try as is, then rotated if that helps fit the remaining shelf width
        if x + it.w > usable_w + 1e-9 && allow_rotate && x + it.h <= usable_w + 1e-9 && it.h <= usable_w {
            swap(&mut it, &mut rot);
        }
        if x + it.w > usable_w + 1e-9 {
            // new shelf
            x = 0.0;
            y += shelf_h + if shelf_h > 0.0 { gap } else { 0.0 };
            shelf_h = 0.0;
            if allow_rotate && it.w > usable_w && it.h <= usable_w {
                swap(&mut it, &mut rot);
            }
        }
        if it.w > usable_w + 1e-9 || y + it.h > usable_h + 1e-9 {
            continue; // doesn't fit on this sheet
        }
        out[i] = NestPlace { x: margin + x, y: margin + y, rotated: rot, placed: true };
        x += it.w + gap;
        shelf_h = shelf_h.max(it.h);
    }
    out
}

pub fn make_polygon(centre: Pt, radius: f64, sides: i32, rotation_rad: f64) -> Contour {
    let sides = sides.max(3);
    let pts: Vec<Pt> = (0..sides)
        .map(|i| {
            let a = rotation_rad + PI / 2.0 + 2.0 * PI * i as f64 / sides as f64;
            pt(centre.x + radius * a.cos(), centre.y + radius * a.sin())
        })
        .collect();
    make_polyline(&pts, true)
}

pub fn make_star(centre: Pt, outer_r: f64, inner_r: f64, points: i32, rotation_rad: f64) -> Contour {
    let points = points.max(3);
    let pts: Vec<Pt> = (0..points * 2)
        .map(|i| {
            let r = if i % 2 == 0 { outer_r } else { inner_r };
            let a = rotation_rad + PI / 2.0 + PI * i as f64 / points as f64;
            pt(centre.x + r * a.cos(), centre.y + r * a.sin())
        })
        .collect();
    make_polyline(&pts, true)
}

fn signed_area(c: &Contour) -> f64 {
    let f = flatten(c, crate::geom::FLATTEN_TOL);
    let mut a = 0.0;
    for i in 0..f.len() {
        let (p, q) = (f[i], f[(i + 1) % f.len()]);
        a += p.x * q.y - q.x * p.y;
    }
    a / 2.0
}

fn unit(v: Pt) -> Pt {
    let l = v.x.hypot(v.y);
    if l > 0.0 {
        v * (1.0 / l)
    } else {
        Pt::default()
    }
}

/// Dogbone / T-bone corner relief for a round bit of radius r: each sharp corner the bit can't reach gets a circular
/// lobe so square parts fit the cut. inside = the contour is a hole/pocket (relieve its convex corners); otherwise a
/// part cut on the outside (relieve its concave corners). tbone puts the lobe across the incoming edge, not the bisector.
/// ponytail: line-line corners only; a corner next to a curve is left alone.
pub fn dogbone(input: &Contour, r: f64, inside: bool, tbone: bool) -> Contour {
    if !input.closed || input.nodes.len() < 3 || r <= 0.0 {
        return input.clone();
    }
    let mut c = input.clone();
    // walk with the region to relieve on the left: a hole CCW, a part's exterior CW; its corners are then left turns
    let flip = (signed_area(&c) > 0.0) != inside;
    if flip {
        reverse(&mut c);
    }
    let n = c.nodes.len();
    let mut out = Contour { nodes: Vec::new(), closed: true };
    for i in 0..n {
        let (a, cn, b) = (&c.nodes[(i + n - 1) % n], &c.nodes[i], &c.nodes[(i + 1) % n]);
        let straight = !a.has_out && !cn.has_in && !cn.has_out && !b.has_in;
        let (d1, d2) = (unit(cn.p - a.p), unit(b.p - cn.p));
        let turn = (d1.x * d2.y - d1.y * d2.x).atan2(d1.x * d2.x + d1.y * d2.y);
        if !straight || turn < 0.2 {
            out.nodes.push(*cn); // not a sharp left turn
            continue;
        }
        let v = if tbone { d1 * -1.0 } else { unit(d2 - d1) }; // lobe centre direction from the corner
        let o = cn.p + v * r; // bit centre that just touches the corner
        let t = 2.0 * r * (d1.x * v.x + d1.y * v.y); // circle meets the edges at t<=0, s>=0
        let s = 2.0 * r * (d2.x * v.x + d2.y * v.y);
        if -t > dist(a.p, cn.p) - 1e-6 || s > dist(b.p, cn.p) - 1e-6 {
            out.nodes.push(*cn); // edge too short
            continue;
        }
        let (p1, p2) = (cn.p + d1 * t, cn.p + d2 * s);
        let a1 = (p1.y - o.y).atan2(p1.x - o.x);
        let a2 = (p2.y - o.y).atan2(p2.x - o.x);
        let ac = (cn.p.y - o.y).atan2(cn.p.x - o.x);
        let ccw = |from: f64, to: f64| {
            let d = (to - from) % (2.0 * PI);
            if d < 0.0 {
                d + 2.0 * PI
            } else {
                d
            }
        };
        let mut dt = ccw(a1, a2);
        if ccw(a1, ac) > dt {
            dt -= 2.0 * PI; // go round through the corner, away from the cut region
        }
        let mut bl = Builder::default();
        bl.start(p1);
        bl.arc(o.x, o.y, r, r, 0.0, a1, dt);
        if s < 1e-9 {
            bl.c.nodes.last_mut().unwrap().p = cn.p; // T-bone ends on the corner
        }
        out.nodes.extend(bl.c.nodes);
    }
    if flip {
        reverse(&mut out);
    }
    out
}

#[cfg(test)]
mod fillet_tests {
    use super::*;
    use crate::geom::{flatten, make_polyline, pt};

    fn area(c: &[Contour]) -> f64 {
        c.iter().map(|c| { let p = flatten(c, 0.001); (0..p.len()).map(|i| { let (a, b) = (p[i], p[(i + 1) % p.len()]); a.x * b.y - b.x * a.y }).sum::<f64>().abs() / 2.0 }).sum()
    }

    #[test]
    fn refit_turns_a_welded_circle_back_into_a_few_curves() {
        // a circle welded to a square, as the app's Weld leaves it: hundreds of points round the arc
        let circle = crate::geom::make_ellipse(0.0, 0.0, 2.0, 2.0);
        let sq = make_polyline(&[pt(1.0, -4.0), pt(4.0, -4.0), pt(4.0, -1.0), pt(1.0, -1.0)], true);
        let welded = boolean_op(BoolOp::Weld, &[circle, sq], &[]);
        assert_eq!(welded.len(), 1);
        let before = welded[0].nodes.len();
        let fit = refit_arcs(&welded[0], 0.001);
        assert!(before > 100 && fit.nodes.len() < 20, "{before} -> {} nodes", fit.nodes.len());
        assert!(fit.closed && fit.nodes.iter().any(|n| n.has_out));
        // same shape: area within a hair, and every original point still within tolerance of the new outline
        assert!((area(std::slice::from_ref(&fit)) - area(&welded)).abs() < 0.01);
        for n in &welded[0].nodes {
            assert!(crate::geom::nearest(&fit, n.p).d < 0.003, "{:?} is {} off", n.p, crate::geom::nearest(&fit, n.p).d);
        }
        // shapes drawn in opposite directions still weld into one
        let cw = make_polyline(&[pt(1.0, -1.0), pt(4.0, -1.0), pt(4.0, -4.0), pt(1.0, -4.0)], true);
        assert_eq!(boolean_op(BoolOp::Weld, &[crate::geom::make_ellipse(0.0, 0.0, 2.0, 2.0), cw.clone()], &[]).len(), 1);
        // offsetting outwards grows a shape whichever way round it was drawn
        let grown = offset_contours(&same_direction(std::slice::from_ref(&cw)), 0.5, false);
        assert!((area(&grown) - 16.0).abs() < 0.01, "{}", area(&grown));
        // a plain rectangle or an already curved shape is left alone
        assert_eq!(refit_arcs(&make_polyline(&[pt(0.0, 0.0), pt(1.0, 0.0), pt(1.0, 1.0)], true), 0.001).nodes.len(), 3);
    }

    #[test]
    fn trim_removes_the_piece_between_crossings() {
        let line = make_polyline(&[pt(0.0, 0.0), pt(10.0, 0.0)], false);
        let (v3, v7) = (make_polyline(&[pt(3.0, -1.0), pt(3.0, 1.0)], false), make_polyline(&[pt(7.0, -1.0), pt(7.0, 1.0)], false));
        let all = [&line, &v3, &v7];
        let ends = |c: &Contour| (c.nodes[0].p, c.nodes[c.nodes.len() - 1].p);
        // middle piece: two pieces left
        let r = trim_at(&all, 0, pt(5.0, 0.1)).unwrap();
        assert_eq!(r.iter().map(ends).collect::<Vec<_>>(), vec![(pt(0.0, 0.0), pt(3.0, 0.0)), (pt(7.0, 0.0), pt(10.0, 0.0))]);
        // an end piece: one piece left
        let r = trim_at(&all, 0, pt(1.0, 0.0)).unwrap();
        assert_eq!(r.iter().map(ends).collect::<Vec<_>>(), vec![(pt(3.0, 0.0), pt(10.0, 0.0))]);
        // not crossed: nothing to trim to
        assert!(trim_at(&[&v3, &v7], 0, pt(3.0, 0.0)).is_none());
        // a square crossed by the long line at y = 0: clicking its top leaves the bottom part, open
        let sq = make_polyline(&[pt(4.0, -1.0), pt(6.0, -1.0), pt(6.0, 1.0), pt(4.0, 1.0)], true);
        let r = trim_at(&[&sq, &line], 0, pt(5.0, 1.0)).unwrap();
        assert_eq!(r.len(), 1);
        assert!(!r[0].closed && r[0].nodes.iter().all(|n| n.p.y <= 1e-9), "{:?}", r[0].nodes.iter().map(|n| n.p).collect::<Vec<_>>());
        assert_eq!(r[0].nodes.len(), 4); // crossing, two bottom corners, crossing
        let r = trim_at(&[&sq, &line], 0, pt(5.0, -1.0)).unwrap();
        assert!(r[0].nodes.iter().all(|n| n.p.y >= -1e-9));
    }

    #[test]
    fn simplify_drops_nodes_but_stays_within_tolerance() {
        // a circle as 720 points and a wiggly open line
        let circle: Vec<Pt> = (0..720).map(|i| { let a = i as f64 * std::f64::consts::TAU / 720.0; pt(2.0 * a.cos(), 2.0 * a.sin()) }).collect();
        let line: Vec<Pt> = (0..=400).map(|i| pt(i as f64 * 0.01, 0.0005 * (i as f64).sin())).collect();
        let out = simplify_contours(&[make_polyline(&circle, true), make_polyline(&line, false)], 0.005);
        assert!(out[0].closed && out[0].nodes.len() < 80 && out[0].nodes.len() > 20, "{} nodes", out[0].nodes.len());
        for n in &out[0].nodes {
            assert!((n.p.x.hypot(n.p.y) - 2.0).abs() < 1e-9); // kept points are original points
        }
        assert!((area(&out[..1]) - std::f64::consts::PI * 4.0).abs() < 0.05);
        assert!(!out[1].closed && out[1].nodes.len() == 2, "a nearly straight line becomes its two ends");
        assert_eq!((out[1].nodes[0].p, out[1].nodes[1].p), (line[0], line[400]));
    }

    #[test]
    fn check_finds_open_duplicate_and_tiny_vectors() {
        let sq = make_polyline(&[pt(0.0, 0.0), pt(2.0, 0.0), pt(2.0, 2.0), pt(0.0, 2.0)], true);
        let open = make_polyline(&[pt(3.0, 0.0), pt(4.0, 0.0), pt(4.0, 1.0)], false);
        let speck = make_polyline(&[pt(5.0, 5.0), pt(5.001, 5.0)], false);
        let moved = make_polyline(&[pt(0.5, 0.0), pt(2.5, 0.0), pt(2.5, 2.0), pt(0.5, 2.0)], true);
        let r = check_vectors(&[(1, &sq), (2, &open), (3, &sq), (4, &speck), (5, &moved)]);
        assert_eq!(r, VectorCheck { open: vec![2], duplicates: vec![3], tiny: vec![4] });
    }

    #[test]
    fn fillet_rounds_the_chosen_corners() {
        // square: 4 outer corners, each loses r²(1 - π/4)
        let sq = [make_polyline(&[pt(0.0, 0.0), pt(2.0, 0.0), pt(2.0, 2.0), pt(0.0, 2.0)], true)];
        let r = 0.25;
        let lost = 4.0 * r * r * (1.0 - std::f64::consts::FRAC_PI_4);
        assert!((area(&fillet_contours(&sq, r, true, false)) - (4.0 - lost)).abs() < 0.002);
        assert!((area(&fillet_contours(&sq, r, false, true)) - 4.0).abs() < 0.002, "a square has no inner corners");
        // L shape: one inner corner gains r²(1 - π/4)
        let l = [make_polyline(&[pt(0.0, 0.0), pt(2.0, 0.0), pt(2.0, 1.0), pt(1.0, 1.0), pt(1.0, 2.0), pt(0.0, 2.0)], true)];
        assert!((area(&fillet_contours(&l, r, false, true)) - (3.0 + lost / 4.0)).abs() < 0.002);
        assert!((area(&fillet_contours(&l, r, true, true)) - (3.0 + lost / 4.0 - 5.0 * lost / 4.0)).abs() < 0.003);
    }

    #[test]
    fn open_ends_extend_to_the_nearest_vector() {
        use crate::geom::{bounds, make_rect, Node};
        // a vertical stub 0..1 at x = 2 under a box 0..4 x 3..5
        let stub = Contour { nodes: vec![Node::at(pt(2.0, 0.0)), Node::at(pt(2.0, 1.0))], ..Default::default() };
        let boxr = make_rect(0.0, 3.0, 4.0, 5.0);
        let e = extend_ends(&stub, &[&boxr]).unwrap();
        let b = bounds(&e);
        assert!((b.y1 - 3.0).abs() < 1e-9 && b.y0 == 0.0, "{b:?}"); // the top end runs up to the box, the bottom meets nothing
        assert!(extend_ends(&boxr, &[&stub]).is_none(), "closed vectors don't extend");
        let far = make_rect(10.0, 10.0, 11.0, 11.0);
        assert!(extend_ends(&stub, &[&far]).is_none());
    }
}
