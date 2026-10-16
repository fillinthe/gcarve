//! Cut simulation playback (Qt app/simplayer.*): toolpaths flattened into timed segments (same speed model as
//! order::estimate_minutes), stamped incrementally into a height-field whose shaded image is a texture over the board.
//! Seeking backwards re-cuts from the start (the field only grows).

use eframe::egui::{self, ColorImage, TextureOptions};
use gcarve_core::document::{Document, Material, Tool};
use gcarve_core::job::{MoveType, Toolpath};
use gcarve_core::pipeline::{tool_for_cut, Generated};
use gcarve_core::sim::{shade_into, sim_grid, sim_stamp, CellRect, HeightField};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub x0: f64,
    pub y0: f64,
    pub z0: f64,
    pub x1: f64,
    pub y1: f64,
    pub z1: f64,
    pub tool: usize,
    pub rapid: bool,
    /// minutes
    pub t0: f64,
    pub t1: f64,
}

/// Flatten a toolpath into timed segments starting at `t` minutes (the machine's `Motion`, so the clock matches the
/// toolpath list and the export estimate); returns the end time.
pub fn segments(tp: &Toolpath, tool: usize, mut t: f64, motion: gcarve_core::order::Motion, out: &mut Vec<Seg>) -> f64 {
    let times = gcarve_core::order::move_minutes(tp, tp.feed_xy, tp.feed_z, motion);
    let (mut x, mut y, mut z, mut have) = (0.0, 0.0, 0.0, false);
    for (mv, &dt) in tp.moves.iter().zip(&times) {
        let (nx, ny, nz) = (mv.x.unwrap_or(x), mv.y.unwrap_or(y), mv.z.unwrap_or(z));
        if have {
            let rapid = mv.kind == MoveType::Rapid;
            out.push(Seg { x0: x, y0: y, z0: z, x1: nx, y1: ny, z1: nz, tool, rapid, t0: t, t1: t + dt });
            t += dt;
        }
        (x, y, z, have) = (nx, ny, nz, true);
    }
    t
}

/// "m:ss / m:ss"
pub fn time_text(pos: f64, total: f64) -> String {
    let fmt = |mins: f64| {
        let s = (mins * 60.0).round() as i64;
        format!("{}:{:02}", s / 60, s % 60)
    };
    format!("{} / {}", fmt(pos), fmt(total))
}

#[derive(Default)]
pub struct Player {
    pub segs: Vec<Seg>,
    tools: Vec<Tool>,
    pub hf: HeightField,
    rgba: Vec<u8>,
    pub material: Material,
    /// what the canvas draws: texture + its board rectangle
    pub view: crate::simview::SimView,
    pub total: f64,
    pub pos: f64,
    next: usize,
    partial: f64,
    pub playing: bool,
    /// minutes of machine time per real second; 0 = instant
    pub speed: f64,
    pub tool_pos: (f64, f64),
    pub tool_dia: f64,
    pub rev: u64,
    pub info: String,
    last_tick: Option<f64>,
    /// mesh consumers (3D view) rebuild when this moves
    pub frame: u64,
    /// carving to the finished part a slice per frame (`finish_step`), so the window keeps drawing a progress bar
    finishing: bool,
}

/// moves stamped per frame while finishing: big jobs take a few seconds either way, but the window stays alive
const FINISH_MOVES_PER_FRAME: usize = 3000;

impl Player {
    pub fn ready(&self) -> bool {
        !self.segs.is_empty() && !self.finishing
    }

    /// 0..1 while the finished part is being carved for display (the "Preparing" card), else None
    pub fn preparing(&self) -> Option<f32> {
        self.finishing.then(|| self.next as f32 / self.segs.len().max(1) as f32)
    }

    /// One slice of the carve to the finished part (call every frame; nothing to do once it is done).
    pub fn finish_step(&mut self, ctx: &egui::Context) {
        if !self.finishing {
            return;
        }
        let k = (self.next + FINISH_MOVES_PER_FRAME).min(self.segs.len());
        let to = if k >= self.segs.len() { self.total } else { self.segs[k - 1].t1 };
        self.advance_to(to, ctx);
        if self.pos >= self.total {
            self.finishing = false;
            self.frame += 1;
            self.refresh_info();
        }
        ctx.request_repaint();
    }

