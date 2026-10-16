//! Program preview (C++ `app/programview.*`): top-down paths, rapids dashed, cuts shaded by depth, the part already
//! sent in yellow, the tool as a crosshair. Polylines are built once per program; only the done index moves.

use eframe::egui::{self, pos2, Color32, Pos2, Shape, Stroke};
use gcarve_core::gcode::{motion_points, Vec3};
use gcarve_core::job::MoveType;

use super::link::Link;

/// (source line, band 0..3 or RAPID, points)
type Poly = (i32, usize, Vec<(f64, f64)>);

#[derive(Default)]
pub struct Preview {
    rev: u64,
    polys: Vec<Poly>,
    bounds: Option<(f64, f64, f64, f64)>,
}

const BAND: [Color32; 4] = [Color32::from_rgb(160, 170, 185), Color32::from_rgb(110, 190, 255), Color32::from_rgb(70, 140, 230), Color32::from_rgb(40, 90, 200)];
const RAPID: usize = 4;

impl Preview {
    fn rebuild(&mut self, link: &Link) {
        self.rev = link.program_rev;
        self.polys.clear();
        self.bounds = None;
        let p = &link.program;
        let zmin = (if p.min.z < 1e299 { p.min.z } else { -1.0 }).min(-1e-3);
        for (k, m) in p.motions.iter().enumerate() {
            if m.machine_coords || (k == 0 && m.kind == MoveType::Rapid) {
                continue; // first move's start is unknown
            }
            let pts: Vec<(f64, f64)> = motion_points(m, 0.005).iter().map(|q| (q.x, q.y)).collect();
            let band = if m.kind == MoveType::Rapid {
                RAPID
            } else {
                let z = m.from.z.min(m.to.z);
                if z >= 0.0 { 0 } else { ((-z / -zmin * 3.0).ceil() as usize).clamp(1, 3) }
            };
            for &(x, y) in &pts {
                let b = self.bounds.get_or_insert((x, y, x, y));
                *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
            self.polys.push((m.line, band, pts));
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, link: &Link) {
        if self.rev != link.program_rev {
            self.rebuild(link);
        }
        let rect = ui.available_rect_before_wrap();
        let w: Vec3 = link.st.wpos();
        // work zero (and the tool when connected) always in view
        let (mut x0, mut y0, mut x1, mut y1) = self.bounds.unwrap_or((0.0, 0.0, 0.0, 0.0));
        let mut grow = |x: f64, y: f64| {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        };
        grow(0.0, 0.0);
        if link.connected() {
            grow(w.x, w.y);
        }
        if x1 - x0 < 1.0 {
            (x0, x1) = (x0 - 0.5, x1 + 0.5);
        }
        if y1 - y0 < 1.0 {
            (y0, y1) = (y0 - 0.5, y1 + 0.5);
        }
        let m = 24.0;
        let s = ((rect.width() as f64 - 2.0 * m) / (x1 - x0)).min((rect.height() as f64 - 2.0 * m) / (y1 - y0));
        if s.is_nan() || s <= 0.0 {
            return;
        }
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let c = rect.center();
        let to = |x: f64, y: f64| -> Pos2 { pos2(c.x + ((x - cx) * s) as f32, c.y - ((y - cy) * s) as f32) }; // Y up
        let painter = ui.painter_at(rect);
        let axis = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 40));
        painter.line_segment([to(x0, 0.0), to(x1, 0.0)], axis);
        painter.line_segment([to(0.0, y0), to(0.0, y1)], axis);
        let line = if link.streaming || link.progress() > 0.0 { link.progress_line() } else { 0 };
        let mut shapes = Vec::with_capacity(self.polys.len());
        let mut done = Vec::new();
        for (ln, band, pts) in &self.polys {
            let screen: Vec<Pos2> = pts.iter().map(|&(x, y)| to(x, y)).collect();
            if *ln <= line {
                done.push(Shape::line(screen.clone(), Stroke::new(2.0, Color32::from_rgb(255, 200, 80))));
            }
            if *band == RAPID {
                shapes.extend(Shape::dashed_line(&[screen[0], *screen.last().unwrap()], Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 120, 90, 110)), 4.0, 3.0));
            } else {
                shapes.push(Shape::line(screen, Stroke::new(1.2, BAND[*band])));
            }
        }
        painter.extend(shapes);
        painter.extend(done);
        if link.connected() {
            let tp = to(w.x, w.y);
            let st = Stroke::new(1.5, Color32::WHITE);
            painter.circle(tp, 6.0, Color32::from_rgba_unmultiplied(255, 255, 255, 60), st);
            painter.line_segment([tp - egui::vec2(10.0, 0.0), tp + egui::vec2(10.0, 0.0)], st);
            painter.line_segment([tp - egui::vec2(0.0, 10.0), tp + egui::vec2(0.0, 10.0)], st);
        }
    }
}
