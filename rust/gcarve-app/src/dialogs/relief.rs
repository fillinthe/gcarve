//! 3D Models (Qt ReliefDialog.qml): import STL, place / size / depth / mirror, remove, and start a 3D Model toolpath.
//! Modeless. Each model is a component: combined in list order (add / subtract / higher / lower), with a lift, a
//! height band and smoothing; a 3D toolpath cuts one or all of them combined (`relief::composite`). Height maps are
//! saved in the project; older projects with only the file path are re-read on the desktop.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::{hint, unit_label, Fields, Host};
use crate::widgets::TipExt;
use eframe::egui;
use gcarve_core::document::{Document, Relief};
use gcarve_core::relief::rasterize_relief;

const CELLS: usize = 400_000;
/// `Relief::combine`: how a component goes onto the ones above it in the list (VCarve's combine modes)
const COMBINE: [&str; 4] = ["Add", "Subtract", "Keep the higher", "Keep the lower"]; // i18n

pub enum Next {
    Keep,
    Close,
    /// open a New 3D Model toolpath dialog
    Toolpath,
}

#[derive(Default)]
pub struct ReliefDialog {
    pub cur: usize,
    /// the relief the fields were filled from
    filled: i32,
    name: String,
    f: Fields,
    keep_aspect: bool,
}

