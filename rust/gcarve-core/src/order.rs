//! Toolpath travel order and machine-time estimate (C++ `core/order.*`). A toolpath is a sequence of "segments": each
//! starts with a rapid travel and ends when the cutter lifts. Segments whose footprints overlap (depth passes of one
//! shape) stay in order as a block; blocks are reordered to shorten the rapid travel (nearest neighbour + or-opt).

use crate::geom::{dist, pt, Pt, Rect};
use crate::job::{Move, MoveType, Toolpath};

/// Move index range `[first, last]`.
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub first: usize,
    pub last: usize,
    pub start: Pt,
    pub end: Pt,
    pub bbox: Rect,
}

pub fn split_segments(tp: &Toolpath) -> Vec<Segment> {
    let mut segs: Vec<Segment> = Vec::new();
    let mut cur = pt(0.0, 0.0);
    let mut have = false;
    for (i, mv) in tp.moves.iter().enumerate() {
        let p = pt(mv.x.unwrap_or(cur.x), mv.y.unwrap_or(cur.y));
        let travel = mv.kind == MoveType::Rapid && (!have || dist(p, cur) > 1e-9);
        if travel || segs.is_empty() {
            let mut bbox = Rect::default();
            bbox.add(p);
            segs.push(Segment { first: i, last: i, start: p, end: p, bbox });
        }
        let s = segs.last_mut().unwrap();
        s.last = i;
        s.end = p;
        s.bbox.add(p);
        cur = p;
        have = true;
    }
    segs
}

/// Visit order for blocks with fixed direction: nearest neighbour from home, then single-block relocations.
pub fn order_blocks(starts: &[Pt], ends: &[Pt], home: Pt) -> Vec<usize> {
    let n = starts.len();
    let mut order = Vec::with_capacity(n);
    let mut used = vec![false; n];
    let mut cur = home;
    for _ in 0..n {
        let (mut best, mut bd) = (0, 1e300);
        for i in 0..n {
            if !used[i] {
                let d = dist(cur, starts[i]);
                if d < bd {
                    bd = d;
                    best = i;
                }
            }
        }
        used[best] = true;
        order.push(best);
        cur = ends[best];
    }
    if n < 3 {
        return order;
    }
    // or-opt with O(1) move deltas: relocate single blocks while it shortens the travel (blocks keep their direction)
    let d = |a: usize, b: usize| dist(ends[a], starts[b]);
    let d_home = |b: usize| dist(home, starts[b]);
    for _ in 0..20 {
        let mut improved = false;
        let mut i = 0;
        while i < order.len() && !improved {
            let blk = order[i];
            let has_next = i + 1 < order.len();
            let before = (if i == 0 { d_home(blk) } else { d(order[i - 1], blk) }) + if has_next { d(blk, order[i + 1]) } else { 0.0 };
            let bridge = if !has_next {
                0.0
            } else if i == 0 {
                d_home(order[i + 1])
            } else {
                d(order[i - 1], order[i + 1])
            };
            let remove_gain = before - bridge;
            let mut o = order.clone();
            o.remove(i);
            let (mut best_delta, mut best_j) = (-1e-9, usize::MAX);
            for j in 0..=o.len() {
                let inn = (if j == 0 { d_home(blk) } else { d(o[j - 1], blk) }) + if j < o.len() { d(blk, o[j]) } else { 0.0 };
                let out = if j >= o.len() {
                    0.0
                } else if j == 0 {
                    d_home(o[j])
                } else {
                    d(o[j - 1], o[j])
                };
                let delta = (inn - out) - remove_gain; // < 0 = shorter tour
                if delta < best_delta {
                    best_delta = delta;
                    best_j = j;
                }
            }
            if best_j != usize::MAX {
                o.insert(best_j, blk);
                order = o;
                improved = true;
            }
            i += 1;
        }
        if !improved {
            break;
        }
    }
    order
}

/// In place; keeps each block's internal order.
pub fn optimize_order(tp: &mut Toolpath, home: Pt) {
    let segs = split_segments(tp);
    if segs.len() < 3 {
        return;
    }
    // blocks: consecutive segments with overlapping footprints keep their order (depth passes, inside-out pockets)
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    let (mut starts, mut ends) = (Vec::new(), Vec::new());
    for (i, s) in segs.iter().enumerate() {
        if let Some(b) = blocks.last_mut() {
            if s.bbox.intersects(&segs[b.1].bbox) {
                b.1 = i;
                *ends.last_mut().unwrap() = s.end;
                continue;
            }
        }
        blocks.push((i, i));
        starts.push(s.start);
        ends.push(s.end);
    }
    if blocks.len() < 3 {
        return;
    }
    let mut out: Vec<Move> = Vec::with_capacity(tp.moves.len());
    for bi in order_blocks(&starts, &ends, home) {
        for s in &segs[blocks[bi].0..=blocks[bi].1] {
            out.extend_from_slice(&tp.moves[s.first..=s.last]);
        }
    }
    tp.moves = out;
}

