//! Photo of the board (View › Photo of the Board…): a picture of the board on the machine, taken from above at any
//! angle. Click its four corners and the photo is straightened onto the board's real size (a perspective fit through
//! the four points), then shown under the drawing so parts can be placed around knots, cracks or earlier cuts. Kept
//! for this session with the sheet it was taken of (not saved in the project).

use super::Host;
use eframe::egui::{self, Color32, Pos2, RichText, Stroke};

/// longest side of the photo kept for clicking, and of the straightened picture
const MAX_PX: u32 = 2000;
const OUT_PX: f64 = 1600.0;

/// The straightened photo under the drawing (`Canvas::board_photo`).
pub struct BoardPhoto {
    pub rgba: Vec<u8>,
    pub w: usize,
    pub h: usize,
    /// the sheet it shows on
    pub sheet: i32,
    pub show: bool,
    pub tex: Option<egui::TextureHandle>,
}

pub struct BoardPhotoDialog {
    name: String,
    rgba: Vec<u8>,
    w: usize,
    h: usize,
    /// the corners clicked so far, in photo pixels: bottom-left, bottom-right, top-right, top-left
    corners: Vec<Pos2>,
    tex: Option<egui::TextureHandle>,
}

const CORNERS: [&str; 4] = ["bottom-left (X0 Y0 side, near you)", "bottom-right", "top-right", "top-left"];

impl BoardPhotoDialog {
    pub fn new(name: &str, bytes: &[u8]) -> Result<Self, String> {
        let img = image::load_from_memory(bytes).map_err(|e| format!("{name}: {e}"))?;
        let img = if img.width().max(img.height()) > MAX_PX { img.resize(MAX_PX, MAX_PX, image::imageops::FilterType::Triangle) } else { img };
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width() as usize, rgba.height() as usize);
        Ok(BoardPhotoDialog { name: name.into(), rgba: rgba.into_raw(), w, h, corners: Vec::new(), tex: None })
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut open = true;
        let mut done = false;
        let tex = self.tex.get_or_insert_with(|| ctx.load_texture("board-photo-src", egui::ColorImage::from_rgba_unmultiplied([self.w, self.h], &self.rgba), egui::TextureOptions::LINEAR)).clone();
        let mut ok = false;
        crate::widgets::Dialog::new(format!("Photo of the board: {}", self.name)).open(&mut open).width(760.0).show(ctx, |ui| {
            let next = self.corners.len();
            ui.label(if next < 4 {
                RichText::new(format!("Click corner {} of 4: the board's {}", next + 1, CORNERS[next])).strong()
            } else {
                RichText::new("All four corners set: OK straightens the photo onto the board").strong()
            });
            ui.label(RichText::new("Take the photo from above, the whole board in view; any angle works. Bottom = the edge where the X/Y zero is set (bottom-left corner).").small().color(crate::theme::TEXT_DIM));
            // the photo, fitted
            let room = egui::vec2(720.0, 460.0);
            let k = (room.x / self.w as f32).min(room.y / self.h as f32);
            let size = egui::vec2(self.w as f32 * k, self.h as f32 * k);
            let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
            let p = ui.painter_at(rect);
            p.image(tex.id(), rect, egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            let to_screen = |c: Pos2| rect.min + (c.to_vec2() * k);
            if let (true, Some(at)) = (resp.clicked() && next < 4, resp.interact_pointer_pos()) {
                self.corners.push(Pos2::ZERO + (at - rect.min) / k);
            }
            let pts: Vec<Pos2> = self.corners.iter().map(|c| to_screen(*c)).collect();
            if pts.len() > 1 {
                let mut line = pts.clone();
                if line.len() == 4 {
                    line.push(line[0]);
                }
                p.add(egui::Shape::line(line, Stroke::new(2.0, crate::theme::ACCENT)));
            }
            for (i, q) in pts.iter().enumerate() {
                p.circle_filled(*q, 6.0, crate::theme::ACCENT);
                p.text(*q + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, format!("{}", i + 1), crate::theme::font(13.0), Color32::WHITE);
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.corners.is_empty(), egui::Button::new("Undo corner")).clicked() {
                    self.corners.pop();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        done = true;
                    }
                    if ui.add_enabled(self.corners.len() == 4, crate::widgets::Button::new("OK").primary(true)).clicked() {
                        ok = true;
                    }
                });
            });
        });
        if ok {
            let m = &host.canvas.doc.material;
            let c: [Pos2; 4] = [self.corners[0], self.corners[1], self.corners[2], self.corners[3]];
            match straighten(&self.rgba, self.w, self.h, c, m.w, m.h) {
                Some((rgba, w, h)) => {
                    host.canvas.board_photo = Some(BoardPhoto { rgba, w, h, sheet: host.canvas.doc.active_sheet, show: true, tex: None });
                    *host.message = "Board photo under the drawing (this session). View › Photo of the Board shows / hides it.".into();
                }
                None => *host.message = "Those four corners don't make a board shape: try again, in the order asked".into(),
            }
            done = true;
        }
        open && !done
    }
}

