//! Toolpath preview, built once per regenerate (Qt Canvas::regenerate preview part + canvas_view.cpp colours):
//! rapids as one dashed chain list, cuts as polylines in 8 depth buckets (shallow = light cyan, deep = indigo), and
//! the cut-width bands rasterised into one texture (like the simulation), never re-tessellated per frame.
//! A V-bit's band is its width at that pass depth.

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use gcarve_core::document::{Document, ToolType};
use gcarve_core::geom::{pt, Pt};
use gcarve_core::job::MoveType;
use gcarve_core::pipeline::Generated;

const MAX_CELLS: f64 = 2_500_000.0; // ponytail: fixed raster, blurs when zoomed far in; per-zoom re-raster if that bothers anyone
pub const BUCKETS: usize = 8; // Qt kDepthBuckets

/// QColor::fromHsvF (sRGB, like Qt)
fn hsv(h: f64, s: f64, v: f64, a: f64) -> Color32 {
    let h6 = (h.fract() * 6.0).rem_euclid(6.0);
    let c = v * s;
    let x = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
    let (r, g, b) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let q = |f: f64| ((f + m) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(q(r), q(g), q(b), (a * 255.0).round() as u8)
}

fn t_of(b: usize) -> f64 {
    b as f64 / (BUCKETS - 1) as f64
}

/// cut line colour of a depth bucket
pub fn cut_color(b: usize) -> Color32 {
    let t = t_of(b);
    hsv(0.5 + 0.2 * t, 0.55 + 0.35 * t, 1.0 - 0.45 * t, 0.9)
}

/// translucent band colour of a depth bucket (material removed, darker when deeper)
fn band_color(b: usize) -> Color32 {
    let t = t_of(b);
    hsv(0.5 + 0.2 * t, 0.5, 0.9 - 0.35 * t, 0.28)
}

#[derive(Default)]
pub struct Bands {
    pub texture: Option<TextureHandle>,
    /// board-space x0, y0, w, h of the texture
    pub rect: Option<[f64; 4]>,
    /// rapid moves (dashed red), one chain per run of consecutive rapids
    pub rapids: Vec<Vec<Pt>>,
    /// cut polylines per depth bucket
    pub cuts: [Vec<Vec<Pt>>; BUCKETS],
    key: (u64, u64, usize, u64),
    built: bool,
}

/// segment (a → b) with its bucket and swept radius
struct Seg {
    a: Pt,
    b: Pt,
    r: f64,
    bucket: usize,
}

impl Bands {
    /// `seq`: bumped by the app on every regenerate (live previews change the paths without a document change)
    /// `only`: which toolpaths to draw (by index), None = all
    pub fn update(&mut self, ctx: &egui::Context, doc: &Document, gen: &Generated, rev: u64, seq: u64, only: Option<&[bool]>) {
        // the paths may arrive a frame after the document change: key on their size too
        let moves: usize = gen.paths.iter().flatten().map(|tp| tp.moves.len()).sum();
        let picked = only.map_or(u64::MAX, |o| o.iter().enumerate().fold(0u64, |h, (i, &w)| if w { h.wrapping_mul(31).wrapping_add(i as u64 + 1) } else { h }));
        let key = (rev, seq, moves, picked);
        if self.built && self.key == key {
            return;
        }
        self.key = key;
        self.built = true;
        self.rapids.clear();
        for c in &mut self.cuts {
            c.clear();
        }
        let max_depth = gen.paths.iter().flatten().flat_map(|tp| &tp.moves).filter_map(|m| m.z).map(|z| -z).fold(0.0_f64, f64::max);
        let bucket = |d: f64| if max_depth > 0.0 { ((d / max_depth * (BUCKETS - 1) as f64 + 0.5) as i64).clamp(0, BUCKETS as i64 - 1) as usize } else { 0 };
        let mut segs: Vec<Seg> = Vec::new();
        // like Qt, the path runs on across toolpaths: the rapid into each toolpath is drawn too
        let mut last: Option<Pt> = None;
        let mut last_z = 0.0;
        let mut last_kind = None::<bool>; // Some(true) = in a rapid chain, Some(false) = in a cut chain
        let mut last_bucket = usize::MAX;
        // a new toolpath being set up in its dialog comes last, with no op in the document yet
        for (i, paths) in gen.paths.iter().enumerate() {
            let (visible, op_tool) = doc.ops.get(i).map_or((true, gen.draft_tool), |op| (op.visible, op.tool));
            if !visible || only.is_some_and(|o| !o.get(i).copied().unwrap_or(false)) {
                continue;
            }
            for tp in paths {
                let ti = if tp.tool >= 0 { tp.tool } else { op_tool };
                let tool = doc.tools.get(ti.max(0) as usize).filter(|_| ti >= 0);
                let mut have_here = false; // band chains restart per toolpath (Qt `h`)
                for m in &tp.moves {
                    if m.x.is_none() && m.y.is_none() {
                        if let Some(z) = m.z {
                            last_z = z;
                        }
                        continue;
                    }
                    let prev = last.unwrap_or(pt(0.0, 0.0));
                    let q = pt(m.x.unwrap_or(prev.x), m.y.unwrap_or(prev.y));
                    let z = m.z.unwrap_or(last_z);
                    if let Some(p) = last {
                        if m.kind == MoveType::Rapid {
                            if q != p {
                                if last_kind != Some(true) || self.rapids.last().and_then(|c| c.last()) != Some(&p) {
                                    self.rapids.push(vec![p]);
                                }
                                self.rapids.last_mut().unwrap().push(q);
                                last_kind = Some(true);
                            }
                        } else {
                            let d = (-z).max(-last_z);
                            let b = bucket(d);
                            let pts: Vec<Pt> = match m.kind {
                                MoveType::ArcCw | MoveType::ArcCcw => crate::canvas::arc_points(p, q, m.i, m.j, m.kind == MoveType::ArcCcw),
                                _ => vec![p, q],
                            };
                            let chain = &mut self.cuts[b];
                            if last_kind != Some(false) || last_bucket != b || chain.last().and_then(|c| c.last()) != Some(&p) {
                                chain.push(vec![p]);
                            }
                            chain.last_mut().unwrap().extend_from_slice(&pts[1..]);
                            last_kind = Some(false);
                            last_bucket = b;
                            if let (Some(t), true) = (tool, have_here) {
                                let w = if t.kind == ToolType::VBit { (2.0 * d * (t.angle.to_radians() / 2.0).tan()).max(0.01) } else { t.diameter };
                                for s in pts.windows(2) {
                                    segs.push(Seg { a: s[0], b: s[1], r: w / 2.0, bucket: b });
                                }
                            }
                        }
                    }
                    last = Some(q);
                    last_z = z;
                    have_here = true;
                }
            }
        }
        self.raster(ctx, &segs);
    }

    fn raster(&mut self, ctx: &egui::Context, segs: &[Seg]) {
        if segs.is_empty() {
            self.texture = None;
            self.rect = None;
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for s in segs {
            x0 = x0.min(s.a.x.min(s.b.x) - s.r);
            y0 = y0.min(s.a.y.min(s.b.y) - s.r);
            x1 = x1.max(s.a.x.max(s.b.x) + s.r);
            y1 = y1.max(s.a.y.max(s.b.y) + s.r);
        }
        // also no side over 4096 px: a long thin set of cuts (one straight line) gave a 1 × 17265 texture, which
        // WebGL refuses and the web app panicked (error log 2026-10-17)
        let res = ((x1 - x0) * (y1 - y0) / MAX_CELLS).sqrt().max((x1 - x0).max(y1 - y0) / 4000.0).max(1e-4);
        let (nx, ny) = (((x1 - x0) / res).ceil() as usize + 1, ((y1 - y0) / res).ceil() as usize + 1);
        let mut mask = vec![0u8; nx * ny]; // bit per depth bucket that sweeps the cell
        for s in segs {
            stamp_capsule(&mut mask, nx, ny, x0, y0, res, (s.a.x, s.a.y), (s.b.x, s.b.y), s.r, 1 << s.bucket);
        }
        // every bucket mix composited once: Qt fills each bucket's swept outline on top of the previous ones
        let palette: Vec<Color32> = (0..256usize).map(|m| (0..BUCKETS).filter(|b| m & (1 << b) != 0).fold(Color32::TRANSPARENT, |acc, b| over(band_color(b), acc))).collect();
        let mut img = ColorImage::new([nx, ny], vec![Color32::TRANSPARENT; nx * ny]);
        for iy in 0..ny {
            let src = iy * nx;
            let dst = (ny - 1 - iy) * nx; // texture rows top first, board y up
            for ix in 0..nx {
                img.pixels[dst + ix] = palette[mask[src + ix] as usize];
            }
        }
        self.texture = Some(ctx.load_texture("bands", img, TextureOptions::LINEAR));
        self.rect = Some([x0, y0, nx as f64 * res, ny as f64 * res]);
    }
}

/// `top` over `bottom` (both premultiplied Color32)
fn over(top: Color32, bottom: Color32) -> Color32 {
    let k = 255 - top.a() as u32;
    let mix = |t: u8, b: u8| (t as u32 + (b as u32 * k + 127) / 255).min(255) as u8;
    Color32::from_rgba_premultiplied(mix(top.r(), bottom.r()), mix(top.g(), bottom.g()), mix(top.b(), bottom.b()), mix(top.a(), bottom.a()))
}

/// Set `bit` in every cell whose centre is within r of segment a-b.
#[allow(clippy::too_many_arguments)]
fn stamp_capsule(mask: &mut [u8], nx: usize, ny: usize, x0: f64, y0: f64, res: f64, a: (f64, f64), b: (f64, f64), r: f64, bit: u8) {
    let ix0 = (((a.0.min(b.0) - r - x0) / res).floor().max(0.0)) as usize;
    let iy0 = (((a.1.min(b.1) - r - y0) / res).floor().max(0.0)) as usize;
    let ix1 = ((((a.0.max(b.0) + r - x0) / res).ceil()) as usize).min(nx - 1);
    let iy1 = ((((a.1.max(b.1) + r - y0) / res).ceil()) as usize).min(ny - 1);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let r2 = r * r;
    for iy in iy0..=iy1 {
        let py = y0 + (iy as f64 + 0.5) * res;
        let row = iy * nx;
        for ix in ix0..=ix1 {
            let cell = &mut mask[row + ix];
            if *cell & bit != 0 {
                continue;
            }
            let px = x0 + (ix as f64 + 0.5) * res;
            let t = if len2 > 0.0 { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let (ex, ey) = (px - (a.0 + t * dx), py - (a.1 + t * dy));
            if ex * ex + ey * ey <= r2 {
                *cell |= bit;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capsule_covers_width_only() {
        let (nx, ny, res) = (100, 100, 0.01);
        let mut o = vec![0u8; nx * ny];
        stamp_capsule(&mut o, nx, ny, 0.0, 0.0, res, (0.2, 0.5), (0.8, 0.5), 0.1, 4);
        let row = |y: f64| (y / res) as usize * nx;
        assert_eq!(o[row(0.5) + 50], 4); // on the line
        assert_eq!(o[row(0.58) + 50], 4); // inside the radius
        assert_eq!(o[row(0.65) + 50], 0); // outside
        assert_eq!(o[row(0.5) + 95], 0); // past the end cap
    }
    #[test]
    fn qt_depth_colours() {
        // QColor::fromHsvF(0.5, 0.55, 1.0, 0.9): shallow cut = light cyan
        let c = cut_color(0).to_srgba_unmultiplied();
        assert_eq!((c[0], c[1], c[2]), (115, 255, 255));
        assert_eq!(hsv(0.0, 1.0, 1.0, 1.0), Color32::from_rgb(255, 0, 0));
    }
}
