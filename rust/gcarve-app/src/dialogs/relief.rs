//! 3D Models (Qt ReliefDialog.qml): import STL, place / size / depth / mirror, remove, and start a 3D Model toolpath.
//! Modeless. Reliefs are saved as source path + placement; on the desktop they are re-rasterized on load.

use super::{hint, unit_label, Fields, Host};
use crate::widgets::TipExt;
use eframe::egui;
use gcarve_core::document::{Document, Relief};
use gcarve_core::relief::{load_stl, rasterize_relief};

const CELLS: usize = 400_000;

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
        self.keep_aspect = true;
    }

    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> Next {
        let mut open = true;
        let mut next = Next::Keep;
        let mut remove = false;
        let mut import = false;
        let mut apply = false;
        let mut mirror: Option<bool> = None;
        let doc = &host.canvas.doc;
        if !doc.reliefs.is_empty() {
            self.cur = self.cur.min(doc.reliefs.len() - 1);
            if doc.reliefs[self.cur].id != self.filled {
                let r = doc.reliefs[self.cur].clone();
                self.fill(doc, &r);
            }
        }
        let have = !doc.reliefs.is_empty();
        crate::widgets::Dialog::new("3D Models").open(&mut open).close_footer().width(520.0).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        if !have {
                            hint(ui, "No 3D models yet");
                        }
                        for (i, r) in doc.reliefs.iter().enumerate() {
                            let label = if r.height.is_empty() { format!("{}  (file missing)", r.name) } else { r.name.clone() };
                            if ui.selectable_label(i == self.cur, label).clicked() {
                                self.cur = i;
                            }
                        }
                    });
                });
                ui.add_enabled_ui(have, |ui| {
                    egui::Grid::new("relief").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("Name");
                        if ui.add(crate::widgets::field(&mut self.name).desired_width(200.0)).lost_focus() {
                            apply = true;
                        }
                        ui.end_row();
                        for (key, label) in [("x", "X"), ("y", "Y"), ("w", "Width"), ("h", "Height")] {
                            ui.label(unit_label(label, doc));
                            if self.f.ui(ui, key, doc) {
                                apply = true;
                            }
                            ui.end_row();
                        }
                        ui.label("");
                        crate::widgets::checkbox(ui, &mut self.keep_aspect, "Keep proportions");
                        ui.end_row();
                        ui.label(unit_label("Depth", doc)).tip("Total carved depth: the model's top sits at the board surface, its base this far down");
                        if self.f.ui(ui, "depth", doc) {
                            apply = true;
                        }
                        ui.end_row();
                        ui.label("");
                        let mut m = doc.reliefs.get(self.cur).map(|r| r.mirror).unwrap_or(false);
                        if crate::widgets::checkbox(ui, &mut m, "Mirror").changed() {
                            mirror = Some(m);
                        }
                        ui.end_row();
                    });
                });
            });
            ui.horizontal(|ui| {
                if ui.button("Import STL…").clicked() {
                    import = true;
                }
                if ui.add_enabled(have, egui::Button::new("Remove")).clicked() {
                    remove = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(have, crate::widgets::Button::new("Add 3D toolpath").primary(true)).clicked() {
                        next = Next::Toolpath;
                    }
                });
            });
            hint(ui, "The model's XY footprint is stretched to the width and height; its height range maps to the depth. Use a ball-nose bit for finishing and an end mill for roughing.");
        });
        if import {
            host.files.pick_into(ctx, "3D models (.stl .3mf)", &["stl", "3mf"], "stl");
        }
        if (apply || mirror.is_some()) && have {
            let doc = &mut host.canvas.doc;
            let vals = ["x", "y", "w", "h", "depth"].map(|k| self.f.get(k, doc));
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
    let tris = match load_stl(bytes) {
        Ok(t) => t,
        Err(e) => {
            host.cloud.report("stl", &crate::files::scrub(&e, name));
            *host.message = format!("Cannot load the 3D model: {e}");
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
    *host.message = format!("Imported 3D model {stem} ({} triangles). Place it in the 3D Models dialog, then add a 3D Model toolpath.", tris.len());
}

/// Desktop: reliefs saved as a source path get their height field back after a project load.
#[cfg(not(target_arch = "wasm32"))]
pub fn rehydrate(doc: &mut Document) {
    for r in &mut doc.reliefs {
        if r.height.is_empty() && !r.source.is_empty() {
            if let Ok(tris) = std::fs::read(&r.source).map_err(|e| e.to_string()).and_then(|b| load_stl(&b)) {
                rasterize_relief(&tris, r, CELLS);
            }
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub fn rehydrate(_doc: &mut Document) {}
