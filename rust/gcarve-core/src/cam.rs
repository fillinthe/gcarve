//! CAM operations: tools, job setup, toolpath generation (C++ `core/cam.*`). Produces the post-processor IR (job.rs).
//! Offsets and booleans go through clipper2-rust with the same calls and precision as the C++ engine (Clipper2 C++);
//! the v-carve medial axis uses boostvoronoi (the Rust port of the Boost.Polygon Voronoi the C++ engine vendors).

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::PI;

use boostvoronoi::prelude::{Builder as VBuilder, SourceCategory};
use clipper2_rust::{difference_d, inflate_paths_d, intersect_d, is_positive, simplify_paths, union_subjects_d, EndType, FillRule, JoinType, PathD, PathsD, PointD};

use crate::arcfit::{fit_arcs, DEFAULT_MAX_SWEEP, DEFAULT_TOL};
use crate::geom::{bounds, dist, dist_to_segment, flatten, lerp, make_polyline, make_rect, nearest, pt, transformed, Affine, Contour, Pt, Rect};
use crate::job::{Job, Move, MoveType, Toolpath};
use crate::order::{optimize_order, order_toolpaths};

// ---- input types (mirror core/cam.h; to be deduped with document.rs) -------------------------------------------------

// Input types live in document.rs (one definition for the document, project files and CAM); re-exported here.
pub use crate::document::{Entry, FeedPreset, Material, Op, OpType, Side, Tool, ToolType};

pub fn preset_for<'a>(t: &'a Tool, material: &str) -> Option<&'a FeedPreset> {
    let key = material.to_ascii_lowercase();
    t.presets.iter().find(|p| p.material.to_ascii_lowercase() == key)
}

/// Base tool with the matching preset applied.
pub fn effective_tool(t: &Tool, material: &str) -> Tool {
    let mut e = t.clone();
    if let Some(p) = preset_for(t, material) {
        e.feed_xy = p.feed_xy;
        e.feed_z = p.feed_z;
        e.rpm = p.rpm;
        if p.stepdown > 0.0 {
            e.stepdown = p.stepdown;
        }
    }
    e
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CamResult {
    pub paths: Vec<Toolpath>,
    pub warnings: Vec<String>,
}

// ---- helpers --------------------------------------------------------------------------------------------------------

const PREC: i32 = 5; // clipper decimal precision (1e-5 in)
const TOL: f64 = 0.0005; // curve flattening / arc tolerance, inches
const RAMP_SEG: f64 = 0.1; // max segment length on ramping laps, so Z changes smoothly and tabs clamp cleanly

/// `std::clamp` semantics (never panics when lo > hi, unlike `f64::clamp`).
pub(crate) fn clampf(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if hi < v {
        hi
    } else {
        v
    }
}

pub fn rapid_to(tp: &mut Toolpath, p: Pt, z: Option<f64>) {
    tp.moves.push(Move::rapid(Some(p.x), Some(p.y), z));
}
pub fn cut_to(tp: &mut Toolpath, p: Pt, z: Option<f64>) {
    tp.moves.push(Move::linear(Some(p.x), Some(p.y), z));
}
/// where the bit actually is after the moves so far (generators must not trust their own tracked position)
fn last_xy(tp: &Toolpath, fallback: Pt) -> Pt {
    let (mut x, mut y) = (None, None);
    for m in tp.moves.iter().rev() {
        if x.is_some() && y.is_some() {
            break;
        }
        if x.is_none() {
            x = m.x;
        }
        if y.is_none() {
            y = m.y;
        }
    }
    pt(x.unwrap_or(fallback.x), y.unwrap_or(fallback.y))
}
fn last_z(tp: &Toolpath) -> Option<f64> {
    tp.moves.last().and_then(|m| m.z)
}

/// Safety net for every toolpath: a rapid that moves sideways must start and end above the surface. A sideways rapid
/// from below Z0 gets a straight lift to `retract_z` first; one aimed below Z0 stays at `retract_z` (the next feed move
/// plunges).
pub fn enforce_safe_rapids(tp: &mut Toolpath, retract_z: f64) {
    let mut out = Vec::with_capacity(tp.moves.len());
    let (mut x, mut y, mut z): (Option<f64>, Option<f64>, Option<f64>) = (None, None, None);
    for &m in &tp.moves {
        let mut mv = m;
        let moved = |a: Option<f64>, b: Option<f64>| matches!((a, b), (Some(a), Some(b)) if (a - b).abs() > 1e-9);
        let sideways = mv.kind == MoveType::Rapid && (moved(mv.x, x) || moved(mv.y, y));
        if sideways {
            if z.is_some_and(|z| z < -1e-9) {
                out.push(Move::rapid(x, y, Some(retract_z)));
                z = Some(retract_z);
            }
            if mv.z.is_some_and(|v| v < -1e-9) {
                mv.z = Some(retract_z);
            }
            if mv.z.is_none() && z.is_some_and(|z| z < -1e-9) {
                mv.z = Some(retract_z);
            }
        }
        out.push(mv);
        if mv.x.is_some() {
            x = mv.x;
        }
        if mv.y.is_some() {
            y = mv.y;
        }
        if mv.z.is_some() {
            z = mv.z;
        }
    }
    tp.moves = out;
}

/// Rounded polygon offset at the CAM tolerance: + grows, - shrinks (empty when the region vanishes)
fn offset_region(region: &PathsD, delta: f64) -> PathsD {
    inflate_paths_d(region, delta, JoinType::Round, EndType::Polygon, 2.0, PREC, TOL)
}

fn to_path(c: &Contour) -> PathD {
    let mut p: PathD = flatten(c, TOL).into_iter().map(|q| PointD::new(q.x, q.y)).collect();
    if c.closed && p.len() > 1 {
        p.pop(); // flatten repeats the start point
    }
    p
}
fn pts_of(p: &PathD) -> Vec<Pt> {
    p.iter().map(|q| pt(q.x, q.y)).collect()
}
fn contour_of(p: &PathD) -> Contour {
    make_polyline(&pts_of(p), true)
}

/// Even-odd ray cast with half-open edges, so a point level with a vertex is counted once.
fn inside(p: Pt, poly: &PathD) -> bool {
    crate::geom::point_in_poly(p, poly.len(), |i| (poly[i].x, poly[i].y))
}

/// Offset rings (levels[0] = outermost) in cut order. Parent = nearest ring one level out, so rings along one wall or
/// around one island chain together; post-order (children first) cuts each local area inside-out before moving on.
fn order_rings(levels: &[PathsD]) -> Vec<(PathD, usize)> {
    struct R {
        path: PathD,
        level: usize,
        kids: Vec<usize>,
        start: Pt,
    }
    let mut rings: Vec<R> = Vec::new();
    let mut polys: Vec<Contour> = Vec::new();
    for (lv, paths) in levels.iter().enumerate() {
        for p in paths {
            rings.push(R { path: p.clone(), level: lv, kids: Vec::new(), start: pt(p[0].x, p[0].y) });
            polys.push(contour_of(p));
        }
    }
    for i in 0..rings.len() {
        if rings[i].level == 0 {
            continue;
        }
        let (mut bd, mut parent) = (1e300, None);
        for j in 0..rings.len() {
            if rings[j].level + 1 != rings[i].level {
                continue;
            }
            let d = nearest(&polys[j], rings[i].start).d;
            if d < bd {
                bd = d;
                parent = Some(j);
            }
        }
        if let Some(p) = parent {
            rings[p].kids.push(i);
        }
    }
    let mut order = Vec::new();
    let mut seen = vec![false; rings.len()];
    let mut walk = rings.first().map_or(Pt::default(), |r| r.start);
    fn visit(i: usize, rings: &[R], seen: &mut [bool], order: &mut Vec<usize>, walk: &mut Pt) {
        seen[i] = true;
        let mut kids = rings[i].kids.clone();
        while !kids.is_empty() {
            let (mut best, mut bd) = (0, 1e300);
            for (k, &kid) in kids.iter().enumerate() {
                let d = dist(*walk, rings[kid].start);
                if d < bd {
                    bd = d;
                    best = k;
                }
            }
            let k = kids.remove(best);
            visit(k, rings, seen, order, walk);
        }
        order.push(i);
        *walk = rings[i].start;
    }
    let mut roots: Vec<usize> = (0..rings.len()).filter(|&i| rings[i].level == 0).collect();
    roots.sort_by_key(|&r| !is_positive(&rings[r].path)); // stable: outer walls first, then islands
    while !roots.is_empty() {
        let (mut best, mut bd) = (0, 1e300);
        let outer = is_positive(&rings[roots[0]].path);
        for (k, &r) in roots.iter().enumerate() {
            if is_positive(&rings[r].path) != outer {
                break;
            }
            let d = dist(walk, rings[r].start);
            if d < bd {
                bd = d;
                best = k;
            }
        }
        let r = roots.remove(best);
        if !seen[r] {
            visit(r, &rings, &mut seen, &mut order, &mut walk);
        }
    }
    for i in 0..rings.len() {
        if !seen[i] {
            visit(i, &rings, &mut seen, &mut order, &mut walk);
        }
    }
    order.into_iter().map(|i| (std::mem::take(&mut rings[i].path), rings[i].level)).collect()
}

/// A sample along the cutter path. s = distance along the path, cumulative over laps of a closed path; in_tab decides
/// Z clamping.
#[derive(Clone, Copy)]
struct P {
    xy: Pt,
    z: f64,
    s: f64,
    in_tab: bool,
}

fn subdivide(pts: &[Pt], max_len: f64) -> Vec<Pt> {
    let mut out = vec![pts[0]];
    for i in 1..pts.len() {
        let n = ((dist(pts[i - 1], pts[i]) / max_len).ceil() as i32).max(1);
        for k in 1..=n {
            out.push(lerp(pts[i - 1], pts[i], k as f64 / n as f64));
        }
    }
    out
}

fn lengths(xy: &[Pt]) -> Vec<f64> {
    let mut s = vec![0.0];
    for i in 1..xy.len() {
        s.push(s[i - 1] + dist(xy[i - 1], xy[i]));
    }
    s
}

fn passes_for(depth: f64, stepdown: f64) -> i32 {
    ((depth / stepdown - 1e-9).ceil() as i32).max(1)
}

/// Distance along `path` (`lens` = cumulative lengths) of the point nearest `p`, when the path comes within `reach`.
fn along(path: &[Pt], lens: &[f64], p: Pt, reach: f64) -> Option<f64> {
    let h = nearest(&make_polyline(path, false), p);
    (h.d <= reach && h.seg + 1 < lens.len()).then(|| lens[h.seg] + h.t * (lens[h.seg + 1] - lens[h.seg]))
}

/// Builds the moves for one cutter path. closed: pts[0] == pts.last().
fn cut_path(path: &[Pt], closed: bool, op: &Op, tool: &Tool, m: &Material, warn: &mut Vec<String>) -> Toolpath {
    let stepdown = if op.stepdown > 0.0 { op.stepdown } else { tool.stepdown };
    let passes = passes_for(op.depth, stepdown);
    let lens = lengths(path);
    let l = *lens.last().unwrap();

    // tab spans along the lap
    let mut spans: Vec<(f64, f64)> = Vec::new();
    let w = op.tab_width + tool.diameter;
    // tabs placed by hand: the points that fall on this cut (the cutter path runs one radius off the outline)
    let by_hand: Vec<f64> = if op.tabs && op.tab_height > 0.0 && l > 0.0 { op.tab_points.iter().filter_map(|p| along(path, &lens, pt(p[0], p[1]), tool.diameter + 0.25)).collect() } else { Vec::new() };
    if !by_hand.is_empty() {
        spans = by_hand.iter().map(|&c| (c - w / 2.0, c + w / 2.0)).collect();
    } else if op.tabs && op.tab_count > 0 && op.tab_height > 0.0 && l > 0.0 {
        if w * op.tab_count as f64 >= l {
            warn.push(format!("{}: tabs don't fit, skipped", op.name));
        } else {
            for i in 0..op.tab_count {
                let c = (i as f64 + 0.5) * l / op.tab_count as f64;
                spans.push((c - w / 2.0, c + w / 2.0));
            }
        }
    }
    let local = |s: f64| if closed && l > 0.0 { s - (s / l).floor() * l } else { s };
    let in_tab = |s: f64| {
        let s = local(s);
        spans.iter().any(|&(a, b)| s > a && s < b)
    };

    // laps: nominal Z per point
    let mut pts: Vec<P> = Vec::new();
    let lap = |pts: &mut Vec<P>, xy: &[Pt], s: &[f64], z0: f64, z1: f64, total: f64, off: f64| {
        for i in 0..xy.len() {
            if i == 0 {
                if let Some(b) = pts.last() {
                    if dist(b.xy, xy[0]) < 1e-12 && b.z == z0 {
                        continue; // lap joins
                    }
                }
            }
            let z = if total > 0.0 { z0 + (z1 - z0) * (s[i] / total).min(1.0) } else { z1 };
            pts.push(P { xy: xy[i], z, s: s[i] + off, in_tab: false });
        }
    };

    let pf = passes as f64;
    if closed && (op.entry == Entry::Spiral || op.entry == Entry::Ramp) {
        let fine = subdivide(path, RAMP_SEG);
        let fs = lengths(&fine);
        let over = if op.entry == Entry::Spiral { l } else { clampf(op.ramp_length, RAMP_SEG, l) }; // ramp distance per pass
        for k in 0..passes {
            let kf = k as f64;
            lap(&mut pts, &fine, &fs, -op.depth * kf / pf, -op.depth * (kf + 1.0) / pf, over, kf * l);
        }
        lap(&mut pts, path, &lens, -op.depth, -op.depth, 0.0, pf * l); // flat finishing lap
    } else {
        let fwd: Vec<Pt> = path.to_vec();
        let rev: Vec<Pt> = path.iter().rev().copied().collect();
        for k in 0..passes {
            let kf = k as f64;
            let z = -op.depth * (kf + 1.0) / pf;
            let back = !closed && k % 2 == 1; // open paths zig-zag instead of retracting
            let xy = if back { &rev } else { &fwd };
            let mut s = lengths(xy);
            if back {
                for v in &mut s {
                    *v = l - *v; // keep s measured from path start so tabs line up
                }
            }
            let off = if closed { kf * l } else { 0.0 };
            if op.entry == Entry::Ramp && !closed {
                // ramp from the previous pass depth to this one over ramp_length, then flat
                let z_prev = -op.depth * kf / pf;
                let fine = subdivide(xy, RAMP_SEG);
                let mut fs = lengths(&fine);
                if back {
                    for v in &mut fs {
                        *v = l - *v;
                    }
                }
                let along = lengths(&fine);
                let over = clampf(op.ramp_length, RAMP_SEG, l);
                for i in 0..fine.len() {
                    pts.push(P { xy: fine[i], z: z_prev + (z - z_prev) * (along[i] / over).min(1.0), s: fs[i] + off, in_tab: false });
                }
                continue;
            }
            if !pts.is_empty() {
                pts.push(P { xy: xy[0], z, s: s[0] + off, in_tab: false }); // plunge in place
            }
            lap(&mut pts, xy, &s, z, z, 0.0, off);
        }
    }

    // tabs: split at span edges (and ramp edges for 3D tabs), then raise Z over the tab
    let ramp = if op.tab_ramped { op.tab_height.max(0.05) } else { 0.0 }; // 3D tabs: ~45 degree ramps, never shorter than 0.05"
    let tab_lift = |s: f64| {
        // how far above the floor (-depth) the tab profile is at s, 0 outside
        let s = local(s);
        let mut lift: f64 = 0.0;
        for &(a, b) in &spans {
            if s >= a && s <= b {
                return op.tab_height;
            }
            if ramp > 0.0 {
                let mut d_out = if s < a { a - s } else { s - b }; // distance outside the span (consider wrap for closed paths)
                if closed && l > 0.0 {
                    d_out = d_out.min((s + l - a).abs().min((s - l - b).abs()));
                }
                if d_out < ramp {
                    lift = lift.max(op.tab_height * (1.0 - d_out / ramp));
                }
            }
        }
        lift
    };
    let mut out: Vec<P> = Vec::new();
    if spans.is_empty() {
        out = pts;
    } else {
        for i in 0..pts.len() {
            let mut b = pts[i];
            b.in_tab = in_tab(b.s);
            if !out.is_empty() {
                let a = pts[i - 1];
                let mut cuts: Vec<f64> = Vec::new();
                let (lo, hi) = (a.s.min(b.s), a.s.max(b.s));
                let k0 = if closed && l > 0.0 { (lo / l).floor() as i32 } else { 0 };
                let k1 = if closed && l > 0.0 { (hi / l).floor() as i32 } else { 0 };
                for k in k0..=k1 {
                    let kl = k as f64 * l;
                    for &(x, y) in &spans {
                        for e in [x + kl, y + kl, x + kl - ramp, y + kl + ramp] {
                            if e > lo && e < hi {
                                cuts.push(e);
                            }
                        }
                    }
                }
                if a.s < b.s {
                    cuts.sort_by(|u, v| u.partial_cmp(v).unwrap());
                } else {
                    cuts.sort_by(|u, v| v.partial_cmp(u).unwrap());
                }
                cuts.dedup_by(|v, u| (*u - *v).abs() < 1e-9);
                for e in cuts {
                    let t = (e - a.s) / (b.s - a.s);
                    let entering = in_tab(e + if b.s > a.s { 1e-9 } else { -1e-9 });
                    let q = P { xy: lerp(a.xy, b.xy, t), z: a.z + (b.z - a.z) * t, s: e, in_tab: !entering };
                    out.push(q);
                    out.push(P { in_tab: entering, ..q });
                }
            }
            out.push(b);
        }
        for p in &mut out {
            let lift = if op.tab_ramped { tab_lift(p.s) } else if p.in_tab { op.tab_height } else { 0.0 }; // square tabs keep vertical walls
            if lift > 0.0 {
                p.z = p.z.max(-op.depth + lift);
            }
        }
    }

    let mut tp = Toolpath { name: op.name.clone(), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };
    rapid_to(&mut tp, out[0].xy, Some(m.retract_z));
    for p in &out {
        let last = tp.moves.last().unwrap();
        if last.kind == MoveType::Linear && last.x == Some(p.xy.x) && last.y == Some(p.xy.y) && last.z == Some(p.z) {
            continue;
        }
        cut_to(&mut tp, p.xy, Some(p.z));
    }
    rapid_to(&mut tp, out.last().unwrap().xy, Some(m.retract_z));
    tp
}

fn depth_checks(op: &Op, m: &Material, res: &mut CamResult) -> bool {
    if op.depth <= 0.0 {
        res.warnings.push(format!("{}: depth must be > 0", op.name));
        return false;
    }
    if op.depth > m.t + 1e-9 {
        res.warnings.push(format!("{}: depth is deeper than the material", op.name));
    }
    true
}

fn indexed_name(name: &str, i: usize) -> String {
    if i > 0 {
        format!("{name} [{i}]")
    } else {
        name.to_string()
    }
}

// ---- generators -----------------------------------------------------------------------------------------------------

/// Lead-in / lead-out for a closed cut ring (`ring[0] == ring[last]`): the loop becomes
/// A → quarter arc → start → around the ring → start → quarter arc → B → A, so every pass plunges at A in the waste
/// and meets the line tangentially (no plunge mark on the part edge). `right` = the waste is on the right of the
/// travel direction. `keep_inside`: for inside cuts the leads must stay within the ring (None = no check).
fn with_leads(ring: &[Pt], radius: f64, right: bool, keep_inside: Option<&PathD>) -> Option<Vec<Pt>> {
    let n = ring.len();
    if n < 3 || radius <= 0.0 {
        return None;
    }
    // start in the middle of the longest straight run, not at a corner: there the arcs have room on both sides
    let k = (0..n - 1).max_by(|&a, &b| dist(ring[a], ring[a + 1]).total_cmp(&dist(ring[b], ring[b + 1])))?;
    let mid = lerp(ring[k], ring[k + 1], 0.5);
    let mut r2 = vec![mid];
    r2.extend_from_slice(&ring[k + 1..n - 1]);
    r2.extend_from_slice(&ring[..=k]);
    r2.push(mid);
    let (ring, n) = (&r2[..], r2.len());
    let unit = |a: Pt, b: Pt| { let d = dist(a, b); (d > 1e-12).then(|| pt((b.x - a.x) / d, (b.y - a.y) / d)) };
    let (t_in, t_out) = (unit(ring[0], ring[1])?, unit(ring[n - 2], ring[n - 1])?);
    let waste = |t: Pt| if right { pt(t.y, -t.x) } else { pt(-t.y, t.x) };
    let s = ring[0];
    const STEPS: usize = 8;
    let arc = |t: Pt, lead_in: bool| -> Vec<Pt> {
        let w = waste(t);
        let c = pt(s.x + w.x * radius, s.y + w.y * radius);
        (0..=STEPS).map(|k| {
            let a = std::f64::consts::FRAC_PI_2 * k as f64 / STEPS as f64;
            // in: from behind the start round to it; out: from it round and away, both tangent to the cut
            let (ft, fw) = if lead_in { (-a.cos(), -a.sin()) } else { (a.sin(), -a.cos()) };
            pt(c.x + (t.x * ft + w.x * fw) * radius, c.y + (t.y * ft + w.y * fw) * radius)
        }).collect()
    };
    let (lead_in, lead_out) = (arc(t_in, true), arc(t_out, false));
    if let Some(poly) = keep_inside {
        if lead_in.iter().chain(&lead_out).take(STEPS).chain(lead_out.iter().skip(1)).any(|&p| !inside(p, poly)) {
            return None;
        }
    }
    let mut out = lead_in; // ends at the start point
    out.extend_from_slice(&ring[1..]);
    out.extend_from_slice(&lead_out[1..]);
    out.push(out[0]); // back to A through the waste: the next pass plunges there
    Some(out)
}

/// contours: the op's shapes (nesting among them decides holes). Each resulting path starts with a rapid to retract
/// height and ends with a retract.
pub fn generate_profile(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    let mut res = CamResult::default();
    if !depth_checks(op, m, &mut res) {
        return res;
    }
    let r = tool.diameter / 2.0;
    let polys: Vec<PathD> = contours.iter().map(to_path).collect();
    // bounding boxes: the hole test below is shapes × shapes, and a box test first makes it cheap on big drawings
    let boxes: Vec<[f64; 4]> = polys
        .iter()
        .map(|p| p.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, q| [b[0].min(q.x), b[1].min(q.y), b[2].max(q.x), b[3].max(q.y)]))
        .collect();

    for (i, c) in contours.iter().enumerate() {
        let mut poly = polys[i].clone();
        if poly.len() < 2 {
            continue;
        }
        if !c.closed {
            if op.side != Side::On {
                res.warnings.push(format!("{}: open shape cut on-line", op.name));
            }
            let tp = cut_path(&pts_of(&poly), false, op, tool, m, &mut res.warnings);
            res.paths.push(tp);
            continue;
        }

        // hole = inside an odd number of the op's other closed shapes
        let p0 = pt(poly[0].x, poly[0].y);
        let in_box = |j: usize| p0.x >= boxes[j][0] && p0.x <= boxes[j][2] && p0.y >= boxes[j][1] && p0.y <= boxes[j][3];
        let depth = (0..contours.len()).filter(|&j| j != i && contours[j].closed && polys[j].len() > 2 && in_box(j) && inside(p0, &polys[j])).count();
        let hole = depth % 2 == 1;

        if !is_positive(&poly) {
            poly.reverse(); // CCW
        }
        let mut offs: PathsD = vec![poly.clone()];
        // separate last pass (VCarve "Do Separate Last Pass"): the stepped passes stay `allowance` off the line, then
        // one full-depth pass on the line takes the rest off, so the edge is cut once, lightly, without step marks
        let mut rough: PathsD = Vec::new();
        let mut waste_outside = None; // which side of the cut ring the waste is on (None: cutting on the line)
        if op.side != Side::On {
            let delta = if (op.side == Side::Outside) != hole { r } else { -r };
            waste_outside = Some(delta > 0.0);
            offs = offset_region(&vec![poly.clone()], delta);
            if offs.is_empty() {
                res.warnings.push(format!("{}: a shape is too small for the tool", op.name));
                continue;
            }
            if op.allowance > 0.0 {
                rough = offset_region(&vec![poly], delta + op.allowance.copysign(delta));
            }
        }
        // Climb milling with a CW spindle keeps the material on the cutter's right (G41 side): outside cuts run CW,
        // inside cuts CCW, matching VCarve (samples/compare). Pocket walls and bores follow the same rule.
        let mut ccw = (op.side == Side::Inside) != hole;
        if !op.climb {
            ccw = !ccw;
        }
        let finish_op = Op { stepdown: op.depth, ..op.clone() }; // the last pass: full depth in one go
        let split = rough.len(); // rough rings first, then the finishing rings, all in one toolpath so they stay in order
        let mut merged: Option<Toolpath> = None;
        for (n, mut o) in rough.into_iter().chain(offs).enumerate() {
            let op = if split > 0 && n >= split { &finish_op } else { op };
            if is_positive(&o) != ccw {
                o.reverse();
            }
            // start at the lowest point (then leftmost): stable and matches VCarve's circles; with `start_first` at
            // the point nearest the vector's first node, which the person set with Make Start Point
            let mut it = 0;
            if op.start_first && !c.nodes.is_empty() {
                let f = c.nodes[0].p;
                let d = |q: &PointD| (q.x - f.x).hypot(q.y - f.y);
                it = (0..o.len()).min_by(|&a, &b| d(&o[a]).total_cmp(&d(&o[b]))).unwrap_or(0);
            } else {
                for k in 1..o.len() {
                    let (a, b) = (o[k], o[it]);
                    if a.y < b.y - 1e-9 || ((a.y - b.y).abs() <= 1e-9 && a.x < b.x) {
                        it = k;
                    }
                }
            }
            o.rotate_left(it);
            let mut pts = pts_of(&o);
            pts.push(pts[0]);
            if let (Some(out), true) = (waste_outside, op.ramp_in > 0.0) {
                match with_leads(&pts, op.ramp_in, ccw == out, (!out).then_some(&o)) {
                    Some(p) => pts = p,
                    None => res.warnings.push(format!("{}: no room for the lead-in in a small shape, plunging on the line there", op.name)),
                }
            }
            let tp = cut_path(&pts, true, op, tool, m, &mut res.warnings);
            if split == 0 {
                res.paths.push(tp);
            } else if let Some(all) = &mut merged {
                all.moves.extend(tp.moves); // each ring starts and ends at the retract height
            } else {
                merged = Some(tp);
            }
        }
        res.paths.extend(merged);
    }
    // name repeats VCarve-style: "Profile", "Profile [1]", ...
    for (i, p) in res.paths.iter_mut().enumerate().skip(1) {
        p.name = indexed_name(&op.name, i);
    }
    res
}

