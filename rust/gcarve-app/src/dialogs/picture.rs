//! View › Picture to Draw Over… (VCarve's Import Bitmap): a picture placed on the sheet under the vectors, saved with
//! the project (`Document::pictures`), to draw over by hand. Placed over the selection (else fitted on the board);
//! this dialog moves, sizes (aspect kept), fades or removes it. Never cut.

use super::Host;
use crate::units;
use eframe::egui;
use gcarve_core::document::{Document, Picture};
use gcarve_core::geom::Rect;
use gcarve_ui::i18n;

/// the file is kept as it is when it is at most this big on its long side and in bytes (a PNG keeps its
/// transparency), else it becomes a 1600 px JPEG
const KEEP_PX: u32 = 1600;
const KEEP_BYTES: usize = 2_000_000;

pub struct PictureDialog {
    /// index into `doc.pictures`
    i: usize,
    x: String,
    y: String,
    w: String,
}

/// a picture of `aspect` (width / height) fitted inside `area`, centred: (x, y, w, h)
fn fit(aspect: f64, area: Rect) -> (f64, f64, f64, f64) {
    let (w, h) = if area.w() / area.h().max(1e-9) > aspect { (area.h() * aspect, area.h()) } else { (area.w(), area.w() / aspect) };
    (area.x0 + (area.w() - w) / 2.0, area.y0 + (area.h() - h) / 2.0, w, h)
}

fn board(doc: &Document) -> Rect {
    Rect { x0: 0.0, y0: 0.0, x1: doc.material.w, y1: doc.material.h }
}

/// Adds the picked file as a picture on the active sheet, over the selection or fitted on the board
pub fn add(name: &str, bytes: &[u8], host: &mut Host) -> Result<PictureDialog, String> {
    let img = crate::files::picture(bytes).map_err(|e| format!("{name}: {e}"))?;
    let (pw, ph) = (img.width(), img.height());
    let data = if pw.max(ph) <= KEEP_PX && bytes.len() <= KEEP_BYTES { bytes.to_vec() } else { super::community::photo_jpeg(bytes)? };
    let c = &mut *host.canvas;
    let area = if c.sel.is_empty() || c.sel_rect.is_empty() { board(&c.doc) } else { c.sel_rect };
    let (x, y, w, h) = fit(pw as f64 / ph.max(1) as f64, area);
    c.doc.pictures.push(Picture { sheet: c.doc.active_sheet, x, y, w, h, opacity: 0.5, data: gcarve_core::photo::b64_encode(&data) });
    c.dirty = true;
    *host.message = i18n::t("Picture under the drawing, saved with the project: draw over it, then View › Picture Settings… to move, fade or remove it.").into();
    Ok(PictureDialog::new(&c.doc, c.doc.pictures.len() - 1))
}

impl PictureDialog {
    pub fn new(doc: &Document, i: usize) -> Self {
        let p = &doc.pictures[i];
        PictureDialog { i, x: units::fmt(doc, p.x), y: units::fmt(doc, p.y), w: units::fmt(doc, p.w) }
    }

