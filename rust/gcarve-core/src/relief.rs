//! 3D reliefs (C++ `core/relief.*`): a height-field carved into the board (top at the surface, base `depth` below),
//! loaded from STL, placed at x,y with size w,h. 3D toolpaths (raster finishing, layered roughing) are generated from
//! the field with the tool's shape (ball, flat or V) so the cutter never gouges the model.

use crate::cam::{clampf, cut_to, rapid_to, CamResult, Material, Op, Tool, ToolType};
use crate::geom::{dist, flatten, pt, Contour, Pt, Rect};
use crate::job::{Move, Toolpath};
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pt3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Triangle {
    pub a: Pt3,
    pub b: Pt3,
    pub c: Pt3,
}

/// STL (binary or ASCII), 3MF (a zip) or OBJ from the file's bytes (the engine does no file I/O: the caller reads the
/// file).
pub fn load_stl(data: &[u8]) -> Result<Vec<Triangle>, String> {
    if data.starts_with(b"PK\x03\x04") {
        return crate::threemf::load_3mf(data);
    }
    if is_obj(data) {
        return load_obj(data);
    }
    let mut out = Vec::new();
    // binary: 80-byte header + uint32 count + 50 bytes per triangle; ASCII starts with "solid" and has "facet" words
    let ascii = data.starts_with(b"solid") && data.windows(5).any(|w| w == b"facet");
    let binary = |n: usize| -> Vec<Triangle> {
        (0..n)
            .map(|i| {
                let f = |k: usize| {
                    let o = 84 + i * 50 + 12 + k * 4;
                    f32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as f64
                };
                let p = |k: usize| Pt3 { x: f(k), y: f(k + 1), z: f(k + 2) };
                Triangle { a: p(0), b: p(3), c: p(6) }
            })
            .collect()
    };
    // the triangles a binary file can hold: its own count, cut to what is really there (some exporters pad the end
    // or write a wrong count; error log 2026-11-22: "no triangles in the STL file" twice)
    let fits = if data.len() >= 84 {
        let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
        n.min((data.len() - 84) / 50)
    } else {
        0
    };
    // checked: on 32-bit (web, Windows 7 x86) a made-up count wrapped the sum and passed, then reserve() panicked
    if !ascii && fits > 0 && fits.checked_mul(50).and_then(|b| b.checked_add(84)) == Some(data.len()) {
        return Ok(binary(fits));
    }
    let text = String::from_utf8_lossy(data);
    let mut tok = text.split_ascii_whitespace();
    let mut verts: Vec<Pt3> = Vec::new();
    'parse: while let Some(t) = tok.next() {
        if t.eq_ignore_ascii_case("vertex") {
            let mut v = [0.0; 3];
            for c in &mut v {
                match tok.next().and_then(|s| s.parse::<f64>().ok()) {
                    Some(x) => *c = x,
                    None => break 'parse, // like a failed `istream >>`: stop reading
                }
            }
            verts.push(Pt3 { x: v[0], y: v[1], z: v[2] });
            if verts.len() == 3 {
                out.push(Triangle { a: verts[0], b: verts[1], c: verts[2] });
                verts.clear();
            }
        }
    }
    if out.is_empty() && fits > 0 {
        out = binary(fits);
    }
    if out.is_empty() {
        return Err("no triangles in the STL file".into());
    }
    Ok(out)
}

/// Wavefront OBJ text: "v" and "f" lines (an ASCII STL starts with "solid"; a binary one has no such lines).
fn is_obj(data: &[u8]) -> bool {
    if data.starts_with(b"solid") {
        return false;
    }
    // a binary STL whose size adds up stays an STL whatever bytes its header or numbers hold
    if data.len() >= 84 && (u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize).checked_mul(50).and_then(|b| b.checked_add(84)) == Some(data.len()) {
        return false;
    }
    let head = String::from_utf8_lossy(&data[..data.len().min(1 << 20)]);
    let has = |k: &str| head.lines().any(|l| l.trim_start().starts_with(k));
    has("v ") && (has("f ") || head.len() == 1 << 20)
}

