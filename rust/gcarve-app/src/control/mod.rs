//! GCarve Control: the machine screen for GRBL / grblHAL / FluidNC controllers (USB serial or TCP; the simulator
//! everywhere). Its own process on the desktop (`gcarve --control [file]`, one per computer via a lock file next to
//! the queue folder) so a design-app crash can't stop a running job; its own tab on the web (`?control`).
//! The design app hands files over through the queue: `send()` writes the GRBL post output there, `open()` starts Control.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
mod link;
mod preview;
mod screen;
#[cfg(target_arch = "wasm32")]
mod webserial;

pub use screen::ControlApp;

use gcarve_core::document::Document;
use gcarve_core::pipeline::{self, Generated};
use serde::{Deserialize, Serialize};

/// QtCore Settings category "control" of the Qt app, persisted with store.rs
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    /// 0 simulator, 1 network, 2 USB
    pub transport: usize,
    pub serial_port: String,
    pub baud: u32,
    pub jog_step: f64,
    pub jog_feed: f64,
    /// Panther 5x8; the controller's $130-$132 win
    pub travel: [f64; 3],
    /// touch plate
    pub plate: f64,
    pub probe_max: f64,
    pub probe_feed: f64,
    pub probe_lift: f64,
    pub warm_rpm: i32,
    pub warm_sec: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { host: "192.168.5.1".into(), port: 23, transport: 0, serial_port: String::new(), baud: 115200, jog_step: 0.1, jog_feed: 200.0, travel: [60.0, 96.0, 6.0], plate: 0.75, probe_max: 2.0, probe_feed: 5.0, probe_lift: 0.25, warm_rpm: 8000, warm_sec: 60 }
    }
}

impl Settings {
    pub fn load() -> Self {
        crate::store::load("control.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        if let Ok(t) = serde_json::to_string_pretty(self) {
            crate::store::save("control.json", &t);
        }
    }
}

// ---- queue: files sent from GCarve ---------------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
pub fn queue_dir() -> Option<std::path::PathBuf> {
    crate::store::config_dir().map(|d| d.join("control-queue"))
}

/// (display name, key): newest first. Desktop key = file path; web key = the name in the stored queue.
pub fn queue() -> Vec<(String, String)> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(dir) = queue_dir() else { return Vec::new() };
        let mut files: Vec<(std::time::SystemTime, String, String)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| ["nc", "gcode", "ngc", "tap"].contains(&x.to_ascii_lowercase().as_str())))
            .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.file_name().to_string_lossy().into_owned(), e.path().to_string_lossy().into_owned()))
            .collect();
        files.sort_by_key(|a| std::cmp::Reverse(a.0));
        files.into_iter().map(|(_, n, p)| (n, p)).collect()
    }
    #[cfg(target_arch = "wasm32")]
    web_queue().into_iter().map(|(n, _)| (n.clone(), n)).collect()
}

/// G-code text of a queue entry
pub fn read_queued(key: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(key).ok()
    }
    #[cfg(target_arch = "wasm32")]
    web_queue().into_iter().find(|(n, _)| n == key).map(|(_, t)| t)
}

#[cfg(target_arch = "wasm32")]
fn web_queue() -> Vec<(String, String)> {
    crate::store::load("control-queue.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// File ▸ Send to GCarve Control: the project as GRBL (inch) files into the queue, then Control opens on the first.
/// Returns the status message. ponytail: safety findings go into the message (like the Qt Jobs path) instead of
/// the export guard's "Export anyway" dialog; Control's own start check is the gate before anything moves.
pub fn send(doc: &Document, gen: Option<&Generated>, project: &str) -> String {
    let Some(gen) = gen.filter(|g| g.paths.iter().any(|p| !p.is_empty())) else { return i18n::t("Nothing to send: add a toolpath first.").into() };
    let ts = chrono::Local::now().format("%I:%M %p, %A %B %-d %Y").to_string();
    let mut post = gcarve_core::find_post("grbl-in").expect("grbl-in post").clone();
    // one file for the whole job: Control stops at each M6 for a manual bit change (grbl itself refuses M6)
    post.tool_change = "({TOOLNAME})\nM5\nG0 Z{SAFEZ}\nT{TOOLNUM} M6\nM3 S{RPM}\nG4 P2\n".into();
    let r = pipeline::export(doc, gen, project, &ts, 0.0, &post);
    let n = r.files.len();
    #[cfg(not(target_arch = "wasm32"))]
    let first = {
        let Some(dir) = queue_dir() else { return i18n::t("No config folder for the GCarve Control queue").into() };
        let _ = std::fs::create_dir_all(&dir);
        let mut first = None;
        for f in &r.files {
            let p = dir.join(&f.filename);
            if let Err(e) = std::fs::write(&p, &f.text) {
                return format!("Cannot write {}: {e}", p.display());
            }
            first.get_or_insert(p);
        }
        first
    };
    #[cfg(target_arch = "wasm32")]
    let first = {
        let mut q = web_queue();
        q.retain(|(n, _)| !r.files.iter().any(|f| &f.filename == n));
        q.splice(0..0, r.files.iter().map(|f| (f.filename.clone(), f.text.clone())));
        crate::store::save("control-queue.json", &serde_json::to_string(&q).unwrap_or_default());
        r.files.first().map(|f| f.filename.clone())
    };
    open(first.as_ref());
    let mut msg = format!("Sent {n} file(s) to GCarve Control");
    if !r.safety.is_empty() {
        msg += &format!(". SAFETY: {}", r.safety.join(" "));
    }
    msg
}

/// File ▸ Open GCarve Control: its own process (desktop) or browser tab (web). A second desktop instance exits at once
/// (lock), its file is already in the queue.
#[cfg(not(target_arch = "wasm32"))]
pub fn open(file: Option<&std::path::PathBuf>) {
    if let Ok(exe) = std::env::current_exe() {
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--control");
        cmd.args(file);
        let _ = cmd.spawn();
    }
}
#[cfg(target_arch = "wasm32")]
pub fn open(_file: Option<&String>) {
    if let Some(w) = web_sys::window() {
        let _ = w.open_with_url_and_target("?control", "gcarve-control");
    }
}

/// `gcarve --control [file]` (desktop main)
#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() -> eframe::Result {
    let shot = std::env::var("GCARVE_SHOT").is_ok();
    // one GCarve Control per computer: two windows must never drive the same machine
    let lock = queue_dir().and_then(|q| {
        let _ = std::fs::create_dir_all(&q);
        std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(q.with_file_name("control.lock")).ok()
    });
    #[allow(clippy::incompatible_msrv)] // File::try_lock is std since 1.89; the workspace builds with newer toolchains
    let locked = lock.as_ref().is_some_and(|f| f.try_lock().is_ok());
    if !locked && !shot {
        println!("GCarve Control is already running");
        return Ok(());
    }
    let file = std::env::args().skip(1).find(|a| !a.starts_with("--"));
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title(i18n::t("GCarve Control")).with_inner_size([1280.0, 820.0]).with_min_inner_size([1000.0, 640.0]),
        ..Default::default()
    };
    let r = eframe::run_native(i18n::t("GCarve Control"), options, Box::new(move |cc| Ok(Box::new(ControlApp::new(cc, file)))));
    drop(lock);
    r
}

/// web main: `?control` in the URL opens Control instead of the design app
#[cfg(target_arch = "wasm32")]
pub fn requested() -> bool {
    web_sys::window().and_then(|w| w.location().search().ok()).is_some_and(|s| s.contains("control"))
}
