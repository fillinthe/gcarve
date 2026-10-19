//! CAM-side dialogs ported from the Qt QML dialogs, plus the widgets they share: expression length fields
//! (NumField), the feeds & speeds meter (FeedMeter), combos. Each dialog keeps its own edit state in a struct and
//! draws itself from `Dialogs::ui`, which the app calls once per frame with a `Host` of the app state it may touch.

pub mod gimport;
pub mod guide;
pub mod job;
#[cfg(not(target_arch = "wasm32"))]
pub mod jobs;
pub mod op;
pub mod posts;
pub mod prefs;
pub mod projects;
pub mod relief;
pub mod shapes;
pub mod share;
pub mod templates;
pub mod tools;
pub mod trace;
pub mod boardphoto;
pub mod vars;

use crate::canvas::Canvas;
use crate::files::Files;
use crate::posts::UserPosts;
use eframe::egui::{self, Color32, RichText};
use gcarve_core::document::{Bind, Document, Tool};
use gcarve_core::expr::identifiers;
use gcarve_core::feeds::FeedAssessment;
use std::collections::BTreeMap;

/// App state the dialogs work on.
pub struct Host<'a> {
    pub canvas: &'a mut Canvas,
    pub files: &'a Files,
    pub message: &'a mut String,
    pub posts: &'a mut UserPosts,
    pub templates: &'a mut crate::templates::Templates,
    pub cloud: &'a mut crate::cloud::Cloud,
    #[cfg(not(target_arch = "wasm32"))]
    pub jobs: &'a mut crate::jobs::Jobs,
}

#[derive(Default)]
pub struct Dialogs {
    pub op: Option<op::OpDialog>,
    pub tools: Option<tools::ToolsDialog>,
    pub posts: Option<posts::PostsDialog>,
    pub job: Option<job::JobDialog>,
    pub relief: Option<relief::ReliefDialog>,
    pub prefs_open: bool,
    pub prefs: prefs::Prefs,
    pub vars: Option<vars::VarsDialog>,
    pub constraint: Option<vars::ConstraintDialog>,
    pub templates: Option<templates::TemplatesDialog>,
    pub gimport: Option<gimport::GcodeImportDialog>,
    pub trace: Option<trace::TraceDialog>,
    pub board_photo: Option<boardphoto::BoardPhotoDialog>,
    pub shapes: Option<shapes::ShapesDialog>,
    #[cfg(not(target_arch = "wasm32"))]
    pub jobs: Option<jobs::JobsDialog>,
}

