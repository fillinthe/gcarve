//! Community projects (gcarve.app/projects/, server community.go). Make It: the step after opening someone's
//! project, so it cuts on this machine (machine and post, bits from the own library, board thickness). Share to
//! Community (File menu): the project, a picture of the simulated cut, photos, the vectors as SVG / DXF and the
//! facts the project page shows; it waits for review before it is public.

use super::Host;
use crate::units;
use eframe::egui::{self, RichText};
use gcarve_core::document::{Document, Tool, ToolType};
use gcarve_core::pipeline;
use std::collections::BTreeSet;

fn section(ui: &mut egui::Ui, t: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(t).family(crate::widgets::bold_family()));
}

/// the bits the toolpaths use (op bits and clearing bits), in tool order
fn used_tools(doc: &Document) -> Vec<usize> {
    let set: BTreeSet<usize> = doc.ops.iter().flat_map(|o| [o.tool, o.clear_tool]).filter_map(|t| usize::try_from(t).ok()).filter(|&t| t < doc.tools.len()).collect();
    set.into_iter().collect()
}

/// the same cutter: type, diameter (and V angle)
pub fn same_bit(a: &Tool, b: &Tool) -> bool {
    a.kind == b.kind && (a.diameter - b.diameter).abs() < 0.003 && (a.kind != ToolType::VBit || (a.angle - b.angle).abs() < 0.6)
}

fn kind_word(k: ToolType) -> &'static str {
    match k {
        ToolType::EndMill => "endmill",
        ToolType::VBit => "vbit",
        ToolType::BallNose => "ballnose",
        ToolType::RoundOver => "roundover",
    }
}

fn bit_text(doc: &Document, t: &Tool) -> String {
    let size = format!("{} {}", units::text(doc, t.diameter), units::name(doc));
    match t.kind {
        ToolType::VBit => format!("{:.0}° V-bit, {size}", t.angle),
        k => format!("{} {}", match k {
            ToolType::BallNose => "ball nose",
            ToolType::RoundOver => "round-over",
            _ => "end mill",
        }, size),
    }
}

/// The machine picked last on this device goes onto an opened project (its post and limits), every sheet.
pub fn use_my_machine(doc: &mut Document) {
    #[cfg(not(test))]
    if let Some((machine, post)) = super::prefs::last_machine() {
        for m in doc.sheets.iter_mut().chain(std::iter::once(&mut doc.material)) {
            m.machine = Some(machine.clone());
            if !post.is_empty() {
                m.post = post.clone();
            }
        }
    }
    #[cfg(test)]
    let _ = doc;
}

/// one project bit and what replaces it: None = keep the project's
struct BitRow {
    idx: usize,
    pick: Option<usize>,
}

/// How an opened project differs from the setup used on this computer: the machine and post picked here last
/// (`mine`) and the bit library (`lib`). Empty = it already fits, nothing to ask.
pub fn differences(doc: &Document, mine: Option<(&gcarve_core::document::MachineLimits, &str)>, lib: &[Tool]) -> Vec<String> {
    let mut out = Vec::new();
    if let Some((m, post)) = mine {
        let theirs = doc.material.machine.as_ref();
        if theirs.is_none_or(|t| t.id != m.id) || (!post.is_empty() && doc.material.post != post) {
            out.push(match theirs.map(|t| t.name.as_str()).filter(|n| !n.is_empty()) {
                Some(n) => format!("It was set up for a {n}; you use a {}.", m.name),
                None => format!("It has no machine set; you use a {}.", m.name),
            });
        }
    }
    if !lib.is_empty() {
        let unknown: Vec<&str> = used_tools(doc).into_iter().map(|i| &doc.tools[i]).filter(|t| !lib.iter().any(|l| same_bit(l, t))).map(|t| t.name.as_str()).collect();
        if !unknown.is_empty() {
            out.push(format!("Not in your bit library: {}.", unknown.join(", ")));
        }
    }
    out
}

/// The differences for this computer (its last machine and bit library)
pub fn differences_here(doc: &Document) -> Vec<String> {
    let mine = super::prefs::last_machine();
    differences(doc, mine.as_ref().map(|(m, p)| (m, p.as_str())), &crate::toollib::load())
}