/// Wavefront OBJ: vertices (`v x y z`) and faces (`f 1 2 3 4`, `f 1/1/1 …`, negative = counted back from the last
/// vertex), polygons split into fans. OBJ is Y-up by convention (Blender, Maya, most exporters): turned Z-up the way
/// Blender reads it back, so the model lies as it did in the modelling program. Materials, normals and groups are
/// ignored.
pub fn load_obj(data: &[u8]) -> Result<Vec<Triangle>, String> {
    let text = String::from_utf8_lossy(data);
    let mut verts: Vec<Pt3> = Vec::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let mut w = line.split_ascii_whitespace();
        match w.next() {
            Some("v") => {
                let c: Vec<f64> = w.take(3).filter_map(|s| s.parse().ok()).collect();
                if c.len() == 3 && c.iter().all(|v| v.is_finite()) {
                    verts.push(Pt3 { x: c[0], y: -c[2], z: c[1] });
                } else {
                    return Err("damaged vertex in the OBJ file".into());
                }
            }
            Some("f") => {
                let ids: Vec<usize> = w
                    .filter_map(|t| t.split('/').next()?.parse::<i64>().ok())
                    .filter_map(|i| if i > 0 { usize::try_from(i - 1).ok() } else { verts.len().checked_sub(i.unsigned_abs() as usize) })
                    .filter(|&i| i < verts.len())
                    .collect();
                for k in 1..ids.len().saturating_sub(1) {
                    out.push(Triangle { a: verts[ids[0]], b: verts[ids[k]], c: verts[ids[k + 1]] });
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        return Err("no faces in the OBJ file".into());
    }
    Ok(out)
}

// One Relief type for the document, project files and carving: document.rs.
pub use crate::document::Relief;

impl Relief {
    pub fn at(&self, ix: i32, iy: i32) -> f32 {
        self.height[iy as usize * self.nx as usize + ix as usize]
    }
    /// surface z (<= 0) at a board point; outside the relief: 0 (uncut surface)
    pub fn surface_at(&self, px: f64, py: f64) -> f64 {
        if self.nx == 0 || px < self.x || py < self.y || px > self.x + self.w || py > self.y + self.h {
            return 0.0;
        }
        let ix = (((px - self.x) / self.w * self.nx as f64) as i32).clamp(0, self.nx - 1);
        let iy = (((py - self.y) / self.h * self.ny as f64) as i32).clamp(0, self.ny - 1);
        -self.depth * (1.0 - self.at(ix, iy) as f64)
    }
}

/// Rasterize triangles (any units/orientation: the XY bounding box is stretched onto the relief's rect, z is
/// normalized to 0..1) into `r.height` at roughly `cells` samples (C++ default 400000).
pub fn rasterize_relief(tris: &[Triangle], r: &mut Relief, cells: usize) {
    r.height.clear();
    r.nx = 0;
    r.ny = 0;
    if tris.is_empty() || r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let (mut x0, mut y0, mut z0, mut x1, mut y1, mut z1) = (1e300_f64, 1e300_f64, 1e300_f64, -1e300_f64, -1e300_f64, -1e300_f64);
    for t in tris {
        for p in [t.a, t.b, t.c] {
            x0 = x0.min(p.x);
            x1 = x1.max(p.x);
            y0 = y0.min(p.y);
            y1 = y1.max(p.y);
            z0 = z0.min(p.z);
            z1 = z1.max(p.z);
        }
    }
    if x1 - x0 < 1e-12 || y1 - y0 < 1e-12 {
        return;
    }
    let aspect = r.w / r.h;
    // no side over 4000: the height map is drawn as a texture (GPUs refuse long sides; a long thin model did it)
    r.nx = (cells as f64 * aspect).sqrt().round().clamp(2.0, 4000.0) as i32;
    r.ny = (r.nx as f64 / aspect).round().clamp(2.0, 4000.0) as i32;
    r.height = vec![0.0; r.nx as usize * r.ny as usize]; // unmapped cells: base
    let zr = if z1 - z0 > 1e-12 { z1 - z0 } else { 1.0 };
    let (sx, sy) = (r.nx as f64 / (x1 - x0), r.ny as f64 / (y1 - y0));
    for t in tris {
        // triangle in cell coordinates
        let (mut ax, ay, mut bx, by, mut cx, cy) = ((t.a.x - x0) * sx, (t.a.y - y0) * sy, (t.b.x - x0) * sx, (t.b.y - y0) * sy, (t.c.x - x0) * sx, (t.c.y - y0) * sy);
        let (az, bz, cz) = ((t.a.z - z0) / zr, (t.b.z - z0) / zr, (t.c.z - z0) / zr);
        if r.mirror {
            let n = r.nx as f64;
            (ax, bx, cx) = (n - ax, n - bx, n - cx);
        }
        let ix0 = 0.max(ax.min(bx).min(cx).floor() as i32);
        let ix1 = (r.nx - 1).min(ax.max(bx).max(cx).ceil() as i32);
        let iy0 = 0.max(ay.min(by).min(cy).floor() as i32);
        let iy1 = (r.ny - 1).min(ay.max(by).max(cy).ceil() as i32);
        let det = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
        if det.abs() < 1e-12 {
            continue;
        }
        for iy in iy0..=iy1 {
            for ix in ix0..=ix1 {
                let (px, py) = (ix as f64 + 0.5, iy as f64 + 0.5);
                let l1 = ((bx - px) * (cy - py) - (cx - px) * (by - py)) / det;
                let l2 = ((cx - px) * (ay - py) - (ax - px) * (cy - py)) / det;
                let l3 = 1.0 - l1 - l2;
                const EPS: f64 = -1e-6;
                if l1 < EPS || l2 < EPS || l3 < EPS {
                    continue;
                }
                let z = (l1 * az + l2 * bz + l3 * cz) as f32;
                let cell = &mut r.height[iy as usize * r.nx as usize + ix as usize];
                *cell = cell.max(z); // top-down z-buffer: the highest surface wins
            }
        }
    }
}

/// Tool tip height that just touches the surface anywhere under the cutter footprint at (x,y).
pub fn tip_height(r: &Relief, t: &Tool, x: f64, y: f64, allowance: f64) -> f64 {
    // the lowest the tip may go so that no part of the cutter dips below the (offset) surface
    // a tapered ball nose only needs the ring whose edge is still within the model's depth (its cone rises fast)
    let rad = t.reach(r.depth + allowance.abs() + 0.01);
    let ball = matches!(t.kind, ToolType::BallNose | ToolType::TaperedBall);
    let vee = t.kind == ToolType::VBit;
    let tan_half = if vee { (t.angle / 2.0 * PI / 180.0).tan() } else { 1.0 };
    let (cw, ch) = (r.w / r.nx as f64, r.h / r.ny as f64);
    let ix0 = ((x - rad - r.x) / cw).floor() as i32;
    let ix1 = ((x + rad - r.x) / cw).ceil() as i32;
    let iy0 = ((y - rad - r.y) / ch).floor() as i32;
    let iy1 = ((y + rad - r.y) / ch).ceil() as i32;
    let mut tip = r.surface_at(x, y) + allowance; // directly under the tip (d = 0, no drop): the exact lower bound
    for iy in iy0..=iy1 {
        for ix in ix0..=ix1 {
            let (cx, cy) = (r.x + (ix as f64 + 0.5) * cw, r.y + (iy as f64 + 0.5) * ch);
            let d = (cx - x).hypot(cy - y);
            if d > rad {
                continue;
            }
            let mut s = if ix < 0 || iy < 0 || ix >= r.nx || iy >= r.ny { 0.0 } else { -r.depth * (1.0 - r.at(ix, iy) as f64) };
            s += allowance;
            // cutter height above its tip at distance d
            let drop = if ball {
                t.ball_rise(d)
            } else if vee {
                d / tan_half
            } else {
                0.0
            };
            tip = tip.max(s - drop);
        }
    }
    tip.min(0.0)
}

/// Drop points that stay on a straight line (within tol) so raster passes don't export every sample.
fn add_simplified(out: &mut Vec<Move>, pts: &[[f64; 3]], tol: f64) {
    if pts.is_empty() {
        return;
    }
    let mut anchor = 0;
    let lin = |p: &[f64; 3]| Move::linear(Some(p[0]), Some(p[1]), Some(p[2]));
    out.push(lin(&pts[0]));
    for i in 2..=pts.len() {
        // can we skip all points between anchor and i-1? check the max deviation of the intermediate points
        let mut ok = i < pts.len();
        if ok {
            let (a, b) = (pts[anchor], pts[i]);
            let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
            let l2 = dx * dx + dy * dy + dz * dz;
            for p in &pts[anchor + 1..i] {
                let t = if l2 > 0.0 { clampf(((p[0] - a[0]) * dx + (p[1] - a[1]) * dy + (p[2] - a[2]) * dz) / l2, 0.0, 1.0) } else { 0.0 };
                let (ex, ey, ez) = (a[0] + dx * t - p[0], a[1] + dy * t - p[1], a[2] + dz * t - p[2]);
                if ex * ex + ey * ey + ez * ez > tol * tol {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            anchor = i - 1;
            out.push(lin(&pts[anchor]));
        }
    }
    if anchor != pts.len() - 1 {
        out.push(lin(&pts[pts.len() - 1]));
    }
}

/// Raster passes over the relief with tool `t` into `tp`. Roughing: only where the surface is below `floor_z`, cut flat
/// at `floor_z`.
#[allow(clippy::too_many_arguments)]
/// `rest`: rest machining of one patch, the rows kept to its box (min x, min y, max x, max y)
fn raster(r: &Relief, m: &Material, along_x: bool, sample: f64, inside_clip: &dyn Fn(f64, f64) -> bool, t: &Tool, step_frac: f64, tp: &mut Toolpath, allowance: f64, floor_z: f64, roughing: bool, rest: Option<Rect>) {
    let step = 0.002_f64.max(clampf(step_frac, 0.02, 1.0) * t.cut_diameter());
    let rad = t.cut_diameter() / 2.0;
    // cover the relief plus a cutter radius so edges get finished
    let mut u0 = (if along_x { r.x } else { r.y }) - rad;
    let mut u1 = (if along_x { r.x + r.w } else { r.y + r.h }) + rad;
    let mut v0 = (if along_x { r.y } else { r.x }) - rad;
    let mut v1 = (if along_x { r.y + r.h } else { r.x + r.w }) + rad;
    u0 = u0.max(0.0);
    v0 = v0.max(0.0);
    u1 = u1.min(if along_x { m.w } else { m.h });
    v1 = v1.min(if along_x { m.h } else { m.w });
    if let Some(b) = rest {
        let (bu0, bu1, bv0, bv1) = if along_x { (b.x0, b.x1, b.y0, b.y1) } else { (b.y0, b.y1, b.x0, b.x1) };
        (u0, u1, v0, v1) = (u0.max(bu0), u1.min(bu1), v0.max(bv0), v1.min(bv1));
        if u1 <= u0 || v1 <= v0 {
            return;
        }
    }
    let rest = rest.is_some();
    let rows = 1.max(((v1 - v0) / step).ceil() as i32);
    let cols = 2.max(((u1 - u0) / sample).ceil() as i32);
    let mut first = true;
    let mut cur = Pt::default();
    let mut flush = |pts: &mut Vec<[f64; 3]>, tp: &mut Toolpath| {
        if pts.len() < if rest { 4 } else { 2 } {
            pts.clear();
            return;
        }
        let st = pt(pts[0][0], pts[0][1]);
        if rest && !first && dist(cur, st) > sample * 1.5 && dist(cur, st) <= REST_HOP {
            // hop just over what the bit would touch on the way (it's all finished or left on purpose), not up to the
            // retract height: rest stretches are many and short
            let n = (dist(cur, st) / sample).ceil().max(1.0) as usize;
            let top = (0..=n).map(|k| {
                let f = k as f64 / n as f64;
                tip_height(r, t, cur.x + (st.x - cur.x) * f, cur.y + (st.y - cur.y) * f, allowance)
            }).fold(f64::MIN, f64::max);
            let hop = (top + 0.02).min(m.retract_z);
            cut_to(tp, cur, Some(hop));
            cut_to(tp, st, Some(hop));
            cut_to(tp, st, Some(pts[0][2]));
        } else if first || dist(cur, st) > sample * 1.5 {
            // move over at the retract height
            if !first {
                rapid_to(tp, cur, Some(m.retract_z));
            }
            rapid_to(tp, st, Some(m.retract_z));
            cut_to(tp, st, Some(pts[0][2]));
        }
        add_simplified(&mut tp.moves, pts, 0.0005);
        cur = pt(pts[pts.len() - 1][0], pts[pts.len() - 1][1]);
        first = false;
        pts.clear();
    };
    for row in 0..=rows {
        let v = v0 + (v1 - v0) * row as f64 / rows as f64;
        let fwd = row % 2 == 0;
        let mut pts: Vec<[f64; 3]> = Vec::new();
        for c in 0..=cols {
            let u = if fwd { u0 + (u1 - u0) * c as f64 / cols as f64 } else { u1 - (u1 - u0) * c as f64 / cols as f64 };
            let (x, y) = if along_x { (u, v) } else { (v, u) };
            if !inside_clip(x, y) {
                flush(&mut pts, tp);
                continue;
            }
            let mut z = tip_height(r, t, x, y, allowance);
            if roughing {
                if z >= floor_z - 1e-6 {
                    flush(&mut pts, tp); // nothing to remove at this layer here
                    continue;
                }
                z = floor_z;
            }
            pts.push([x, y, z]);
        }
        flush(&mut pts, tp);
    }
    if !first {
        rapid_to(tp, cur, Some(m.retract_z));
    }
}

/// Closed vectors flattened, with their boxes: the even-odd "inside" test for a machining boundary (none = all)
struct Clip(Vec<(Vec<Pt>, Rect)>);

impl Clip {
    fn new(boundary: &[Contour]) -> Clip {
        Clip(
            boundary
                .iter()
                .filter(|c| c.closed)
                .map(|c| flatten(c, 0.002))
                .filter(|p| p.len() >= 3)
                .map(|pts| {
                    let mut b = Rect::default();
                    for &q in &pts {
                        b.add(q);
                    }
                    (pts, b)
                })
                .collect(),
        )
    }
    fn inside(&self, x: f64, y: f64) -> bool {
        self.0.is_empty() || self.0.iter().filter(|(p, b)| x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1 && crate::geom::point_in_poly(pt(x, y), p.len(), |i| (p[i].x, p[i].y))).count() % 2 == 1
    }
}

/// An earlier 3D finishing toolpath on the same model, for rest machining: its bit and the vectors that limited it
pub struct Earlier {
    pub tool: Tool,
    pub boundary: Vec<Contour>,
}

/// Rest machining: a grid over the model marking where bit `t` reaches material the `earlier` bits left (each placed
/// on every grid point of its area, the reach of its edge, not its scallops), widened by `t`'s radius
struct RestMask {
    x0: f64,
    y0: f64,
    s: f64,
    nx: usize,
    ny: usize,
    /// patch number + 1 per grid point (0 = nothing to cut)
    patch: Vec<u32>,
    /// each patch's box (board inches)
    boxes: Vec<Rect>,
    /// each patch: rows along X give fewer separate stretches than rows along Y (fewer lifts and plunges)
    along_x: Vec<bool>,
}

/// leftover thinner than this is not worth a pass (in)
const REST_TOL: f64 = 0.002;
/// a stretch of leftover is cut only when it is this thick somewhere (in)
const REST_SEED: f64 = 0.008;

/// 8-connected groups of the `on` points of an `nx` × `ny` grid: group number (1…) per point, 0 = off; and how many
fn patches(on: &[bool], nx: usize, ny: usize) -> (Vec<u32>, usize) {
    let mut id = vec![0u32; on.len()];
    let mut count = 0;
    for start in 0..on.len() {
        if !on[start] || id[start] > 0 {
            continue;
        }
        count += 1;
        id[start] = count as u32;
        let mut todo = vec![start];
        while let Some(n) = todo.pop() {
            let (i, j) = ((n % nx) as i64, (n / nx) as i64);
            for (di, dj) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (qi, qj) = (i + di, j + dj);
                if qi >= 0 && qj >= 0 && (qi as usize) < nx && (qj as usize) < ny {
                    let q = qj as usize * nx + qi as usize;
                    if on[q] && id[q] == 0 {
                        id[q] = count as u32;
                        todo.push(q);
                    }
                }
            }
        }
    }
    (id, count)
}
/// rest machining: stretches closer than this are linked by a low hop over the finished surface instead of a lift to
/// the retract height and a slow plunge (in); each patch is cut on its own, so its rows follow on closely
const REST_HOP: f64 = 0.15;

impl RestMask {
    fn new(r: &Relief, m: &Material, t: &Tool, earlier: &[Earlier]) -> RestMask {
        let pad = t.cut_diameter() / 2.0;
        let (x0, y0) = ((r.x - pad).max(0.0), (r.y - pad).max(0.0));
        let (x1, y1) = ((r.x + r.w + pad).min(m.w), (r.y + r.h + pad).min(m.h));
        // fine enough that a groove the new bit fits in gets a point, and that an earlier ball's own rounding between
        // points stays under the tolerance; ≤ 2 M points
        let ball = earlier.iter().map(|e| e.tool.cut_diameter() / 2.0).fold(f64::MAX, f64::min);
        let cell = (r.w / r.nx.max(1) as f64).min(r.h / r.ny.max(1) as f64);
        let s = (t.cut_diameter() / 2.0).min((4.0 * ball * REST_TOL).sqrt()).max(cell).max(((x1 - x0) * (y1 - y0) / 2.0e6).sqrt()).max(0.002);
        let (nx, ny) = (((x1 - x0) / s).ceil() as usize + 1, ((y1 - y0) / s).ceil() as usize + 1);
        let at = |i: usize, j: usize| (x0 + i as f64 * s, y0 + j as f64 * s);
        // what the earlier bits leave: board top (0) until one comes down lower
        let mut left = vec![0.0_f64; nx * ny];
        for e in earlier {
            let clip = Clip::new(&e.boundary);
            let reach = e.tool.reach(r.depth + 0.01);
            let k = (reach / s).ceil() as i64;
            let ring: Vec<(i64, i64, f64)> = (-k..=k).flat_map(|dj| (-k..=k).map(move |di| (di, dj))).filter_map(|(di, dj)| {
                let d = (di as f64).hypot(dj as f64) * s;
                (d <= reach).then(|| (di, dj, e.tool.ball_rise(d)))
            }).collect();
            for j in 0..ny {
                for i in 0..nx {
                    let (x, y) = at(i, j);
                    if !clip.inside(x, y) {
                        continue;
                    }
                    let z = tip_height(r, &e.tool, x, y, 0.0);
                    for &(di, dj, rise) in &ring {
                        let (qi, qj) = (i as i64 + di, j as i64 + dj);
                        if qi >= 0 && qj >= 0 && (qi as usize) < nx && (qj as usize) < ny {
                            let c = &mut left[qj as usize * nx + qi as usize];
                            *c = c.min(z + rise);
                        }
                    }
                }
            }
        }
        // where the new bit's tip gets lower than that: stretches that somewhere go deeper than REST_SEED (a model's
        // stair-stepped slopes leave specks just over the tolerance everywhere), then widened by its radius
        let dz: Vec<f64> = (0..nx * ny).map(|n| {
            let (x, y) = at(n % nx, n / nx);
            left[n] - tip_height(r, t, x, y, 0.0)
        }).collect();
        let (blobs, count) = patches(&dz.iter().map(|&d| d > REST_TOL).collect::<Vec<_>>(), nx, ny);
        let mut deep = vec![false; count + 1];
        for n in 0..nx * ny {
            deep[blobs[n] as usize] |= dz[n] > REST_SEED;
        }
        let k = (pad / s).ceil() as i64 + 1;
        let mut on = vec![false; nx * ny];
        for n in (0..nx * ny).filter(|&n| blobs[n] > 0 && deep[blobs[n] as usize]) {
            let (i, j) = ((n % nx) as i64, (n / nx) as i64);
            for dj in -k..=k {
                for di in -k..=k {
                    let (qi, qj) = (i + di, j + dj);
                    if di * di + dj * dj <= k * k && qi >= 0 && qj >= 0 && (qi as usize) < nx && (qj as usize) < ny {
                        on[qj as usize * nx + qi as usize] = true;
                    }
                }
            }
        }
        // separate patches, each cut on its own
        let (patch, count) = patches(&on, nx, ny);
        let mut boxes = vec![Rect::default(); count];
        for n in (0..nx * ny).filter(|&n| patch[n] > 0) {
            let (x, y) = at(n % nx, n / nx);
            boxes[patch[n] as usize - 1].add(pt(x - s, y - s));
            boxes[patch[n] as usize - 1].add(pt(x + s, y + s));
        }
        // per patch, the row direction that crosses its edges least often (a long narrow groove is cut along itself)
        let (mut cross_x, mut cross_y) = (vec![0usize; count], vec![0usize; count]);
        for n in (0..nx * ny).filter(|&n| patch[n] > 0) {
            let id = patch[n] as usize - 1;
            if n % nx == 0 || patch[n - 1] != patch[n] {
                cross_x[id] += 1;
            }
            if n < nx || patch[n - nx] != patch[n] {
                cross_y[id] += 1;
            }
        }
        let along_x = cross_x.iter().zip(&cross_y).map(|(x, y)| x <= y).collect();
        RestMask { x0, y0, s, nx, ny, patch, boxes, along_x }
    }
    fn at(&self, x: f64, y: f64) -> u32 {
        let (i, j) = (((x - self.x0) / self.s).round(), ((y - self.y0) / self.s).round());
        if i >= 0.0 && j >= 0.0 && (i as usize) < self.nx && (j as usize) < self.ny { self.patch[j as usize * self.nx + i as usize] } else { 0 }
    }
}

/// op fields used: stepover (finish, fraction of diameter), raster_angle (0 along X, 90 along Y), rough/rough_stepdown/
/// rough_stepover (layered roughing with rough_tool, leaving op.allowance), name.
/// `boundary`: optional closed vectors; when given, only the area inside them is machined.
pub fn generate_relief(op: &Op, finish_tool: &Tool, rough_tool: Option<&Tool>, r: &Relief, m: &Material, boundary: Option<&[Contour]>) -> CamResult {
    generate_relief_rest(op, finish_tool, rough_tool, r, m, boundary, None)
}

/// `generate_relief` with rest machining: `rest` = the earlier 3D finishing toolpaths on this model (`op.rest` set);
/// the finishing pass then only cuts where they left material this bit reaches
pub fn generate_relief_rest(op: &Op, finish_tool: &Tool, rough_tool: Option<&Tool>, r: &Relief, m: &Material, boundary: Option<&[Contour]>, rest: Option<&[Earlier]>) -> CamResult {
    let mut res = CamResult::default();
    let clip = Clip::new(boundary.unwrap_or(&[]));
    let inside_clip = |x: f64, y: f64| clip.inside(x, y);
    if r.nx == 0 {
        res.warnings.push(format!("{}: relief has no model loaded", op.name));
        return res;
    }
    if r.depth > m.t + 1e-9 {
        res.warnings.push(format!("{}: relief is deeper than the material", op.name));
    }
    let sample = 0.004_f64.max((r.w / r.nx as f64).min(r.h / r.ny as f64)); // along-line sampling
    let along_x = op.raster_angle % 180 == 0;
    if let (true, Some(rt)) = (op.rough, rough_tool) {
        let mut rough = Toolpath { name: format!("{} [rough]", op.name), tool: op.clear_tool, feed_xy: rt.feed_xy, feed_z: rt.feed_z, moves: Vec::new(), comp: 0 };
        let sd = crate::cam::step_down(op.rough_stepdown, rt); // never 0: z -= 0 looped for ever
        let mut z = -sd;
        while z > -r.depth - 1e-9 {
            raster(r, m, along_x, sample, &inside_clip, rt, op.rough_stepover, &mut rough, op.allowance, z.max(-r.depth), true, None);
            z -= sd;
        }
        if !rough.moves.is_empty() {
            res.paths.push(rough);
        }
    }
    let mut fin = Toolpath { name: op.name.clone(), tool: -1, feed_xy: finish_tool.feed_xy, feed_z: finish_tool.feed_z, moves: Vec::new(), comp: 0 };
    match rest {
        Some([]) => {
            res.warnings.push(format!("{}: rest machining needs an earlier 3D toolpath on this model; cutting everything", op.name));
            raster(r, m, along_x, sample, &inside_clip, finish_tool, op.stepover, &mut fin, 0.0, 0.0, false, None);
        }
        Some(earlier) => {
            let mask = RestMask::new(r, m, finish_tool, earlier);
            // patch by patch, the nearest next
            let mut left: Vec<usize> = (0..mask.boxes.len()).collect();
            let mut here = pt(0.0, 0.0);
            let mid = |b: &Rect| pt((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
            while !left.is_empty() {
                let k = (0..left.len()).min_by(|&a, &b| dist(here, mid(&mask.boxes[left[a]])).total_cmp(&dist(here, mid(&mask.boxes[left[b]])))).unwrap();
                let id = left.swap_remove(k);
                let b = mask.boxes[id];
                raster(r, m, mask.along_x[id], sample, &|x, y| clip.inside(x, y) && mask.at(x, y) == id as u32 + 1, finish_tool, op.stepover, &mut fin, 0.0, 0.0, false, Some(b));
                here = mid(&b);
            }
        }
        None => raster(r, m, along_x, sample, &inside_clip, finish_tool, op.stepover, &mut fin, 0.0, 0.0, false, None),
    }
    res.paths.push(fin);
    res
}

/// Project toolpath onto a 3D model: every cutting move (below the surface) is cut into short steps whose depth is
/// measured down from the model's surface there instead of from the flat board top; rapids and lifts stay as they are.
pub fn project_paths(paths: &mut [crate::job::Toolpath], r: &Relief) {
    use crate::job::{Move, MoveType};
    const STEP: f64 = 0.02;
    for tp in paths {
        let (mut x, mut y, mut z) = (0.0, 0.0, f64::MAX);
        let mut out = Vec::with_capacity(tp.moves.len() * 2);
        for m in &tp.moves {
            let (nx, ny, nz) = (m.x.unwrap_or(x), m.y.unwrap_or(y), m.z.unwrap_or(z));
            let cutting = m.kind == MoveType::Linear && (nz < 0.0 || z < 0.0) && z != f64::MAX;
            if cutting {
                let n = ((nx - x).hypot(ny - y) / STEP).ceil().max(1.0) as usize;
                for k in 1..=n {
                    let f = k as f64 / n as f64;
                    let (px, py, pz) = (x + (nx - x) * f, y + (ny - y) * f, z + (nz - z) * f);
                    let dz = if pz < 0.0 { r.surface_at(px, py) } else { 0.0 };
                    out.push(Move::linear(Some(px), Some(py), Some(pz + dz)));
                }
            } else {
                out.push(*m);
            }
            (x, y, z) = (nx, ny, nz);
        }
        tp.moves = out;
    }
}

/// Height field from the relief's picture: brightness is height (white = the board's surface, black = `depth` down),
/// or the other way round for a lithophane (`pic_invert`). Auto levels stretch the picture's own range over the whole
/// depth; smoothing is a box blur of `pic_smooth` pixels. The grid is the picture's own pixels (rows flipped: y is up).
/// A picture that doesn't decode leaves the height empty ("file missing" in the dialog).
pub fn rasterize_picture(r: &mut Relief) {
    r.height.clear();
    (r.nx, r.ny) = (0, 0);
    let Some(g) = crate::photo::Gray::from_op(r.pic_w, r.pic_h, &r.pic).filter(|g| g.w >= 2 && g.h >= 2 && g.w.max(g.h) <= 4000) else { return };
    let (lo, hi) = if r.pic_levels {
        let (dark_lo, dark_hi) = g.levels(); // darkness range; brightness is its mirror
        (1.0 - dark_hi, 1.0 - dark_lo)
    } else {
        (0.0, 1.0)
    };
    let mut v: Vec<f32> = g.px.iter().map(|&p| (((f64::from(p) / 255.0 - lo) / (hi - lo).max(1e-6)).clamp(0.0, 1.0)) as f32).collect();
    // box blur, rows then columns (clamped edges)
    v = box_blur(&v, g.w, g.h, r.pic_smooth.min(20) as usize);
    if r.pic_invert {
        v.iter_mut().for_each(|x| *x = 1.0 - *x);
    }
    r.nx = g.w as i32;
    r.ny = g.h as i32;
    r.height = (0..g.h).rev().flat_map(|row| v[row * g.w..(row + 1) * g.w].to_vec()).collect(); // row 0 of the picture = top = highest y
}

/// Box blur of a `w` × `h` grid, `k` cells each way, rows then columns (edges clamped); `k` 0 = a copy.
fn box_blur(v: &[f32], w: usize, h: usize, k: usize) -> Vec<f32> {
    if k == 0 {
        return v.to_vec();
    }
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0.0; src.len()];
        let (len, lines) = if horizontal { (w, h) } else { (h, w) };
        for line in 0..lines {
            let at = |i: usize| if horizontal { src[line * w + i] } else { src[i * w + line] };
            for i in 0..len {
                let (a, b) = (i.saturating_sub(k), (i + k).min(len - 1));
                let val = (a..=b).map(at).sum::<f32>() / (b - a + 1) as f32;
                if horizontal { out[line * w + i] = val } else { out[i * w + line] = val }
            }
        }
        out
    };
    pass(&pass(v, true), false)
}

// ---- 3D components ---------------------------------------------------------------------------------------------

/// `Op::relief` value for a 3D toolpath that cuts every component combined.
pub const ALL_MODELS: i32 = 0;

/// A height map as text for the project: 16-bit heights, zlib, base64.
pub fn pack_heights(h: &[f32]) -> String {
    let raw: Vec<u8> = h.iter().flat_map(|&v| ((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes()).collect();
    crate::photo::b64_encode(&miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6))
}
/// `pack_heights` back; empty when the text is damaged or not `n` values long.
pub fn unpack_heights(text: &str, n: usize) -> Vec<f32> {
    let Ok(raw) = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&crate::photo::b64_decode(text), 2 * n) else { return Vec::new() };
    if raw.len() != 2 * n {
        return Vec::new();
    }
    raw.as_chunks::<2>().0.iter().map(|b| f32::from(u16::from_le_bytes(*b)) / 65535.0).collect()
}

/// The component's own heights (0..1) after smoothing and the slice.
fn shaped(r: &Relief) -> Vec<f32> {
    let mut v = box_blur(&r.height, r.nx as usize, r.ny as usize, r.smooth.min(50) as usize);
    let (lo, hi) = (r.clip_lo.clamp(0.0, 1.0) as f32, r.clip_hi.clamp(0.0, 1.0) as f32);
    if lo > 0.0 || hi < 1.0 {
        let span = (hi - lo).max(1e-4);
        v.iter_mut().for_each(|x| *x = ((x.clamp(lo, hi.max(lo)) - lo) / span).clamp(0.0, 1.0));
    }
    v
}

fn plain(r: &Relief) -> bool {
    r.combine == 0 && r.lift == 0.0 && r.clip_lo <= 0.0 && r.clip_hi >= 1.0 && r.smooth == 0
}

/// The components combined, in order, into one model over their joint box: each adds its height (lift + depth ×
/// shape) where it lies, subtracts it, or keeps the higher / lower of it and what is there; the board between them is
/// the base (0). The result is an ordinary relief whose top touches the board surface, so everything that cuts or
/// follows a relief takes it. None when nothing stands out.
pub fn composite(parts: &[&Relief]) -> Option<Relief> {
    let parts: Vec<&Relief> = parts.iter().copied().filter(|r| r.nx >= 2 && r.ny >= 2 && r.height.len() == (r.nx * r.ny) as usize).collect();
    match parts.as_slice() {
        [] => return None,
        [r] if plain(r) => return Some((*r).clone()),
        _ => {}
    }
    let (x0, y0) = parts.iter().fold((f64::MAX, f64::MAX), |a, r| (a.0.min(r.x), a.1.min(r.y)));
    let (x1, y1) = parts.iter().fold((f64::MIN, f64::MIN), |a, r| (a.0.max(r.x + r.w), a.1.max(r.y + r.h)));
    let (w, h) = (x1 - x0, y1 - y0);
    // the finest component's cells, at most about 1.5 M cells (4000 a side: drawn as a texture)
    let cell = parts.iter().map(|r| (r.w / r.nx as f64).min(r.h / r.ny as f64)).fold(f64::MAX, f64::min).max((w * h / 1.5e6).sqrt());
    let nx = ((w / cell).ceil() as i32).clamp(2, 4000);
    let ny = ((h / cell).ceil() as i32).clamp(2, 4000);
    let (cw, ch) = (w / nx as f64, h / ny as f64);
    let mut c = vec![0.0f64; (nx * ny) as usize];
    for r in &parts {
        let v = shaped(r);
        // only the cells whose centre lies on this component
        let (ix0, ix1) = (((r.x - x0) / cw - 0.5).ceil().max(0.0) as i32, (((r.x + r.w - x0) / cw - 0.5).floor() as i32).min(nx - 1));
        let (iy0, iy1) = (((r.y - y0) / ch - 0.5).ceil().max(0.0) as i32, (((r.y + r.h - y0) / ch - 0.5).floor() as i32).min(ny - 1));
        for iy in iy0..=iy1 {
            let py = y0 + (iy as f64 + 0.5) * ch;
            let jy = (((py - r.y) / r.h * r.ny as f64) as i32).clamp(0, r.ny - 1);
            for ix in ix0..=ix1 {
                let px = x0 + (ix as f64 + 0.5) * cw;
                let jx = (((px - r.x) / r.w * r.nx as f64) as i32).clamp(0, r.nx - 1);
                let hv = r.lift + r.depth * f64::from(v[(jy * r.nx + jx) as usize]);
                let cur = &mut c[(iy * nx + ix) as usize];
                *cur = match r.combine {
                    0 => *cur + hv,
                    1 => *cur - hv,
                    2 => cur.max(hv),
                    _ => cur.min(hv),
                };
            }
        }
    }
    // the base is the board's level (0) wherever components cover the whole box
    let (lo, top) = c.iter().fold((0.0f64, f64::MIN), |a, &v| (a.0.min(v), a.1.max(v)));
    if top - lo < 1e-6 {
        return None;
    }
    let name = parts.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(" + ");
    Some(Relief {
        id: ALL_MODELS,
        name,
        x: x0,
        y: y0,
        w,
        h,
        depth: top - lo,
        nx,
        ny,
        height: c.iter().map(|&v| ((v - lo) / (top - lo)) as f32).collect(),
        ..Default::default()
    })
}

/// What a 3D toolpath (or a toolpath that follows a model) cuts: one component with its settings, or with
/// `ALL_MODELS` every component combined.
pub fn model_for(doc: &crate::document::Document, id: i32) -> Option<Relief> {
    composite(&doc.reliefs.iter().filter(|r| id == ALL_MODELS || r.id == id).collect::<Vec<_>>())
}

/// Closed vectors round where the model stands above its base (VCarve's boundary from components): to cut it out or
/// to keep a 3D toolpath inside it.
pub fn outline(r: &Relief) -> Vec<Contour> {
    let (w, h) = (r.nx as usize, r.ny as usize);
    if w < 2 || h < 2 || r.height.len() != w * h {
        return Vec::new();
    }
    // the tracer reads row 0 as the top: rows flipped; 0 = ink where the model stands
    let gray: Vec<u8> = (0..h).rev().flat_map(|row| r.height[row * w..(row + 1) * w].iter().map(|&v| if v > 0.002 { 0u8 } else { 255 })).collect();
    let o = crate::trace::TraceOptions { threshold: Some(128), width_in: r.w, smooth_px: 0.7, min_area_px: 4.0, ..Default::default() };
    // `a.then(b)` applies b first: stretch rows to the cell height, then move onto the board
    let to_board = crate::geom::Affine::translate(r.x, r.y).then(&crate::geom::Affine::scale(1.0, (r.h / h as f64) / (r.w / w as f64)));
    crate::trace::trace(&gray, w, h, &o).iter().map(|c| crate::geom::transformed(c, &to_board)).collect()
}

#[cfg(test)]
mod obj_tests {
    use super::*;

    /// A Y-up OBJ square pyramid (quad base, triangle sides, slashes and a negative index) comes in Z-up, its tip the
    /// highest point, and carves like the same STL would.
    #[test]
    fn an_obj_model_loads_upright_with_quads_slashes_and_relative_indices() {
        let obj = b"# pyramid\nmtllib x.mtl\nv 0 0 0\nv 2 0 0\nv 2 0 -2\nv 0 0 -2\nv 1 1.5 -1\nvn 0 1 0\nf 1/1/1 2/2/1 3/3/1 4/4/1\nf 1 2 5\nf 2 3 5\nf 3 4 5\nf 4 1 -1\n";
        let tris = load_stl(obj).unwrap();
        assert_eq!(tris.len(), 2 + 4, "the quad base is two triangles");
        let top = tris.iter().flat_map(|t| [t.a, t.b, t.c]).fold(f64::MIN, |m, p| m.max(p.z));
        assert!((top - 1.5).abs() < 1e-9, "Y-up height became Z");
        assert!(tris.iter().flat_map(|t| [t.a, t.b, t.c]).all(|p| (0.0..=2.0).contains(&p.y)), "depth -Z became +Y");
        let mut r = Relief { w: 2.0, h: 2.0, depth: 0.5, ..Default::default() };
        rasterize_relief(&tris, &mut r, 400);
        let peak = r.height.iter().cloned().fold(0.0f32, f32::max);
        assert!(peak > 0.9 && r.at(0, 0) < 0.2, "the tip stands up in the middle");
        assert!(load_stl(b"v 0 0 0\nv 1 0 0\n").is_err() && load_stl(b"v 0 0 x\nf 1 1 1\n").is_err());
    }
    #[test]
    fn binary_stl_with_padding_or_a_wrong_count_still_loads() {
        let mut b = vec![0u8; 80];
        b.extend(9u32.to_le_bytes()); // says 9, holds 2
        for k in 0..2 {
            b.extend([0u8; 12]);
            for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, k as f32] {
                b.extend(v.to_le_bytes());
            }
            b.extend([0u8; 2]);
        }
        assert_eq!(load_stl(&b).unwrap().len(), 2);
        b[80..84].copy_from_slice(&2u32.to_le_bytes());
        b.extend([0u8; 7]); // padding after the last triangle
        assert_eq!(load_stl(&b).unwrap().len(), 2);
        assert_eq!(load_stl(b"SOLID x\nFACET NORMAL 0 0 1\nVERTEX 0 0 0\nVERTEX 1 0 0\nVERTEX 0 1 0\n").unwrap().len(), 1);
    }

}

#[cfg(test)]
mod component_tests {
    use super::*;

    /// a w × h model whose cells are all `v`
    fn flat(id: i32, x: f64, y: f64, w: f64, h: f64, depth: f64, v: f32) -> Relief {
        Relief { id, name: format!("m{id}"), x, y, w, h, depth, nx: 20, ny: 20, height: vec![v; 400], ..Default::default() }
    }

    #[test]
    fn one_plain_component_is_the_model_itself() {
        let r = flat(1, 1.0, 1.0, 2.0, 2.0, 0.3, 0.5);
        assert_eq!(composite(&[&r]), Some(r.clone()));
    }

    #[test]
    fn components_add_subtract_and_keep_the_higher_in_list_order() {
        // a 0.2 block over the left 2 in, a 0.1 block over the right 2 in, overlapping on x 1..2
        let a = flat(1, 0.0, 0.0, 2.0, 1.0, 0.2, 1.0);
        let mut b = flat(2, 1.0, 0.0, 2.0, 1.0, 0.1, 1.0);
        let z = |m: &Relief, x: f64| m.surface_at(x, 0.5);
        let m = composite(&[&a, &b]).unwrap();
        assert!((m.depth - 0.3).abs() < 1e-6 && (m.x, m.w) == (0.0, 3.0));
        assert!(z(&m, 1.5).abs() < 1e-6 && (z(&m, 0.5) + 0.1).abs() < 1e-6 && (z(&m, 2.5) + 0.2).abs() < 1e-6, "the overlap adds up to the top");
        b.combine = 2;
        let m = composite(&[&a, &b]).unwrap();
        assert!(z(&m, 1.5).abs() < 1e-6 && z(&m, 0.5).abs() < 1e-6 && (z(&m, 2.5) + 0.1).abs() < 1e-6, "the higher one wins where they meet");
        b.combine = 1;
        let m = composite(&[&a, &b]).unwrap();
        // left 0.2, overlap 0.1, right -0.1: top 0.2 at the surface
        assert!((m.depth - 0.3).abs() < 1e-6 && (z(&m, 1.5) + 0.1).abs() < 1e-6 && (z(&m, 2.5) + 0.3).abs() < 1e-6, "subtracted below the base");
        // a lift raises a component's base
        b.combine = 0;
        b.lift = 0.05;
        let m = composite(&[&a, &b]).unwrap();
        assert!((m.depth - 0.35).abs() < 1e-6 && (z(&m, 2.5) + 0.2).abs() < 1e-6);
    }

    #[test]
    fn a_slice_keeps_part_of_the_height_and_smoothing_softens_it() {
        // a ramp from 0 at the left to 1 at the right
        let mut r = Relief { id: 1, x: 0.0, y: 0.0, w: 2.0, h: 1.0, depth: 0.4, nx: 20, ny: 2, height: (0..40).map(|i| (i % 20) as f32 / 19.0).collect(), ..Default::default() };
        r.clip_hi = 0.5; // a flat top over the right half
        let m = composite(&[&r]).unwrap();
        assert!(m.surface_at(1.9, 0.5).abs() < 1e-6 && m.surface_at(1.2, 0.5).abs() < 1e-6 && (m.surface_at(0.02, 0.5) + 0.4).abs() < 1e-6);
        r.clip_hi = 1.0;
        r.clip_lo = 0.5; // the zero plane half way up: the left half is the base
        let m = composite(&[&r]).unwrap();
        assert!((m.surface_at(0.5, 0.5) + 0.4).abs() < 1e-6 && m.surface_at(1.99, 0.5).abs() < 1e-6);
        let step = Relief { id: 2, x: 0.0, y: 0.0, w: 2.0, h: 1.0, depth: 0.4, nx: 20, ny: 2, height: (0..40).map(|i| if i % 20 < 10 { 0.0 } else { 1.0 }).collect(), smooth: 2, ..Default::default() };
        let m = composite(&[&step]).unwrap();
        assert!(m.height.iter().any(|&v| v > 0.1 && v < 0.9), "a soft step");
    }

    #[test]
    fn heights_survive_packing_and_the_outline_hugs_the_model() {
        let r = flat(1, 0.0, 0.0, 1.0, 1.0, 0.2, 0.0);
        let mut h = r.height.clone();
        for iy in 5..15 {
            for ix in 4..12 {
                h[iy * 20 + ix] = 0.7;
            }
        }
        let back = unpack_heights(&pack_heights(&h), h.len());
        assert!(back.iter().zip(&h).all(|(a, b)| (a - b).abs() < 1e-4));
        assert!(unpack_heights("damaged", h.len()).is_empty() && unpack_heights(&pack_heights(&h), 7).is_empty());
        let m = Relief { x: 2.0, y: 1.0, w: 2.0, h: 1.0, height: h, ..r };
        let o = outline(&m);
        assert_eq!(o.len(), 1, "one block, one outline");
        let b = o[0].nodes.iter().fold((f64::MAX, f64::MIN, f64::MAX, f64::MIN), |a, n| (a.0.min(n.p.x), a.1.max(n.p.x), a.2.min(n.p.y), a.3.max(n.p.y)));
        // cells 4..12 of 20 across 2 in from x 2 = 2.4 .. 3.2; rows 5..15 of 20 across 1 in from y 1 = 1.25 .. 1.75
        assert!((b.0 - 2.4).abs() < 0.06 && (b.1 - 3.2).abs() < 0.06 && (b.2 - 1.25).abs() < 0.04 && (b.3 - 1.75).abs() < 0.04, "{b:?}");
    }
}

#[cfg(test)]
mod component_project_tests {
    use super::*;
    use crate::document::{Document, Op, OpType, Tool, ToolType};

    /// Two models saved in a project come back with their heights and settings on any computer (no STL file needed),
    /// and a 3D toolpath on all of them cuts the combined surface.
    #[test]
    fn components_travel_in_the_project_and_cut_combined() {
        let mut d = Document::default();
        let ramp: Vec<f32> = (0..400).map(|i| (i % 20) as f32 / 19.0).collect();
        d.reliefs.push(Relief { id: 1, name: "ramp".into(), source: "/nowhere/ramp.stl".into(), x: 1.0, y: 1.0, w: 4.0, h: 2.0, depth: 0.2, nx: 20, ny: 20, height: ramp, ..Default::default() });
        d.reliefs.push(Relief { id: 2, name: "block".into(), source: "/nowhere/block.stl".into(), x: 4.0, y: 1.0, w: 2.0, h: 2.0, depth: 0.1, nx: 20, ny: 20, height: vec![1.0; 400], combine: 2, lift: 0.05, clip_lo: 0.1, clip_hi: 0.9, smooth: 1, ..Default::default() });
        d.next_relief_id = 3;
        let back = crate::project::load_project(&crate::project::save_project(&d)).unwrap();
        for (a, b) in d.reliefs.iter().zip(&back.reliefs) {
            assert_eq!((a.nx, a.ny, a.combine, a.lift, a.clip_lo, a.clip_hi, a.smooth), (b.nx, b.ny, b.combine, b.lift, b.clip_lo, b.clip_hi, b.smooth));
            assert!(a.height.iter().zip(&b.height).all(|(x, y)| (x - y).abs() < 1e-4) && a.height.len() == b.height.len());
        }
        // plain settings stay out of the file
        let mut p = Document::default();
        p.reliefs.push(Relief { id: 1, nx: 2, ny: 2, height: vec![0.0; 4], ..Default::default() });
        let plain = crate::project::save_project(&p);
        assert!(!plain.contains("\"combine\"") && !plain.contains("\"slice\"") && plain.contains("\"heights\""));

        let mut d = back;
        d.tools.push(Tool { kind: ToolType::BallNose, diameter: 0.125, stepdown: 0.1, feed_xy: 80.0, feed_z: 30.0, ..Default::default() });
        d.ops.push(Op { kind: OpType::Relief, name: "3D".into(), relief: ALL_MODELS, tool: 0, stepover: 0.3, ..Default::default() });
        let g = crate::pipeline::regenerate(&d);
        let xs: Vec<f64> = g.paths[0].iter().flat_map(|p| p.moves.iter()).filter_map(|m| m.x).collect();
        assert!(g.warnings.iter().all(|w| !w.contains("not loaded")), "{:?}", g.warnings);
        let (lo, hi) = xs.iter().fold((f64::MAX, f64::MIN), |a, &x| (a.0.min(x), a.1.max(x)));
        assert!(lo < 1.2 && hi > 5.8, "spans both models: {lo}..{hi}");
    }
}

#[cfg(test)]
mod picture_tests {
    use super::*;
    use crate::photo::b64_encode;

    fn from(px: &[u8], w: u32, h: u32) -> Relief {
        Relief { w: 4.0, h: 2.0, depth: 0.3, pic: b64_encode(px), pic_w: w, pic_h: h, pic_levels: false, pic_smooth: 0, ..Default::default() }
    }

    #[test]
    fn a_picture_becomes_a_height_field_light_high_or_dark_high() {
        // 2 × 2: top row black, white; bottom row grey, white
        let mut r = from(&[0, 255, 128, 255], 2, 2);
        rasterize_picture(&mut r);
        assert_eq!((r.nx, r.ny), (2, 2));
        // y is up: the picture's bottom row comes first
        let g = 128.0 / 255.0;
        assert!((r.at(0, 0) - g).abs() < 1e-6 && r.at(1, 0) == 1.0 && r.at(0, 1) == 0.0 && r.at(1, 1) == 1.0, "{:?}", r.height);
        r.pic_invert = true;
        rasterize_picture(&mut r);
        assert!(r.at(0, 1) == 1.0 && r.at(1, 1) == 0.0 && (r.at(0, 0) - (1.0 - g)).abs() < 1e-6, "{:?}", r.height);
        // a picture that isn't there leaves nothing to cut
        r.pic = "!!".into();
        rasterize_picture(&mut r);
        assert!(r.height.is_empty() && r.nx == 0);
    }

    #[test]
    fn auto_levels_use_the_whole_depth_and_smoothing_softens_steps() {
        // a dull picture (greys 100..160) spreads from 0 to 1
        let px: Vec<u8> = (0..100u32).map(|i| 100 + (i % 10) as u8 * 6).collect();
        let mut r = from(&px, 10, 10);
        r.pic_levels = true;
        rasterize_picture(&mut r);
        let (lo, hi) = r.height.iter().fold((1.0f32, 0.0f32), |(a, b), &x| (a.min(x), b.max(x)));
        assert!(lo < 0.05 && hi > 0.95, "{lo} {hi}");
        // a hard step: smoothing leaves values between, and keeps 0..1
        let step: Vec<u8> = (0..60u32).map(|i| if i % 6 < 3 { 0 } else { 255 }).collect();
        let mut s = from(&step, 6, 10);
        rasterize_picture(&mut s);
        let sharp = s.height.iter().filter(|&&x| x > 0.01 && x < 0.99).count();
        s.pic_smooth = 1;
        rasterize_picture(&mut s);
        let soft = s.height.iter().filter(|&&x| x > 0.01 && x < 0.99).count();
        assert!(sharp == 0 && soft > 10 && s.height.iter().all(|&x| (0.0..=1.0).contains(&x)), "{sharp} {soft}");
    }

    /// The ball-nose finish of a picture relief goes deepest where the picture is dark, and stays above the surface
    /// where it is white: a 3D look at what the picture will carve.
    #[test]
    fn a_picture_relief_cuts_deep_where_it_is_dark() {
        use crate::document::{Material, Op, OpType, Tool, ToolType};
        // left half black, right half white
        let px: Vec<u8> = (0..400).map(|i| if i % 20 < 10 { 0 } else { 255 }).collect();
        let mut r = Relief { id: 1, x: 1.0, y: 1.0, w: 2.0, h: 2.0, depth: 0.3, pic: b64_encode(&px), pic_w: 20, pic_h: 20, pic_levels: false, pic_smooth: 0, ..Default::default() };
        rasterize_picture(&mut r);
        let op = Op { kind: OpType::Relief, name: "R".into(), relief: 1, stepover: 0.2, rough: false, ..Default::default() };
        let tool = Tool { kind: ToolType::BallNose, diameter: 0.125, stepdown: 0.1, feed_xy: 80.0, feed_z: 30.0, ..Default::default() };
        let res = generate_relief(&op, &tool, None, &r, &Material::default(), None);
        let cuts: Vec<(f64, f64)> = res.paths.iter().flat_map(|p| p.moves.iter()).filter_map(|m| Some((m.x?, m.z?))).collect();
        let left = cuts.iter().filter(|(x, _)| *x < 1.8).map(|c| c.1).fold(0.0, f64::min);
        let right = cuts.iter().filter(|(x, _)| *x > 2.2).map(|c| c.1).fold(0.0, f64::min);
        assert!(left < -0.28 && right > -0.02, "dark half deepest {left}, white half {right}");
    }
}

#[cfg(test)]
mod project_tests {
    use super::*;

    #[test]
    fn a_projected_toolpath_rides_the_model_surface() {
        use crate::job::{Move, Toolpath};
        // a flat-topped model 0.5 deep whose right half is all the way down
        let r = Relief { id: 1, x: 0.0, y: 0.0, w: 4.0, h: 2.0, depth: 0.5, nx: 4, ny: 2, height: vec![1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0], ..Default::default() };
        let mut tp = vec![Toolpath { moves: vec![Move::rapid(Some(0.5), Some(1.0), Some(0.2)), Move::linear(None, None, Some(-0.1)), Move::linear(Some(3.5), None, None), Move::rapid(None, None, Some(0.2))], ..Default::default() }];
        project_paths(&mut tp, &r);
        let z_at = |x: f64| tp[0].moves.iter().filter(|m| m.x.is_some_and(|mx| (mx - x).abs() < 0.011) && m.z.is_some()).map(|m| m.z.unwrap()).next();
        assert!((z_at(1.0).unwrap() + 0.1).abs() < 1e-9, "on the top: as drawn");
        assert!((z_at(3.0).unwrap() + 0.6).abs() < 1e-9, "down in the low half: 0.1 below its surface");
        assert_eq!(tp[0].moves.last().unwrap().z, Some(0.2), "the lift stays");
    }
}

#[cfg(test)]
mod taper_rest_tests {
    use super::*;
    use crate::document::{Document, Op, OpType, Tool, ToolType};

    fn taper() -> Tool {
        Tool { kind: ToolType::TaperedBall, diameter: 0.25, radius: 0.25 / 25.4, angle: 5.12, stepdown: 0.1, feed_xy: 60.0, feed_z: 20.0, ..Default::default() }
    }

    #[test]
    fn the_tapered_profile_is_smooth_where_the_ball_meets_the_cone_and_reach_undoes_it() {
        let t = taper();
        let (r, a) = (t.radius, 5.12_f64.to_radians());
        let d = r * a.cos();
        let (lo, hi) = (t.ball_rise(d - 1e-10), t.ball_rise(d + 1e-10));
        assert!((lo - hi).abs() < 1e-8, "no step: {lo} {hi}");
        let slope = |x: f64| (t.ball_rise(x + 1e-9) - t.ball_rise(x - 1e-9)) / 2e-9;
        assert!((slope(d - 1e-7) - slope(d + 1e-7)).abs() < 0.05 * slope(d + 1e-5), "no kink");
        for h in [0.001, 0.005, 0.05, 0.3] {
            assert!((t.ball_rise(t.reach(h)) - h).abs() < 1e-9, "{h}");
        }
        assert_eq!(t.reach(5.0), 0.125, "never wider than the shank");
        assert!((t.cut_diameter() - 0.5 / 25.4).abs() < 1e-12);
    }

    /// A 0.4 in tall wall: the tapered bit never cuts into it, while a straight ball the size of its tip would come
    /// right down beside it (its cone would gouge the wall).
    #[test]
    fn a_tapered_ball_nose_keeps_its_cone_out_of_steep_walls() {
        // left half low (0), right half the surface (1): a vertical wall at x = 1
        let r = Relief { id: 1, x: 0.0, y: 0.0, w: 2.0, h: 0.5, depth: 0.4, nx: 400, ny: 100, height: (0..40_000).map(|i| if i % 400 < 200 { 0.0 } else { 1.0 }).collect(), ..Default::default() };
        let t = taper();
        let straight = Tool { kind: ToolType::BallNose, diameter: t.cut_diameter(), ..Default::default() };
        let (x, y) = (0.985, 0.25); // 0.015 in from the wall
        let zt = tip_height(&r, &t, x, y, 0.0);
        assert!(tip_height(&r, &straight, x, y, 0.0) < -0.39, "the straight tip reaches the floor");
        assert!(zt > -0.15, "the cone holds the tapered bit up: {zt}");
        // nowhere under the bit is the cutter below the model
        let (cw, ch) = (r.w / r.nx as f64, r.h / r.ny as f64);
        for iy in 0..r.ny {
            for ix in 0..r.nx {
                let (cx, cy) = (r.x + (ix as f64 + 0.5) * cw, r.y + (iy as f64 + 0.5) * ch);
                let d = (cx - x).hypot(cy - y);
                if d <= 0.125 {
                    assert!(zt + t.ball_rise(d) >= -r.depth * (1.0 - r.at(ix, iy) as f64) - 1e-9, "gouge at {cx},{cy}");
                }
            }
        }
    }

    /// A board with three narrow grooves (0.06 in wide): a 1/4 ball nose can't reach their bottoms; a 1/16 ball nose
    /// rest pass then cuts only in and next to the grooves, and nothing over the flats.
    fn grooves() -> (Document, f64) {
        let (nx, ny, w) = (500, 500, 5.0);
        let cell = w / nx as f64;
        let groove = |x: f64| [1.5, 2.5, 3.5].iter().any(|g| (x - g).abs() < 0.03);
        let height: Vec<f32> = (0..nx * ny).map(|i| if groove(((i % nx) as f64 + 0.5) * cell) { 0.0 } else { 1.0 }).collect();
        let mut d = Document::default();
        d.material.w = 6.0;
        d.material.h = 6.0;
        d.reliefs.push(Relief { id: 1, x: 0.5, y: 0.5, w, h: w, depth: 0.25, nx, ny, height, ..Default::default() });
        d.next_relief_id = 2;
        d.tools.clear();
        d.tools.push(Tool { kind: ToolType::BallNose, diameter: 0.25, stepdown: 0.1, feed_xy: 100.0, feed_z: 30.0, ..Default::default() });
        d.tools.push(Tool { kind: ToolType::BallNose, diameter: 0.0625, stepdown: 0.05, feed_xy: 60.0, feed_z: 20.0, ..Default::default() });
        d.ops.push(Op { kind: OpType::Relief, name: "Finish".into(), relief: 1, tool: 0, stepover: 0.1, ..Default::default() });
        d.ops.push(Op { kind: OpType::Relief, name: "Details".into(), relief: 1, tool: 1, stepover: 0.1, rest: true, ..Default::default() });
        (d, 0.5)
    }

    fn cut_xs(paths: &[crate::job::Toolpath]) -> Vec<f64> {
        paths.iter().flat_map(|p| p.moves.iter()).filter(|m| m.kind == crate::job::MoveType::Linear && m.z.is_some_and(|z| z < -0.01)).filter_map(|m| m.x).collect()
    }

    #[test]
    fn rest_machining_cuts_only_what_the_bigger_bit_left() {
        let (d, x0) = grooves();
        let g = crate::pipeline::regenerate(&d);
        assert!(g.warnings.iter().all(|w| !w.contains("rest machining")), "{:?}", g.warnings);
        let rest = &g.paths[1];
        let xs = cut_xs(rest);
        assert!(!xs.is_empty(), "the rest pass cuts");
        // every cutting move is within a bit or so of a groove: the flats between them are left alone
        for x in &xs {
            let near = [1.5, 2.5, 3.5].iter().map(|g| (x - (g + x0)).abs()).fold(f64::MAX, f64::min);
            assert!(near < 0.2, "cut over a flat at x = {x}");
        }
        // and all three grooves get cut
        for g in [1.5, 2.5, 3.5] {
            assert!(xs.iter().any(|x| (x - (g + x0)).abs() < 0.03), "groove at {g}");
        }
        let mut full = d.clone();
        full.ops[1].rest = false;
        let all = crate::pipeline::regenerate(&full);
        assert!(g.minutes[1] * 3.0 < all.minutes[1], "far less time than a full pass: {} vs {} min", g.minutes[1], all.minutes[1]);
    }

    #[test]
    fn rest_machining_with_nothing_before_it_warns_and_cuts_everything() {
        let (mut d, _) = grooves();
        d.ops.remove(0);
        let g = crate::pipeline::regenerate(&d);
        assert!(g.warnings.iter().any(|w| w.contains("rest machining needs an earlier 3D toolpath")), "{:?}", g.warnings);
        let mut plain = d.clone();
        plain.ops[0].rest = false;
        assert_eq!(g.paths[0], crate::pipeline::regenerate(&plain).paths[0]);
        // the switch survives a save
        let back = crate::project::load_project(&crate::project::save_project(&d)).unwrap();
        assert!(back.ops[0].rest);
        let mut t = d.clone();
        t.tools[0] = taper();
        let back = crate::project::load_project(&crate::project::save_project(&t)).unwrap();
        assert_eq!((back.tools[0].kind, back.tools[0].radius, back.tools[0].angle), (ToolType::TaperedBall, t.tools[0].radius, 5.12));
    }
}
