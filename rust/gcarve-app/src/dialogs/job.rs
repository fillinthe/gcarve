//! Board / Job Setup (Qt JobDialog.qml + OriginPicker.qml): material, size, thickness, safe/retract Z, XY datum,
//! Z zero, auto feeds, post. Sheets: add / remove / switch (the shell's SheetTabs call the same `sheet_*` helpers).

use super::{hint, material_combo, unit_label, Fields, Host};
use crate::widgets::TipExt;
use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use crate::catalog::limits_line;
use gcarve_core::document::{Document, MachineLimits, Material};

pub const ORIGIN_NAMES: [&str; 5] = ["Bottom-left", "Bottom-right", "Top-left", "Top-right", "Centre"];

pub enum Next {
    Keep,
    Close,
    /// open the post chooser (stays open)
    Posts,
}

pub struct JobDialog {
    kind: String,
    f: Fields,
    origin: i32,
    z_bottom: bool,
    auto_feeds: bool,
    machine: Option<MachineLimits>,
    post: Option<String>, // set when a machine is picked
    pub browser: crate::machinebrowser::MachineBrowser,
    /// opened for the machine only (Machine tile / menu): a pick applies at once, closing the browser closes all
    pub machine_only: bool,
    /// the user's own machines (library.rs), the one being edited (index, None = new), its share box
    mine: Vec<crate::library::CustomMachine>,
    editing: Option<(Option<usize>, crate::library::CustomMachine)>,
    share: (usize, super::share::ShareBox),
}

