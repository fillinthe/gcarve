//! Preferences (Qt PreferencesDialog.qml): UI scale, larger text, high contrast, larger handles. All off by default
//! (the stock look must not change); saved in the config store. Plus the account-synced part (Qt Theme.synced):
//! new-document defaults, grid and snap settings, edited here, in the status-bar Snap popover and the View menu.

use super::Host;
use eframe::egui::{self, Color32, RichText};
use serde::{Deserialize, Serialize};

const KEY: &str = "prefs.json";
pub const SCALES: [f32; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub ui_scale: f32,
    pub large_text: bool,
    pub high_contrast: bool,
    pub large_handles: bool,
    /// the "Start a job" card (View ▸ Start Card; its close button turns it off for good)
    pub start_card: bool,
    /// new-document defaults + grid + snap, synced to the account (cloud.rs fetch_prefs / push_prefs)
    pub synced: Synced,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { ui_scale: 1.0, large_text: false, high_contrast: false, large_handles: false, start_card: true, synced: Synced::default() }
    }
}

// ---- snap / grid / new-document defaults (Qt Theme.snapPrefs / gridPrefs / newDoc) --------------------------------

/// Snap kinds (Qt Canvas::SnapKind bits)
pub const SNAP_POINTS: u8 = 1;
pub const SNAP_MIDPOINTS: u8 = 2;
pub const SNAP_CENTRES: u8 = 4;
pub const SNAP_QUADRANTS: u8 = 8;
pub const SNAP_EDGE: u8 = 16;
pub const SNAP_GRID: u8 = 32;
pub const SNAP_ALL: u8 = 63;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapPrefs {
    /// SNAP_* bits
    pub kinds: u8,
    /// snap distance in screen pixels (2..50)
    pub px: f64,
}
impl Default for SnapPrefs {
    fn default() -> Self {
        SnapPrefs { kinds: SNAP_ALL, px: 8.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridPrefs {
    pub show: bool,
    /// inches; 0 = auto (1 / 10 / 100 … of the project unit, following the zoom)
    pub spacing: f64,
    /// major line every N
    pub major: i32,
    pub rulers: bool,
}
impl Default for GridPrefs {
    fn default() -> Self {
        GridPrefs { show: true, spacing: 0.0, major: 10, rulers: true }
    }
}

/// Board and setup of every new document (opened files keep their own). Lengths in inches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NewDoc {
    pub board_w: f64,
    pub board_h: f64,
    pub board_t: f64,
    pub material: String,
    /// XY zero: 0 BL, 1 BR, 2 TL, 3 TR, 4 centre
    pub origin: i32,
    pub z_bottom: bool,
    /// "in" | "ft" | "mm"
    pub units: String,
    pub post: String,
    pub snap_on: bool,
}
impl Default for NewDoc {
    fn default() -> Self {
        NewDoc { board_w: 60.0, board_h: 96.0, board_t: 0.75, material: "Plywood".into(), origin: 0, z_bottom: false, units: "in".into(), post: "wincnc-single".into(), snap_on: false }
    }
}

const LAST_MACHINE: &str = "machine-last.json";

/// The machine (and its post) chosen most recently on this device, for new drawings.
#[cfg_attr(test, allow(dead_code))]
pub fn last_machine() -> Option<(gcarve_core::document::MachineLimits, String)> {
    let v: serde_json::Value = serde_json::from_str(&crate::store::load(LAST_MACHINE)?).ok()?;
    Some((serde_json::from_value(v.get("machine")?.clone()).ok()?, v.get("post").and_then(|p| p.as_str()).unwrap_or("").to_string()))
}

/// Remember (or, with None, forget) the machine for new drawings.
pub fn set_last_machine(machine: Option<&gcarve_core::document::MachineLimits>, post: &str) {
    match machine {
        Some(m) => crate::store::save(LAST_MACHINE, &serde_json::json!({ "machine": m, "post": post }).to_string()),
        None => crate::store::remove(LAST_MACHINE),
    }
}

impl NewDoc {
    /// A fresh canvas takes the defaults (Qt Canvas::applyNewDocDefaults).
    pub fn apply(&self, canvas: &mut crate::canvas::Canvas) {
        let doc = &mut canvas.doc;
        let m = &mut doc.material;
        for (v, d) in [(&mut m.w, self.board_w), (&mut m.h, self.board_h), (&mut m.t, self.board_t)] {
            if d > 0.0 {
                *v = d;
            }
        }
        if !self.material.is_empty() {
            m.kind = self.material.clone();
        }
        m.origin = self.origin.clamp(0, 4);
        m.z_bottom = self.z_bottom;
        if !self.post.is_empty() {
            m.post = self.post.clone();
        }
        // the machine picked last time (this device): its limits scale the suggested feeds, its controller's post
        // is used, and a default board bigger than its table starts at the table size
        #[cfg(not(test))]
        if let Some((machine, post)) = last_machine() {
            let fits = |w: f64, h: f64| machine.travel_x <= 0.0 || machine.travel_y <= 0.0 || (w <= machine.travel_x + 1e-6 && h <= machine.travel_y + 1e-6);
            if !fits(m.w, m.h) && !fits(m.h, m.w) {
                (m.w, m.h) = (machine.travel_x, machine.travel_y);
            }
            if !post.is_empty() {
                m.post = post;
            }
            m.machine = Some(machine);
        }
        doc.sheets = vec![m.clone()];
        doc.active_sheet = 0;
        doc.bare_unit_in = crate::units::BARE[crate::units::NAMES.iter().position(|n| *n == self.units).unwrap_or(0)];
        canvas.snap = self.snap_on;
        canvas.view = None; // fit the new board
        canvas.rev += 1;
    }
}

/// What the account keeps (Qt Theme.synced): the new-document fields at the top level plus `grid` and `snap` objects,
/// same JSON names as the Qt app, so both apps share one account copy.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Synced {
    #[serde(flatten)]
    pub new_doc: NewDoc,
    pub grid: GridPrefs,
    pub snap: SnapPrefs,
}

impl Synced {
    /// Account copy over the local one (Qt Theme.applySynced): keys the server has replace ours, missing / null /
    /// unknown keys keep the local value.
    pub fn merged(&self, server: &serde_json::Value) -> Synced {
        use serde_json::Value;
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let (Some(dst), Some(src)) = (v.as_object_mut(), server.as_object()) {
            for (k, val) in src {
                match (dst.get_mut(k), val) {
                    (Some(Value::Object(d)), Value::Object(s)) => {
                        for (k2, v2) in s.iter().filter(|(_, v2)| !v2.is_null()) {
                            if let Some(slot) = d.get_mut(k2) {
                                *slot = v2.clone();
                            }
                        }
                    }
                    (Some(d), v) if !v.is_null() && !v.is_object() => *d = v.clone(),
                    _ => {}
                }
            }
        }
        serde_json::from_value(v).unwrap_or_else(|_| self.clone())
    }
}

/// A length in inches edited in the project unit (expressions and unit suffixes allowed); true when committed.
fn len_field(ui: &mut egui::Ui, id: &str, inches: &mut f64, doc: &gcarve_core::document::Document, width: f32) -> bool {
    let key = ui.id().with(id);
    let mut buf = ui.data(|d| d.get_temp::<String>(key)).unwrap_or_else(|| if *inches > 0.0 { crate::units::text(doc, *inches) } else { String::new() });
    // the typed text lives in temp data until Enter / focus loss commits it
    match crate::units::num_field(ui, id, &mut buf, doc, true, width) {
        Some(v) if v > 0.0 => {
            *inches = v;
            ui.data_mut(|d| d.remove::<String>(key));
            true
        }
        Some(_) => {
            ui.data_mut(|d| d.remove::<String>(key)); // not a length: back to the stored value
            false
        }
        None => {
            if *inches <= 0.0 && buf.is_empty() || buf == crate::units::text(doc, *inches) {
                ui.data_mut(|d| d.remove::<String>(key));
            } else {
                ui.data_mut(|d| d.insert_temp(key, buf));
            }
            false
        }
    }
}

/// Snap kinds + distance, grid and rulers (Qt SnapGridSettings.qml): the status-bar Snap popover, View ▸ Snap
/// Settings… and Preferences share it. Grid spacing shows in the project units, stored in inches.
pub fn snap_grid_ui(ui: &mut egui::Ui, s: &mut Synced, doc: &gcarve_core::document::Document) {
    use crate::widgets::{checkbox, SpinBox, TipExt};
    let head = |ui: &mut egui::Ui, t: &str| ui.label(RichText::new(t).font(crate::widgets::bold(crate::theme::FONT_SIZE)).color(crate::theme::TEXT_DIM));
    let note = |ui: &mut egui::Ui, t: &str| ui.add(egui::Label::new(RichText::new(t).size(11.0).color(crate::theme::TEXT.gamma_multiply(0.6))).wrap());
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.set_max_width(300.0);
    head(ui, "Snap to");
    egui::Grid::new("snapkinds").num_columns(2).spacing([14.0, 2.0]).show(ui, |ui| {
        let kinds = [("Points (nodes)", SNAP_POINTS), ("Midpoints", SNAP_MIDPOINTS), ("Centres", SNAP_CENTRES), ("Quadrants", SNAP_QUADRANTS), ("On edge", SNAP_EDGE), ("Grid", SNAP_GRID)];
        for (i, (label, bit)) in kinds.into_iter().enumerate() {
            let mut on = s.snap.kinds & bit != 0;
            if checkbox(ui, &mut on, label).changed() {
                s.snap.kinds = if on { s.snap.kinds | bit } else { s.snap.kinds & !bit };
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Snap distance");
        ui.add(SpinBox::new(&mut s.snap.px, 2.0..=50.0).width(90.0));
        ui.label("px");
    });
    note(ui, "Measure and Dimension always snap to objects; the boxes above still apply.");
    ui.add_space(8.0);
    head(ui, "Grid");
    checkbox(ui, &mut s.grid.show, "Show grid");
    ui.horizontal(|ui| {
        ui.label("Spacing");
        let mut auto = s.grid.spacing <= 0.0;
        if checkbox(ui, &mut auto, "Auto").tip("Follows the zoom (1 / 10 / 100 … of the unit)").changed() {
            s.grid.spacing = if auto { 0.0 } else { doc.bare_unit_in }; // one project unit to start from
        }
        ui.add_enabled_ui(!auto, |ui| {
            let mut v = if auto { 0.0 } else { s.grid.spacing };
            if len_field(ui, "gridspacing", &mut v, doc, 90.0) && !auto {
                s.grid.spacing = v;
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label("Major line every");
        ui.add(SpinBox::new(&mut s.grid.major, 1..=100).width(90.0));
        ui.label("lines");
    });
    checkbox(ui, &mut s.grid.rulers, "Rulers");
}

impl Prefs {
    pub fn load() -> Self {
        crate::store::load(KEY).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        crate::store::save(KEY, &serde_json::to_string_pretty(self).unwrap_or_default());
    }

    /// Larger handles: canvas hit / snap / node radii multiplier (Qt Canvas::hitScale)
    pub fn hit_scale(&self) -> f64 {
        if self.large_handles { 1.7 } else { 1.0 }
    }

    /// Apply to the egui context (over theme::apply). `shot` = screenshot runs keep the stock scale.
    pub fn apply(&self, ctx: &egui::Context, shot: bool) {
        crate::theme::apply(ctx);
        if !shot {
            ctx.set_zoom_factor(self.ui_scale);
        }
        let k = if self.large_text { 15.0 / 13.0 } else { 1.0 };
        ctx.all_styles_mut(|s| {
            for f in s.text_styles.values_mut() {
                f.size = (f.size * k * 100.0).round() / 100.0;
            }
            s.spacing.interact_size.y = if self.large_text { 34.0 } else { crate::theme::CONTROL_HEIGHT }; // Theme.controlHeight: rows, buttons, list items
        });
        if self.high_contrast {
            let mut v = ctx.global_style().visuals.clone();
            v.override_text_color = Some(Color32::WHITE);
            v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, Color32::BLACK);
            v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, Color32::BLACK);
            v.selection.bg_fill = Color32::from_rgb(0x3d, 0x8e, 0xff);
            v.hyperlink_color = Color32::from_rgb(0x8c, 0xc8, 0xff);
            ctx.set_visuals(v);
        }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut open = true;
        let before = self.clone();
        crate::widgets::Dialog::new("Preferences").open(&mut open).close_footer().show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
            let section = |ui: &mut egui::Ui, t: &str| {
                ui.add_space(4.0);
                ui.label(RichText::new(t).font(crate::widgets::bold(crate::theme::FONT_SIZE)).color(crate::theme::TEXT_DIM));
            };
            let note = |ui: &mut egui::Ui, t: &str| {
                ui.allocate_ui(egui::vec2(220.0, 30.0), |ui| ui.add(egui::Label::new(RichText::new(t).size(11.0).color(crate::theme::TEXT.gamma_multiply(0.6))).wrap()));
            };
            section(ui, "Interface");
            ui.horizontal(|ui| {
                ui.label("Scale");
                let cur = SCALES.iter().position(|s| (*s - self.ui_scale).abs() < 0.01).unwrap_or(0);
                let mut idx = cur;
                crate::widgets::combo_box("scale").selected_text(format!("{}%", (SCALES[cur] * 100.0).round())).width(110.0).show_ui(ui, |ui| {
                    for (i, s) in SCALES.iter().enumerate() {
                        ui.selectable_value(&mut idx, i, format!("{}%", (s * 100.0).round()));
                    }
                });
                self.ui_scale = SCALES[idx];
                note(ui, "Whole interface, including the canvas");
            });
            section(ui, "Accessibility");
            for (flag, label, h) in [(&mut self.large_text, "Larger text", "15 px labels and taller controls"), (&mut self.high_contrast, "High contrast", "White text, black borders, brighter blue (WCAG AA+)"), (&mut self.large_handles, "Larger handles", "Nodes, grips and snap targets are 1.7× easier to hit")] {
                ui.horizontal(|ui| {
                    crate::widgets::switch(ui, flag, label);
                    note(ui, h);
                });
            }
            ui.add_space(6.0);
            if crate::widgets::right(ui, 140.0, |ui| ui.add(crate::widgets::Button::new("Reset to defaults").flat().min_width(140.0))).clicked() {
                *self = Prefs { start_card: self.start_card, synced: self.synced.clone(), ..Prefs::default() };
            }
            section(ui, "New documents");
            note(ui, "Used by every new tab; opened files keep their own board. Synced to your GCarve account when signed in.");
            let doc = &host.canvas.doc;
            let nd = &mut self.synced.new_doc;
            egui::Grid::new("newdoc").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Board W × H × T");
                ui.horizontal(|ui| {
                    len_field(ui, "nd.w", &mut nd.board_w, doc, 90.0);
                    ui.label("×");
                    len_field(ui, "nd.h", &mut nd.board_h, doc, 90.0);
                    ui.label("×");
                    len_field(ui, "nd.t", &mut nd.board_t, doc, 90.0);
                });
                ui.end_row();
                ui.label("Material");
                super::material_combo(ui, "nd.material", &mut nd.material, 220.0);
                ui.end_row();
                ui.label("Project units");
                let mut u = crate::units::NAMES.iter().position(|n| *n == nd.units).unwrap_or(0);
                if super::combo(ui, "nd.units", &mut u, &[(0, "inches"), (1, "feet"), (2, "mm")], 120.0) {
                    nd.units = crate::units::NAMES[u].into();
                }
                ui.end_row();
                ui.label("XY zero");
                let aspect = if nd.board_w > 0.0 && nd.board_h > 0.0 { nd.board_w / nd.board_h } else { 1.5 };
                let row_h = (96.0 / aspect.clamp(0.5, 3.0)).round() as f32 + 8.0; // the picker's height (as in the Job dialog)
                ui.allocate_ui_with_layout(egui::vec2(0.0, row_h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    super::job::origin_picker(ui, &mut nd.origin, aspect);
                    ui.label(super::job::ORIGIN_NAMES[nd.origin.clamp(0, 4) as usize]);
                });
                ui.end_row();
                ui.label("Z zero");
                super::combo(ui, "nd.z", &mut nd.z_bottom, &[(false, "Material surface (touch top)"), (true, "Machine bed (touch bottom)")], 240.0);
                ui.end_row();
                ui.label("Post-processor");
                let posts = host.posts.all();
                let cur = posts.iter().find(|(p, _)| p.id == nd.post).map(|(p, _)| p.name.clone()).unwrap_or_else(|| nd.post.clone());
                crate::widgets::combo_box("nd.post").width(240.0).selected_text(cur).show_ui(ui, |ui| {
                    for (p, _) in &posts {
                        if ui.selectable_label(p.id == nd.post, &p.name).clicked() {
                            nd.post = p.id.clone();
                        }
                    }
                });
                ui.end_row();
                ui.label("Snap");
                crate::widgets::switch(ui, &mut nd.snap_on, "On in new documents");
                ui.end_row();
            });
            section(ui, "Snap & grid");
            snap_grid_ui(ui, &mut self.synced, doc);
            if crate::widgets::right(ui, 220.0, |ui| ui.add(crate::widgets::Button::new("Reset new-document defaults").flat().min_width(220.0))).clicked() {
                self.synced = Synced::default();
            }
        });
        if *self != before {
            self.save();
            self.apply(ctx, false);
            host.canvas.hit_scale = self.hit_scale(); // other tabs take it when they become current
        }
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn synced_json_uses_the_qt_names_and_round_trips() {
        let s = Synced::default();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v, json!({"boardW": 60.0, "boardH": 96.0, "boardT": 0.75, "material": "Plywood", "origin": 0, "zBottom": false, "units": "in",
                             "post": "wincnc-single", "snapOn": false, "grid": {"show": true, "spacing": 0.0, "major": 10, "rulers": true}, "snap": {"kinds": 63, "px": 8.0}}));
        let mut t = s.clone();
        t.new_doc.board_w = 48.0;
        t.new_doc.units = "mm".into();
        t.grid.spacing = 0.5;
        t.snap.kinds = SNAP_POINTS | SNAP_GRID;
        assert_eq!(serde_json::from_value::<Synced>(serde_json::to_value(&t).unwrap()).unwrap(), t);
        // the whole Prefs file keeps them too
        let p = Prefs { synced: t.clone(), ..Prefs::default() };
        assert_eq!(serde_json::from_str::<Prefs>(&serde_json::to_string(&p).unwrap()).unwrap(), p);
    }