/// Offset-strategy pocket: concentric rings from the boundary inward, cut inside-out per depth pass.
/// ponytail: no raster strategy yet; add raster when a user needs it for long thin pockets.
pub fn generate_pocket(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    let mut res = CamResult::default();
    if !depth_checks(op, m, &mut res) {
        return res;
    }
    // Engraving fill (VCarve "Quick Engrave"): a V-bit or engraving bit only cuts as wide as its cone is at the cut
    // depth, so the walls are set in and the rings / rows spaced by that width, not by the bit's full diameter.
    let engrave;
    let tool = if tool.kind == ToolType::VBit && tool.angle > 0.0 && tool.angle < 180.0 {
        engrave = Tool { diameter: (2.0 * op.depth * (tool.angle.to_radians() / 2.0).tan()).clamp(0.004, tool.diameter.max(0.004)), ..tool.clone() };
        &engrave
    } else {
        tool
    };
    let r = tool.diameter / 2.0;
    let step = clampf(op.stepover, 0.05, 1.0) * tool.diameter;

    let mut polys = PathsD::new();
    for c in contours {
        if !c.closed {
            res.warnings.push(format!("{}: open shapes can't be pocketed, skipped", op.name));
            continue;
        }
        let p = to_path(c);
        if p.len() >= 3 {
            polys.push(p);
        }
    }
    if polys.is_empty() {
        return res;
    }
    // Even-odd union: nested shapes become islands, overlapping shapes merge. Holes come out clockwise.
    let region = union_subjects_d(&polys, FillRule::EvenOdd, PREC);
    let boundary = offset_region(&region, -(r + op.allowance));
    if boundary.is_empty() {
        res.warnings.push(format!("{}: pocket is too small for the tool", op.name));
        return res;
    }
    if op.raster {
        res.paths.push(pocket_raster(op, tool, &boundary, step, m));
        return res;
    }
    // Each ring is the previous one moved in by a stepover (as the C++ engine does). Round joins add points ring after
    // ring (a 40 in pocket reached 1.3 million, a panel of 278 shapes 7 million), so a heavy ring is thinned by a
    // tenth of the arc tolerance before it is offset again: the shape stays within 0.00005 in per level, the point
    // count stays near the boundary's, and each offset stays cheap (offsetting from the boundary every time was 0.4 s
    // a ring on a busy panel).
    let mut levels = vec![boundary.clone()];
    loop {
        let mut prev = levels.last().unwrap().clone();
        if prev.iter().map(Vec::len).sum::<usize>() > 2000 {
            prev = simplify_paths(&prev, TOL / 10.0, true);
        }
        let next = offset_region(&prev, -step);
        if next.is_empty() {
            break;
        }
        levels.push(next);
        if levels.len() > 10000 {
            break; // safety
        }
    }
    let prof = std::env::var_os("GCARVE_PROF").is_some();
    if prof {
        eprintln!("pocket: {} levels, {} rings, {} points, boundary {} points", levels.len(), levels.iter().map(Vec::len).sum::<usize>(), levels.iter().flatten().map(Vec::len).sum::<usize>(), boundary.iter().map(Vec::len).sum::<usize>());
    }
    let inside_boundary = |q: Pt| boundary.iter().filter(|p| inside(q, p)).count() % 2 == 1;

    let mut rings: Vec<Vec<Pt>> = Vec::new();
    for (mut p, _) in order_rings(&levels) {
        let hole = !is_positive(&p);
        let mut ccw = !hole; // climb with CW spindle: outer walls CCW, island walls CW
        if !op.climb {
            ccw = !ccw;
        }
        if is_positive(&p) != ccw {
            p.reverse();
        }
        let mut pts = pts_of(&p);
        pts.push(pts[0]);
        rings.push(pts);
    }
    if rings.is_empty() {
        return res;
    }

    let stepdown = if op.stepdown > 0.0 { op.stepdown } else { tool.stepdown };
    let passes = passes_for(op.depth, stepdown);
    let pf = passes as f64;
    let mut tp = Toolpath { name: op.name.clone(), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };

    let mut cur = rings[0][0];
    rapid_to(&mut tp, cur, Some(m.retract_z));
    let mut first = true;
    for k in 0..passes {
        let kf = k as f64;
        let z = -op.depth * (kf + 1.0) / pf;
        for (ri, ring) in rings.iter().enumerate() {
            // start each ring at its point nearest to where the cutter is
            let (mut bi, mut bd) = (0, 1e300);
            for (j, &q) in ring[..ring.len() - 1].iter().enumerate() {
                let d = dist(cur, q);
                if d < bd {
                    bd = d;
                    bi = j;
                }
            }
            let mut lp: Vec<Pt> = ring[bi..ring.len() - 1].to_vec();
            lp.extend_from_slice(&ring[..bi]);
            lp.push(lp[0]);

            // link: stay down if the straight move stays inside the pocket (already-cleared or to-be-cleared stock), else retract
            let mut safe = !first;
            let mut t = 1;
            while t < 8 && safe {
                safe = inside_boundary(lerp(cur, lp[0], t as f64 / 8.0)); // endpoints lie on rings, so test the interior only
                t += 1;
            }
            if !safe && !first {
                rapid_to(&mut tp, cur, Some(m.retract_z));
                rapid_to(&mut tp, lp[0], Some(m.retract_z));
            } else if !first {
                let lz = last_z(&tp);
                cut_to(&mut tp, lp[0], lz);
            }
            if op.entry == Entry::Ramp && ri == 0 {
                // first ring of the pass: ramp down along the loop instead of plunging
                let z_prev = -op.depth * kf / pf;
                let over = RAMP_SEG.max(op.ramp_length);
                let mut run = 0.0;
                let fine = subdivide(&lp, RAMP_SEG);
                for j in 0..fine.len() {
                    if j > 0 {
                        run += dist(fine[j - 1], fine[j]);
                    }
                    cut_to(&mut tp, fine[j], Some(z_prev + (z - z_prev) * (run / over).min(1.0)));
                }
                if run < over {
                    for &f in &fine[1..] {
                        cut_to(&mut tp, f, Some(z)); // short loop: one more flat lap
                    }
                }
            } else {
                cut_to(&mut tp, lp[0], Some(z)); // plunge (or already there)
                for &q in &lp[1..] {
                    cut_to(&mut tp, q, Some(z));
                }
            }
            cur = *lp.last().unwrap();
            first = false;
        }
    }
    rapid_to(&mut tp, cur, Some(m.retract_z));
    res.paths.push(tp);
    res
}

