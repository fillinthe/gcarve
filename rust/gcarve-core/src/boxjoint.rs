//! Box Maker: a finger-jointed box (outer width × depth × height, board thickness t) as flat panels to cut.
//!
//! Every joined edge is split into an odd number of equal fingers; one panel keeps the even ones, its neighbour the
//! odd ones (odd count = the same at both ends, so the direction along the edge doesn't matter). Bottom and lid own
//! the corner cubes, front / back own the vertical edges' ends: every cube of the box shell belongs to exactly one
//! panel (test: panel areas × t = shell volume). Slots are widened by `play` for the fit; bit corners are relieved
//! with dogbones when a bit diameter is given. Own design (boxes.py, GPL, was only looked at for the idea).

use crate::geom::{make_rect, Contour};
use crate::vecops::{boolean_op, dogbone, BoolOp};

#[derive(Clone, Copy, Debug)]
pub struct BoxSpec {
    /// outer sizes, inches
    pub w: f64,
    pub d: f64,
    pub h: f64,
    /// board thickness = finger depth
    pub t: f64,
    /// wanted finger width (rounded so each edge gets a whole, odd number)
    pub finger: f64,
    /// extra slot width for the fit (total, split both sides)
    pub play: f64,
    /// a lid jointed like the bottom; false = open top
    pub lid: bool,
    /// router bit diameter for dogbones, 0 = sharp corners
    pub bit: f64,
}

/// Fingers on an edge of length l: odd, at least 3.
fn count(l: f64, finger: f64) -> usize {
    let k = (l / finger.max(1e-3)).round().max(3.0) as usize;
    if k.is_multiple_of(2) { k - 1 } else { k }
}

/// One panel: a × b, with each edge (bottom, right, top, left) free (None) or jointed (Some(keeps the even fingers)).
fn panel(s: &BoxSpec, a: f64, b: f64, edges: [Option<bool>; 4]) -> Vec<Contour> {
    let (t, e) = (s.t, 0.01); // slots reach a little past the edge so the subtraction leaves no sliver
    let mut slots = Vec::new();
    for (k, edge) in edges.iter().enumerate() {
        let Some(even) = *edge else { continue };
        let l = if k % 2 == 0 { a } else { b };
        let n = count(l, s.finger);
        let seg = l / n as f64;
        for i in (0..n).filter(|i| (i % 2 == 0) != even) {
            let (u0, u1) = (i as f64 * seg - s.play / 2.0, (i + 1) as f64 * seg + s.play / 2.0);
            slots.push(match k {
                0 => make_rect(u0, -e, u1, t),
                1 => make_rect(a - t, u0, a + e, u1),
                2 => make_rect(u0, b - t, u1, b + e),
                _ => make_rect(-e, u0, t, u1),
            });
        }
    }
    let out = boolean_op(BoolOp::Subtract, &[make_rect(0.0, 0.0, a, b)], &slots);
    if s.bit > 0.0 {
        out.iter().map(|c| dogbone(c, s.bit / 2.0 + 0.0005, false, false)).collect()
    } else {
        out
    }
}

/// The panels, each in its own coordinates (corner at 0,0), with its name and size.
pub fn panels(s: &BoxSpec) -> Vec<(&'static str, f64, f64, Vec<Contour>)> {
    let j = Some;
    let lid = |even: bool| if s.lid { Some(even) } else { None };
    let mut out = vec![("Bottom", s.w, s.d, panel(s, s.w, s.d, [j(true); 4]))];
    if s.lid {
        out.push(("Lid", s.w, s.d, panel(s, s.w, s.d, [j(true); 4])));
    }
    for name in ["Front", "Back"] {
        out.push((name, s.w, s.h, panel(s, s.w, s.h, [j(false), j(true), lid(false), j(true)])));
    }
    for name in ["Left side", "Right side"] {
        out.push((name, s.d, s.h, panel(s, s.d, s.h, [j(false), j(false), lid(false), j(false)])));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // straight-sided panels: the shoelace over the nodes is the area
    fn signed_area(c: &Contour) -> f64 {
        let n = c.nodes.len();
        (0..n).map(|i| { let (a, b) = (c.nodes[i].p, c.nodes[(i + 1) % n].p); a.x * b.y - b.x * a.y }).sum::<f64>() / 2.0
    }

    #[test]
    fn every_bit_of_the_box_shell_belongs_to_exactly_one_panel() {
        for lid in [true, false] {
            let s = BoxSpec { w: 8.0, d: 5.0, h: 3.5, t: 0.5, finger: 1.0, play: 0.0, lid, bit: 0.0 };
            let ps = panels(&s);
            assert_eq!(ps.len(), if lid { 6 } else { 5 });
            let area: f64 = ps.iter().flat_map(|p| p.3.iter()).map(|c| signed_area(c).abs()).sum();
            let inner_h = if lid { s.h - 2.0 * s.t } else { s.h - s.t };
            let shell = s.w * s.d * s.h - (s.w - 2.0 * s.t) * (s.d - 2.0 * s.t) * inner_h;
            assert!((area * s.t - shell).abs() < 1e-3, "lid {lid}: {} vs {shell}", area * s.t);
            for (name, a, b, cs) in &ps {
                assert_eq!(cs.len(), 1, "{name} is one piece");
                let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for p in &cs[0].nodes {
                    (x0, y0, x1, y1) = (x0.min(p.p.x), y0.min(p.p.y), x1.max(p.p.x), y1.max(p.p.y));
                }
                assert!((x1 - x0 - a).abs() < 1e-6 && (y1 - y0 - b).abs() < 1e-6, "{name} keeps its outer size");
            }
        }
        // play widens the slots: less wood, never more
        let tight = BoxSpec { w: 8.0, d: 5.0, h: 3.5, t: 0.5, finger: 1.0, play: 0.0, lid: true, bit: 0.0 };
        let loose = BoxSpec { play: 0.01, ..tight };
        let sum = |s: &BoxSpec| panels(s).iter().flat_map(|p| p.3.iter()).map(|c| signed_area(c).abs()).sum::<f64>();
        assert!(sum(&loose) < sum(&tight));
        // a bit diameter relieves the inside corners (more nodes: the lobes), the outer size stays
        let boned = panels(&BoxSpec { bit: 0.25, ..tight });
        assert!(boned[0].3[0].nodes.len() > panels(&tight)[0].3[0].nodes.len() + 8);
        assert_eq!(count(10.0, 1.0), 9);
        assert_eq!(count(1.0, 1.0), 3);
    }
}
