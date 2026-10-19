//! 3D view of the simulated board (Qt app/sim3d.*): the height-field decimated to a small grid, lit every frame from
//! the viewer's upper left (so a recess always reads as a recess, whatever the angle), projected on the CPU with an orthographic orbit camera and drawn as one egui mesh in painter's order
//! (far corner first), plus the toolpath lines and the cutter marker. Mouse style = `Nav` (Fusion 360, Blender or
//! VCarve), standard views = `VIEWS`, SpaceMouse = `spacemouse.rs` → `puck`.

use crate::playback::Player;
use crate::widgets::TipExt;
use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke};
use gcarve_core::sim::{wood, HeightField};

/// ponytail: CPU mesh; ≤ 60k quads keeps a frame under a few ms. A GPU callback when boards get huge.
/// mesh size cap: the browser draws on one thread, so it gets fewer squares (each square = one block of sim cells)
const MAX_QUADS: usize = if cfg!(target_arch = "wasm32") { 120_000 } else { 200_000 };
/// while the cut plays (the mesh is rebuilt every frame)
const PLAYING_QUADS: usize = 60_000;

/// A patch of the carved surface as mesh vertices: one per `step` × `step` block of sim cells over the cell window
/// `cells` = [x0, y0, x1, y1). Each vertex takes its block's centre cell, so walls stay where they are (the deepest
/// cell of the block widened every cut and swallowed thin strips between cuts: owner, 2026-10-19).
#[derive(Default)]
struct Grid {
    nx: usize,
    ny: usize,
    step: usize,
    cells: [usize; 4],
    /// the mesh budget it was built for (the full one once playback stops)
    budget: usize,
    res: f64,
    x0: f64,
    y0: f64,
    px: Vec<f32>,
    py: Vec<f32>,
    pz: Vec<f32>,
    col: Vec<Color32>,
    /// unlit wood colour and the surface slope (d depth / dx, dy); None = cut through (colour as is)
    lit: Vec<Option<([f32; 3], [f32; 2])>>,
}

impl Grid {
    /// the smallest block size that keeps the window under `budget` squares
    fn step_for(c: [usize; 4], budget: usize) -> usize {
        let (w, h) = (c[2].saturating_sub(c[0]).max(1), c[3].saturating_sub(c[1]).max(1));
        let mut step = 1;
        while (w / step) * (h / step) > budget {
            step += 1;
        }
        step
    }

    fn build(hf: &HeightField, t: f64, cells: [usize; 4], step: usize) -> Grid {
        let nx = 2.max((cells[2] - cells[0]) / step);
        let ny = 2.max((cells[3] - cells[1]) / step);
        let res = hf.res * step as f64;
        let (x0, y0) = (hf.x0 + cells[0] as f64 * hf.res, hf.y0 + cells[1] as f64 * hf.res);
        let mut depth = vec![0.0f32; nx * ny];
        for iy in 0..ny {
            for ix in 0..nx {
                let sx = (hf.nx as usize - 1).min(cells[0] + ix * step + step / 2) as i32;
                let sy = (hf.ny as usize - 1).min(cells[1] + iy * step + step / 2) as i32;
                depth[iy * nx + ix] = hf.at(sx, sy);
            }
        }
        let at = |ix: usize, iy: usize| f64::from(depth[iy.min(ny - 1) * nx + ix.min(nx - 1)]);
        let n = nx * ny;
        let mut g = Grid { nx, ny, step, cells, budget: 0, res, x0, y0, px: Vec::with_capacity(n), py: Vec::with_capacity(n), pz: Vec::with_capacity(n), col: Vec::with_capacity(n), lit: Vec::with_capacity(n) };
        for iy in 0..ny {
            for ix in 0..nx {
                g.px.push((x0 + (ix as f64 + 0.5) * res) as f32);
                g.py.push((y0 + (iy as f64 + 0.5) * res) as f32);
                let d = at(ix, iy);
                if d >= t - 1e-6 {
                    g.pz.push(-t as f32); // through-cut: the table
                    g.col.push(Color32::from_rgb(40, 42, 46));
                    g.lit.push(None);
                } else {
                    g.pz.push(-d as f32);
                    g.col.push(Color32::BLACK); // set per frame (`shade`)
                    let (r, gr, b) = wood(d, t);
                    let (gx, gy) = ((at(ix + 1, iy) - at(ix.saturating_sub(1), iy)) / (2.0 * res), (at(ix, iy + 1) - at(ix, iy.saturating_sub(1))) / (2.0 * res));
                    g.lit.push(Some(([r as f32, gr as f32, b as f32], [gx as f32, gy as f32])));
                }
            }
        }
        g
    }