pub struct MakeIt {
    title: String,
    /// why it was offered on opening (differences from this computer's setup)
    reasons: Vec<String>,
    /// the machine used here, offered when the project has another one: switch to it on Make it
    mine: Option<(gcarve_core::document::MachineLimits, String)>,
    switch: bool,
    thick: String,
    old_t: f64,
    lib: Vec<Tool>,
    bits: Vec<BitRow>,
    /// Choose machine… was pressed (Dialogs opens the machine browser)
    pub want_machine: bool,
}

pub enum Next {
    Keep,
    Close,
    /// applied: show the simulation
    Made,
}

impl MakeIt {
    pub fn new(doc: &Document, title: &str) -> Self {
        let lib = crate::toollib::load();
        let bits = used_tools(doc)
            .into_iter()
            .map(|idx| {
                let t = &doc.tools[idx];
                let pick = lib.iter().position(|l| l.name == t.name && same_bit(l, t)).or_else(|| lib.iter().position(|l| same_bit(l, t)));
                BitRow { idx, pick }
            })
            .collect();
        let mine = super::prefs::last_machine().filter(|(m, p)| doc.material.machine.as_ref().is_none_or(|t| t.id != m.id) || (!p.is_empty() && doc.material.post != *p));
        MakeIt { title: title.into(), reasons: Vec::new(), switch: mine.is_some(), mine, thick: units::text(doc, doc.material.t), old_t: doc.material.t, lib, bits, want_machine: false }
    }