/// Two-bit pocket (VCarve "Use Larger Area Clearance Tool"): a larger end mill clears everything it can reach,
/// then the op's own (smaller) bit cuts only what was left: the corners and narrow parts. Two toolpaths, the first
/// with `Toolpath::tool` = the clearance bit so it posts to that bit's file. Without a larger clearance bit this is
/// the plain pocket.
pub fn generate_pocket_two(op: &Op, tool: &Tool, contours: &[Contour], m: &Material, clear_tool: Option<&Tool>) -> CamResult {
    let Some(big) = clear_tool.filter(|c| c.kind == ToolType::EndMill && c.diameter > tool.diameter + 1e-6) else {
        return generate_pocket(op, tool, contours, m);
    };
    let mut res = generate_pocket(op, big, contours, m);
    for p in &mut res.paths {
        p.name = format!("{} [clear]", op.name);
        p.tool = op.clear_tool;
    }
    let polys: PathsD = contours.iter().filter(|c| c.closed).map(to_path).filter(|p| p.len() >= 3).collect();
    if polys.is_empty() {
        return res;
    }
    // the pocket's finished outline (walls keep the allowance), what the big bit reaches of it, and the rest
    let region = offset_region(&union_subjects_d(&polys, FillRule::EvenOdd, PREC), -op.allowance);
    let (rb, rs) = (big.diameter / 2.0, tool.diameter / 2.0);
    let reached = offset_region(&offset_region(&region, -rb), rb);
    let rest = difference_d(&region, &reached, FillRule::NonZero, PREC);
    // slivers thinner than a few thousandths are only the rounding of the arcs: nothing to cut there
    let rest = offset_region(&offset_region(&rest, -0.003), 0.003);
    if rest.is_empty() {
        res.warnings.retain(|w| !w.contains("too small")); // nothing left for the small bit either way
        return res;
    }
    if res.paths.is_empty() {
        res.warnings.clear(); // the big bit doesn't fit at all: the small one does the whole pocket
        return generate_pocket(op, tool, contours, m);
    }
    // the small bit pockets the rest, widened by its own diameter so it enters from the cleared side, inside the walls
    let area = intersect_d(&offset_region(&rest, 2.0 * rs), &region, FillRule::NonZero, PREC);
    let small = Op { allowance: 0.0, raster: false, ..op.clone() };
    let mut fine = generate_pocket(&small, tool, &region_contours(&area), m);
    res.paths.append(&mut fine.paths);
    res.warnings.extend(fine.warnings.into_iter().filter(|w| !w.contains("too small")));
    res
}

