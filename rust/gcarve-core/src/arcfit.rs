//! Replaces runs of straight G1 moves at constant Z that lie on one circle with G2/G3 arcs (C++ `core/arcfit.*`).

use std::f64::consts::PI;

use crate::geom::{dist, pt, Pt};
use crate::job::{Move, MoveType, Toolpath};

/// C++ defaults: tolerance 0.001 in, max sweep a quarter turn.
pub const DEFAULT_TOL: f64 = 0.001;
pub const DEFAULT_MAX_SWEEP: f64 = PI / 2.0;

#[derive(Clone, Copy)]
struct Circle {
    c: Pt,
    r: f64,
}

fn through(a: Pt, b: Pt, c: Pt) -> Option<Circle> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-12 {
        return None; // collinear
    }
    let (a2, b2, c2) = (a.x * a.x + a.y * a.y, b.x * b.x + b.y * b.y, c.x * c.x + c.y * c.y);
    let ctr = pt((a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d, (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d);
    Some(Circle { c: ctr, r: dist(ctr, a) })
}

/// Sweep from a to b around c, in the direction given by ccw, in [0, 2pi).
fn sweep(c: Pt, a: Pt, b: Pt, ccw: bool) -> f64 {
    let mut s = (b.y - c.y).atan2(b.x - c.x) - (a.y - c.y).atan2(a.x - c.x);
    if !ccw {
        s = -s;
    }
    while s < 0.0 {
        s += 2.0 * PI;
    }
    while s >= 2.0 * PI {
        s -= 2.0 * PI;
    }
    s
}

/// Does the polyline pts[i..=j] fit one arc? Returns the circle and direction (ccw).
fn fits(pts: &[Pt], i: usize, j: usize, tol: f64, max_sweep: f64) -> Option<(Circle, bool)> {
    if j - i < 2 {
        return None;
    }
    let c = through(pts[i], pts[(i + j) / 2], pts[j])?;
    if c.r < 0.01 || c.r > 1000.0 {
        return None; // Fusion post limits for WinCNC
    }
    // direction from the first step that actually moves (pocket links repeat a point: a zero step gives a random sign)
    let mut k1 = i + 1;
    while k1 < j && dist(pts[k1], pts[i]) < 1e-9 {
        k1 += 1;
    }
    let (a, b) = (pts[i] - c.c, pts[k1] - c.c);
    let ccw = a.x * b.y - a.y * b.x > 0.0;
    if sweep(c.c, pts[i], pts[j], ccw) > max_sweep + 1e-9 {
        return None;
    }
    let mut prev = 0.0;
    for k in i + 1..=j {
        if (dist(pts[k], c.c) - c.r).abs() > tol {
            return None;
        }
        let s = sweep(c.c, pts[i], pts[k], ccw);
        if s < prev - 1e-9 {
            return None; // must advance monotonically
        }
        prev = s;
        // the chord between consecutive points must hug the arc: sagitta check
        let chord = dist(pts[k - 1], pts[k]);
        if chord > 2.0 * c.r {
            return None;
        }
        let sag = c.r - (c.r * c.r - chord * chord / 4.0).max(0.0).sqrt();
        if sag > tol * 2.0 {
            return None; // original segments were themselves > tol off any arc through them
        }
    }
    Some((c, ccw))
}

/// tol: max deviation of the original points from the fitted arc (inches). max_sweep in radians.
pub fn fit_arcs(input: &Toolpath, tol: f64, max_sweep: f64) -> Toolpath {
    let mut out = Toolpath { moves: Vec::new(), ..input.clone() };
    let mv = &input.moves;
    let n = mv.len();
    let mut i = 0;
    // current position tracking: moves may omit coordinates
    let mut cur = pt(0.0, 0.0);
    let mut cur_z: Option<f64> = None;
    while i < n {
        let m = &mv[i];
        // collect a run of G1 moves at constant Z with XY motion, starting from the current position
        let starts_run = m.kind == MoveType::Linear && m.x.is_some() && m.y.is_some() && m.z.is_none_or(|z| cur_z == Some(z));
        if !starts_run {
            out.moves.push(*m);
            if let Some(x) = m.x {
                cur.x = x;
            }
            if let Some(y) = m.y {
                cur.y = y;
            }
            if m.z.is_some() {
                cur_z = m.z;
            }
            i += 1;
            continue;
        }
        let mut pts = vec![cur];
        let mut j = i;
        while j < n && mv[j].kind == MoveType::Linear && mv[j].x.is_some() && mv[j].y.is_some() && (mv[j].z.is_none() || cur_z.is_none() || mv[j].z == cur_z) {
            pts.push(pt(mv[j].x.unwrap(), mv[j].y.unwrap()));
            j += 1;
        }
        // greedy arc fitting over pts; pts[k] corresponds to move i+k-1 (pts[0] is the start position)
        let mut k = 0;
        while k + 1 < pts.len() {
            let mut best: Option<(Circle, bool, usize)> = None;
            for e in k + 2..pts.len() {
                if let Some((c, ccw)) = fits(&pts, k, e, tol, max_sweep) {
                    best = Some((c, ccw, e));
                } else if best.is_some() || e - k > 8 {
                    break; // extension failed after a fit, or never fit in the first few points: not an arc
                }
            }
            if let Some((c, ccw, end)) = best {
                out.moves.push(Move {
                    kind: if ccw { MoveType::ArcCcw } else { MoveType::ArcCw },
                    x: Some(pts[end].x),
                    y: Some(pts[end].y),
                    z: None,
                    i: c.c.x - pts[k].x,
                    j: c.c.y - pts[k].y,
                });
                k = end;
            } else {
                out.moves.push(mv[i + k]);
                k += 1;
            }
        }
        cur = *pts.last().unwrap();
        i = j;
    }
    out
}