impl ReliefDialog {
    fn fill(&mut self, doc: &Document, r: &Relief) {
        self.f = Fields::new(doc);
        self.filled = r.id;
        self.name = r.name.clone();
        self.f.set("x", r.x);
        self.f.set("y", r.y);
        self.f.set("w", r.w);
        self.f.set("h", r.h);
        self.f.set("depth", r.depth);
        self.f.set("lift", r.lift);
        self.keep_aspect = true;
    }

    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> Next {
        let mut open = true;
        let mut next = Next::Keep;
        let mut remove = false;
        let mut import = false;
        let mut apply = false;
        let mut mirror: Option<bool> = None;
        let mut picture = false;
        let mut pic_edit: Option<(bool, u32, bool)> = None;
        // component settings: (combine, slice low %, slice high %, smoothing)
        let mut comp_edit: Option<(u8, f64, f64, u32)> = None;
        let mut swap: Option<usize> = None;
        let mut outline = false;
        let doc = &host.canvas.doc;
        if !doc.reliefs.is_empty() {
            self.cur = self.cur.min(doc.reliefs.len() - 1);
            if doc.reliefs[self.cur].id != self.filled {
                let r = doc.reliefs[self.cur].clone();
                self.fill(doc, &r);
            }
        }
        let have = !doc.reliefs.is_empty();
        crate::widgets::Dialog::new(i18n::t("3D Models")).open(&mut open).close_footer().width(520.0).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        if !have {
                            hint(ui, i18n::t("No 3D models yet"));
                        }
                        for (i, r) in doc.reliefs.iter().enumerate() {
                            let label = if r.height.is_empty() { format!("{}  (file missing)", r.name) } else { r.name.clone() };
                            // after the first: how it goes onto the ones above it
                            let label = if i > 0 { format!("{}  {label}", ["+", "−", "▲", "▼"][r.combine.min(3) as usize]) } else { label };
                            if ui.selectable_label(i == self.cur, label).clicked() {
                                self.cur = i;
                            }
                        }
                    });
                    if doc.reliefs.len() > 1 {
                        ui.horizontal(|ui| {
                            if ui.add_enabled(self.cur > 0, egui::Button::new(i18n::t("Up"))).tip(i18n::t("Components combine from the top of the list down")).clicked() {
                                swap = Some(self.cur - 1);
                            }
                            if ui.add_enabled(self.cur + 1 < doc.reliefs.len(), egui::Button::new(i18n::t("Down"))).clicked() {
                                swap = Some(self.cur);
                            }
                        });
                    }
                });
                ui.add_enabled_ui(have, |ui| {
                    egui::Grid::new("relief").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label(i18n::t("Name"));
                        if ui.add(crate::widgets::field(&mut self.name).desired_width(200.0)).lost_focus() {
                            apply = true;
                        }
                        ui.end_row();
                        for (key, label) in [("x", "X"), ("y", "Y"), ("w", i18n::t("Width")), ("h", i18n::t("Height"))] {
                            ui.label(unit_label(label, doc));
                            if self.f.ui(ui, key, doc) {
                                apply = true;
                            }
                            ui.end_row();
                        }
                        ui.label("");
                        crate::widgets::checkbox(ui, &mut self.keep_aspect, i18n::t("Keep proportions"));
                        ui.end_row();
                        ui.label(unit_label(i18n::t("Depth"), doc)).tip(i18n::t("Total carved depth: the model's top sits at the board surface, its base this far down"));
                        if self.f.ui(ui, "depth", doc) {
                            apply = true;
                        }
                        ui.end_row();
                        ui.label("");
                        let mut m = doc.reliefs.get(self.cur).map(|r| r.mirror).unwrap_or(false);
                        if crate::widgets::checkbox(ui, &mut m, i18n::t("Mirror")).changed() {
                            mirror = Some(m);
                        }
                        ui.end_row();
                        if let Some(r) = doc.reliefs.get(self.cur) {
                            let (mut comb, mut lo, mut hi, mut sm) = (r.combine, r.clip_lo * 100.0, r.clip_hi * 100.0, r.smooth);
                            let mut changed = false;
                            let several = doc.reliefs.len() > 1;
                            if several {
                            ui.label(i18n::t("Combine")).tip(i18n::t("How this component goes onto the ones above it in the list when a 3D toolpath cuts all models combined"));
                            let names: Vec<(u8, String)> = COMBINE.iter().enumerate().map(|(i, n)| (i as u8, i18n::tr(n))).collect();
                            let refs: Vec<(u8, &str)> = names.iter().map(|(i, n)| (*i, n.as_str())).collect();
                            changed |= super::combo(ui, "relief-combine", &mut comb, &refs, 200.0);
                            ui.end_row();
                            ui.label(unit_label(i18n::t("Lift"), doc)).tip(i18n::t("Raises this component's base above the board's base, so it stands on top of the others"));
                            if self.f.ui(ui, "lift", doc) {
                                apply = true;
                            }
                            ui.end_row();
                            }
                            ui.label(i18n::t("Keep height (%)")).tip(i18n::t("Keep only this band of the model's height, stretched over its depth. Raise the bottom to cut away a base or background (a zero plane); lower the top to flatten it"));
                            ui.horizontal(|ui| {
                                changed |= ui.add(egui::DragValue::new(&mut lo).range(0.0..=99.0).speed(0.5).prefix(i18n::t("from "))).changed();
                                changed |= ui.add(egui::DragValue::new(&mut hi).range(1.0..=100.0).speed(0.5).prefix(i18n::t("to "))).changed();
                            });
                            ui.end_row();
                            if r.pic.is_empty() {
                                ui.label(i18n::t("Smoothing"));
                                changed |= ui.add(egui::DragValue::new(&mut sm).range(0..=50).suffix(i18n::t(" cells"))).on_hover_text(i18n::t("Softens facets and steps in the model's surface")).changed();
                                ui.end_row();
                            }
                            if changed {
                                comp_edit = Some((comb, lo / 100.0, hi.max(lo + 1.0) / 100.0, sm));
                            }
                        }
                        // a relief made from a picture: how its brightness becomes height
                        if let Some(r) = doc.reliefs.get(self.cur).filter(|r| !r.pic.is_empty()) {
                            let (mut inv, mut smooth, mut levels) = (r.pic_invert, r.pic_smooth, r.pic_levels);
                            ui.label(i18n::t("Picture"));
                            let mut changed = crate::widgets::checkbox(ui, &mut inv, i18n::t("Dark parts stand high")).on_hover_text(i18n::t("Off: light parts stay high (white = the surface). On: a lithophane, thick where dark and thin where light, to hold up to a light behind it")).changed();
                            ui.end_row();
                            ui.label(i18n::t("Smoothing"));
                            changed |= ui.add(egui::DragValue::new(&mut smooth).range(0..=20).suffix(i18n::t(" px"))).on_hover_text(i18n::t("Blurs grain and noise so the surface doesn't step")).changed();
                            ui.end_row();
                            ui.label("");
                            changed |= crate::widgets::checkbox(ui, &mut levels, i18n::t("Auto contrast")).on_hover_text(i18n::t("Stretches the picture's own light and dark over the whole depth")).changed();
                            ui.end_row();
                            if changed {
                                pic_edit = Some((inv, smooth, levels));
                            }
                        }
                    });
                });
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button(i18n::t("Import model…")).clicked() {
                    import = true;
                }
                if ui.button(i18n::t("From a picture…")).on_hover_text(i18n::t("A photo or logo becomes the height: light parts stay high, dark parts are carved deep (or the other way round for a lithophane)")).clicked() {
                    picture = true;
                }
                if ui.add_enabled(have, egui::Button::new(i18n::t("Remove"))).clicked() {
                    remove = true;
                }
                if ui.add_enabled(have, egui::Button::new(i18n::t("Outline as vectors"))).tip(i18n::t("Draws closed vectors round where the models stand above their base (all of them combined): to cut them out, or to keep a 3D toolpath inside")).clicked() {
                    outline = true;
                }
            });
            // one row high (a bare right-to-left layout takes all the height left)
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(have, crate::widgets::Button::new(i18n::t("Add 3D toolpath")).primary(true)).clicked() {
                    next = Next::Toolpath;
                }
            });
            hint(ui, i18n::t("The model's (or picture's) footprint is stretched to the width and height; its height range maps to the depth. Use a ball-nose bit for finishing and an end mill for roughing. A lithophane: tick Dark parts stand high, cut it from white acrylic or Corian and keep the thinnest part about 1 / 16 in thick (board thickness minus depth)."));
        });
        if import {
            host.files.pick_into(ctx, i18n::t("3D models (.stl .3mf .obj .step)"), crate::files::MODEL_EXTS, "stl");
        }
        if picture {
            host.files.pick_into(ctx, i18n::t("Pictures (.png .jpg .webp)"), &["png", "jpg", "jpeg", "webp"], "relief-picture");
        }
        if let Some((inv, smooth, levels)) = pic_edit.filter(|_| have) {
            let doc = &mut host.canvas.doc;
            doc.checkpoint();
            let r = &mut doc.reliefs[self.cur];
            (r.pic_invert, r.pic_smooth, r.pic_levels) = (inv, smooth, levels);
            gcarve_core::relief::rasterize_picture(r);
            host.canvas.touch();
        }
        if let Some(j) = swap.filter(|_| have) {
            let doc = &mut host.canvas.doc;
            doc.checkpoint();
            doc.reliefs.swap(j, j + 1);
            self.cur = if self.cur == j { j + 1 } else { j };
            host.canvas.touch();
        }
        if let Some((comb, lo, hi, sm)) = comp_edit.filter(|_| have) {
            let doc = &mut host.canvas.doc;
            doc.checkpoint();
            let r = &mut doc.reliefs[self.cur];
            (r.combine, r.clip_lo, r.clip_hi, r.smooth) = (comb, lo, hi, sm);
            host.canvas.touch();
        }
        if outline && have {
            let doc = &mut host.canvas.doc;
            let lines = gcarve_core::relief::composite(&doc.reliefs.iter().collect::<Vec<_>>()).map(|m| gcarve_core::relief::outline(&m)).unwrap_or_default();
            if lines.is_empty() {
                *host.message = i18n::t("Nothing stands above the base to outline").into();
            } else {
                doc.checkpoint();
                let layer = doc.layer_named("3D outline");
                let ids: std::collections::BTreeSet<i32> = lines.into_iter().map(|c| doc.add(c, Some(layer))).collect();
                *host.message = i18n::tf("Drew {n} outlines round the 3D models", &[("n", &ids.len().to_string())]);
                host.canvas.sel = ids;
                host.canvas.mutated();
            }
        }
        if (apply || mirror.is_some()) && have {
            let doc = &mut host.canvas.doc;
            let vals = ["x", "y", "w", "h", "depth", "lift"].map(|k| self.f.get(k, doc));
            let keep = self.keep_aspect;
            let name = self.name.clone();
            doc.checkpoint();
            let r = &mut doc.reliefs[self.cur];
            let aspect = if r.h > 0.0 { r.w / r.h } else { 1.0 };
            let (w_changed, h_changed) = ((vals[2] - r.w).abs() > 1e-9, (vals[3] - r.h).abs() > 1e-9);
            r.name = name;
            if vals[0].is_finite() {
                r.x = vals[0];
            }
            if vals[1].is_finite() {
                r.y = vals[1];
            }
            if vals[2].is_finite() && vals[2] > 0.0 {
                r.w = vals[2];
            }
            if vals[3].is_finite() && vals[3] > 0.0 {
                r.h = vals[3];
            }
            if keep && w_changed && !h_changed {
                r.h = r.w / aspect;
            } else if keep && h_changed && !w_changed {
                r.w = r.h * aspect;
            }
            if vals[4].is_finite() && vals[4] > 0.0 {
                r.depth = vals[4];
            }
            if vals[5].is_finite() {
                r.lift = vals[5];
            }
            if let Some(m) = mirror {
                r.mirror = m;
            }
            let r = r.clone();
            self.fill(doc, &r);
            host.canvas.touch();
        }
        if remove && have {
            let doc = &mut host.canvas.doc;
            doc.checkpoint();
            let id = doc.reliefs[self.cur].id;
            doc.reliefs.remove(self.cur);
            doc.ops.retain(|o| o.relief != id);
            self.cur = self.cur.saturating_sub(1);
            self.filled = -1;
            host.canvas.touch();
        }
        if matches!(next, Next::Toolpath) {
            return Next::Toolpath;
        }
        if open { Next::Keep } else { Next::Close }
    }
}

