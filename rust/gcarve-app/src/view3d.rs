//! 3D view of the simulated board (Qt app/sim3d.*): the height-field decimated to a small grid, lit every frame from
//! the viewer's upper left (so a recess always reads as a recess, whatever the angle), projected on the CPU with an orthographic orbit camera and drawn as one egui mesh in painter's order
//! (far corner first), plus the toolpath lines and the cutter marker. Mouse style = `Nav` (Fusion 360, Blender or
//! VCarve), standard views = `VIEWS`, SpaceMouse = `spacemouse.rs` → `puck`.

use crate::playback::Player;
use crate::widgets::TipExt;
use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke};
use gcarve_core::sim::{wood, HeightField};

/// ponytail: CPU mesh; ≤ 60k quads keeps a frame under a few ms. A GPU callback when boards get huge.
const MAX_QUADS: usize = 60_000;

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
    pub show_paths: bool,
    /// perspective camera (fov 35°) instead of orthographic; remembered in the store (`PERSPECTIVE_KEY`)
    pub perspective: bool,
    pub nav: Nav,
    /// wheel / zoom-drag direction turned round (`ZOOM_REV_KEY`)
    pub zoom_reverse: bool,
    /// screen offset of the projection centre (px), Qt panX / panY
    pan: egui::Vec2,
    frame: u64,
    nx: usize,
    ny: usize,
    /// grid vertices: world xyz + colour
    px: Vec<f32>,
    py: Vec<f32>,
    pz: Vec<f32>,
    col: Vec<Color32>,
    /// grid vertices: unlit wood colour and the surface slope (d depth / dx, dy); None = cut through (colour as is)
    lit: Vec<Option<([f32; 3], [f32; 2])>>,
    /// board box (sides + bottom + the uncut rim) appended after the grid
    skirt: Vec<[u32; 3]>,
    bw: f64,
    bh: f64,
    bt: f64,
}

impl Default for View3d {
    fn default() -> Self {
        View3d { want_trace: false, yaw: 35.0, pitch: 55.0, zoom: 1.0, z_scale: 1.0, show_paths: true, perspective: false, nav: Nav::Fusion, zoom_reverse: false, pan: egui::Vec2::ZERO, frame: u64::MAX, nx: 0, ny: 0, px: Vec::new(), py: Vec::new(), pz: Vec::new(), col: Vec::new(), lit: Vec::new(), skirt: Vec::new(), bw: 1.0, bh: 1.0, bt: 0.1 }
    }
}

const PERSPECTIVE_KEY: &str = "view3d-perspective";
const NAV_KEY: &str = "view3d-nav";
const ZOOM_REV_KEY: &str = "view3d-zoom-reverse";

impl View3d {
    /// the remembered projection (screenshots keep the orthographic default)
    pub fn load(shot: bool) -> Self {
        let stored = !shot && crate::store::load(PERSPECTIVE_KEY).as_deref() == Some("1");
        let nav = crate::store::load(NAV_KEY).filter(|_| !shot).and_then(|n| Nav::from_key(&n)).unwrap_or(Nav::Fusion);
        let zoom_reverse = !shot && crate::store::load(ZOOM_REV_KEY).as_deref() == Some("1");
        View3d { perspective: stored || std::env::var("GCARVE_PERSP").is_ok(), nav, zoom_reverse, ..Default::default() } // GCARVE_PERSP: dev, screenshots
    }

