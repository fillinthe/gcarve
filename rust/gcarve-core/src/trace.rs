//! Bitmap tracing (VCarve "Trace Bitmap"): a grayscale image → closed vector outlines (outer shapes and their holes).
//! Threshold (Otsu's automatic one by default) → marching squares on the pixel corners → loops joined → simplified
//! (Douglas-Peucker) → specks dropped → scaled to inches with Y up. No I/O: the app decodes the file.

use crate::geom::{pt, Contour, Node, Pt};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct TraceOptions {
    /// 0..255: pixels darker than this are "ink"; None = Otsu's automatic threshold
    pub threshold: Option<u8>,
    /// trace the light parts instead
    pub invert: bool,
    /// Douglas-Peucker tolerance in pixels (0.5 = tight, 2 = smooth and few nodes)
    pub smooth_px: f64,
    /// outlines enclosing fewer pixels than this are dropped (dust, JPEG noise)
    pub min_area_px: f64,
    /// width of the traced image on the board, inches (height follows the aspect)
    pub width_in: f64,
    /// single strokes along the middle of the ink (signatures, handwriting, line art to engrave) instead of outlines
    pub centreline: bool,
}

impl Default for TraceOptions {
    fn default() -> Self {
        TraceOptions { threshold: None, invert: false, smooth_px: 1.0, min_area_px: 16.0, width_in: 6.0, centreline: false }
    }
}

/// Otsu's threshold: the gray level that best splits the histogram into two classes.
pub fn otsu(gray: &[u8]) -> u8 {
    let mut hist = [0u64; 256];
    for &g in gray {
        hist[g as usize] += 1;
    }
    let total = gray.len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, &n)| i as f64 * n as f64).sum();
    let (mut sum_b, mut w_b, mut best, mut best_t) = (0.0, 0.0, -1.0, 128u8);
    for (t, &n) in hist.iter().enumerate() {
        w_b += n as f64;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += t as f64 * n as f64;
        let (m_b, m_f) = (sum_b / w_b, (sum - sum_b) / w_f);
        let between = w_b * w_f * (m_b - m_f).powi(2);
        if between > best {
            best = between;
            best_t = t as u8;
        }
    }
    best_t.saturating_add(1) // "darker than": the threshold level itself belongs to the dark class
}

fn area(p: &[Pt]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i].x * p[(i + 1) % n].y - p[(i + 1) % n].x * p[i].y).sum::<f64>() / 2.0
}

fn seg_dist(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
}

