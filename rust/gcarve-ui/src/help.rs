//! Help ▸ Report a Bug / Request a Change / Check for Updates, shared by GCarve and GSketch. The app calls `init` once
//! (its GitHub repo for releases, its version); a newer release is announced (status + menu) and "Update now" downloads
//! this computer's file and opens it, falling back to the download page in the browser when that fails.

use crate::cloud::Cloud;
use crate::{theme, widgets};
use eframe::egui::{self, RichText};
use std::sync::{Mutex, OnceLock};

static INFO: OnceLock<(&'static str, &'static str)> = OnceLock::new();
/// (tag without "v", release page) of the newest release; None until the answer lands
static NEWEST: Mutex<Option<(String, String)>> = Mutex::new(None);
static FAILED: Mutex<bool> = Mutex::new(false);
/// direct download link of the newest release's file for this computer (see `asset_suffix`)
static ASSET: Mutex<Option<String>> = Mutex::new(None);
/// where "Update now" sends people when the download or opening fails (`set_download_page`; default the release page)
static PAGE: OnceLock<&'static str> = OnceLock::new();
static UPDATE: Mutex<Upd> = Mutex::new(Upd::Idle);

#[derive(Clone, PartialEq)]
enum Upd {
    Idle,
    Downloading,
    /// saved here and opened
    Done(String),
    /// the download page was opened instead (bool = already sent)
    Failed(String, bool),
}

/// The site's download page: "Update now" falls back to it when the direct download doesn't work.
pub fn set_download_page(url: &'static str) {
    let _ = PAGE.set(url);
}

/// The end of this computer's file name in a release (GCarve-0.1.2-macOS.dmg, …-Windows-x64.zip, …-Linux-aarch64.tar.gz).
fn asset_suffix() -> &'static str {
    if cfg!(target_os = "macos") {
        "-macOS.dmg"
    } else if cfg!(all(windows, target_vendor = "win7", target_arch = "x86")) {
        "-Windows-win7-x86.zip"
    } else if cfg!(all(windows, target_vendor = "win7")) {
        "-Windows-win7-x64.zip"
    } else if cfg!(windows) {
        "-Windows-x64.zip"
    } else if cfg!(target_arch = "aarch64") {
        "-Linux-aarch64.tar.gz"
    } else {
        "-Linux-x86_64.tar.gz"
    }
}

/// The release file for `suffix` in a GitHub releases/latest answer (only github.com links).
fn pick_asset(release: &serde_json::Value, suffix: &str) -> Option<String> {
    release["assets"].as_array()?.iter().find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(suffix)))?["browser_download_url"]
        .as_str()
        .filter(|u| u.starts_with("https://github.com/"))
        .map(str::to_string)
}

/// `repo` = "owner/name" on GitHub that holds the public releases; `version` = the running program's version.
pub fn init(repo: &'static str, version: &'static str) {
    #[cfg(not(target_arch = "wasm32"))]
    let version = std::env::var("GCARVE_VERSION").map_or(version, |v| Box::leak(v.into_boxed_str())); // dev: pretend to be an older build
    let _ = INFO.set((repo, version));
}

fn parse(v: &str) -> Vec<u32> {
    v.trim().trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn version() -> &'static str {
    INFO.get().map_or(env!("CARGO_PKG_VERSION"), |i| i.1)
}

/// The newest version's number and page when it is newer than this program.
pub fn available() -> Option<(String, String)> {
    NEWEST.lock().ok()?.clone().filter(|(v, _)| parse(v) > parse(version()))
}

/// Dev / screenshots: behave as if `v` (at `url`) were the newest release.
pub fn pretend_newest(v: &str, url: &str) {
    *NEWEST.lock().unwrap() = Some((v.to_string(), url.to_string()));
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
                *ASSET.lock().unwrap() = j.as_ref().and_then(|j| pick_asset(j, asset_suffix()));
            } else {
                *FAILED.lock().unwrap() = true;
            }
            ctx.request_repaint();
        });
    }
    #[cfg(target_arch = "wasm32")]
    let _ = ctx;
}

/// Download the newest file for this computer into Downloads and open it (macOS mounts the .dmg, Windows shows the
/// zip, Linux the folder). Any failure → `Upd::Failed`, and the dialog opens the download page instead.
#[cfg(not(target_arch = "wasm32"))]
fn update_now(ctx: &egui::Context) {
    let Some(url) = ASSET.lock().ok().and_then(|a| a.clone()) else {
        *UPDATE.lock().unwrap() = Upd::Failed("No file for this computer in the release.".into(), false);
        return;
    };
    *UPDATE.lock().unwrap() = Upd::Downloading;
    let ctx = ctx.clone();
    let mut req = ehttp::Request::get(&url);
    req.headers.insert("User-Agent", "GCarve");
    ehttp::fetch(req, move |r| {
        let name = url.rsplit('/').next().unwrap_or("update").to_string();
        let res = r.map_err(|e| e.to_string()).and_then(|r| if r.ok && r.bytes.len() > 1_000_000 { Ok(r.bytes) } else { Err(format!("download failed ({})", r.status)) }).and_then(|bytes| {
            let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(std::path::PathBuf::from);
            let dir = home.map(|h| h.join("Downloads")).filter(|d| d.is_dir()).unwrap_or_else(std::env::temp_dir);
            let path = dir.join(&name);
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            let target = if cfg!(target_os = "linux") { dir.clone() } else { path.clone() };
            let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
            std::process::Command::new(opener).arg(&target).spawn().map_err(|e| e.to_string())?;
            Ok(path.display().to_string())
        });
        *UPDATE.lock().unwrap() = match res {
            Ok(p) => Upd::Done(p),
            Err(e) => Upd::Failed(e, false),
        };
        ctx.request_repaint();
    });
}