    /// Rebuild when the toolpaths changed (`rev`). Visible ops only; `complete` jumps to the finished part.
    pub fn update(&mut self, ctx: &egui::Context, doc: &Document, gen: &Generated, rev: u64, complete: bool) {
        if self.rev == rev {
            return;
        }
        self.rev = rev;
        self.playing = false;
        let eff: Vec<Tool> = (0..doc.tools.len()).map(|i| tool_for_cut(doc, i)).collect();
        self.segs.clear();
        self.tools.clear();
        let mut sps: Vec<(&Toolpath, usize)> = Vec::new();
        let mut t = 0.0;
        let motion = gcarve_core::pipeline::motion(doc);
        for (op, ps) in doc.ops.iter().zip(&gen.paths) {
            if !op.visible {
                continue;
            }
            for tp in ps {
                let ti = if tp.tool >= 0 { tp.tool } else { op.tool } as usize;
                let Some(tool) = eff.get(ti) else { continue };
                self.tools.push(tool.clone());
                t = segments(tp, self.tools.len() - 1, t, motion, &mut self.segs);
                sps.push((tp, self.tools.len() - 1));
            }
        }
        self.total = t;
        self.tool_dia = self.tools.iter().map(|t| t.diameter).fold(0.0, f64::max);
        if sps.is_empty() {
            self.view.texture = None;
            self.view.rect = None;
            self.hf = HeightField::default();
            self.info.clear();
            return;
        }
        self.material = doc.material.clone();
        let paths: Vec<(&Toolpath, &Tool)> = sps.iter().map(|(tp, i)| (*tp, &self.tools[*i])).collect();
        self.hf = sim_grid(&paths, &doc.material, 1_500_000);
        self.view.rect = Some([self.hf.x0, self.hf.y0, self.hf.nx as f64 * self.hf.res, self.hf.ny as f64 * self.hf.res]);
        self.rgba.clear();
        self.reset();
        self.view.texture = Some(ctx.load_texture("sim", self.image(), TextureOptions::LINEAR));
        self.finishing = complete; // carved a slice per frame by `finish_step`
        self.frame += 1;
        self.refresh_info();
    }

    fn refresh_info(&mut self) {
        let mut maxd = 0.0f32;
        let mut cut = 0usize;
        for &d in &self.hf.depth {
            if d > 0.0 {
                cut += 1;
            }
            maxd = maxd.max(d);
        }
        let est = time_text(self.pos, self.total);
        self.info = format!("{} path(s) at {:.3} in/cell · deepest cut {:.3} in · {:.1}% of the board touched · est. {}", self.tools.len(), self.hf.res, maxd, 100.0 * cut as f64 / self.hf.depth.len().max(1) as f64, est.rsplit(" / ").next().unwrap_or(""));
        if maxd as f64 > self.material.t + 1e-6 {
            self.info += &format!("  ⚠ cuts {:.3} in below the board", maxd as f64 - self.material.t);
        }
    }

    fn image(&self) -> ColorImage {
        ColorImage::from_rgba_unmultiplied([self.hf.nx as usize, self.hf.ny as usize], &self.rgba)
    }

    pub fn reset(&mut self) {
        self.finishing = false;
        self.hf.depth.iter_mut().for_each(|d| *d = 0.0);
        shade_into(&self.hf, &self.material, &mut self.rgba, CellRect { ix0: 0, iy0: 0, ix1: self.hf.nx - 1, iy1: self.hf.ny - 1 });
        self.pos = 0.0;
        self.next = 0;
        self.partial = 0.0;
        if let Some(s) = self.segs.first() {
            self.tool_pos = (s.x0, s.y0);
        }
    }

