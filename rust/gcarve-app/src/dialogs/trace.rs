//! Trace Bitmap (VCarve-style): a PNG / JPEG becomes closed vectors, or single centre lines (Centreline). Live preview over the picture while the
//! threshold, invert, smoothing, speck size, output width and the traced part (drag a box) change; OK adds the outlines as one group on a
//! "Traced <name>" layer, centred on the board. The tracing itself is gcarve_core::trace.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
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
    /// trace only this part of the picture (VCarve "trace a selected area"): x0, y0, x1, y1 in pixels, top-left origin
    crop: Option<[usize; 4]>,
    /// drag on the preview in progress: its start, in pixels
    drag_from: Option<(f32, f32)>,
    /// the picture in colour (same size as `gray`), for the preview and for tracing one colour
    rgb: Vec<[u8; 3]>,
    /// trace by colour (VCarve "tracing colour images"): the picked colour; None = dark parts by threshold
    colour: Option<[u8; 3]>,
    /// how far from the picked colour still counts (RGB distance)
    tolerance: f32,
    /// colour mode chosen, waiting for a click on the picture
    by_colour: bool,
}

/// The same picture in colour at the size `load_gray` made it (transparent = white); gray-filled when it can't be read
fn load_rgb(bytes: &[u8], w: usize, h: usize) -> Vec<[u8; 3]> {
    let Ok(img) = image::load_from_memory(bytes) else { return vec![[255; 3]; w * h] };
    let img = if img.width() as usize != w || img.height() as usize != h { img.resize_exact(w as u32, h as u32, image::imageops::FilterType::Triangle) } else { img };
    img.to_rgba8().pixels().map(|p| { let [r, g, b, a] = p.0; let k = f32::from(a) / 255.0; let mix = |c: u8| (f32::from(c) * k + 255.0 * (1.0 - k)).round() as u8; [mix(r), mix(g), mix(b)] }).collect()
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
        let rgb = load_rgb(bytes, w, h);
        let auto = otsu(&gray);
        // a sensible size: 60% of the board width, at most 12 in
        let width_in = (doc.material.w * 0.6).clamp(1.0, 12.0);
        let opts = TraceOptions { width_in, ..Default::default() };
        Ok(TraceDialog { name: name.into(), width_text: crate::units::text(doc, width_in), gray, w, h, auto, threshold: i32::from(auto), opts, preview: Vec::new(), dirty: true, texture: None, crop: None, drag_from: None, rgb, colour: None, tolerance: 60.0, by_colour: false })
    }

    fn retrace(&mut self) {
        self.opts.threshold = Some(self.threshold.clamp(1, 255) as u8);
        let [x0, y0, x1, y1] = self.area();
        // by colour: pixels near the picked colour are the "ink" (0), the rest paper (255), traced at the middle
        let src: Vec<u8> = match (self.by_colour, self.colour) {
            (true, Some(c)) => {
                self.opts.threshold = Some(128);
                let t2 = self.tolerance * self.tolerance;
                self.rgb.iter().map(|p| if (0..3).map(|i| (f32::from(p[i]) - f32::from(c[i])).powi(2)).sum::<f32>() <= t2 { 0 } else { 255 }).collect()
            }
            _ => self.gray.clone(),
        };
        let sub: Vec<u8> = (y0..y1).flat_map(|y| src[y * self.w + x0..y * self.w + x1].iter().copied()).collect();
        self.preview = trace(&sub, x1 - x0, y1 - y0, &self.opts);
        self.dirty = false;
    }

    /// the traced part: the crop, or the whole picture
    fn area(&self) -> [usize; 4] {
        self.crop.unwrap_or([0, 0, self.w, self.h])
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        if self.dirty {
            self.retrace();
        }
        let tex = self.texture.get_or_insert_with(|| {
            let rgba: Vec<u8> = self.rgb.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
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
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail.x, size.y), egui::Sense::click_and_drag());
            let r = egui::Rect::from_center_size(rect.center(), size);
            let p = ui.painter_at(rect);
            p.rect_filled(r, 0.0, Color32::WHITE);
            p.image(tex_id, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::from_white_alpha(90));
            // drag a box on the picture = trace only that part
            let px = |q: egui::Pos2| (((q.x - r.left()) / k).clamp(0.0, self.w as f32), ((q.y - r.top()) / k).clamp(0.0, self.h as f32));
            if self.by_colour && resp.clicked() {
                if let Some(q) = resp.interact_pointer_pos() {
                    let (x, y) = px(q);
                    let i = (y as usize).min(self.h - 1) * self.w + (x as usize).min(self.w - 1);
                    self.colour = self.rgb.get(i).copied();
                    changed = true;
                }
            }
            if resp.drag_started() {
                self.drag_from = resp.interact_pointer_pos().map(px);
            }
            let dragged = match (self.drag_from, resp.interact_pointer_pos().map(px)) {
                (Some(a), Some(b)) => Some([a.0.min(b.0) as usize, a.1.min(b.1) as usize, a.0.max(b.0).ceil() as usize, a.1.max(b.1).ceil() as usize]),
                _ => None,
            };
            if resp.drag_stopped() {
                if let Some(c) = dragged.filter(|c| c[2] - c[0] >= 8 && c[3] - c[1] >= 8) {
                    self.crop = Some(c);
                    changed = true;
                }
                self.drag_from = None;
            }
            let [ax0, ay0, ax1, ay1] = dragged.filter(|_| self.drag_from.is_some()).unwrap_or(self.area());
            let area = egui::Rect::from_min_max(r.left_top() + egui::vec2(ax0 as f32 * k, ay0 as f32 * k), r.left_top() + egui::vec2(ax1 as f32 * k, ay1 as f32 * k));
            if self.crop.is_some() || self.drag_from.is_some() {
                p.rect_stroke(area, 0.0, Stroke::new(1.5, crate::theme::ACCENT), egui::StrokeKind::Middle);
            }
            let [x0, _, x1, y1] = self.area();
            let s = (x1 - x0) as f64 * k as f64 / self.opts.width_in; // screen px per inch
            let (left, bottom) = (r.left() + x0 as f32 * k, r.top() + y1 as f32 * k);
            let to = |x: f64, y: f64| egui::pos2(left + (x * s) as f32, bottom - (y * s) as f32);
            let nodes: usize = self.preview.iter().map(|c| c.nodes.len()).sum();
            for c in &self.preview {
                let pts: Vec<egui::Pos2> = c.nodes.iter().map(|n| to(n.p.x, n.p.y)).collect();
                let st = Stroke::new(1.5, Color32::from_rgb(0xe8, 0x6a, 0x10));
                p.add(if c.closed { egui::Shape::closed_line(pts, st) } else { egui::Shape::line(pts, st) });
            }
            ui.horizontal(|ui| {
                let [x0, y0, x1, y1] = self.area();
                let part = if self.crop.is_some() { format!(" · tracing a {} × {} px part", x1 - x0, y1 - y0) } else { i18n::t(" · drag a box on the picture to trace only part of it").into() };
                ui.label(RichText::new(format!("{} outline(s), {} nodes · {} × {} px{part}", self.preview.len(), nodes, self.w, self.h)).small().color(crate::theme::TEXT_DIM));
                if self.crop.is_some() && ui.small_button(i18n::t("Whole picture")).clicked() {
                    self.crop = None;
                    changed = true;
                }
            });
            ui.add_space(4.0);
            egui::Grid::new("trace").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                ui.label(i18n::t("Trace"));
                ui.horizontal(|ui| {
                    changed |= crate::widgets::radio_value(ui, &mut self.by_colour, false, i18n::t("Dark parts")).changed();
                    changed |= crate::widgets::radio_value(ui, &mut self.by_colour, true, i18n::t("One colour")).changed();
                });
                ui.end_row();
                if self.by_colour {
                    ui.label(i18n::t("Colour"));
                    ui.horizontal(|ui| {
                        match self.colour {
                            Some(c) => {
                                let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 18.0), egui::Sense::hover());
                                ui.painter().rect_filled(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]));
                                hint(ui, i18n::t("click the picture to pick another"));
                            }
                            None => hint(ui, i18n::t("click the picture on the colour to trace")),
                        }
                    });
                    ui.end_row();
                    ui.label(i18n::t("Colour range"));
                    changed |= ui.add(egui::Slider::new(&mut self.tolerance, 5.0..=200.0).fixed_decimals(0)).on_hover_text(i18n::t("How different a pixel may be from the picked colour and still count")).changed();
                    ui.end_row();
                }
                ui.add_enabled(!self.by_colour, egui::Label::new(i18n::t("Threshold")));
                ui.add_enabled_ui(!self.by_colour, |ui| ui.horizontal(|ui| {
                    changed |= ui.add(egui::Slider::new(&mut self.threshold, 1..=255)).changed();
                    if ui.add_enabled(self.threshold != i32::from(self.auto), egui::Button::new(i18n::t("Auto"))).clicked() {
                        self.threshold = i32::from(self.auto);
                        changed = true;
                    }
                }));
                ui.end_row();
                ui.label("");
                changed |= crate::widgets::checkbox(ui, &mut self.opts.invert, i18n::t("Trace the light parts (invert)")).changed();
                ui.end_row();
                ui.label("");
                changed |= crate::widgets::checkbox(ui, &mut self.opts.centreline, i18n::t("Centreline: one line down the middle of each stroke")).on_hover_text(i18n::t("For signatures, handwriting and line drawings: single lines to engrave with a V-bit (Profile on the line), a drag knife or a pen, instead of outlines around the ink.")).changed();
                ui.end_row();
                ui.label(i18n::t("Smoothing"));
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::Slider::new(&mut self.opts.smooth_px, 0.3..=4.0).fixed_decimals(1)).changed();
                    hint(ui, i18n::t("higher = fewer nodes, softer corners"));
                });
                ui.end_row();
                ui.label(i18n::t("Ignore specks under"));
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::Slider::new(&mut self.opts.min_area_px, 0.0..=400.0).fixed_decimals(0).suffix(i18n::t(" px²"))).changed();
                });
                ui.end_row();
                ui.label(unit_label(i18n::t("Width"), doc));
                ui.horizontal(|ui| {
                    if let Some(v) = crate::units::num_field(ui, "trace-w", &mut self.width_text, doc, true, 110.0) {
                        if v > 0.0 && (v - self.opts.width_in).abs() > 1e-9 {
                            self.opts.width_in = v;
                            changed = true;
                        }
                    }
                    let [x0, y0, x1, y1] = self.area();
                    let hgt = self.opts.width_in * (y1 - y0) as f64 / (x1 - x0) as f64;
                    hint(ui, &format!("height {} (keeps the picture's proportions)", crate::units::text(doc, hgt)));
                });
                ui.end_row();
            });
            hint(ui, i18n::t("High-contrast artwork (logos, lettering, silhouettes) traces best. The outlines land centred on the board as one group on their own layer."));
            let (o, c) = crate::widgets::ok_cancel_text(ui, i18n::t("Add vectors"));
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

    /// Visual check of sketch carving from a 3D model: GCARVE_SKETCH_STL=model.stl GCARVE_SKETCH_OUT=prefix
    /// cargo test -p gcarve-app sketch_from_model_sim -- --ignored → prefix-shade.png (the picture) + prefix-sim.png
    #[test]
    #[ignore]
    fn sketch_from_model_sim() {
        use gcarve_core::document::{Material, Op, OpType, Relief, Tool, ToolType};
        let (Ok(stl), Ok(out)) = (std::env::var("GCARVE_SKETCH_STL"), std::env::var("GCARVE_SKETCH_OUT")) else { return };
        let tris = gcarve_core::relief::load_stl(&std::fs::read(stl).unwrap()).unwrap();
        let (x0, x1, y0, y1) = tris.iter().flat_map(|t| [t.a, t.b, t.c]).fold((1e300, -1e300, 1e300, -1e300), |b, p| (p.x.min(b.0), p.x.max(b.1), p.y.min(b.2), p.y.max(b.3)));
        let mut r = Relief { w: 12.0, h: 12.0 * (y1 - y0) / (x1 - x0), depth: 0.4, ..Default::default() };
        gcarve_core::relief::rasterize_relief(&tris, &mut r, 400_000);
        let (g, w, h) = gcarve_core::photo::shaded_relief(&r, 600);
        image::GrayImage::from_raw(w as u32, h as u32, g.clone()).unwrap().save(format!("{out}-shade.png")).unwrap();
        let op = Op { kind: OpType::Photo, name: "S".into(), depth: 0.04, sketch: true, photo: gcarve_core::photo::b64_encode(&g), photo_w: w as u32, photo_h: h as u32, ..Default::default() };
        let tool = Tool { name: "V30".into(), kind: ToolType::VBit, diameter: 0.25, angle: 30.0, ..Default::default() };
        let m = Material { w: r.w, h: r.h, t: 0.75, ..Default::default() };
        let res = gcarve_core::cam::generate(&op, &tool, &[gcarve_core::geom::make_rect(0.0, 0.0, m.w, m.h)], &m, None);
        save_sim(&[(&res.paths, &tool)], &m, &format!("{out}-sim.png"));
    }

    /// Visual check of a relief made from a picture: GCARVE_RELIEF_PIC=photo.jpg GCARVE_RELIEF_OUT=out.png [GCARVE_RELIEF_INVERT=1]
    /// cargo test -p gcarve-app relief_picture_sim -- --ignored (ball-nose finish simulated in wood)
    #[test]
    #[ignore]
    fn relief_picture_sim() {
        use gcarve_core::document::{Material, Op, OpType, Relief, Tool, ToolType};
        let (Ok(pic), Ok(out)) = (std::env::var("GCARVE_RELIEF_PIC"), std::env::var("GCARVE_RELIEF_OUT")) else { return };
        let (gray, w, h) = super::load_gray(&pic, &std::fs::read(&pic).unwrap(), 500).unwrap();
        let aspect = w as f64 / h as f64;
        let m = Material { w: 8.0 * aspect.max(1.0), h: 8.0 / aspect.min(1.0), t: 0.75, ..Default::default() };
        let mut r = Relief { id: 1, w: 7.0 * aspect.min(1.0).max(0.5) * aspect.max(1.0) / aspect.max(1.0), h: 0.0, x: 0.5, y: 0.5, depth: 0.25, pic: gcarve_core::photo::b64_encode(&gray), pic_w: w as u32, pic_h: h as u32, pic_invert: std::env::var("GCARVE_RELIEF_INVERT").is_ok(), ..Default::default() };
        r.w = (m.w - 1.0).min((m.h - 1.0) * aspect);
        r.h = r.w / aspect;
        gcarve_core::relief::rasterize_picture(&mut r);
        let op = Op { kind: OpType::Relief, name: "R".into(), relief: 1, stepover: 0.12, rough: false, ..Default::default() };
        let tool = Tool { kind: ToolType::BallNose, diameter: 0.125, stepdown: 0.1, feed_xy: 80.0, feed_z: 30.0, ..Default::default() };
        let res = gcarve_core::relief::generate_relief(&op, &tool, None, &r, &m, None);
        save_sim(&[(&res.paths, &tool)], &m, &out);
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

#[cfg(test)]
mod webp_test {
    #[test]
    fn a_webp_picture_loads_like_a_png() {
        let mut img = image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 255, 255, 255]));
        img.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        let mut bytes = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut bytes).encode(img.as_raw(), 4, 2, image::ExtendedColorType::Rgba8).unwrap();
        let (gray, w, h) = super::load_gray("x.webp", &bytes, 600).unwrap();
        assert_eq!((w, h, gray[0], gray[1]), (4, 2, 0, 255));
    }
}