impl JobDialog {
    /// My machines: use / edit / share / delete, save the current one, or set one up from scratch
    fn my_machines(&mut self, ui: &mut egui::Ui, cloud: &mut crate::cloud::Cloud, post_now: &str) {
        use crate::library::{save_machines, CustomMachine};
        ui.add_space(4.0);
        ui.label(RichText::new("My machines").family(crate::widgets::bold_family()));
        let mut changed = false;
        let mut remove = None;
        for (i, m) in self.mine.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(&m.limits.name).on_hover_text(limits_line(&m.limits));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::IconButton::new("delete").size(14.0).padding(3.0).tip("Remove from My machines").show(ui).clicked() {
                        remove = Some(i);
                    }
                    if ui.small_button("Edit").clicked() {
                        self.editing = Some((Some(i), m.clone()));
                    }
                    if ui.small_button("Use").clicked() {
                        self.machine = Some(m.limits.clone());
                        if !m.post.is_empty() {
                            self.post = Some(m.post.clone());
                        }
                    }
                });
            });
            if self.share.0 == i {
                let data = || serde_json::to_value(vec![m.clone()]).unwrap_or_default();
                self.share.1.ui(ui, cloud, "this machine", "machines", &m.limits.name, data);
            } else if ui.small_button("Share…").clicked() {
                self.share = (i, super::share::ShareBox::opened());
            }
        }
        if let Some(i) = remove {
            self.mine.remove(i);
            changed = true;
        }
        if self.editing.is_none() {
            ui.horizontal(|ui| {
                if let Some(m) = self.machine.clone().filter(|_| ui.button("Save current as mine").on_hover_text("Keep this machine (and the chosen post) in My machines, on every device").clicked()) {
                    self.editing = Some((None, CustomMachine { limits: MachineLimits { id: String::new(), ..m }, post: post_now.into() }));
                }
                if ui.button("New custom machine…").clicked() {
                    let limits = MachineLimits { name: "My machine".into(), rpm_min: 6000.0, rpm_max: 18000.0, feed_xy: 400.0, feed_z: 100.0, travel_x: 48.0, travel_y: 96.0, travel_z: 6.0, ..Default::default() };
                    self.editing = Some((None, CustomMachine { limits, post: post_now.into() }));
                }
            });
        }
        let mut done = None;
        if let Some((i, m)) = &mut self.editing {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                let l = &mut m.limits;
                egui::Grid::new("custom-machine").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(crate::widgets::field(&mut l.name).desired_width(220.0));
                    ui.end_row();
                    ui.label("Spindle");
                    ui.add(crate::widgets::field(&mut l.spindle).hint_text("e.g. 3.0 kW HSD").desired_width(220.0));
                    ui.end_row();
                    let num = |ui: &mut egui::Ui, v: &mut f64, suffix: &str, max: f64| ui.add(egui::DragValue::new(v).range(0.0..=max).suffix(suffix));
                    ui.label("RPM");
                    ui.horizontal(|ui| {
                        num(ui, &mut l.rpm_min, "", 100000.0);
                        ui.label("to");
                        num(ui, &mut l.rpm_max, "", 100000.0);
                    });
                    ui.end_row();
                    ui.label("Max feed");
                    ui.horizontal(|ui| {
                        num(ui, &mut l.feed_xy, " in/min XY", 10000.0);
                        num(ui, &mut l.feed_z, " in/min Z", 10000.0);
                    });
                    ui.end_row();
                    ui.label("Travel");
                    ui.horizontal(|ui| {
                        num(ui, &mut l.travel_x, " X", 1000.0);
                        num(ui, &mut l.travel_y, " Y", 1000.0);
                        num(ui, &mut l.travel_z, " Z in", 100.0);
                    });
                    ui.end_row();
                    ui.label("Tool changer");
                    ui.add(egui::DragValue::new(&mut l.tool_slots).range(0..=99).custom_formatter(|n, _| if n < 1.0 { "manual changes".into() } else { format!("{n:.0} slots") }));
                    ui.end_row();
                    ui.label("Class");
                    let classes = [("", "Not sure"), ("industrial", "Industrial"), ("prosumer", "Prosumer"), ("hobby", "Hobby"), ("desktop", "Desktop")];
                    egui::ComboBox::from_id_salt("machine-class").selected_text(classes.iter().find(|c| c.0 == l.class).map_or("Not sure", |c| c.1)).show_ui(ui, |ui| {
                        for (k, t) in classes {
                            ui.selectable_value(&mut l.class, k.to_string(), t).on_hover_text("Scales suggested chip loads: hobby machines get lighter cuts");
                        }
                    });
                    ui.end_row();
                });
                hint(ui, &format!("Post: {} (the one chosen below when saved)", if m.post.is_empty() { "unchanged" } else { m.post.as_str() }));
                ui.horizontal(|ui| {
                    let ok = !m.limits.name.trim().is_empty();
                    if ui.add_enabled(ok, egui::Button::new("Save to My machines").fill(crate::theme::ACCENT)).clicked() {
                        done = Some(Some((*i, m.clone())));
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(None);
                    }
                });
            });
        }
        if let Some(r) = done {
            if let Some((i, mut m)) = r {
                m.limits.name = m.limits.name.trim().to_string();
                if m.limits.id.is_empty() {
                    m.limits.id = format!("custom:{}", m.limits.name.to_lowercase());
                }
                match i {
                    Some(i) => self.mine[i] = m.clone(),
                    None => {
                        crate::library::upsert_machines(&mut self.mine, vec![m.clone()]);
                    }
                }
                self.machine = Some(m.limits); // and use it for this project
                if !m.post.is_empty() {
                    self.post = Some(m.post);
                }
                changed = true;
            }
            self.editing = None;
        }
        if changed {
            save_machines(&self.mine);
        }
    }

    pub fn new(doc: &Document) -> Self {
        let m = &doc.material;
        let mut f = Fields::new(doc);
        f.set_bound("w", &m.bind, m.w);
        f.set_bound("h", &m.bind, m.h);
        f.set_bound("t", &m.bind, m.t);
        f.set_bound("safeZ", &m.bind, m.safe_z);
        f.set_bound("retractZ", &m.bind, m.retract_z);
        JobDialog { kind: m.kind.clone(), f, origin: m.origin, z_bottom: m.z_bottom, auto_feeds: m.auto_feeds, machine: m.machine.clone(), post: None, browser: Default::default(), machine_only: false, mine: crate::library::machines(), editing: None, share: Default::default() }
    }

    fn apply(&self, m: &mut Material, doc: &Document) {
        m.kind = self.kind.trim().to_string();
        m.w = self.f.get_or("w", doc, m.w).max(0.01);
        m.h = self.f.get_or("h", doc, m.h).max(0.01);
        m.t = self.f.get_or("t", doc, m.t).max(0.01);
        m.safe_z = self.f.get_or("safeZ", doc, m.safe_z);
        m.retract_z = self.f.get_or("retractZ", doc, m.retract_z);
        m.bind = self.f.bindings(&["w", "h", "t", "safeZ", "retractZ"]);
        m.origin = self.origin.clamp(0, 4);
        m.z_bottom = self.z_bottom;
        m.auto_feeds = self.auto_feeds;
        m.machine = self.machine.clone();
        if let Some(p) = &self.post {
            m.post = p.clone();
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> Next {
        if self.machine_only {
            // only the machine browser shows; a pick goes straight into the board settings
            return match self.browser.ui(ctx) {
                Some((l, post)) => {
                    self.machine = Some(l);
                    if post.is_some() {
                        self.post = post;
                    }
                    self.commit(host);
                    *host.message = format!("Machine: {}", self.machine.as_ref().map(|m| m.name.as_str()).unwrap_or_default());
                    // an empty drawing whose board can't fit on this machine: start with the machine's table size
                    let doc = &mut host.canvas.doc;
                    if let Some((tx, ty)) = doc.material.machine.as_ref().map(|m| (m.travel_x, m.travel_y)).filter(|t| t.0 > 0.0 && t.1 > 0.0) {
                        let b = &doc.material;
                        let fits = |w: f64, h: f64| w <= tx + 1e-6 && h <= ty + 1e-6;
                        if doc.shapes.is_empty() && !fits(b.w, b.h) && !fits(b.h, b.w) && b.bind.is_empty() {
                            (doc.material.w, doc.material.h) = (tx, ty);
                            doc.sync_sheet();
                            host.canvas.view = None; // fit the new board
                            host.canvas.touch();
                            *host.message += &format!(": board set to its {} × {} in table (change it under Board)", crate::dialogs::fmt_len(tx), crate::dialogs::fmt_len(ty));
                        }
                    }
                    Next::Close
                }
                None if !self.browser.open => Next::Close,
                None => Next::Keep,
            };
        }
        let mut next = Next::Keep;
        let mut ok = false;
        let doc = &host.canvas.doc;
        let post_id = self.post.clone().unwrap_or_else(|| doc.material.post.clone());
        let post_name = host.posts.all().into_iter().find(|(p, _)| p.id == post_id).map(|(p, _)| p.name).unwrap_or(post_id);
        let sheets = doc.sheets.len();
        let active = doc.active_sheet;
        let mut sheet_act: Option<i32> = None; // -1 add, -2 remove, >= 0 switch
        crate::widgets::Dialog::new("Board / Job Setup").show(ctx, |ui| {
            // board and heights on the left, where zero is on the right, machine and output underneath
            ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
            ui.label(RichText::new("Board").family(crate::widgets::bold_family()));
            egui::Grid::new("job-board").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                ui.label("Sheet");
                ui.horizontal(|ui| {
                    for i in 0..sheets {
                        if ui.selectable_label(i as i32 == active, doc.sheet_name(i)).clicked() {
                            sheet_act = Some(i as i32);
                        }
                    }
                    if ui.small_button("+").tip("Add a sheet (another board with its own settings)").clicked() {
                        sheet_act = Some(-1);
                    }
                    if ui.add_enabled(sheets > 1, egui::Button::new("−").small()).tip("Remove this sheet (its shapes move to the first sheet)").clicked() {
                        sheet_act = Some(-2);
                    }
                });
                ui.end_row();
                ui.label("Material");
                material_combo(ui, "kind", &mut self.kind, 220.0);
                ui.end_row();
                ui.label(unit_label("Board width X", doc));
                ui.horizontal(|ui| {
                    self.f.ui(ui, "w", doc);
                    if ui.add(crate::widgets::Button::new("Swap W/H").icon("swap", false)).clicked() {
                        let w = self.f.text("w").to_string();
                        let h = self.f.text("h").to_string();
                        self.f.map.insert("w".into(), h);
                        self.f.map.insert("h".into(), w);
                    }
                });
                ui.end_row();
                ui.label(unit_label("Board height Y", doc));
                self.f.ui(ui, "h", doc);
                ui.end_row();
                ui.label(unit_label("Thickness", doc));
                self.f.ui(ui, "t", doc);
                ui.end_row();
                ui.label(unit_label("Lift for moves", doc)).tip("Safe Z: how high above the board the bit travels between parts. Clear your clamps.");
                self.f.ui(ui, "safeZ", doc);
                ui.end_row();
                ui.label(unit_label("Small lift", doc)).tip("Retract Z: the short hop between nearby cuts of one toolpath, above the board.");
                self.f.ui(ui, "retractZ", doc);
                ui.end_row();
            });
            });
            ui.add_space(18.0);
            ui.vertical(|ui| {
            ui.label(RichText::new("Where you zero the machine").family(crate::widgets::bold_family()));
            egui::Grid::new("job-zero").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                ui.label("X and Y").on_hover_text("XY zero (datum): the point of the board you set as X0 Y0 on the machine");
                let (w, h) = (self.f.get_or("w", doc, 1.5), self.f.get_or("h", doc, 1.0));
                let aspect = if w > 0.0 && h > 0.0 { w / h } else { 1.5 };
                let row_h = (96.0 / aspect.clamp(0.5, 3.0)).round() as f32 + 8.0; // the picker's height: label centred on it
                ui.allocate_ui_with_layout(egui::vec2(0.0, row_h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    origin_picker(ui, &mut self.origin, aspect);
                    ui.label(RichText::new(format!("{}{}", ORIGIN_NAMES[self.origin as usize], if self.origin == 4 { " of the board" } else { " corner" })).color(crate::theme::TEXT_DIM));
                });
                ui.end_row();
                ui.label("Z (bit height)").on_hover_text("Z zero: where you touch the bit off");
                ui.vertical(|ui| {
                    crate::widgets::radio_value(ui, &mut self.z_bottom, false, "Touch off on top of the board");
                    crate::widgets::radio_value(ui, &mut self.z_bottom, true, "Touch off on the spoilboard / table");
                });
                ui.end_row();
                ui.label("");
                ui.vertical(|ui| {
                    ui.set_width(380.0);
                    // the whole setup as one paragraph, the way you'd tell someone at the machine
                    let u = |v: f64| crate::units::text(doc, v);
                    let (w, h, t) = (self.f.get_or("w", doc, doc.material.w), self.f.get_or("h", doc, doc.material.h), self.f.get_or("t", doc, doc.material.t));
                    let corner = match self.origin { 4 => "the centre of the board".to_string(), o => format!("its {} corner", ORIGIN_NAMES[o.clamp(0, 3) as usize].to_lowercase()) };
                    let z = if self.z_bottom { "on the spoilboard under it" } else { "on its top" };
                    let lift = self.f.get_or("safeZ", doc, doc.material.safe_z);
                    hint(ui, &format!("At the machine: lay down a {} × {} board, {} thick, of {}. Zero X and Y at {corner} and touch the bit off {z}. Between parts the bit lifts {} above the board. The yellow marker on the drawing shows your X/Y zero.", u(w), u(h), u(t), self.kind.to_lowercase(), u(lift)));
                });
                ui.end_row();
            });
            });
            });
            ui.separator();
            ui.label(RichText::new("Machine and output").family(crate::widgets::bold_family()));
            egui::Grid::new("job-machine").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                ui.label("Machine");
                ui.vertical(|ui| {
                    ui.set_width(380.0);
                    match &self.machine {
                        Some(m) => {
                            ui.label(RichText::new(&m.name).family(crate::widgets::bold_family()));
                            ui.label(RichText::new(format!("{}{}", if m.spindle.is_empty() { String::new() } else { format!("{} · ", m.spindle) }, limits_line(m))).color(crate::theme::TEXT_DIM).small());
                        }
                        None => hint(ui, "No machine chosen: generic limits (18,000 RPM, 600 in/min). Pick yours so suggested feeds stay inside its spindle and feed limits and export warns about anything beyond them."),
                    }
                    ui.horizontal(|ui| {
                        if ui.button(if self.machine.is_some() { "Change machine…" } else { "Choose machine…" }).clicked() {
                            self.browser.show(self.machine.as_ref());
                        }
                        if self.machine.is_some() && ui.button("No machine").on_hover_text("Generic limits (18,000 RPM, 600 in/min)").clicked() {
                            self.machine = None;
                        }
                    });
                    let post_now = self.post.clone().unwrap_or_else(|| host.canvas.doc.material.post.clone());
                    self.my_machines(ui, host.cloud, &post_now);
                });
                ui.end_row();
                ui.label("Feeds & speeds");
                crate::widgets::checkbox(ui, &mut self.auto_feeds, "Auto-set feeds for bits with no preset for this material").tip("Uses chip-load tables (bit diameter, flutes, material type). Bits with a saved preset for the material keep it.");
                ui.end_row();
                ui.label("G-code style").on_hover_text("Post-processor: the dialect your controller reads (WinCNC, GRBL, Mach3…)");
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(&post_name).truncate());
                    if ui.button("Choose…").clicked() {
                        next = Next::Posts;
                    }
                });
                ui.end_row();
                ui.label("");
                ui.vertical(|ui| {
                    ui.set_width(380.0);
                    hint(ui, "40+ built-in machines and controllers; upload Vectric .pp or JSON posts in the chooser.");
                    hint(ui, "Any length field accepts math and units: 4ft, 48in, 1200mm, 2*gap + 1in.");
                });
                ui.end_row();
            });
            let (o, c) = crate::widgets::ok_cancel(ui);
            ok |= o;
            if c {
                next = Next::Close;
            }
        });
        if let Some(a) = sheet_act {
            // apply the edits to the current sheet first, then move
            let doc = &mut host.canvas.doc;
            doc.checkpoint();
            let mut m = doc.material.clone();
            self.apply(&mut m, doc);
            doc.material = m;
            doc.sync_sheet();
            match a {
                -1 => sheet_add(doc),
                -2 => sheet_remove(doc, doc.active_sheet),
                i => doc.set_active_sheet(i),
            }
            host.canvas.apply_bindings(); // board-w / thickness and bound board fields follow
            host.canvas.mutated();
            *self = JobDialog::new(&host.canvas.doc);
            return Next::Keep;
        }
        if let Some((l, post)) = self.browser.ui(ctx) {
            self.machine = Some(l);
            if post.is_some() {
                self.post = post;
            }
        }
        if ok {
            self.commit(host);
            host.canvas.view = None; // refit the board
            return Next::Close;
        }
        next
    }

    /// The settings into the document (one undo step)
    fn commit(&self, host: &mut Host) {
        let doc = &mut host.canvas.doc;
        doc.checkpoint();
        let mut m = doc.material.clone();
        self.apply(&mut m, doc);
        doc.material = m;
        doc.sync_sheet();
        host.canvas.apply_bindings(); // board-w / thickness and bound board fields follow
        host.canvas.mutated();
        // new drawings start with this machine too (prefs::NewDoc::apply)
        let m = &host.canvas.doc.material;
        super::prefs::set_last_machine(m.machine.as_ref(), &m.post);
    }
}