impl Dialogs {
    /// Dev hook: `GCARVE_DIALOG=<name>` opens a dialog for screenshots.
    pub fn open_named(&mut self, name: &str, canvas: &Canvas) {
        let sel: Vec<i32> = canvas.sel.iter().copied().collect();
        let kind = |k| Some(op::OpDialog::new(&canvas.doc, k, &sel));
        use gcarve_core::document::OpType::*;
        match name {
            "profile" => self.op = kind(Profile),
            "pocket" => self.op = kind(Pocket),
            "drill" => self.op = kind(Drill),
            "vcarve" => self.op = kind(VCarve),
            "bore" => self.op = kind(Bore),
            "inlay" => self.op = kind(Inlay),
            "flute" => self.op = kind(Flute),
            "chamfer" => self.op = kind(Chamfer),
            "prism" => self.op = kind(Prism),
            "texture" => self.op = kind(Texture),
            "photo" => {
                self.op = kind(Photo);
                let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/branding/appicon-1024.png")).unwrap_or_default();
                if let (Some(d), Ok((g, w, h))) = (self.op.as_mut(), trace::load_gray("icon", &bytes, 600)) {
                    d.set_photo(&g, w, h);
                }
            }
            "relief3d" => self.op = kind(Relief),
            "tools" => self.tools = Some(tools::ToolsDialog::new(&canvas.doc, 0)),
            "bits" => {
                let mut d = tools::ToolsDialog::new(&canvas.doc, 0);
                d.browser.open = true;
                if let Ok(q) = std::env::var("GCARVE_SEARCH") {
                    d.browser.preset(&q);
                }
                self.tools = Some(d);
            }
            "posts" => self.posts = Some(posts::PostsDialog::default()),
            "job" => self.job = Some(job::JobDialog::new(&canvas.doc)),
            "machines" => {
                let mut d = job::JobDialog::new(&canvas.doc);
                d.browser.show(None);
                d.browser.preset(&std::env::var("GCARVE_SEARCH").unwrap_or_default());
                self.job = Some(d);
            }
            "relief" => self.relief = Some(relief::ReliefDialog::default()),
            "prefs" => self.prefs_open = true,
            "variables" => self.vars = Some(Default::default()),
            "constraint" => self.constraint = Some(vars::ConstraintDialog::new(canvas)),
            "templates" => self.templates = Some(templates::TemplatesDialog::new("Showcase sign")),
            "shapes" => self.shapes = Some(shapes::ShapesDialog::new(&canvas.doc)),
            "trace" => {
                let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/branding/appicon-1024.png")).unwrap_or_default();
                self.trace = trace::TraceDialog::new("appicon-1024.png", &bytes, &canvas.doc).ok();
            }
            "gimport" => {
                let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference/01_V90.tap")).unwrap_or_default();
                self.gimport = Some(gimport::GcodeImportDialog::new("01_V90.tap", text, &canvas.doc));
            }
            #[cfg(not(target_arch = "wasm32"))]
            "jobs" => self.jobs = Some(jobs::JobsDialog::default()),
            _ => {}
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) {
        // a .vtdb read in the browser (sql.js) finished
        #[cfg(target_arch = "wasm32")]
        if let Some(r) = crate::vtdb::poll() {
            tools::vtdb_done(r, host);
        }
        // side files picked for a dialog
        for (tag, name, bytes) in host.files.aux_rx.try_iter() {
            match tag {
                "tools" => tools::import_file(ctx, &name, &bytes, host),
                "photo" => match (self.op.as_mut(), trace::load_gray(&name, &bytes, 600)) {
                    (Some(d), Ok((gray, w, h))) => d.set_photo(&gray, w, h),
                    (_, Err(e)) => *host.message = e,
                    _ => {}
                },
                "post" => posts::import_file(&name, &bytes, host),
                "stl" => relief::import_file(&name, &bytes, host),
                "template" => templates::import_file(&name, &bytes, host),
                "font" => {
                    *host.message = match crate::text::add_font(bytes) {
                        Ok(f) => format!("Added the font \"{f}\": it is kept on this device, and the text dialog uses it now."),
                        Err(e) => {
                            host.cloud.report("font", &crate::files::scrub(&e, &name));
                            format!("{name}: {e}")
                        }
                    }
                }
                "boardphoto" => match boardphoto::BoardPhotoDialog::new(&name, &bytes) {
                    Ok(d) => self.board_photo = Some(d),
                    Err(e) => *host.message = e,
                },
                "image" => match trace::TraceDialog::new(&name, &bytes, &host.canvas.doc) {
                    Ok(d) => self.trace = Some(d),
                    Err(e) => {
                        host.cloud.report("trace", &crate::files::scrub(&e, &name));
                        *host.message = e;
                    }
                },
                "gcode" => self.gimport = Some(gimport::GcodeImportDialog::new(&name, String::from_utf8_lossy(&bytes).into_owned(), &host.canvas.doc)),
                _ => {}
            }
        }
        if let Some(mut d) = self.op.take() {
            if d.ui(ctx, host) {
                if d.open_bits && self.tools.is_none() {
                    // Bits…: the Tool Library on top; remember the count to pick a bit added there
                    d.tools_before = Some(host.canvas.doc.tools.len());
                    self.tools = Some(tools::ToolsDialog::new(&host.canvas.doc, d.op.tool.max(0) as usize));
                }
                d.open_bits = false;
                if std::mem::take(&mut d.pick_photo) {
                    host.files.pick_into(ctx, "Picture (PNG, JPEG)", &["png", "jpg", "jpeg"], "photo");
                }
                self.op = Some(d);
            }
        }
        if let Some(mut d) = self.tools.take() {
            if d.ui(ctx, host) {
                self.tools = Some(d);
            }
        }
        if self.tools.is_none() {
            if let Some(d) = &mut self.op {
                if let Some(n) = d.tools_before.take() {
                    let len = host.canvas.doc.tools.len();
                    if len > n {
                        d.op.tool = len as i32 - 1;
                    }
                }
            }
        }
        if let Some(mut d) = self.job.take() {
            match d.ui(ctx, host) {
                job::Next::Keep => self.job = Some(d),
                job::Next::Close => {}
                job::Next::Posts => {
                    self.job = Some(d);
                    self.posts = Some(posts::PostsDialog::default());
                }
            }
        }
        if let Some(mut d) = self.posts.take() {
            if d.ui(ctx, host) {
                self.posts = Some(d);
            }
        }
        if let Some(mut d) = self.relief.take() {
            match d.ui(ctx, host) {
                relief::Next::Keep => self.relief = Some(d),
                relief::Next::Close => {}
                relief::Next::Toolpath => {
                    let sel: Vec<i32> = host.canvas.sel.iter().copied().collect();
                    self.op = Some(op::OpDialog::new(&host.canvas.doc, gcarve_core::document::OpType::Relief, &sel));
                }
            }
        }
        if self.prefs_open {
            self.prefs_open = self.prefs.ui(ctx, host);
        }
        if let Some(mut d) = self.vars.take() {
            if d.ui(ctx, host) {
                self.vars = Some(d);
            }
        }
        if let Some(mut d) = self.constraint.take() {
            if d.ui(ctx, host) {
                self.constraint = Some(d);
            }
        }
        if let Some(mut d) = self.shapes.take() {
            if d.ui(ctx, host) {
                self.shapes = Some(d);
            }
        }
        if let Some(mut d) = self.trace.take() {
            if d.ui(ctx, host) {
                self.trace = Some(d);
            }
        }
        if let Some(mut d) = self.board_photo.take() {
            if d.ui(ctx, host) {
                self.board_photo = Some(d);
            }
        }
        if let Some(mut d) = self.gimport.take() {
            if d.ui(ctx, host) {
                self.gimport = Some(d);
            }
        }
        if let Some(mut d) = self.templates.take() {
            if d.ui(ctx, host) {
                self.templates = Some(d);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(mut d) = self.jobs.take() {
            if d.ui(ctx, host) {
                self.jobs = Some(d);
            }
        }
    }
}

// ---- NumField: expression length fields -------------------------------------------------------------------------

/// Text of every numeric field of a dialog, keyed like the document's `bind` maps, shown in the project unit
/// (units.rs). A field whose text names a variable stays bound to it (Qt NumField.bound).
#[derive(Default, Clone)]
pub struct Fields {
    pub map: BTreeMap<String, String>,
    /// inches per project unit at the time the dialog opened
    unit: f64,
    dec: usize,
}

impl Fields {
    pub fn new(doc: &Document) -> Self {
        Fields { map: BTreeMap::new(), unit: doc.bare_unit_in, dec: crate::units::decimals(doc) }
    }
    fn fmt(&self, inches: f64) -> String {
        let u = if self.unit > 0.0 { self.unit } else { 1.0 };
        format!("{:.*}", self.dec.max(3), inches / u) // NumField.set: toFixed(3)
    }
    /// inches -> project-unit text
    pub fn set(&mut self, key: &str, v: f64) {
        let t = self.fmt(v);
        self.map.insert(key.into(), t);
    }
    /// inches -> project-unit text with one more decimal (ToolsDialog diameter: `decimals: dec + 1`)
    pub fn set_fine(&mut self, key: &str, v: f64) {
        let u = if self.unit > 0.0 { self.unit } else { 1.0 };
        self.map.insert(key.into(), format!("{:.*}", self.dec + 1, v / u));
    }
    /// plain number: feeds and angles, one decimal like their Qt NumFields (`decimals: 1`)
    pub fn set_raw(&mut self, key: &str, v: f64) {
        self.map.insert(key.into(), format!("{v:.1}"));
    }
    /// the bound expression when one exists, else the value
    pub fn set_bound(&mut self, key: &str, bind: &Bind, v: f64) {
        match bind.get(key) {
            Some(e) if !e.trim().is_empty() => self.map.insert(key.into(), e.clone()),
            _ => {
                let t = self.fmt(v);
                self.map.insert(key.into(), t)
            }
        };
    }
    pub fn text(&self, key: &str) -> &str {
        self.map.get(key).map(String::as_str).unwrap_or("")
    }
    /// evaluated value (inches); NaN when the text is not a valid expression
    pub fn get(&self, key: &str, doc: &Document) -> f64 {
        eval_len(self.text(key), doc).unwrap_or(f64::NAN)
    }
    pub fn get_or(&self, key: &str, doc: &Document, fallback: f64) -> f64 {
        let v = self.get(key, doc);
        if v.is_finite() { v } else { fallback }
    }
    /// plain number (feeds, counts): NaN when invalid
    pub fn raw(&self, key: &str) -> f64 {
        self.text(key).trim().parse().unwrap_or(f64::NAN)
    }
    pub fn raw_or(&self, key: &str, fallback: f64) -> f64 {
        let v = self.raw(key);
        if v.is_finite() { v } else { fallback }
    }
    /// keys whose text references variables -> expression (the rest are plain numbers, not bound)
    pub fn bindings(&self, keys: &[&str]) -> Bind {
        let mut b = Bind::new();
        for k in keys {
            if let Some(t) = self.map.get(*k) {
                if !identifiers(t).is_empty() {
                    b.insert((*k).to_string(), t.clone());
                }
            }
        }
        b
    }
    /// One length field (units::num_field: unit badge, expression tooltip). Returns true when Enter / focus loss
    /// committed a valid value.
    pub fn ui(&mut self, ui: &mut egui::Ui, key: &str, doc: &Document) -> bool {
        self.ui_sized(ui, key, doc, 110.0, true)
    }
    pub fn ui_sized(&mut self, ui: &mut egui::Ui, key: &str, doc: &Document, width: f32, length: bool) -> bool {
        let text = self.map.entry(key.into()).or_default();
        crate::units::num_field(ui, key, text, doc, length, width).is_some()
    }
}

/// "Cut depth (in)" in the project unit
pub fn unit_label(name: &str, doc: &Document) -> String {
    format!("{name} ({})", crate::units::name(doc))
}
/// "Feed XY (in/min)"
pub fn rate_label(name: &str, doc: &Document) -> String {
    format!("{name} ({}/min)", crate::units::name(doc))
}

pub fn fmt_len(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
}

/// Length expression with units and the document's variables, in inches.
pub fn eval_len(text: &str, doc: &Document) -> Result<f64, String> {
    if text.trim().is_empty() {
        return Err("empty".into());
    }
    doc.eval(text)
}

// ---- FeedMeter --------------------------------------------------------------------------------------------------

/// Feeds & speeds meter: green sweet spot in the middle, rubbing on the left, overload on the right.
pub fn feed_meter(ui: &mut egui::Ui, a: &FeedAssessment, width: f32) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0; // FeedMeter.qml spacing
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 14.0), egui::Sense::hover());
        let p = ui.painter().clone();
        let k = |c: Color32| c.gamma_multiply(0.85);
        let stops = [(0.0, Color32::from_rgb(0xc0, 0x55, 0x2f)), (0.3, Color32::from_rgb(0xd7, 0xa3, 0x34)), (0.5, Color32::from_rgb(0x3f, 0x9a, 0x5b)), (0.7, Color32::from_rgb(0xd7, 0xa3, 0x34)), (1.0, Color32::from_rgb(0xc0, 0x55, 0x2f))];
        // radius 7: round end caps in the end colour, gradient between them
        let r = rect.height() / 2.0;
        p.circle_filled(egui::pos2(rect.left() + r, rect.center().y), r, k(stops[0].1));
        p.circle_filled(egui::pos2(rect.right() - r, rect.center().y), r, k(stops[4].1));
        let inner = rect.shrink2(egui::vec2(r, 0.0));
        let mut mesh = egui::Mesh::default();
        for (i, (f, c)) in stops.iter().enumerate() {
            let x = inner.left() + inner.width() * *f;
            mesh.colored_vertex(egui::pos2(x, rect.top()), k(*c));
            mesh.colored_vertex(egui::pos2(x, rect.bottom()), k(*c));
            if i > 0 {
                let b = (i * 2) as u32;
                mesh.add_triangle(b - 2, b - 1, b);
                mesh.add_triangle(b - 1, b + 1, b);
            }
        }
        p.add(egui::Shape::mesh(mesh));
        let nx = (rect.left() + (rect.width() - 4.0) * a.meter.clamp(0.0, 1.0) as f32).round();
        p.rect(egui::Rect::from_min_size(egui::pos2(nx, rect.top() - 3.0), egui::vec2(4.0, rect.height() + 6.0)), 2.0, Color32::WHITE, egui::Stroke::new(1.0, crate::theme::c(0x222222)), egui::StrokeKind::Inside);
        let (lr, _) = ui.allocate_exact_size(egui::vec2(width, 13.0), egui::Sense::hover());
        let dim = crate::theme::TEXT.gamma_multiply(0.6);
        let f = crate::theme::font(10.0);
        p.text(lr.left_center(), egui::Align2::LEFT_CENTER, "rubs / burns", f.clone(), dim);
        p.text(lr.center(), egui::Align2::CENTER_CENTER, "sweet spot", f.clone(), dim);
        p.text(lr.right_center(), egui::Align2::RIGHT_CENTER, "breaks bit / tears", f, dim);
        if !a.message.is_empty() {
            let color = match a.level { 2 => crate::theme::DANGER, 1 => Color32::from_rgb(0xd7, 0xa3, 0x34), _ => crate::theme::TEXT };
            ui.add(egui::Label::new(RichText::new(&a.message).size(11.0).color(color)).wrap());
        }
    });
}

