//! Shapes library (VCarve clipart): the built-in shapes of gcarve_core::clipart by category, plus "My shapes" =
//! selections the user saved (clipboard format: shapes, text and the toolpaths that only use them, with their bits).
//! Click a tile to add it centred on the board: built-ins at the width typed, saved shapes at their own size.
//! My shapes live in the local store (desktop config folder / browser storage).

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::{hint, unit_label, Host};
use crate::widgets::TipExt;
use eframe::egui;
use gcarve_core::clipart::{bounds_all, library, place, ClipShape, CATEGORIES};
use gcarve_core::document::Document;
use gcarve_core::geom::{transformed, Affine, Contour, Pt};
use gcarve_core::project::{load_clip, save_clip};
use serde::{Deserialize, Serialize};

const STORE_KEY: &str = "shapes.json";

#[derive(Serialize, Deserialize, Clone)]
struct Saved {
    name: String,
    /// clipboard JSON (project.rs save_clip)
    clip: String,
}

pub struct ShapesDialog {
    builtin: Vec<ClipShape>,
    saved: Vec<Saved>,
    /// outlines of each saved entry for its thumbnail
    saved_preview: Vec<Vec<Contour>>,
    tab: usize,
    width_text: String,
    new_name: String,
}

fn load_saved() -> Vec<Saved> {
    crate::store::load(STORE_KEY).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn preview_of(clip: &str) -> Vec<Contour> {
    let mut d = Document::default();
    load_clip(clip, &mut d, Pt::default());
    d.shapes.into_iter().map(|s| s.c).collect()
}

impl ShapesDialog {
    pub fn new(doc: &Document) -> Self {
        let saved = load_saved();
        let saved_preview = saved.iter().map(|s| preview_of(&s.clip)).collect();
        ShapesDialog { builtin: library(), saved, saved_preview, tab: 0, width_text: crate::units::text(doc, 6.0), new_name: String::new() }
    }

    fn store(&self) {
        crate::store::save(STORE_KEY, &serde_json::to_string(&self.saved).unwrap_or_default());
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut open = true;
        let (mut add_builtin, mut add_saved, mut delete) = (None, None, None);
        let mut save = false;
        let doc = &host.canvas.doc;
        let width = super::eval_len(&self.width_text, doc).ok().filter(|w| *w > 0.0);
        let sel = host.canvas.sel.len();
        crate::widgets::Dialog::new(i18n::t("Shapes library")).open(&mut open).close_footer().width(620.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (i, c) in CATEGORIES.iter().chain(std::iter::once(&"My shapes")).enumerate() {
                    if ui.add(egui::Button::selectable(self.tab == i, *c)).clicked() {
                        self.tab = i;
                    }
                }
            });
            crate::theme::hline(ui);
            if self.tab < CATEGORIES.len() {
                ui.horizontal(|ui| {
                    ui.label(unit_label(i18n::t("Width"), doc));
                    crate::units::num_field(ui, "shape-w", &mut self.width_text, doc, true, 110.0);
                    hint(ui, i18n::t("click a shape to add it, centred on the board (height keeps the proportions)"));
                });
            } else {
                ui.horizontal(|ui| {
                    ui.add(crate::widgets::field(&mut self.new_name).hint_text(i18n::t("Name for the selection")).desired_width(240.0));
                    save = ui
                        .add_enabled(sel > 0 && !self.new_name.trim().is_empty(), egui::Button::new(i18n::t("Save selection")))
                        .tip(i18n::t("Keeps the selected shapes (and text) at their size, with any toolpaths that only use them and their bits"))
                        .clicked();
                });
                if self.saved.is_empty() {
                    hint(ui, i18n::t("Nothing saved yet. Select a part you make often (a logo, a sign blank, a jig), name it and press Save selection: it comes back with its toolpaths."));
                }
            }
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(crate::widgets::list_height(ui, 150.0)).show(ui, |ui| {
                ui.set_width(620.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                    if self.tab < CATEGORIES.len() {
                        for (i, s) in self.builtin.iter().enumerate().filter(|(_, s)| s.category == CATEGORIES[self.tab]) {
                            if crate::widgets::shape_tile(ui, &s.contours, s.name, &format!("Add a {} {}", crate::units::text(doc, width.unwrap_or(6.0)), s.name.to_lowercase())).clicked() {
                                add_builtin = Some(i);
                            }
                        }
                    } else {
                        for (i, s) in self.saved.iter().enumerate() {
                            let r = crate::widgets::shape_tile(ui, &self.saved_preview[i], &s.name, i18n::t("Click to add · right-click to delete"));
                            if r.clicked() {
                                add_saved = Some(i);
                            }
                            r.context_menu(|ui| {
                                if ui.button(i18n::t("Delete")).clicked() {
                                    delete = Some(i);
                                    ui.close();
                                }
                            });
                        }
                    }
                });
            });
        });
        let centre = |doc: &Document, w: f64, h: f64| Pt { x: (doc.material.w - w) / 2.0, y: (doc.material.h - h) / 2.0 };
        if let (Some(i), Some(w)) = (add_builtin, width) {
            let doc = &mut host.canvas.doc;
            let s = &self.builtin[i];
            let b = bounds_all(&s.contours);
            let at = centre(doc, w, b.h() * w / b.w().max(1e-9));
            doc.checkpoint();
            let group = doc.new_group_id();
            let mut ids = std::collections::BTreeSet::new();
            for c in place(&s.contours, w, at) {
                let id = doc.add(c, None);
                if s.contours.len() > 1 {
                    if let Some(sh) = doc.find_mut(id) {
                        sh.group = group;
                    }
                }
                ids.insert(id);
            }
            *host.message = format!("Added {}", s.name);
            host.canvas.sel = ids;
            host.canvas.mutated();
        }
        if let Some(i) = add_saved {
            let doc = &mut host.canvas.doc;
            doc.checkpoint();
            let ids = load_clip(&self.saved[i].clip, doc, Pt::default());
            let mut b = gcarve_core::geom::Rect::default();
            for id in &ids {
                if let Some(s) = doc.find(*id) {
                    b.add_rect(&gcarve_core::geom::bounds(&s.c));
                }
            }
            let at = centre(doc, b.w(), b.h());
            let m = Affine::translate(at.x - b.x0, at.y - b.y0);
            for id in &ids {
                if let Some(s) = doc.find_mut(*id) {
                    s.c = transformed(&s.c, &m);
                }
            }
            *host.message = format!("Added {}", self.saved[i].name);
            host.canvas.sel = ids;
            host.canvas.mutated();
        }
        if save {
            let clip = save_clip(&host.canvas.doc, &host.canvas.sel);
            self.saved_preview.push(preview_of(&clip));
            self.saved.push(Saved { name: self.new_name.trim().to_string(), clip });
            *host.message = format!("Saved “{}” to My shapes", self.new_name.trim());
            self.new_name.clear();
            self.store();
        }
        if let Some(i) = delete {
            self.saved.remove(i);
            self.saved_preview.remove(i);
            self.store();
        }
        open
    }
}
