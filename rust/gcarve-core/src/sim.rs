//! Material removal simulation (C++ `core/sim.*`): a height-field of the board (depth below the surface per cell)
//! carved by toolpaths.

use crate::cam::{Material, Tool, ToolType};
use crate::job::{MoveType, Toolpath};
use std::f64::consts::PI;

#[derive(Clone, Debug, PartialEq)]
pub struct HeightField {
    pub nx: i32,
    pub ny: i32,
    /// cell size, inches
    pub res: f64,
    /// world position of cell (0,0)'s corner
    pub x0: f64,
    pub y0: f64,
    /// row-major, y up: index = iy * nx + ix; depth >= 0 below the surface
    pub depth: Vec<f32>,
}
impl Default for HeightField {
    fn default() -> Self {
        HeightField { nx: 0, ny: 0, res: 0.02, x0: 0.0, y0: 0.0, depth: Vec::new() }
    }
}
impl HeightField {
    pub fn at(&self, ix: i32, iy: i32) -> f32 {
        self.depth[iy as usize * self.nx as usize + ix as usize]
    }
    /// nearest cell, 0 outside
    pub fn sample(&self, x: f64, y: f64) -> f32 {
        let ix = ((x - self.x0) / self.res).floor() as i32;
        let iy = ((y - self.y0) / self.res).floor() as i32;
        if ix < 0 || iy < 0 || ix >= self.nx || iy >= self.ny {
            return 0.0;
        }
        self.at(ix, iy)
    }
}

/// A toolpath and the tool that cuts it.
pub type SimPath<'a> = (&'a Toolpath, &'a Tool);

/// Inclusive dirty cell rectangle `[ix0, iy0, ix1, iy1]` (ix0 > ix1 when nothing was cut).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRect {
    pub ix0: i32,
    pub iy0: i32,
    pub ix1: i32,
    pub iy1: i32,
}
impl Default for CellRect {
    fn default() -> Self {
        CellRect { ix0: 1, iy0: 1, ix1: 0, iy1: 0 }
    }
}
impl CellRect {
    pub fn is_empty(&self) -> bool {
        self.ix0 > self.ix1 || self.iy0 > self.iy1
    }
}

/// Stamp the tool footprint centred at (cx, cy) cutting to `d` below the surface.
fn stamp(hf: &mut HeightField, t: &Tool, cx: f64, cy: f64, d: f64) {
    if d <= 0.0 {
        return;
    }
    let mut r = t.diameter / 2.0;
    let tan_half = if t.kind == ToolType::VBit { (t.angle / 2.0 * PI / 180.0).tan() } else { 0.0 };
    if t.kind == ToolType::VBit {
        r = r.min(d * tan_half); // the cone only reaches this far at this depth
    }
    let ix0 = 0.max(((cx - r - hf.x0) / hf.res).floor() as i32);
    let ix1 = (hf.nx - 1).min(((cx + r - hf.x0) / hf.res).floor() as i32);
    let iy0 = 0.max(((cy - r - hf.y0) / hf.res).floor() as i32);
    let iy1 = (hf.ny - 1).min(((cy + r - hf.y0) / hf.res).floor() as i32);
    for iy in iy0..=iy1 {
        let y = hf.y0 + (iy as f64 + 0.5) * hf.res;
        let row = iy as usize * hf.nx as usize;
        for ix in ix0..=ix1 {
            let x = hf.x0 + (ix as f64 + 0.5) * hf.res;
            let dist = (x - cx).hypot(y - cy);
            if dist > r {
                continue;
            }
            let cut = match t.kind {
                ToolType::VBit => d - dist / tan_half,
                ToolType::BallNose => d - (r - (r * r - dist * dist).max(0.0).sqrt()),
                // the wood keeps a quarter round: vertical at the tip, level with the surface at the rim
                ToolType::RoundOver => {
                    let rr = if t.radius > 0.0 { t.radius.min(r) } else { r };
                    let x = (dist - (r - rr)).max(0.0); // 0 across the flat tip
                    d - (rr * rr - (rr - x) * (rr - x)).max(0.0).sqrt()
                }
                ToolType::EndMill => d,
            };
            let cell = &mut hf.depth[row + ix as usize];
            if cut > *cell as f64 {
                *cell = cut as f32;
            }
        }
    }
}