/// The dialogs behind the three Help items.
#[derive(Default)]
pub struct Help {
    report: Option<Report>,
    pub update_open: bool,
    told: bool,
    told_dev: bool,
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
                    let page = PAGE.get().map_or(url.as_str(), |p| p);
                    let state = UPDATE.lock().map(|u| u.clone()).unwrap_or(Upd::Idle);
                    match state {
                        #[cfg(not(target_arch = "wasm32"))]
                        Upd::Idle if std::env::var("GCARVE_UPDATE_NOW").is_ok() && !self.told_dev => {
                            // dev: press Update now at once; a URL value replaces the file's link (a bad one tests the fallback)
                            self.told_dev = true;
                            if let Ok(u) = std::env::var("GCARVE_UPDATE_NOW").map(|u| u.trim().to_string()).and_then(|u| if u.starts_with("http") { Ok(u) } else { Err(std::env::VarError::NotPresent) }) {
                                *ASSET.lock().unwrap() = Some(u);
                            }
                            update_now(ctx);
                        }
                        Upd::Idle => {
                            ui.label(RichText::new("Update now downloads it and opens it; then quit this one and install it over it. Your projects and settings stay.").small().color(theme::TEXT_DIM));
                            ui.horizontal(|ui| {
                                #[cfg(not(target_arch = "wasm32"))]
                                if ui.add(widgets::Button::new(format!("Update now to {v}")).primary(true)).clicked() {
                                    update_now(ctx);
                                }
                                if ui.button("Download page").clicked() {
                                    ctx.open_url(egui::OpenUrl::new_tab(page));
                                }
                            });
                        }
                        Upd::Downloading => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(format!("Downloading {app} {v}…"));
                            });
                        }
                        Upd::Done(path) => {
                            ui.label(RichText::new(format!("Downloaded to {path}")).small().color(theme::TEXT_DIM));
                            let how = if cfg!(target_os = "macos") {
                                format!("Quit {app}, then drag {app} from the window that opened onto Applications and choose Replace.")
                            } else if cfg!(windows) {
                                format!("Quit {app}, then copy the files from the zip that opened over your {app} folder.")
                            } else {
                                format!("Quit {app}, then unpack the file and run ./install.sh (or ./gcarve) from it.")
                            };
                            ui.label(how);
                            if ui.button("Didn't open? Download page").clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(page));
                            }
                        }
                        Upd::Failed(e, sent) => {
                            if !sent {
                                ctx.open_url(egui::OpenUrl::new_tab(page)); // the fallback: the download page in the browser
                                *UPDATE.lock().unwrap() = Upd::Failed(e.clone(), true);
                            }
                            ui.label(RichText::new(format!("The update couldn't be downloaded here ({e}). The download page opened in your browser: get {app} {v} there.")).color(theme::DANGER));
                            if ui.button("Open the download page again").clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(page));
                            }
                        }
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
    /// `cargo test -p gcarve-ui live_update -- --ignored`: a 0.1.0 build must see the real newest release.
    #[test]
    #[ignore]
    fn live_update_check_finds_a_newer_release() {
        super::init("fillinthe/gcarve", "0.1.0");
        super::check(&super::egui::Context::default());
        for _ in 0..100 {
            if let Some((v, url)) = super::available() {
                assert!(super::parse(&v) > super::parse("0.1.0") && url.starts_with("https://github.com/fillinthe/gcarve/"));
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("no answer from GitHub in 10 s");
    }

    #[test]
    fn the_release_file_for_each_computer_is_found() {
        let j = serde_json::json!({"assets": [
            {"name": "GCarve-0.1.2-macOS.dmg", "browser_download_url": "https://github.com/x/y/releases/download/v0.1.2/GCarve-0.1.2-macOS.dmg"},
            {"name": "GCarve-0.1.2-Windows-x64.zip", "browser_download_url": "https://github.com/x/y/releases/download/v0.1.2/GCarve-0.1.2-Windows-x64.zip"},
            {"name": "GCarve-0.1.2-Windows-win7-x64.zip", "browser_download_url": "https://github.com/x/y/releases/download/v0.1.2/GCarve-0.1.2-Windows-win7-x64.zip"},
            {"name": "evil-Linux-x86_64.tar.gz", "browser_download_url": "https://evil.example/x.tar.gz"}]});
        assert!(super::pick_asset(&j, "-Windows-x64.zip").unwrap().ends_with("GCarve-0.1.2-Windows-x64.zip"));
        assert!(super::pick_asset(&j, "-Windows-win7-x64.zip").unwrap().ends_with("win7-x64.zip"));
        assert!(super::pick_asset(&j, "-Linux-x86_64.tar.gz").is_none(), "only github.com links");
        assert!(super::pick_asset(&j, super::asset_suffix()).is_some() || !cfg!(target_os = "macos"));
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(super::parse("v0.10.0") > super::parse("0.9.3"));
        assert!(super::parse("0.1.1") > super::parse("0.1.0"));
        assert!(super::parse("0.1.0") <= super::parse("0.1.0"));
    }
}