    /// Offered on opening a project made with another setup: says what differs
    pub fn with_reasons(mut self, reasons: Vec<String>) -> Self {
        self.reasons = reasons;
        self
    }

    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> Next {
        let mut open = true;
        let mut next = Next::Keep;
        crate::widgets::Dialog::new("Make it on your machine").open(&mut open).width(560.0).show(ctx, |ui| {
            let doc = &host.canvas.doc;
            ui.label(RichText::new(&self.title).strong().size(15.0));
            if self.reasons.is_empty() {
                ui.label(RichText::new("Set it up for your machine, your bits and your board. You can change any of it later.").color(crate::theme::TEXT_DIM));
            } else {
                ui.label(RichText::new("This project was saved with a different setup from yours:").color(crate::theme::ORANGE));
                for r in &self.reasons {
                    ui.label(RichText::new(format!("•  {r}")).color(crate::theme::ORANGE));
                }
                ui.label(RichText::new("Switch it to your machine and bits below, or Skip to keep it as it is.").color(crate::theme::TEXT_DIM));
            }
            ui.add_space(8.0);
            section(ui, "Machine");
            let post = pipeline::active_post(doc, &|id| host.posts.find(id));
            if let Some((m, p)) = &self.mine {
                let their = doc.material.machine.as_ref().map_or("no machine".to_string(), |t| t.name.clone());
                let p_name = gcarve_core::postdef::find_post(p).cloned().or_else(|| host.posts.find(p)).map_or(p.clone(), |d| d.name);
                ui.checkbox(&mut self.switch, format!("Use my machine: {} (G-code for {p_name})", m.name)).on_hover_text(format!("The project is set up for {their}, G-code for {}", post.name));
            }
            ui.horizontal(|ui| {
                let name = doc.material.machine.as_ref().map(|m| m.name.clone()).filter(|n| !n.is_empty());
                ui.label(match &name {
                    Some(n) => RichText::new(format!("{n}  ·  G-code for {}", post.name)),
                    None => RichText::new(format!("No machine chosen yet  ·  G-code for {}", post.name)).color(crate::theme::ORANGE),
                });
                if ui.button("Choose machine…").clicked() {
                    self.want_machine = true;
                }
            });
            ui.add_space(6.0);
            section(ui, "Board");
            ui.horizontal(|ui| {
                ui.label("Your board's thickness");
                units::num_field(ui, "makeit-t", &mut self.thick, doc, true, 90.0);
                ui.label(RichText::new(format!("drawn for {}", units::text(doc, self.old_t))).color(crate::theme::TEXT_DIM));
            });
            ui.label(RichText::new("Cuts that go through the board follow the new thickness; the other depths stay as drawn.").small().color(crate::theme::TEXT_DIM));
            ui.add_space(6.0);
            section(ui, "Bits");
            if self.bits.is_empty() {
                ui.label(RichText::new("This project has no toolpaths yet.").color(crate::theme::TEXT_DIM));
            }
            for (r, row) in self.bits.iter_mut().enumerate() {
                let t = &doc.tools[row.idx];
                ui.horizontal(|ui| {
                    ui.label(RichText::new(bit_text(doc, t)).strong()).on_hover_text(&t.name);
                    ui.label("→");
                    let choices: Vec<usize> = {
                        let mut c: Vec<usize> = (0..self.lib.len()).filter(|&i| self.lib[i].kind == t.kind).collect();
                        c.sort_by(|&a, &b| (self.lib[a].diameter - t.diameter).abs().total_cmp(&(self.lib[b].diameter - t.diameter).abs()));
                        c
                    };
                    let shown = row.pick.map_or_else(|| format!("The project's bit ({})", t.name), |i| self.lib[i].name.clone());
                    crate::widgets::combo_box(("makeit-bit", r)).width(300.0).selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(&mut row.pick, None, format!("The project's bit ({})", t.name));
                        for i in choices {
                            let l = &self.lib[i];
                            let tag = if same_bit(l, t) { "  ✓ same size" } else { "" };
                            ui.selectable_value(&mut row.pick, Some(i), format!("{}{tag}", l.name));
                        }
                    });
                });
                if row.pick.is_some_and(|i| !same_bit(&self.lib[i], t)) {
                    ui.label(RichText::new("A different size cuts differently: check the result in Simulate.").small().color(crate::theme::ORANGE));
                }
            }
            if !self.bits.is_empty() && self.bits.iter().all(|b| b.pick.is_none()) {
                ui.label(RichText::new("Your bits keep their own feeds; the project's bits get feeds suggested for your machine.").small().color(crate::theme::TEXT_DIM));
            }
            match crate::widgets::buttons(ui, &[("Skip", false), ("Make it", true)]) {
                Some(0) => next = Next::Close,
                Some(1) => next = Next::Made,
                _ => {}
            }
        });
        if !open {
            return Next::Close;
        }
        if let Next::Made = next {
            self.apply(host);
        }
        next
    }

    fn apply(&self, host: &mut Host) {
        let canvas = &mut *host.canvas;
        canvas.doc.checkpoint();
        let doc = &mut canvas.doc;
        if let (true, Some((m, post))) = (self.switch, &self.mine) {
            for s in doc.sheets.iter_mut().chain(std::iter::once(&mut doc.material)) {
                s.machine = Some(m.clone());
                if !post.is_empty() {
                    s.post = post.clone();
                }
            }
        }
        if let Some(t) = units::get(doc, &self.thick, true).ok().filter(|t| *t > 0.0) {
            if (t - self.old_t).abs() > 1e-6 {
                set_thickness(doc, self.old_t, t);
            }
        }
        for row in &self.bits {
            if let Some(i) = row.pick {
                doc.tools[row.idx] = self.lib[i].clone();
            }
        }
        canvas.apply_bindings();
        canvas.touch();
        *host.message = "Ready to cut: watch it in Simulate, then Export G-code".into();
    }
}

/// A new board thickness: every sheet drawn at `old` gets `new`, and cuts at or below the old bottom (through
/// cuts, with their spoilboard margin) move with it. Depths bound to an expression follow it on their own.
pub fn set_thickness(doc: &mut Document, old: f64, new: f64) {
    for m in doc.sheets.iter_mut().chain(std::iter::once(&mut doc.material)) {
        if (m.t - old).abs() < 1e-6 && !m.bind.contains_key("t") {
            m.t = new;
        }
    }
    for op in &mut doc.ops {
        if !op.bind.contains_key("depth") && op.depth >= old - 1e-3 {
            op.depth += new - old;
        }
    }
}

// ---- Share to Community ------------------------------------------------------------------------------------------

const LICENSES: [(&str, &str, &str); 3] = [
    ("cc-by", "Free to use, credit me (CC BY 4.0)", "Anyone may cut, sell and change it, as long as they credit you"),
    ("cc-by-nc", "Free for non-commercial use, credit me (CC BY-NC 4.0)", "Anyone may cut and change it, not sell it; credit you"),
    ("cc0", "No conditions (CC0)", "Public domain: anyone may do anything with it"),
];
const MAX_PHOTOS: usize = 4;

