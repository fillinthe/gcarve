//! Post-processor chooser (Qt PostDialog.qml): search the built-ins and user posts, upload .pp / .json, export as
//! JSON, delete user posts; the pick goes to `Material::post` on the active sheet.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::{hint, Host};
use crate::widgets::TipExt;
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct PostsDialog {
    search: String,
}

impl PostsDialog {
    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut open = true;
        let mut close = false;
        let mut select: Option<String> = None;
        let mut export: Option<String> = None;
        let mut delete: Option<String> = None;
        let mut upload = false;
        let all = host.posts.all();
        let current = host.canvas.doc.material.post.clone();
        // the best matches first; with no search, the post in use, then your own, then the built-ins A to Z
        let mut shown: Vec<_> = all.iter().filter_map(|e| crate::posts::score(&e.0, &self.search).map(|sc| (sc, e))).collect();
        shown.sort_by(|a, b| b.0.cmp(&a.0).then((b.1 .0.id == current).cmp(&(a.1 .0.id == current))).then(b.1 .1.cmp(&a.1 .1)).then(a.1 .0.name.to_lowercase().cmp(&b.1 .0.name.to_lowercase())));
        let shown: Vec<_> = shown.into_iter().map(|(_, e)| e).collect();
        crate::widgets::Dialog::new(i18n::t("Post-Processors")).open(&mut open).close_footer().width(780.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let r = ui.add(crate::widgets::field(&mut self.search).desired_width(600.0).hint_text(i18n::t("Search: machine, maker, controller, mm / inch, .ext (e.g. shark mm, fanuc atc, .sbp)")));
                crate::widgets::focus_first(ui, &r);
                if ui.button(i18n::t("Upload posts…")).tip(i18n::t("Add Vectric .pp or GCarve .json posts: pick one, or select a whole folder of them at once (VCarve keeps its own in File › Open Application Data Folder › PostP)")).clicked() {
                    upload = true;
                }
            });
            hint(ui, &i18n::tf("Selected for this board: {current}", &[("current", &current)]));
            egui::ScrollArea::vertical().max_height(crate::widgets::list_height(ui, 110.0)).show(ui, |ui| {
                ui.set_width(780.0);
                for (p, custom) in &shown {
                    let on = p.id == current;
                    let r = crate::widgets::list_row(ui, on, |ui| {
                        ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(640.0);
                            ui.label(RichText::new(&p.name).family(crate::widgets::bold_family()));
                            ui.label(RichText::new(format!("{}  ·  {}  ·  .{}  ·  {}", p.controller, p.machines, p.extension, if p.inches { "inches" } else { "mm" })).small().color(if on { egui::Color32::WHITE } else { crate::theme::TEXT_DIM }));
                        });
                        ui.label(RichText::new(if *custom { "custom" } else { "built-in" }).small().color(if on { egui::Color32::WHITE } else { crate::theme::TEXT_DIM }));
                        if crate::icons::IconButton::new("export").size(15.0).padding(4.0).tip(i18n::t("Export as editable JSON")).show(ui).clicked() {
                            export = Some(p.id.clone());
                        }
                        if *custom && crate::icons::IconButton::new("delete").size(15.0).padding(4.0).tip(i18n::t("Delete this custom post")).show(ui).clicked() {
                            delete = Some(p.id.clone());
                        }
                        });
                    });
                    if r.clicked() {
                        select = Some(p.id.clone());
                    }
                    if r.double_clicked() {
                        select = Some(p.id.clone());
                        close = true;
                    }
                }
            });
            hint(ui, &format!("{} of {} posts. Click to select, double-click to select and close. Exported JSON can be edited and uploaded again; Vectric .pp files are translated on upload (verify the first file on your machine).", shown.len(), all.len()));
        });
        if let Some(id) = select {
            if host.canvas.doc.material.post != id {
                host.canvas.doc.checkpoint();
                host.canvas.doc.material.post = id;
                host.canvas.doc.sync_sheet();
                host.canvas.touch();
            }
        }
        if let Some(id) = export {
            if let Some(json) = host.posts.export_json(&id) {
                crate::files::save(&format!("{id}.json"), json.into_bytes(), host.files.msg_tx.clone());
            }
        }
        if let Some(id) = delete {
            host.posts.remove(&id);
            *host.message = i18n::tf("Deleted post {id}", &[("id", &id)]);
        }
        if upload {
            host.files.pick_many_into(ctx, i18n::t("Post-processors (.pp, .json)"), &["pp", "json"], "post");
        }
        open && !close
    }
}

pub fn import_file(name: &str, bytes: &[u8], host: &mut Host) {
    match host.posts.add(name, &String::from_utf8_lossy(bytes)) {
        Ok(id) => {
            host.canvas.doc.checkpoint();
            host.canvas.doc.material.post = id.clone();
            host.canvas.doc.sync_sheet();
            host.canvas.touch();
            *host.message = i18n::tf("Added post {id} and selected it for this board", &[("id", &id)]);
        }
        Err(e) => {
            host.cloud.report("post", &crate::files::scrub(&e, name));
            *host.message = format!("{name}: {e}");
        }
    }
}
