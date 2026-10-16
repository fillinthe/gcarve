//! Jobs organizer (Qt JobsDialog.qml), desktop only: job folders under the jobs root, projects inside them,
//! G-code exports with revision history.

use super::{combo, Host};
use crate::jobs::{Jobs, STATUSES};
use eframe::egui::{self, Color32, RichText};
use std::path::PathBuf;

#[derive(Default)]
pub struct JobsDialog {
    current: usize,
    note: String,
    prompt: Option<(bool, String)>, // (new job?, name)
    history_open: Option<PathBuf>,
}

fn status_color(s: &str) -> Color32 {
    match s {
        "In progress" => crate::theme::MODE_TOOLPATHS,
        "Cut" => crate::theme::MODE_SIMULATE,
        "Delivered" => crate::theme::MODE_DESIGN,
        _ => crate::theme::TEXT_DIM,
    }
}

impl JobsDialog {
    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut open = true;
        enum Act {
            Create(String),
            Root,
            Status(PathBuf, String),
            Notes(PathBuf, String),
            ExportAll(PathBuf),
            NewProject(PathBuf, String),
            Add(PathBuf),
            Open(PathBuf),
            Export(PathBuf),
            Reveal(PathBuf),
        }
        let mut act: Option<Act> = None;
        let jobs: &Jobs = host.jobs;
        self.current = self.current.min(jobs.jobs.len().saturating_sub(1));
        let job = jobs.jobs.get(self.current).cloned();
        const H: f32 = 487.0;
        const LEFT: f32 = 351.0; // RowLayout gives the 250 px preferred list the spare width too // content height: 600 px with the title and the Close footer (Qt min(600, window − 40))
        use crate::icons::IconButton;
        use crate::theme;
        crate::widgets::Dialog::new("Jobs").open(&mut open).close_footer().flush().width(898.0).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                // ---- left: jobs
                ui.vertical(|ui| {
                    ui.set_width(LEFT);
                    let list_h = H - 45.0;
                    egui::ScrollArea::vertical().max_height(list_h).min_scrolled_height(list_h).auto_shrink([false, false]).id_salt("jobs").show(ui, |ui| {
                        if jobs.jobs.is_empty() {
                            ui.add_space(list_h / 2.0 - 70.0);
                            blank(ui, "jobs", "No jobs yet", "A job is a folder of projects that cut together. Create one, add projects, export: the G-code lands in the job with every revision kept.");
                        }
                        for (i, j) in jobs.jobs.iter().enumerate() {
                            let (r, resp) = ui.allocate_exact_size(egui::vec2(LEFT, 30.0), egui::Sense::click());
                            let on = i == self.current;
                            let p = ui.painter();
                            p.rect_filled(r, 0.0, if on { theme::ACTIVE } else if resp.hovered() { theme::ROW_HOVER } else { Color32::TRANSPARENT });
                            p.circle_filled(egui::pos2(r.left() + 14.0, r.center().y), 4.0, status_color(&j.status));
                            let font = if on { crate::widgets::bold(theme::FONT_SIZE) } else { theme::font(theme::FONT_SIZE) };
                            let pr = ui.painter_at(egui::Rect::from_min_max(r.min, egui::pos2(r.right() - 30.0, r.bottom())));
                            pr.text(egui::pos2(r.left() + 26.0, r.center().y), egui::Align2::LEFT_CENTER, &j.name, font, theme::TEXT);
                            p.text(egui::pos2(r.right() - 12.0, r.center().y), egui::Align2::RIGHT_CENTER, j.projects.len().to_string(), theme::font(11.0), theme::TEXT.gamma_multiply(0.5));
                            if resp.clicked() {
                                self.current = i;
                                self.history_open = None;
                            }
                        }
                    });
                    theme::hline(ui);
                    ui.horizontal(|ui| {
                        ui.set_height(44.0);
                        ui.add_space(8.0);
                        if IconButton::new("add").text("New Job").show(ui).clicked() {
                            self.prompt = Some((true, String::new()));
                        }
                        ui.add_space(ui.available_width() - 31.0);
                        if IconButton::new("open").size(15.0).tip(&format!("Choose the jobs folder (now: {})", jobs.root.display())).show(ui).clicked() {
                            act = Some(Act::Root);
                        }
                    });
                });
                theme::vsep(ui, H);
                // ---- right: the selected job
                ui.vertical(|ui| egui::Frame::new().inner_margin(14).show(ui, |ui| { // vertical: a Frame keeps the row's layout
                    let w = 898.0 - LEFT - 1.0 - 28.0;
                    ui.set_width(w);
                    ui.set_height(H - 28.0);
                    ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                    let Some(j) = &job else {
                        ui.add_space(H / 2.0 - 90.0);
                        blank(ui, "jobs", "Pick or create a job", &format!("Jobs live in {}", jobs.root.display()));
                        return;
                    };
                    ui.horizontal(|ui| {
                        ui.set_height(theme::CONTROL_HEIGHT);
                        ui.add(egui::Label::new(RichText::new(&j.name).font(crate::widgets::bold(theme::FONT_SIZE + 4.0)).color(theme::TEXT)).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled_ui(!j.projects.is_empty(), |ui| IconButton::new("export").text("Export All").primary(true).tip("Write G-code for every project into the job's gcode folder (saved files; earlier G-code moves to history)").show(ui)).inner.clicked() {
                                act = Some(Act::ExportAll(j.path.clone()));
                            }
                            if IconButton::new("open").tip(if cfg!(target_os = "macos") { "Show the job folder in Finder" } else { "Show the job folder in the file manager" }).show(ui).clicked() {
                                act = Some(Act::Reveal(j.path.clone()));
                            }
                            let items: Vec<(&str, &str)> = STATUSES.iter().map(|s| (*s, *s)).collect();
                            let mut cur = STATUSES.iter().find(|s| **s == j.status).copied().unwrap_or("Planned");
                            if combo(ui, "status", &mut cur, &items, 130.0) {
                                act = Some(Act::Status(j.path.clone(), cur.to_string()));
                            }
                        });
                    });
                    let mut notes = j.notes.clone();
                    if ui.add(crate::widgets::field(&mut notes).desired_width(w).hint_text("Notes: client, material, due date…")).lost_focus() && notes != j.notes {
                        act = Some(Act::Notes(j.path.clone(), notes));
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Projects").family(crate::widgets::bold_family()).color(theme::TEXT_DIM));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if IconButton::new("add").text("Add…").tip("Copy an existing project file into this job").show(ui).clicked() {
                                act = Some(Act::Add(j.path.clone()));
                            }
                            if IconButton::new("new").text("New").tip("New project saved into this job (opens in a tab)").show(ui).clicked() {
                                self.prompt = Some((false, String::new()));
                            }
                        });
                    });
                    let list_h = ui.available_height() - if self.note.is_empty() { 0.0 } else { 40.0 };
                    egui::ScrollArea::vertical().max_height(list_h).min_scrolled_height(list_h).auto_shrink([false, false]).id_salt("projects").show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if j.projects.is_empty() {
                            ui.add_space(list_h / 2.0 - 70.0);
                            blank(ui, "board", "No projects in this job", "Add an existing .tlp or start a new one. Export sends G-code to the job's gcode folder; the file name stays the same and older versions move to history.");
                        }
                        for p in &j.projects {
                            egui::Frame::new().fill(theme::BG).stroke(egui::Stroke::new(1.0, theme::BORDER)).corner_radius(theme::RADIUS).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                                ui.set_width(w - 22.0);
                                ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
                                ui.horizontal(|ui| {
                                    crate::icons::paint(ui, "board", egui::Rect::from_min_size(ui.cursor().min + egui::vec2(0.0, 2.0), egui::vec2(16.0, 16.0)), 16.0, theme::TOOL_FILE);
                                    ui.add_space(24.0);
                                    ui.label(RichText::new(&p.name).family(crate::widgets::bold_family()).color(theme::TEXT));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.spacing_mut().item_spacing.x = 4.0;
                                        if IconButton::new("export").size(15.0).padding(3.0).tip("Export this project's G-code into the job (from the saved file)").show(ui).clicked() {
                                            act = Some(Act::Export(p.path.clone()));
                                        }
                                        if IconButton::new("open").size(15.0).padding(3.0).tip("Open in a tab").show(ui).clicked() {
                                            act = Some(Act::Open(p.path.clone()));
                                        }
                                        ui.add_space(4.0);
                                        if p.revision > 0 {
                                            ui.label(RichText::new(format!("rev {}", p.revision)).size(11.0).family(crate::widgets::bold_family()).color(theme::MODE_SIMULATE));
                                        }
                                        if p.stale {
                                            ui.label(RichText::new("⚠ changed since export").size(11.0).color(theme::WARNING));
                                        }
                                    });
                                });
                                let line = format!("Saved {}{}", p.modified, if p.exported.is_empty() { "  ·  not exported yet".to_string() } else { format!("  ·  G-code {} ({} file{})", p.exported, p.files, if p.files == 1 { "" } else { "s" }) });
                                ui.add(egui::Label::new(RichText::new(line).size(11.0).color(theme::TEXT.gamma_multiply(0.7))).truncate());
                                if p.revision > 1 {
                                    let on = self.history_open.as_ref() == Some(&p.path);
                                    let r = ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        crate::icons::paint(ui, if on { "chevron-down" } else { "chevron-right" }, egui::Rect::from_min_size(ui.cursor().min + egui::vec2(0.0, 1.0), egui::vec2(11.0, 11.0)), 11.0, theme::ACCENT);
                                        ui.add_space(13.0);
                                        ui.label(RichText::new(format!("{} earlier revision{} in history", p.revision - 1, if p.revision > 2 { "s" } else { "" })).size(11.0).color(theme::ACCENT));
                                    });
                                    if ui.interact(r.response.rect, ui.id().with(("hist", &p.path)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                        self.history_open = if on { None } else { Some(p.path.clone()) };
                                    }
                                    if on {
                                        for r in jobs.revisions(&p.path) {
                                            ui.horizontal(|ui| {
                                                ui.add_space(14.0);
                                                ui.add_sized([28.0, 16.0], egui::Label::new(RichText::new(format!("r{}", r.rev)).size(11.0).family(crate::widgets::bold_family()).color(theme::TEXT.gamma_multiply(0.8))));
                                                ui.label(RichText::new(format!("{}  ·  {}", r.name, r.time)).size(11.0).color(theme::TEXT.gamma_multiply(0.7)));
                                                if IconButton::new("open").size(13.0).padding(2.0).tip("Show this revision in the file manager").show(ui).clicked() {
                                                    act = Some(Act::Reveal(r.path.clone()));
                                                }
                                            });
                                        }
                                    }
                                }
                            });
                        }
                    });
                    if !self.note.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&self.note).size(11.0).color(theme::TEXT_DIM)).wrap());
                    }
                }));
            });
        });
        // one small prompt serves new job / new project (Qt namePrompt)
        if let Some((is_job, text)) = &mut self.prompt {
            let (title, hint_text) = if *is_job { ("New job", "Job name (e.g. Smith sign)") } else { ("New project", "Project name") };
            let mut keep = true;
            let (ok, cancel) = crate::widgets::Dialog::new(title).id("jobs-prompt").open(&mut keep).show(ctx, |ui| {
                let r = ui.add(crate::widgets::field(text).desired_width(320.0).hint_text(hint_text));
                if ui.memory(|m| m.focused().is_none()) {
                    r.request_focus();
                }
                crate::widgets::ok_cancel(ui)
            });
            let name = text.trim().to_string();
            if ok && !name.is_empty() {
                act = Some(if *is_job { Act::Create(name) } else { Act::NewProject(job.as_ref().map(|j| j.path.clone()).unwrap_or_default(), name) });
            }
            if ok || cancel || !keep {
                self.prompt = None;
            }
        }
        let jobs: &mut Jobs = host.jobs;
        match act {
            Some(Act::Create(name)) => {
                if let Some(p) = jobs.create(&name) {
                    self.current = jobs.jobs.iter().position(|j| j.path == p).unwrap_or(0);
                }
                self.note = jobs.status.clone();
            }
            Some(Act::Root) => {
                if let Some(d) = rfd::FileDialog::new().set_title("Jobs folder").pick_folder() {
                    jobs.set_root(d);
                }
            }
            Some(Act::Status(p, s)) => jobs.set_meta(&p, "status", &s),
            Some(Act::Notes(p, s)) => jobs.set_meta(&p, "notes", &s),
            Some(Act::ExportAll(p)) => self.note = jobs.export_job(&p, host.posts),
            Some(Act::Export(p)) => self.note = jobs.export_project(&p, host.posts),
            Some(Act::Reveal(p)) => Jobs::reveal(&p),
            Some(Act::Add(job)) => {
                if let Some(f) = rfd::FileDialog::new().add_filter("GCarve project", &["tlp"]).pick_file() {
                    jobs.add_project(&job, &f);
                    self.note = jobs.status.clone();
                }
            }
            Some(Act::NewProject(job, name)) => {
                // Qt: a new tab saved straight into the job; here: an empty project written there, opened in a tab
                let path = job.join(format!("{name}.tlp"));
                let mut doc = gcarve_core::document::Document::default();
                doc.tools = crate::toollib::load();
                match std::fs::write(&path, gcarve_core::project::save_project(&doc)) {
                    Ok(()) => {
                        jobs.refresh();
                        host.files.open_path(&path);
                        return false;
                    }
                    Err(e) => self.note = format!("Cannot save {}: {e}", path.display()),
                }
            }
            Some(Act::Open(p)) => {
                host.files.open_path(&p);
                return false;
            }
            None => {}
        }
        open
    }
}

/// JobsDialog.qml `Blank`: dim icon, bold title, small dim detail, centred
fn blank(ui: &mut egui::Ui, icon: &str, title: &str, detail: &str) {
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
        crate::icons::paint(ui, icon, r, 28.0, crate::theme::TEXT_DISABLED);
        ui.label(RichText::new(title).family(crate::widgets::bold_family()).color(crate::theme::TEXT));
        ui.add(egui::Label::new(RichText::new(detail).size(11.0).color(crate::theme::TEXT.gamma_multiply(0.6))).wrap().halign(egui::Align::Center));
    });
}