    /// the field's extent on the board (inches)
    fn field(&self) -> (f64, f64, f64, f64) {
        (self.x0, self.y0, self.x0 + self.nx as f64 * self.res, self.y0 + self.ny as f64 * self.res)
    }

    fn shade(&mut self, light: [f32; 3], zs: f32) {
        for (c, l) in self.col.iter_mut().zip(&self.lit) {
            if let Some((wood, [gx, gy])) = l {
                let (gx, gy) = (gx * zs, gy * zs);
                let lambert = ((gx * light[0] + gy * light[1] + light[2]) / (gx * gx + gy * gy + 1.0).sqrt()).max(0.0);
                let k = (0.35 + 0.65 * lambert) * 255.0;
                *c = Color32::from_rgb((wood[0] * k) as u8, (wood[1] * k) as u8, (wood[2] * k) as u8);
            }
        }
    }

    /// Its vertices and squares into `mesh`, far corner first (`order` = columns, rows increasing). Squares wholly
    /// inside `skip` (a detail window drawn over this one) are left out.
    fn emit(&self, mesh: &mut Mesh, project: &impl Fn(f64, f64, f64) -> Pos2, order: (bool, bool), skip: Option<[usize; 4]>) {
        let base = mesh.vertices.len() as u32;
        for i in 0..self.px.len() {
            mesh.colored_vertex(project(self.px[i] as f64, self.py[i] as f64, self.pz[i] as f64), self.col[i]);
        }
        let (nx, ny) = (self.nx, self.ny);
        let inside = |ix: usize, iy: usize| {
            skip.is_some_and(|w| {
                let (cx0, cy0) = (self.cells[0] + ix * self.step, self.cells[1] + iy * self.step);
                cx0 >= w[0] && cy0 >= w[1] && cx0 + 2 * self.step <= w[2] && cy0 + 2 * self.step <= w[3]
            })
        };
        for j in 0..ny - 1 {
            let iy = if order.1 { j } else { ny - 2 - j };
            for i in 0..nx - 1 {
                let ix = if order.0 { i } else { nx - 2 - i };
                if inside(ix, iy) {
                    continue;
                }
                let a = base + (iy * nx + ix) as u32;
                let (b, c, d) = (a + 1, a + nx as u32, a + nx as u32 + 1);
                mesh.add_triangle(a, b, c);
                mesh.add_triangle(b, d, c);
            }
        }
    }
}

pub use gcarve_ui::nav::{Act, Nav};

/// Standard views: (label, yaw, pitch). Front = seen from the Y0 edge (the operator's side of the table).
const VIEWS: [(&str, f64, f64); 6] = [("Top", 0.0, 90.0), ("Front", 0.0, 0.0), ("Back", 180.0, 0.0), ("Left", 90.0, 0.0), ("Right", -90.0, 0.0), ("Angled", 35.0, 55.0)];

