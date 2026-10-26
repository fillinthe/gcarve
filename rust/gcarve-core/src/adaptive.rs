//! Adaptive clearing (constant-load pocketing, like Fusion's "2D Adaptive Clearing"): the bit's sideways load stays at a
//! set value everywhere, instead of the spikes an offset-ring pocket makes in corners and narrow necks. That lets the
//! bit cut deep at speed and live longer.
//!
//! How (our own design; the idea is the general one used by every adaptive tool): the material still to be cut is a
//! fine grid. Every step the tool looks at a few headings and takes the one where the bit would remove the target
//! amount of material (a chosen sliver width), steering along the edge of what is cut, then the grid is updated. When
//! nothing is left in reach it finds the nearest uncut material and links to it (staying down where the way is already
//! clear, else lifting). Climb milling = the uncut material stays on the right of the travel direction.
//!
//! Pure geometry (no clipper, no I/O); `cam::pocket_adaptive` turns the plan into moves per depth pass.

use crate::geom::{dist, pt, Pt};

pub struct Params {
    /// tool radius
    pub radius: f64,
    /// radial engagement the bit holds: the width of material it removes along the way (inches)
    pub engage: f64,
    /// climb milling: uncut material on the right of travel
    pub climb: bool,
    /// give up (Err) when the path would need more steps than this
    pub max_steps: usize,
}

/// One continuous cut. `down`: the move from the previous run's end to this run's start can stay at depth (the way is
/// already cleared); otherwise lift, move and plunge.
pub struct Run {
    pub pts: Vec<Pt>,
    pub down: bool,
}

/// One connected part of the pocket: where to go in (a helix of radius `helix` about `entry`, or a plain plunge when
/// 0), then the runs. The first run starts at `entry + (helix, 0)` (at `entry` for a plunge).
pub struct Comp {
    pub entry: Pt,
    pub helix: f64,
    pub runs: Vec<Run>,
}

pub struct Plan {
    pub comps: Vec<Comp>,
    pub steps: usize,
    pub cell: f64,
}

struct Grid {
    x0: f64,
    y0: f64,
    h: f64,
    nx: usize,
    ny: usize,
    /// 1 = material still to cut
    mat: Vec<u8>,
    /// 1 = the bit's centre may be here
    ok: Vec<u8>,
    /// material cells left per 16 × 16 block (nearest-material search)
    blocks: Vec<u32>,
    bnx: usize,
    r: f64,
}

const B: usize = 16;

/// Scanline fill of closed polygons (even-odd) into a 0 / 1 raster; cell centres at (i + 0.5) · h.
fn fill(polys: &[Vec<Pt>], x0: f64, y0: f64, h: f64, nx: usize, ny: usize) -> Vec<u8> {
    let mut xs: Vec<Vec<f64>> = vec![Vec::new(); ny];
    for p in polys {
        for i in 0..p.len() {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            if (a.y - b.y).abs() < 1e-12 {
                continue;
            }
            let (lo, hi) = if a.y < b.y { (a, b) } else { (b, a) };
            // rows whose centre y lies in [lo.y, hi.y)
            let j0 = (((lo.y - y0) / h - 0.5).ceil().max(0.0)) as usize;
            let j1 = ((((hi.y - y0) / h - 0.5).ceil()) as i64 - 1).min(ny as i64 - 1);
            if j1 < 0 {
                continue;
            }
            for (j, row) in xs.iter_mut().enumerate().take(j1 as usize + 1).skip(j0) {
                let y = y0 + (j as f64 + 0.5) * h;
                row.push(lo.x + (hi.x - lo.x) * (y - lo.y) / (hi.y - lo.y));
            }
        }
    }
    let mut out = vec![0u8; nx * ny];
    for (j, row) in xs.iter_mut().enumerate() {
        row.sort_by(f64::total_cmp);
        for c in row.as_chunks::<2>().0 {
            let i0 = (((c[0] - x0) / h - 0.5).ceil().max(0.0)) as usize;
            let i1 = (((c[1] - x0) / h - 0.5).ceil() as i64 - 1).min(nx as i64 - 1);
            if i1 >= i0 as i64 {
                out[j * nx + i0..=j * nx + i1 as usize].fill(1);
            }
        }
    }
    out
}