#[cfg(test)]
mod crop_test {
    #[test]
    fn tracing_one_colour_leaves_the_others() {
        // a red and a blue square, equally dark in gray: by colour only the red one is traced
        let mut img = image::RgbImage::from_pixel(200, 100, image::Rgb([255, 255, 255]));
        for (x0, c) in [(20, [220, 30, 30]), (120, [30, 30, 220])] {
            for y in 30..70 {
                for x in x0..x0 + 40 {
                    img.put_pixel(x, y, image::Rgb(c));
                }
            }
        }
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let doc = gcarve_core::document::Document::default();
        let mut d = super::TraceDialog::new("rb.png", png.get_ref(), &doc).unwrap();
        d.by_colour = true;
        d.colour = Some([220, 30, 30]);
        d.retrace();
        assert_eq!(d.preview.len(), 1);
        let b = gcarve_core::geom::bounds(&d.preview[0]);
        assert!(b.x1 < d.opts.width_in / 2.0, "the red square is on the left: {b:?}");
    }

    #[test]
    fn a_cropped_area_traces_only_what_is_inside() {
        // two black squares on white, side by side
        let mut img = image::GrayImage::from_pixel(200, 100, image::Luma([255]));
        for (x0, x1) in [(20, 60), (120, 160)] {
            for y in 30..70 {
                for x in x0..x1 {
                    img.put_pixel(x, y, image::Luma([0]));
                }
            }
        }
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let doc = gcarve_core::document::Document::default();
        let mut d = super::TraceDialog::new("two.png", png.get_ref(), &doc).unwrap();
        d.retrace();
        assert_eq!(d.preview.len(), 2);
        d.crop = Some([100, 0, 200, 100]);
        d.retrace();
        assert_eq!(d.preview.len(), 1);
        // the traced square spans 40 of the 100 cropped pixels: 40 % of the output width
        let b = gcarve_core::geom::bounds(&d.preview[0]);
        assert!((b.w() / d.opts.width_in - 0.4).abs() < 0.03, "{}", b.w() / d.opts.width_in);
    }