/// dev (GCARVE_BOARD_PHOTO): a photo straightened through corners given as 8 numbers, without the dialog
#[cfg(not(target_arch = "wasm32"))]
pub fn dev_photo(bytes: &[u8], n: &[f32], doc: &gcarve_core::document::Document) -> Option<BoardPhoto> {
    let d = BoardPhotoDialog::new("dev", bytes).ok()?;
    let c = [Pos2::new(n[0], n[1]), Pos2::new(n[2], n[3]), Pos2::new(n[4], n[5]), Pos2::new(n[6], n[7])];
    let (rgba, w, h) = straighten(&d.rgba, d.w, d.h, c, doc.material.w, doc.material.h)?;
    Some(BoardPhoto { rgba, w, h, sheet: doc.active_sheet, show: true, tex: None })
}

/// The perspective fit taking board points (inches, Y up) to photo pixels through four corner pairs, as the 3 × 3
/// matrix [a b c; d e f; g h 1] (row major, 8 numbers); None when the points are degenerate.
pub fn homography(board: [(f64, f64); 4], photo: [(f64, f64); 4]) -> Option<[f64; 8]> {
    // two rows per pair: a x + b y + c - g x u - h y u = u, d x + e y + f - g x v - h y v = v
    let mut m = [[0.0f64; 9]; 8];
    for k in 0..4 {
        let ((x, y), (u, v)) = (board[k], photo[k]);
        m[2 * k] = [x, y, 1.0, 0.0, 0.0, 0.0, -x * u, -y * u, u];
        m[2 * k + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -x * v, -y * v, v];
    }
    // Gaussian elimination with partial pivoting
    for col in 0..8 {
        let piv = (col..8).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        if m[piv][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, piv);
        let pivot = m[col];
        for (r, row) in m.iter_mut().enumerate() {
            if r != col {
                let f = row[col] / pivot[col];
                for (x, p) in row.iter_mut().zip(pivot.iter()).skip(col) {
                    *x -= f * p;
                }
            }
        }
    }
    let mut h = [0.0; 8];
    for (i, v) in h.iter_mut().enumerate() {
        *v = m[i][8] / m[i][i];
    }
    h.iter().all(|v| v.is_finite()).then_some(h)
}

/// The photo resampled onto the board (top row first, `OUT_PX` on the long side): every output pixel's board point
/// goes through the fit into the photo, bilinear.
fn straighten(src: &[u8], sw: usize, sh: usize, c: [Pos2; 4], bw: f64, bh: f64) -> Option<(Vec<u8>, usize, usize)> {
    if bw <= 0.0 || bh <= 0.0 {
        return None;
    }
    let photo = c.map(|p| (p.x as f64, p.y as f64));
    let h = homography([(0.0, 0.0), (bw, 0.0), (bw, bh), (0.0, bh)], photo)?;
    let k = OUT_PX / bw.max(bh);
    let (w, hh) = ((bw * k).round().max(1.0) as usize, (bh * k).round().max(1.0) as usize);
    let mut out = vec![0u8; w * hh * 4];
    let at = |x: usize, y: usize, ch: usize| f64::from(src[(y.min(sh - 1) * sw + x.min(sw - 1)) * 4 + ch]);
    for oy in 0..hh {
        for ox in 0..w {
            let (x, y) = ((ox as f64 + 0.5) / k, bh - (oy as f64 + 0.5) / k);
            let d = h[6] * x + h[7] * y + 1.0;
            let (u, v) = ((h[0] * x + h[1] * y + h[2]) / d, (h[3] * x + h[4] * y + h[5]) / d);
            if !(u >= 0.0 && v >= 0.0 && u < sw as f64 - 1.0 && v < sh as f64 - 1.0) {
                continue; // outside the photo: transparent
            }
            let (x0, y0) = (u.floor() as usize, v.floor() as usize);
            let (fx, fy) = (u - x0 as f64, v - y0 as f64);
            for ch in 0..4 {
                let top = at(x0, y0, ch) * (1.0 - fx) + at(x0 + 1, y0, ch) * fx;
                let bot = at(x0, y0 + 1, ch) * (1.0 - fx) + at(x0 + 1, y0 + 1, ch) * fx;
                out[(oy * w + ox) * 4 + ch] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some((out, w, hh))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fit_takes_each_board_corner_to_its_photo_corner() {
        // a 48 × 24 board seen in perspective (the far edge narrower), photo Y down
        let board = [(0.0, 0.0), (48.0, 0.0), (48.0, 24.0), (0.0, 24.0)];
        let photo = [(100.0, 900.0), (1500.0, 880.0), (1300.0, 300.0), (300.0, 320.0)];
        let h = homography(board, photo).unwrap();
        for (b, p) in board.iter().zip(photo) {
            let d = h[6] * b.0 + h[7] * b.1 + 1.0;
            let (u, v) = ((h[0] * b.0 + h[1] * b.1 + h[2]) / d, (h[3] * b.0 + h[4] * b.1 + h[5]) / d);
            assert!((u - p.0).abs() < 1e-6 && (v - p.1).abs() < 1e-6, "{b:?} -> ({u}, {v}) not {p:?}");
        }
        // three corners in a line: no fit
        assert!(homography(board, [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0)]).is_none());
        // straightening a 2 × 2 photo whose corners are the board's gives a picture of the board's shape
        let src = vec![200u8; 64 * 64 * 4];
        let (_, w, hh) = straighten(&src, 64, 64, [Pos2::new(0.0, 63.0), Pos2::new(63.0, 63.0), Pos2::new(63.0, 0.0), Pos2::new(0.0, 0.0)], 48.0, 24.0).unwrap();
        assert_eq!((w, hh), (1600, 800));
    }
}
