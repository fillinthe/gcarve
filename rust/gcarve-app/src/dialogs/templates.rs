//! Toolpath templates (Qt TemplatesDialog.qml): save the current toolpaths as a template, apply one to this document
//! (summary lists what was skipped), rename / delete / import / export. Library: crate::templates.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::{hint, Host};
use crate::icons::IconButton;
use crate::widgets::TipExt;
use eframe::egui::{self, RichText};
use gcarve_core::template::{apply_template, make_template, save_template, summary_text};

#[derive(Default)]
pub struct TemplatesDialog {
    name: String,
    description: String,
    /// last apply summary, or an error
    summary: String,
    failed: bool,
    renaming: Option<(usize, String)>,
}

impl TemplatesDialog {
    pub fn new(name: &str) -> Self {
        TemplatesDialog { name: name.into(), ..Default::default() }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut open = true;
        let (mut apply, mut export, mut delete, mut rename, mut save, mut import) = (None, None, None, None, false, false);
        let has_ops = !host.canvas.doc.ops.is_empty();
        crate::widgets::Dialog::new(i18n::t("Toolpath templates")).open(&mut open).close_footer().width(600.0).show(ctx, |ui| {
            ui.label(RichText::new(i18n::t("Save the current toolpaths as a template")).family(crate::widgets::bold_family()));
            ui.horizontal(|ui| {
                let a = ui.add(crate::widgets::field(&mut self.name).desired_width(220.0).hint_text(i18n::t("Template name")));
                let b = ui.add(crate::widgets::field(&mut self.description).desired_width(290.0).hint_text(i18n::t("Description (optional)")));
                let enter = (a.lost_focus() || b.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let can = has_ops && !self.name.trim().is_empty();
                save = ui.add_enabled(can, egui::Button::new(i18n::t("Save"))).tip(i18n::t("Toolpaths, their bits and the variables they use; each toolpath remembers its layer")).clicked() || (enter && can);
            });
            if !self.summary.is_empty() {
                let text = RichText::new(&self.summary).size(12.0);
                ui.label(if self.failed { text.color(crate::theme::DANGER) } else { text.monospace() });
            }
            crate::theme::hline(ui);
            if host.templates.items.is_empty() {
                hint(ui, i18n::t("No templates yet. Templates keep toolpaths, their bits and variables; applied to another design, each toolpath uses the shapes on the layer of the same name."));
            }
            egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                ui.set_width(600.0);
                for (i, e) in host.templates.items.iter().enumerate() {
                    let r = crate::widgets::list_row(ui, false, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(420.0);
                                match &mut self.renaming {
                                    Some((k, text)) if *k == i => {
                                        let f = ui.add(crate::widgets::field(text).desired_width(300.0));
                                        f.request_focus();
                                        if f.lost_focus() {
                                            rename = Some((i, text.clone()));
                                        }
                                    }
                                    _ => {
                                        ui.label(RichText::new(&e.t.name).family(crate::widgets::bold_family()));
                                    }
                                }
                                let n = e.t.ops.len();
                                let desc = if e.t.description.is_empty() { String::new() } else { format!("  ·  {}", e.t.description) };
                                ui.label(RichText::new(format!("{n} toolpath{}{desc}", if n == 1 { "" } else { "s" })).small().color(crate::theme::TEXT_DIM));
                            });
                            if ui.button(i18n::t("Apply")).clicked() {
                                apply = Some(i);
                            }
                            if IconButton::new("export").size(15.0).padding(4.0).tip(i18n::t("Export to a .gctemplate file")).show(ui).clicked() {
                                export = Some(i);
                            }
                            if IconButton::new("edit").size(15.0).padding(4.0).tip(i18n::t("Rename")).show(ui).clicked() {
                                self.renaming = Some((i, e.t.name.clone()));
                            }
                            if IconButton::new("delete").size(15.0).padding(4.0).tip(i18n::t("Delete")).show(ui).clicked() {
                                delete = Some(i);
                            }
                        });
                    });
                    if r.double_clicked() {
                        apply = Some(i);
                    }
                }
            });
            ui.horizontal(|ui| {
                import = ui.button(i18n::t("Import…")).tip(i18n::t("Add a .gctemplate file to the library")).clicked();
                hint(ui, if host.templates.account { i18n::t("Stored in your GCarve account.") } else if cfg!(target_arch = "wasm32") { i18n::t("Stored in this browser (sign in to keep them in your account).") } else { i18n::t("Stored in the GCarve settings folder.") });
            });
        });
        if save {
            let t = make_template(&host.canvas.doc, self.name.trim(), self.description.trim());
            let n = t.ops.len();
            match host.templates.add(t) {
                Ok(()) => (self.summary, self.failed) = (format!("Saved \"{}\" ({n} toolpaths)", self.name.trim()), false),
                Err(e) => (self.summary, self.failed) = (e, true),
            }
            self.description.clear();
        }
        if let Some(i) = apply {
            let t = host.templates.items[i].t.clone();
            let c = &mut *host.canvas;
            c.doc.checkpoint();
            let r = apply_template(&t, &mut c.doc);
            c.apply_bindings(); // the template's expressions against this board and these variables
            c.mutated();
            *host.message = format!("Template \"{}\": {} toolpaths added, {} skipped", t.name, r.applied, r.skipped.len());
            (self.summary, self.failed) = (summary_text(&r), false);
        }
        if let Some(i) = export {
            let t = &host.templates.items[i].t;
            crate::files::save(&format!("{}.gctemplate", t.name), save_template(t).into_bytes(), host.files.msg_tx.clone());
        }
        if let Some((i, name)) = rename {
            self.renaming = None;
            if !name.trim().is_empty() {
                host.templates.rename(i, &name);
            }
        }
        if let Some(i) = delete {
            host.templates.remove(i);
        }
        if import {
            host.files.pick_into(ctx, i18n::t("Toolpath templates (.gctemplate)"), &["gctemplate"], "template");
        }
        open
    }
}

pub fn import_file(name: &str, bytes: &[u8], host: &mut Host) {
    *host.message = match host.templates.import(&String::from_utf8_lossy(bytes)) {
        Ok(t) => format!("Imported template \"{t}\""),
        Err(e) => {
            host.cloud.report("template", &crate::files::scrub(&e, name));
            format!("{name}: {e}")
        }
    };
}