    /// Stamp everything between the current position and `minutes`; returns the dirty cell rect.
    pub fn advance(&mut self, minutes: f64) -> CellRect {
        let minutes = minutes.clamp(0.0, self.total);
        let mut dirty = CellRect::default();
        while self.next < self.segs.len() {
            let s = self.segs[self.next];
            let len = s.t1 - s.t0;
            let f1 = if len > 0.0 { ((minutes - s.t0) / len).clamp(0.0, 1.0) } else { 1.0 };
            if f1 <= self.partial && minutes < s.t1 {
                break;
            }
            let f0 = self.partial;
            if !s.rapid && f1 > f0 {
                let at = |f: f64| (s.x0 + (s.x1 - s.x0) * f, s.y0 + (s.y1 - s.y0) * f, s.z0 + (s.z1 - s.z0) * f);
                let (a, b) = (at(f0), at(f1));
                let r = sim_stamp(&mut self.hf, &self.tools[s.tool], a.0, a.1, a.2, b.0, b.1, b.2);
                if !r.is_empty() {
                    dirty = if dirty.is_empty() { r } else { CellRect { ix0: dirty.ix0.min(r.ix0), iy0: dirty.iy0.min(r.iy0), ix1: dirty.ix1.max(r.ix1), iy1: dirty.iy1.max(r.iy1) } };
                }
            }
            self.tool_pos = (s.x0 + (s.x1 - s.x0) * f1, s.y0 + (s.y1 - s.y0) * f1);
            if f1 >= 1.0 - 1e-12 {
                self.next += 1;
                self.partial = 0.0;
            } else {
                self.partial = f1;
                break;
            }
        }
        self.pos = minutes;
        dirty
    }

    fn advance_to(&mut self, minutes: f64, ctx: &egui::Context) {
        let r = self.advance(minutes);
        if r.is_empty() {
            return;
        }
        shade_into(&self.hf, &self.material, &mut self.rgba, r);
        let Some(tex) = &mut self.view.texture else { return };
        // the shaded image has row 0 at the top (max y); the dirty rect is in y-up cells
        let (ax, bx) = (0.max(r.ix0 - 1) as usize, ((self.hf.nx - 1).min(r.ix1 + 1)) as usize);
        let (ay, by) = (0.max(r.iy0 - 1), (self.hf.ny - 1).min(r.iy1 + 1));
        let (ty0, ty1) = ((self.hf.ny - 1 - by) as usize, (self.hf.ny - 1 - ay) as usize);
        let (w, h) = (bx - ax + 1, ty1 - ty0 + 1);
        let nx = self.hf.nx as usize;
        let mut px = Vec::with_capacity(w * h * 4);
        for ty in ty0..=ty1 {
            let o = (ty * nx + ax) * 4;
            px.extend_from_slice(&self.rgba[o..o + w * 4]);
        }
        tex.set_partial([ax, ty0], ColorImage::from_rgba_unmultiplied([w, h], &px), TextureOptions::LINEAR);
        let _ = ctx;
        self.frame += 1;
    }

    pub fn play(&mut self) {
        if !self.ready() {
            return;
        }
        if self.pos >= self.total {
            self.reset();
            self.frame += 1;
            self.texture_full_refresh();
        }
        self.playing = true;
        self.last_tick = None;
    }
    pub fn pause(&mut self) {
        self.playing = false;
    }
    pub fn toggle(&mut self) {
        if self.playing {
            self.pause()
        } else {
            self.play()
        }
    }

    fn texture_full_refresh(&mut self) {
        let img = self.image();
        if let Some(t) = &mut self.view.texture {
            t.set(img, TextureOptions::LINEAR);
        }
    }