impl Grid {
    fn cell(&self, p: Pt) -> Option<(usize, usize)> {
        let (fx, fy) = ((p.x - self.x0) / self.h, (p.y - self.y0) / self.h);
        (fx >= 0.0 && fy >= 0.0 && (fx as usize) < self.nx && (fy as usize) < self.ny).then_some((fx as usize, fy as usize))
    }
    fn centre(&self, ix: usize, iy: usize) -> Pt {
        pt(self.x0 + (ix as f64 + 0.5) * self.h, self.y0 + (iy as f64 + 0.5) * self.h)
    }
    /// the part (label) the cell under `p` belongs to; 0 outside every part
    fn label_at(&self, label: &[u32], p: Pt) -> u32 {
        self.cell(p).map_or(0, |(i, j)| label[j * self.nx + i])
    }
    fn ok_at(&self, p: Pt) -> bool {
        self.cell(p).is_some_and(|(i, j)| self.ok[j * self.nx + i] == 1)
    }
    /// the cell index range a disc of radius `rad` at `p` can touch
    fn span(&self, p: Pt, rad: f64) -> (usize, usize, usize, usize) {
        let lo = |v: f64, n: usize| (v.floor().max(0.0) as usize).min(n.saturating_sub(1));
        let hi = |v: f64, n: usize| (v.ceil().max(0.0) as usize).min(n.saturating_sub(1));
        (lo((p.x - rad - self.x0) / self.h - 0.5, self.nx), hi((p.x + rad - self.x0) / self.h - 0.5, self.nx), lo((p.y - rad - self.y0) / self.h - 0.5, self.ny), hi((p.y + rad - self.y0) / self.h - 0.5, self.ny))
    }
    /// material cells inside the bit's disc at `p`: what a cut there would remove
    fn engagement(&self, p: Pt) -> u32 {
        let (i0, i1, j0, j1) = self.span(p, self.r);
        let r2 = self.r * self.r;
        let mut n = 0;
        for j in j0..=j1 {
            let dy = self.y0 + (j as f64 + 0.5) * self.h - p.y;
            let dy2 = dy * dy;
            if dy2 > r2 {
                continue;
            }
            let row = &self.mat[j * self.nx..(j + 1) * self.nx];
            for (i, &m) in row.iter().enumerate().take(i1 + 1).skip(i0) {
                if m == 1 {
                    let dx = self.x0 + (i as f64 + 0.5) * self.h - p.x;
                    if dx * dx + dy2 <= r2 {
                        n += 1;
                    }
                }
            }
        }
        n
    }
    /// cut: remove the material in a disc of radius `rad`
    fn clear(&mut self, p: Pt, rad: f64) {
        let (i0, i1, j0, j1) = self.span(p, rad);
        let r2 = rad * rad;
        for j in j0..=j1 {
            let dy = self.y0 + (j as f64 + 0.5) * self.h - p.y;
            let dy2 = dy * dy;
            if dy2 > r2 {
                continue;
            }
            for i in i0..=i1 {
                if self.mat[j * self.nx + i] == 1 {
                    let dx = self.x0 + (i as f64 + 0.5) * self.h - p.x;
                    if dx * dx + dy2 <= r2 {
                        self.mat[j * self.nx + i] = 0;
                        self.blocks[(j / B) * self.bnx + i / B] -= 1;
                    }
                }
            }
        }
    }
    fn drop_cell(&mut self, i: usize, j: usize) {
        if self.mat[j * self.nx + i] == 1 {
            self.mat[j * self.nx + i] = 0;
            self.blocks[(j / B) * self.bnx + i / B] -= 1;
        }
    }
    /// is any cell the bit's centre may use within reach of (i, j)?
    fn reachable(&self, i: usize, j: usize) -> bool {
        let c = self.centre(i, j);
        let (i0, i1, j0, j1) = self.span(c, self.r + self.h);
        let r2 = (self.r + self.h) * (self.r + self.h);
        for jj in j0..=j1 {
            for ii in i0..=i1 {
                if self.ok[jj * self.nx + ii] == 1 {
                    let q = self.centre(ii, jj);
                    if (q.x - c.x).powi(2) + (q.y - c.y).powi(2) <= r2 {
                        return true;
                    }
                }
            }
        }
        false
    }
    /// the nearest cell that still has material and can be reached (unreachable ones are dropped on the way)
    fn nearest_material(&mut self, from: Pt) -> Option<(usize, usize)> {
        let bny = self.ny.div_ceil(B);
        let (fb_x, fb_y) = (((from.x - self.x0) / self.h / B as f64).floor().clamp(0.0, (self.bnx - 1) as f64) as i64, ((from.y - self.y0) / self.h / B as f64).floor().clamp(0.0, (bny - 1) as f64) as i64);
        loop {
            let mut best: Option<(f64, usize, usize)> = None;
            let max_ring = self.bnx.max(bny) as i64;
            for ring in 0..=max_ring {
                for by in (fb_y - ring).max(0)..=(fb_y + ring).min(bny as i64 - 1) {
                    for bx in (fb_x - ring).max(0)..=(fb_x + ring).min(self.bnx as i64 - 1) {
                        if (bx - fb_x).abs().max((by - fb_y).abs()) != ring || self.blocks[by as usize * self.bnx + bx as usize] == 0 {
                            continue;
                        }
                        for j in by as usize * B..((by as usize + 1) * B).min(self.ny) {
                            for i in bx as usize * B..((bx as usize + 1) * B).min(self.nx) {
                                if self.mat[j * self.nx + i] == 1 {
                                    let c = self.centre(i, j);
                                    let d = (c.x - from.x).powi(2) + (c.y - from.y).powi(2);
                                    if best.is_none_or(|b| d < b.0) {
                                        best = Some((d, i, j));
                                    }
                                }
                            }
                        }
                    }
                }
                // blocks further out cannot hold anything nearer than this
                if let Some((d, ..)) = best {
                    if d.sqrt() <= ring as f64 * B as f64 * self.h {
                        break;
                    }
                }
            }
            let (_, i, j) = best?;
            if self.reachable(i, j) {
                return Some((i, j));
            }
            self.drop_cell(i, j); // a sliver in a corner no bit reaches
        }
    }
}