/// XY zero picker: a little board with the five datum positions (0 BL, 1 BR, 2 TL, 3 TR, 4 centre).
pub fn origin_picker(ui: &mut egui::Ui, value: &mut i32, aspect: f64) -> bool {
    let w = 96.0;
    let h = (96.0 / aspect.clamp(0.5, 3.0)).round() as f32;
    let (outer, _) = ui.allocate_exact_size(egui::vec2(w, h + 8.0), Sense::hover());
    let rect = egui::Rect::from_min_size(outer.min + egui::vec2(0.0, 4.0), egui::vec2(w, h));
    let p = ui.painter();
    p.rect(rect, 3.0, Color32::from_rgba_unmultiplied(138, 122, 90, 46), Stroke::new(1.0, Color32::from_rgb(0x8a, 0x7a, 0x5a)), egui::StrokeKind::Inside);
    let mut changed = false;
    for (i, (fx, fy)) in [(0.0, 1.0), (1.0, 1.0), (0.0, 0.0), (1.0, 0.0), (0.5, 0.5)].into_iter().enumerate() {
        let c = egui::pos2(rect.left() + 8.0 + fx * (w - 16.0), rect.top() + 8.0 + fy * (h - 16.0));
        let r = ui.interact(egui::Rect::from_center_size(c, egui::vec2(16.0, 16.0)), ui.id().with(("origin", i)), Sense::click()).tip(format!("XY zero at {}", ORIGIN_NAMES[i].to_lowercase()));
        let on = *value == i as i32;
        // OriginPicker.qml: the datum is Theme.highlightText (the canvas marker's yellow)
        let hl = crate::theme::c(0xffd34d);
        let fill = if on { hl } else if r.hovered() { crate::theme::RAISED_HOVER } else { crate::theme::FIELD };
        p.circle(c, 8.0, fill, Stroke::new(1.0, if on { hl } else { crate::theme::BORDER_STRONG }));
        p.circle_filled(c, 3.0, if on { Color32::from_rgb(0x1b, 0x1d, 0x21) } else { crate::theme::TEXT_DIM });
        if r.clicked() {
            *value = i as i32;
            changed = true;
        }
    }
    changed
}