pub struct ShareDialog {
    title: String,
    summary: String,
    tags: String,
    license: usize,
    author: String,
    rights: bool,
    /// the simulated cut (JPEG) and its preview texture
    cover: Option<Vec<u8>>,
    cover_rgba: Option<(Vec<u8>, usize, usize)>,
    cover_tex: Option<egui::TextureHandle>,
    /// photos of the real thing: (name, JPEG)
    photos: Vec<(String, Vec<u8>)>,
    minutes: f64,
    problems: Vec<String>,
    sending: bool,
    pub sent: Option<Result<String, String>>,
    /// Add photos… was pressed (Dialogs opens the file picker)
    pub want_photo: bool,
}

/// JPEG of straight RGBA, through-cut holes on a dark spoilboard
fn jpeg(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let bg = [52u16, 47, 42];
    let rgb: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|p| (0..3).map(move |c| ((u16::from(p[c]) * u16::from(p[3]) + bg[c] * (255 - u16::from(p[3]))) / 255) as u8)).collect();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 86).encode(&rgb, w, h, image::ExtendedColorType::Rgb8).ok()?;
    Some(out)
}

/// a photo for the page: a PNG or JPEG, at most 1600 px on its long side, as JPEG
pub fn photo_jpeg(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("Not a picture GCarve can read: {e}"))?;
    let img = if img.width().max(img.height()) > 1600 { img.resize(1600, 1600, image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    jpeg(&rgba, rgba.width(), rgba.height()).ok_or_else(|| "Could not convert the picture".to_string())
}

impl ShareDialog {
    pub fn new(doc: &Document, name: &str) -> Self {
        let gen = pipeline::regenerate(doc);
        let mut problems = Vec::new();
        if doc.ops.is_empty() {
            problems.push("Add the toolpaths first: people open the project to cut it.".to_string());
        }
        problems.extend(gen.warnings.iter().filter(|w| w.ends_with("no tool")).cloned());
        if doc.ops.iter().zip(&gen.paths).all(|(_, p)| p.is_empty()) && !doc.ops.is_empty() {
            problems.push("The toolpaths cut nothing on this sheet.".to_string());
        }
        let carved = crate::setupsheet::carved(doc, &gen, 1_200_000);
        let cover = carved.as_ref().and_then(|(rgba, w, h, _)| jpeg(rgba, *w, *h));
        let cover_rgba = carved.map(|(rgba, w, h, _)| (rgba, w as usize, h as usize));
        let title = name.trim_end_matches(".tlp").trim_end_matches(".crv").to_string();
        ShareDialog { title: if title == "Untitled" { String::new() } else { title }, summary: String::new(), tags: String::new(), license: 0, author: String::new(), rights: false, cover, cover_rgba, cover_tex: None, photos: Vec::new(), minutes: gen.minutes.iter().sum(), problems, sending: false, sent: None, want_photo: false }
    }

    pub fn add_photo(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if self.photos.len() >= MAX_PHOTOS {
            return Err(format!("Up to {MAX_PHOTOS} photos"));
        }
        self.photos.push((name.to_string(), photo_jpeg(bytes)?));
        Ok(())
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut open = true;
        let mut close = false;
        let mut send = false;
        crate::widgets::Dialog::new("Share to Community").open(&mut open).width(600.0).show(ctx, |ui| {
            if let Some(r) = &self.sent {
                match r {
                    Ok(url) => {
                        ui.label(RichText::new("Thank you! Your project is waiting for review.").strong().size(15.0));
                        ui.label(format!("Once it is approved (usually within a day) everyone can open it at gcarve.app{url}, pick their own machine and bits, and cut it."));
                        ui.label(RichText::new("You can remove it there at any time while signed in.").color(crate::theme::TEXT_DIM));
                        if let Some(0) = crate::widgets::buttons(ui, &[("Close", true)]) {
                            close = true;
                        }
                    }
                    Err(e) => {
                        ui.label(RichText::new(format!("Not shared: {e}")).color(crate::theme::ORANGE));
                        if let Some(0) = crate::widgets::buttons(ui, &[("Back", true)]) {
                            self.sent = None;
                        }
                    }
                }
                return;
            }
            ui.label(RichText::new("Share this project on gcarve.app/projects so others can open it, pick their machine and bits, and cut it. It is checked by us before it appears.").color(crate::theme::TEXT_DIM));
            ui.add_space(6.0);
            if !host.cloud.signed_in() {
                ui.label(RichText::new("Sign in first (Help › Account): shared projects carry your chosen name, never your email.").color(crate::theme::ORANGE));
            }
            for p in &self.problems {
                ui.label(RichText::new(format!("⚠ {p}")).color(crate::theme::ORANGE));
            }
            egui::Grid::new("share-form").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Title");
                ui.add(crate::widgets::field(&mut self.title).hint_text("e.g. Family name sign").desired_width(420.0).char_limit(80));
                ui.end_row();
                ui.label("About it");
                ui.add(egui::TextEdit::multiline(&mut self.summary).hint_text("What it is, the wood you used, tips for cutting it…").desired_rows(4).desired_width(420.0).char_limit(2000));
                ui.end_row();
                ui.label("Tags");
                ui.add(crate::widgets::field(&mut self.tags).hint_text("sign, v-carve, gift (comma between)").desired_width(420.0));
                ui.end_row();
                ui.label("Shown as");
                ui.add(crate::widgets::field(&mut self.author).hint_text("your name or shop (optional)").desired_width(260.0).char_limit(40));
                ui.end_row();
                ui.label("License");
                ui.vertical(|ui| {
                    for (i, (_, label, tip)) in LICENSES.iter().enumerate() {
                        ui.radio_value(&mut self.license, i, *label).on_hover_text(*tip);
                    }
                });
                ui.end_row();
                ui.label("Pictures");
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        if let Some((rgba, w, h)) = &self.cover_rgba {
                            let tex = self.cover_tex.get_or_insert_with(|| ctx.load_texture("share-cover", egui::ColorImage::from_rgba_unmultiplied([*w, *h], rgba), egui::TextureOptions::LINEAR));
                            let k = 120.0 / (*w).max(*h) as f32;
                            ui.add(egui::Image::new((tex.id(), egui::vec2(*w as f32 * k, *h as f32 * k))));
                        }
                        ui.vertical(|ui| {
                            ui.label(RichText::new("The simulated cut is always included.").small().color(crate::theme::TEXT_DIM));
                            let mut gone = None;
                            for (i, (name, _)) in self.photos.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(name).small());
                                    if ui.small_button("✕").on_hover_text("Remove this photo").clicked() {
                                        gone = Some(i);
                                    }
                                });
                            }
                            if let Some(i) = gone {
                                self.photos.remove(i);
                            }
                            if ui.add_enabled(self.photos.len() < MAX_PHOTOS, egui::Button::new("Add photos of the real thing…")).clicked() {
                                self.want_photo = true;
                            }
                        });
                    });
                });
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.checkbox(&mut self.rights, "I made this design, or I have the right to share it under this license");
            let ready = host.cloud.signed_in() && self.problems.is_empty() && self.rights && self.title.trim().chars().count() >= 3 && self.cover.is_some() && !self.sending;
            match crate::widgets::buttons(ui, &[("Cancel", false), (if self.sending { "Sharing…" } else { "Share" }, ready)]) {
                Some(0) => close = true,
                Some(1) if ready => send = true,
                _ => {}
            }
        });
        if send {
            let body = self.body(&host.canvas.doc);
            host.cloud.community_share(ctx, body);
            self.sending = true;
        }
        open && !close
    }

    pub fn answered(&mut self, r: Result<String, String>) {
        self.sending = false;
        self.sent = Some(r);
    }

    /// what POST /api/community takes
    fn body(&self, doc: &Document) -> serde_json::Value {
        use gcarve_core::setupsheet::base64;
        let tools = used_tools(doc);
        let facts = serde_json::json!({
            "units": if units::name(doc) == "mm" { "mm" } else { "in" },
            "w": doc.material.w, "h": doc.material.h, "t": doc.material.t,
            "material": doc.material.kind,
            "sheets": doc.sheets.len().max(1),
            "bits": tools.iter().map(|&i| { let t = &doc.tools[i]; serde_json::json!({"name": t.name, "kind": kind_word(t.kind), "dia": t.diameter, "angle": if t.kind == ToolType::VBit { t.angle } else { 0.0 }}) }).collect::<Vec<_>>(),
            "ops": doc.ops.iter().map(|o| serde_json::json!({"name": o.name, "kind": super::op::kind_name(o.kind)})).collect::<Vec<_>>(),
            "minutes": (self.minutes * 10.0).round() / 10.0,
        });
        // the vectors of the active sheet, for people who use other software
        let items: Vec<(&gcarve_core::geom::Contour, &str)> = doc.shapes.iter().filter(|s| s.sheet == doc.active_sheet && doc.layers.get(s.layer as usize).is_none_or(|l| !l.keep_out)).map(|s| (&s.c, doc.layers.get(s.layer as usize).map_or("0", |l| l.name.as_str()))).collect();
        let photos: Vec<serde_json::Value> = self.cover.iter().chain(self.photos.iter().map(|p| &p.1)).map(|b| serde_json::json!({"mime": "image/jpeg", "data": base64(b)})).collect();
        let tags: Vec<String> = self.tags.split(',').map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).take(8).collect();
        serde_json::json!({
            "title": self.title.trim(), "summary": self.summary.trim(), "tags": tags, "license": LICENSES[self.license].0,
            "author": self.author.trim(), "facts": facts, "project": gcarve_core::project::save_project(doc),
            "svg": gcarve_core::svg::export_svg(&items, doc.material.w, doc.material.h), "dxf": gcarve_core::dxf::export_dxf(&items),
            "photos": photos, "rights": self.rights,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::document::Op;

    #[test]
    fn a_project_from_another_setup_lists_what_differs() {
        use gcarve_core::document::MachineLimits;
        let mine = MachineLimits { id: "camaster-panther".into(), name: "CAMaster Panther".into(), ..Default::default() };
        let v = Tool { name: "V-Bit (90 deg 0.5\")".into(), kind: ToolType::VBit, diameter: 0.5, angle: 90.0, ..Default::default() };
        let mut d = Document::default();
        d.tools = vec![v.clone(), Tool { name: "1/8 Ball".into(), kind: ToolType::BallNose, diameter: 0.125, ..Default::default() }];
        d.ops = vec![Op { tool: 0, ..Default::default() }, Op { tool: 1, ..Default::default() }];
        d.material.post = "grbl".into();
        let r = differences(&d, Some((&mine, "wincnc-single")), &[Tool { name: "my 90° V".into(), ..v.clone() }]);
        assert_eq!(r.len(), 2, "{r:?}");
        assert!(r[0].contains("no machine") && r[1].contains("1/8 Ball") && !r[1].contains("V-Bit"), "{r:?}");
        // the same machine, post and bits: nothing to ask
        d.material.machine = Some(mine.clone());
        d.material.post = "wincnc-single".into();
        d.ops.pop();
        assert!(differences(&d, Some((&mine, "wincnc-single")), &[v]).is_empty());
    }

    #[test]
    fn a_thinner_board_moves_the_through_cuts_only() {
        let mut d = Document::default();
        d.material.t = 0.75;
        d.sheets = vec![d.material.clone()];
        d.ops = vec![Op { depth: 0.8, ..Default::default() }, Op { depth: 0.2, ..Default::default() }, Op { depth: 0.75, ..Default::default() }];
        d.ops[2].bind.insert("depth".into(), "board-t".into());
        set_thickness(&mut d, 0.75, 0.5);
        assert_eq!((d.material.t, d.sheets[0].t), (0.5, 0.5));
        assert!((d.ops[0].depth - 0.55).abs() < 1e-9, "through cut keeps its margin below the board");
        assert_eq!(d.ops[1].depth, 0.2, "a pocket keeps its depth");
        assert_eq!(d.ops[2].depth, 0.75, "a bound depth follows its expression instead");
        let a = Tool { kind: ToolType::VBit, diameter: 0.5, angle: 90.0, ..Default::default() };
        assert!(same_bit(&a, &Tool { name: "other".into(), ..a.clone() }));
        assert!(!same_bit(&a, &Tool { angle: 60.0, ..a.clone() }));
    }
}

/// The starter projects GCarve publishes on gcarve.app/projects (made here so they stay correct as the engine changes):
/// `GCARVE_SEED_OUT=dir cargo test -p gcarve-app community_seeds -- --ignored` writes one POST /api/community body
/// per project; the server's `gcarve-api community-seed <email> <file>` publishes it.
#[cfg(test)]
mod seeds {
    use super::*;
    use crate::canvas::Canvas;
    use gcarve_core::document::{Op, OpType, Side, TextItem};
    use gcarve_core::geom::{make_ellipse, make_rect, pt, Contour};
    use gcarve_core::vecops::{fillet_contours, make_star};

    fn tools() -> Vec<Tool> {
        let t = |name: &str, kind, diameter, angle, stepdown, feed_xy, feed_z| Tool { name: name.into(), kind, diameter, angle, stepdown, feed_xy, feed_z, rpm: 18000, ..Default::default() };
        vec![
            t("1/4 End Mill (0.25 inch)", ToolType::EndMill, 0.25, 90.0, 0.125, 100.0, 30.0),
            t("90° V-Bit (0.5 inch)", ToolType::VBit, 0.5, 90.0, 0.1, 80.0, 30.0),
            t("3/8 Ball Nose (0.375 inch)", ToolType::BallNose, 0.375, 90.0, 0.1, 80.0, 30.0),
        ]
    }
    const END_MILL: i32 = 0;
    const V90: i32 = 1;
    const BALL: i32 = 2;

    fn board(c: &mut Canvas, w: f64, h: f64, t: f64, kind: &str) {
        c.doc.tools = tools();
        let m = &mut c.doc.material;
        (m.w, m.h, m.t, m.kind) = (w, h, t, kind.into());
        c.doc.sheets = vec![c.doc.material.clone()];
    }

    fn rounded(x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> Contour {
        fillet_contours(&[make_rect(x0, y0, x1, y1)], r, true, true).remove(0)
    }

    fn op(kind: OpType, name: &str, tool: i32, ids: Vec<i32>, depth: f64) -> Op {
        Op { kind, name: name.into(), tool, shape_ids: ids, depth, tab_corners: true, ..Default::default() }
    }

    fn cut_out(ids: Vec<i32>, depth: f64, tabs: i32) -> Op {
        Op { side: Side::Outside, tabs: true, tab_count: tabs, ..op(OpType::Profile, "Cut out", END_MILL, ids, depth) }
    }

    fn text(c: &mut Canvas, s: &str, h: f64, at: (f64, f64)) -> Vec<i32> {
        let n = c.place_text(TextItem { text: s.into(), family: "Liberation Sans".into(), height: h, bold: true, align: 1, ..Default::default() }, pt(at.0, at.1), None);
        assert!(n > 0);
        let id = c.doc.texts.last().unwrap().id;
        c.doc.shapes.iter().filter(|s| s.text == id).map(|s| s.id).collect()
    }

    /// a sign: carved text, a V-groove border, cut out with rounded corners
    fn sign(words: &str, w: f64, h: f64, cap: f64, wood: &str) -> Canvas {
        let mut c = Canvas::default();
        board(&mut c, w, h, 0.75, wood);
        let letters = text(&mut c, words, cap, (w / 2.0, h / 2.0));
        let border = c.doc.add(rounded(0.85, 0.85, w - 0.85, h - 0.85, 0.5), None);
        let outline = c.doc.add(rounded(0.3, 0.3, w - 0.3, h - 0.3, 0.8), None);
        c.doc.ops = vec![
            Op { flat_depth: 0.2, ..op(OpType::VCarve, "Letters", V90, letters, 0.2) },
            Op { side: Side::On, ..op(OpType::Profile, "Border groove", V90, vec![border], 0.1) },
            cut_out(vec![outline], 0.75, 4),
        ];
        c
    }

    fn coasters() -> Canvas {
        let mut c = Canvas::default();
        board(&mut c, 10.0, 10.0, 0.5, "Maple");
        let (mut rings, mut stars) = (Vec::new(), Vec::new());
        for (x, y) in [(2.5, 2.5), (7.5, 2.5), (2.5, 7.5), (7.5, 7.5)] {
            rings.push(c.doc.add(make_ellipse(x, y, 2.0, 2.0), None));
            stars.push(c.doc.add(make_star(pt(x, y), 1.3, 0.5, 5, std::f64::consts::FRAC_PI_2), None));
        }
        c.doc.ops = vec![Op { flat_depth: 0.15, ..op(OpType::VCarve, "Stars", V90, stars, 0.15) }, cut_out(rings, 0.5, 3)];
        c
    }

    fn cutting_board() -> Canvas {
        let mut c = Canvas::default();
        board(&mut c, 16.0, 11.0, 0.75, "Maple");
        let outline = c.doc.add(rounded(0.4, 0.4, 15.6, 10.6, 1.0), None);
        let groove = c.doc.add(rounded(1.25, 1.25, 14.75, 9.75, 0.6), None);
        c.doc.ops = vec![Op { side: Side::On, ..op(OpType::Profile, "Juice groove", BALL, vec![groove], 0.2) }, cut_out(vec![outline], 0.75, 4)];
        c
    }

    fn share(c: &Canvas, title: &str, summary: &str, tags: &str) -> serde_json::Value {
        let doc = &c.doc;
        let gen = pipeline::regenerate(doc);
        assert!(gen.warnings.is_empty(), "{title}: {:?}", gen.warnings);
        assert!(pipeline::order_issues(doc).is_empty(), "{title}: {:?}", pipeline::order_issues(doc));
        let mut d = ShareDialog::new(doc, title);
        assert!(d.problems.is_empty() && d.cover.is_some(), "{title}: {:?}", d.problems);
        (d.title, d.summary, d.tags, d.author, d.license, d.rights) = (title.into(), summary.into(), tags.into(), "GCarve".into(), 2, true);
        d.body(doc)
    }

    #[test]
    #[ignore]
    fn community_seeds() {
        let Ok(out) = std::env::var("GCARVE_SEED_OUT") else { return };
        let out = std::path::PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        let showcase = |file: &str| {
            let mut c = Canvas::default();
            let text = if file == "showcase.tlp" { String::from_utf8(crate::app::demo_project()).unwrap() } else { std::fs::read_to_string(format!("{}/../../samples/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap() };
            c.doc = gcarve_core::project::load_project(&text).unwrap();
            c.doc.ops.sort_by_key(|o| o.name == "Cut out"); // the part is cut free last
            c
        };
        let list = [
            (sign("WELCOME", 24.0, 8.0, 2.4, "Pine"), "Welcome sign (24 × 8 in)", "A classic porch sign: bold V-carved letters, a V-groove border and rounded corners. Cut from a 3/4 in pine or cedar board. Stain or paint the letters before the final sanding for crisp edges.", "sign, v-carve, outdoor, beginner"),
            (sign("742", 14.0, 7.0, 3.6, "Cedar"), "House number sign", "Large V-carved house numbers with a groove border. Change the text to your own number in GCarve (double-click the numbers), then make it. Cedar or another outdoor wood; seal it well.", "sign, v-carve, house number, outdoor, beginner"),
            (coasters(), "Star coasters (set of 4)", "Four 4 in round coasters with a V-carved star, cut out with tabs from one 10 × 10 in, 1/2 in thick board. A quick first project and a good gift; finish with food-safe oil.", "coaster, v-carve, gift, beginner, kitchen"),
            (cutting_board(), "Cutting board with juice groove", "A 15 × 10 in cutting board with a ball-nose juice groove and rounded corners. Use a glued-up hardwood blank (maple, walnut, cherry) and finish with mineral oil.", "kitchen, cutting board, ball nose, gift"),
            (showcase("showcase.tlp"), "Sampler board: every toolpath type", "One board that shows what each toolpath does: profile, pocket, drill, V-carve lettering, bore, flute and more. Handy for testing a new machine or bit set before a real job.", "sampler, test, v-carve, pocket, beginner"),
            (showcase("cabinet.tlp"), "Cabinet doors with hinge cups (MDF)", "Two cabinet doors from a 60 × 32 in MDF sheet: recessed panels, a bevelled edge, 35 mm hinge cups and handle holes, cut out last. Resize the doors in GCarve before you make them.", "cabinet, furniture, mdf, pocket, drill"),
        ];
        for (i, (c, title, summary, tags)) in list.iter().enumerate() {
            let body = share(c, title, summary, tags);
            std::fs::write(out.join(format!("{:02}.json", i + 1)), body.to_string()).unwrap();
            println!("{title}: {} KB", body.to_string().len() / 1024);
        }
    }
}
