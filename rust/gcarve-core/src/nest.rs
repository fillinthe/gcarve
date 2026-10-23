//! True-shape nesting (C++ core/nest.*): bottom-left placement using no-fit polygons (Clipper2 Minkowski sums).

use std::f64::consts::PI;

use clipper2_rust::{
    area, difference_d, get_bounds_path, inflate_paths_d, intersect_d, is_positive, minkowski_sum_d, union_subjects_d, EndType, FillRule, JoinType,
    PathD, PathsD, PointD,
};

use crate::geom::Pt;

const PREC: i32 = 4; // 1e-4 in

/// Closed outer polygon of the part (inches); holes are not needed.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NestPart {
    pub outline: Vec<Pt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct NestPlacement {
    pub placed: bool,
    /// radians, applied about the origin first
    pub rotation: f64,
    /// then translate by this
    pub offset: Pt,
    /// mirrored left to right (about x = 0) before the rotation
    pub mirror: bool,
}

fn to_path(pts: &[Pt]) -> PathD {
    pts.iter().map(|q| PointD::new(q.x, q.y)).collect()
}
fn map(p: &PathD, f: impl Fn(&PointD) -> PointD) -> PathD {
    p.iter().map(f).collect()
}

/// Largest path of a set (the inflated outline may come back with slivers).
fn largest(ps: &PathsD) -> PathD {
    let mut best = PathD::new();
    let mut ba = -1.0;
    for p in ps {
        let a = area(p).abs();
        if a > ba {
            ba = a;
            best = p.clone();
        }
    }
    best
}

/// rot_step_deg: 0 = no rotation, 90 = try 0/90/180/270, 45, 15 ... Parts are tried largest-area first.
/// ponytail: single sheet, bottom-left heuristic over NFP vertices; good for tens of parts, not a genetic optimiser.
pub fn nest_shapes(parts: &[NestPart], sheet_w: f64, sheet_h: f64, gap: f64, margin: f64, rot_step_deg: i32) -> Vec<NestPlacement> {
    nest_shapes_with(parts, sheet_w, sheet_h, gap, margin, rot_step_deg, false, None)
}

