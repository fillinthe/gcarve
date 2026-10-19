//! File ▸ Projects (cloud): folders, open, save here, new folder, rename / move / delete. Folders are a path on
//! each project (server `folder`); folders made here that hold nothing yet are remembered on this device per account.

use crate::cloud::{Cloud, ProjectInfo};
use crate::theme;
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct ProjectsDialog {
    pub open: bool,
    /// current folder, "" = top
    pub folder: String,
    /// "Save here" name
    pub name: String,
    /// folders made here that hold nothing yet (local store, per account)
    made: Vec<String>,
    made_for: String,
    new_folder: Option<String>,
    rename: Option<(String, String)>,
    confirm_delete: Option<String>,
    /// clicked project (Open button)
    picked: Option<String>,
}

pub enum Action {
    Open(String),
    /// name, folder
    SaveNew(String, String),
    /// overwrite this project with the current document
    Replace(String),
    FromComputer,
    Download,
}

fn parent(f: &str) -> &str {
    f.rsplit_once('/').map_or("", |(p, _)| p)
}

fn join(a: &str, b: &str) -> String {
    if a.is_empty() { b.to_string() } else { format!("{a}/{b}") }
}

/// Every folder: from the projects (and their parents) plus the ones just made
fn all_folders(projects: &[ProjectInfo], made: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in projects.iter().map(|p| p.folder.as_str()).chain(made.iter().map(String::as_str)) {
        let mut f = f;
        while !f.is_empty() {
            if !out.iter().any(|x| x == f) {
                out.push(f.to_string());
            }
            f = parent(f);
        }
    }
    out.sort_by_key(|f| f.to_lowercase());
    out
}

impl ProjectsDialog {
    pub fn show(&mut self, name: &str) {
        self.open = true;
        self.name = name.to_string();
    }

