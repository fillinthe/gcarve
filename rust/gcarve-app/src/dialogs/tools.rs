//! Tool Library (Qt ToolsDialog.qml): the project's bits (= the user's saved library), per-material feed presets,
//! chip-load suggestions and the feeds meter. Edits save the library for new projects.

use super::{feed_meter, hint, material_combo, rate_label, unit_label, Fields, Host};
use crate::widgets::TipExt;
use crate::toollib;
use eframe::egui::{self, RichText};
use gcarve_core::document::{Document, FeedPreset, Tool, ToolType};
use gcarve_core::feeds::{FeedAssessment, FeedSuggestion};
use gcarve_core::pipeline;

pub struct ToolsDialog {
    pub cur: usize,
    name: String,
    kind: ToolType,
    flutes: String,
    rpm: String,
    slot: String,
    f: Fields,
    // preset being edited
    p_material: String,
    p_rpm: String,
    pub browser: crate::bitbrowser::BitBrowser,
    share_one: super::share::ShareBox,
    share_all: super::share::ShareBox,
    query: String,
    /// the form's text when the bit was loaded / last applied: a different text = the person edited something
    loaded_key: String,
    /// undo point taken for this bit's edits
    editing: bool,
    /// time of the last edit not yet written to the library file
    dirty_since: Option<f64>,
}

impl ToolsDialog {
    pub fn new(doc: &Document, cur: usize) -> Self {
        let mut d = ToolsDialog { cur: 0, name: String::new(), kind: ToolType::EndMill, flutes: "2".into(), rpm: String::new(), slot: String::new(), f: Fields::new(doc), p_material: doc.material.kind.clone(), p_rpm: String::new(), browser: Default::default(), share_one: Default::default(), share_all: Default::default(), query: String::new(), loaded_key: String::new(), editing: false, dirty_since: None };
        d.load(doc, cur);
        d
    }

    fn load(&mut self, doc: &Document, i: usize) {
        let Some(t) = doc.tools.get(i) else { return };
        self.cur = i;
        self.name = t.name.clone();
        self.kind = t.kind;
        self.flutes = t.flutes.to_string();
        self.rpm = t.rpm.to_string();
        self.slot = t.slot.to_string();
        self.f.set_fine("diameter", t.diameter);
        self.f.set_raw("angle", t.angle);
        self.f.set_fine("radius", t.radius);
        self.f.set_fine("cutLength", t.cut_length);
        self.f.set("stepdown", t.stepdown);
        self.f.set_raw("feedXY", t.feed_xy);
        self.f.set_raw("feedZ", t.feed_z);
        let hit = t.presets.iter().find(|p| p.material.eq_ignore_ascii_case(&self.p_material));
        self.f.set_raw("pFeedXY", hit.map(|p| p.feed_xy).unwrap_or(t.feed_xy));
        self.f.set_raw("pFeedZ", hit.map(|p| p.feed_z).unwrap_or(t.feed_z));
        self.f.set("pStepdown", hit.map(|p| p.stepdown).unwrap_or(t.stepdown));
        self.p_rpm = hit.map(|p| p.rpm).unwrap_or(t.rpm).to_string();
        self.loaded_key = self.edit_key();
        self.editing = false;
    }

    /// The form's numbers into the selected bit (one undo point per bit and editing session); false = nothing to do
    fn apply_edit(&mut self, doc: &mut Document) -> bool {
        if self.cur >= doc.tools.len() {
            return false;
        }
        if !std::mem::replace(&mut self.editing, true) {
            doc.checkpoint();
        }
        let t = self.tool(doc, &doc.tools[self.cur]);
        doc.tools[self.cur] = t;
        true
    }

    /// everything the bit form edits, as one string (not the material preset fields)
    fn edit_key(&self) -> String {
        let f: String = ["diameter", "angle", "radius", "cutLength", "stepdown", "feedXY", "feedZ"].iter().map(|k| format!("{}|", self.f.text(k))).collect();
        format!("{f}{}|{:?}|{}|{}|{}", self.name, self.kind, self.flutes, self.rpm, self.slot)
    }