/// Douglas-Peucker on an open polyline (keeps both ends).
pub(crate) fn dp(p: &[Pt], tol: f64, out: &mut Vec<Pt>) {
    let mut keep = vec![false; p.len()];
    keep[0] = true;
    keep[p.len() - 1] = true;
    let mut stack = vec![(0, p.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (mut far, mut d) = (0, 0.0);
        for i in a + 1..b {
            let di = seg_dist(p[i], p[a], p[b]);
            if di > d {
                (far, d) = (i, di);
            }
        }
        if d > tol {
            keep[far] = true;
            stack.push((a, far));
            stack.push((far, b));
        }
    }
    out.extend(p.iter().zip(keep).filter(|(_, k)| *k).map(|(q, _)| *q));
}

/// Closed loop: split at the point farthest from the first so both halves have real ends.
pub(crate) fn simplify_closed(p: &[Pt], tol: f64) -> Vec<Pt> {
    if p.len() < 4 || tol <= 0.0 {
        return p.to_vec();
    }
    let far = (1..p.len()).max_by(|&a, &b| (p[a].x - p[0].x).hypot(p[a].y - p[0].y).total_cmp(&(p[b].x - p[0].x).hypot(p[b].y - p[0].y))).unwrap();
    let mut out = Vec::new();
    dp(&p[..=far], tol, &mut out);
    out.pop();
    let mut tail: Vec<Pt> = p[far..].to_vec();
    tail.push(p[0]);
    dp(&tail, tol, &mut out);
    out.pop();
    out
}

/// Traces `gray` (row-major, `w` x `h`, 0 = black). Returns closed outlines in inches, the image's bottom-left at
/// (0, 0). Holes come out as their own loops (pocket / V-carve treat nested loops as islands).
pub fn trace(gray: &[u8], w: usize, h: usize, o: &TraceOptions) -> Vec<Contour> {
    if w == 0 || h == 0 || gray.len() < w * h {
        return Vec::new();
    }
    let t = o.threshold.unwrap_or_else(|| otsu(gray));
    if o.centreline {
        let ink: Vec<bool> = gray[..w * h].iter().map(|&g| (g < t) != o.invert).collect();
        return centrelines(ink, w, h, o);
    }
    let ink = |x: i64, y: i64| -> bool {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            return false; // a blank border closes every outline
        }
        (gray[y as usize * w + x as usize] < t) != o.invert
    };
    // marching squares over pixel corners; edge points on cell sides, keyed in half-pixel units
    // cell (x, y) covers pixel centres (x-1..x, y-1..y); corners tl tr br bl
    let mut next: HashMap<(i64, i64), (i64, i64)> = HashMap::new();
    let mut add = |a: (i64, i64), b: (i64, i64)| {
        next.insert(a, b);
    };
    for y in 0..=h as i64 {
        for x in 0..=w as i64 {
            let (tl, tr, br, bl) = (ink(x - 1, y - 1), ink(x, y - 1), ink(x, y), ink(x - 1, y));
            let code = (tl as u8) << 3 | (tr as u8) << 2 | (br as u8) << 1 | bl as u8;
            // side midpoints (doubled coordinates): top, right, bottom, left
            let (top, right, bottom, left) = ((2 * x - 1, 2 * y - 2), (2 * x, 2 * y - 1), (2 * x - 1, 2 * y), (2 * x - 2, 2 * y - 1));
            // ink kept on the left of every directed segment (image coordinates, y down)
            match code {
                0 | 15 => {}
                1 => add(bottom, left),
                2 => add(right, bottom),
                3 => add(right, left),
                4 => add(top, right),
                5 => {
                    add(top, right); // saddle: the two inks stay apart (as cases 4 + 1)
                    add(bottom, left);
                }
                6 => add(top, bottom),
                7 => add(top, left),
                8 => add(left, top),
                9 => add(bottom, top),
                10 => {
                    add(left, top);
                    add(right, bottom);
                }
                11 => add(right, top),
                12 => add(left, right),
                13 => add(bottom, right),
                14 => add(left, bottom),
                _ => unreachable!(),
            }
        }
    }
    let scale = o.width_in / w as f64;
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
    let mut starts: Vec<(i64, i64)> = next.keys().copied().collect();
    starts.sort_unstable();
    for s in starts {
        if seen.contains(&s) {
            continue;
        }
        let mut loop_px = Vec::new();
        let mut cur = s;
        while seen.insert(cur) {
            loop_px.push(pt(cur.0 as f64 / 2.0 + 0.5, cur.1 as f64 / 2.0 + 0.5));
            match next.get(&cur) {
                Some(&n) => cur = n,
                None => break,
            }
        }
        if loop_px.len() < 3 || area(&loop_px).abs() < o.min_area_px {
            continue;
        }
        let simple = simplify_closed(&loop_px, o.smooth_px);
        if simple.len() < 3 {
            continue;
        }
        let nodes = simple.iter().map(|p| Node::at(pt(p.x * scale, (h as f64 - p.y) * scale))).collect();
        out.push(Contour { nodes, closed: true });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::bounds;

    /// A 40x30 image: a black 20x10 rectangle with a 6x4 white hole, plus one stray black pixel.
    fn picture() -> (Vec<u8>, usize, usize) {
        let (w, h) = (40, 30);
        let mut g = vec![255u8; w * h];
        for y in 10..20 {
            for x in 10..30 {
                g[y * w + x] = 0;
            }
        }
        for y in 13..17 {
            for x in 17..23 {
                g[y * w + x] = 255;
            }
        }
        g[2 * w + 2] = 0;
        (g, w, h)
    }

    #[test]
    fn otsu_splits_black_and_white() {
        let (g, _, _) = picture();
        let t = otsu(&g);
        assert!(t > 0 && t < 255, "{t}");
    }

    #[test]
    fn traces_outline_and_hole_drops_speck() {
        let (g, w, h) = picture();
        let o = TraceOptions { width_in: 4.0, ..Default::default() }; // 0.1 in per pixel
        let cs = trace(&g, w, h, &o);
        assert_eq!(cs.len(), 2, "outer + hole, speck dropped: {cs:?}");
        let mut b: Vec<_> = cs.iter().map(bounds).collect();
        b.sort_by(|a, c| c.w().total_cmp(&a.w()));
        // outer: pixels 10..30 x 10..20 → 2.0 x 1.0 in, y flipped: rows 10..20 of 30 → y 1.0..2.0
        assert!((b[0].w() - 2.0).abs() < 0.11 && (b[0].h() - 1.0).abs() < 0.11, "{:?}", b[0]);
        assert!((b[0].y0 - 1.0).abs() < 0.06 && (b[0].x0 - 1.0).abs() < 0.06, "{:?}", b[0]);
        assert!((b[1].w() - 0.6).abs() < 0.11 && (b[1].h() - 0.4).abs() < 0.11, "{:?}", b[1]);
        // simplified: a rectangle needs only its corners (marching squares cuts them, so a few more)
        assert!(cs.iter().all(|c| c.nodes.len() <= 8), "{:?}", cs.iter().map(|c| c.nodes.len()).collect::<Vec<_>>());
        // invert: the white background with the rectangle as a hole, plus the white hole as its own shape
        let inv = trace(&g, w, h, &TraceOptions { invert: true, ..o });
        assert!(inv.len() >= 2);
    }
}

/// Centreline trace: Zhang-Suen thinning (1984) down to a one-pixel skeleton, then each run of skeleton pixels
/// between ends and junctions becomes an open line (rings with neither become closed ones). Spurs (a junction to an
/// end, shorter than about the stroke's width) are thinning noise and dropped, as are lines shorter than
/// `min_area_px`'s side. Own implementation.
fn centrelines(mut ink: Vec<bool>, w: usize, h: usize, o: &TraceOptions) -> Vec<Contour> {
    let at = |ink: &[bool], x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && ink[y as usize * w + x as usize];
    // P2..P9 clockwise from north
    const NB: [(i64, i64); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];
    loop {
        let mut changed = false;
        for pass in 0..2 {
            let mut del = Vec::new();
            for y in 0..h as i64 {
                for x in 0..w as i64 {
                    if !at(&ink, x, y) {
                        continue;
                    }
                    let p: Vec<bool> = NB.iter().map(|(dx, dy)| at(&ink, x + dx, y + dy)).collect();
                    let b = p.iter().filter(|v| **v).count();
                    let a = (0..8).filter(|&i| !p[i] && p[(i + 1) % 8]).count();
                    let (c1, c2) = if pass == 0 { (p[0] && p[2] && p[4], p[2] && p[4] && p[6]) } else { (p[0] && p[2] && p[6], p[0] && p[4] && p[6]) };
                    if (2..=6).contains(&b) && a == 1 && !c1 && !c2 {
                        del.push(y as usize * w + x as usize);
                    }
                }
            }
            changed |= !del.is_empty();
            for i in del {
                ink[i] = false;
            }
        }
        if !changed {
            break;
        }
    }
    let nbrs = |i: usize| -> Vec<usize> {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        // a diagonal step that a side neighbour already links (staircases left by the thinning) is not a branch
        NB.iter()
            .filter(|(dx, dy)| at(&ink, x + dx, y + dy) && (*dx == 0 || *dy == 0 || !(at(&ink, x + dx, y) || at(&ink, x, y + dy))))
            .map(|(dx, dy)| ((y + dy) as usize) * w + (x + dx) as usize)
            .collect()
    };
    let node = |i: usize| nbrs(i).len() != 2; // an end or a junction
    let mut used: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut chains: Vec<(Vec<usize>, bool)> = Vec::new(); // pixels, closed
    let walk = |start: usize, first: usize, used: &mut std::collections::HashSet<(usize, usize)>| -> (Vec<usize>, bool) {
        let mut path = vec![start, first];
        used.insert((start.min(first), start.max(first)));
        let (mut prev, mut cur) = (start, first);
        while !node(cur) && cur != start {
            let Some(&nx) = nbrs(cur).iter().find(|&&n| n != prev && !used.contains(&(cur.min(n), cur.max(n)))) else { break };
            used.insert((cur.min(nx), cur.max(nx)));
            path.push(nx);
            (prev, cur) = (cur, nx);
        }
        let closed = cur == start && path.len() > 3;
        if closed {
            path.pop();
        }
        (path, closed)
    };
    let px: Vec<usize> = (0..w * h).filter(|&i| ink[i]).collect();
    for &i in px.iter().filter(|&&i| node(i)) {
        for n in nbrs(i) {
            if !used.contains(&(i.min(n), i.max(n))) {
                chains.push(walk(i, n, &mut used));
            }
        }
    }
    for &i in &px {
        if let Some(&n) = nbrs(i).iter().find(|&&n| !used.contains(&(i.min(n), i.max(n)))) {
            chains.push(walk(i, n, &mut used)); // rings: no end or junction on them
        }
    }
    let scale = o.width_in / w as f64;
    let min_len = o.min_area_px.sqrt().max(3.0);
    let ends = |c: &[usize]| [c[0], c[c.len() - 1]].iter().filter(|&&i| nbrs(i).len() == 1).count();
    chains
        .into_iter()
        .filter(|(c, closed)| {
            let len = c.windows(2).map(|p| ((p[0] % w) as f64 - (p[1] % w) as f64).hypot((p[0] / w) as f64 - (p[1] / w) as f64)).sum::<f64>();
            // a spur (one free end on a junction) or a speck shorter than the minimum goes
            *closed || !(len < min_len || (ends(c) == 1 && len < 2.0 * min_len))
        })
        .map(|(c, closed)| {
            let pts: Vec<Pt> = c.iter().map(|&i| pt((i % w) as f64 + 0.5, (i / w) as f64 + 0.5)).collect();
            let simple = if closed { simplify_closed(&pts, o.smooth_px) } else { let mut out = Vec::new(); dp(&pts, o.smooth_px, &mut out); out };
            Contour { nodes: simple.iter().map(|p| Node::at(pt(p.x * scale, (h as f64 - p.y) * scale))).collect(), closed }
        })
        .collect()
}

#[cfg(test)]
mod centreline_tests {
    use super::*;

    #[test]
    fn a_thick_stroke_traces_as_one_line_down_its_middle_and_a_ring_stays_closed() {
        // a 9 px thick horizontal bar from x 20 to 80 at y 30, and a ring of radius 15 around (50, 70)
        let (w, h) = (100, 100);
        let mut g = vec![255u8; w * h];
        for y in 26..35 {
            for x in 20..80 {
                g[y * w + x] = 0;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let r = ((x as f64 - 50.0).powi(2) + (y as f64 - 70.0).powi(2)).sqrt();
                if (r - 15.0).abs() < 2.5 {
                    g[y * w + x] = 0;
                }
            }
        }
        let o = TraceOptions { centreline: true, width_in: 10.0, ..Default::default() };
        let lines = trace(&g, w, h, &o);
        let open: Vec<_> = lines.iter().filter(|c| !c.closed).collect();
        let rings: Vec<_> = lines.iter().filter(|c| c.closed).collect();
        assert_eq!((open.len(), rings.len()), (1, 1), "{lines:?}");
        // the bar's line: along its middle (y 30.5 px from the top = 6.95 in up), most of its length
        let b = crate::geom::bounds(open[0]);
        assert!((b.y0 - 6.95).abs() < 0.15 && (b.y1 - 6.95).abs() < 0.15, "{b:?}");
        assert!(b.x1 - b.x0 > 4.5, "thinning takes about half the stroke off each end: {b:?}");
        let r = crate::geom::bounds(rings[0]);
        assert!(((r.x1 - r.x0) - 3.0).abs() < 0.3, "ring of diameter 30 px = 3 in: {r:?}");
    }
}
