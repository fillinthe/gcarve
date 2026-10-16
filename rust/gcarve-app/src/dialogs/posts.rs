//! Post-processor chooser (Qt PostDialog.qml): search the built-ins and user posts, upload .pp / .json, export as
//! JSON, delete user posts; the pick goes to `Material::post` on the active sheet.

use super::{hint, Host};
use crate::widgets::TipExt;
use crate::posts::matches;
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
        let shown: Vec<_> = all.iter().filter(|(p, _)| matches(p, &self.search)).collect();
        crate::widgets::Dialog::new("Post-Processors").open(&mut open).close_footer().width(780.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let r = ui.add(crate::widgets::field(&mut self.search).desired_width(662.0).hint_text("Search by machine, controller, units or extension (e.g. Shapeoko, Mach4, mm, .sbp)"));
                crate::widgets::focus_first(ui, &r);
                if ui.button("Upload post…").tip("Add a Vectric .pp or a GCarve .json post").clicked() {
                    upload = true;
                }
            });
            hint(ui, &format!("Selected for this board: {current}"));
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
                        if crate::icons::IconButton::new("export").size(15.0).padding(4.0).tip("Export as editable JSON").show(ui).clicked() {
                            export = Some(p.id.clone());
                        }
                        if *custom && crate::icons::IconButton::new("delete").size(15.0).padding(4.0).tip("Delete this custom post").show(ui).clicked() {
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
            *host.message = format!("Deleted post {id}");
        }
        if upload {
            host.files.pick_into(ctx, "Post-processors (.pp, .json)", &["pp", "json"], "post");
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
            *host.message = format!("Added post {id} and selected it for this board");
        }
        Err(e) => {
            host.cloud.report("post", &crate::files::scrub(&e, name));
            *host.message = format!("{name}: {e}");
        }
    }
}
