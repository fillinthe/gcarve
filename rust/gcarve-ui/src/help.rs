//! Help ▸ Report a Bug / Request a Change / Check for Updates, shared by GCarve and GSketch. The app calls `init` once
//! (its GitHub repo for releases, its version); updates are only announced (status + menu), installing is a download.

use crate::cloud::Cloud;
use crate::{theme, widgets};
use eframe::egui::{self, RichText};
use std::sync::{Mutex, OnceLock};

static INFO: OnceLock<(&'static str, &'static str)> = OnceLock::new();
/// (tag without "v", release page) of the newest release; None until the answer lands
static NEWEST: Mutex<Option<(String, String)>> = Mutex::new(None);
static FAILED: Mutex<bool> = Mutex::new(false);

/// `repo` = "owner/name" on GitHub that holds the public releases; `version` = the running program's version.
pub fn init(repo: &'static str, version: &'static str) {
    let _ = INFO.set((repo, version));
}

fn parse(v: &str) -> Vec<u32> {
    v.trim().trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

fn version() -> &'static str {
    INFO.get().map_or("0", |i| i.1)
}

/// The newest version's number and page when it is newer than this program.
pub fn available() -> Option<(String, String)> {
    NEWEST.lock().ok()?.clone().filter(|(v, _)| parse(v) > parse(version()))
}

/// Ask GitHub for the newest release (desktop only); repaints when the answer lands. Cheap to call again ("Check again").
pub fn check(ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some((repo, _)) = INFO.get() {
        let ctx = ctx.clone();
        let mut req = ehttp::Request::get(format!("https://api.github.com/repos/{repo}/releases/latest"));
        req.headers.insert("User-Agent", "GCarve");
        ehttp::fetch(req, move |r| {
            let j = r.ok().filter(|r| r.ok).and_then(|r| serde_json::from_slice::<serde_json::Value>(&r.bytes).ok());
            let tag = j.as_ref().and_then(|j| j["tag_name"].as_str()).filter(|t| t.len() <= 20);
            if let (Some(t), Some(u)) = (tag, j.as_ref().and_then(|j| j["html_url"].as_str()).filter(|u| u.starts_with("https://github.com/"))) {
                *NEWEST.lock().unwrap() = Some((t.trim_start_matches('v').to_string(), u.to_string()));
            } else {
                *FAILED.lock().unwrap() = true;
            }
            ctx.request_repaint();
        });
    }
    #[cfg(target_arch = "wasm32")]
    let _ = ctx;
}

/// The dialogs behind the three Help items.
#[derive(Default)]
pub struct Help {
    report: Option<Report>,
    pub update_open: bool,
    told: bool,
}

struct Report {
    title: &'static str,
    kind: &'static str,
    subject: String,
    body: String,
    email: String,
    sending: bool,
    result: Option<Result<String, String>>,
}

impl Help {
    /// kind: "bug" | "suggestion" | "question"
    pub fn open_report(&mut self, cloud: &Cloud, kind: &'static str) {
        let title = if kind == "bug" { "Report a bug" } else { "Request a change" };
        self.report = Some(Report { title, kind, subject: String::new(), body: String::new(), email: cloud.user.as_ref().map(|u| u.email.clone()).unwrap_or_default(), sending: false, result: None });
    }

    pub fn open_update(&mut self, ctx: &egui::Context) {
        self.update_open = true;
        *FAILED.lock().unwrap() = false;
        check(ctx);
    }

    /// Reply::RequestSent
    pub fn on_reply(&mut self, r: Result<i64, String>) {
        if let Some(d) = &mut self.report {
            d.sending = false;
            d.result = Some(r.map(|id| format!("Sent (#{id}). Thanks! The answer comes by email and on your account page.")));
            if matches!(d.result, Some(Ok(_))) {
                d.subject.clear();
                d.body.clear();
            }
        }
    }

    /// Call once per frame. `quiet` = no start-up check (screenshots, local testing); returns a one-line notice the first
    /// time a newer version is known.
    pub fn ui(&mut self, ctx: &egui::Context, cloud: &mut Cloud, quiet: bool) -> Option<String> {
        static STARTED: OnceLock<()> = OnceLock::new();
        if !quiet && !Cloud::web() && STARTED.set(()).is_ok() {
            check(ctx);
        }
        self.report_ui(ctx, cloud);
        self.update_ui(ctx);
        if !self.told && !quiet {
            if let Some((v, _)) = available() {
                self.told = true;
                return Some(format!("{} {v} is available: Help › Check for Updates", crate::store::app()));
            }
        }
        None
    }