    fn spec(&self, doc: &Document) -> gcarve_core::feeds::Cutter {
        gcarve_core::feeds::Cutter { vbit: self.kind == ToolType::VBit, diameter: self.f.get_or("diameter", doc, 0.25), flutes: self.flutes.trim().parse().unwrap_or(2) }
    }
    fn advice(&self, doc: &Document, material: &str, fxy: &str, fz: &str, rpm: &str, step: &str) -> (FeedSuggestion, FeedAssessment) {
        let c = self.spec(doc);
        let t = Tool { kind: self.kind, diameter: c.diameter, flutes: c.flutes, chip_loads: doc.tools.get(self.cur).map(|t| t.chip_loads.clone()).unwrap_or_default(), ..Default::default() };
        let mut d = doc.clone();
        d.material.kind = material.to_string(); // presets are per material name
        let s = pipeline::suggest(&d, &t);
        let a = pipeline::assess(&d, &t, self.f.raw_or(fxy, 0.0), self.f.raw_or(fz, 0.0), rpm.trim().parse().unwrap_or(0), self.f.get_or(step, doc, 0.0));
        (s, a)
    }

    fn tool(&self, doc: &Document, base: &Tool) -> Tool {
        Tool {
            name: self.name.clone(),
            kind: self.kind,
            diameter: self.f.get_or("diameter", doc, base.diameter).max(0.001),
            angle: self.f.raw_or("angle", base.angle),
            radius: self.f.get_or("radius", doc, base.radius).max(0.0),
            cut_length: self.f.get_or("cutLength", doc, base.cut_length).max(0.0),
            stepdown: self.f.get_or("stepdown", doc, base.stepdown).max(0.0),
            feed_xy: self.f.raw_or("feedXY", base.feed_xy).max(0.0),
            feed_z: self.f.raw_or("feedZ", base.feed_z).max(0.0),
            rpm: self.rpm.trim().parse().unwrap_or(base.rpm),
            flutes: self.flutes.trim().parse::<i32>().unwrap_or(2).max(1),
            slot: self.slot.trim().parse::<i32>().unwrap_or(base.slot).max(0),
            ..base.clone()
        }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut open = true;
        enum Act {
            Load(usize),
            Add,
            Delete,
            Duplicate,
            SavePreset,
            RemovePreset,
            Import,
            Export,
            AddBit(Tool),
        }
        let mut act: Option<Act> = None;
        let doc = &host.canvas.doc;
        if self.cur >= doc.tools.len() {
            self.cur = doc.tools.len().saturating_sub(1);
        }
        let board = doc.material.kind.clone();
        let slots = toollib::machine_slots(doc);
        let (base_s, base_a) = self.advice(doc, &board, "feedXY", "feedZ", &self.rpm, "stepdown");
        let (pre_s, pre_a) = self.advice(doc, &self.p_material.clone(), "pFeedXY", "pFeedZ", &self.p_rpm, "pStepdown");
        let unit = if doc.bare_unit_in > 0.0 { doc.bare_unit_in } else { 1.0 };
        let unit_name = crate::units::name(doc);
        let used = |i: usize| doc.ops.iter().filter(|o| o.tool as usize == i || (o.clear_tool >= 0 && o.clear_tool as usize == i)).count();
        let q = self.query.trim().to_lowercase();
        let shown: Vec<usize> = (0..doc.tools.len()).filter(|&i| q.is_empty() || format!("{} {}", doc.tools[i].name, kind_label(&doc.tools[i])).to_lowercase().contains(&q)).collect();
        crate::widgets::Dialog::new("Tool Library").width(900.0).open(&mut open).close_footer().show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                // ---- the library: search, rows with size / type / use, then the actions
                ui.vertical(|ui| {
                    ui.set_width(290.0);
                    ui.add(crate::widgets::field(&mut self.query).hint_text("Search your bits…").desired_width(290.0));
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().max_height(crate::widgets::list_height(ui, 250.0)).id_salt("tools-list").show(ui, |ui| {
                        ui.set_width(290.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for &i in &shown {
                            let t = &doc.tools[i];
                            let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::click());
                            let on = i == self.cur;
                            if on || resp.hovered() {
                                ui.painter().rect_filled(r, 5.0, if on { crate::theme::ACCENT } else { crate::theme::ROW_HOVER });
                            }
                            crate::bitbrowser::bit_icon(ui.painter(), egui::Rect::from_min_size(r.left_top() + egui::vec2(8.0, 6.0), egui::vec2(16.0, 32.0)), crate::bitbrowser::tool_family(t), Some(t.angle));
                            let sub_col = if on { egui::Color32::from_white_alpha(200) } else { crate::theme::TEXT_DIM };
                            let name = if slots > 0 { format!("{}  {}", slot_text(t.slot), t.name) } else { t.name.clone() };
                            let p = ui.painter_at(r);
                            p.text(r.left_top() + egui::vec2(34.0, 15.0), egui::Align2::LEFT_CENTER, name, crate::theme::font(13.0), crate::theme::TEXT);
                            let n = used(i);
                            let sub = format!("{} · Ø {} {} · {}F{}", kind_label(t), super::fmt_len(t.diameter / unit), unit_name, t.flutes, if n > 0 { format!(" · {n} toolpath{}", if n == 1 { "" } else { "s" }) } else { String::new() });
                            p.text(r.left_top() + egui::vec2(34.0, 31.0), egui::Align2::LEFT_CENTER, sub, crate::theme::font(11.0), sub_col);
                            if resp.clicked() {
                                act = Some(Act::Load(i));
                            }
                        }
                        if shown.is_empty() {
                            hint(ui, "No bit matches. Clear the search, or add one from the catalog.");
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(crate::widgets::Button::new("Browse catalog…").primary(true)).tip("Whiteside, Amana, SpeTool, Onsrud and more: pick by type, maker and size").clicked() {
                            self.browser.open = true;
                        }
                        if ui.button("New bit").tip("A blank 1/4 in end mill to fill in").clicked() {
                            act = Some(Act::Add);
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Duplicate").clicked() {
                            act = Some(Act::Duplicate);
                        }
                        let n = used(self.cur);
                        let del = if n > 0 { format!("Delete ({n} toolpath{} switch)", if n == 1 { "" } else { "s" }) } else { "Delete".into() };
                        if ui.add_enabled(doc.tools.len() > 1, egui::Button::new(del)).clicked() {
                            act = Some(Act::Delete);
                        }
                    });
                    ui.add_space(4.0);
                    egui::CollapsingHeader::new("Import, export, share").id_salt("tools-more").show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Import…").tip("GCarve bit library (.json), a Fusion 360 tools.json or a Vectric tool database (.vtdb)").clicked() {
                                act = Some(Act::Import);
                            }
                            if ui.button("Export…").clicked() {
                                act = Some(Act::Export);
                            }
                        });
                        if let Some(t) = doc.tools.get(self.cur) {
                            let one = std::slice::from_ref(t);
                            self.share_one.ui(ui, host.cloud, "this bit", "bits", &t.name, || crate::library::bits_json(one));
                        }
                        self.share_all.ui(ui, host.cloud, "all bits", "bits", &format!("{} bits", doc.tools.len()), || crate::library::bits_json(&doc.tools));
                    });
                    hint(ui, "Changes save as you type, to this project and to your library for new projects.");
                });
                ui.separator();
                // ---- the selected bit
                ui.vertical(|ui| {
                    ui.set_min_width(560.0);
                    let cur = doc.tools.get(self.cur);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(30.0, 48.0), egui::Sense::hover());
                        if let Some(t) = cur {
                            crate::bitbrowser::bit_icon(ui.painter(), r, crate::bitbrowser::tool_family(t), Some(self.f.raw_or("angle", t.angle)));
                        }
                        ui.vertical(|ui| {
                            ui.add(crate::widgets::field(&mut self.name).desired_width(440.0).font(crate::theme::font(15.0)));
                            let n = used(self.cur);
                            let text = if n == 0 { "Not used by any toolpath in this project".to_string() } else { format!("Used by {n} toolpath{} in this project", if n == 1 { "" } else { "s" }) };
                            ui.label(RichText::new(text).small().color(crate::theme::TEXT_DIM));
                        });
                    });
                    ui.add_space(8.0);
                    ui.horizontal_top(|ui| {
                        card(ui, "Shape", 270.0, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for (k, label) in [(ToolType::EndMill, "End mill"), (ToolType::VBit, "V-bit"), (ToolType::BallNose, "Ball nose"), (ToolType::RoundOver, "Round-over")] {
                                    ui.selectable_value(&mut self.kind, k, label);
                                }
                            });
                            ui.add_space(4.0);
                            egui::Grid::new("tool-shape").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                                ui.label(unit_label("Diameter", doc));
                                self.f.ui_sized(ui, "diameter", doc, 100.0, true);
                                ui.end_row();
                                match self.kind {
                                    ToolType::VBit => {
                                        ui.label("V angle (deg)");
                                        self.f.ui_sized(ui, "angle", doc, 100.0, false);
                                        ui.end_row();
                                    }
                                    ToolType::RoundOver => {
                                        ui.label(unit_label("Round radius", doc)).on_hover_text("The radius of the round it cuts. 0 = half the diameter (a true point); smaller leaves a flat tip.");
                                        self.f.ui_sized(ui, "radius", doc, 100.0, true);
                                        ui.end_row();
                                    }
                                    _ => {}
                                }
                                if self.kind != ToolType::VBit {
                                    ui.label(unit_label("Cutting length", doc)).on_hover_text("How long the flutes are. Export warns when a toolpath cuts deeper. 0 = not known.");
                                    self.f.ui_sized(ui, "cutLength", doc, 100.0, true);
                                    ui.end_row();
                                }
                                ui.label("Flutes");
                                ui.add(crate::widgets::field(&mut self.flutes).desired_width(60.0));
                                ui.end_row();
                                ui.label(if slots > 0 { format!("ATC slot (1–{slots})") } else { "Tool changer slot".into() });
                                ui.add(crate::widgets::field(&mut self.slot).desired_width(60.0)).on_hover_text("For machines with a tool changer: the pocket the bit is fetched from. 0 = manual change.");
                                ui.end_row();
                            });
                        });
                        card(ui, &format!("Cutting in {board}"), 270.0, |ui| {
                            egui::Grid::new("tool-cut").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                                ui.label(rate_label("Feed XY", doc));
                                self.f.ui_sized(ui, "feedXY", doc, 100.0, false);
                                ui.end_row();
                                ui.label(rate_label("Plunge Z", doc));
                                self.f.ui_sized(ui, "feedZ", doc, 100.0, false);
                                ui.end_row();
                                ui.label(unit_label("Pass depth", doc));
                                self.f.ui_sized(ui, "stepdown", doc, 100.0, true);
                                ui.end_row();
                                ui.label("Spindle RPM");
                                ui.add(crate::widgets::field(&mut self.rpm).desired_width(100.0));
                                ui.end_row();
                            });
                            ui.add_space(4.0);
                            if ui.button(format!("Suggest for {board}")).tip("Fill feed, plunge, pass depth and RPM from chip-load tables for this bit in the board material").clicked() {
                                self.f.set_raw("feedXY", base_s.feed_xy);
                                self.f.set_raw("feedZ", base_s.feed_z);
                                self.f.set("stepdown", base_s.stepdown);
                                self.rpm = base_s.rpm.to_string();
                            }
                        });
                    });
                    ui.add_space(4.0);
                    feed_meter(ui, &base_a, 550.0);
                    ui.add_space(8.0);
                    // ---- feeds that depend on the board material
                    card(ui, "Feeds by material", 550.0, |ui| {
                        hint(ui, &format!("Toolpaths use the preset that matches the board material ({board}); otherwise the numbers above."));
                        let presets = doc.tools.get(self.cur).map(|t| t.presets.clone()).unwrap_or_default();
                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(250.0);
                                egui::ScrollArea::vertical().max_height(150.0).id_salt("presets").show(ui, |ui| {
                                    ui.set_width(250.0);
                                    if presets.is_empty() {
                                        hint(ui, "No presets yet: pick a material, set the numbers, Save preset.");
                                    }
                                    for p in &presets {
                                        let on = p.material.eq_ignore_ascii_case(&self.p_material);
                                        if ui.selectable_label(on, format!("{}  ·  F {:.0} / {:.0}  ·  {} rpm", p.material, p.feed_xy, p.feed_z, p.rpm)).clicked() {
                                            self.p_material = p.material.clone();
                                            self.f.set_raw("pFeedXY", p.feed_xy);
                                            self.f.set_raw("pFeedZ", p.feed_z);
                                            self.f.set("pStepdown", p.stepdown);
                                            self.p_rpm = p.rpm.to_string();
                                        }
                                    }
                                });
                            });
                            ui.vertical(|ui| {
                                egui::Grid::new("preset").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                                    ui.label("Material");
                                    material_combo(ui, "p-mat", &mut self.p_material, 190.0);
                                    ui.end_row();
                                    ui.label(rate_label("Feed XY", doc));
                                    self.f.ui_sized(ui, "pFeedXY", doc, 100.0, false);
                                    ui.end_row();
                                    ui.label(rate_label("Plunge Z", doc));
                                    self.f.ui_sized(ui, "pFeedZ", doc, 100.0, false);
                                    ui.end_row();
                                    ui.label(unit_label("Pass depth", doc));
                                    self.f.ui_sized(ui, "pStepdown", doc, 100.0, true);
                                    ui.end_row();
                                    ui.label("Spindle RPM");
                                    ui.add(crate::widgets::field(&mut self.p_rpm).desired_width(100.0));
                                    ui.end_row();
                                });
                                ui.horizontal(|ui| {
                                    if ui.add(crate::widgets::Button::new("Save preset").primary(true)).clicked() {
                                        act = Some(Act::SavePreset);
                                    }
                                    if ui.button("Suggest").tip(format!("Fill from chip-load tables for this bit in {}", self.p_material)).clicked() {
                                        self.f.set_raw("pFeedXY", pre_s.feed_xy);
                                        self.f.set_raw("pFeedZ", pre_s.feed_z);
                                        self.f.set("pStepdown", pre_s.stepdown);
                                        self.p_rpm = pre_s.rpm.to_string();
                                    }
                                    if ui.button("Remove").clicked() {
                                        act = Some(Act::RemovePreset);
                                    }
                                });
                                feed_meter(ui, &pre_a, 270.0);
                            });
                        });
                    });
                });
            });
        });
        if let Some(t) = self.browser.ui(ctx) {
            act = Some(Act::AddBit(t));
        }
        let doc = &mut host.canvas.doc;
        let mut saved = false;
        match act {
            Some(Act::Load(i)) => self.load(doc, i),
            Some(Act::Add) => {
                doc.checkpoint();
                let mut t = Tool { name: "New Tool".into(), ..Default::default() };
                let s = pipeline::suggest(doc, &t);
                (t.feed_xy, t.feed_z, t.stepdown, t.rpm) = (s.feed_xy, s.feed_z, s.stepdown, s.rpm);
                doc.tools.push(t);
                let n = doc.tools.len() - 1;
                let slots = toollib::machine_slots(doc);
                toollib::assign_slots(&mut doc.tools, n, slots);
                self.load(doc, n);
                saved = true;
            }
            Some(Act::Delete) if doc.tools.len() > 1 => {
                doc.checkpoint();
                let i = self.cur;
                doc.tools.remove(i);
                for op in &mut doc.ops {
                    // the Qt app: toolpaths on the deleted bit fall back to the first; later ones shift down
                    if op.tool as usize == i {
                        op.tool = 0;
                    } else if op.tool as usize > i {
                        op.tool -= 1;
                    }
                    if op.clear_tool as usize == i && op.clear_tool >= 0 {
                        op.clear_tool = -1;
                    } else if op.clear_tool as usize > i && op.clear_tool >= 0 {
                        op.clear_tool -= 1;
                    }
                }
                self.load(doc, i.saturating_sub(1));
                saved = true;
            }
            Some(Act::Duplicate) => {
                doc.checkpoint();
                let mut t = doc.tools[self.cur].clone();
                t.name = format!("{} copy", t.name);
                t.slot = 0;
                doc.tools.push(t);
                let n = doc.tools.len() - 1;
                let slots = toollib::machine_slots(doc);
                toollib::assign_slots(&mut doc.tools, n, slots);
                self.load(doc, n);
                saved = true;
            }
            Some(Act::SavePreset) => {
                let p = FeedPreset { material: self.p_material.trim().to_string(), feed_xy: self.f.raw_or("pFeedXY", 100.0), feed_z: self.f.raw_or("pFeedZ", 50.0), stepdown: self.f.get_or("pStepdown", doc, 0.25), rpm: self.p_rpm.trim().parse().unwrap_or(18000) };
                if !p.material.is_empty() {
                    doc.checkpoint();
                    let t = &mut doc.tools[self.cur];
                    match t.presets.iter_mut().find(|x| x.material.eq_ignore_ascii_case(&p.material)) {
                        Some(x) => *x = p,
                        None => t.presets.push(p),
                    }
                    saved = true;
                }
            }
            Some(Act::RemovePreset) => {
                doc.checkpoint();
                let m = self.p_material.clone();
                doc.tools[self.cur].presets.retain(|p| !p.material.eq_ignore_ascii_case(&m));
                saved = true;
            }
            Some(Act::AddBit(t)) => {
                doc.checkpoint();
                doc.tools.push(t);
                let n = doc.tools.len() - 1;
                let slots = toollib::machine_slots(doc);
                toollib::assign_slots(&mut doc.tools, n, slots);
                self.load(doc, n);
                saved = true;
            }
            Some(Act::Import) => host.files.pick_into(ctx, "Bit library (tools.json, .vtdb)", &["json", "vtdb"], "tools"),
            Some(Act::Export) => crate::files::save("tools.json", gcarve_core::project::tool_library_to_json(&doc.tools).into_bytes(), host.files.msg_tx.clone()),
            _ => {}
        }
        // a field the person changed goes into the project at once; the library file is written when typing pauses
        let now = ctx.input(|i| i.time);
        let key = self.edit_key();
        if !saved && key != self.loaded_key {
            if self.apply_edit(&mut host.canvas.doc) {
                host.canvas.touch();
                self.dirty_since = Some(now);
            }
            self.loaded_key = key;
        }
        if saved {
            self.dirty_since = None;
            toollib::save(&host.canvas.doc.tools);
            host.canvas.touch();
        } else if let Some(t0) = self.dirty_since {
            if now - t0 > 0.7 || !open {
                self.dirty_since = None;
                toollib::save(&host.canvas.doc.tools);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(750));
            }
        }
        open
    }
}