/// Qt Canvas::importRelief: fit into half the board, keep the model's proportions, centre it.
pub fn import_file(name: &str, bytes: &[u8], host: &mut Host) {
    let doc = &mut host.canvas.doc;
    let tris = match crate::files::model_tris(name, bytes) {
        Ok(t) => t,
        Err(e) => {
            host.cloud.report("stl", &crate::files::scrub(&e, name));
            *host.message = i18n::tf("Cannot load the 3D model: {e}", &[("e", &e)]);
            return;
        }
    };
    let (mut x0, mut y0, mut x1, mut y1) = (1e300_f64, 1e300_f64, -1e300_f64, -1e300_f64);
    for t in &tris {
        for q in [t.a, t.b, t.c] {
            x0 = x0.min(q.x);
            x1 = x1.max(q.x);
            y0 = y0.min(q.y);
            y1 = y1.max(q.y);
        }
    }
    let aspect = if x1 - x0 > 1e-9 && y1 - y0 > 1e-9 { (x1 - x0) / (y1 - y0) } else { 1.0 };
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
    let mut r = Relief { id: doc.next_relief_id, name: stem.clone(), source: name.to_string(), ..Default::default() };
    doc.next_relief_id += 1;
    let (max_w, max_h) = (doc.material.w * 0.5, doc.material.h * 0.5);
    r.w = max_w.min(max_h * aspect);
    r.h = r.w / aspect;
    r.x = (doc.material.w - r.w) / 2.0;
    r.y = (doc.material.h - r.h) / 2.0;
    r.depth = 0.25f64.min(doc.material.t * 0.5);
    rasterize_relief(&tris, &mut r, CELLS);
    doc.checkpoint();
    doc.reliefs.push(r);
    host.canvas.touch();
    *host.message = i18n::tf("Imported 3D model {stem} ({tris} triangles). Place it in the 3D Models dialog, then add a 3D Model toolpath.", &[("stem", &stem), ("tris", &(tris.len()))]);
}

