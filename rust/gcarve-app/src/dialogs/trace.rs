//! Trace Bitmap (VCarve-style): a PNG / JPEG becomes closed vectors. Live preview over the picture while the
//! threshold, invert, smoothing, speck size and output width change; OK adds the outlines as one group on a
//! "Traced <name>" layer, centred on the board. The tracing itself is gcarve_core::trace.

use super::{hint, unit_label, Host};
use eframe::egui::{self, Color32, RichText, Stroke};
use gcarve_core::geom::{bounds, transformed, Affine, Contour};
use gcarve_core::trace::{otsu, trace, TraceOptions};

/// Longest side traced (bigger pictures are scaled down first: same outlines, a fraction of the time).
const MAX_PX: u32 = 1600;

pub struct TraceDialog {
    name: String,
    gray: Vec<u8>,
    w: usize,
    h: usize,
    auto: u8,
    threshold: i32,
    opts: TraceOptions,
    width_text: String,
    preview: Vec<Contour>,
    dirty: bool,
    texture: Option<egui::TextureHandle>,
}

/// A PNG / JPEG as 8-bit gray (transparent = white), scaled down to `max_px` on the long side. Also Photo V-Carve.
pub fn load_gray(name: &str, bytes: &[u8], max_px: u32) -> Result<(Vec<u8>, usize, usize), String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("{name}: {e}"))?;
    let img = if img.width().max(img.height()) > max_px { img.resize(max_px, max_px, image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let gray = rgba
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            let l = (0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)) * f64::from(a) / 255.0 + 255.0 * (1.0 - f64::from(a) / 255.0);
            l.round().clamp(0.0, 255.0) as u8
        })
        .collect();
    Ok((gray, w, h))
}

impl TraceDialog {
    /// Decodes the picture (transparent = white); Err = not an image GCarve can read.
    pub fn new(name: &str, bytes: &[u8], doc: &gcarve_core::document::Document) -> Result<Self, String> {
        let (gray, w, h) = load_gray(name, bytes, MAX_PX)?;
        let auto = otsu(&gray);
        // a sensible size: 60% of the board width, at most 12 in
        let width_in = (doc.material.w * 0.6).clamp(1.0, 12.0);
        let opts = TraceOptions { width_in, ..Default::default() };
        Ok(TraceDialog { name: name.into(), width_text: crate::units::text(doc, width_in), gray, w, h, auto, threshold: i32::from(auto), opts, preview: Vec::new(), dirty: true, texture: None })
    }