/// A Vectric tool database was read (desktop at once, the browser a moment later)
pub fn vtdb_done(r: Result<(Vec<Tool>, usize), String>, host: &mut Host) {
    match r {
        Ok((tools, presets)) => {
            let (added, updated) = merge_into(host, tools);
            *host.message = format!("Imported Vectric tools: {added} added, {updated} updated, {presets} material presets");
        }
        Err(e) => *host.message = e,
    }
}

/// A picked tools.json (GCarve or Fusion): merged into the document's tools and the saved library.
pub fn import_file(ctx: &egui::Context, name: &str, bytes: &[u8], host: &mut Host) {
    if name.to_ascii_lowercase().ends_with(".vtdb") {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            vtdb_done(crate::vtdb::import(bytes), host);
        }
        #[cfg(target_arch = "wasm32")]
        {
            *host.message = format!("Reading {name}…");
            crate::vtdb::start(bytes.to_vec(), ctx.clone());
        }
        return;
    }
    match toollib::import(&String::from_utf8_lossy(bytes)) {
        Ok(tools) => {
            let (added, updated) = merge_into(host, tools);
            *host.message = format!("{name}: {added} tools added, {updated} updated");
        }
        Err(e) => {
            host.cloud.report("bit library", &crate::files::scrub(&e, name));
            *host.message = format!("{name}: {e}");
        }
    }
}