/// Qt Canvas::toolAdvice: the tool as it will cut in the board material, where the feeds come from, assessment.
pub fn tool_advice(doc: &Document, i: usize) -> Option<(Tool, &'static str, FeedAssessment)> {
    let t = doc.tools.get(i)?;
    let e = gcarve_core::pipeline::tool_for_cut(doc, i);
    let a = gcarve_core::pipeline::assess(doc, &e, e.feed_xy, e.feed_z, e.rpm, e.stepdown);
    let src = if gcarve_core::cam::preset_for(t, &doc.material.kind).is_some() { "preset" } else if doc.material.auto_feeds { "auto" } else { "default" };
    Some((e, src, a))
}

/// Tool line + meter as shown under the tool combo of every toolpath dialog.
pub fn tool_advice_ui(ui: &mut egui::Ui, doc: &Document, i: usize, width: f32) {
    let Some((e, src, a)) = tool_advice(doc, i) else { return };
    let note = match src { "preset" => " (saved preset)", "auto" => " (auto from chip load, edit in Tool Library)", _ => " (bit defaults)" };
    ui.add(egui::Label::new(RichText::new(format!("In {}: F {:.0} / plunge {:.0} in/min · {} rpm · pass {} in{note}", doc.material.kind, e.feed_xy, e.feed_z, e.rpm, fmt_len(e.stepdown))).small().color(crate::theme::TEXT_DIM)).wrap());
    if src == "auto" && doc.material.machine.is_none() {
        // no machine = full industrial chip loads: far too fast for a hobby or desktop router
        ui.add(egui::Label::new(RichText::new("No machine chosen: these feeds suit a heavy industrial router. Pick yours (Toolpaths › Choose Machine…) and they are scaled to it.").size(11.0).color(Color32::from_rgb(0xd7, 0xa3, 0x34))).wrap());
    }
    feed_meter(ui, &a, width);
}