pub struct View3d {
    /// Mouse menu › Save SpaceMouse trace was clicked (the app writes the file)
    pub want_trace: bool,
    pub yaw: f64,
    pub pitch: f64,
    pub zoom: f64,
    pub z_scale: f64,
    /// what is drawn over the carved board, shared with the 2D simulation (Simulate bar › Show)
    pub show: SimShow,
    /// mouse wheel or trackpad, per scroll gesture
    wheel: gcarve_ui::nav::WheelKind,
    /// perspective camera (fov 35°) instead of orthographic; remembered in the store (`PERSPECTIVE_KEY`)
    pub perspective: bool,
    pub nav: Nav,
    /// wheel / zoom-drag direction turned round (`ZOOM_REV_KEY`)
    pub zoom_reverse: bool,
    /// screen offset of the projection centre (px), Qt panX / panY
    pan: egui::Vec2,
    frame: u64,
    /// the whole carved surface, coarse enough to stay quick
    coarse: Grid,
    /// zoomed in and still for a moment: the part on screen again at full detail, drawn over `coarse`
    detail: Option<Grid>,
    /// when the camera last moved, and where it was
    quiet: f64,
    last_view: [f32; 8],
    /// board box (sides, bottom, the uncut rim round the field): vertices and triangles
    skirt_v: Vec<([f32; 3], Color32)>,
    skirt: Vec<[u32; 3]>,
    bw: f64,
    bh: f64,
    bt: f64,
}

impl Default for View3d {
    fn default() -> Self {
        View3d { want_trace: false, yaw: 35.0, pitch: 55.0, zoom: 1.0, z_scale: 1.0, show: SimShow::default(), wheel: Default::default(), perspective: false, nav: Nav::Fusion, zoom_reverse: false, pan: egui::Vec2::ZERO, frame: u64::MAX, coarse: Grid::default(), detail: None, quiet: 0.0, last_view: [0.0; 8], skirt_v: Vec::new(), skirt: Vec::new(), bw: 1.0, bh: 1.0, bt: 0.1 }
    }
}

const PERSPECTIVE_KEY: &str = "view3d-perspective";
const SHOW_KEY: &str = "sim-show";

/// What the simulation draws over the board, in 2D and 3D (Simulate bar › Show), remembered per device.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SimShow {
    pub paths: ShowPaths,
    pub rapids: bool,
    pub cutter: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ShowPaths {
    /// every move: cut ones bright, the rest faint
    All,
    /// only the moves cut so far: the tool's trail
    Trail,
    Hidden,
}

impl Default for SimShow {
    fn default() -> Self {
        SimShow { paths: ShowPaths::All, rapids: true, cutter: true }
    }
}

impl SimShow {
    fn load() -> Self {
        let Some(v) = crate::store::load(SHOW_KEY) else { return Self::default() };
        let b: Vec<&str> = v.split(',').collect();
        let paths = match b.first().copied() {
            Some("trail") => ShowPaths::Trail,
            Some("hidden") => ShowPaths::Hidden,
            _ => ShowPaths::All,
        };
        SimShow { paths, rapids: b.get(1) != Some(&"0"), cutter: b.get(2) != Some(&"0") }
    }
    pub fn save(&self) {
        let paths = match self.paths {
            ShowPaths::All => "all",
            ShowPaths::Trail => "trail",
            ShowPaths::Hidden => "hidden",
        };
        crate::store::save(SHOW_KEY, &format!("{paths},{},{}", self.rapids as u8, self.cutter as u8));
    }
}

