//! Waterline 3D finishing: laps at a constant height round the model, one every step down. Rows back and forth
//! (relief.rs `raster`) leave steep sides stair-stepped, because there one stepover sideways is a long way down;
//! a lap at each height keeps the same spacing down the wall. Each lap is a contour line of the bit-tip surface
//! (the lowest the tip may be at every point, `tip_height`), so the bit's true shape never gouges the model.
//! With a steepness limit only the stretches where that surface is steeper are cut: the rest is the rows' job.

use crate::cam::{clampf, cut_to, rapid_to, Material, Tool};
use crate::geom::{dist, pt, Pt};
use crate::job::Toolpath;
use crate::relief::{add_simplified, tip_height, Relief};
use std::collections::HashMap;

/// most points of the bit-tip grid
const MAX_GRID: f64 = 600_000.0;
/// the next lap this near is reached without lifting (when the way is clear)
const LINK: f64 = 0.3;

/// The laps into `tp`. `step_frac` × the bit = the step down between laps; `steep_deg` > 0 keeps only stretches where
/// the bit-tip surface is steeper than that (0 = whole laps).
pub(crate) fn waterline(r: &Relief, m: &Material, t: &Tool, step_frac: f64, steep_deg: f64, inside: &dyn Fn(f64, f64) -> bool, tp: &mut Toolpath) {
    let rad = t.cut_diameter() / 2.0;
    // room round the model for the widest the bit touches at this depth (a tapered ball's cone, not only its tip)
    let pad = (2.0 * rad).max(t.reach(r.depth) + rad);
    let (x0, y0) = ((r.x - pad).max(0.0), (r.y - pad).max(0.0));
    let (x1, y1) = ((r.x + r.w + pad).min(m.w), (r.y + r.h + pad).min(m.h));
    if x1 <= x0 || y1 <= y0 || r.nx == 0 {
        return;
    }
    let g = 0.004_f64.max((r.w / r.nx as f64).min(r.h / r.ny as f64)).max(((x1 - x0) * (y1 - y0) / MAX_GRID).sqrt());
    let (nx, ny) = (((x1 - x0) / g).ceil() as usize + 1, ((y1 - y0) / g).ceil() as usize + 1);
    let tip: Vec<f64> = (0..nx * ny).map(|k| tip_height(r, t, x0 + (k % nx) as f64 * g, y0 + (k / nx) as f64 * g, 0.0)).collect();
    let at = |i: usize, j: usize| tip[j * nx + i];
    // how steep the bit-tip surface is at a point (rise over run), from the cell it is in
    let grad = |p: Pt| {
        let (i, j) = ((((p.x - x0) / g) as usize).min(nx - 2), (((p.y - y0) / g) as usize).min(ny - 2));
        let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
        (((b - a) + (c - d)) / (2.0 * g), ((d - a) + (c - b)) / (2.0 * g))
    };
    let slope = |p: Pt| {
        let (gx, gy) = grad(p);
        gx.hypot(gy)
    };
    // The grid places a lap to within a cell, and against a wall a cell sideways is a long way down: slide each point
    // across the slope to where the bit just touches at this height (never inside the model; no such place = dropped)
    let settle = |p: Pt, z: f64| -> Option<Pt> {
        let (gx, gy) = grad(p);
        let l = gx.hypot(gy);
        let ok = |q: Pt| tip_height(r, t, q.x, q.y, 0.0) <= z + 1e-4;
        if l < 1e-9 {
            return ok(p).then_some(p);
        }
        let along = |s: f64| pt(p.x - gx / l * s, p.y - gy / l * s); // s > 0 = downhill, away from the model
        let (mut inside, mut clear) = (-g, 1.5 * g);
        if !ok(along(clear)) {
            return None;
        }
        if ok(along(inside)) {
            return ok(p).then_some(p);
        }
        for _ in 0..8 {
            let mid = (inside + clear) / 2.0;
            if ok(along(mid)) {
                clear = mid;
            } else {
                inside = mid;
            }
        }
        Some(along(clear))
    };
    let min_slope = if steep_deg > 0.0 { clampf(steep_deg, 1.0, 85.0).to_radians().tan() } else { 0.0 };
    let keep = |p: Pt| inside(p.x, p.y) && (min_slope == 0.0 || slope(p) >= min_slope);
    let dz = 0.002_f64.max(clampf(step_frac, 0.02, 1.0) * t.cut_diameter());
    // every lap of every height first, then the order: down one stack of laps (one side of the model) without
    // lifting where the next lap below is near, before moving to the next stack
    let mut laps: Vec<(f64, Vec<Pt>, bool)> = Vec::new();
    let mut z = -dz;
    while z > -r.depth + 1e-6 {
        let mut runs: Vec<(Vec<Pt>, bool)> = Vec::new();
        for (pts, closed) in contours(&tip, nx, ny, z - 1e-7) {
            let pts: Vec<Option<Pt>> = pts.into_iter().map(|p| pt(x0 + p.x * g, y0 + p.y * g)).map(|p| settle(p, z).filter(|q| keep(*q))).collect();
            split(pts, closed, &mut runs);
        }
        runs.retain(|(p, _)| p.windows(2).map(|w| dist(w[0], w[1])).sum::<f64>() >= rad.max(3.0 * g));
        for (mut p, closed) in runs {
            // climb: the model on the right of the way round (a raised part is lapped clockwise)
            let (a, b) = (p[0], p[1]);
            let (l, n) = (dist(a, b).max(1e-12), g * 0.7);
            let left = pt((a.x + b.x) / 2.0 - (b.y - a.y) / l * n, (a.y + b.y) / 2.0 + (b.x - a.x) / l * n);
            if tip_height(r, t, left.x, left.y, 0.0) > z {
                p.reverse();
            }
            laps.push((z, p, closed));
        }
        z -= dz;
    }
    let mut cur = pt(x0, y0);
    let mut cur_z = 0.0;
    let mut first = true;
    // how near a lap's start can be (a whole lap starts at its point nearest the bit)
    let near = |cur: Pt, (_, p, closed): &(f64, Vec<Pt>, bool)| if *closed { p.iter().map(|q| dist(cur, *q)).fold(f64::MAX, f64::min) } else { dist(cur, p[0]) };
    while !laps.is_empty() {
        // the lap one step below this one when it is near; else the nearest of the highest laps left
        let below = (0..laps.len()).filter(|&k| !first && (laps[k].0 - (cur_z - dz)).abs() < dz / 2.0 && near(cur, &laps[k]) <= LINK).min_by(|&a, &b| near(cur, &laps[a]).total_cmp(&near(cur, &laps[b])));
        let k = below.unwrap_or_else(|| {
            let high = laps.iter().map(|l| l.0).fold(f64::MIN, f64::max);
            (0..laps.len()).filter(|&k| laps[k].0 >= high - dz / 2.0).min_by(|&a, &b| near(cur, &laps[a]).total_cmp(&near(cur, &laps[b]))).unwrap()
        });
        let (z, mut p, closed) = laps.swap_remove(k);
        if closed {
            let s = (0..p.len()).min_by(|&a, &b| dist(cur, p[a]).total_cmp(&dist(cur, p[b]))).unwrap();
            p.rotate_left(s);
            p.push(p[0]);
        }
        // a short way to the next lap is crossed just over the model (a hair above the highest the bit may be on the
        // way, never lower than where it is); a long way, or a rise on the way, means a lift to the travel height
        let d = dist(cur, p[0]);
        let over = (!first && d <= LINK).then(|| {
            let n = (d / (g / 2.0)).ceil().max(1.0) as usize;
            (0..=n).map(|k| k as f64 / n as f64).map(|f| pt(cur.x + (p[0].x - cur.x) * f, cur.y + (p[0].y - cur.y) * f)).map(|q| if inside(q.x, q.y) { tip_height(r, t, q.x, q.y, 0.0) } else { f64::MAX }).fold(cur_z.max(z), f64::max) + 0.003
        });
        match over.filter(|h| *h <= cur_z + 0.06) {
            Some(h) => {
                cut_to(tp, cur, Some(h));
                cut_to(tp, p[0], Some(h));
                cut_to(tp, p[0], Some(z));
            }
            None => {
                if !first {
                    rapid_to(tp, cur, Some(m.retract_z));
                }
                rapid_to(tp, p[0], Some(m.retract_z));
                cut_to(tp, p[0], Some(z));
            }
        }
        let p3: Vec<[f64; 3]> = p.iter().map(|q| [q.x, q.y, z]).collect();
        add_simplified(&mut tp.moves, &p3, 0.0005);
        (cur, cur_z, first) = (p[p.len() - 1], z, false);
    }
    if !first {
        rapid_to(tp, cur, Some(m.retract_z));
    }
}