    /// Called every frame while shown: advances by the elapsed wall time × speed.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if !self.playing {
            return;
        }
        let now = ctx.input(|i| i.time);
        let dt_min = match self.last_tick {
            Some(last) => (now - last) / 60.0 * if self.speed > 0.0 { self.speed } else { 1e9 },
            None => 0.0,
        };
        self.last_tick = Some(now);
        self.advance_to(self.pos + dt_min, ctx);
        if self.pos >= self.total {
            self.playing = false;
            self.refresh_info();
        }
        ctx.request_repaint();
    }

    pub fn seek(&mut self, frac: f64, ctx: &egui::Context) {
        if !self.ready() {
            return;
        }
        let target = frac.clamp(0.0, 1.0) * self.total;
        if target < self.pos {
            self.reset();
            self.texture_full_refresh();
        }
        self.advance_to(target, ctx);
        self.frame += 1;
    }

    pub fn progress(&self) -> f64 {
        if self.total > 0.0 { self.pos / self.total } else { 0.0 }
    }
    pub fn time_text(&self) -> String {
        time_text(self.pos, self.total)
    }
    /// Z of the tool tip at the current time (0 when idle)
    pub fn tip_z(&self) -> f64 {
        self.segs.iter().find(|s| s.t0 <= self.pos && self.pos <= s.t1 + 1e-9).map(|s| {
            let l = s.t1 - s.t0;
            let f = if l > 0.0 { (self.pos - s.t0) / l } else { 1.0 };
            s.z0 + (s.z1 - s.z0) * f
        }).unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::job::Move;

    fn path() -> Toolpath {
        Toolpath { feed_xy: 100.0, feed_z: 50.0, moves: vec![Move::rapid(Some(0.0), Some(0.0), Some(0.5)), Move::linear(None, None, Some(-0.25)), Move::linear(Some(10.0), Some(0.0), None), Move::rapid(None, None, Some(0.5))], ..Default::default() }
    }

    #[test]
    fn timing_matches_estimate_minutes() {
        let tp = path();
        let mut segs = Vec::new();
        let plain = gcarve_core::order::Motion::default();
        let t = segments(&tp, 0, 0.0, plain, &mut segs);
        assert_eq!(segs.len(), 3);
        assert!((t - gcarve_core::order::estimate_minutes(&tp, 100.0, 50.0, plain.rapid_ipm)).abs() < 1e-12);
        let fast = gcarve_core::order::Motion { rapid_ipm: 400.0, accel: 30.0 };
        let ta = segments(&tp, 0, 0.0, fast, &mut Vec::new());
        assert!((ta - gcarve_core::order::estimate_minutes_with(&tp, 100.0, 50.0, fast)).abs() < 1e-12 && ta > t);
        assert!((segs[0].t1 - 0.75 / 50.0).abs() < 1e-12); // plunge at feed_z
        assert!((segs[1].t1 - segs[1].t0 - 0.1).abs() < 1e-12); // 10 in at 100 ipm
        assert!(segs[2].rapid);
        assert_eq!(time_text(1.5, 62.0), "1:30 / 62:00");
    }

    #[test]
    fn advance_stamps_incrementally_and_rewind_resets() {
        let tp = path();
        let tool = Tool::default();
        let mut p = Player { material: Material { t: 0.75, ..Default::default() }, ..Default::default() };
        p.tools.push(tool.clone());
        p.total = segments(&tp, 0, 0.0, Default::default(), &mut p.segs);
        p.hf = sim_grid(&[(&tp, &tool)], &p.material, 20_000);
        p.reset();
        let half = p.segs[1].t0 + 0.05;
        let r = p.advance(half);
        assert!(!r.is_empty());
        assert!((p.tool_pos.0 - 5.0).abs() < 1e-9, "tool half way along the cut: {:?}", p.tool_pos);
        let cut_half = p.hf.depth.iter().filter(|&&d| d > 0.0).count();
        p.advance(p.total);
        let cut_all = p.hf.depth.iter().filter(|&&d| d > 0.0).count();
        assert!(cut_all > cut_half && cut_half > 0, "half {cut_half} all {cut_all} grid {}x{} res {} pos {} total {} next {} partial {}", p.hf.nx, p.hf.ny, p.hf.res, p.pos, p.total, p.next, p.partial);
        assert!(p.hf.depth.iter().all(|&d| d <= 0.25 + 1e-6));
        p.reset();
        assert!(p.hf.depth.iter().all(|&d| d == 0.0));
        assert_eq!(p.pos, 0.0);
    }
}