    fn retrace(&mut self) {
        self.opts.threshold = Some(self.threshold.clamp(1, 255) as u8);
        self.preview = trace(&self.gray, self.w, self.h, &self.opts);
        self.dirty = false;
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        if self.dirty {
            self.retrace();
        }
        let tex = self.texture.get_or_insert_with(|| {
            let rgba: Vec<u8> = self.gray.iter().flat_map(|&g| [g, g, g, 255]).collect();
            ctx.load_texture("trace-src", egui::ColorImage::from_rgba_unmultiplied([self.w, self.h], &rgba), egui::TextureOptions::LINEAR)
        });
        let tex_id = tex.id();
        let doc = &host.canvas.doc;
        let (mut ok, mut keep) = (false, true);
        let mut changed = false;
        crate::widgets::Dialog::new(format!("Trace image: {}", self.name)).id("trace").width(640.0).show(ctx, |ui| {
            // preview: the picture dimmed, traced outlines on top
            let avail = egui::vec2(640.0, 360.0);
            let k = (avail.x / self.w as f32).min(avail.y / self.h as f32);
            let size = egui::vec2(self.w as f32 * k, self.h as f32 * k);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(avail.x, size.y), egui::Sense::hover());
            let r = egui::Rect::from_center_size(rect.center(), size);
            let p = ui.painter_at(rect);
            p.rect_filled(r, 0.0, Color32::WHITE);
            p.image(tex_id, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::from_white_alpha(90));
            let s = r.width() as f64 / self.opts.width_in; // screen px per inch
            let to = |x: f64, y: f64| egui::pos2(r.left() + (x * s) as f32, r.bottom() - (y * s) as f32);
            let nodes: usize = self.preview.iter().map(|c| c.nodes.len()).sum();
            for c in &self.preview {
                let pts: Vec<egui::Pos2> = c.nodes.iter().map(|n| to(n.p.x, n.p.y)).collect();
                p.add(egui::Shape::closed_line(pts, Stroke::new(1.5, Color32::from_rgb(0xe8, 0x6a, 0x10))));
            }
            ui.label(RichText::new(format!("{} outline(s), {} nodes · {} × {} px", self.preview.len(), nodes, self.w, self.h)).small().color(crate::theme::TEXT_DIM));
            ui.add_space(4.0);
            egui::Grid::new("trace").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                ui.label("Threshold");
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::Slider::new(&mut self.threshold, 1..=255)).changed();
                    if ui.add_enabled(self.threshold != i32::from(self.auto), egui::Button::new("Auto")).clicked() {
                        self.threshold = i32::from(self.auto);
                        changed = true;
                    }
                });
                ui.end_row();
                ui.label("");
                changed |= crate::widgets::checkbox(ui, &mut self.opts.invert, "Trace the light parts (invert)").changed();
                ui.end_row();
                ui.label("Smoothing");
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::Slider::new(&mut self.opts.smooth_px, 0.3..=4.0).fixed_decimals(1)).changed();
                    hint(ui, "higher = fewer nodes, softer corners");
                });
                ui.end_row();
                ui.label("Ignore specks under");
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::Slider::new(&mut self.opts.min_area_px, 0.0..=400.0).fixed_decimals(0).suffix(" px²")).changed();
                });
                ui.end_row();
                ui.label(unit_label("Width", doc));
                ui.horizontal(|ui| {
                    if let Some(v) = crate::units::num_field(ui, "trace-w", &mut self.width_text, doc, true, 110.0) {
                        if v > 0.0 && (v - self.opts.width_in).abs() > 1e-9 {
                            self.opts.width_in = v;
                            changed = true;
                        }
                    }
                    let hgt = self.opts.width_in * self.h as f64 / self.w as f64;
                    hint(ui, &format!("height {} (keeps the picture's proportions)", crate::units::text(doc, hgt)));
                });
                ui.end_row();
            });
            hint(ui, "High-contrast artwork (logos, lettering, silhouettes) traces best. The outlines land centred on the board as one group on their own layer.");
            let (o, c) = crate::widgets::ok_cancel_text(ui, "Add vectors");
            ok = o && !self.preview.is_empty();
            keep = !c;
        });
        if changed {
            self.dirty = true;
        }
        if ok {
            let doc = &mut host.canvas.doc;
            let mut all = gcarve_core::geom::Rect::default();
            for c in &self.preview {
                all.add_rect(&bounds(c));
            }
            let m = Affine::translate((doc.material.w - all.w()) / 2.0 - all.x0, (doc.material.h - all.h()) / 2.0 - all.y0);
            doc.checkpoint();
            let stem = self.name.rsplit_once('.').map_or(self.name.as_str(), |(a, _)| a);
            let layer = doc.layer_named(&format!("Traced {stem}"));
            let group = doc.new_group_id();
            let mut ids = std::collections::BTreeSet::new();
            for c in &self.preview {
                let id = doc.add(transformed(c, &m), Some(layer));
                if let Some(s) = doc.find_mut(id) {
                    s.group = group;
                }
                ids.insert(id);
            }
            *host.message = format!("Traced {}: {} vectors", self.name, ids.len());
            host.canvas.sel = ids;
            host.canvas.mutated();
            return false;
        }
        keep
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod photo_look {
    /// Visual check of Photo V-Carve: GCARVE_PHOTO_OUT=out.png cargo test -p gcarve-app photo_sim -- --ignored
    #[test]
    #[ignore]
    fn photo_sim() {
        use gcarve_core::document::{Material, Op, OpType, Tool, ToolType};
        let Ok(out) = std::env::var("GCARVE_PHOTO_OUT") else { return };
        // a lit ball on a soft gradient: every tone from white to black
        let (w, h) = (400usize, 300usize);
        let g: Vec<u8> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f64, (i / w) as f64);
                let (dx, dy) = ((x - 200.0) / 110.0, (y - 150.0) / 110.0);
                let r2 = dx * dx + dy * dy;
                let v = if r2 < 1.0 { let z = (1.0 - r2).sqrt(); (0.15 + 0.85 * (-0.5 * dx - 0.5 * dy + 0.7 * z).max(0.0)).min(1.0) } else { 0.35 + 0.6 * x / w as f64 };
                (v * 255.0) as u8
            })
            .collect();
        let op = Op { kind: OpType::Photo, name: "P".into(), depth: 0.06, photo: gcarve_core::photo::b64_encode(&g), photo_w: w as u32, photo_h: h as u32, ..Default::default() };
        let tool = Tool { name: "V60".into(), kind: ToolType::VBit, diameter: 0.5, angle: 60.0, ..Default::default() };
        let m = Material { w: 12.0, h: 12.0 * h as f64 / w as f64, t: 0.75, ..Default::default() };
        let r = gcarve_core::cam::generate(&op, &tool, &[gcarve_core::geom::make_rect(0.0, 0.0, m.w, m.h)], &m, None);
        save_sim(&[(&r.paths, &tool)], &m, &out);
    }

    fn save_sim(cuts: &[(&Vec<gcarve_core::Toolpath>, &gcarve_core::document::Tool)], m: &gcarve_core::document::Material, out: &str) {
        let paths: Vec<_> = cuts.iter().flat_map(|(ps, t)| ps.iter().map(move |p| (p, *t))).collect();
        let hf = gcarve_core::sim::simulate(&paths, m, 1_200_000);
        let rgba = gcarve_core::sim::shade(&hf, m);
        image::RgbaImage::from_raw(hf.nx as u32, hf.ny as u32, rgba).unwrap().save(out).unwrap();
    }

    /// Visual check of bit shapes: the same groove with a round-over, a ball nose, a V-bit and an end mill, left to
    /// right. GCARVE_BITS_OUT=out.png cargo test -p gcarve-app bit_shapes_sim -- --ignored
    #[test]
    #[ignore]
    fn bit_shapes_sim() {
        use gcarve_core::document::{Material, Op, OpType, Side, Tool, ToolType};
        let Ok(out) = std::env::var("GCARVE_BITS_OUT") else { return };
        let m = Material { w: 8.0, h: 4.0, t: 0.75, ..Default::default() };
        let op = Op { kind: OpType::Profile, name: "P".into(), depth: 0.25, side: Side::On, ..Default::default() };
        let tools: Vec<Tool> = [ToolType::RoundOver, ToolType::BallNose, ToolType::VBit, ToolType::EndMill].into_iter().map(|kind| Tool { kind, diameter: 0.5, angle: 90.0, stepdown: 0.25, ..Default::default() }).collect();
        let cuts: Vec<Vec<gcarve_core::Toolpath>> = tools.iter().enumerate().map(|(i, t)| gcarve_core::cam::generate(&op, t, &[gcarve_core::geom::make_rect(0.6 + 1.9 * i as f64, 0.8, 1.9 + 1.9 * i as f64, 3.2)], &m, None).paths).collect();
        save_sim(&cuts.iter().zip(&tools).collect::<Vec<_>>(), &m, &out);
    }
}