/// `nest_shapes`, also trying each part mirrored when `mirror` (VCarve "Mirror parts to find best fit"), and only
/// inside the closed `boundary` when given (VCarve "Nest boundary": an offcut's outline)
#[allow(clippy::too_many_arguments)]
pub fn nest_shapes_with(parts: &[NestPart], sheet_w: f64, sheet_h: f64, gap: f64, margin: f64, rot_step_deg: i32, mirror: bool, boundary: Option<&[Pt]>) -> Vec<NestPlacement> {
    let boundary: Option<PathD> = boundary.map(to_path).filter(|b| b.len() >= 3).map(|mut b| {
        if !is_positive(&b) {
            b.reverse();
        }
        b
    });
    let mut out = vec![NestPlacement::default(); parts.len()];
    let areas: Vec<f64> = parts.iter().map(|p| area(&to_path(&p.outline)).abs()).collect();
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|&a, &b| areas[b].total_cmp(&areas[a]));

    let mut rots = vec![0.0];
    if rot_step_deg > 0 {
        rots.extend((1..).map(|k| k * rot_step_deg).take_while(|&d| d < 360).map(|d| d as f64 * PI / 180.0));
    }

    let half = gap / 2.0;
    let (x0, y0, x1, y1) = (margin, margin, sheet_w - margin, sheet_h - margin);
    let mut placed_outlines: Vec<PathD> = Vec::new(); // inflated, in sheet coordinates

    for i in order {
        let plain = to_path(&parts[i].outline);
        if plain.len() < 3 {
            continue;
        }
        let mut found = false;
        let (mut best_y, mut best_x, mut best_rot, mut best_mirror) = (1e300, 1e300, 0.0, false);
        let mut best_outline = PathD::new();
        let mut best_off = Pt::default();
        for (flip, &rot) in [false, true].into_iter().filter(|f| !*f || mirror).flat_map(|f| rots.iter().map(move |r| (f, r))) {
            let mut base = if flip { map(&plain, |q| PointD::new(-q.x, q.y)) } else { plain.clone() };
            if !is_positive(&base) {
                base.reverse();
            }
            let (s, c) = rot.sin_cos();
            let r = map(&base, |q| PointD::new(c * q.x - s * q.y, s * q.x + c * q.y));
            let infl = if half > 0.0 { inflate_paths_d(&vec![r], half, JoinType::Round, EndType::Polygon, 2.0, PREC, 0.0) } else { vec![r] };
            let p = largest(&infl);
            if p.is_empty() {
                continue;
            }
            let b = get_bounds_path(&p);
            let (w, h) = (b.width(), b.height());
            if w > x1 - x0 + 1e-9 || h > y1 - y0 + 1e-9 {
                continue;
            }
            // reference point: the inflated outline's bbox min corner moved to the origin (RectD top = min y here)
            let rf = map(&p, |q| PointD::new(q.x - b.left, q.y - b.top));
            // feasible reference positions inside the sheet
            let ifp: PathD = vec![PointD::new(x0, y0), PointD::new(x1 - w, y0), PointD::new(x1 - w, y1 - h), PointD::new(x0, y1 - h)];
            let mut feasible: PathsD = vec![ifp];
            if !placed_outlines.is_empty() {
                let neg = map(&rf, |q| PointD::new(-q.x, -q.y));
                // Clipper's Minkowski sum sweeps the part along the placed part's OUTLINE: positions where the new part
                // would sit wholly inside a larger placed part touch no outline and were left "free" (a small part got
                // nested on top of a big one). Such a position puts every point of the new part inside the placed one,
                // so the placed outline shifted by one of the new part's own points covers exactly those positions.
                let nfps: PathsD = placed_outlines.iter().flat_map(|a| {
                    let mut v = minkowski_sum_d(&neg, a, true, 2);
                    v.push(map(a, |q| PointD::new(q.x - rf[0].x, q.y - rf[0].y)));
                    v
                }).collect();
                let nfps = union_subjects_d(&nfps, FillRule::NonZero, PREC);
                feasible = difference_d(&feasible, &nfps, FillRule::NonZero, PREC);
            }
            if let Some(bp) = &boundary {
                // inside the boundary: the reference point's own spot inside it, and the part crossing none of its edge
                let neg = map(&rf, |q| PointD::new(-q.x, -q.y));
                let within = vec![map(bp, |q| PointD::new(q.x - rf[0].x, q.y - rf[0].y))];
                let crossing = union_subjects_d(&minkowski_sum_d(&neg, bp, true, 2), FillRule::NonZero, PREC);
                let fits = difference_d(&within, &crossing, FillRule::NonZero, PREC);
                feasible = intersect_d(&feasible, &fits, FillRule::NonZero, PREC);
            }
            // bottom-left vertex of the feasible region
            for v in feasible.iter().flatten() {
                if v.y < best_y - 1e-9 || ((v.y - best_y).abs() <= 1e-9 && v.x < best_x - 1e-9) {
                    best_y = v.y;
                    best_x = v.x;
                    best_rot = rot;
                    best_mirror = flip;
                    found = true;
                    best_outline = map(&rf, |q| PointD::new(q.x + v.x, q.y + v.y));
                    best_off = Pt { x: v.x - b.left, y: v.y - b.top }; // offset to apply after rotating the ORIGINAL part
                }
            }
        }
        if !found {
            continue;
        }
        placed_outlines.push(best_outline);
        out[i] = NestPlacement { placed: true, rotation: best_rot, offset: best_off, mirror: best_mirror };
    }
    out
}

#[cfg(test)]
mod overlap_tests {
    use super::*;

    #[test]
    fn parts_nest_only_inside_the_boundary() {
        use crate::geom::pt;
        // a triangular offcut on a 20 x 10 sheet: squares must stay inside it
        let tri = [pt(0.0, 0.0), pt(10.0, 0.0), pt(0.0, 10.0)];
        let sq = vec![pt(0.0, 0.0), pt(1.0, 0.0), pt(1.0, 1.0), pt(0.0, 1.0)];
        let parts: Vec<NestPart> = (0..12).map(|_| NestPart { outline: sq.clone() }).collect();
        let places = nest_shapes_with(&parts, 20.0, 10.0, 0.0, 0.0, 0, false, Some(&tri));
        let n = places.iter().filter(|p| p.placed).count();
        assert!(n >= 8, "{n} placed");
        for p in places.iter().filter(|p| p.placed) {
            for q in &sq {
                let (x, y) = (q.x + p.offset.x, q.y + p.offset.y);
                assert!(x >= -1e-6 && y >= -1e-6 && x + y <= 10.0 + 1e-6, "({x}, {y}) is outside the offcut");
            }
        }
    }