    /// look at a centreline trace: GCARVE_CENTRE_IN=picture.png GCARVE_CENTRE_OUT=overlay.svg cargo test -p gcarve-app centreline_look -- --ignored
    #[test]
    #[ignore]
    fn centreline_look() {
        let (Ok(inp), Ok(out)) = (std::env::var("GCARVE_CENTRE_IN"), std::env::var("GCARVE_CENTRE_OUT")) else { return };
        let (g, w, h) = crate::dialogs::trace::load_gray("x", &std::fs::read(&inp).unwrap(), 1600).unwrap();
        let o = gcarve_core::trace::TraceOptions { centreline: true, width_in: w as f64 / 100.0, ..Default::default() };
        let lines = gcarve_core::trace::trace(&g, w, h, &o);
        let paths: String = lines.iter().map(|c| format!("<polyline points='{}' fill='none' stroke='red' stroke-width='1.5'/>", c.nodes.iter().map(|n| format!("{},{}", n.p.x * 100.0, h as f64 - n.p.y * 100.0)).collect::<Vec<_>>().join(" "))).collect();
        std::fs::write(out, format!("<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}'><image href='file://{inp}' width='{w}' height='{h}' opacity='0.35'/>{paths}</svg>")).unwrap();
        println!("{} lines", lines.len());
    }
}