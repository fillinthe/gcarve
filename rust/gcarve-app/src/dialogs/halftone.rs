//! Halftone from a picture (Vectors › Halftone from Picture…): the picture becomes dots on a grid, bigger where it
//! is darker (`photo::halftone`), centred on the board on the "Halftone" layer, with a V-bit drilling toolpath that
//! plunges each dot until the cone is exactly as wide as the dot (`Op::fit_holes`; 90° V-bit from the library, added
//! when missing). One plunge per dot: a V-carve of thousands of circles was a minute of work for the same cones.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::{hint, unit_label, Host};
use crate::canvas::Canvas;
use eframe::egui;
use gcarve_core::document::{Op, OpType, ToolType};
use gcarve_core::geom::{make_ellipse, Affine};
use gcarve_core::photo::{halftone, Gray};

pub struct HalftoneDialog {
    name: String,
    gray: Gray,
    /// width, spacing
    size: [String; 2],
    /// biggest dot, % of the spacing
    max: f64,
    invert: bool,
    hex: bool,
}

impl HalftoneDialog {
    pub fn new(name: &str, bytes: &[u8], doc: &gcarve_core::document::Document) -> Result<Self, String> {
        let (px, w, h) = super::trace::load_gray(name, bytes, 800)?;
        let width = (doc.material.w - 2.0).clamp(2.0, 24.0);
        Ok(HalftoneDialog { name: name.to_string(), gray: Gray { w, h, px }, size: [width, 0.25].map(|v| crate::units::text(doc, v)), max: 95.0, invert: false, hex: true })
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let (mut open, mut ok, mut cancel) = (true, false, false);
        let doc = &host.canvas.doc;
        let n: Vec<f64> = self.size.iter().map(|s| crate::units::get(doc, s, true).unwrap_or(0.0)).collect();
        let dots = halftone(&self.gray, n[0], n[1], self.max / 100.0, self.invert, self.hex).len();
        crate::widgets::Dialog::new(i18n::t("Halftone from Picture")).open(&mut open).width(400.0).show(ctx, |ui| {
            ui.label(egui::RichText::new(&self.name).strong());
            egui::Grid::new("halftone").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                for (i, label) in ["Width", "Dot spacing"].iter().enumerate() {
                    ui.label(unit_label(label, doc));
                    crate::units::num_field(ui, label, &mut self.size[i], doc, true, 110.0);
                    ui.end_row();
                }
                ui.label(i18n::t("Biggest dot"));
                ui.add(egui::Slider::new(&mut self.max, 40.0..=100.0).suffix(i18n::t(" % of the spacing")));
                ui.end_row();
            });
            crate::widgets::checkbox(ui, &mut self.invert, i18n::t("Invert (light parts get the dots: for a dark board)"));
            crate::widgets::checkbox(ui, &mut self.hex, i18n::t("Staggered rows (rounder look)"));
            hint(ui, &format!("{dots} dots, {} high. Each is one V-bit plunge, the cone as wide as the dot.", crate::units::text(doc, n[0] * self.gray.h as f64 / self.gray.w.max(1) as f64)));
            let (o, c) = crate::widgets::ok_cancel(ui);
            ok = o;
            cancel = c;
        });
        if cancel {
            return false;
        }
        if ok {
            if !(n[0] > 0.0 && n[1] >= 0.03) {
                *host.message = i18n::t("Halftone: a width, and dots at least 0.03 in apart.").into();
            } else if dots > 60_000 {
                *host.message = format!("Halftone: {dots} dots is too many to cut; space them further apart.");
            } else {
                *host.message = make(host.canvas, &self.gray, n[0], n[1], self.max / 100.0, self.invert, self.hex);
                return false;
            }
        }
        open
    }
}