    #[test]
    fn mirrored_placements_stay_on_the_sheet_and_apart() {
        use crate::geom::pt;
        // an L-shaped (handed) part, eight of them on a sheet just big enough
        let l = vec![pt(0.0, 0.0), pt(3.0, 0.0), pt(3.0, 1.0), pt(1.0, 1.0), pt(1.0, 2.0), pt(0.0, 2.0)];
        let parts: Vec<NestPart> = (0..8).map(|_| NestPart { outline: l.clone() }).collect();
        let places = nest_shapes_with(&parts, 10.0, 6.0, 0.1, 0.1, 90, true, None);
        let placed: Vec<Vec<Pt>> = places.iter().zip(&parts).filter(|(p, _)| p.placed).map(|(p, part)| {
            let (s, c) = p.rotation.sin_cos();
            part.outline.iter().map(|q| { let x = if p.mirror { -q.x } else { q.x }; pt(c * x - s * q.y + p.offset.x, s * x + c * q.y + p.offset.y) }).collect()
        }).collect();
        assert_eq!(placed.len(), 8);
        for o in &placed {
            assert!(o.iter().all(|q| q.x > 0.0 && q.x < 10.0 && q.y > 0.0 && q.y < 6.0), "{o:?}");
        }
        let paths: Vec<PathD> = placed.iter().map(|o| to_path(o)).collect();
        for a in 0..paths.len() {
            for b in a + 1..paths.len() {
                let both = clipper2_rust::intersect_d(&vec![paths[a].clone()], &vec![paths[b].clone()], FillRule::NonZero, PREC);
                assert!(both.iter().map(|p| area(p).abs()).sum::<f64>() < 1e-6, "parts {a} and {b} overlap");
            }
        }
    }

    /// placed outline of part i
    fn placed(parts: &[NestPart], out: &[NestPlacement], i: usize) -> Vec<Pt> {
        let (s, c) = out[i].rotation.sin_cos();
        parts[i].outline.iter().map(|q| Pt { x: c * q.x - s * q.y + out[i].offset.x, y: s * q.x + c * q.y + out[i].offset.y }).collect()
    }

    #[test]
    fn a_small_part_is_never_nested_on_top_of_a_bigger_one() {
        // a 16 x 14 rectangle, a pentagon that would fit inside it, and a small square: all must end up apart
        let rect = vec![Pt { x: 0.0, y: 0.0 }, Pt { x: 16.0, y: 0.0 }, Pt { x: 16.0, y: 14.0 }, Pt { x: 0.0, y: 14.0 }];
        let pent: Vec<Pt> = (0..5).map(|k| { let a = k as f64 * std::f64::consts::TAU / 5.0 + 0.3; Pt { x: 40.0 + 6.5 * a.cos(), y: 40.0 + 6.5 * a.sin() } }).collect();
        let small = vec![Pt { x: 0.0, y: 0.0 }, Pt { x: 3.0, y: 0.0 }, Pt { x: 3.0, y: 3.0 }, Pt { x: 0.0, y: 3.0 }];
        let parts = [NestPart { outline: rect }, NestPart { outline: pent }, NestPart { outline: small }];
        for rot in [0, 90] {
            let out = nest_shapes(&parts, 48.0, 48.0, 0.25, 0.5, rot);
            assert!(out.iter().all(|p| p.placed));
            let shapes: Vec<PathD> = (0..3).map(|i| placed(&parts, &out, i).iter().map(|q| PointD::new(q.x, q.y)).collect()).collect();
            for i in 0..3 {
                for j in i + 1..3 {
                    let both = clipper2_rust::intersect_d(&vec![shapes[i].clone()], &vec![shapes[j].clone()], FillRule::NonZero, PREC);
                    let overlap: f64 = both.iter().map(|p| area(p).abs()).sum();
                    assert!(overlap < 1e-6, "rotation step {rot}: parts {i} and {j} overlap by {overlap:.3} sq in");
                }
            }
        }
    }
}