/// Incremental use (animated playback): allocate the grid for these paths (≈ `cells` cells), then stamp moves one at
/// a time with `sim_stamp`.
pub fn sim_grid(paths: &[SimPath], m: &Material, cells: usize) -> HeightField {
    // grid only where cutting happens (plus the widest tool), clipped to the board: resolution follows the design
    let (mut bx0, mut by0, mut bx1, mut by1, mut max_r) = (1e300_f64, 1e300_f64, -1e300_f64, -1e300_f64, 0.0_f64);
    for (path, tool) in paths {
        max_r = max_r.max(tool.diameter / 2.0);
        for mv in &path.moves {
            if let (Some(x), Some(y)) = (mv.x, mv.y) {
                bx0 = bx0.min(x);
                bx1 = bx1.max(x);
                by0 = by0.min(y);
                by1 = by1.max(y);
            }
        }
    }
    if bx0 > bx1 {
        (bx0, by0, bx1, by1) = (0.0, 0.0, m.w, m.h);
    }
    bx0 = 0.0_f64.max(bx0 - max_r - 0.05);
    by0 = 0.0_f64.max(by0 - max_r - 0.05);
    bx1 = m.w.min(bx1 + max_r + 0.05);
    by1 = m.h.min(by1 + max_r + 0.05);
    let w = 1e-3_f64.max(bx1 - bx0);
    let h = 1e-3_f64.max(by1 - by0);
    let res = 0.002_f64.max((w * h / cells as f64).sqrt());
    let nx = 1.max((w / res).ceil() as i32);
    let ny = 1.max((h / res).ceil() as i32);
    HeightField { nx, ny, res, x0: bx0, y0: by0, depth: vec![0.0; nx as usize * ny as usize] }
}