/// Reorders whole toolpaths (one per shape) that share the same `tool`; paths of other tools keep their slots.
pub fn order_toolpaths(paths: &mut [Toolpath], home: Pt) {
    let mut tools: Vec<i32> = Vec::new();
    for tp in paths.iter() {
        if !tools.contains(&tp.tool) {
            tools.push(tp.tool);
        }
    }
    for t in tools {
        let (mut idx, mut starts, mut ends) = (Vec::new(), Vec::new(), Vec::new());
        for (i, p) in paths.iter().enumerate() {
            if p.tool != t || p.moves.is_empty() {
                continue;
            }
            let (f, l) = (p.moves[0], p.moves[p.moves.len() - 1]);
            idx.push(i);
            starts.push(pt(f.x.unwrap_or(0.0), f.y.unwrap_or(0.0)));
            ends.push(pt(l.x.unwrap_or(0.0), l.y.unwrap_or(0.0)));
        }
        if idx.len() < 2 {
            continue;
        }
        let ord = order_blocks(&starts, &ends, home);
        let picked: Vec<Toolpath> = ord.iter().map(|&o| std::mem::take(&mut paths[idx[o]])).collect();
        for (k, tp) in picked.into_iter().enumerate() {
            paths[idx[k]] = tp; // same slots, new order
        }
    }
}

/// How a machine moves, for time estimates: rapid speed (in/min) and acceleration (in/s²; 0 = instant, the C++
/// estimate, kept for the setup sheet golden and the playback timeline).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub rapid_ipm: f64,
    pub accel: f64,
}

impl Default for Motion {
    fn default() -> Self {
        Motion { rapid_ipm: 400.0, accel: 0.0 }
    }
}

/// Minutes at the given feeds; rapids at `rapid_ipm` (C++ default 400). Z-only linear moves use `feed_z`.
pub fn estimate_minutes(tp: &Toolpath, feed_xy: f64, feed_z: f64, rapid_ipm: f64) -> f64 {
    estimate_minutes_with(tp, feed_xy, feed_z, Motion { rapid_ipm, accel: 0.0 })
}

/// Seconds for `len` inches entered at `v0`, left at `v1`, cruising at most at `f` (in/s), accelerating at `a`:
/// a trapezoid, or a triangle when the move is too short to reach `f`.
fn move_seconds(len: f64, v0: f64, v1: f64, f: f64, a: f64) -> f64 {
    let (da, dd) = ((f * f - v0 * v0) / (2.0 * a), (f * f - v1 * v1) / (2.0 * a));
    if da + dd <= len {
        return (f - v0) / a + (f - v1) / a + (len - da - dd) / f;
    }
    let peak = ((2.0 * a * len + v0 * v0 + v1 * v1) / 2.0).sqrt();
    if peak < v0.max(v1) {
        return 2.0 * len / (v0 + v1).max(1e-9); // can't change speed that much in this distance: average of the two
    }
    (peak - v0) / a + (peak - v1) / a
}

/// `estimate_minutes` with acceleration: every move speeds up from and slows down to the speed it can keep through
/// the corner at either end ((1 + cos turn) / 2 of the slower move: straight on = full speed, square = half, a
/// reversal = stop). Detailed carving (V-carve, photo, short moves) takes much longer than length / feed.
/// ponytail: no look-ahead past the next corner and no jerk; real controllers differ, the model only has to be close.
pub fn estimate_minutes_with(tp: &Toolpath, feed_xy: f64, feed_z: f64, m: Motion) -> f64 {
    move_minutes(tp, feed_xy, feed_z, m).iter().sum() // accel 0: the C++ sum, same order of additions
}

/// Minutes of every move of `tp` (same index as `tp.moves`; the first move, from nowhere, is 0). One model for the
/// estimates and the simulation's clock.
pub fn move_minutes(tp: &Toolpath, feed_xy: f64, feed_z: f64, m: Motion) -> Vec<f64> {
    // (move index, direction, length, cruise speed in/s) of every move that goes somewhere
    let mut out = vec![0.0; tp.moves.len()];
    let mut segs: Vec<(usize, [f64; 3], f64, f64)> = Vec::with_capacity(tp.moves.len());
    let (mut cur, mut z, mut have) = (pt(0.0, 0.0), 0.0, false);
    for (i, mv) in tp.moves.iter().enumerate() {
        let p = pt(mv.x.unwrap_or(cur.x), mv.y.unwrap_or(cur.y));
        let nz = mv.z.unwrap_or(z);
        if have {
            let dxy = dist(cur, p);
            let l = dxy.hypot((nz - z).abs());
            let f = if mv.kind == MoveType::Rapid {
                m.rapid_ipm
            } else if dxy < 1e-9 {
                feed_z
            } else {
                feed_xy
            };
            if f > 0.0 {
                out[i] = l / f;
                if l > 1e-9 {
                    segs.push((i, [(p.x - cur.x) / l, (p.y - cur.y) / l, (nz - z) / l], l, f / 60.0));
                }
            }
        }
        cur = p;
        z = nz;
        have = true;
    }
    if m.accel <= 0.0 {
        return out;
    }
    let junction = |a: &(usize, [f64; 3], f64, f64), b: &(usize, [f64; 3], f64, f64)| {
        let cos = a.1[0] * b.1[0] + a.1[1] * b.1[1] + a.1[2] * b.1[2];
        a.3.min(b.3) * ((1.0 + cos) / 2.0).max(0.0)
    };
    for k in 0..segs.len() {
        let v0 = if k > 0 { junction(&segs[k - 1], &segs[k]) } else { 0.0 };
        let v1 = if k + 1 < segs.len() { junction(&segs[k], &segs[k + 1]) } else { 0.0 };
        out[segs[k].0] = move_seconds(segs[k].2, v0, v1, segs[k].3, m.accel) / 60.0;
    }
    out
}