/// The simulated moves as lines (`project` = machine x y z to the screen). Consecutive moves of one colour go out as
/// one line and moves shorter than half a pixel are merged, so a big job stays cheap to draw every frame.
pub fn draw_moves(painter: &egui::Painter, p: &Player, show: SimShow, project: impl Fn(f64, f64, f64) -> Pos2) {
    if show.paths == ShowPaths::Hidden {
        return;
    }
    let trail = show.paths == ShowPaths::Trail;
    let mut run: Vec<Pos2> = Vec::new();
    let mut run_col = Color32::TRANSPARENT;
    let flush = |run: &mut Vec<Pos2>, col: Color32| {
        if run.len() > 1 {
            painter.add(Shape::line(std::mem::take(run), Stroke::new(1.0, col)));
        }
        run.clear();
    };
    for s in &p.segs {
        let done = s.t1 <= p.pos + 1e-9;
        if trail && s.t0 >= p.pos {
            break; // segments are in time order
        }
        if s.rapid && !show.rapids {
            flush(&mut run, run_col);
            continue;
        }
        let col = if s.rapid {
            Color32::from_rgba_unmultiplied(230, 90, 90, 110)
        } else if done {
            Color32::from_rgba_unmultiplied(90, 200, 230, 230)
        } else {
            Color32::from_rgba_unmultiplied(90, 200, 230, 120)
        };
        // the move under way stops at the cutter when only the trail is shown
        let f = if trail && !done && s.t1 > s.t0 { ((p.pos - s.t0) / (s.t1 - s.t0)).clamp(0.0, 1.0) } else { 1.0 };
        let (a, b) = (project(s.x0, s.y0, s.z0), project(s.x0 + (s.x1 - s.x0) * f, s.y0 + (s.y1 - s.y0) * f, s.z0 + (s.z1 - s.z0) * f));
        if col != run_col || run.last().is_none_or(|l| l.distance(a) > 0.5) {
            flush(&mut run, run_col);
            run_col = col;
            run.push(a);
        }
        if run.last().is_some_and(|l| l.distance(b) >= 0.5) {
            run.push(b);
        }
    }
    flush(&mut run, run_col);
}
const NAV_KEY: &str = "view3d-nav";
const ZOOM_REV_KEY: &str = "view3d-zoom-reverse";

impl View3d {
    /// the remembered projection (screenshots keep the orthographic default)
    pub fn load(shot: bool) -> Self {
        let stored = !shot && crate::store::load(PERSPECTIVE_KEY).as_deref() == Some("1");
        let nav = crate::store::load(NAV_KEY).filter(|_| !shot).and_then(|n| Nav::from_key(&n)).unwrap_or(Nav::Fusion);
        let zoom_reverse = !shot && crate::store::load(ZOOM_REV_KEY).as_deref() == Some("1");
        // GCARVE_SIM_SHOW=trail|hidden: dev, screenshots of the Show menu's choices
        let show = match std::env::var("GCARVE_SIM_SHOW").as_deref() {
            Ok("trail") => SimShow { paths: ShowPaths::Trail, ..SimShow::default() },
            Ok("hidden") => SimShow { paths: ShowPaths::Hidden, cutter: false, ..SimShow::default() },
            _ if shot => SimShow::default(),
            _ => SimShow::load(),
        };
        let mut v = View3d { perspective: stored || std::env::var("GCARVE_PERSP").is_ok(), nav, zoom_reverse, show, ..Default::default() };
        // GCARVE_VIEW3D=yaw,pitch[,zoom]: dev, screenshots from a given angle
        if let Ok(a) = std::env::var("GCARVE_VIEW3D") {
            let n: Vec<f64> = a.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (v.yaw, v.pitch, v.zoom) = (n.first().copied().unwrap_or(v.yaw), n.get(1).copied().unwrap_or(v.pitch), n.get(2).copied().unwrap_or(v.zoom));
        }
        v // GCARVE_PERSP: dev, screenshots
    }