/// Stamps the straight move from (x0,y0,z0) to (x1,y1,z1); only the part below the surface cuts. Returns the dirty
/// cell rectangle.
#[allow(clippy::too_many_arguments)]
pub fn sim_stamp(hf: &mut HeightField, tool: &Tool, x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> CellRect {
    if z0 >= 0.0 && z1 >= 0.0 {
        return CellRect::default();
    }
    // stamp spacing: half a cell for tiny bits, up to r/8 for big ones (the scallop between stamps, s^2/8r, stays < 0.001 in)
    let step = crate::cam::clampf(tool.diameter / 16.0, hf.res * 0.5, hf.res * 4.0);
    let l = (x1 - x0).hypot(y1 - y0);
    let n = 1.max((l / step).ceil() as i32);
    for k in 0..=n {
        let t = k as f64 / n as f64;
        stamp(hf, tool, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, -(z0 + (z1 - z0) * t));
    }
    let rad = tool.diameter / 2.0 + hf.res;
    let cl = |v: f64, n: i32| (v as i32).clamp(0, n - 1);
    CellRect {
        ix0: cl(((x0.min(x1) - rad - hf.x0) / hf.res).floor(), hf.nx),
        ix1: cl(((x0.max(x1) + rad - hf.x0) / hf.res).ceil(), hf.nx),
        iy0: cl(((y0.min(y1) - rad - hf.y0) / hf.res).floor(), hf.ny),
        iy1: cl(((y0.max(y1) + rad - hf.y0) / hf.res).ceil(), hf.ny),
    }
}

/// `cells`: target cell count (resolution adapts to the cut region; the C++ default is 1.5M).
/// ponytail: CPU stamping; a GPU/dexel model when boards get huge.
pub fn simulate(paths: &[SimPath], m: &Material, cells: usize) -> HeightField {
    let mut hf = sim_grid(paths, m, cells);
    for (path, tool) in paths {
        let (mut have, mut px, mut py, mut pz) = (false, 0.0, 0.0, 0.0);
        for mv in &path.moves {
            let (x, y, z) = (mv.x.unwrap_or(px), mv.y.unwrap_or(py), mv.z.unwrap_or(pz));
            // arcs are fitted at export time only; generated paths are linear, so straight segments are exact here
            if have && mv.kind != MoveType::Rapid {
                sim_stamp(&mut hf, tool, px, py, pz, x, y, z);
            }
            (px, py, pz, have) = (x, y, z, true);
        }
    }
    hf
}

/// Wood colour (rgb 0..1, before lighting) of a cell cut `d` deep into a board `t` thick: the uncut surface is a
/// darker weathered tan, cut faces are fresh light wood that darkens with depth.
pub fn wood(d: f64, t: f64) -> (f64, f64, f64) {
    if d > 1e-5 {
        let k = (d / t.max(1e-6)).min(1.0);
        (0.96 - 0.45 * k, 0.84 - 0.45 * k, 0.62 - 0.38 * k)
    } else {
        (0.62, 0.49, 0.33)
    }
}

/// Shade only the cells inside the rectangle (expanded by one for the normals) into an existing RGBA image of the field.
pub fn shade_into(hf: &HeightField, m: &Material, img: &mut Vec<u8>, r: CellRect) {
    let size = hf.nx as usize * hf.ny as usize * 4;
    if img.len() != size {
        *img = vec![0; size];
    }
    if r.is_empty() {
        return;
    }
    let (ax, bx) = (0.max(r.ix0 - 1), (hf.nx - 1).min(r.ix1 + 1));
    let (ay, by) = (0.max(r.iy0 - 1), (hf.ny - 1).min(r.iy1 + 1));
    // light from upper-left, slightly above
    let (mut lx, mut ly, mut lz): (f64, f64, f64) = (-0.45, 0.6, 0.66);
    let ll: f64 = (lx * lx + ly * ly + lz * lz).sqrt();
    lx /= ll;
    ly /= ll;
    lz /= ll;
    let depth_at = |ix: i32, iy: i32| hf.at(ix.clamp(0, hf.nx - 1), iy.clamp(0, hf.ny - 1)) as f64;
    let byte = |v: f64| (v as i32).clamp(0, 255) as u8;
    for iy in ay..=by {
        for ix in ax..=bx {
            let d = depth_at(ix, iy);
            let o = ((hf.ny - 1 - iy) as usize * hf.nx as usize + ix as usize) * 4; // row 0 = top
            if d >= m.t - 1e-6 {
                img[o + 3] = 0; // cut through: hole
                continue;
            }
            // surface normal from the depth gradient (depth grows downward, so height = -depth)
            let gx = (depth_at(ix + 1, iy) - depth_at(ix - 1, iy)) / (2.0 * hf.res);
            let gy = (depth_at(ix, iy + 1) - depth_at(ix, iy - 1)) / (2.0 * hf.res);
            let nl = (gx * gx + gy * gy + 1.0).sqrt();
            let (nx, ny, nz) = (gx / nl, gy / nl, 1.0 / nl);
            let lambert = (nx * lx + ny * ly + nz * lz).max(0.0);
            let light = 0.35 + 0.65 * lambert;
            let (br, bg, bb) = wood(d, m.t);
            img[o] = byte(br * light * 255.0);
            img[o + 1] = byte(bg * light * 255.0);
            img[o + 2] = byte(bb * light * 255.0);
            img[o + 3] = 255;
        }
    }
}

/// Shaded top-down render: RGBA8, row 0 = top (max y). Through-cuts come out transparent.
pub fn shade(hf: &HeightField, m: &Material) -> Vec<u8> {
    let mut img = Vec::new();
    shade_into(hf, m, &mut img, CellRect { ix0: 0, iy0: 0, ix1: hf.nx - 1, iy1: hf.ny - 1 });
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_over_bit_leaves_a_quarter_round() {
        // 1 in bit (radius 0.5) plunged 0.5 deep at the middle of a 2 in grid, 0.01 in cells
        let mut hf = HeightField { nx: 200, ny: 200, res: 0.01, x0: 0.0, y0: 0.0, depth: vec![0.0; 40_000] };
        let t = Tool { kind: ToolType::RoundOver, diameter: 1.0, ..Default::default() };
        stamp(&mut hf, &t, 1.0, 1.0, 0.5);
        let at = |dist: f64| hf.depth[100 * 200 + (100.0 + dist / 0.01) as usize] as f64;
        // full depth at the tip, half the radius out the round is at 0.5 - sqrt(0.25 - 0.0625), nothing at the rim
        assert!(at(0.0) > 0.39, "{}", at(0.0)); // first cell centre is 0.005 off the tip, where the wall is steep
        assert!((at(0.25) - (0.5 - (0.25f64 - 0.245 * 0.245).sqrt())).abs() < 0.01, "{}", at(0.25));
        assert!(at(0.45) < 0.01 && at(0.6) == 0.0);
        // a 0.25 radius on the same bit: flat tip 0.5 across at full depth, the round outside it
        hf.depth.fill(0.0);
        stamp(&mut hf, &Tool { radius: 0.25, ..t }, 1.0, 1.0, 0.5);
        let at = |dist: f64| hf.depth[100 * 200 + (100.0 + dist / 0.01) as usize] as f64;
        assert!(at(0.0) == 0.5 && at(0.2) == 0.5 && at(0.4) < 0.3 && at(0.4) > 0.25, "{} {}", at(0.2), at(0.4));
    }
}