    fn rebuild(&mut self, p: &Player) {
        let hf = &p.hf;
        let m = &p.material;
        (self.bw, self.bh, self.bt) = (m.w, m.h, m.t);
        let mut step = 1;
        while (hf.nx as usize / step) * (hf.ny as usize / step) > MAX_QUADS {
            step += 1;
        }
        let nx = 2.max(hf.nx as usize / step);
        let ny = 2.max(hf.ny as usize / step);
        let mut lo = HeightField { nx: nx as i32, ny: ny as i32, res: hf.res * step as f64, x0: hf.x0, y0: hf.y0, depth: vec![0.0; nx * ny] };
        for iy in 0..ny {
            for ix in 0..nx {
                let mut d = 0.0f32; // deepest cell in the block, so narrow cuts survive the downsampling
                for b in 0..step {
                    for a in 0..step {
                        let sx = (hf.nx as usize - 1).min(ix * step + a) as i32;
                        let sy = (hf.ny as usize - 1).min(iy * step + b) as i32;
                        d = d.max(hf.at(sx, sy));
                    }
                }
                lo.depth[iy * nx + ix] = d;
            }
        }
        let n = nx * ny;
        self.lit = Vec::with_capacity(n);
        let at = |ix: usize, iy: usize| f64::from(lo.depth[iy.min(ny - 1) * nx + ix.min(nx - 1)]);
        self.px = Vec::with_capacity(n + 12);
        self.py = Vec::with_capacity(n + 12);
        self.pz = Vec::with_capacity(n + 12);
        self.col = Vec::with_capacity(n + 12);
        for iy in 0..ny {
            for ix in 0..nx {
                let v = iy * nx + ix;
                self.px.push((lo.x0 + (ix as f64 + 0.5) * lo.res) as f32);
                self.py.push((lo.y0 + (iy as f64 + 0.5) * lo.res) as f32);
                let d = f64::from(lo.depth[v]);
                if d >= m.t - 1e-6 {
                    self.pz.push(-m.t as f32); // through-cut: the table
                    self.col.push(Color32::from_rgb(40, 42, 46));
                    self.lit.push(None);
                } else {
                    self.pz.push(-lo.depth[v]);
                    self.col.push(Color32::BLACK); // set per frame (`ui`)
                    let (r, g, b) = wood(d, m.t);
                    let (gx, gy) = ((at(ix + 1, iy) - at(ix.saturating_sub(1), iy)) / (2.0 * lo.res), (at(ix, iy + 1) - at(ix, iy.saturating_sub(1))) / (2.0 * lo.res));
                    self.lit.push(Some(([r as f32, g as f32, b as f32], [gx as f32, gy as f32])));
                }
            }
        }
        (self.nx, self.ny) = (nx, ny);
        // skirt: board box + the uncut rim between the field and the board edge
        let mut add = |x: f64, y: f64, z: f64, c: Color32| {
            self.px.push(x as f32);
            self.py.push(y as f32);
            self.pz.push(z as f32);
            self.col.push(c);
            (self.px.len() - 1) as u32
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
        let (fx0, fy0, fx1, fy1) = (lo.x0, lo.y0, lo.x0 + nx as f64 * lo.res, lo.y0 + ny as f64 * lo.res);
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

    pub fn ui(&mut self, ui: &mut egui::Ui, p: &Player) {
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
                    if gcarve_ui::nav::wheel_is_mouse(unit, delta) {
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
        if self.frame != p.frame || self.px.is_empty() {
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
        for (c, l) in self.col.iter_mut().zip(&self.lit) {
            if let Some((wood, [gx, gy])) = l {
                let (gx, gy) = (gx * zs, gy * zs);
                let lambert = ((gx * light[0] + gy * light[1] + light[2]) / (gx * gx + gy * gy + 1.0).sqrt()).max(0.0);
                let k = (0.35 + 0.65 * lambert) * 255.0;
                *c = Color32::from_rgb((wood[0] * k) as u8, (wood[1] * k) as u8, (wood[2] * k) as u8);
            }
        }
        let mut mesh = Mesh::default();
        for i in 0..self.px.len() {
            mesh.colored_vertex(project(self.px[i] as f64, self.py[i] as f64, self.pz[i] as f64), self.col[i]);
        }
        for t in &self.skirt {
            mesh.add_triangle(t[0], t[1], t[2]);
        }
        // painter's order: far corner first. Depth into the screen = x·sy + y·cy, so rows get farther as y grows when
        // cy > 0 (then the last row goes first), columns as x grows when sy > 0. The other way round a pocket's near
        // wall is painted over the board in front of it and the pocket reads as a raised block.
        let (nx, ny) = (self.nx, self.ny);
        let (x_inc, y_inc) = (sy <= 0.0, cy <= 0.0);
        for j in 0..ny - 1 {
            let iy = if y_inc { j } else { ny - 2 - j };
            for i in 0..nx - 1 {
                let ix = if x_inc { i } else { nx - 2 - i };
                let a = (iy * nx + ix) as u32;
                let (b, c, d) = (a + 1, a + nx as u32, a + nx as u32 + 1);
                mesh.add_triangle(a, b, c);
                mesh.add_triangle(b, d, c);
            }
        }
        painter.add(Shape::mesh(mesh));
        if self.show_paths {
            for s in &p.segs {
                let done = s.t1 <= p.pos + 1e-9;
                let c = if s.rapid { Color32::from_rgba_unmultiplied(230, 90, 90, 110) } else if done { Color32::from_rgba_unmultiplied(90, 200, 230, 230) } else { Color32::from_rgba_unmultiplied(90, 200, 230, 120) };
                painter.line_segment([project(s.x0, s.y0, s.z0), project(s.x1, s.y1, s.z1)], Stroke::new(1.0, c));
            }
        }
        // the cutter: a yellow ring at the tool's position on the surface plus a stem down to the tip
        let (tx, ty) = p.tool_pos;
        let rad = (p.tool_dia / 2.0).max(0.05);
        let yellow = Color32::from_rgb(255, 211, 77);
        let ring: Vec<Pos2> = (0..24).map(|k| {
            let a = std::f64::consts::TAU * k as f64 / 24.0;
            project(tx + rad * a.cos(), ty + rad * a.sin(), 0.0)
        }).collect();
        painter.add(Shape::closed_line(ring, Stroke::new(1.5, yellow)));
        painter.line_segment([project(tx, ty, 0.3), project(tx, ty, p.tip_z().min(0.0))], Stroke::new(1.5, yellow));
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