    fn rebuild(&mut self, p: &Player) {
        let hf = &p.hf;
        let m = &p.material;
        (self.bw, self.bh, self.bt) = (m.w, m.h, m.t);
        // while the cut plays the surface is rebuilt every frame: a lighter mesh then, the full one when it stops
        let budget = if p.playing { PLAYING_QUADS } else { MAX_QUADS };
        let all = [0, 0, hf.nx.max(1) as usize, hf.ny.max(1) as usize];
        self.coarse = Grid::build(hf, m.t, all, Grid::step_for(all, budget));
        self.coarse.budget = budget;
        self.detail = None;
        // skirt: board box + the uncut rim between the field and the board edge
        self.skirt_v.clear();
        let mut add = |x: f64, y: f64, z: f64, c: Color32| {
            self.skirt_v.push(([x as f32, y as f32, z as f32], c));
            (self.skirt_v.len() - 1) as u32
        };
        let side = Color32::from_rgb(150, 120, 80);
        let bottom = Color32::from_rgb(90, 72, 48);
        let top = Color32::from_rgb(158, 125, 84);
        let (x1, y1) = (m.w, m.h);
        let t00 = add(0.0, 0.0, 0.0, side);
        let t10 = add(x1, 0.0, 0.0, side);
        let t11 = add(x1, y1, 0.0, side);
        let t01 = add(0.0, y1, 0.0, side);
        let b00 = add(0.0, 0.0, -m.t, bottom);
        let b10 = add(x1, 0.0, -m.t, bottom);
        let b11 = add(x1, y1, -m.t, bottom);
        let b01 = add(0.0, y1, -m.t, bottom);
        let (fx0, fy0, fx1, fy1) = self.coarse.field();
        let r00 = add(fx0, fy0, 0.0, top);
        let r10 = add(fx1, fy0, 0.0, top);
        let r11 = add(fx1, fy1, 0.0, top);
        let r01 = add(fx0, fy1, 0.0, top);
        self.skirt.clear();
        self.skirt.push([b00, b01, b11]);
        self.skirt.push([b00, b11, b10]);
        for s in [[b00, b10, t10, t00], [b10, b11, t11, t10], [b11, b01, t01, t11], [b01, b00, t00, t01]] {
            self.skirt.push([s[0], s[1], s[2]]);
            self.skirt.push([s[0], s[2], s[3]]);
        }
        let ring = [t00, t10, t11, t01, r00, r10, r11, r01];
        for k in 0..4 {
            let (a, b, c, d) = (ring[k], ring[(k + 1) % 4], ring[4 + (k + 1) % 4], ring[4 + k]);
            self.skirt.push([a, b, c]);
            self.skirt.push([a, c, d]);
        }
        self.frame = p.frame;
    }