fn rot(v: (f64, f64), a: f64) -> (f64, f64) {
    let (s, c) = a.sin_cos();
    (v.0 * c - v.1 * s, v.0 * s + v.1 * c)
}

/// Plan the clearing. `centre`: where the bit's centre may go (the pocket inset by the bit radius, even-odd polygons);
/// `material`: what has to be cut (that region grown back by the bit radius, so only what the bit can reach).
pub fn plan(centre: &[Vec<Pt>], material: &[Vec<Pt>], p: &Params) -> Result<Plan, String> {
    let r = p.radius;
    let engage = p.engage.clamp(r * 0.06, r * 1.0);
    // cells: fine enough to count a thin sliver (engage / 3), never coarser than a quarter of the radius
    let mut h = (r / 8.0).min(engage / 3.0).clamp(0.003, 0.02);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for q in material.iter().chain(centre).flatten() {
        (x0, y0, x1, y1) = (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y));
    }
    if x0 > x1 {
        return Ok(Plan { comps: Vec::new(), steps: 0, cell: h });
    }
    let (w, ht) = (x1 - x0 + 4.0 * h, y1 - y0 + 4.0 * h);
    while (w / h) * (ht / h) > 5.0e6 {
        h *= 1.2; // a big pocket: a coarser grid keeps the memory in bounds
    }
    if h > r / 3.0 {
        return Err("the pocket is too large for adaptive clearing with this bit".into());
    }
    let (x0, y0) = (x0 - 2.0 * h, y0 - 2.0 * h);
    let (nx, ny) = ((w / h).ceil() as usize + 1, (ht / h).ceil() as usize + 1);
    let mut mat = fill(material, x0, y0, h, nx, ny);
    // the bit centre may use a cell only when all eight neighbours are inside too: whichever point of the cell the
    // path really passes, it is inside the region (a cell is only tested at its centre)
    let ok = {
        let raw = fill(centre, x0, y0, h, nx, ny);
        let mut e = vec![0u8; nx * ny];
        for j in 1..ny.saturating_sub(1) {
            for i in 1..nx.saturating_sub(1) {
                let k = j * nx + i;
                if raw[k] == 1 && raw[k - 1] == 1 && raw[k + 1] == 1 && raw[k - nx] == 1 && raw[k + nx] == 1 && raw[k - nx - 1] == 1 && raw[k - nx + 1] == 1 && raw[k + nx - 1] == 1 && raw[k + nx + 1] == 1 {
                    e[k] = 1;
                }
            }
        }
        e
    };
    // The strip along the walls that only the wall lap reaches (the last cell and a half before the wall) is left to that lap:
    // chasing it with the planner made light, crawling cuts along every wall.
    let outward = distance(&ok, nx, ny, false);
    for (k, m) in mat.iter_mut().enumerate() {
        if *m == 1 && f64::from(outward[k]) * h >= r - 1.6 * h {
            *m = 0;
        }
    }
    drop(outward);
    let bnx = nx.div_ceil(B);
    let mut blocks = vec![0u32; bnx * ny.div_ceil(B)];
    for j in 0..ny {
        for i in 0..nx {
            if mat[j * nx + i] == 1 {
                blocks[(j / B) * bnx + i / B] += 1;
            }
        }
    }
    let mut g = Grid { x0, y0, h, nx, ny, mat, ok, blocks, bnx, r };

    // step and the target cells per step
    let ds = (r * 0.4).clamp(2.0 * h, 0.08);
    let target = (engage * ds / (h * h)).max(2.0);
    let area: f64 = g.mat.iter().map(|&m| f64::from(m)).sum::<f64>() * h * h;
    if area / (engage * ds) > p.max_steps as f64 * 2.0 {
        return Err(format!("it would need about {} steps: raise the load or use rings", (area / (engage * ds)) as usize));
    }
    let side = if p.climb { -1.0 } else { 1.0 }; // turning toward the material: left (+) for conventional, right (-) for climb
    let (label, infos) = components(&g);
    let mut plan = Plan { comps: Vec::new(), steps: 0, cell: h };
    // the part being cut: its label, its entry and the runs so far
    let mut open: Option<(u32, Comp)> = None;
    let mut cur = pt((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let mut heading: Option<f64> = None;
    let mut opens = 0;
    loop {
        plan.steps += 1;
        if plan.steps > p.max_steps {
            return Err(format!("it would need more than {} steps: raise the load or use rings", p.max_steps));
        }
        if let (Some(head), Some((_, comp))) = (heading, open.as_mut()) {
            // 9 headings around the current one; keep the one whose cut is nearest the target load. When none of them is
            // allowed (the heading points into a wall) the fan opens wider: 90°, then 135° either way.
            let eval = |g: &Grid, d: f64| -> Option<u32> {
                let a = head + d;
                let q = pt(cur.x + ds * a.cos(), cur.y + ds * a.sin());
                g.ok_at(q).then(|| g.engagement(q))
            };
            let mut samples: Vec<(f64, Option<u32>)> = Vec::new();
            for wide in [1.0, 2.0, 3.0] {
                let dmax = 45f64.to_radians() * wide;
                samples = (-4..=4).map(|k| {
                    let d = dmax * f64::from(k) / 4.0;
                    (d, eval(&g, d))
                }).collect();
                if samples.iter().any(|s| s.1.is_some()) {
                    break;
                }
            }
            let cost = |a: u32| (f64::from(a) - target).abs() / target;
            let best = samples.iter().enumerate().filter_map(|(i, s)| s.1.map(|a| (i, cost(a)))).min_by(|a, b| a.1.total_cmp(&b.1).then((a.0 as i32 - 4).abs().cmp(&(b.0 as i32 - 4).abs())));
            if let Some((bi, _)) = best {
                let (mut d, a_best) = (samples[bi].0, samples[bi].1.unwrap());
                // refine toward the target between the best sample and the neighbour on the side that fixes the error
                let toward = if f64::from(a_best) < target { side } else { -side }; // the direction of more load
                let nb = (bi as i32 + if toward > 0.0 { 1 } else { -1 }).clamp(0, 8) as usize;
                if nb != bi {
                    if let Some(a_nb) = samples[nb].1 {
                        if (f64::from(a_best) - target) * (f64::from(a_nb) - target) < 0.0 {
                            let (mut lo, mut hi, mut a_lo) = (d, samples[nb].0, a_best);
                            for _ in 0..3 {
                                let mid = (lo + hi) / 2.0;
                                match eval(&g, mid) {
                                    Some(a_mid) if (f64::from(a_mid) - target) * (f64::from(a_lo) - target) > 0.0 => {
                                        lo = mid;
                                        a_lo = a_mid;
                                    }
                                    Some(_) => hi = mid,
                                    None => break,
                                }
                            }
                            d = lo;
                        }
                    }
                }
                let a = head + d;
                let q = pt(cur.x + ds * a.cos(), cur.y + ds * a.sin());
                let took = if g.ok_at(q) { g.engagement(q) } else { 0 };
                if took >= 1 && f64::from(took) >= 0.2 * target {
                    g.clear(q, r);
                    comp.runs.last_mut().unwrap().pts.push(q);
                    cur = q;
                    heading = Some(a);
                    continue;
                }
            }
            heading = None;
        }
        // stuck (or starting): the nearest material, and a spot beside it where the bit takes a light cut
        let Some((mi, mj)) = g.nearest_material(cur) else { break };
        let m = g.centre(mi, mj);
        let Some((q, start_heading)) = start_spot(&g, m, target, cur, ds, side) else {
            g.drop_cell(mi, mj); // nowhere the bit can stand to cut it
            continue;
        };
        let lab = g.label_at(&label, q);
        if open.as_ref().is_none_or(|o| o.0 != lab) {
            // another separate part of the pocket: close this one, go in at the roomiest spot of the next
            opens += 1;
            if opens > 64 || lab == 0 {
                return Err("the pocket has too many separate parts".into());
            }
            if let Some((_, done)) = open.take() {
                plan.comps.push(done);
            }
            let (entry, clearance) = infos[lab as usize - 1];
            let helix = (0.55 * r).min(clearance * 0.9);
            let helix = if helix >= 0.15 * r { helix } else { 0.0 };
            let start = pt(entry.x + helix, entry.y);
            g.clear(entry, helix + r);
            open = Some((lab, Comp { entry, helix, runs: vec![Run { pts: vec![start], down: false }] }));
            cur = start;
            heading = None;
            continue;
        }
        let comp = &mut open.as_mut().unwrap().1;
        // a short move stays at depth when the way is cut air, or at most grazes a little material (small hops along the
        // edge of what is cut); a longer one only through cut air, and a long one lifts
        let hop = dist(cur, q) <= 2.0 * ds;
        let allow = if hop { (2.0 * target) as u32 } else { 0 };
        if dist(cur, q) <= 6.0 * r && link_clear(&g, cur, q, allow) {
            comp.runs.last_mut().unwrap().pts.push(q);
        } else {
            comp.runs.push(Run { pts: vec![q], down: false }); // lift, move, plunge
        }
        g.clear(q, r); // always cuts at least `m`: every pass through here uses up material
        cur = q;
        heading = Some(start_heading);
    }
    if let Some((_, done)) = open.take() {
        plan.comps.push(done);
    }
    for c in &mut plan.comps {
        c.runs.retain(|r| r.pts.len() > 1);
    }
    plan.comps.retain(|c| !c.runs.is_empty());
    Ok(plan)
}

/// Chamfer distance in cells: from the nearest disallowed cell (`inward`, for the cells that are allowed) or from the
/// nearest allowed cell (not `inward`, for the others); 0 where the question doesn't apply.
fn distance(ok: &[u8], nx: usize, ny: usize, inward: bool) -> Vec<f32> {
    let want = u8::from(inward); // the cells whose distance is asked for
    let mut d: Vec<f32> = ok.iter().map(|&v| if v == want { 1e6 } else { 0.0 }).collect();
    let (a, b) = (1.0f32, std::f32::consts::SQRT_2);
    for j in 0..ny {
        for i in 0..nx {
            let k = j * nx + i;
            if d[k] == 0.0 {
                continue;
            }
            let mut v = d[k];
            if i > 0 {
                v = v.min(d[k - 1] + a);
            }
            if j > 0 {
                v = v.min(d[k - nx] + a);
                if i > 0 {
                    v = v.min(d[k - nx - 1] + b);
                }
                if i + 1 < nx {
                    v = v.min(d[k - nx + 1] + b);
                }
            }
            d[k] = v;
        }
    }
    for j in (0..ny).rev() {
        for i in (0..nx).rev() {
            let k = j * nx + i;
            if d[k] == 0.0 {
                continue;
            }
            let mut v = d[k];
            if i + 1 < nx {
                v = v.min(d[k + 1] + a);
            }
            if j + 1 < ny {
                v = v.min(d[k + nx] + a);
                if i + 1 < nx {
                    v = v.min(d[k + nx + 1] + b);
                }
                if i > 0 {
                    v = v.min(d[k + nx - 1] + b);
                }
            }
            d[k] = v;
        }
    }
    d
}

/// Where, and which way, to start cutting the material at `m`. Every spot the bit may use within reach of `m` is tried:
/// it should take a light cut there (about 60 % of the target load), and the first step along the edge must take about
/// the target (looked at one step ahead: a start at the wrong end of a wedge between the cut area and a wall would
/// slot into virgin material). The preferred side (climb, or conventional as chosen) wins unless it can't be done, then
/// the other direction is used. Returns the spot and the heading.
fn start_spot(g: &Grid, m: Pt, target: f64, from: Pt, ds: f64, side: f64) -> Option<(Pt, f64)> {
    let (ci, cj) = g.cell(m)?;
    let rc = ((g.r + g.h) / g.h).ceil() as i64;
    let stride = if rc > 8 { 2 } else { 1 };
    let mut best: Option<(f64, Pt, f64)> = None;
    let mut dj = -rc;
    while dj <= rc {
        let mut di = -rc;
        while di <= rc {
            let (i, j) = (ci as i64 + di, cj as i64 + dj);
            if i >= 0 && j >= 0 && (i as usize) < g.nx && (j as usize) < g.ny && g.ok[j as usize * g.nx + i as usize] == 1 {
                let c = g.centre(i as usize, j as usize);
                if dist(c, m) <= g.r {
                    let e = g.engagement(c);
                    if e >= 1 {
                        let base = (f64::from(e) - 0.6 * target).abs() / target + 0.01 * dist(c, from) / g.h / target;
                        let u = material_direction(g, c, ds, m);
                        for (turn, penalty) in [(-side, 0.0), (side, 0.6)] {
                            let t = rot(u, turn * std::f64::consts::FRAC_PI_2);
                            let head = t.1.atan2(t.0);
                            // the best of a fan of first steps
                            let first = (-2..=2)
                                .filter_map(|k| {
                                    let a = head + f64::from(k) * 22.5f64.to_radians();
                                    let q = pt(c.x + ds * a.cos(), c.y + ds * a.sin());
                                    g.ok_at(q).then(|| ((f64::from(g.engagement(q)) - target).abs() / target, a))
                                })
                                .min_by(|a, b| a.0.total_cmp(&b.0));
                            if let Some((err, a)) = first {
                                let cost = base + 2.0 * err + penalty;
                                if best.is_none_or(|b| cost < b.0) {
                                    best = Some((cost, c, a));
                                }
                            }
                        }
                    }
                }
            }
            di += stride;
        }
        dj += stride;
    }
    best.map(|b| (b.1, b.2))
}

/// unit vector from `q` toward the middle of the uncut material within a bit radius and one step of it (`fallback`'s
/// direction when it is too evenly spread to tell)
fn material_direction(g: &Grid, q: Pt, ds: f64, fallback: Pt) -> (f64, f64) {
    let reach = g.r + ds;
    let (i0, i1, j0, j1) = g.span(q, reach);
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    for j in j0..=j1 {
        for i in i0..=i1 {
            if g.mat[j * g.nx + i] == 1 {
                let c = g.centre(i, j);
                if dist(c, q) <= reach {
                    sx += c.x - q.x;
                    sy += c.y - q.y;
                    n += 1.0;
                }
            }
        }
    }
    let (vx, vy) = if n > 0.0 { (sx / n, sy / n) } else { (0.0, 0.0) };
    let len = vx.hypot(vy);
    if len > 0.15 * g.r {
        (vx / len, vy / len)
    } else {
        let d = dist(q, fallback).max(1e-9);
        ((fallback.x - q.x) / d, (fallback.y - q.y) / d)
    }
}

/// can the bit go straight from `a` to `b` inside the allowed region, with at most `allow` uncut cells under it on the way
/// (0 = through cut air; the last point is where the cut begins and may touch material)?
fn link_clear(g: &Grid, a: Pt, b: Pt, allow: u32) -> bool {
    let n = ((dist(a, b) / (g.r * 0.5)).ceil() as usize).max(1);
    (1..=n).all(|k| {
        let t = k as f64 / n as f64;
        let q = pt(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        g.ok_at(q) && (k == n || g.engagement(q) <= allow)
    })
}

/// Connected parts of the allowed region: a label per cell (0 = not allowed) and, per label (1-based), the entry point
/// (the spot farthest from any wall) with its clearance in inches.
fn components(g: &Grid) -> (Vec<u32>, Vec<(Pt, f64)>) {
    let (nx, ny) = (g.nx, g.ny);
    let d = distance(&g.ok, nx, ny, true);
    let mut label = vec![0u32; nx * ny];
    let mut infos: Vec<(Pt, f64)> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for s in 0..nx * ny {
        if g.ok[s] != 1 || label[s] != 0 {
            continue;
        }
        let id = infos.len() as u32 + 1;
        label[s] = id;
        stack.push(s);
        let mut best = s;
        while let Some(k) = stack.pop() {
            if d[k] > d[best] {
                best = k;
            }
            let (i, j) = (k % nx, k / nx);
            for (ni, nj) in [(i.wrapping_sub(1), j), (i + 1, j), (i, j.wrapping_sub(1)), (i, j + 1)] {
                if ni < nx && nj < ny {
                    let nk = nj * nx + ni;
                    if g.ok[nk] == 1 && label[nk] == 0 {
                        label[nk] = id;
                        stack.push(nk);
                    }
                }
            }
        }
        infos.push((g.centre(best % nx, best / nx), (f64::from(d[best]) * g.h - 0.5 * g.h).max(0.0)));
    }
    (label, infos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Pt> {
        vec![pt(x0, y0), pt(x1, y0), pt(x1, y1), pt(x0, y1)]
    }

    /// the plan's cutting path as one list of points, runs joined (links at depth are cutting too)
    fn path_of(plan: &Plan) -> Vec<Vec<Pt>> {
        plan.comps.iter().flat_map(|c| c.runs.iter().map(|r| r.pts.clone())).collect()
    }

    /// An independent check: walk the path with a fine raster of "cut" cells; returns (share of the reachable area left
    /// uncut, the load (material removed per unit of travel, as a width) at the 95th percentile and at the maximum).
    fn measure(plan: &Plan, region: &[(f64, f64, f64, f64)], bit_r: f64, reach: impl Fn(Pt) -> bool) -> (f64, f64, f64) {
        let h = 0.005;
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for r in region {
            (x0, y0, x1, y1) = (x0.min(r.0), y0.min(r.1), x1.max(r.2), y1.max(r.3));
        }
        let (nx, ny) = (((x1 - x0) / h) as usize + 1, ((y1 - y0) / h) as usize + 1);
        let mut cut = vec![false; nx * ny];
        let mut widths: Vec<(f64, f64)> = Vec::new(); // (load, length)
        let rr = (bit_r / h).ceil() as i64;
        let stamp = |p: Pt, cut: &mut Vec<bool>| -> usize {
            let (ci, cj) = (((p.x - x0) / h) as i64, ((p.y - y0) / h) as i64);
            let mut n = 0;
            for dj in -rr..=rr {
                for di in -rr..=rr {
                    let (i, j) = (ci + di, cj + dj);
                    if i < 0 || j < 0 || i as usize >= nx || j as usize >= ny {
                        continue;
                    }
                    let c = pt(x0 + (i as f64 + 0.5) * h, y0 + (j as f64 + 0.5) * h);
                    if dist(c, p) <= bit_r && !cut[j as usize * nx + i as usize] {
                        cut[j as usize * nx + i as usize] = true;
                        n += 1;
                    }
                }
            }
            n
        };
        for comp in &plan.comps {
            stamp(comp.entry, &mut cut);
            for k in 0..36 {
                let a = f64::from(k) * std::f64::consts::TAU / 36.0;
                stamp(pt(comp.entry.x + comp.helix * a.cos(), comp.entry.y + comp.helix * a.sin()), &mut cut);
            }
            for run in &comp.runs {
                let mut last: Option<Pt> = None;
                for &q in &run.pts {
                    if let Some(l) = last {
                        // walk the straight piece in small steps; the load = new area / length
                        let n = ((dist(l, q) / 0.02).ceil() as usize).max(1);
                        let mut new = 0;
                        for t in 1..=n {
                            let f = t as f64 / n as f64;
                            new += stamp(pt(l.x + (q.x - l.x) * f, l.y + (q.y - l.y) * f), &mut cut);
                        }
                        if dist(l, q) > 0.01 {
                            let wd = new as f64 * h * h / dist(l, q);
                            widths.push((wd, dist(l, q)));
                        }
                    } else {
                        stamp(q, &mut cut);
                    }
                    last = Some(q);
                }
            }
        }
        let (mut total, mut left) = (0usize, 0usize);
        for j in 0..ny {
            for i in 0..nx {
                let c = pt(x0 + (i as f64 + 0.5) * h, y0 + (j as f64 + 0.5) * h);
                if reach(c) {
                    total += 1;
                    if !cut[j * nx + i] {
                        left += 1;
                    }
                }
            }
        }
        // by distance travelled: a tiny hop does not weigh as much as a long lap
        widths.sort_by(|a, b| a.0.total_cmp(&b.0));
        let all: f64 = widths.iter().map(|w| w.1).sum();
        let (mut acc, mut p95) = (0.0, 0.0);
        for (wd, len) in &widths {
            acc += len;
            p95 = *wd;
            if acc >= 0.95 * all {
                break;
            }
        }
        (left as f64 / total.max(1) as f64, p95, widths.last().map_or(0.0, |w| w.0))
    }

    /// ADAPT_BMP=file.bmp: the cut order (colour by time) and the path, for looking at
    fn draw(plan: &Plan, w: f64, ht: f64, r: f64, file: &str) {
        let sc = 200.0; // px per inch
        let (pw, ph) = ((w * sc) as usize, (ht * sc) as usize);
        let mut order = vec![f64::NAN; pw * ph];
        let mut n = 0.0;
        let total: f64 = plan.comps.iter().flat_map(|c| c.runs.iter()).map(|r| r.pts.len() as f64).sum();
        let mut paths: Vec<Pt> = Vec::new();
        for c in &plan.comps {
            for run in &c.runs {
                for &q in &run.pts {
                    n += 1.0;
                    paths.push(q);
                    let (ci, cj) = ((q.x * sc) as i64, (q.y * sc) as i64);
                    let rr = (r * sc) as i64;
                    for dj in -rr..=rr {
                        for di in -rr..=rr {
                            let (i, j) = (ci + di, cj + dj);
                            if i >= 0 && j >= 0 && (i as usize) < pw && (j as usize) < ph && ((di * di + dj * dj) as f64) <= (r * sc) * (r * sc) && order[j as usize * pw + i as usize].is_nan() {
                                order[j as usize * pw + i as usize] = n / total;
                            }
                        }
                    }
                }
            }
        }
        let mut px = vec![[40u8, 40, 40]; pw * ph];
        for (k, o) in order.iter().enumerate() {
            if !o.is_nan() {
                let h = o * 300.0; // blue → red
                let (rr, gg, bb) = (((h / 300.0) * 255.0) as u8, (255.0 - (h / 150.0 - 1.0).abs() * 255.0).clamp(0.0, 255.0) as u8, (255.0 - (h / 300.0) * 255.0) as u8);
                px[k] = [rr, gg, bb];
            }
        }
        for run in plan.comps.iter().flat_map(|c| c.runs.iter()) {
            for pr in run.pts.windows(2) {
                let m = ((dist(pr[0], pr[1]) * sc) as usize).max(1);
                for t in 0..=m {
                    let f = t as f64 / m as f64;
                    let (x, y) = (((pr[0].x + (pr[1].x - pr[0].x) * f) * sc) as usize, ((pr[0].y + (pr[1].y - pr[0].y) * f) * sc) as usize);
                    if x < pw && y < ph {
                        px[y * pw + x] = [0, 0, 0];
                    }
                }
            }
            if !run.down {
                if let Some(q) = run.pts.first() {
                    let (x, y) = ((q.x * sc) as usize, (q.y * sc) as usize);
                    for d in 0..4 {
                        if x + d < pw && y < ph {
                            px[y * pw + x + d] = [255, 255, 255];
                        }
                    }
                }
            }
        }
        // 24-bit BMP, bottom row first (y is up already)
        let mut out = Vec::new();
        let size = 54 + pw * ph * 3 + ph * (pw * 3 % 4 != 0) as usize * (4 - pw * 3 % 4);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(size as u32).to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0, 54, 0, 0, 0, 40, 0, 0, 0]);
        out.extend_from_slice(&(pw as u32).to_le_bytes());
        out.extend_from_slice(&(ph as u32).to_le_bytes());
        out.extend_from_slice(&[1, 0, 24, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for j in 0..ph {
            for i in 0..pw {
                let p = px[j * pw + i];
                out.extend_from_slice(&[p[2], p[1], p[0]]);
            }
            for _ in 0..(4 - pw * 3 % 4) % 4 {
                out.push(0);
            }
        }
        std::fs::write(file, out).unwrap();
    }

    #[test]
    fn a_rectangle_is_cleared_with_the_load_held() {
        let r = 0.125;
        let engage = 0.03; // 12 % of the 0.25 bit
        let (w, ht) = (3.0, 2.0);
        // the bit centre stays r inside the walls; the material is that region grown back by r
        let plan = plan(&[rect(r, r, w - r, ht - r)], &[rect(0.0, 0.0, w, ht)], &Params { radius: r, engage, climb: true, max_steps: 400_000 }).unwrap();
        assert_eq!(plan.comps.len(), 1);
        assert!(plan.comps[0].helix > 0.05, "a helix is possible in this room: {}", plan.comps[0].helix);
        // everything the bit can reach (within r of the centre region) ends up cut
        // (the strip right at the walls is the wall lap's, in cam.rs)
        let reach = |c: Pt| c.x > 0.03 && c.x < w - 0.03 && c.y > 0.03 && c.y < ht - 0.03 && {
            let q = pt(c.x.clamp(r, w - r), c.y.clamp(r, ht - r));
            dist(c, q) <= r - 0.004
        };
        let (left, p95, max) = measure(&plan, &[(0.0, 0.0, w, ht)], r, reach);
        if let Ok(f) = std::env::var("ADAPT_BMP") {
            draw(&plan, w, ht, r, &f);
        }
        eprintln!("steps {} cell {:.4} runs {} left {:.4} load p95 {:.4} max {:.4} (target {engage})", plan.steps, plan.cell, plan.comps[0].runs.len(), left, p95, max);
        assert!(left < 0.01, "{:.2} % of the reachable area was left", left * 100.0);
        assert!(p95 < engage * 1.8, "load at the 95th percentile {p95:.4} against a target of {engage}");
        // every point of the path is inside the allowed region
        for run in path_of(&plan) {
            for q in run {
                assert!(q.x >= r - 1e-6 && q.x <= w - r + 1e-6 && q.y >= r - 1e-6 && q.y <= ht - r + 1e-6, "{q:?} outside");
            }
        }
    }

    #[test]
    fn an_island_stays_standing_and_both_sides_are_cleared() {
        let r = 0.125;
        let engage = 0.03;
        // 3 × 2 pocket with a 0.8 × 0.6 island in the middle: the allowed region has a hole (the island grown by r)
        let outer = rect(r, r, 3.0 - r, 2.0 - r);
        let hole = rect(1.1 - r, 0.7 - r, 1.9 + r, 1.3 + r);
        let mut hole_rev = hole.clone();
        hole_rev.reverse();
        let material_outer = rect(0.0, 0.0, 3.0, 2.0);
        let material_hole = rect(1.1, 0.7, 1.9, 1.3);
        let plan = plan(&[outer, hole_rev], &[material_outer, material_hole], &Params { radius: r, engage, climb: true, max_steps: 400_000 }).unwrap();
        let on_island = |c: Pt| c.x > 1.1 && c.x < 1.9 && c.y > 0.7 && c.y < 1.3;
        let mut cut_island = 0;
        for run in path_of(&plan) {
            for q in run {
                // the centre never goes inside the island's grown outline
                assert!(!(q.x > 1.1 - r + 1e-6 && q.x < 1.9 + r - 1e-6 && q.y > 0.7 - r + 1e-6 && q.y < 1.3 + r - 1e-6), "{q:?} is in the island");
                if on_island(q) {
                    cut_island += 1;
                }
            }
        }
        assert_eq!(cut_island, 0);
        let reach = |c: Pt| {
            let in_out = c.x > 0.03 && c.x < 2.97 && c.y > 0.03 && c.y < 1.97 && dist(c, pt(c.x.clamp(r, 3.0 - r), c.y.clamp(r, 2.0 - r))) <= r - 0.004;
            in_out && !on_island(c) && !(c.x > 1.1 - r && c.x < 1.9 + r && c.y > 0.7 - r && c.y < 1.3 + r && dist(c, pt(c.x.clamp(1.1, 1.9), c.y.clamp(0.7, 1.3))) <= r - 0.004 && false)
        };
        // an island's own outline is cut only by the wall lap (cam.rs): the area measured is the open floor
        let (left, p95, _) = measure(&plan, &[(0.0, 0.0, 3.0, 2.0)], r, |c| reach(c) && !(c.x > 1.1 - 0.02 && c.x < 1.9 + 0.02 && c.y > 0.7 - 0.02 && c.y < 1.3 + 0.02));
        if let Ok(f) = std::env::var("ADAPT_BMP2") {
            draw(&plan, 3.0, 2.0, r, &f);
        }
        eprintln!("island: steps {} runs {} left {:.4} p95 {:.4}", plan.steps, plan.comps.iter().map(|c| c.runs.len()).sum::<usize>(), left, p95);
        assert!(left < 0.01 && p95 < engage * 1.8, "left {left:.4}, load {p95:.4}");
    }

    #[test]
    fn separate_parts_and_too_big_jobs_are_reported() {
        let r = 0.125;
        let two = plan(&[rect(r, r, 1.0, 1.0), rect(2.0, r, 3.0, 1.0)], &[rect(0.0, 0.0, 1.0 + r, 1.0 + r), rect(2.0 - r, 0.0, 3.0 + r, 1.0 + r)], &Params { radius: r, engage: 0.03, climb: false, max_steps: 400_000 }).unwrap();
        assert_eq!(two.comps.len(), 2, "each part gets its own entry");
        // a huge pocket at a thin load needs more steps than allowed: an error, so the caller can use rings
        let big = plan(&[rect(r, r, 30.0, 30.0)], &[rect(0.0, 0.0, 30.0 + r, 30.0 + r)], &Params { radius: r, engage: 0.03, climb: true, max_steps: 50_000 });
        assert!(big.is_err());
    }
}