/// Merged by name into the document's tools and the saved library (new bits get free ATC slots).
fn merge_into(host: &mut Host, tools: Vec<Tool>) -> (usize, usize) {
    let doc = &mut host.canvas.doc;
    doc.checkpoint();
    let n = doc.tools.len();
    let r = toollib::merge(&mut doc.tools, tools);
    let slots = toollib::machine_slots(doc);
    toollib::assign_slots(&mut doc.tools, n, slots);
    toollib::save(&doc.tools);
    host.canvas.touch();
    r
}

/// "T3" / "T–" (no slot) for tool lists on tool-changer machines
pub fn slot_text(slot: i32) -> String {
    if slot > 0 { format!("T{slot}") } else { "T–".into() }
}

/// "End mill", "V-bit 90°", "Ball nose", "Round-over"
fn kind_label(t: &Tool) -> String {
    match t.kind {
        ToolType::EndMill => "End mill".into(),
        ToolType::VBit => format!("V-bit {}°", super::fmt_len(t.angle)),
        ToolType::BallNose => "Ball nose".into(),
        ToolType::RoundOver => "Round-over".into(),
    }
}

/// A titled panel of the dialog
fn card(ui: &mut egui::Ui, title: &str, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(crate::theme::BG).corner_radius(6.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(width - 20.0);
            ui.label(RichText::new(title).family(crate::widgets::bold_family()));
            ui.add_space(4.0);
            add(ui);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edited_field_reaches_the_bit_with_one_undo_point() {
        let mut doc = Document::default();
        doc.tools = vec![Tool { name: "A".into(), diameter: 0.25, ..Default::default() }];
        let mut d = ToolsDialog::new(&doc, 0);
        assert_eq!(d.edit_key(), d.loaded_key, "a freshly loaded form is not an edit");
        d.f.set_raw("feedXY", 55.0);
        d.name = "Renamed".into();
        assert_ne!(d.edit_key(), d.loaded_key);
        assert!(d.apply_edit(&mut doc));
        assert_eq!((doc.tools[0].feed_xy, doc.tools[0].name.as_str(), doc.tools[0].diameter), (55.0, "Renamed", 0.25));
        // more typing in the same bit stays one undo step: a single undo gives the original back
        d.f.set_raw("feedZ", 11.0);
        assert!(d.apply_edit(&mut doc));
        assert_eq!(doc.tools[0].feed_z, 11.0);
        assert!(doc.undo());
        assert_eq!((doc.tools[0].feed_xy, doc.tools[0].feed_z, doc.tools[0].name.as_str()), (100.0, 50.0, "A"));
    }
}