    /// `paths`: the status bar's Paths switch, which hides the toolpath lines here too
    pub fn ui(&mut self, ui: &mut egui::Ui, p: &Player, paths: bool) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, crate::theme::CANVAS_BG);
        if !p.ready() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Generate toolpaths to see the carved board", egui::FontId::proportional(14.0), crate::theme::TEXT_DIM);
            return;
        }
        // buttons per `Nav`; the wheel zooms toward the pointer; trackpads: two-finger swipe pans or orbits, pinch zooms
        let (shift, ctrl, hover) = ui.input(|i| (i.modifiers.shift, i.modifiers.ctrl || i.modifiers.command, i.pointer.hover_pos()));
        let zdir = if self.zoom_reverse { -1.0 } else { 1.0 };
        let touch = ui.input(|i| i.multi_touch()); // touch screens: two fingers pan, pinch zooms (zoom_delta below)
        if let Some(t) = touch.filter(|_| resp.hovered()) {
            self.pan += t.translation_delta;
        } else if let Some(b) = [egui::PointerButton::Primary, egui::PointerButton::Middle, egui::PointerButton::Secondary].into_iter().find(|b| resp.dragged_by(*b)) {
            let d = resp.drag_delta();
            match self.nav.drag(b, shift, ctrl) {
                Act::Orbit => self.orbit(d),
                Act::Pan => self.pan += d,
                Act::Zoom => self.zoom_at((-f64::from(d.y) * 0.01 * zdir).exp(), rect.center() + self.pan, rect.center()), // up = closer
            }
        }
        if resp.hovered() {
            let at = hover.unwrap_or(rect.center());
            let mut factor = f64::from(ui.input(|i| i.zoom_delta())); // pinch / Ctrl+wheel
            let events = ui.input(|i| i.raw.events.clone());
            for e in events {
                if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = e {
                    if modifiers.ctrl || modifiers.command {
                        continue; // already in zoom_delta
                    }
                    if self.wheel.is_mouse(ui.input(|i| i.time), unit, delta) {
                        let lines = if unit == egui::MouseWheelUnit::Point { delta.y as f64 / 50.0 } else { delta.y as f64 };
                        factor *= (lines * 0.12 * zdir).exp();
                    } else if self.nav.swipe(shift) == Act::Orbit {
                        self.orbit(delta);
                    } else {
                        self.pan += delta;
                    }
                }
            }
            if (factor - 1.0).abs() > 1e-6 {
                self.zoom_at(factor, at, rect.center());
            }
        }
        let stopped = !p.playing && self.coarse.budget < MAX_QUADS;
        if self.frame != p.frame || self.coarse.px.is_empty() || stopped {
            self.rebuild(p);
        }
        // orthographic orbit camera about the board centre
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let (cy, sy, cp, sp) = (yaw.cos(), yaw.sin(), pitch.cos(), pitch.sin());
        let (cxw, cyw) = (self.bw / 2.0, self.bh / 2.0);
        let r = self.bw.hypot(self.bh) * 0.55 + self.bt;
        let scale = rect.width().min(rect.height()) as f64 / (2.0 * r) * self.zoom;
        let (ox, oy) = ((rect.center().x + self.pan.x) as f64, (rect.center().y + self.pan.y) as f64);
        // perspective: camera on the orbit at the distance where a 35° view frames the same radius as the orthographic one
        let dist = if self.perspective { r / (35f64.to_radians() / 2.0).tan() } else { f64::INFINITY };
        let project = |x: f64, y: f64, z: f64| {
            let (x, y, z) = (x - cxw, y - cyw, z * self.z_scale);
            let rx = x * cy - y * sy;
            let ry = x * sy + y * cy;
            let vy = ry * sp + z * cp;
            let k = if dist.is_finite() { dist / (dist - (z * sp - ry * cp)).max(dist * 0.05) } else { 1.0 }; // toward the camera = bigger
            Pos2::new((ox + rx * k * scale) as f32, (oy - vy * k * scale) as f32)
        };
        // the 2D shader's light (upper left, slightly above), fixed to the screen instead of the board
        let (la, lb, lc) = (-0.45 / 1.0003, 0.6 / 1.0003, 0.66 / 1.0003); // along screen right, screen up, towards the viewer
        let light = [(la * cy + lb * sy * sp - lc * sy * cp) as f32, (-la * sy + lb * cy * sp - lc * cy * cp) as f32, (lb * cp + lc * sp) as f32];
        let zs = self.z_scale as f32;
        // zoomed in and the camera still for a moment: the part of the field on screen again at full detail
        let now = ui.input(|i| i.time);
        let view = [self.yaw as f32, self.pitch as f32, self.zoom as f32, self.pan.x, self.pan.y, rect.width(), rect.height(), self.perspective as u8 as f32];
        if view != self.last_view {
            (self.last_view, self.quiet) = (view, now);
        }
        if p.playing || self.coarse.step <= 1 {
            self.detail = None;
        } else if now - self.quiet < 0.3 {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(320));
        } else {
            // the board points under the screen corners (orthographic, on the top face), with a margin for perspective
            let hf = &p.hf;
            let under = |q: Pos2| {
                let (rx, vy) = ((q.x as f64 - ox) / scale, (oy - q.y as f64) / scale);
                let ry = vy / sp.max(0.05);
                (rx * cy + ry * sy + cxw, -rx * sy + ry * cy + cyw)
            };
            let pts = [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()].map(under);
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for (x, y) in pts {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
            let grow = if self.perspective { 0.5 } else { 0.15 };
            let (gx, gy) = ((x1 - x0) * grow, (y1 - y0) * grow);
            let cell = |v: f64, o: f64, n: i32| (((v - o) / hf.res).floor().max(0.0) as usize).min(n.max(1) as usize);
            let win = [cell(x0 - gx, hf.x0, hf.nx), cell(y0 - gy, hf.y0, hf.ny), cell(x1 + gx, hf.x0, hf.nx), cell(y1 + gy, hf.y0, hf.ny)];
            let step = Grid::step_for(win, MAX_QUADS);
            if win[2] - win[0] < 4 || win[3] - win[1] < 4 || step >= self.coarse.step {
                self.detail = None;
            } else if self.detail.as_ref().is_none_or(|d| d.cells != win || d.step != step) {
                self.detail = Some(Grid::build(hf, p.material.t, win, step));
            }
        }
        let mut mesh = Mesh::default();
        for (v, c) in &self.skirt_v {
            mesh.colored_vertex(project(v[0] as f64, v[1] as f64, v[2] as f64), *c);
        }
        for t in &self.skirt {
            mesh.add_triangle(t[0], t[1], t[2]);
        }
        // painter's order: far corner first. Depth into the screen = x·sy + y·cy, so rows get farther as y grows when
        // cy > 0 (then the last row goes first), columns as x grows when sy > 0. The other way round a pocket's near
        // wall is painted over the board in front of it and the pocket reads as a raised block.
        let order = (sy <= 0.0, cy <= 0.0);
        self.coarse.shade(light, zs);
        self.coarse.emit(&mut mesh, &project, order, self.detail.as_ref().map(|d| d.cells));
        if let Some(d) = &mut self.detail {
            d.shade(light, zs);
            d.emit(&mut mesh, &project, order, None);
        }
        painter.add(Shape::mesh(mesh));
        if paths {
            draw_moves(&painter, p, self.show, project);
        }
        // the cutter: a yellow ring at the tool's position on the surface plus a stem down to the tip
        let (tx, ty) = p.tool_pos;
        let rad = (p.tool_dia / 2.0).max(0.05);
        let yellow = Color32::from_rgb(255, 211, 77);
        let ring: Vec<Pos2> = (0..24).map(|k| {
            let a = std::f64::consts::TAU * k as f64 / 24.0;
            project(tx + rad * a.cos(), ty + rad * a.sin(), 0.0)
        }).collect();
        if self.show.cutter {
            painter.add(Shape::closed_line(ring, Stroke::new(1.5, yellow)));
            painter.line_segment([project(tx, ty, 0.3), project(tx, ty, p.tip_z().min(0.0))], Stroke::new(1.5, yellow));
        }
        // Sim3D.qml overlays: Z exaggeration + Reset view top right, the mouse hint bottom left
        if resp.double_clicked() {
            self.reset_view();
        }
        let top = Rect::from_min_max(rect.left_top() + egui::vec2(10.0, 10.0), rect.right_top() + egui::vec2(-10.0, 40.0));
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(top).layout(egui::Layout::right_to_left(egui::Align::Center)));
        row.spacing_mut().item_spacing.x = 6.0;
        if row.add(crate::widgets::Button::new("Reset view").flat()).tip("Default angle and zoom (double-click the view does the same)").clicked() {
            self.reset_view();
        }
        // Ortho | Persp segmented switch (right to left: Persp first)
        row.spacing_mut().item_spacing.x = 0.0;
        for (persp, label, tip) in [(true, "Persp", "Perspective camera: near parts look bigger"), (false, "Ortho", "Orthographic: true proportions, no foreshortening")] {
            let on = self.perspective == persp;
            let b = crate::widgets::Button::new(label).primary(on).min_width(52.0); // Qt flat checkable: blue when checked
            if row.add(if on { b } else { b.flat() }).tip(tip).clicked() && self.perspective != persp {
                self.perspective = persp;
                crate::store::save(PERSPECTIVE_KEY, if persp { "1" } else { "0" });
            }
        }
        row.spacing_mut().item_spacing.x = 6.0;
        crate::widgets::combo_box("view3d-views").selected_text("View").width(76.0).show_ui(&mut row, |ui| {
            for (label, yaw, pitch) in VIEWS {
                if ui.selectable_label(false, label).clicked() {
                    (self.yaw, self.pitch, self.pan) = (yaw, pitch, egui::Vec2::ZERO);
                }
            }
        });
        // phones: View and Reset view only (touch: one finger orbits, two pan and pinch-zoom)
        let phone = rect.width() < 520.0;
        if phone {
            return;
        }
        crate::widgets::combo_box("view3d-nav").selected_text("Mouse").width(84.0).show_ui(&mut row, |ui| {
            ui.set_min_width(230.0);
            for (nav, key, label, _) in Nav::ALL {
                if ui.selectable_label(self.nav == nav, format!("{label} style")).clicked() {
                    self.nav = nav;
                    crate::store::save(NAV_KEY, key);
                }
            }
            ui.separator();
            if crate::widgets::checkbox(ui, &mut self.zoom_reverse, "Reverse zoom direction").changed() {
                crate::store::save(ZOOM_REV_KEY, if self.zoom_reverse { "1" } else { "0" });
            }
            ui.separator();
            let (line, can) = crate::spacemouse::status();
            if can && ui.selectable_label(false, "Connect a SpaceMouse…").clicked() {
                crate::spacemouse::connect(true);
            }
            if !line.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(line).size(12.0).color(crate::theme::TEXT_DIM)).wrap());
            }
            if crate::spacemouse::axes().is_some() && ui.selectable_label(false, "Save SpaceMouse trace (for a bug report)").clicked() {
                self.want_trace = true;
            }
        });
        row.spacing_mut().slider_width = 90.0;
        let mut z = self.z_scale;
        if row.add(egui::Slider::new(&mut z, 0.5..=4.0).show_value(false).handle_shape(egui::style::HandleShape::Circle)).tip("Vertical exaggeration").changed() {
            self.z_scale = z;
        }
        row.label(egui::RichText::new(format!("Z ×{:.1}", self.z_scale)).size(11.0).color(crate::theme::TEXT.gamma_multiply(0.7)));
        painter.text(rect.left_bottom() + egui::vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, format!("{} · double-click resets", self.nav.row().3), crate::theme::font(11.0), crate::theme::TEXT.gamma_multiply(0.5));
    }

    /// The board follows the pointer: dragging down tips its top towards the viewer.
    fn orbit(&mut self, d: egui::Vec2) {
        self.yaw += d.x as f64 * 0.5;
        self.pitch = (self.pitch + d.y as f64 * 0.5).clamp(0.0, 90.0);
    }

    /// SpaceMouse axes (−1..1: x right, y towards the user, z down, then the turns about them) move the board as if
    /// the puck were the board (`a` = `spacemouse::Filter::step`).
    /// ponytail: axis directions follow the 3Dconnexion convention, not tried on every model; roll is ignored
    /// (the camera is a turntable). A per-axis flip in the Mouse menu if a model disagrees.
    pub fn puck(&mut self, a: [f32; 6], dt: f32, centre: Pos2) {
        self.pan += egui::vec2(a[0], a[2]) * 700.0 * dt;
        self.zoom_at((f64::from(a[1] * dt) * 1.6 * if self.zoom_reverse { -1.0 } else { 1.0 }).exp(), centre + self.pan, centre);
        self.yaw -= f64::from((a[5] + a[4]) * dt) * 120.0; // twist and tilt left / right both turn the board about its vertical
        self.pitch = (self.pitch + f64::from(a[3] * dt) * 120.0).clamp(0.0, 90.0);
    }

    /// Zoom by `factor` keeping the point under `at` (screen) where it is: the projection centre moves with it.
    fn zoom_at(&mut self, factor: f64, at: Pos2, centre: Pos2) {
        let z = (self.zoom * factor).clamp(0.2, 40.0);
        let k = (z / self.zoom) as f32;
        let rel = at - centre;
        self.pan = rel - (rel - self.pan) * k;
        self.zoom = z;
    }

    /// Qt Sim3DView::resetView: default angle, zoom and pan (exaggeration kept)
    fn reset_view(&mut self) {
        let d = View3d::default();
        (self.yaw, self.pitch, self.zoom, self.pan) = (d.yaw, d.pitch, d.zoom, d.pan);
    }
}


#[cfg(test)]
mod nav_tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let mut v = View3d::default();
        let (centre, at) = (Pos2::new(400.0, 300.0), Pos2::new(600.0, 200.0));
        v.pan = egui::vec2(30.0, -10.0);
        // the doc point under `at` sits (at - centre - pan) / zoom from the projection centre
        let before = (at - centre - v.pan) / v.zoom as f32;
        v.zoom_at(2.0, at, centre);
        let after = (at - centre - v.pan) / v.zoom as f32;
        assert!((before - after).length() < 1e-4, "{before:?} {after:?}");
        assert!((v.zoom - 2.0).abs() < 1e-9);
    }

}
