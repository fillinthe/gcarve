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
/// The last problem the app showed (Cloud::report notes every one): (what went wrong, when it was first seen on screen,
/// egui seconds; None until the next frame). The status bar offers "Report this" for a while after.
static PROBLEM: Mutex<Option<(String, Option<f64>)>> = Mutex::new(None);

/// Apps that want the "it stopped last time" report form at start (crash.txt from their panic hook) call this once.
/// Off by default: GSketch offers its own.
pub fn offer_crash_reports() {
    CRASH_OFFER.store(true, std::sync::atomic::Ordering::Relaxed);
}
static CRASH_OFFER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Something went wrong in front of the user: remember it so they can report it in one click.
pub fn note_problem(text: &str) {
    *PROBLEM.lock().unwrap_or_else(|e| e.into_inner()) = Some((text.chars().take(2000).collect(), None));
}

/// The problem to offer a report for: shown in the last 90 s and not reported or dismissed yet.
pub fn recent_problem(now: f64) -> Option<String> {
    let mut p = PROBLEM.lock().unwrap_or_else(|e| e.into_inner());
    let (text, at) = p.as_mut()?;
    let at = *at.get_or_insert(now);
    (now - at < 90.0).then(|| text.clone())
}

fn forget_problem() {
    *PROBLEM.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

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
        req.headers.insert("User-Agent", crate::i18n::t("GCarve"));
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
        *UPDATE.lock().unwrap() = Upd::Failed(crate::i18n::t("No file for this computer in the release.").into(), false);
        return;
    };
    *UPDATE.lock().unwrap() = Upd::Downloading;
    let ctx = ctx.clone();
    let mut req = ehttp::Request::get(&url);
    req.headers.insert("User-Agent", crate::i18n::t("GCarve"));
    ehttp::fetch(req, move |r| {
        let name = url.rsplit('/').next().unwrap_or("update").to_string();
        let res = r.map_err(|e| e.to_string()).and_then(|r| if r.ok && r.bytes.len() > 1_000_000 { Ok(r.bytes) } else { Err(format!("download failed ({})", r.status)) }).and_then(|bytes| {
            let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(std::path::PathBuf::from);
            let dir = home.map(|h| h.join(crate::i18n::t("Downloads"))).filter(|d| d.is_dir()).unwrap_or_else(std::env::temp_dir);
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
    /// what the app sends along (the error, or the crash), shown read-only and added to the message
    attached: String,
    /// a crash from the last run: say so, and "Don't send" closes it for good
    crash: bool,
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
        let title = if kind == "bug" { crate::i18n::t("Report a bug") } else { crate::i18n::t("Request a change") };
        self.report = Some(Report { title, attached: String::new(), crash: false, kind, subject: String::new(), body: String::new(), email: cloud.user.as_ref().map(|u| u.email.clone()).unwrap_or_default(), sending: false, result: None });
    }

    /// "Report this": the report form with the problem attached (`crash` = the last run ended in a crash).
    pub fn open_problem(&mut self, cloud: &Cloud, problem: &str, crash: bool) {
        let app = crate::store::app();
        let first = problem.lines().next().unwrap_or("").trim();
        let subject = if crash { format!("{app} closed unexpectedly") } else { format!("Problem: {}", first.chars().take(70).collect::<String>()) };
        let title = if crash { crate::i18n::t("Sorry, it stopped last time") } else { crate::i18n::t("Report this problem") };
        self.report = Some(Report { title, attached: problem.trim().to_string(), crash, kind: "bug", subject, body: String::new(), email: cloud.user.as_ref().map(|u| u.email.clone()).unwrap_or_default(), sending: false, result: None });
        forget_problem();
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
            if d.crash && matches!(d.result, Some(Ok(_))) {
                crate::store::remove("crash.txt");
            }
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
        static CRASH: OnceLock<()> = OnceLock::new();
        if !quiet && CRASH_OFFER.load(std::sync::atomic::Ordering::Relaxed) && CRASH.set(()).is_ok() {
            if let Some(c) = crate::store::load("crash.txt").filter(|c| !c.trim().is_empty()) {
                self.open_problem(cloud, &c, true);
            }
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
            let intro = if d.crash {
                format!("{app} hit a problem and closed last time. Telling us what you were doing helps us fix it: the technical details below go with it.")
            } else if !d.attached.is_empty() {
                format!("Tell us what you were doing when this happened. It goes straight to the people building {app}, with the details below, the version and the system.")
            } else {
                format!("Report a problem, ask for a feature or ask a question. It goes straight to the people building {app}. The version and system are added for you.")
            };
            ui.add(egui::Label::new(RichText::new(intro).small().color(theme::TEXT_DIM)).wrap());
            egui::Grid::new("req").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label(crate::i18n::t("Type"));
                let items = [("bug", crate::i18n::t("Problem / bug")), ("suggestion", crate::i18n::t("Feature or change")), ("question", crate::i18n::t("Question"))];
                widgets::combo_box("req-kind").selected_text(items.iter().find(|i| i.0 == d.kind).map_or("", |i| i.1)).width(220.0).show_ui(ui, |ui| {
                    for (v, l) in items {
                        ui.selectable_value(&mut d.kind, v, l);
                    }
                });
                ui.end_row();
                ui.label(crate::i18n::t("Summary"));
                ui.add(widgets::field(&mut d.subject).desired_width(330.0).hint_text(crate::i18n::t("e.g. Export stops at 50%")));
                ui.end_row();
                ui.label(crate::i18n::t("Details"));
                ui.add(egui::TextEdit::multiline(&mut d.body).desired_width(330.0).desired_rows(if d.attached.is_empty() { 6 } else { 4 }).hint_text(if d.attached.is_empty() { crate::i18n::t("What happened, and what did you expect? Steps to reproduce help a lot.") } else { crate::i18n::t("What were you doing? (optional, but it helps a lot)") }));
                ui.end_row();
                if !d.attached.is_empty() {
                    ui.label(crate::i18n::t("Sent along"));
                    egui::ScrollArea::vertical().id_salt("attached").max_height(90.0).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&d.attached).monospace().small().color(theme::TEXT_DIM)).wrap());
                    });
                    ui.end_row();
                }
                if !signed_in {
                    ui.label(crate::i18n::t("Your email"));
                    ui.add(widgets::field(&mut d.email).desired_width(330.0).hint_text(crate::i18n::t("so we can answer")));
                    ui.end_row();
                }
            });
            // with the problem attached the note is optional (the server wants a few words: the details count)
            let ok = (d.body.trim().len() >= 5 || !d.attached.is_empty()) && (signed_in || d.email.contains('@'));
            ui.horizontal(|ui| {
                if ui.add_enabled(ok && !d.sending, egui::Button::new(if d.sending { crate::i18n::t("Sending…") } else { crate::i18n::t("Send") })).clicked() {
                    d.sending = true;
                    d.result = None;
                    let sys = format!("\n\n— {app} {} · {} {}{}", version(), std::env::consts::OS, std::env::consts::ARCH, if Cloud::web() { crate::i18n::t(" · web") } else { "" });
                    let note = if d.body.trim().is_empty() { crate::i18n::t("(no note)") } else { d.body.trim() };
                    let details = if d.attached.is_empty() { String::new() } else { format!("\n\nDetails:\n{}", d.attached) };
                    cloud.send_request(ctx, d.kind, d.subject.trim(), &format!("{note}{details}{sys}"), d.email.trim());
                }
                if signed_in && ui.link(crate::i18n::t("Your requests and answers")).clicked() {
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
            if self.report.as_ref().is_some_and(|d| d.crash) {
                crate::store::remove("crash.txt"); // dismissed: don't ask again for that crash
            }
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
        widgets::Dialog::new(crate::i18n::t("Check for updates")).id("update").open(&mut self.update_open).close_footer().width(380.0).show(ctx, |ui| {
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
                            ui.label(RichText::new(crate::i18n::t("Update now downloads it and opens it; then quit this one and install it over it. Your projects and settings stay.")).small().color(theme::TEXT_DIM));
                            ui.horizontal(|ui| {
                                #[cfg(not(target_arch = "wasm32"))]
                                if ui.add(widgets::Button::new(format!("Update now to {v}")).primary(true)).clicked() {
                                    update_now(ctx);
                                }
                                if ui.button(crate::i18n::t("Download page")).clicked() {
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
                            if ui.button(crate::i18n::t("Didn't open? Download page")).clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(page));
                            }
                        }
                        Upd::Failed(e, sent) => {
                            if !sent {
                                ctx.open_url(egui::OpenUrl::new_tab(page)); // the fallback: the download page in the browser
                                *UPDATE.lock().unwrap() = Upd::Failed(e.clone(), true);
                            }
                            ui.label(RichText::new(format!("The update couldn't be downloaded here ({e}). The download page opened in your browser: get {app} {v} there.")).color(theme::DANGER));
                            if ui.button(crate::i18n::t("Open the download page again")).clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(page));
                            }
                        }
                    }
                }
                (Some(_), None) => {
                    ui.label(RichText::new(crate::i18n::t("You have the newest version.")).color(theme::MODE_SIMULATE));
                }
                (None, _) if failed => {
                    ui.label(RichText::new(crate::i18n::t("Could not reach the update server. Check your internet connection and try again.")).color(theme::DANGER));
                }
                _ => {
                    ui.label(crate::i18n::t("Checking…"));
                }
            }
            if ui.button(crate::i18n::t("Check again")).clicked() {
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