/// Adds the dots and their drilling toolpath; returns the status line
pub fn make(c: &mut Canvas, g: &Gray, width: f64, spacing: f64, max: f64, invert: bool, hex: bool) -> String {
    let dots = halftone(g, width, spacing, max, invert, hex);
    if dots.is_empty() {
        return i18n::t("Halftone: the picture came out blank at these settings.").into();
    }
    let height = width * g.h as f64 / g.w as f64;
    let m = &c.doc.material;
    let at = Affine::translate((m.w - width) / 2.0, (m.h - height) / 2.0);
    c.doc.checkpoint();
    let vbit = super::sign::bit(c, ToolType::VBit, 0.5, i18n::t("90° V-Bit (0.5 inch)"));
    let layer = c.doc.layer_named(i18n::t("Halftone"));
    let group = c.doc.new_group_id();
    let mut ids = std::collections::BTreeSet::new();
    let mut r_max: f64 = 0.0;
    for (p, r) in &dots {
        let q = at.apply(*p);
        let id = c.doc.add(make_ellipse(q.x, q.y, *r, *r), Some(layer));
        if let Some(s) = c.doc.find_mut(id) {
            s.group = group;
        }
        ids.insert(id);
        r_max = r_max.max(*r);
    }
    let half = (c.doc.tools[vbit as usize].angle / 2.0).to_radians().tan().max(1e-6);
    c.doc.ops.push(Op { kind: OpType::Drill, name: i18n::t("Halftone").into(), tool: vbit, layer_scope: layer, depth: r_max / half + 0.01, fit_holes: true, ..Default::default() });
    c.sel = ids;
    c.sel_changed();
    c.mutated();
    format!("Halftone: {} dots on the \"Halftone\" layer with a V-bit drilling toolpath (deepest {}). A smooth, even board shows it best.", dots.len(), crate::units::text(&c.doc, r_max / half))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_halftone_lands_centred_with_a_v_carve_that_cones_every_dot() {
        let (w, h) = (60usize, 40usize);
        // a dark disc on white
        let px = (0..w * h).map(|i| { let (x, y) = ((i % w) as f64 - 30.0, (i / w) as f64 - 20.0); if x.hypot(y) < 15.0 { 20 } else { 250 } }).collect();
        let g = Gray { w, h, px };
        let mut c = Canvas::default();
        (c.doc.material.w, c.doc.material.h) = (12.0, 10.0);
        let msg = make(&mut c, &g, 6.0, 0.25, 0.9, false, true);
        assert!(msg.starts_with("Halftone: "), "{msg}");
        let mut b = gcarve_core::geom::Rect::default();
        c.sel.iter().for_each(|id| b.add_rect(&gcarve_core::geom::bounds(&c.doc.find(*id).unwrap().c)));
        assert!(((b.x0 + b.x1) / 2.0 - 6.0).abs() < 0.3 && ((b.y0 + b.y1) / 2.0 - 5.0).abs() < 0.3, "centred: {b:?}");
        let gen = gcarve_core::pipeline::regenerate(&c.doc);
        assert!(gen.warnings.is_empty(), "{:?}", gen.warnings);
        assert!(!gen.paths[0].is_empty());
    }

    /// look: GCARVE_HALFTONE_IN=photo.jpg GCARVE_HALFTONE_OUT=x.png cargo test -p gcarve-app halftone_picture -- --ignored
    #[test]
    #[ignore]
    fn halftone_picture() {
        let (Ok(inp), Ok(out)) = (std::env::var("GCARVE_HALFTONE_IN"), std::env::var("GCARVE_HALFTONE_OUT")) else { return };
        let d = HalftoneDialog::new(&inp, &std::fs::read(&inp).unwrap(), &gcarve_core::document::Document::default()).unwrap();
        let mut c = Canvas::default();
        (c.doc.material.w, c.doc.material.h) = (14.0, 14.0 * d.gray.h as f64 / d.gray.w as f64 + 2.0);
        println!("{}", make(&mut c, &d.gray, 12.0, 0.15, 0.95, false, true));
        let gen = gcarve_core::pipeline::regenerate(&c.doc);
        let (rgba, w, h, _) = crate::setupsheet::carved(&c.doc, &gen, 900_000).unwrap();
        image::RgbaImage::from_raw(w, h, rgba).unwrap().save(out).unwrap();
    }
}