/// The kept stretches of one contour line (a whole lap stays closed when every point is kept)
fn split(mut pts: Vec<Option<Pt>>, closed: bool, out: &mut Vec<(Vec<Pt>, bool)>) {
    if closed && pts.iter().all(|p| p.is_some()) {
        if pts.len() >= 3 {
            out.push((pts.into_iter().flatten().collect(), true));
        }
        return;
    }
    if closed {
        // start at a dropped point, so no kept stretch runs over the seam
        let s = pts.iter().position(|p| p.is_none()).unwrap_or(0);
        pts.rotate_left(s);
    }
    let mut run: Vec<Pt> = Vec::new();
    for p in pts.into_iter().chain(std::iter::once(None)) {
        if let Some(p) = p {
            run.push(p);
        } else if run.len() >= 2 {
            out.push((std::mem::take(&mut run), false));
        } else {
            run.clear();
        }
    }
}

/// Contour lines of a grid of values at `level` (marching squares), in grid units, each with whether it closes.
/// Lines only end at the grid's edge.
fn contours(v: &[f64], nx: usize, ny: usize, level: f64) -> Vec<(Vec<Pt>, bool)> {
    // a crossing is named by the grid edge it is on: point index × 2, + 1 for the edge going up instead of right
    let (bottom, left) = (|i: usize, j: usize| ((j * nx + i) * 2) as u64, |i: usize, j: usize| ((j * nx + i) * 2 + 1) as u64);
    let mut segs: Vec<[u64; 2]> = Vec::new();
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            let hi = |i: usize, j: usize| v[j * nx + i] > level;
            let case = hi(i, j) as u8 | (hi(i + 1, j) as u8) << 1 | (hi(i + 1, j + 1) as u8) << 2 | (hi(i, j + 1) as u8) << 3;
            let (b, r, t, l) = (bottom(i, j), left(i + 1, j), bottom(i, j + 1), left(i, j));
            match case {
                1 | 14 => segs.push([l, b]),
                2 | 13 => segs.push([b, r]),
                3 | 12 => segs.push([l, r]),
                4 | 11 => segs.push([r, t]),
                6 | 9 => segs.push([b, t]),
                7 | 8 => segs.push([l, t]),
                5 => segs.extend([[l, b], [r, t]]),
                10 => segs.extend([[b, r], [l, t]]),
                _ => {}
            }
        }
    }
    let mut at: HashMap<u64, [usize; 2]> = HashMap::with_capacity(segs.len());
    for (k, s) in segs.iter().enumerate() {
        for e in s {
            let slot = at.entry(*e).or_insert([usize::MAX; 2]);
            slot[(slot[0] != usize::MAX) as usize] = k;
        }
    }
    let point = |e: u64| {
        let (k, up) = ((e / 2) as usize, e % 2 == 1);
        let (i, j) = (k % nx, k / nx);
        let (a, b) = (v[k], if up { v[k + nx] } else { v[k + 1] });
        let f = if (b - a).abs() > 1e-12 { clampf((level - a) / (b - a), 0.0, 1.0) } else { 0.5 };
        if up { pt(i as f64, j as f64 + f) } else { pt(i as f64 + f, j as f64) }
    };
    let mut used = vec![false; segs.len()];
    // from edge `e` of segment `k`, on through the segments that follow until none does or the line closes
    let walk = |mut k: usize, mut e: u64, used: &mut Vec<bool>, line: &mut Vec<u64>| loop {
        let next = at[&e].into_iter().find(|&n| n != usize::MAX && n != k && !used[n]);
        let Some(n) = next else { return };
        used[n] = true;
        e = if segs[n][0] == e { segs[n][1] } else { segs[n][0] };
        line.push(e);
        k = n;
    };
    let mut out = Vec::new();
    for k in 0..segs.len() {
        if used[k] {
            continue;
        }
        used[k] = true;
        let mut line = vec![segs[k][0], segs[k][1]];
        walk(k, segs[k][1], &mut used, &mut line);
        let closed = line.len() > 3 && line[0] == line[line.len() - 1];
        if closed {
            line.pop();
        } else {
            line.reverse();
            walk(k, segs[k][0], &mut used, &mut line);
        }
        out.push((line.into_iter().map(point).collect(), closed));
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::cam::{Op, OpType, Tool, ToolType};
    use crate::document::{Document, Relief};
    use crate::job::MoveType;

    /// a round boss: flat top 1 in across, sides at 60°, flat floor 0.5 in down
    fn boss() -> Document {
        let (n, w, depth) = (300, 4.0, 0.5);
        let height: Vec<f32> = (0..n * n)
            .map(|k| {
                let (x, y) = (((k % n) as f64 + 0.5) * w / n as f64 - 2.0, ((k / n) as f64 + 0.5) * w / n as f64 - 2.0);
                (1.0 - ((x.hypot(y) - 0.5).max(0.0) * 60f64.to_radians().tan() / depth).min(1.0)) as f32
            })
            .collect();
        let mut d = Document::default();
        (d.material.w, d.material.h, d.material.t) = (6.0, 6.0, 0.75);
        d.reliefs.push(Relief { id: 1, x: 1.0, y: 1.0, w, h: w, depth, nx: n, ny: n, height, ..Default::default() });
        d.next_relief_id = 2;
        d.tools.clear();
        d.tools.push(Tool { kind: ToolType::BallNose, diameter: 0.25, stepdown: 0.1, feed_xy: 100.0, feed_z: 30.0, ..Default::default() });
        d.ops.push(Op { kind: OpType::Relief, name: "Sides".into(), relief: 1, tool: 0, stepover: 0.1, waterline: true, steep_angle: 30.0, ..Default::default() });
        d
    }

    #[test]
    fn waterline_laps_the_steep_sides_at_even_heights_and_touches_the_model() {
        let d = boss();
        let g = crate::pipeline::regenerate(&d);
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);
        let (r, t) = (&d.reliefs[0], &d.tools[0]);
        let (mut pos, mut laps, mut cuts) = ([0.0, 0.0, 1.0], std::collections::BTreeSet::new(), 0);
        for mv in g.paths[0].iter().flat_map(|p| &p.moves) {
            let to = [mv.x.unwrap_or(pos[0]), mv.y.unwrap_or(pos[1]), mv.z.unwrap_or(pos[2])];
            // a lap runs at one of the heights (every 0.025 in); the short crossings between laps run a hair higher
            let on_a_lap = ((to[2] / 0.025).round() * 0.025 - to[2]).abs() < 1e-6;
            if mv.kind == MoveType::Linear && to[2] < 0.0 {
                let lowest = crate::relief::tip_height(r, t, to[0], to[1], 0.0);
                assert!(lowest <= to[2] + 2e-4, "at {to:?} the bit may only go to {lowest}");
            }
            if mv.kind == MoveType::Linear && to[2] < 0.0 && pos[2] == to[2] && on_a_lap {
                cuts += 1;
                laps.insert((to[2] * 1e6).round() as i64);
                // on the model: the bit touches it without going in, and only on the slope or down the wall at the
                // model's edge (the board stands uncut round it), not on the top or the floor
                let lowest = crate::relief::tip_height(r, t, to[0], to[1], 0.0);
                assert!(lowest <= to[2] + 2e-4, "at {to:?} the bit may only go to {lowest}");
                let from_centre = (to[0] - 3.0).hypot(to[1] - 3.0);
                let from_wall = 2.0 - (to[0] - 3.0).abs().max((to[1] - 3.0).abs());
                // on the slope the bit rests on it; beside the upright wall it stands clear of the floor
                let on_slope = from_centre > 0.45 && from_centre < 1.0 && lowest > to[2] - 0.012;
                assert!(on_slope || (from_wall > 0.0 && from_wall < 0.14), "a lap {from_centre} in from the centre, {from_wall} in from the wall, lowest {lowest} at {to:?}");
            }
            assert!(mv.kind != MoveType::Rapid || to[2] >= 0.0, "a rapid below the surface");
            pos = to;
        }
        // one lap every 0.025 in down the 0.5 in side (the lowest ones would sit on the floor: left out)
        assert!(cuts > 200 && (15..=19).contains(&laps.len()), "{cuts} moves on {} laps", laps.len());
        let zs: Vec<i64> = laps.into_iter().collect();
        assert!(zs.windows(2).all(|w| w[1] - w[0] == 25_000), "{zs:?}");
    }

    #[test]
    fn without_a_steepness_limit_every_lap_is_whole() {
        let mut d = boss();
        d.ops[0].steep_angle = 0.0;
        let g = crate::pipeline::regenerate(&d);
        // the settings come back from a saved project
        d.ops[0].steep_angle = 40.0;
        let back = crate::project::load_project(&crate::project::save_project(&d)).unwrap();
        assert!(back.ops[0].waterline && back.ops[0].steep_angle == 40.0);
        // a whole lap ends where it began
        let moves: Vec<_> = g.paths[0].iter().flat_map(|p| &p.moves).collect();
        let lifts = moves.windows(2).filter(|w| w[0].kind == MoveType::Linear && w[1].kind == MoveType::Rapid).count();
        // laps down the 60° slope are 0.014 in apart: the bit steps from one to the next without lifting; the upright
        // wall at the model's edge is lapped the same way. So far fewer lifts than the 38 laps
        assert!((2..=6).contains(&lifts), "{lifts} lifts for 38 laps");
        let laps = moves.iter().filter_map(|m| m.z).filter(|z| *z < 0.0 && ((z / 0.025).round() * 0.025 - z).abs() < 1e-6).map(|z| (z * 1e6).round() as i64).collect::<std::collections::BTreeSet<_>>().len();
        assert_eq!(laps, 19);
    }
}