pub fn combo<T: PartialEq + Copy>(ui: &mut egui::Ui, id: &str, cur: &mut T, items: &[(T, &str)], width: f32) -> bool {
    let label = items.iter().find(|(v, _)| *v == *cur).map(|(_, l)| *l).unwrap_or("");
    let mut changed = false;
    crate::widgets::combo_box(id).selected_text(label).width(width).show_ui(ui, |ui| {
        for (v, l) in items {
            if ui.selectable_value(cur, *v, *l).changed() {
                changed = true;
            }
        }
    });
    changed
}

/// Tool chooser by index (-1 allowed with `none` label).
pub fn tool_combo(ui: &mut egui::Ui, id: &str, cur: &mut i32, tools: &[Tool], none: Option<&str>, width: f32) -> bool {
    let label = if *cur < 0 { none.unwrap_or("").to_string() } else { tools.get(*cur as usize).map(|t| t.name.clone()).unwrap_or_default() };
    let mut changed = false;
    crate::widgets::combo_box(id).selected_text(label).width(width).show_ui(ui, |ui| {
        if let Some(n) = none {
            changed |= ui.selectable_value(cur, -1, n).changed();
        }
        for (i, t) in tools.iter().enumerate() {
            changed |= ui.selectable_value(cur, i as i32, &t.name).changed();
        }
    });
    changed
}

pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small().color(crate::theme::TEXT_DIM)).wrap());
}

pub const MATERIALS: [&str; 9] = ["Plywood", "Baltic birch", "MDF", "Hardwood", "Softwood", "Acrylic", "HDPE", "Aluminium", "Foam"];

/// Editable material combo (Qt editable ComboBox: type a name or pick one from the chevron).
pub fn material_combo(ui: &mut egui::Ui, id: &str, text: &mut String, width: f32) -> bool {
    let mut changed = false;
    let r = ui.add(crate::widgets::field(text).id_salt(id).desired_width(width).margin(egui::Margin { left: 9, right: 30, top: 5, bottom: 5 }));
    changed |= r.changed();
    let br = egui::Rect::from_min_max(egui::pos2(r.rect.right() - 28.0, r.rect.top()), r.rect.max);
    let b = ui.interact(br, r.id.with("pick"), egui::Sense::click());
    crate::widgets::chevron(ui.painter(), br, if b.hovered() { crate::theme::TEXT } else { crate::theme::TEXT_DIM });
    egui::Popup::menu(&b).width(r.rect.width()).show(|ui| {
        for m in MATERIALS {
            if ui.selectable_label(text == m, m).clicked() {
                *text = m.to_string();
                changed = true;
            }
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_evaluate_units_and_bind_variables() {
        let mut doc = Document::default();
        doc.material.t = 0.75;
        doc.variables.push(gcarve_core::document::Variable { name: "gap".into(), expr: "0.1".into() });
        let mut f = Fields::new(&doc);
        f.map.insert("depth".into(), "thickness + 1/8in".into());
        f.set("stepdown", 0.25);
        f.map.insert("tabWidth".into(), "12mm".into());
        assert!((f.get("depth", &doc) - 0.875).abs() < 1e-9);
        assert!((f.get("tabWidth", &doc) - 12.0 / 25.4).abs() < 1e-9);
        let b = f.bindings(&["depth", "stepdown", "tabWidth"]);
        assert_eq!(b.get("depth").map(String::as_str), Some("thickness + 1/8in"));
        assert!(!b.contains_key("stepdown") && !b.contains_key("tabWidth"));
        let mut g = Fields::new(&doc);
        g.set_bound("depth", &b, 0.5);
        assert_eq!(g.text("depth"), "thickness + 1/8in");
        doc.bare_unit_in = crate::units::BARE[2];
        let mut mm = Fields::new(&doc);
        mm.set("depth", 1.0);
        assert_eq!(mm.text("depth"), "25.400");
        assert!((mm.get("depth", &doc) - 1.0).abs() < 1e-9);
        assert_eq!(fmt_len(0.125), "0.125");
        assert_eq!(fmt_len(2.0), "2");
    }
}