    #[test]
    fn account_copy_wins_key_by_key() {
        let mut local = Synced::default();
        local.new_doc.material = "MDF".into();
        local.grid.major = 5;
        // a Qt account copy: ints where we have floats, a partial grid object, a null, an unknown key
        let server = json!({"boardW": 24, "units": "mm", "post": null, "grid": {"show": false}, "snap": {"kinds": 1, "px": 12}, "future": 1});
        let m = local.merged(&server);
        assert_eq!(m.new_doc.board_w, 24.0);
        assert_eq!(m.new_doc.board_h, 96.0, "missing: local");
        assert_eq!(m.new_doc.units, "mm");
        assert_eq!(m.new_doc.post, "wincnc-single", "null: local");
        assert_eq!(m.new_doc.material, "MDF");
        assert!(!m.grid.show && m.grid.major == 5 && m.grid.rulers);
        assert_eq!((m.snap.kinds, m.snap.px), (1, 12.0));
        // garbage keeps everything local
        assert_eq!(local.merged(&json!({"boardW": "wide"})), local);
        assert_eq!(local.merged(&json!(null)), local);
    }

    #[test]
    fn new_documents_take_the_defaults() {
        let mut c = crate::canvas::Canvas::default();
        assert_eq!((c.doc.material.w, c.doc.material.h), (48.0, 24.0), "core default untouched (goldens)");
        NewDoc::default().apply(&mut c);
        let m = &c.doc.material;
        assert_eq!((m.w, m.h, m.t, m.kind.as_str(), m.origin, m.z_bottom, m.post.as_str()), (60.0, 96.0, 0.75, "Plywood", 0, false, "wincnc-single"));
        assert_eq!(c.doc.sheets.len(), 1);
        assert_eq!(c.doc.sheets[0].w, 60.0);
        assert!(!c.snap && c.doc.bare_unit_in == 1.0);
        let nd = NewDoc { board_w: 1220.0 / 25.4, units: "mm".into(), origin: 9, snap_on: true, material: String::new(), ..NewDoc::default() };
        nd.apply(&mut c);
        assert!((c.doc.material.w - 48.031).abs() < 1e-3 && c.doc.material.origin == 4 && c.snap);
        assert_eq!(c.doc.material.kind, "Plywood", "empty material keeps the board's");
        assert_eq!(crate::units::name(&c.doc), "mm");
    }
}