/// A picture becomes a relief (light = high, or dark = high for a lithophane): sized like an imported model, half the
/// board, keeping the picture's shape, centred. The picture is kept in the project (≤ 600 px), so it comes back anywhere.
pub fn import_picture(name: &str, bytes: &[u8], host: &mut Host) {
    let (gray, w, h) = match super::trace::load_gray(name, bytes, 600) {
        Ok(g) => g,
        Err(e) => {
            host.cloud.report("relief picture", &crate::files::scrub(&e, name));
            *host.message = i18n::tf("Cannot read the picture: {e}", &[("e", &e)]);
            return;
        }
    };
    let doc = &mut host.canvas.doc;
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
    let mut r = Relief { id: doc.next_relief_id, name: stem.clone(), source: name.to_string(), pic: gcarve_core::photo::b64_encode(&gray), pic_w: w as u32, pic_h: h as u32, ..Default::default() };
    doc.next_relief_id += 1;
    let aspect = w as f64 / h as f64;
    let (max_w, max_h) = (doc.material.w * 0.5, doc.material.h * 0.5);
    r.w = max_w.min(max_h * aspect);
    r.h = r.w / aspect;
    r.x = (doc.material.w - r.w) / 2.0;
    r.y = (doc.material.h - r.h) / 2.0;
    r.depth = 0.2f64.min(doc.material.t * 0.5);
    gcarve_core::relief::rasterize_picture(&mut r);
    doc.checkpoint();
    doc.reliefs.push(r);
    host.canvas.touch();
    *host.message = i18n::tf("Made a relief from {stem}. Set its depth and size in 3D Models, then add a 3D Model toolpath (a ball-nose bit finishes it best).", &[("stem", &stem)]);
}

/// Desktop: reliefs saved as a source path get their height field back after a project load (a picture's is rebuilt
/// by the project loader on every device).
#[cfg(not(target_arch = "wasm32"))]
pub fn rehydrate(doc: &mut Document) {
    for r in &mut doc.reliefs {
        if r.height.is_empty() && r.pic.is_empty() && !r.source.is_empty() {
            if let Ok(tris) = std::fs::read(&r.source).map_err(|e| e.to_string()).and_then(|b| crate::files::model_tris(&r.source, &b)) {
                rasterize_relief(&tris, r, CELLS);
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub fn rehydrate(_doc: &mut Document) {}