    /// `current`: cloud id of the open document
    pub fn ui(&mut self, ctx: &egui::Context, cloud: &mut Cloud, current: &str) -> Option<Action> {
        if !self.open {
            return None;
        }
        let mut act = None;
        let mut open = true;
        let key = format!("cloud-folders-{}.json", cloud.user.as_ref().map_or("", |u| u.email.as_str()));
        if self.made_for != key {
            self.made = crate::store::load(&key).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            self.made_for = key.clone();
        }
        let made_before = self.made.clone();
        let folders = all_folders(&cloud.projects, &self.made);
        if !self.folder.is_empty() && !folders.contains(&self.folder) {
            self.folder.clear(); // its last project moved away
        }
        crate::widgets::Dialog::new("Open or save").open(&mut open).close_footer().width(620.0).show(ctx, |ui| {
            // a file from this computer first: people coming from VCarve have their .crv files there
            egui::Frame::new().fill(theme::TEXT.gamma_multiply(0.05)).corner_radius(8.0).inner_margin(12.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if ui.add(crate::widgets::Button::new("Open a file from this computer…").primary(true).min_width(250.0)).clicked() {
                        act = Some(Action::FromComputer);
                    }
                    ui.label(RichText::new("or drop it anywhere on the window").color(theme::TEXT_DIM));
                });
                ui.add_space(4.0);
                ui.label(RichText::new("GCarve projects (.tlp), VCarve projects (.crv), SVG, DXF, DWG, PDF, Illustrator, pictures and G-code (.tap, .nc, .gcode)").small().color(theme::TEXT_DIM));
            });
            ui.add_space(8.0);
            // breadcrumb + new folder
            ui.horizontal(|ui| {
                let mut path = String::new();
                if ui.link(RichText::new("My online projects").strong()).clicked() {
                    self.folder.clear();
                }
                for part in self.folder.clone().split('/').filter(|p| !p.is_empty()) {
                    path = join(&path, part);
                    ui.label(RichText::new("›").color(theme::TEXT_DIM));
                    if ui.link(part).clicked() {
                        self.folder = path.clone();
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::icons::IconButton::new("add").text("New folder").tip("Make a folder here").show(ui).clicked() {
                        self.new_folder = Some(String::new());
                    }
                });
            });
            if let Some(mut n) = self.new_folder.take() {
                let mut keep = true;
                ui.horizontal(|ui| {
                    let r = ui.add(crate::widgets::field(&mut n).hint_text("Folder name").desired_width(260.0));
                    r.request_focus();
                    let ok = !n.trim().is_empty() && !n.contains('/');
                    if ok && (ui.button("Create").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))) {
                        let f = join(&self.folder, n.trim());
                        self.made.push(f.clone());
                        self.folder = f;
                        keep = false;
                    } else if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        keep = false;
                    }
                });
                if keep {
                    self.new_folder = Some(n);
                }
            }
            if cloud.error == "sign in required" {
                // not a mistake of the visitor's: say what an account gives instead
                ui.label(RichText::new("Sign in (free) to keep projects online, in folders, on every computer.").color(theme::TEXT_DIM));
            } else if !cloud.error.is_empty() {
                ui.label(RichText::new(&cloud.error).color(theme::DANGER));
            }
            if let Some((used, max, n, max_n)) = cloud.storage.filter(|s| s.1 > 0) {
                // what the plan allows, so a full account is no surprise
                let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_048_576.0);
                ui.horizontal(|ui| {
                    ui.add(egui::ProgressBar::new((used as f32 / max as f32).clamp(0.0, 1.0)).desired_width(160.0).fill(if used * 10 >= max * 9 { theme::WARNING } else { theme::ACCENT }));
                    ui.label(RichText::new(format!("{} of {} · {n} of {max_n} projects", mb(used), mb(max))).small().color(theme::TEXT_DIM));
                });
            }
            ui.separator();
            egui::ScrollArea::vertical().max_height(crate::widgets::list_height(ui, 215.0)).show(ui, |ui| {
                ui.set_min_height(280.0);
                ui.set_width(ui.available_width());
                let subs: Vec<&String> = folders.iter().filter(|f| parent(f) == self.folder).collect();
                let here: Vec<ProjectInfo> = cloud.projects.iter().filter(|p| p.folder == self.folder).cloned().collect();
                if subs.is_empty() && here.is_empty() {
                    let r = ui.max_rect();
                    let msg = if cloud.busy > 0 { "Loading…" } else if self.folder.is_empty() { "No cloud projects yet" } else { "Empty folder: save a project here" };
                    ui.painter().text(egui::pos2(r.center().x, r.top() + 130.0), egui::Align2::CENTER_CENTER, msg, theme::font(theme::FONT_SIZE), theme::TEXT_DIM);
                }
                if !self.folder.is_empty() && row(ui, "open", "..", "Up one folder", theme::TEXT_DIM).clicked() {
                    self.folder = parent(&self.folder).to_string();
                }
                for f in subs {
                    let n = cloud.projects.iter().filter(|p| p.folder == *f || p.folder.starts_with(&format!("{f}/"))).count();
                    let leaf = f.rsplit('/').next().unwrap_or(f);
                    let resp = row(ui, "open", leaf, &format!("{n} project(s)"), theme::TEXT);
                    if resp.clicked() {
                        self.folder = f.clone();
                    }
                    if n == 0 {
                        resp.context_menu(|ui| {
                            if ui.button("Remove empty folder").clicked() {
                                self.made.retain(|m| m != f && !m.starts_with(&format!("{f}/")));
                                ui.close();
                            }
                        });
                    }
                }
                for p in here {
                    let when = chrono::DateTime::from_timestamp(p.updated, 0).map(|d| d.with_timezone(&chrono::Local).format("%b %-d, %H:%M").to_string()).unwrap_or_default();
                    let col = if p.id == current { theme::ACCENT } else { theme::TEXT };
                    if let Some((id, mut n)) = self.rename.take_if(|(id, _)| *id == p.id) {
                        let mut keep = true;
                        ui.horizontal(|ui| {
                            let r = ui.add(crate::widgets::field(&mut n).desired_width(300.0));
                            r.request_focus();
                            if ui.button("Rename").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                                if !n.trim().is_empty() {
                                    cloud.update_project(ctx, &id, Some(n.trim()), None);
                                }
                                keep = false;
                            } else if ui.button("Cancel").clicked() {
                                keep = false;
                            }
                        });
                        if keep {
                            self.rename = Some((id, n));
                        }
                        continue;
                    }
                    if self.picked.as_deref() == Some(&p.id) {
                        let r = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 30.0));
                        ui.painter().rect_filled(r, 4.0, theme::ACTIVE);
                    }
                    let resp = row(ui, "save", &p.name, &format!("{when} · {} KB", (p.size + 1023) / 1024), col);
                    if resp.double_clicked() {
                        act = Some(Action::Open(p.id.clone()));
                    } else if resp.clicked() {
                        self.name = p.name.clone();
                        self.picked = Some(p.id.clone());
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Open").clicked() {
                            act = Some(Action::Open(p.id.clone()));
                            ui.close();
                        }
                        if ui.button("Rename…").clicked() {
                            self.rename = Some((p.id.clone(), p.name.clone()));
                            ui.close();
                        }
                        ui.menu_button("Move to", |ui| {
                            for f in std::iter::once(String::new()).chain(folders.iter().cloned()) {
                                if f != p.folder && ui.button(if f.is_empty() { "My projects (top)".to_string() } else { f.replace('/', " › ") }).clicked() {
                                    cloud.update_project(ctx, &p.id, None, Some(&f));
                                    ui.close();
                                }
                            }
                        });
                        ui.separator();
                        if ui.button(RichText::new("Delete…").color(theme::DANGER)).clicked() {
                            self.confirm_delete = Some(p.id.clone());
                            ui.close();
                        }
                    });
                    if self.confirm_delete.as_deref() == Some(&p.id) {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("Delete “{}” from the cloud?", p.name)).color(theme::DANGER));
                            if ui.button("Delete").clicked() {
                                cloud.delete_project(ctx, &p.id);
                                cloud.refresh(ctx);
                                self.confirm_delete = None;
                            }
                            if ui.button("Keep").clicked() {
                                self.confirm_delete = None;
                            }
                        });
                    }
                }
            });
            ui.separator();
            // save the current document into this folder (same name here = replace it)
            ui.horizontal(|ui| {
                ui.label("Save as");
                ui.add(crate::widgets::field(&mut self.name).desired_width(ui.available_width() - 180.0));
                let pick = self.picked.clone().filter(|id| cloud.projects.iter().any(|p| p.id == *id && p.folder == self.folder));
                if ui.add_enabled(pick.is_some(), egui::Button::new("Open")).clicked() {
                    act = pick.map(Action::Open);
                }
                let name = self.name.trim().to_string();
                let same = cloud.projects.iter().find(|p| p.folder == self.folder && p.name.eq_ignore_ascii_case(&name));
                let label = if same.is_some() { "Replace" } else { "Save here" };
                if ui.add_enabled(!name.is_empty() && cloud.busy == 0, egui::Button::new(label)).clicked() {
                    act = Some(match same {
                        Some(p) => Action::Replace(p.id.clone()),
                        None => Action::SaveNew(name, self.folder.clone()),
                    });
                }
            });
            // hint on its own line: next to the two links it was cut off
            ui.label(RichText::new("Click to pick · double-click to open · right-click to rename, move or delete").small().color(theme::TEXT_DIM));
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.link("Download .tlp").on_hover_text("Save a copy to this computer").clicked() {
                        act = Some(Action::Download);
                    }
                });
            });
        });
        if !open {
            self.open = false;
        }
        if matches!(act, Some(Action::Open(_) | Action::FromComputer)) {
            self.open = false;
        }
        if let Some(Action::SaveNew(_, f)) = &act {
            self.made.retain(|m| m != f);
        }
        // a folder that now holds projects needs no remembering
        self.made.retain(|m| !cloud.projects.iter().any(|p| p.folder == *m || p.folder.starts_with(&format!("{m}/"))));
        if self.made != made_before {
            crate::store::save(&key, &serde_json::to_string(&self.made).unwrap_or_default());
        }
        act
    }
}

/// One list row: icon, name, dim detail; whole row clickable
fn row(ui: &mut egui::Ui, icon: &str, name: &str, detail: &str, col: egui::Color32) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, theme::ROW_HOVER);
    }
    crate::icons::paint(ui, icon, egui::Rect::from_min_size(rect.left_top() + egui::vec2(8.0, 7.0), egui::vec2(16.0, 16.0)), 16.0, theme::TEXT_DIM);
    ui.painter().text(egui::pos2(rect.left() + 32.0, rect.center().y), egui::Align2::LEFT_CENTER, name, theme::font(13.0), col);
    ui.painter().text(egui::pos2(rect.right() - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, detail, theme::font(11.0), theme::TEXT_DIM);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_include_parents_and_new_ones() {
        let p = |f: &str| ProjectInfo { folder: f.into(), ..Default::default() };
        assert_eq!(all_folders(&[p("Shop/Signs"), p(""), p("Kitchen")], &["Shop/Doors".into()]), ["Kitchen", "Shop", "Shop/Doors", "Shop/Signs"]);
    }
}