// ---- sheets (also used by the shell's sheet tabs) -----------------------------------------------------------------

pub fn sheet_add(doc: &mut Document) {
    doc.sync_sheet();
    let m = gcarve_core::document::Material { name: String::new(), ..doc.material.clone() }; // a new sheet starts unnamed
    doc.sheets.push(m);
    doc.set_active_sheet(doc.sheets.len() as i32 - 1);
}

/// Removes sheet `i` (shapes on it move to sheet 0, later sheets shift down); keeps at least one sheet.
pub fn sheet_remove(doc: &mut Document, i: i32) {
    if doc.sheets.len() <= 1 || i < 0 || i as usize >= doc.sheets.len() {
        return;
    }
    doc.sync_sheet();
    doc.sheets.remove(i as usize);
    for s in &mut doc.shapes {
        if s.sheet == i {
            s.sheet = 0;
        } else if s.sheet > i {
            s.sheet -= 1;
        }
    }
    let next = if doc.active_sheet >= i { (doc.active_sheet - 1).max(0) } else { doc.active_sheet };
    doc.active_sheet = -1; // force the material reload
    doc.set_active_sheet(next);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheets_add_switch_remove() {
        let mut doc = Document::default();
        doc.material.w = 10.0;
        sheet_add(&mut doc);
        assert_eq!((doc.sheets.len(), doc.active_sheet), (2, 1));
        doc.material.w = 20.0;
        let id = doc.add(gcarve_core::geom::make_rect(0.0, 0.0, 1.0, 1.0), None);
        assert_eq!(doc.find(id).unwrap().sheet, 1);
        doc.set_active_sheet(0);
        assert_eq!(doc.material.w, 10.0);
        assert_eq!(doc.sheets[1].w, 20.0);
        sheet_remove(&mut doc, 1);
        assert_eq!((doc.sheets.len(), doc.active_sheet, doc.find(id).unwrap().sheet), (1, 0, 0));
        sheet_remove(&mut doc, 0);
        assert_eq!(doc.sheets.len(), 1);
    }
}