    /// the first picture on the active sheet
    pub fn for_sheet(doc: &Document) -> Option<Self> {
        doc.pictures.iter().position(|p| p.sheet == doc.active_sheet).map(|i| Self::new(doc, i))
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut open = true;
        let c = &mut *host.canvas;
        if c.doc.pictures.get(self.i).is_none() {
            return false;
        }
        c.pic_pick = (3, self.i);
        let mut remove = false;
        let mut refresh = c.pic_drag.is_some(); // the fields follow a drag on the drawing
        crate::widgets::Dialog::new(i18n::t("Picture to Draw Over")).open(&mut open).close_footer().modeless(ctx.content_rect().right_top() + egui::vec2(-330.0, 120.0), egui::Align2::RIGHT_TOP).show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
            egui::Grid::new("picture-grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                for (label, k) in [(i18n::t("X (left)"), 0), (i18n::t("Y (bottom)"), 1), (i18n::t("Width"), 2)] {
                    ui.label(label);
                    let buf = match k { 0 => &mut self.x, 1 => &mut self.y, _ => &mut self.w };
                    if let Some(v) = units::num_field(ui, &format!("picture-{k}"), buf, &c.doc, true, 110.0) {
                        let p = &mut c.doc.pictures[self.i];
                        match k {
                            0 => p.x = v,
                            1 => p.y = v,
                            _ if v > 0.0 => {
                                p.h *= v / p.w;
                                p.w = v;
                            }
                            _ => {}
                        }
                        refresh = true;
                    }
                    ui.end_row();
                }
                ui.label(i18n::t("Height"));
                ui.label(units::fmt(&c.doc, c.doc.pictures[self.i].h)).on_hover_text(i18n::t("Follows the width: the picture keeps its shape"));
                ui.end_row();
                ui.label(i18n::t("Strength"));
                let p = &mut c.doc.pictures[self.i];
                let mut pct = (p.opacity * 100.0).round();
                if ui.add(egui::Slider::new(&mut pct, 10.0..=100.0).suffix(" %")).changed() {
                    p.opacity = pct / 100.0;
                    c.dirty = true;
                }
                ui.end_row();
            });
            ui.horizontal(|ui| {
                let aspect = { let p = &c.doc.pictures[self.i]; p.w / p.h.max(1e-9) };
                if ui.button(i18n::t("Fit to the board")).clicked() {
                    let (x, y, w, h) = fit(aspect, board(&c.doc));
                    let p = &mut c.doc.pictures[self.i];
                    (p.x, p.y, p.w, p.h) = (x, y, w, h);
                    refresh = true;
                }
                let has_sel = !c.sel.is_empty() && !c.sel_rect.is_empty();
                if ui.add_enabled(has_sel, egui::Button::new(i18n::t("Fit to the selection"))).clicked() {
                    let (x, y, w, h) = fit(aspect, c.sel_rect);
                    let p = &mut c.doc.pictures[self.i];
                    (p.x, p.y, p.w, p.h) = (x, y, w, h);
                    refresh = true;
                }
                if ui.button(i18n::t("Remove")).clicked() {
                    remove = true;
                }
            });
            crate::dialogs::hint(ui, i18n::t("Drag it on the drawing to move it. Only a guide to draw over: it is never cut and doesn't show in the G-code. Trace Image… makes vectors from a picture by itself."));
        });
        if remove {
            c.doc.pictures.remove(self.i);
            c.dirty = true;
            return false;
        }
        if refresh {
            c.dirty = true;
            *self = Self::new(&c.doc, self.i);
        }
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// real pointer events through egui: with Picture Settings open (pic_pick), a drag on the picture moves it
    #[test]
    fn dragging_the_picture_moves_it_while_its_settings_are_open() {
        use crate::canvas::Canvas;
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(8, 4, image::Rgba([0, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let mut c = Canvas::default();
        c.doc.pictures.push(Picture { sheet: 0, x: 10.0, y: 6.0, w: 8.0, h: 4.0, opacity: 0.5, data: gcarve_core::photo::b64_encode(&png) });
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0));
        let frame = |c: &mut Canvas, events: Vec<egui::Event>| {
            c.pic_pick = (3, 0);
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    c.ui(ui, None, None);
                });
            });
            out.textures_delta.clear();
        };
        frame(&mut c, vec![]);
        frame(&mut c, vec![]);
        let v = c.view.as_ref().expect("the canvas has a view");
        let (from, zoom) = (v.to_screen(gcarve_core::geom::pt(14.0, 8.0)), v.zoom);
        let to = from + egui::vec2(2.0 * zoom, 0.0); // 2 in to the right
        let btn = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut c, vec![egui::Event::PointerMoved(from), btn(from, true)]);
        for k in 1..=5 {
            frame(&mut c, vec![egui::Event::PointerMoved(from + (to - from) * k as f32 / 5.0)]);
        }
        frame(&mut c, vec![btn(to, false)]);
        let p = &c.doc.pictures[0];
        assert!((p.x - 12.0).abs() < 0.05 && (p.y - 6.0).abs() < 0.05, "picture at {}, {}", p.x, p.y);
        assert!(c.dirty);
    }

    #[test]
    fn a_picture_fits_inside_its_area_centred_and_keeps_its_shape() {
        let area = Rect { x0: 0.0, y0: 0.0, x1: 48.0, y1: 24.0 };
        assert_eq!(fit(1.0, area), (12.0, 0.0, 24.0, 24.0));
        assert_eq!(fit(4.0, area), (0.0, 6.0, 48.0, 12.0));
    }
}