    fn report_ui(&mut self, ctx: &egui::Context, cloud: &mut Cloud) {
        let Some(d) = &mut self.report else { return };
        let mut open = true;
        let signed_in = cloud.user.is_some();
        let app = crate::store::app();
        widgets::Dialog::new(d.title).id("report").open(&mut open).close_footer().width(460.0).show(ctx, |ui| {
            ui.add(egui::Label::new(RichText::new(format!("Report a problem, ask for a feature or ask a question. It goes straight to the people building {app}. The version and system are added for you.")).small().color(theme::TEXT_DIM)).wrap());
            egui::Grid::new("req").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Type");
                let items = [("bug", "Problem / bug"), ("suggestion", "Feature or change"), ("question", "Question")];
                widgets::combo_box("req-kind").selected_text(items.iter().find(|i| i.0 == d.kind).map_or("", |i| i.1)).width(220.0).show_ui(ui, |ui| {
                    for (v, l) in items {
                        ui.selectable_value(&mut d.kind, v, l);
                    }
                });
                ui.end_row();
                ui.label("Summary");
                ui.add(widgets::field(&mut d.subject).desired_width(330.0).hint_text("e.g. Export stops at 50%"));
                ui.end_row();
                ui.label("Details");
                ui.add(egui::TextEdit::multiline(&mut d.body).desired_width(330.0).desired_rows(6).hint_text("What happened, and what did you expect? Steps to reproduce help a lot."));
                ui.end_row();
                if !signed_in {
                    ui.label("Your email");
                    ui.add(widgets::field(&mut d.email).desired_width(330.0).hint_text("so we can answer"));
                    ui.end_row();
                }
            });
            let ok = d.body.trim().len() >= 5 && (signed_in || d.email.contains('@'));
            ui.horizontal(|ui| {
                if ui.add_enabled(ok && !d.sending, egui::Button::new(if d.sending { "Sending…" } else { "Send" })).clicked() {
                    d.sending = true;
                    d.result = None;
                    let sys = format!("\n\n— {app} {} · {} {}{}", version(), std::env::consts::OS, std::env::consts::ARCH, if Cloud::web() { " · web" } else { "" });
                    cloud.send_request(ctx, d.kind, d.subject.trim(), &format!("{}{sys}", d.body.trim()), d.email.trim());
                }
                if signed_in && ui.link("Your requests and answers").clicked() {
                    cloud.open_web(ctx, "/dashboard/");
                }
            });
            match &d.result {
                Some(Ok(t)) => {
                    ui.label(RichText::new(t).color(theme::MODE_SIMULATE));
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(theme::DANGER));
                }
                None => {}
            }
        });
        if !open {
            self.report = None;
        }
    }

    fn update_ui(&mut self, ctx: &egui::Context) {
        if !self.update_open {
            return;
        }
        let app = crate::store::app();
        let newest = NEWEST.lock().ok().and_then(|n| n.clone());
        let failed = FAILED.lock().map(|f| *f).unwrap_or(false);
        widgets::Dialog::new("Check for updates").id("update").open(&mut self.update_open).close_footer().width(380.0).show(ctx, |ui| {
            ui.label(format!("You have {app} {}.", version()));
            match (&newest, available()) {
                (_, Some((v, url))) => {
                    ui.label(RichText::new(format!("{app} {v} is available.")).color(theme::ACCENT));
                    ui.label(RichText::new("Download it from the release page and install it over this one. Your projects and settings stay.").small().color(theme::TEXT_DIM));
                    if ui.add(widgets::Button::new(format!("Download {v}")).primary(true)).clicked() {
                        ctx.open_url(egui::OpenUrl::new_tab(url));
                    }
                }
                (Some(_), None) => {
                    ui.label(RichText::new("You have the newest version.").color(theme::MODE_SIMULATE));
                }
                (None, _) if failed => {
                    ui.label(RichText::new("Could not reach the update server. Check your internet connection and try again.").color(theme::DANGER));
                }
                _ => {
                    ui.label("Checking…");
                }
            }
            if ui.button("Check again").clicked() {
                *FAILED.lock().unwrap() = false;
                *NEWEST.lock().unwrap() = None;
                check(ctx);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions_compare_by_number() {
        assert!(super::parse("v0.10.0") > super::parse("0.9.3"));
        assert!(super::parse("0.1.1") > super::parse("0.1.0"));
        assert!(super::parse("0.1.0") <= super::parse("0.1.0"));
    }
}