/// Raster pocket clearing (VCarve "Raster"): back-and-forth rows `step` apart along `op.raster_angle` inside
/// `boundary` (the bit-centre region), then one lap round every wall to take off the scallops the row ends leave.
/// ponytail: a row that can't reach the next one in a straight line inside the pocket (islands, narrow necks) lifts
/// and rapids; chain the row pieces region by region if pockets with many islands lift too much.
fn pocket_raster(op: &Op, tool: &Tool, boundary: &PathsD, step: f64, m: &Material) -> Toolpath {
    let (sn, cs) = (op.raster_angle as f64).to_radians().sin_cos();
    let rot = |p: Pt| pt(p.x * cs + p.y * sn, -p.x * sn + p.y * cs); // rows become horizontal
    let back = |p: Pt| pt(p.x * cs - p.y * sn, p.x * sn + p.y * cs);
    let polys: Vec<Vec<Pt>> = boundary.iter().map(|p| pts_of(p).into_iter().map(rot).collect()).collect();
    let (y0, y1) = polys.iter().flatten().fold((1e300_f64, -1e300_f64), |(a, b), p| (a.min(p.y), b.max(p.y)));
    // rows: even-odd crossings of each scan line, as (x from, x to) pieces left to right
    let mut rows: Vec<(f64, Vec<(f64, f64)>)> = Vec::new();
    let n = (((y1 - y0) / step).ceil() as usize).max(1);
    for k in 0..=n {
        let y = (y0 + (y1 - y0) * k as f64 / n as f64).clamp(y0 + 1e-6, y1 - 1e-6); // first / last row just inside the walls
        let mut xs: Vec<f64> = Vec::new();
        for p in &polys {
            for i in 0..p.len() {
                let (a, b) = (p[i], p[(i + 1) % p.len()]);
                if (a.y <= y) != (b.y <= y) {
                    xs.push(a.x + (b.x - a.x) * (y - a.y) / (b.y - a.y));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        let pieces: Vec<(f64, f64)> = xs.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).filter(|c| c.1 - c.0 > 1e-6).collect();
        if !pieces.is_empty() {
            rows.push((y, pieces));
        }
    }
    let inside_boundary = |q: Pt| boundary.iter().filter(|p| inside(q, p)).count() % 2 == 1;
    let stepdown = if op.stepdown > 0.0 { op.stepdown } else { tool.stepdown };
    let passes = passes_for(op.depth, stepdown);
    let mut tp = Toolpath { name: op.name.clone(), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };
    for k in 0..passes {
        let (z_prev, z) = (-op.depth * k as f64 / passes as f64, -op.depth * (k + 1) as f64 / passes as f64);
        let mut cur: Option<Pt> = None;
        let mut flip = false;
        for (y, pieces) in &rows {
            let mut segs: Vec<(Pt, Pt)> = pieces.iter().map(|&(a, b)| (back(pt(a, *y)), back(pt(b, *y)))).collect();
            if flip {
                segs.reverse();
                for s in &mut segs {
                    *s = (s.1, s.0);
                }
            }
            flip = !flip;
            for (a, b) in segs {
                // stay down when the straight link to the next row piece lies inside the pocket
                let down = cur.is_some_and(|c| (1..8).all(|t| inside_boundary(lerp(c, a, t as f64 / 8.0))));
                if down {
                    cut_to(&mut tp, a, Some(z));
                } else {
                    if let Some(c) = cur {
                        rapid_to(&mut tp, c, Some(m.retract_z));
                    }
                    rapid_to(&mut tp, a, Some(m.retract_z));
                    if op.entry == Entry::Plunge {
                        cut_to(&mut tp, a, Some(z));
                    } else {
                        // ramp down along the row piece, then come back along it at depth
                        cut_to(&mut tp, a, Some(z_prev));
                        cut_to(&mut tp, b, Some(z));
                        cut_to(&mut tp, a, Some(z));
                    }
                }
                cut_to(&mut tp, b, Some(z));
                cur = Some(b);
            }
        }
        // walls: the region's outer rings run CCW and its island rings CW, which is climb (same rule as the ring pocket)
        for p in boundary {
            let mut ring = p.clone();
            if !op.climb {
                ring.reverse();
            }
            let mut pts = pts_of(&ring);
            pts.push(pts[0]);
            let down = cur.is_some_and(|c| (1..8).all(|t| inside_boundary(lerp(c, pts[0], t as f64 / 8.0))));
            if !down {
                if let Some(c) = cur {
                    rapid_to(&mut tp, c, Some(m.retract_z));
                }
                rapid_to(&mut tp, pts[0], Some(m.retract_z));
            }
            for &q in &pts {
                cut_to(&mut tp, q, Some(z));
            }
            cur = pts.last().copied();
        }
        if let Some(c) = cur {
            rapid_to(&mut tp, c, Some(m.retract_z));
        }
    }
    tp
}

/// Straight (peck) drilling at `ctr`: full retract between pecks. Shared by drill and the small-hole bore fallback.
fn peck_drill(tp: &mut Toolpath, ctr: Pt, op: &Op, m: &Material) {
    rapid_to(tp, ctr, Some(m.retract_z));
    let peck = if op.peck > 0.0 { op.peck } else { op.depth };
    let mut d = peck.min(op.depth);
    loop {
        cut_to(tp, ctr, Some(-d));
        rapid_to(tp, ctr, Some(m.retract_z));
        if d >= op.depth - 1e-9 {
            break;
        }
        d = (d + peck).min(op.depth);
    }
}

/// Drills at the centre of each shape's bounding box (circles -> their centre). Full retract between pecks.
pub fn generate_drill(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    let mut res = CamResult::default();
    if !depth_checks(op, m, &mut res) {
        return res;
    }
    for c in contours {
        let b = bounds(c);
        if b.is_empty() {
            continue;
        }
        if b.w() > tool.diameter * 1.05 || b.h() > tool.diameter * 1.05 {
            res.warnings.push(format!("{}: a hole is bigger than the drill, only its centre is drilled", op.name));
        }
        let ctr = pt((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
        let mut tp = Toolpath { name: indexed_name(&op.name, res.paths.len()), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };
        peck_drill(&mut tp, ctr, op, m);
        res.paths.push(tp);
    }
    res
}

// ---- medial axis (Voronoi of the outline segments, inner edges only) ------------------------------------------------
// Each skeleton point carries r = distance to the nearest outline; a V-bit on the skeleton at depth r / tan(angle/2)
// carves exactly up to the outline, so following the skeleton carves the whole shape (VCarve-style, not offset rings).
#[derive(Clone, Copy, Debug)]
struct SkelPt {
    p: Pt,
    r: f64,
}

type Key = (i64, i64);
fn key(p: Pt) -> Key {
    ((p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64)
}

fn region_boxes(region: &PathsD) -> Vec<Rect> {
    region
        .iter()
        .map(|p| {
            let mut b = Rect::default();
            for q in p {
                b.add(pt(q.x, q.y));
            }
            b
        })
        .collect()
}
fn in_box(b: &Rect, q: Pt, pad: f64) -> bool {
    q.x >= b.x0 - pad && q.x <= b.x1 + pad && q.y >= b.y0 - pad && q.y <= b.y1 + pad
}
fn inside_region(region: &PathsD, boxes: &[Rect], q: Pt) -> bool {
    region.iter().zip(boxes).filter(|(p, b)| in_box(b, q, 0.0) && inside(q, p)).count() % 2 == 1
}

fn medial_axis(region: &PathsD) -> Vec<Vec<SkelPt>> {
    const S: f64 = 1e5; // integer coordinates at 1e-5 in
    let mut segs: Vec<[i32; 4]> = Vec::new();
    for p in region {
        for i in 0..p.len() {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            let sg = [(a.x * S).round() as i32, (a.y * S).round() as i32, (b.x * S).round() as i32, (b.y * S).round() as i32];
            if sg[0..2] != sg[2..4] {
                segs.push(sg);
            }
        }
    }
    let mut pieces: Vec<Vec<SkelPt>> = Vec::new();
    if segs.len() < 3 {
        return pieces;
    }
    let Ok(vd) = VBuilder::<i32>::default().with_segments(segs.iter()).and_then(|b| b.build()) else {
        return pieces; // ponytail: a Voronoi failure carves nothing (warned as "nothing to carve")
    };
    let seg_pt = |i: usize, high: bool| if high { pt(segs[i][2] as f64 / S, segs[i][3] as f64 / S) } else { pt(segs[i][0] as f64 / S, segs[i][1] as f64 / S) };
    let site_dist = |c: &boostvoronoi::prelude::Cell, q: Pt| {
        let i = c.source_index().usize();
        if c.contains_point() {
            dist(q, seg_pt(i, c.source_category() == SourceCategory::SegmentEnd))
        } else {
            dist_to_segment(q, seg_pt(i, false), seg_pt(i, true))
        }
    };
    let boxes = region_boxes(region);
    let vpt = |v| {
        let v = vd.vertex(v).unwrap();
        pt(v.x() / S, v.y() / S)
    };
    for e in vd.edges() {
        let eid = e.id();
        let Ok(twin) = e.twin() else { continue };
        if !e.is_primary() || !vd.edge_is_finite(eid).unwrap_or(false) || eid.usize() > twin.usize() {
            continue;
        }
        let tw = vd.edge(twin).unwrap();
        let (a, b) = (vpt(e.vertex0().unwrap()), vpt(tw.vertex0().unwrap()));
        let cell = vd.cell(e.cell().unwrap()).unwrap();
        let tcell = vd.cell(tw.cell().unwrap()).unwrap();
        let mut pts: Vec<Pt> = Vec::new();
        if e.is_curved() {
            // parabola: focus = the point site, directrix = the segment site
            let (pc, sc) = if cell.contains_point() { (cell, tcell) } else { (tcell, cell) };
            let f = seg_pt(pc.source_index().usize(), pc.source_category() == SourceCategory::SegmentEnd);
            let (p0, p1) = (seg_pt(sc.source_index().usize(), false), seg_pt(sc.source_index().usize(), true));
            let mut u = p1 - p0;
            let l = u.x.hypot(u.y);
            u = pt(u.x / l, u.y / l);
            let n = pt(-u.y, u.x);
            let steps = ((dist(a, b) / 0.01).ceil() as i32).max(2);
            for k in 0..=steps {
                if k == 0 {
                    pts.push(a);
                    continue;
                }
                if k == steps {
                    pts.push(b);
                    continue;
                }
                let q = lerp(a, b, k as f64 / steps as f64);
                let t = (q.x - p0.x) * u.x + (q.y - p0.y) * u.y;
                let dd = pt(p0.x + u.x * t, p0.y + u.y * t);
                let d = pt(dd.x - f.x, dd.y - f.y);
                let nd = n.x * d.x + n.y * d.y;
                if nd.abs() < 1e-12 {
                    pts.push(q);
                    continue;
                }
                let h = -(d.x * d.x + d.y * d.y) / (2.0 * nd);
                pts.push(pt(dd.x + n.x * h, dd.y + n.y * h));
            }
        } else {
            pts = vec![a, b];
        }
        let half = pts.len() / 2;
        let mid = lerp(pts[half - if pts.len().is_multiple_of(2) { 1 } else { 0 }], pts[half], 0.5);
        if !inside_region(region, &boxes, mid) {
            continue;
        }
        pieces.push(pts.into_iter().map(|q| SkelPt { p: q, r: site_dist(cell, q) }).collect());
    }
    // join pieces into chains at shared endpoints; chains stop at junctions (degree != 2)
    let mut at: HashMap<Key, Vec<usize>> = HashMap::new();
    for (i, pc) in pieces.iter().enumerate() {
        at.entry(key(pc[0].p)).or_default().push(i);
        at.entry(key(pc[pc.len() - 1].p)).or_default().push(i);
    }
    let deg = |p: Pt| at.get(&key(p)).map_or(0, |v| v.len());
    let mut used = vec![false; pieces.len()];
    let mut chains: Vec<Vec<SkelPt>> = Vec::new();
    let mut walk = |start: usize, from_front: bool, used: &mut Vec<bool>| {
        let mut chain: Vec<SkelPt> = Vec::new();
        let (mut i, mut forward) = (start, from_front);
        loop {
            used[i] = true;
            let pc = &pieces[i];
            let k0 = if chain.is_empty() { 0 } else { 1 };
            for k in k0..pc.len() {
                chain.push(if forward { pc[k] } else { pc[pc.len() - 1 - k] });
            }
            let nb = &at[&key(chain[chain.len() - 1].p)];
            if nb.len() != 2 {
                break;
            }
            let j = if nb[0] == i { nb[1] } else { nb[0] };
            if used[j] {
                break;
            }
            forward = key(pieces[j][0].p) == key(chain[chain.len() - 1].p);
            i = j;
        }
        chains.push(chain);
    };
    for i in 0..pieces.len() {
        // start from endpoints / junctions
        if used[i] {
            continue;
        }
        let f = deg(pieces[i][0].p) != 2;
        let b = deg(pieces[i][pieces[i].len() - 1].p) != 2;
        if f {
            walk(i, true, &mut used);
        } else if b {
            walk(i, false, &mut used);
        }
    }
    for i in 0..pieces.len() {
        if !used[i] {
            walk(i, true, &mut used); // leftover loops
        }
    }
    chains
}

/// Depth-first walk over the skeleton graph (junctions where branches meet), retracing branches to get back to the
/// junction, so a whole shape is carved without lifting.
struct SkelWalk<'a> {
    skeleton: &'a [Vec<SkelPt>],
    adj: HashMap<Key, Vec<usize>>,
    done: Vec<bool>,
    max_depth: f64,
    start: f64,
    tan_half: f64,
    cur: Pt,
}
impl SkelWalk<'_> {
    fn z_at(&self, r: f64) -> f64 {
        -self.max_depth.min(self.start + r / self.tan_half)
    }
    fn emit_chain(&self, tp: &mut Toolpath, j: usize, reverse: bool) {
        let ch = &self.skeleton[j];
        if !reverse {
            for s in &ch[1..] {
                cut_to(tp, s.p, Some(self.z_at(s.r)));
            }
        } else {
            for s in ch[..ch.len() - 1].iter().rev() {
                cut_to(tp, s.p, Some(self.z_at(s.r)));
            }
        }
    }
    fn adj_of(&self, p: Pt) -> &[usize] {
        self.adj.get(&key(p)).map_or(&[], |v| v.as_slice())
    }
    fn walk(&mut self, tp: &mut Toolpath, at: Pt, must_return: bool) {
        let branches: Vec<usize> = self.adj_of(at).iter().copied().filter(|&j| !self.done[j]).collect();
        for (k, &j) in branches.iter().enumerate() {
            if self.done[j] {
                continue;
            }
            self.done[j] = true;
            let ch = &self.skeleton[j];
            let from_front = key(ch[0].p) == key(at);
            self.emit_chain(tp, j, !from_front);
            let far = if from_front { ch[ch.len() - 1].p } else { ch[0].p };
            let more = self.adj_of(far).iter().any(|&q| !self.done[q]);
            let ret = must_return || k + 1 < branches.len();
            if more {
                self.walk(tp, far, ret);
            }
            if ret {
                self.emit_chain(tp, j, from_front); // retrace back to the junction
            } else {
                self.cur = far;
            }
            if !ret {
                return;
            }
        }
        self.cur = at;
    }
}

/// Floor rings of one depth level: closed paths at `z`.
struct Level {
    paths: PathsD,
    z: f64,
}

fn region_contours(p: &PathsD) -> Vec<Contour> {
    p.iter().map(contour_of).collect()
}

pub fn generate_vcarve(op: &Op, tool: &Tool, contours: &[Contour], m: &Material, clear_tool: Option<&Tool>) -> CamResult {
    const V_STEP: f64 = 0.01; // ring spacing, inches
    let mut res = CamResult::default();
    if tool.kind != ToolType::VBit || tool.angle <= 0.0 || tool.angle >= 180.0 {
        res.warnings.push(format!("{}: needs a V-bit tool", op.name));
        return res;
    }
    let tan_half = (tool.angle / 2.0 * PI / 180.0).tan();
    let max_depth = if op.flat_depth > 0.0 { op.flat_depth.min(m.t) } else { m.t };
    if op.flat_depth > m.t {
        res.warnings.push(format!("{}: flat depth is deeper than the material", op.name));
    }
    let start = op.start_depth.max(0.0);
    if start >= max_depth {
        res.warnings.push(format!("{}: start depth must be above the flat depth", op.name));
        return res;
    }

    let mut polys = PathsD::new();
    for c in contours {
        if !c.closed {
            res.warnings.push(format!("{}: open shapes can't be v-carved, skipped", op.name));
            continue;
        }
        let p = to_path(c);
        if p.len() >= 3 {
            polys.push(p);
        }
    }
    if polys.is_empty() {
        return res;
    }
    let region = union_subjects_d(&polys, FillRule::EvenOdd, PREC);

    let mut levels: Vec<Level> = Vec::new(); // floor-clearing rings when the carve hits the flat depth
    // 0.001 in: well under the cut tolerance, far fewer sites
    let skeleton = medial_axis(&simplify_paths(&region, 0.001, true));
    if skeleton.is_empty() {
        res.warnings.push(format!("{}: nothing to carve", op.name));
        return res;
    }
    let mut capped = false;
    let mut deepest_v: f64 = 0.0;
    for sp in skeleton.iter().flatten() {
        if start + sp.r / tan_half > max_depth + 1e-9 {
            capped = true;
        }
        deepest_v = deepest_v.max(max_depth.min(start + sp.r / tan_half));
    }
    let cone_depth = tool.diameter / 2.0 / tan_half; // deepest V the bit itself can make
    if tool.diameter > 0.0 && deepest_v > cone_depth + 0.005 {
        res.warnings.push(format!(
            "{}: carves {:.3} in deep but the bit's V is only {:.3} in deep; the widest parts get stepped walls (use a wider bit or a flat depth)",
            op.name, deepest_v, cone_depth
        ));
    }
    let mut clearance = Toolpath::default(); // optional end-mill pass over the flat floor
    if capped {
        if op.flat_depth <= 0.0 {
            res.warnings.push(format!("{}: shape is too wide to carve to a point within the material; floor clamped to the thickness", op.name));
        }
        let s0 = (max_depth - start) * tan_half;
        let use_clear = clear_tool.filter(|c| c.kind == ToolType::EndMill && c.diameter > 0.0);
        if let Some(ct) = use_clear {
            // V-bit: one lap along the floor boundary so the wall meets the floor cleanly; end mill: offset pocket of the floor
            let edge = offset_region(&region, -s0);
            if !edge.is_empty() {
                levels.push(Level { paths: edge.clone(), z: -max_depth });
            }
            // ponytail: single depth pass at the flat depth; add stepdown passes if someone clears deep floors with a small end mill
            let flat = Op { kind: OpType::Pocket, depth: max_depth, stepdown: max_depth, entry: Entry::Plunge, ..op.clone() };
            let floor_region = region_contours(&edge);
            let pk = if floor_region.is_empty() { CamResult::default() } else { generate_pocket(&flat, ct, &floor_region, m) };
            match pk.paths.into_iter().next() {
                Some(p) if !p.moves.is_empty() => {
                    clearance = p;
                    clearance.name = format!("{} [clearance]", op.name);
                }
                _ => res.warnings.push(format!("{}: clearance tool is too big for the flat area, V-bit clears it", op.name)),
            }
        }
        if use_clear.is_none() || clearance.moves.is_empty() {
            // the V-bit clears the floor with its flat width at that depth
            let width = 2.0 * s0;
            let step = V_STEP.max(0.4 * width);
            // Small regions: every ring straight from the outline (C++ parity). Busy drawings: each ring from the
            // last, thinned when heavy (see generate_pocket); offsetting a big region by a growing distance every
            // time was seconds per ring.
            if region.iter().map(Vec::len).sum::<usize>() <= 1000 {
                let mut s = s0;
                loop {
                    let p = offset_region(&region, -s);
                    if p.is_empty() {
                        break;
                    }
                    levels.push(Level { paths: p, z: -max_depth });
                    s += step;
                }
            } else {
                let mut p = offset_region(&region, -s0);
                while !p.is_empty() {
                    levels.push(Level { paths: p.clone(), z: -max_depth });
                    if p.iter().map(Vec::len).sum::<usize>() > 2000 {
                        p = simplify_paths(&p, TOL / 10.0, true);
                    }
                    p = offset_region(&p, -step);
                }
            }
        }
    }

    let mut preclear = Toolpath::default(); // start depth > 0: remove the material above the carving first
    if start > 1e-9 {
        match clear_tool.filter(|c| c.kind == ToolType::EndMill) {
            Some(ct) => {
                let flat = Op { kind: OpType::Pocket, depth: start, stepdown: 0.0, entry: Entry::Plunge, allowance: 0.0, bind: BTreeMap::new(), ..op.clone() };
                let pk = generate_pocket(&flat, ct, &region_contours(&region), m);
                if let Some(p) = pk.paths.into_iter().next() {
                    preclear = p;
                    preclear.name = format!("{} [clear to start depth]", op.name);
                    preclear.tool = op.clear_tool;
                }
                res.warnings.extend(pk.warnings);
            }
            None => res.warnings.push(format!("{}: start depth needs a clearance end mill to remove the material above it", op.name)),
        }
    }

    let mut tp = Toolpath { name: op.name.clone(), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };
    let mut first = true;
    let mut cur = Pt::default();
    {
        // deep carves in layers like VCarve: at each pass depth L above the deepest point, one lap of the outline
        // inset to where the carve is L deep, so the final skeleton pass doesn't take the whole V in one go
        let stepdown = if op.stepdown > 0.0 { op.stepdown } else { tool.stepdown };
        let mut deepest: f64 = 0.0;
        for sp in skeleton.iter().flatten() {
            deepest = deepest.max(max_depth.min(start + sp.r / tan_half));
        }
        let mut l = start + stepdown;
        while stepdown > 0.0 && l < deepest - 1e-3 {
            for mut p in offset_region(&region, -(l - start) * tan_half) {
                if p.len() < 3 {
                    continue;
                }
                let hole = !is_positive(&p);
                if is_positive(&p) == hole {
                    p.reverse(); // outer CCW, holes CW (climb)
                }
                if !first {
                    rapid_to(&mut tp, cur, Some(m.retract_z));
                }
                let p0 = pt(p[0].x, p[0].y);
                rapid_to(&mut tp, p0, Some(m.retract_z));
                for q in &p {
                    cut_to(&mut tp, pt(q.x, q.y), Some(-l));
                }
                cut_to(&mut tp, p0, Some(-l));
                cur = p0;
                first = false;
            }
            l += stepdown;
        }
    }
    {
        // follow the medial axis: depth = distance to the outline / tan(half angle), capped at the flat depth
        let boxes = region_boxes(&region);
        let mut adj: HashMap<Key, Vec<usize>> = HashMap::new();
        for (i, ch) in skeleton.iter().enumerate() {
            adj.entry(key(ch[0].p)).or_default().push(i);
            adj.entry(key(ch[ch.len() - 1].p)).or_default().push(i);
        }
        let mut w = SkelWalk { skeleton: &skeleton, adj, done: vec![false; skeleton.len()], max_depth, start, tan_half, cur };
        // components: start each at a leaf (dead end) when there is one, so the walk needs the least retracing
        for _ in 0..skeleton.len() {
            if !first {
                w.cur = last_xy(&tp, w.cur);
            }
            let cur = w.cur;
            let (mut best, mut leaf, mut bd): (Option<usize>, bool, f64) = (None, false, 1e300);
            for (i, ch) in skeleton.iter().enumerate() {
                if w.done[i] {
                    continue;
                }
                for p in [ch[0].p, ch[ch.len() - 1].p] {
                    let is_leaf = w.adj_of(p).len() == 1;
                    let d = if first { 0.0 } else { dist(cur, p) };
                    if best.is_none() || (is_leaf && !leaf) || (is_leaf == leaf && d < bd) {
                        best = Some(i);
                        leaf = is_leaf;
                        bd = d;
                    }
                }
            }
            let Some(best) = best else { break };
            let ch = &skeleton[best];
            let (front, back) = (ch[0], ch[ch.len() - 1]);
            let mut st = if w.adj_of(front.p).len() == 1 || !leaf { front.p } else { back.p };
            if !leaf {
                st = if dist(cur, front.p) <= dist(cur, back.p) { front.p } else { back.p };
            }
            let r0 = if key(st) == key(front.p) { front.r } else { back.r };
            // link from where we are: stay down when a straight link at the shallower depth keeps the bit inside the shape
            let z_start = w.z_at(r0);
            let z_link = z_start.max(if tp.moves.is_empty() { -1e9 } else { last_z(&tp).unwrap_or(-1e9) });
            let need = -z_link * tan_half + 1e-4;
            let mut down = !first && dist(cur, st) < 2.0;
            let mut t = 1;
            while t < 8 && down {
                let q = lerp(cur, st, t as f64 / 8.0);
                t += 1;
                if !inside_region(&region, &boxes, q) {
                    down = false;
                    break;
                }
                let mut dmin: f64 = 1e300;
                for (pp, b) in region.iter().zip(&boxes) {
                    if dmin <= need {
                        break;
                    }
                    if !in_box(b, q, need) {
                        continue; // a far-away outline can't be closer than `need`
                    }
                    for i in 0..pp.len() {
                        if dmin <= need {
                            break;
                        }
                        let (pa, pb) = (pp[i], pp[(i + 1) % pp.len()]);
                        dmin = dmin.min(dist_to_segment(q, pt(pa.x, pa.y), pt(pb.x, pb.y)));
                    }
                }
                if dmin < need {
                    down = false;
                }
            }
            if down {
                if z_link > last_z(&tp).unwrap_or(0.0) {
                    cut_to(&mut tp, cur, Some(z_link));
                }
                cut_to(&mut tp, st, Some(z_link));
                cut_to(&mut tp, st, Some(z_start));
            } else {
                if !first {
                    rapid_to(&mut tp, cur, Some(m.retract_z));
                }
                rapid_to(&mut tp, st, Some(m.retract_z));
                cut_to(&mut tp, st, Some(z_start));
            }
            first = false;
            w.cur = st;
            w.walk(&mut tp, st, false);
        }
        cur = w.cur;
    }
    // floor rings all sit at the flat depth inside this area, so links that stay inside it can stay down
    let floor_area = if capped { offset_region(&region, -(max_depth - start) * tan_half) } else { PathsD::new() };
    let level_paths: Vec<PathsD> = levels.iter().map(|l| l.paths.clone()).collect();
    let mut prev_level: Option<usize> = None;
    for (mut p, lvi) in order_rings(&level_paths) {
        let lv = &levels[lvi];
        // the region we are leaving: the previous ring's level (its floor is at least that deep everywhere inside)
        let prev_region = match prev_level {
            Some(pl) => Some(&levels[pl].paths),
            None if !floor_area.is_empty() => Some(&floor_area),
            None => None,
        };
        prev_level = Some(lvi);
        let hole = !is_positive(&p);
        if is_positive(&p) == hole {
            p.reverse(); // outer CCW, holes CW (climb)
        }
        // start at the point nearest to where we are, to keep links short
        if !tp.moves.is_empty() {
            cur = last_xy(&tp, cur);
        }
        let mut bi = 0;
        if !first {
            let mut bd = 1e300;
            for (j, q) in p.iter().enumerate() {
                let d = dist(cur, pt(q.x, q.y));
                if d < bd {
                    bd = d;
                    bi = j;
                }
            }
        }
        p.rotate_left(bi);
        let s = pt(p[0].x, p[0].y);
        if first {
            rapid_to(&mut tp, s, Some(m.retract_z));
        } else {
            // stay down if the link stays inside the region we're leaving (its final floor is at least this deep)
            let mut safe = prev_region.is_some();
            let mut t = 1;
            while t < 8 && safe {
                let q = lerp(cur, s, t as f64 / 8.0);
                safe = prev_region.unwrap().iter().filter(|pp| inside(q, pp)).count() % 2 == 1;
                t += 1;
            }
            if safe {
                let lz = last_z(&tp).unwrap_or(0.0);
                let link_z = lz.max(lv.z); // shallower of the two depths
                if link_z > lz {
                    cut_to(&mut tp, cur, Some(link_z)); // come up first
                }
                cut_to(&mut tp, s, Some(link_z));
            } else {
                rapid_to(&mut tp, cur, Some(m.retract_z));
                rapid_to(&mut tp, s, Some(m.retract_z));
            }
        }
        cut_to(&mut tp, s, Some(lv.z));
        for q in &p[1..] {
            cut_to(&mut tp, pt(q.x, q.y), Some(lv.z));
        }
        cut_to(&mut tp, s, Some(lv.z));
        cur = s;
        first = false;
    }
    cur = last_xy(&tp, cur);
    rapid_to(&mut tp, cur, Some(m.retract_z));
    if !preclear.moves.is_empty() {
        res.paths.insert(0, preclear);
    }
    res.paths.push(tp);
    if !clearance.moves.is_empty() {
        clearance.tool = op.clear_tool;
        res.paths.push(clearance);
    }
    res
}

/// Flute: cut along each vector's centreline to `depth` in passes, ramping in/out over ramp_in/ramp_out.
pub fn generate_flute(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    let mut res = CamResult::default();
    if !depth_checks(op, m, &mut res) {
        return res;
    }
    let stepdown = if op.stepdown > 0.0 { op.stepdown } else { tool.stepdown };
    let passes = passes_for(op.depth, stepdown);
    for c in contours {
        let pts = flatten(c, TOL);
        if pts.len() < 2 {
            continue;
        }
        let pts = subdivide(&pts, RAMP_SEG);
        let s = lengths(&pts);
        let l = *s.last().unwrap();
        if l < 1e-9 {
            continue;
        }
        let (mut a, mut b) = (clampf(op.ramp_in, 0.0, l), clampf(op.ramp_out, 0.0, l));
        if a + b > l {
            let k = l / (a + b);
            a *= k;
            b *= k;
        }
        let mut tp = Toolpath { name: indexed_name(&op.name, res.paths.len()), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };
        rapid_to(&mut tp, pts[0], Some(m.retract_z));
        for p in 0..passes {
            let d = op.depth * (p as f64 + 1.0) / passes as f64;
            let back = p % 2 == 1; // alternate direction on open vectors to avoid a retract
            for k in 0..pts.len() {
                let idx = if back { pts.len() - 1 - k } else { k };
                let mut f: f64 = 1.0;
                if a > 1e-9 {
                    f = f.min(s[idx] / a);
                }
                if b > 1e-9 {
                    f = f.min((l - s[idx]) / b);
                }
                cut_to(&mut tp, pts[idx], Some(-d * clampf(f, 0.0, 1.0)));
            }
        }
        let last = *tp.moves.last().unwrap();
        rapid_to(&mut tp, pt(last.x.unwrap(), last.y.unwrap()), Some(m.retract_z));
        res.paths.push(tp);
    }
    res
}

fn vbit_ok(op: &Op, tool: &Tool, res: &mut CamResult) -> bool {
    if tool.kind == ToolType::VBit && tool.angle > 0.0 && tool.angle < 180.0 {
        return true;
    }
    res.warnings.push(format!("{}: needs a V-bit tool", op.name));
    false
}

/// Chamfer: a V-bit's tip rides each vector (sharp corners stay sharp) at the depth that gives the bevel: side Inside /
/// Outside = the bevel face of width w lies inside / outside the vector (holes flip, as in profile), V depth = w / tan(half
/// angle); On = a centred V-groove of total width w (depth = w/2 / tan). Passes by pass depth below the start depth.
pub fn generate_chamfer(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    let mut res = CamResult::default();
    if !vbit_ok(op, tool, &mut res) {
        return res;
    }
    let tan_half = (tool.angle / 2.0 * PI / 180.0).tan();
    let d = if op.chamfer_width > 0.0 { (if op.side == Side::On { op.chamfer_width / 2.0 } else { op.chamfer_width }) / tan_half } else { op.depth };
    if d <= 0.0 {
        res.warnings.push(format!("{}: chamfer size must be > 0", op.name));
        return res;
    }
    let start = op.start_depth.max(0.0);
    if start + d > m.t + 1e-9 {
        res.warnings.push(format!("{}: depth is deeper than the material", op.name));
    }
    if tool.diameter > 0.0 && d * tan_half > tool.diameter / 2.0 + 1e-9 {
        res.warnings.push(format!("{}: chamfer is wider than the bit's V (use a wider bit)", op.name));
    }
    let stepdown = if op.stepdown > 0.0 { op.stepdown } else { tool.stepdown };
    let passes = passes_for(d, stepdown);

    let polys: Vec<PathD> = contours.iter().map(to_path).collect();
    for (i, c) in contours.iter().enumerate() {
        let mut poly = polys[i].clone();
        if poly.len() < 2 {
            continue;
        }
        let closed = c.closed && poly.len() > 2;
        if closed {
            // hole = inside an odd number of the op's other closed shapes: its bevel side flips
            let p0 = pt(poly[0].x, poly[0].y);
            let depth = (0..contours.len()).filter(|&j| j != i && contours[j].closed && polys[j].len() > 2 && inside(p0, &polys[j])).count();
            let hole = depth % 2 == 1;
            // climb (CW spindle): the bevel face on the cutter's right. Face inside -> CW, face outside -> CCW.
            let mut ccw = if op.side == Side::On { hole } else { (op.side == Side::Outside) != hole };
            if !op.climb {
                ccw = !ccw;
            }
            if is_positive(&poly) != ccw {
                poly.reverse();
            }
            let mut it = 0;
            for k in 1..poly.len() {
                let (a, b) = (poly[k], poly[it]);
                if a.y < b.y - 1e-9 || ((a.y - b.y).abs() <= 1e-9 && a.x < b.x) {
                    it = k;
                }
            }
            poly.rotate_left(it);
        }
        let mut pts = pts_of(&poly);
        if closed {
            pts.push(pts[0]);
        }
        let mut tp = Toolpath { name: op.name.clone(), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };
        rapid_to(&mut tp, pts[0], Some(m.retract_z));
        for k in 0..passes {
            let z = -(start + d * (k as f64 + 1.0) / passes as f64);
            let back = !closed && k % 2 == 1; // open vectors zig-zag instead of retracting
            for j in 0..pts.len() {
                cut_to(&mut tp, if back { pts[pts.len() - 1 - j] } else { pts[j] }, Some(z));
            }
        }
        let end = last_xy(&tp, pts[0]);
        rapid_to(&mut tp, end, Some(m.retract_z));
        res.paths.push(tp);
    }
    for (i, p) in res.paths.iter_mut().enumerate().skip(1) {
        p.name = indexed_name(&op.name, i);
    }
    res
}

/// Prism (VCarve "Prism carving", raised): the V-bit's tip rides each outline at depth R/tan(half angle), R = radius of
/// the largest disc that fits inside the shape touching that outline point (the medial-axis radius), also limited by the
/// room outside (neighbouring shapes) and capped at `depth` (max depth). The bit's flank cuts a facet from the outline up
/// to the shape's centre line, which stays at the surface: a bevelled, raised-looking shape with an equal V groove
/// outside; where the cap limits the depth the top stays flat. V-carve is the inverse (a recess, deepest on the centre
/// line); no V-carve setting gives a prism. Pass depth layers.
pub fn generate_prism(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    const SEG: f64 = 0.02; // outline sample spacing: depth follows the local width smoothly
    let mut res = CamResult::default();
    if !vbit_ok(op, tool, &mut res) {
        return res;
    }
    let tan_half = (tool.angle / 2.0 * PI / 180.0).tan();
    if op.depth <= 0.0 {
        res.warnings.push(format!("{}: max depth must be > 0", op.name));
        return res;
    }
    if op.depth > m.t + 1e-9 {
        res.warnings.push(format!("{}: depth is deeper than the material", op.name));
    }
    let reach_max = op.depth.min(m.t) * tan_half;

    let mut polys = PathsD::new();
    for c in contours {
        if !c.closed {
            res.warnings.push(format!("{}: open shapes can't be prism carved, skipped", op.name));
            continue;
        }
        let p = to_path(c);
        if p.len() >= 3 {
            polys.push(p);
        }
    }
    if polys.is_empty() {
        return res;
    }
    let region = union_subjects_d(&polys, FillRule::EvenOdd, PREC); // outers CCW, holes CW: region on the left

    // (a, b, box x0, y0, x1, y1)
    let mut segs: Vec<(Pt, Pt, [f64; 4])> = Vec::new();
    for p in &region {
        for i in 0..p.len() {
            let (a, b) = (pt(p[i].x, p[i].y), pt(p[(i + 1) % p.len()].x, p[(i + 1) % p.len()].y));
            segs.push((a, b, [a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)]));
        }
    }
    // Largest t such that a disc of radius t touching q, centred along dir, fits between the outlines (bisection: the
    // distance to the outline minus t never grows with t). The small relative slack keeps flattened curves smooth.
    // ponytail: the disc is checked along the normal only; a cone this deep could still nick an odd neighbour sideways.
    let reach = |near: &[usize], q: Pt, dir: Pt| -> f64 {
        let fits = |t: f64| {
            let c = q + dir * t;
            let mut dm = 1e300f64;
            for &s in near {
                dm = dm.min(dist_to_segment(c, segs[s].0, segs[s].1));
            }
            dm >= t * (1.0 - 1e-3) - 1e-9
        };
        if fits(reach_max) {
            return reach_max;
        }
        let (mut lo, mut hi) = (0.0, reach_max);
        for _ in 0..30 {
            let mid = (lo + hi) / 2.0;
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    let left_n = |a: Pt, b: Pt| {
        let l = dist(a, b);
        if l > 0.0 {
            pt(-(b.y - a.y) / l, (b.x - a.x) / l)
        } else {
            Pt::default()
        }
    };
    let stepdown = if op.stepdown > 0.0 { op.stepdown } else { tool.stepdown };
    let mut deepest: f64 = 0.0;
    let mut near: Vec<usize> = Vec::new();
    for ring in &region {
        let mut lp = pts_of(ring);
        lp.push(lp[0]);
        let mut lp = subdivide(&lp, SEG);
        lp.pop(); // closed: last == first
        let n = lp.len();
        if n < 3 {
            continue;
        }
        let mut zs = vec![0.0; n];
        let mut ring_deep: f64 = 0.0;
        for i in 0..n {
            let q = lp[i];
            let mut nn = left_n(lp[(i + n - 1) % n], q) + left_n(q, lp[(i + 1) % n]); // inward: bisector at corners
            let l = nn.x.hypot(nn.y);
            nn = if l > 1e-9 { nn * (1.0 / l) } else { left_n(q, lp[(i + 1) % n]) };
            near.clear();
            let pad = 2.0 * reach_max;
            for (k, s) in segs.iter().enumerate() {
                let b = s.2;
                if q.x >= b[0] - pad && q.x <= b[2] + pad && q.y >= b[1] - pad && q.y <= b[3] + pad {
                    near.push(k);
                }
            }
            let r = reach(&near, q, nn).min(reach(&near, q, nn * -1.0));
            zs[i] = if r < 1e-6 { 0.0 } else { -r / tan_half }; // convex corners: exactly at the surface
            ring_deep = ring_deep.max(-zs[i]);
        }
        deepest = deepest.max(ring_deep);
        if ring_deep < 1e-6 {
            continue;
        }
        // start at the deepest point (a convex corner sits at Z0)
        let mut s0 = 0;
        for i in 1..n {
            if zs[i] < zs[s0] {
                s0 = i;
            }
        }
        lp.rotate_left(s0);
        zs.rotate_left(s0);
        let passes = passes_for(ring_deep, stepdown);
        let mut tp = Toolpath { name: op.name.clone(), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };
        rapid_to(&mut tp, lp[0], Some(m.retract_z));
        for k in 0..passes {
            let floor_z = if k + 1 == passes { -1e300 } else { -stepdown * (k as f64 + 1.0) };
            for i in 0..=n {
                cut_to(&mut tp, lp[i % n], Some(zs[i % n].max(floor_z)));
            }
        }
        rapid_to(&mut tp, lp[0], Some(m.retract_z));
        res.paths.push(tp);
    }
    if res.paths.is_empty() {
        res.warnings.push(format!("{}: nothing to carve", op.name));
        return res;
    }
    let cone_depth = tool.diameter / 2.0 / tan_half;
    if tool.diameter > 0.0 && deepest > cone_depth + 0.005 {
        res.warnings.push(format!(
            "{}: bevel is {:.3} in deep but the bit's V is only {:.3} in deep; the facets stop short of the centre (use a wider bit or a smaller max depth)",
            op.name, deepest, cone_depth
        ));
    }
    for (i, p) in res.paths.iter_mut().enumerate().skip(1) {
        p.name = indexed_name(&op.name, i);
    }
    res
}

/// Inlay: female = v-carve of the design (flat depth), male = v-carve of the inverted design (a margin rectangle minus
/// the shapes) starting at the glue gap depth, mirrored so it fits when flipped. Paths named "... [female]" / "... [male]".
pub fn generate_inlay(op: &Op, tool: &Tool, contours: &[Contour], m: &Material, clear_tool: Option<&Tool>) -> CamResult {
    let mut res = CamResult::default();
    if op.flat_depth <= 0.0 {
        res.warnings.push(format!("{}: inlay needs a flat depth (pocket depth of the female)", op.name));
        return res;
    }
    if op.glue_gap <= 0.0 || op.glue_gap >= op.flat_depth {
        res.warnings.push(format!("{}: glue gap must be between 0 and the flat depth", op.name));
        return res;
    }
    // female: the design itself
    let f = Op { kind: OpType::VCarve, start_depth: 0.0, name: format!("{} [female]", op.name), ..op.clone() };
    let female = generate_vcarve(&f, tool, contours, m, clear_tool);
    res.warnings.extend(female.warnings);
    res.paths.extend(female.paths);
    // male: margin rectangle around the design minus the design, mirrored left-right (it is flipped over to fit)
    let mut b = Rect::default();
    for c in contours {
        b.add_rect(&bounds(c));
    }
    if b.is_empty() {
        return res;
    }
    let mg = op.inlay_margin.max(0.0);
    let mut inv = vec![make_rect(b.x0 - mg, b.y0 - mg, b.x1 + mg, b.y1 + mg)];
    inv.extend(contours.iter().filter(|c| c.closed).cloned());
    let cx = (b.x0 + b.x1) / 2.0;
    let mirror = Affine::translate(cx, 0.0) * Affine::scale(-1.0, 1.0) * Affine::translate(-cx, 0.0);
    let inv: Vec<Contour> = inv.iter().map(|c| transformed(c, &mirror)).collect();
    // the male's floor sits at the same depth: plug height = flat_depth - glue_gap
    let mo = Op { kind: OpType::VCarve, name: format!("{} [male]", op.name), start_depth: op.glue_gap, flat_depth: op.flat_depth, ..op.clone() };
    let male = generate_vcarve(&mo, tool, &inv, m, clear_tool);
    res.warnings.extend(male.warnings);
    res.paths.extend(male.paths);
    if clear_tool.is_none() {
        res.warnings.push(format!("{}: pick a clearance end mill so the male blank is cleared to the glue gap", op.name));
    }
    res
}

/// Thread mill: the bit circles the hole once per `pitch`, rising from the bottom to the top (climb, CCW), entering
/// and leaving on a half-circle so the thread flank has no dwell mark. Hole size = the thread's major diameter
/// (from the shape's bounds); `op.allowance` leaves material for a second, finishing pass.
fn thread_mill(tp: &mut Toolpath, ctr: Pt, r: f64, op: &Op, m: &Material) {
    const SEGS: i32 = 36;
    let turns = (op.depth / op.thread_pitch).ceil().max(1.0) as i32;
    let z_bot = -op.depth;
    let z_top = z_bot + turns as f64 * op.thread_pitch;
    let on = |a: f64| pt(ctr.x + r * a.cos(), ctr.y + r * a.sin());
    // half circle of diameter r from the centre to the helix start (angle 0), arriving along +y like the helix
    let half = |tp: &mut Toolpath, a0: f64, z: f64| {
        let c = pt(ctr.x + r / 2.0, ctr.y);
        for k in 1..=18 {
            let a = a0 + PI * k as f64 / 18.0;
            cut_to(tp, pt(c.x + r / 2.0 * a.cos(), c.y + r / 2.0 * a.sin()), Some(z));
        }
    };
    rapid_to(tp, ctr, Some(m.retract_z));
    cut_to(tp, ctr, Some(z_bot));
    half(tp, PI, z_bot); // centre -> (r, 0) via below
    for k in 1..=turns * SEGS {
        let a = 2.0 * PI * k as f64 / SEGS as f64;
        cut_to(tp, on(a), Some(z_bot + op.thread_pitch * k as f64 / SEGS as f64));
    }
    half(tp, 0.0, z_top); // (r, 0) -> centre via above
    rapid_to(tp, ctr, Some(m.retract_z));
}

/// Bore: helical descent around the hole (diameter from the shape's bounds), then a flat finishing circle.
/// With `op.thread_pitch` > 0 it is a thread mill instead (see `thread_mill`).
pub fn generate_bore(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    let mut res = CamResult::default();
    if !depth_checks(op, m, &mut res) {
        return res;
    }
    let pitch = if op.stepdown > 0.0 { op.stepdown } else { tool.stepdown }; // depth per revolution
    for c in contours {
        let b = bounds(c);
        if b.is_empty() {
            continue;
        }
        let ctr = pt((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
        let hole = b.w().min(b.h());
        let r = hole / 2.0 - tool.diameter / 2.0 - op.allowance;
        let mut tp = Toolpath { name: indexed_name(&op.name, res.paths.len()), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };
        if r <= 1e-6 {
            // hole no bigger than the bit: straight (peck) drill instead
            res.warnings.push(format!("{}: hole is not larger than the bit, drilled straight", op.name));
            peck_drill(&mut tp, ctr, op, m);
            res.paths.push(tp);
            continue;
        }
        if op.thread_pitch > 0.0 {
            thread_mill(&mut tp, ctr, r, op, m);
            res.paths.push(tp);
            continue;
        }
        // helix: segments every 10 degrees, climb (CCW) for an inside cut with a CW spindle
        const SEGS: i32 = 36;
        let turns = op.depth / pitch;
        let total = ((turns * SEGS as f64).ceil() as i32).max(1);
        let at = |a: f64| pt(ctr.x + r * a.cos(), ctr.y + r * a.sin());
        let st = at(-PI / 2.0); // start at the bottom of the circle like the profile cut
        rapid_to(&mut tp, st, Some(m.retract_z));
        cut_to(&mut tp, st, Some(0.0));
        for k in 1..=total {
            let a = -PI / 2.0 + 2.0 * PI * k as f64 / SEGS as f64;
            let z = -op.depth.min(op.depth * k as f64 / total as f64);
            cut_to(&mut tp, at(a), Some(z));
        }
        // flat finishing circle at full depth
        let a0 = -PI / 2.0 + 2.0 * PI * total as f64 / SEGS as f64;
        for k in 1..=SEGS {
            cut_to(&mut tp, at(a0 + 2.0 * PI * k as f64 / SEGS as f64), Some(-op.depth));
        }
        rapid_to(&mut tp, at(a0), Some(m.retract_z));
        res.paths.push(tp);
    }
    res
}

/// VCarve "Texture": rows of short strokes that scoop down to the depth and back up, staggered and randomised
/// (length and depth by `vary`, fixed seed so the G-code is repeatable), clipped to the closed shapes.
pub fn generate_texture(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    const HOP: f64 = 0.04; // above the surface between strokes of a row
    const STEP: f64 = 0.02; // stroke sample spacing
    let mut res = CamResult::default();
    if op.depth <= 0.0 {
        res.warnings.push(format!("{}: depth must be > 0", op.name));
        return res;
    }
    let polys: PathsD = contours.iter().filter(|c| c.closed).map(to_path).filter(|p| p.len() >= 3).collect();
    if polys.len() < contours.len() {
        res.warnings.push(format!("{}: open shapes can't hold a texture, skipped", op.name));
    }
    if polys.is_empty() {
        return res;
    }
    let region = union_subjects_d(&polys, FillRule::EvenOdd, PREC);
    let boxes = region_boxes(&region);
    let spacing = (tool.diameter * op.stepover).max(0.01);
    let len = op.ramp_length.max(0.05);
    let vary = op.vary.clamp(0.0, 1.0);
    let (sa, ca) = (op.raster_angle as f64).to_radians().sin_cos();
    let to_xy = |u: f64, v: f64| pt(u * ca - v * sa, u * sa + v * ca);
    let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in region.iter().flatten() {
        let (u, v) = (p.x * ca + p.y * sa, -p.x * sa + p.y * ca);
        (u0, u1, v0, v1) = (u0.min(u), u1.max(u), v0.min(v), v1.max(v));
    }
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut tp = Toolpath { name: op.name.clone(), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };
    let mut row = 0;
    let mut v = v0 + spacing / 2.0;
    while v < v1 {
        // the strokes of this row as point runs (inside the shapes only), then cut, every other row backwards
        let mut strokes: Vec<Vec<(Pt, f64)>> = Vec::new();
        let mut u = u0 - rnd() * len;
        while u < u1 {
            let l = len * (1.0 - vary * 0.5 * rnd());
            let d = (op.depth * (1.0 - vary * 0.6 * rnd())).min(m.t);
            let n = ((l / STEP).ceil() as usize).max(4);
            let mut run: Vec<(Pt, f64)> = Vec::new();
            for k in 0..=n {
                let t = k as f64 / n as f64;
                let q = to_xy(u + t * l, v);
                if inside_region(&region, &boxes, q) {
                    run.push((q, -d * (t * PI).sin()));
                } else if !run.is_empty() {
                    strokes.push(std::mem::take(&mut run));
                }
            }
            if run.len() > 1 {
                strokes.push(run);
            }
            u += l + len * (0.1 + 0.2 * rnd()) * (1.0 + vary);
        }
        if row % 2 == 1 {
            strokes.reverse();
            strokes.iter_mut().for_each(|s| s.reverse());
        }
        for (i, s) in strokes.iter().enumerate() {
            rapid_to(&mut tp, s[0].0, Some(if i == 0 { m.retract_z } else { HOP }));
            for &(q, z) in s {
                cut_to(&mut tp, q, Some(z));
            }
            rapid_to(&mut tp, s[s.len() - 1].0, Some(HOP));
        }
        if let Some(s) = strokes.last() {
            rapid_to(&mut tp, s[s.len() - 1].0, Some(m.retract_z));
        }
        v += spacing;
        row += 1;
    }
    if tp.moves.is_empty() {
        res.warnings.push(format!("{}: the shapes are too small for this texture", op.name));
    } else {
        res.paths.push(tp);
    }
    res
}

/// VCarve "Photo V-Carve": rows of V-grooves across the closed shapes, each point as deep as the picture is dark
/// there (light parts stay uncut, the bit lifts over them). Rows sit 2·depth·tan(half angle) apart, so full black
/// grooves just meet. The picture is fitted, centred, into the shapes' bounds.
pub fn generate_photo(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    const HOP: f64 = 0.04;
    const MIN_DEPTH: f64 = 0.004; // lighter than this: no groove
    let mut res = CamResult::default();
    if !vbit_ok(op, tool, &mut res) {
        return res;
    }
    if op.depth <= 0.0 {
        res.warnings.push(format!("{}: depth must be > 0", op.name));
        return res;
    }
    let Some(img) = crate::photo::Gray::from_op(op.photo_w, op.photo_h, &op.photo) else {
        res.warnings.push(format!("{}: pick a picture for the photo carving", op.name));
        return res;
    };
    let polys: PathsD = contours.iter().filter(|c| c.closed).map(to_path).filter(|p| p.len() >= 3).collect();
    if polys.is_empty() {
        res.warnings.push(format!("{}: select a closed shape (a rectangle) to hold the picture", op.name));
        return res;
    }
    let region = union_subjects_d(&polys, FillRule::EvenOdd, PREC);
    let boxes = region_boxes(&region);
    let mut b = Rect::default();
    boxes.iter().for_each(|r| {
        b.add(pt(r.x0, r.y0));
        b.add(pt(r.x1, r.y1));
    });
    // picture fitted into the bounds, centred
    let aspect = img.w as f64 / img.h as f64;
    let (iw, ih) = if b.w() / b.h() > aspect { (b.h() * aspect, b.h()) } else { (b.w(), b.w() / aspect) };
    let (ix, iy) = (b.x0 + (b.w() - iw) / 2.0, b.y0 + (b.h() - ih) / 2.0);
    let depth = op.depth.min(m.t);
    let (lo, hi) = img.levels();
    let tan_half = (tool.angle / 2.0).to_radians().tan();
    let spacing = (2.0 * depth * tan_half).max(0.01);
    let step = (spacing / 2.0).min(0.02);
    let (sa, ca) = (op.raster_angle as f64).to_radians().sin_cos();
    let to_xy = |u: f64, v: f64| pt(u * ca - v * sa, u * sa + v * ca);
    let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in region.iter().flatten() {
        let (u, v) = (p.x * ca + p.y * sa, -p.x * sa + p.y * ca);
        (u0, u1, v0, v1) = (u0.min(u), u1.max(u), v0.min(v), v1.max(v));
    }
    let mut tp = Toolpath { name: op.name.clone(), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new() };
    let mut row = 0;
    let mut v = v0 + spacing / 2.0;
    while v < v1 {
        let mut runs: Vec<Vec<(Pt, f64)>> = Vec::new();
        let mut run: Vec<(Pt, f64)> = Vec::new();
        let n = ((u1 - u0) / step).ceil() as usize;
        for k in 0..=n {
            let q = to_xy(u0 + k as f64 * step, v);
            let (fu, fv) = ((q.x - ix) / iw, (q.y - iy) / ih);
            let mut d = 0.0;
            if (0.0..=1.0).contains(&fu) && (0.0..=1.0).contains(&fv) && inside_region(&region, &boxes, q) {
                let dark = ((img.dark(fu, fv) - lo) / (hi - lo)).clamp(0.0, 1.0);
                d = depth * if op.photo_invert { 1.0 - dark } else { dark };
            }
            if d >= MIN_DEPTH {
                run.push((q, -d));
            } else if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
        }
        if !run.is_empty() {
            runs.push(run);
        }
        if row % 2 == 1 {
            runs.reverse();
            runs.iter_mut().for_each(|r| r.reverse());
        }
        for (i, r) in runs.iter().enumerate() {
            rapid_to(&mut tp, r[0].0, Some(if i == 0 { m.retract_z } else { HOP }));
            for &(q, z) in r {
                cut_to(&mut tp, q, Some(z));
            }
            rapid_to(&mut tp, r[r.len() - 1].0, Some(HOP));
        }
        if let Some(r) = runs.last() {
            rapid_to(&mut tp, r[r.len() - 1].0, Some(m.retract_z));
        }
        v += spacing;
        row += 1;
    }
    if tp.moves.is_empty() {
        res.warnings.push(format!("{}: the picture is too light to carve at this depth", op.name));
    } else {
        res.paths.push(tp);
    }
    res
}

/// Dispatch on op.kind (not Relief: see relief.rs), then shortest travel order, the safe-rapid net and naming.
pub fn generate(op: &Op, tool: &Tool, contours: &[Contour], m: &Material, clear_tool: Option<&Tool>) -> CamResult {
    let mut r = match op.kind {
        OpType::Pocket => generate_pocket_two(op, tool, contours, m, clear_tool),
        OpType::Drill => generate_drill(op, tool, contours, m),
        OpType::VCarve => generate_vcarve(op, tool, contours, m, clear_tool),
        OpType::Bore => generate_bore(op, tool, contours, m),
        OpType::Inlay => generate_inlay(op, tool, contours, m, clear_tool),
        OpType::Flute => generate_flute(op, tool, contours, m),
        OpType::Chamfer => generate_chamfer(op, tool, contours, m),
        OpType::Prism => generate_prism(op, tool, contours, m),
        OpType::Texture => generate_texture(op, tool, contours, m),
        OpType::Photo => generate_photo(op, tool, contours, m),
        _ => generate_profile(op, tool, contours, m),
    };
    for tp in &mut r.paths {
        optimize_order(tp, Pt::default()); // shortest rapid travel between shapes; depth passes keep their order
        enforce_safe_rapids(tp, m.retract_z);
    }
    order_toolpaths(&mut r.paths, Pt::default()); // one path per shape (profiles): visit them in a short tour too
    // ponytail: like the C++ engine, "[clearance]" / "[female]" suffixes count as indexed names and get renumbered
    let prefix = format!("{} [", op.name);
    let mut n = 0;
    for tp in &mut r.paths {
        if tp.name == op.name || tp.name.starts_with(&prefix) {
            tp.name = indexed_name(&op.name, n);
            n += 1;
        }
    }
    r
}

/// Document → file coordinates: add to every move so the file is relative to the board's origin corner and Z zero.
pub fn origin_shift(m: &Material) -> (f64, f64, f64) {
    const OX: [f64; 5] = [0.0, 1.0, 0.0, 1.0, 0.5];
    const OY: [f64; 5] = [0.0, 0.0, 1.0, 1.0, 0.5];
    let org = m.origin.clamp(0, 4) as usize;
    (-m.w * OX[org], -m.h * OY[org], if m.z_bottom { m.t } else { 0.0 })
}

/// One Job per tool (single-tool post: one file per tool), in order of first use. Names: "01_<tool>".
pub fn build_jobs(ops: &[Op], tools: &[Tool], generated: &[Vec<Toolpath>], m: &Material, timestamp: &str) -> Vec<Job> {
    let mut jobs: Vec<Job> = Vec::new();
    let mut job_tool: Vec<i32> = Vec::new();
    let mut job_last_op: Vec<Option<usize>> = Vec::new();
    let mat = format!("{:.3}\" x {:.3}\" x {:.3}\"", m.w, m.h, m.t);
    let (dx, dy, dz) = origin_shift(m);
    for (i, op) in ops.iter().enumerate() {
        let Some(gen) = generated.get(i) else { continue };
        for tp in gen {
            let tool_idx = if tp.tool >= 0 { tp.tool } else { op.tool };
            if tool_idx < 0 || tool_idx as usize >= tools.len() {
                continue;
            }
            let ji = match job_tool.iter().position(|&t| t == tool_idx) {
                Some(ji) => ji,
                None => {
                    let t = &tools[tool_idx as usize];
                    let mut safe = String::new();
                    for ch in t.name.chars() {
                        let c = if ch.is_ascii_alphanumeric() { ch } else { '_' };
                        if c != '_' || (!safe.is_empty() && !safe.ends_with('_')) {
                            safe.push(c);
                        }
                    }
                    while safe.ends_with('_') {
                        safe.pop();
                    }
                    jobs.push(Job {
                        filename: format!("{:02}_{}", jobs.len() + 1, safe),
                        timestamp: timestamp.into(),
                        material: mat.clone(),
                        tool: t.name.clone(),
                        rpm: t.rpm,
                        slot: t.slot,
                        ..Default::default()
                    });
                    job_tool.push(tool_idx);
                    job_last_op.push(None);
                    jobs.len() - 1
                }
            };
            // XY origin and Z zero: shift every move so the file is relative to the chosen corner and touch-off
            let mut shifted = tp.clone();
            for mv in &mut shifted.moves {
                mv.x = mv.x.map(|v| v + dx);
                mv.y = mv.y.map(|v| v + dy);
                mv.z = mv.z.map(|v| v + dz);
            }
            // one toolpath block per op and bit, like VCarve: the per-shape paths of an op ("Drill [1]"...) join the block before
            let job = &mut jobs[ji];
            let fit = fit_arcs(&shifted, DEFAULT_TOL, DEFAULT_MAX_SWEEP);
            let joins = job_last_op[ji] == Some(i) && job.toolpaths.last().is_some_and(|b| b.feed_xy == tp.feed_xy && b.feed_z == tp.feed_z);
            if joins {
                job.toolpaths.last_mut().unwrap().moves.extend(fit.moves);
            } else {
                job.toolpaths.push(fit);
                let nm = &mut job.toolpaths.last_mut().unwrap().name; // "Drill [2]" -> "Drill" (keeps "[clearance]" and the like)
                let pre = format!("{} [", op.name);
                if nm.starts_with(&pre) && nm.len() > pre.len() + 1 && nm.as_bytes()[pre.len()..nm.len() - 1].iter().all(u8::is_ascii_digit) {
                    *nm = op.name.clone();
                }
            }
            job_last_op[ji] = Some(i);
        }
    }
    for j in &mut jobs {
        j.z_zero = if m.z_bottom { "Table" } else { "Material Surface" }.into();
        if m.z_bottom {
            // heights count up from the spoilboard: a thicker board than entered puts rapids into it
            j.notes.push(format!("Z zero = TABLE (spoilboard). This file assumes the board is {:.3} in thick: measure it.", m.t));
        }
        // first approach of each file comes down from safe Z (reference: "G0 X Y" then "G0 Z0.800")
        if let Some(mv) = j.toolpaths.first_mut().map(|t| &mut t.moves).filter(|mv| !mv.is_empty()) {
            let mut xy = mv[0];
            xy.z = None;
            mv[0] = Move::rapid(None, None, Some(m.safe_z + if m.z_bottom { m.t } else { 0.0 }));
            mv.insert(0, xy);
        }
    }
    jobs
}

/// Safety check before export, in board coordinates (Z0 = top, XY0 = bottom-left, before the origin shift): sideways
/// rapids below the surface, cuts into the spoilboard, cutting off the board, zero feeds/RPM, safe Z not above the board.
/// One line per problem; empty = fine. Shown to the user before writing files (export can still go ahead).
pub fn safety_issues(ops: &[Op], tools: &[Tool], generated: &[Vec<Toolpath>], m: &Material) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: String| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    if m.safe_z <= 0.0 {
        add("Safe Z is not above the material: rapids would drag the bit across the board.".into());
    }
    if m.retract_z <= 0.0 {
        add("Retract Z is not above the material.".into());
    }
    if m.retract_z > m.safe_z {
        add("Retract Z is above safe Z.".into());
    }
    const SPOIL: f64 = 0.0625; // cutting a little into the spoilboard is normal for through cuts
    for (op, gen) in ops.iter().zip(generated) {
        for tp in gen {
            let ti = if tp.tool >= 0 { tp.tool } else { op.tool };
            if ti < 0 || ti as usize >= tools.len() || tp.moves.is_empty() {
                continue;
            }
            let t = &tools[ti as usize];
            let who = format!("{} ({})", tp.name, t.name);
            if tp.feed_xy <= 0.0 || tp.feed_z <= 0.0 {
                add(format!("{who}: feed rate is zero."));
            }
            if t.rpm <= 0 {
                add(format!("{who}: spindle speed is zero."));
            }
            let margin = t.diameter / 2.0 + 0.01;
            let (mut x, mut y, mut z, mut zmin) = (0.0, 0.0, m.safe_z, 0.0_f64);
            let (mut rapid_in, mut off) = (false, false);
            for mv in &tp.moves {
                let (nx, ny, nz) = (mv.x.unwrap_or(x), mv.y.unwrap_or(y), mv.z.unwrap_or(z));
                let sideways = (nx - x).abs() > 1e-9 || (ny - y).abs() > 1e-9;
                if mv.kind == MoveType::Rapid && sideways && z.min(nz) < -1e-6 {
                    rapid_in = true;
                }
                if nz < -1e-6 && (nx < -margin || ny < -margin || nx > m.w + margin || ny > m.h + margin) {
                    off = true;
                }
                zmin = zmin.min(nz);
                (x, y, z) = (nx, ny, nz);
            }
            if rapid_in {
                add(format!("{who}: rapid sideways move inside the material."));
            }
            if off {
                add(format!("{who}: cuts outside the board."));
            }
            if zmin < -(m.t + SPOIL) {
                add(format!("{who}: cuts {:.3} in into the spoilboard (deepest Z {:.3}, board {:.3} thick).", -zmin - m.t, zmin, m.t));
            }
            // a V-bit's depth is the cone's business; every other bit cuts only as deep as its flutes reach
            if t.kind != ToolType::VBit && t.cut_length > 0.0 && -zmin > t.cut_length + 1e-3 {
                add(format!("{who}: cuts {:.3} in deep but the bit's flutes are only {:.3} in long: the shank would rub (burning, a broken bit). Use a longer bit or a shallower cut.", -zmin, t.cut_length));
            }
        }
    }
    out
}

/// Default air-cut raise: deepest possible cut + 1/4 in clearance, at least 1 in.
pub fn air_cut_raise(m: &Material) -> f64 {
    1.0_f64.max(m.t + 0.0625 + 0.25)
}

/// Air-cut test: every Z raised by dz so the whole program runs above the board with the normal Z zero.
/// Files get an _AIRCUT suffix and a header warning.
pub fn make_air_cut(jobs: &mut [Job], dz: f64) {
    for j in jobs {
        for tp in &mut j.toolpaths {
            for mv in &mut tp.moves {
                mv.z = mv.z.map(|z| z + dz);
            }
        }
        j.filename += "_AIRCUT";
        j.notes.push("AIR CUT TEST - NOT FOR CUTTING".into());
        let zero = if j.z_zero == "Table" { "table (spoilboard)" } else { "material" };
        j.notes.push(format!("Every Z is raised {dz:.3} in. Zero Z on the {zero} as usual: the bit stays above the board."));
    }
}

/// Toolpath tiling (VCarve "Toolpath Tiling"): a board longer than the machine is cut in sections. Tile k covers
/// `tile_len` along X (or Y) starting at k·(tile_len − overlap); each tile becomes its own set of jobs with only the
/// cutting inside it, shifted so the tile starts at zero: slide the board by (tile_len − overlap) between tiles.
/// Where a cut crosses a tile edge the bit lifts there and comes back down at the same spot in the next tile (the
/// overlap makes the two cuts meet). XY zero must be the bottom-left corner. Tiles with no cutting are left out.
pub fn tile_jobs(jobs: &[Job], along_x: bool, tile_len: f64, overlap: f64, board_len: f64) -> Vec<Job> {
    let stride = tile_len - overlap;
    if stride <= 0.0 || tile_len <= 0.0 || board_len <= tile_len {
        return jobs.to_vec();
    }
    let n = ((board_len - overlap) / stride).ceil().max(1.0) as usize;
    let mut out = Vec::new();
    for k in 0..n {
        let (a, b) = (k as f64 * stride, k as f64 * stride + tile_len);
        for j in jobs {
            // the height this job travels at: its highest Z (works for Z zero on the surface or on the table)
            let lift = j.toolpaths.iter().flat_map(|t| &t.moves).filter_map(|m| m.z).fold(f64::MIN, f64::max);
            let mut tj = Job { toolpaths: Vec::new(), ..j.clone() };
            for tp in &j.toolpaths {
                let mut t = Toolpath { moves: Vec::new(), ..tp.clone() };
                let (mut cur, mut have) = ((0.0, 0.0, lift), false); // where the bit is in the full program
                let mut at: Option<(f64, f64, f64)> = None; // where it is in this tile's program (None: lifted outside)
                for m in &tp.moves {
                    let to = (m.x.unwrap_or(cur.0), m.y.unwrap_or(cur.1), m.z.unwrap_or(cur.2));
                    let from = if have { cur } else { (to.0, to.1, lift) };
                    (cur, have) = (to, true);
                    let (u0, u1) = if along_x { (from.0, to.0) } else { (from.1, to.1) };
                    // the part of this move inside the tile, as t in 0..1
                    let (t0, t1) = if (u1 - u0).abs() < 1e-12 {
                        if u0 < a - 1e-9 || u0 > b + 1e-9 { continue } else { (0.0, 1.0) }
                    } else {
                        let (ta, tb) = ((a - u0) / (u1 - u0), (b - u0) / (u1 - u0));
                        (ta.min(tb).max(0.0), ta.max(tb).min(1.0))
                    };
                    if t1 - t0 < 1e-12 && !(t0 == 0.0 && t1 == 1.0) {
                        continue;
                    }
                    let lerp3 = |t: f64| (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t, from.2 + (to.2 - from.2) * t);
                    let (e, x) = (lerp3(t0), lerp3(t1));
                    let near = |p: (f64, f64, f64), q: (f64, f64, f64)| (p.0 - q.0).abs() < 1e-9 && (p.1 - q.1).abs() < 1e-9 && (p.2 - q.2).abs() < 1e-9;
                    if !at.is_some_and(|p| near(p, e)) {
                        // come in from above: over the entry point at travel height, then straight down to it
                        if let Some(p) = at {
                            t.moves.push(Move::rapid(Some(p.0), Some(p.1), Some(lift)));
                        }
                        t.moves.push(Move::rapid(Some(e.0), Some(e.1), Some(lift)));
                        if e.2 < lift - 1e-9 {
                            t.moves.push(if m.kind == MoveType::Rapid { Move::rapid(Some(e.0), Some(e.1), Some(e.2)) } else { Move::linear(Some(e.0), Some(e.1), Some(e.2)) });
                        }
                    }
                    if !near(e, x) {
                        t.moves.push(Move { kind: m.kind, x: Some(x.0), y: Some(x.1), z: Some(x.2), ..Default::default() });
                    }
                    at = Some(x);
                    if t1 < 1.0 - 1e-12 {
                        t.moves.push(Move::rapid(Some(x.0), Some(x.1), Some(lift))); // leaving the tile: lift here
                        at = None;
                    }
                }
                if t.moves.iter().any(|m| m.kind != MoveType::Rapid) {
                    if let Some(p) = at.filter(|p| p.2 < lift - 1e-9) {
                        t.moves.push(Move::rapid(Some(p.0), Some(p.1), Some(lift)));
                    }
                    for m in &mut t.moves {
                        if along_x { m.x = m.x.map(|v| v - a) } else { m.y = m.y.map(|v| v - a) }
                    }
                    tj.toolpaths.push(t);
                }
            }
            if !tj.toolpaths.is_empty() {
                tj.filename = format!("{}_Tile{}of{n}", j.filename, k + 1);
                tj.notes.push(format!("TILE {} of {n}: {} {:.3} to {:.3} in of the board, shifted to start at zero", k + 1, if along_x { "X" } else { "Y" }, a, b.min(board_len)));
                out.push(tj);
            }
        }
    }
    out
}

#[cfg(test)]
mod last_pass_tests {
    use super::*;
    use crate::geom::make_polyline;

    #[test]
    fn a_v_bit_pocket_is_an_engraving_fill_sized_by_the_cut_width() {
        // 90 degree V-bit 0.02 in deep cuts 0.04 in wide: walls 0.02 in, lines 40 % of 0.04 apart
        let sq = [make_polyline(&[pt(0.0, 0.0), pt(1.0, 0.0), pt(1.0, 1.0), pt(0.0, 1.0)], true)];
        let vbit = Tool { kind: ToolType::VBit, diameter: 0.5, angle: 90.0, stepdown: 0.25, feed_xy: 60.0, feed_z: 20.0, ..Default::default() };
        let m = Material::default();
        for raster in [false, true] {
            let op = Op { kind: OpType::Pocket, name: "E".into(), depth: 0.02, stepover: 0.4, climb: true, raster, ..Default::default() };
            let res = generate_pocket(&op, &vbit, &sq, &m);
            assert!(res.warnings.is_empty() && res.paths.len() == 1, "{:?}", res.warnings);
            let cuts: Vec<_> = res.paths[0].moves.iter().filter(|q| q.kind == MoveType::Linear && q.z.unwrap() < 0.0).collect();
            let (lo, hi) = cuts.iter().fold((1e9_f64, -1e9_f64), |(a, b), q| (a.min(q.x.unwrap()), b.max(q.x.unwrap())));
            assert!((lo - 0.02).abs() < 0.002 && (hi - 0.98).abs() < 0.002, "raster {raster}: cuts span x {lo:.3}..{hi:.3}");
            assert!(cuts.iter().all(|q| (q.z.unwrap() + 0.02).abs() < 1e-9));
            // dense enough to fill: far more cutting than the perimeter alone
            let len: f64 = res.paths[0].moves.windows(2).filter(|w| w[1].kind == MoveType::Linear && w[1].z.unwrap() < 0.0 && w[0].z.unwrap() < 0.0).map(|w| (w[1].x.unwrap() - w[0].x.unwrap()).hypot(w[1].y.unwrap() - w[0].y.unwrap())).sum();
            assert!(len > 40.0, "raster {raster}: {len:.1} in of lines");
        }
    }

    #[test]
    fn two_bit_pocket_clears_everything_and_the_small_bit_only_does_the_rest() {
        // 4 x 3 pocket, 1/2 in bit to clear, 1/8 in bit for the corners
        let shapes = [make_polyline(&[pt(0.0, 0.0), pt(4.0, 0.0), pt(4.0, 3.0), pt(0.0, 3.0)], true)];
        let small = Tool { diameter: 0.125, stepdown: 0.25, feed_xy: 60.0, feed_z: 20.0, ..Default::default() };
        let big = Tool { diameter: 0.5, stepdown: 0.25, feed_xy: 120.0, feed_z: 40.0, ..Default::default() };
        let m = Material::default();
        let op = Op { kind: OpType::Pocket, name: "P".into(), depth: 0.25, stepover: 0.4, climb: true, clear_tool: 1, ..Default::default() };
        let res = generate_pocket_two(&op, &small, &shapes, &m, Some(&big));
        assert!(res.warnings.is_empty(), "{:?}", res.warnings);
        assert_eq!(res.paths.len(), 2);
        assert_eq!((res.paths[0].tool, res.paths[0].feed_xy, res.paths[1].tool, res.paths[1].feed_xy), (1, 120.0, -1, 60.0));
        // floor coverage on a 0.02 in grid by both bits together; the small bit's cutting length is a fraction of a full pocket's
        let mut covered = vec![false; 200 * 150];
        let mut cut_len = [0.0, 0.0];
        for (k, (tp, r)) in res.paths.iter().zip([0.25, 0.0625]).enumerate() {
            for w in tp.moves.windows(2) {
                let (a, b) = (&w[0], &w[1]);
                if b.kind != MoveType::Linear || b.z.unwrap() > -0.25 + 1e-9 || a.z.unwrap() > -0.25 + 1e-9 {
                    continue;
                }
                let (ax, ay, bx, by) = (a.x.unwrap(), a.y.unwrap(), b.x.unwrap(), b.y.unwrap());
                cut_len[k] += (bx - ax).hypot(by - ay);
                let steps = ((bx - ax).hypot(by - ay) / 0.01).ceil().max(1.0) as usize;
                for i in 0..=steps {
                    let (x, y) = (ax + (bx - ax) * i as f64 / steps as f64, ay + (by - ay) * i as f64 / steps as f64);
                    assert!(x >= r - 0.002 && x <= 4.0 - r + 0.002 && y >= r - 0.002 && y <= 3.0 - r + 0.002, "bit {k} leaves the pocket at {x:.3},{y:.3}");
                    let (c0, c1, r0, r1) = (((x - r) / 0.02).floor().max(0.0) as usize, (((x + r) / 0.02).ceil() as usize).min(199), ((y - r) / 0.02).floor().max(0.0) as usize, (((y + r) / 0.02).ceil() as usize).min(149));
                    for cy in r0..=r1 {
                        for cx in c0..=c1 {
                            if (cx as f64 * 0.02 + 0.01 - x).hypot(cy as f64 * 0.02 + 0.01 - y) <= r {
                                covered[cy * 200 + cx] = true;
                            }
                        }
                    }
                }
            }
        }
        // every cell the small bit could reach is cut (corners keep its own radius)
        let missed = (0..200 * 150).filter(|&i| { let (x, y) = ((i % 200) as f64 * 0.02 + 0.01, (i / 200) as f64 * 0.02 + 0.01); let d = |v: f64, hi: f64| (0.0625 - v).max(v - (hi - 0.0625)).max(0.0); let inside_reach = d(x, 4.0) == 0.0 || d(y, 3.0) == 0.0 || d(x, 4.0).hypot(d(y, 3.0)) <= 0.0625 - 0.02; inside_reach && !covered[i] }).count();
        assert_eq!(missed, 0, "floor cells neither bit reached");
        let full_small = generate_pocket(&op, &small, &shapes, &m).paths[0].moves.windows(2).filter(|w| w[1].kind == MoveType::Linear && w[1].z.unwrap() < -0.25 + 1e-9 && w[0].z.unwrap() < -0.25 + 1e-9).map(|w| (w[1].x.unwrap() - w[0].x.unwrap()).hypot(w[1].y.unwrap() - w[0].y.unwrap())).sum::<f64>();
        assert!(cut_len[1] < full_small * 0.2, "small bit cuts {:.1} in vs {:.1} for the whole pocket", cut_len[1], full_small);
        // no larger bit chosen: the plain pocket
        assert_eq!(generate_pocket_two(&op, &big, &shapes, &m, Some(&small)).paths.len(), 1);
    }

    #[test]
    fn tiles_cover_every_cut_once_shifted_to_zero() {
        // an outside profile round a 10 x 2 part on a 12 in board, cut in 5 in tiles with 1 in overlap
        let part = [make_polyline(&[pt(1.0, 1.0), pt(11.0, 1.0), pt(11.0, 3.0), pt(1.0, 3.0)], true)];
        let tool = Tool { diameter: 0.25, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, ..Default::default() };
        let m = Material { w: 12.0, h: 4.0, ..Default::default() };
        let op = Op { kind: OpType::Profile, name: "P".into(), side: Side::Outside, depth: 0.5, climb: true, entry: Entry::Plunge, ..Default::default() };
        let tp = generate_profile(&op, &tool, &part, &m).paths.remove(0);
        let cut_len = |moves: &[Move], shift: f64, lo: f64, hi: f64| -> f64 {
            // length of cutting below the surface within lo..hi (x on the full board)
            let mut cur = (0.0, 0.0, 1.0);
            let mut sum = 0.0;
            for mv in moves {
                let to = (mv.x.unwrap_or(cur.0), mv.y.unwrap_or(cur.1), mv.z.unwrap_or(cur.2));
                if mv.kind == MoveType::Linear && to.2 < 0.0 && cur.2 < 0.0 {
                    // the share of the segment whose board x lies in lo..hi
                    let (x0, x1) = (cur.0.min(to.0) + shift, cur.0.max(to.0) + shift);
                    let share = if x1 - x0 < 1e-12 { f64::from(u8::from(x0 >= lo && x0 < hi)) } else { ((x1.min(hi) - x0.max(lo)) / (x1 - x0)).max(0.0) };
                    sum += (to.0 - cur.0).hypot(to.1 - cur.1) * share;
                }
                cur = to;
            }
            sum
        };
        let whole = cut_len(&tp.moves, 0.0, -1e9, 1e9);
        let job = Job { filename: "J".into(), toolpaths: vec![tp], ..Default::default() };
        let tiles = tile_jobs(std::slice::from_ref(&job), true, 5.0, 1.0, 12.0);
        assert_eq!(tiles.len(), 3, "tiles start at 0, 4 and 8");
        assert_eq!(tiles[1].filename, "J_Tile2of3");
        let lift = m.retract_z;
        let mut covered = 0.0;
        for (k, t) in tiles.iter().enumerate() {
            let mv = &t.toolpaths[0].moves;
            let mut cur = (0.0, 0.0, lift);
            for q in mv {
                let to = (q.x.unwrap(), q.y.unwrap(), q.z.unwrap());
                assert!(to.0 >= -1e-9 && to.0 <= 5.0 + 1e-9, "tile {k}: x {} outside the tile", to.0);
                if q.kind == MoveType::Rapid && ((to.0 - cur.0).abs() > 1e-9 || (to.1 - cur.1).abs() > 1e-9) {
                    assert!(cur.2 >= lift - 1e-9 && to.2 >= lift - 1e-9, "tile {k}: sideways rapid below travel height");
                }
                cur = to;
            }
            assert!((cur.2 - lift).abs() < 1e-9, "tile {k} ends lifted");
            // each tile's own 4 in (its stride) of the board, so the overlaps aren't counted twice
            covered += cut_len(mv, k as f64 * 4.0, k as f64 * 4.0, if k == 2 { 1e9 } else { k as f64 * 4.0 + 4.0 });
        }
        assert!((covered - whole).abs() < 0.01, "cut length {covered} vs {whole}");
        // a board that fits is left alone
        assert_eq!(tile_jobs(std::slice::from_ref(&job), true, 20.0, 1.0, 12.0).len(), 1);
    }

    #[test]
    fn raster_pocket_clears_the_whole_floor_and_stays_inside() {
        // 4 x 3 pocket with a 1 x 1 island in the middle, 1/4 in bit, rows at 30 degrees
        let shapes = [make_polyline(&[pt(0.0, 0.0), pt(4.0, 0.0), pt(4.0, 3.0), pt(0.0, 3.0)], true), make_polyline(&[pt(1.5, 1.0), pt(2.5, 1.0), pt(2.5, 2.0), pt(1.5, 2.0)], true)];
        let tool = Tool { diameter: 0.25, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, ..Default::default() };
        let m = Material::default();
        for (angle, entry) in [(0, Entry::Plunge), (30, Entry::Ramp), (90, Entry::Plunge)] {
            let op = Op { kind: OpType::Pocket, name: "P".into(), depth: 0.5, stepover: 0.4, raster: true, raster_angle: angle, climb: true, entry, ..Default::default() };
            let res = generate_pocket(&op, &tool, &shapes, &m);
            assert!(res.warnings.is_empty() && res.paths.len() == 1);
            let mv = &res.paths[0].moves;
            let in_pocket = |x: f64, y: f64| { let e = 0.125 - 0.002; x >= e && x <= 4.0 - e && y >= e && y <= 3.0 - e && (1.5 - x).max(x - 2.5).max(0.0).hypot((1.0 - y).max(y - 2.0).max(0.0)) >= e };
            // the bit centre never leaves the pocket while down: check every cut and every point along it
            let mut covered = vec![false; 80 * 60]; // 0.05 in cells: is the floor reached by the bit at full depth?
            for w in mv.windows(2) {
                let (a, b) = (&w[0], &w[1]);
                if b.kind != MoveType::Linear {
                    continue;
                }
                let (ax, ay, bx, by) = (a.x.unwrap(), a.y.unwrap(), b.x.unwrap(), b.y.unwrap());
                let steps = ((bx - ax).hypot(by - ay) / 0.02).ceil().max(1.0) as usize;
                for i in 0..=steps {
                    let t = i as f64 / steps as f64;
                    let (x, y) = (ax + (bx - ax) * t, ay + (by - ay) * t);
                    assert!(in_pocket(x, y), "angle {angle}: cut at {x:.3},{y:.3} is outside the pocket");
                    if b.z.unwrap() < -0.5 + 1e-9 && a.z.unwrap() < -0.5 + 1e-9 {
                        for cy in 0..60 {
                            for cx in 0..80 {
                                let (px, py) = (cx as f64 * 0.05 + 0.025, cy as f64 * 0.05 + 0.025);
                                if (px - x).hypot(py - y) <= 0.125 {
                                    covered[cy * 80 + cx] = true;
                                }
                            }
                        }
                    }
                }
            }
            let missed = (0..80 * 60).filter(|&i| { let (px, py) = ((i % 80) as f64 * 0.05 + 0.025, (i / 80) as f64 * 0.05 + 0.025); in_pocket(px, py) && !covered[i] }).count();
            assert_eq!(missed, 0, "angle {angle}: floor cells the bit never reached");
            // no sideways rapid below the surface
            for w in mv.windows(2) {
                if w[1].kind == MoveType::Rapid && (w[1].x != w[0].x || w[1].y != w[0].y) {
                    assert!(w[0].z.unwrap() > 0.0 && w[1].z.unwrap() > 0.0, "angle {angle}: sideways rapid in the material");
                }
            }
        }
    }

    #[test]
    fn leads_plunge_in_the_waste_and_never_cut_the_part() {
        let sq = [make_polyline(&[pt(1.0, 1.0), pt(3.0, 1.0), pt(3.0, 3.0), pt(1.0, 3.0)], true)];
        let tool = Tool { diameter: 0.25, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, ..Default::default() };
        let m = Material::default();
        for (side, climb) in [(Side::Outside, true), (Side::Outside, false), (Side::Inside, true), (Side::Inside, false)] {
            let op = Op { kind: OpType::Profile, name: "P".into(), side, depth: 0.5, ramp_in: 0.25, climb, entry: Entry::Plunge, ..Default::default() };
            let res = generate_profile(&op, &tool, &sq, &m);
            assert!(res.warnings.is_empty(), "{side:?} {climb}: {:?}", res.warnings);
            let mv = &res.paths[0].moves;
            // bit centre never closer to the part than its radius: outside cuts stay ≥ r outside the square, inside ≥ r inside
            let margin = |x: f64, y: f64| if side == Side::Inside { (x - 1.0).min(3.0 - x).min(y - 1.0).min(3.0 - y) } else { (1.0 - x).max(x - 3.0).max(0.0).hypot((1.0 - y).max(y - 3.0).max(0.0)) };
            for q in mv.iter().filter(|q| q.kind == MoveType::Linear) {
                assert!(margin(q.x.unwrap(), q.y.unwrap()) >= 0.125 - 0.002, "{side:?} climb {climb}: cut at {:?},{:?} is in the part", q.x, q.y);
            }
            // every plunge (a Z-only move going down) happens off the line, in the waste
            let mut plunges = 0;
            for w in mv.windows(2) {
                if w[1].kind == MoveType::Linear && w[1].x == w[0].x && w[1].y == w[0].y && w[1].z < w[0].z {
                    plunges += 1;
                    assert!(margin(w[1].x.unwrap(), w[1].y.unwrap()) > 0.125 + 0.05, "{side:?}: plunge on the line");
                }
            }
            assert_eq!(plunges, 2, "one plunge per pass");
        }
        // a hole too small for the lead: warned, cut without it
        let small = [make_polyline(&[pt(0.0, 0.0), pt(0.5, 0.0), pt(0.5, 0.5), pt(0.0, 0.5)], true)];
        let op = Op { kind: OpType::Profile, name: "P".into(), side: Side::Inside, depth: 0.25, ramp_in: 0.25, climb: true, ..Default::default() };
        assert_eq!(generate_profile(&op, &tool, &small, &m).warnings.len(), 1);
    }

    #[test]
    fn separate_last_pass_roughs_off_the_line_then_finishes_on_it() {
        let sq = [make_polyline(&[pt(1.0, 1.0), pt(3.0, 1.0), pt(3.0, 3.0), pt(1.0, 3.0)], true)];
        let tool = Tool { diameter: 0.25, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, ..Default::default() };
        let m = Material::default();
        let op = Op { kind: OpType::Profile, name: "P".into(), side: Side::Outside, depth: 0.75, allowance: 0.02, climb: true, ..Default::default() };
        let res = generate_profile(&op, &tool, &sq, &m);
        assert_eq!(res.paths.len(), 1, "one toolpath: rough then finish");
        let cuts: Vec<_> = res.paths[0].moves.iter().filter(|mv| mv.kind == MoveType::Linear).collect();
        // distance of a cut point from the square's outline (outside it)
        let off = |mv: &&Move| { let (x, y) = (mv.x.unwrap(), mv.y.unwrap()); let dx = (1.0 - x).max(x - 3.0).max(0.0); let dy = (1.0 - y).max(y - 3.0).max(0.0); dx.hypot(dy).max(dx.max(dy)) };
        let (rough, finish): (Vec<_>, Vec<_>) = cuts.iter().partition(|mv| off(mv) > 0.135);
        assert!(!rough.is_empty() && !finish.is_empty());
        assert!(rough.iter().all(|mv| (off(mv) - 0.145).abs() < 0.002), "rough passes sit r + allowance off the line");
        assert!(finish.iter().all(|mv| (off(mv) - 0.125).abs() < 0.002), "the last pass is on the line");
        // every rough cut comes before the first finishing cut, and the finish reaches full depth
        let first_finish = cuts.iter().position(|mv| off(mv) < 0.135).unwrap();
        assert!(cuts[first_finish..].iter().all(|mv| off(mv) < 0.135));
        assert!(finish.iter().any(|mv| (mv.z.unwrap() + 0.75).abs() < 1e-9) && rough.iter().any(|mv| (mv.z.unwrap() + 0.75).abs() < 1e-9));
        // off: the same op without an allowance is unchanged (one ring on the line)
        let plain = generate_profile(&Op { allowance: 0.0, ..op.clone() }, &tool, &sq, &m);
        assert!(plain.paths[0].moves.iter().filter(|mv| mv.kind == MoveType::Linear).all(|mv| (off(&mv) - 0.125).abs() < 0.002));
    }

    #[test]
    fn thread_mill_rises_one_pitch_per_turn_from_the_bottom() {
        let op = Op { kind: OpType::Bore, name: "T".into(), depth: 0.4, thread_pitch: 0.1, ..Default::default() };
        let tool = Tool { diameter: 0.25, ..Default::default() };
        let m = Material { w: 4.0, h: 4.0, t: 0.75, ..Default::default() };
        let r = generate_bore(&op, &tool, &[crate::geom::make_ellipse(2.0, 2.0, 0.5, 0.5)], &m);
        assert_eq!(r.paths.len(), 1, "{:?}", r.warnings);
        let zs: Vec<f64> = r.paths[0].moves.iter().filter(|mv| mv.kind != MoveType::Rapid).filter_map(|mv| mv.z).collect();
        assert!((zs[0] + 0.4).abs() < 1e-9, "starts at the bottom");
        // 4 turns of 0.1: the helix ends at the surface and rises monotonically
        assert!(zs.windows(2).all(|w| w[1] >= w[0] - 1e-9) && zs.last().unwrap().abs() < 1e-9, "{zs:?}");
        // hole 1.0 across, bit 0.25: the helix radius is 0.375
        let far = r.paths[0].moves.iter().filter_map(|mv| Some((mv.x? - 2.0).hypot(mv.y? - 2.0))).fold(0.0, f64::max);
        assert!((far - 0.375).abs() < 1e-6, "{far}");
    }

    #[test]
    fn a_profile_can_start_at_the_vectors_first_node() {
        // square 0..4: default start = lowest then leftmost corner (0,0); first node set to the top-right corner
        let mut sq = crate::geom::make_rect(0.0, 0.0, 4.0, 4.0);
        sq.nodes.rotate_left(2); // the node order now starts at (4,4)
        let m = Material { w: 10.0, h: 10.0, t: 0.75, ..Default::default() };
        let tool = Tool { diameter: 0.25, ..Default::default() };
        let start = |first: bool| {
            let op = Op { kind: OpType::Profile, name: "P".into(), depth: 0.25, side: Side::On, start_first: first, ..Default::default() };
            let r = generate_profile(&op, &tool, &[sq.clone()], &m);
            let mv = r.paths[0].moves.iter().find(|mv| mv.x.is_some() && mv.y.is_some()).unwrap();
            (mv.x.unwrap(), mv.y.unwrap())
        };
        let (x0, y0) = start(false);
        assert!(x0.abs() < 1e-6 && y0.abs() < 1e-6, "{x0} {y0}");
        let (x1, y1) = start(true);
        assert!((x1 - 4.0).abs() < 1e-6 && (y1 - 4.0).abs() < 1e-6, "{x1} {y1}");
    }
}
