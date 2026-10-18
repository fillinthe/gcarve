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

/// STL (binary or ASCII) from the file's bytes (the engine does no file I/O: the caller reads the file).
pub fn load_stl(data: &[u8]) -> Result<Vec<Triangle>, String> {
    let mut out = Vec::new();
    // binary: 80-byte header + uint32 count + 50 bytes per triangle; ASCII starts with "solid" and has "facet" words
    let ascii = data.starts_with(b"solid") && data.windows(5).any(|w| w == b"facet");
    if !ascii && data.len() >= 84 {
        let n = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
        if 84 + n * 50 == data.len() {
            out.reserve(n);
            for i in 0..n {
                let f = |k: usize| {
                    let o = 84 + i * 50 + 12 + k * 4;
                    f32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as f64
                };
                let p = |k: usize| Pt3 { x: f(k), y: f(k + 1), z: f(k + 2) };
                out.push(Triangle { a: p(0), b: p(3), c: p(6) });
            }
            return Ok(out);
        }
    }
    let text = String::from_utf8_lossy(data);
    let mut tok = text.split_ascii_whitespace();
    let mut verts: Vec<Pt3> = Vec::new();
    'parse: while let Some(t) = tok.next() {
        if t == "vertex" {
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
    if out.is_empty() {
        return Err("no triangles in the STL file".into());
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
    let rad = t.diameter / 2.0;
    let ball = t.kind == ToolType::BallNose;
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
                rad - (rad * rad - d * d).max(0.0).sqrt()
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
fn raster(r: &Relief, m: &Material, along_x: bool, sample: f64, inside_clip: &dyn Fn(f64, f64) -> bool, t: &Tool, step_frac: f64, tp: &mut Toolpath, allowance: f64, floor_z: f64, roughing: bool) {
    let step = 0.002_f64.max(clampf(step_frac, 0.02, 1.0) * t.diameter);
    let rad = t.diameter / 2.0;
    // cover the relief plus a cutter radius so edges get finished
    let mut u0 = (if along_x { r.x } else { r.y }) - rad;
    let mut u1 = (if along_x { r.x + r.w } else { r.y + r.h }) + rad;
    let mut v0 = (if along_x { r.y } else { r.x }) - rad;
    let mut v1 = (if along_x { r.y + r.h } else { r.x + r.w }) + rad;
    u0 = u0.max(0.0);
    v0 = v0.max(0.0);
    u1 = u1.min(if along_x { m.w } else { m.h });
    v1 = v1.min(if along_x { m.h } else { m.w });
    let rows = 1.max(((v1 - v0) / step).ceil() as i32);
    let cols = 2.max(((u1 - u0) / sample).ceil() as i32);
    let mut first = true;
    let mut cur = Pt::default();
    let mut flush = |pts: &mut Vec<[f64; 3]>, tp: &mut Toolpath| {
        if pts.len() < 2 {
            pts.clear();
            return;
        }
        let st = pt(pts[0][0], pts[0][1]);
        if first || dist(cur, st) > sample * 1.5 {
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

/// op fields used: stepover (finish, fraction of diameter), raster_angle (0 along X, 90 along Y), rough/rough_stepdown/
/// rough_stepover (layered roughing with rough_tool, leaving op.allowance), name.
/// `boundary`: optional closed vectors; when given, only the area inside them is machined.
pub fn generate_relief(op: &Op, finish_tool: &Tool, rough_tool: Option<&Tool>, r: &Relief, m: &Material, boundary: Option<&[Contour]>) -> CamResult {
    let mut res = CamResult::default();
    // boundary polygons (flattened) with bounding boxes for a cheap inside test per raster sample
    let mut clip: Vec<(Vec<Pt>, Rect)> = Vec::new();
    for c in boundary.unwrap_or(&[]) {
        if !c.closed {
            continue;
        }
        let pts = flatten(c, 0.002);
        if pts.len() < 3 {
            continue;
        }
        let mut b = Rect::default();
        for &q in &pts {
            b.add(q);
        }
        clip.push((pts, b));
    }
    let inside_clip = |x: f64, y: f64| {
        if clip.is_empty() {
            return true;
        }
        let mut n = 0;
        for (p, b) in &clip {
            if x < b.x0 || x > b.x1 || y < b.y0 || y > b.y1 {
                continue;
            }
            let mut inn = false;
            let mut j = p.len() - 1;
            for i in 0..p.len() {
                if (p[i].y > y) != (p[j].y > y) && x < (p[j].x - p[i].x) * (y - p[i].y) / (p[j].y - p[i].y) + p[i].x {
                    inn = !inn;
                }
                j = i;
            }
            if inn {
                n += 1;
            }
        }
        n % 2 == 1
    };
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
        let mut rough = Toolpath { name: format!("{} [rough]", op.name), tool: op.clear_tool, feed_xy: rt.feed_xy, feed_z: rt.feed_z, moves: Vec::new() };
        let sd = if op.rough_stepdown > 0.0 { op.rough_stepdown } else { rt.stepdown };
        let mut z = -sd;
        while z > -r.depth - 1e-9 {
            raster(r, m, along_x, sample, &inside_clip, rt, op.rough_stepover, &mut rough, op.allowance, z.max(-r.depth), true);
            z -= sd;
        }
        if !rough.moves.is_empty() {
            res.paths.push(rough);
        }
    }
    let mut fin = Toolpath { name: op.name.clone(), tool: -1, feed_xy: finish_tool.feed_xy, feed_z: finish_tool.feed_z, moves: Vec::new() };
    raster(r, m, along_x, sample, &inside_clip, finish_tool, op.stepover, &mut fin, 0.0, 0.0, false);
    res.paths.push(fin);
    res
}
