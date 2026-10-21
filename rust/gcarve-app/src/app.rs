
//! Window layout: menu bar, mode bar (Design / Simulate), tool rail (left), inspector (right), canvas.
//! Both side panels fold to a thin edge like the Qt app. Documents live in tabs (tabs.rs): the current one is in
//! the `canvas` / `name` / `cloud_id` / `project_path` / `gen` fields, the others wait in `tabs`.

use crate::canvas::{Canvas, Request, Tool};
use crate::widgets::TipExt;
use crate::popups::Dialogs;
use crate::files::{self, Files, Loaded, Picked};
use crate::palette::{self, Palette};
use crate::tabs::{self, DocState, TabAct};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::units;
use crate::cloud::{Cloud, Reply};
// ---- CAM dialogs / simulation (dialogs/*, playback, view3d, toollib, posts, jobs) ----
use crate::dialogs::{self, Dialogs as CamDialogs, Host};
use crate::playback::Player;
use crate::posts::UserPosts;
use crate::view3d::View3d;
use gcarve_core::pipeline::{self, Generated};
use gcarve_core::project;
use crate::inspector;
use crate::shell::{self, Mode, Shell};
use crate::theme;
use eframe::egui::{self, RichText};

/// safety findings + files waiting for "export anyway"
/// Export dialog state (Qt ExportGuard.qml)
struct ExportDlg {
    air: bool,
    /// air-cut raise text (project unit)
    raise: String,
    /// editable file names, one per output file
    names: Vec<String>,
    /// (tools, toolpaths) per file
    plan: Vec<(String, String)>,
    issues: Vec<String>,
    /// also write <project>_setup.html (the printable job sheet)
    sheet: bool,
    /// web: several files go into one .zip
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    zip: bool,
    /// cut a board longer than the machine in sections: on, along Y, tile length and overlap (project unit)
    tile: bool,
    tile_y: bool,
    tile_len: String,
    tile_overlap: String,
    #[cfg(not(target_arch = "wasm32"))]
    folder: Option<PathBuf>,
}

/// seconds after the last edit before the previews are worked out again
const GEN_PAUSE: f64 = 0.4;
/// store key: the last folder G-code was exported to (desktop)
#[cfg(not(target_arch = "wasm32"))]
const EXPORT_FOLDER_KEY: &str = "export-folder";
/// fastest simulation playback (× machine time): above it each frame only stamps more moves
const MAX_SIM_SPEED: f64 = 200.0;

pub struct App {
    canvas: Canvas,
    mode: Mode,
    left_open: bool,
    right_open: bool,
    status: String,
    message: String,   // last file/command result
    name: String,      // project name in the title
    files: Files,
    shell: Shell,      // mode-layout chrome state (inspector tab, selected toolpath, palette…)
    frame: u64,
    gen: Option<Generated>,
    gen_rev: u64,
    /// which toolpaths `gen` holds (by index); while drawing only the ones on screen are worked out
    gen_cov: Vec<bool>,
    /// the last edit seen (document rev, dialog settings) and when: previews wait `GEN_PAUSE` after it
    edit_seen: (u64, Option<(Option<usize>, gcarve_core::document::Op)>),
    edit_at: f64,
    /// View › Every Toolpath While Drawing (store `design-all-paths`): off = only the picked toolpath / the one in its dialog
    all_paths: bool,
    /// toolpath previews on at all (status bar Paths, ⇧⌘T, the Toolpaths tab's Show switch; store `show-paths`, per
    /// device): off = nothing is drawn or worked out while drawing. Copied onto the canvas each frame.
    show_paths: bool,
    /// account library sync: the account copy has been merged (uploads only after that), last uploaded library rev
    lib_ready: bool,
    lib_pushed: u64,
    /// Shared with me: waiting shares, window open
    shares: Vec<crate::cloud::ShareInfo>,
    shares_open: bool,
    /// the open toolpath dialog's settings the previews were last built with (live preview)
    gen_draft: Option<(Option<usize>, gcarve_core::document::Op)>,
    export: Option<ExportDlg>,
    // ---- CAM dialogs / simulation ----
    cam: CamDialogs,
    posts: UserPosts,
    /// toolpath template library (templates.rs)
    templates: crate::templates::Templates,
    player: Player,
    view3d: View3d,
    /// the simulation is about to be rebuilt: this frame shows "Preparing the simulation" first
    sim_wait: bool,
    /// a toolpath folder's Export: only its toolpaths are exported (None = all)
    export_group: Option<String>,
    /// View › All Sheets: every sheet side by side (allsheets.rs), remembered per device
    all_sheets: bool,
    /// files picked / dropped, read one frame after the "Opening…" card is shown (`busy_card`)
    opening: Vec<Picked>,
    open_wait: bool,
    /// toolpaths of a big drawing are about to be rebuilt: this frame shows "Generating toolpaths" first
    gen_wait: bool,
    /// a SpaceMouse allowed earlier was looked for
    puck_tried: bool,
    puck_filter: crate::spacemouse::Filter,
    puck_trace: crate::spacemouse::Trace,
    view_3d: bool,
    watch_pending: bool,
    /// simulation shown over the Design canvas (Qt canvas.showSim: Watch the Cut stays in Design); Simulate mode always shows it
    show_sim: bool,
    shot: bool,
    // web: recovery copy (recover.rs): pending prompt, when to write the next copy (ctx time), last dirty flag sent to the page
    recover: Option<crate::recover::Recovery>,
    recover_at: Option<f64>,
    recover_rev: u64,
    last_dirty: bool,
    /// a drawing is on screen (web: the page asks before a reload)
    last_open: bool,
    pending_dialog: String,
    project_path: Option<PathBuf>, // desktop: where the project is saved (always None on the web)
    #[cfg(not(target_arch = "wasm32"))]
    jobs: crate::jobs::Jobs,
    // ---- tabs, files, window ----
    doc_id: u64,                 // id of the current document (DocState::id)
    next_doc_id: u64,
    tabs: Vec<Option<DocState>>, // tab order; None = the current document (in the fields above)
    cur: usize,
    recent: Vec<String>,
    confirm: Option<Pending>,    // "Save changes to …?" before closing a dirty tab / quitting
    after_save: Option<Pending>, // continue once the Save As of that document lands
    palette: Palette,
    /// web: `?open=<project id>` in the address (GSketch's Send to GCarve): opened once the session is known
    open_at_start: Option<String>,
    /// /app/?share=1 (the "Share your project" button on gworkshop.app/projects): sign in, pick a project, then the
    /// Share to Community box (steps below, 0 = off)
    share_start: u8,
    title: String,               // last window title sent
    about_open: bool,
    /// usage counts: op count while the toolpath dialog was open; "toolpath" already sent this session
    ops_before: Option<usize>,
    counted_toolpath: bool,
    counted_open: bool,
    /// signed in as an admin: usage isn't counted
    staff: bool,
    /// web smoke test (`?smoke`): stage and the first stage's numbers
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    smoke_step: u8,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    smoke_note: String,
    guide: dialogs::guide::Guide,
    ui_state: UiState,
    cloud: Cloud,
    cloud_id: String,          // cloud project this document came from / was saved to
    account: gcarve_ui::account::Account,
    /// web start: the sign-in box only appears once the session check says nobody is signed in
    /// Help ▸ Request a Change
    help: gcarve_ui::help::Help,
    /// dev: GCARVE_MOVIE=dir writes the simulation as numbered PNG frames (see `movie_tick`)
    #[cfg_attr(not(all(feature = "screenshot", not(target_arch = "wasm32"))), allow(dead_code))] // read only by movie_tick
    movie: Option<(std::path::PathBuf, usize, usize, bool)>,
    projects: dialogs::projects::ProjectsDialog,
    dialogs: Dialogs,  // vector-op / text dialogs (popups.rs)
    select_all_on_load: bool, // dev hook GCARVE_SELECT=all
    /// why the account box opened (shown on top of it; cleared when it closes)
    /// last frame's canvas tool: picking Measure brings the Properties tab (its readout) forward
    prev_tool: Tool,
    dev_measure: bool,       // desktop: the account dialog can't be dismissed until licensed (Qt desktopGate)
    // account app defaults (Qt Main.qml prefsLoaded / prefsPush)
    prefs_seen: dialogs::prefs::Synced, // last synced settings handled (saved, pushed, applied to a blank tab)
    prefs_push_at: Option<f64>,         // push 1.5 s after the last edit
    prefs_from_server: bool,            // the change came from the account: don't push it back
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        crate::fonts::install(&cc.egui_ctx);
        crate::icons::install(&cc.egui_ctx, crate::app_icons::RAW);
        let files = Files::default();
        #[cfg(not(target_arch = "wasm32"))]
        for arg in std::env::args().skip(1).filter(|a| !a.starts_with("--")) {
            files.open_path(std::path::Path::new(&arg)); // gcarve project.tlp / file.svg
        }
        #[cfg(not(target_arch = "wasm32"))]
        let dialog = std::env::var("GCARVE_DIALOG").unwrap_or_default();
        #[cfg(target_arch = "wasm32")]
        let dialog = String::new();
        #[cfg(not(target_arch = "wasm32"))]
        let mode = if std::env::var("GCARVE_MODE").as_deref() == Ok("simulate") { Mode::Simulate } else { Mode::Design }; // dev: screenshots ("toolpaths" = Design + Toolpaths tab)
        #[cfg(target_arch = "wasm32")]
        let mode = Mode::Design;
        let shot = std::env::var("GCARVE_SHOT").is_ok();
        let ui_state = if shot { UiState::default() } else { UiState::load() }; // screenshots use the stock layout (Qt: temp settings)
        let prefs = if shot { dialogs::prefs::Prefs::default() } else { dialogs::prefs::Prefs::load() }; // screenshots: stock settings (Qt: temp settings)
        gcarve_ui::help::init("fillinthe/gcarve", env!("CARGO_PKG_VERSION"));
        gcarve_ui::help::set_download_page("https://gcarve.app/download/");
        let mut app = App { files, mode, canvas: new_canvas(&prefs), left_open: ui_state.tool_rail, right_open: ui_state.side_panel, status: String::new(), message: "Open a VCarve project (.crv), an SVG, DXF or PDF, or draw with R / E / P.".into(), name: "Untitled".into(), shell: Shell::new(), frame: 0, gen: None, gen_rev: 0, gen_cov: Vec::new(), edit_seen: (0, None), edit_at: 0.0, show_paths: shot || crate::store::load("show-paths").as_deref() != Some("0"), all_paths: std::env::var("GCARVE_ALL_PATHS").map_or(shot || crate::store::load("design-all-paths").as_deref() == Some("1"), |v| v == "1"), lib_ready: false, lib_pushed: 0, shares: Vec::new(), shares_open: false, gen_draft: None, export: None, cloud: Cloud::new(), cloud_id: String::new(), account: Default::default(), help: Default::default(), movie: std::env::var("GCARVE_MOVIE").ok().map(|d| (d.into(), 0, std::env::var("GCARVE_MOVIE_FRAMES").ok().and_then(|n| n.parse().ok()).unwrap_or(600), false)), projects: Default::default(), dialogs: Dialogs::default(), select_all_on_load: false, prev_tool: Tool::Select, dev_measure: false,
            prefs_seen: prefs.synced.clone(), prefs_push_at: None, prefs_from_server: false,
            cam: CamDialogs { prefs, ..Default::default() }, posts: UserPosts::load(), templates: Default::default(), player: Player::default(), view3d: View3d::load(shot), sim_wait: false, export_group: None, all_sheets: !shot && crate::store::load("all-sheets").as_deref() == Some("1") || std::env::var("GCARVE_ALL_SHEETS").is_ok(), opening: Vec::new(), open_wait: false, gen_wait: false, puck_tried: shot, puck_filter: Default::default(), puck_trace: Default::default(), view_3d: false, watch_pending: false, show_sim: false, shot, pending_dialog: String::new(), recover: if shot { None } else { crate::recover::load() }, recover_at: None, recover_rev: 0, last_dirty: false, last_open: false,
            project_path: None,
            #[cfg(not(target_arch = "wasm32"))]
            jobs: crate::jobs::Jobs::load(),
            doc_id: 1, next_doc_id: 2, tabs: vec![None], cur: 0, recent: if cfg!(target_arch = "wasm32") { Vec::new() } else { files::recent() }, confirm: None, after_save: None, palette: Palette::default(), open_at_start: None, share_start: 0, title: String::new(), about_open: false, ops_before: None, counted_toolpath: false, counted_open: false, staff: false, smoke_step: 0, smoke_note: String::new(), guide: Default::default(), ui_state };
        // keyboard zoom is the canvas's (⌘+ / ⌘− / ⌘0), not the whole UI's
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        // ---- CAM dialogs / simulation ----
        app.cam.prefs.apply(&cc.egui_ctx, app.shot);
        app.canvas.hit_scale = app.cam.prefs.hit_scale();
        crate::icons::install(&cc.egui_ctx, crate::app_icons::RAW);
        app.view_3d = dialog == "sim3d";
        if dialog == "sim3d" {
            app.mode = Mode::Simulate;
        }
        if dialog == "watch" {
            app.show_sim = true; // Qt --dialog watch: playback over Design
            app.watch_pending = true;
        }
        app.pending_dialog = dialog.clone(); // opened on frame 2, once the command-line file is in
        app.player.speed = 10.0; // SimBar default
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Ok(p) = std::env::var("GCARVE_STL") {
                app.files.open_path(std::path::Path::new(&p)); // dev: a 3D model for the relief dialogs
            }
            if dialog == "jobs" && app.shot {
                app.jobs = crate::jobs::Jobs::sample(); // dev: like the Qt --dialog jobs sample job
            }
        }
        app.cloud.check_session(&cc.egui_ctx);
        app.cloud.fetch_config(&cc.egui_ctx);
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(slug) = std::env::var("GCARVE_COMMUNITY") {
            app.cloud.community_open(&cc.egui_ctx, &slug); // dev: what /app/?community=<slug> does on the web
        }
        #[cfg(target_arch = "wasm32")]
        {
            // "Open in GCarve" on a community project page: /app/?community=<slug>
            let slug = web_sys::window().and_then(|w| w.location().search().ok()).and_then(|q| {
                q.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix("community=")).filter(|s| !s.is_empty() && s.len() < 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).map(str::to_string)
            });
            if let Some(s) = slug {
                app.cloud.community_open(&cc.egui_ctx, &s);
            }
            app.share_start = u8::from(web_sys::window().and_then(|w| w.location().search().ok()).is_some_and(|q| q.trim_start_matches('?').split('&').any(|kv| kv == "share=1")));
            app.open_at_start = web_sys::window().and_then(|w| w.location().search().ok()).and_then(|q| {
                q.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix("open=")).filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).map(str::to_string)
            });
        }
        // the web app is open to everyone: no sign-in box at start (it opens when someone brings a file in or uses the cloud)
        app.account.open = dialog == "account" || dialog == "account-create";
        app.account.startup = false;
        if dialog == "request" {
            app.help.open_report(&app.cloud, "suggestion");
        }
        app.account.gate = dialog == "licence"; // dev: the desktop licence gate in a screenshot
        app.guide.open = dialog == "guide"; // dev: Help ▸ How Do I…?
        app.dev_measure = dialog == "measure"; // dev: a finished measurement, set once the project has loaded
        app.account.open |= app.account.gate;
        app.projects.open = dialog == "cloud";
        if app.projects.open {
            // dev screenshot: a few sample projects in folders
            let p = |n: &str, f: &str| crate::cloud::ProjectInfo { id: n.into(), name: n.into(), folder: f.into(), size: 40_000, updated: 1_791_000_000 };
            app.cloud.projects = vec![p("Welcome sign", ""), p("Cabinet doors", "Kitchen"), p("Drawer fronts", "Kitchen"), p("Bar top", "Shop/Signs"), p("Logo", "Shop")];
            app.cloud.storage = Some((13_400_000, 41_943_040, 5, 200));
        }
        app.shell.side_tab = if app.mode == Mode::Design { 0 } else { 2 }; // Qt setMode
        #[cfg(not(target_arch = "wasm32"))]
        if std::env::var("GCARVE_MODE").as_deref() == Ok("toolpaths") {
            app.shell.side_tab = 2;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(t) = std::env::var("GCARVE_TAB").ok().and_then(|t| t.parse().ok()) { app.shell.side_tab = t } // dev: screenshots
            app.shell.op_sel = std::env::var("GCARVE_TPSEL").ok().and_then(|t| t.parse().ok()); // dev: a picked toolpath
            if let Some(g) = std::env::var("GCARVE_TPGROUP").ok().and_then(|t| t.parse().ok()) { app.shell.tp_group = g }
            if dialog == "folded" { app.left_open = false; app.right_open = false }
            if dialog == "demo" { app.shell.push("demo") } // dev: the built-in demo as a first visitor gets it
            // dev hooks for screenshots: GCARVE_TOOL=nodes|rect|…, GCARVE_SELECT=all, GCARVE_DIALOG=text|offset|dogbone|array|align|nest
            if let Ok(t) = std::env::var("GCARVE_TOOL") {
                if let Some((tool, _, _)) = Tool::ALL.iter().find(|(_, l, _)| l.eq_ignore_ascii_case(&t)) {
                    app.canvas.tool = *tool;
                }
            }
            let req = match dialog.as_str() { "text" => Some(Request::Text(false)), "offset" => Some(Request::Offset), "dogbone" => Some(Request::Dogbone), "array" => Some(Request::Array), "align" => Some(Request::Align), "nest" => Some(Request::Nest), _ => None };
            app.canvas.requests.extend(req);
            if dialog == "shapemenu" || dialog == "sizepopup" {
                app.canvas.shot_hook = dialog.clone();
            }
            if dialog == "palette" {
                app.palette.show();
                app.palette.query = "v".into();
            }
            app.select_all_on_load = std::env::var("GCARVE_SELECT").is_ok_and(|v| v == "all");
        }
        app
    }
}


/// A fresh document gets the user's saved bit library (Qt: new projects load tools.json) and the new-document
/// defaults (Preferences ▸ New documents).
fn new_canvas(prefs: &dialogs::prefs::Prefs) -> Canvas {
    let mut c = Canvas::default();
    c.doc.tools = crate::toollib::load();
    prefs.synced.new_doc.apply(&mut c);
    c.rev = 1;
    c
}

/// Window layout remembered between runs (Qt Settings category "ui").
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct UiState {
    side_panel: bool,
    tool_rail: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState { side_panel: true, tool_rail: true }
    }
}

impl UiState {
    fn load() -> Self {
        crate::store::load("ui.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
}

/// What waits for the "Save changes?" answer.
#[derive(Clone, Copy)]
enum Pending {
    CloseTab(u64),
    Quit,
}

// ---- account-synced settings (snap, grid, new-document defaults) ----
impl App {
    /// Every frame: the current canvas takes the snap / grid settings; a change is saved, pushed to the account
    /// 1.5 s after the last edit (Qt prefsPush), and an empty untouched tab takes new board defaults at once.
    fn sync_prefs(&mut self, ctx: &egui::Context) {
        let s = &self.cam.prefs.synced;
        self.canvas.snap_prefs = s.snap;
        self.canvas.grid = s.grid;
        let now = ctx.input(|i| i.time);
        if *s != self.prefs_seen {
            if s.new_doc != self.prefs_seen.new_doc && DocState::blank(&self.canvas) && self.project_path.is_none() && self.cloud_id.is_empty() && !self.canvas.doc.can_undo() {
                s.new_doc.apply(&mut self.canvas);
            }
            if !std::mem::take(&mut self.prefs_from_server) && self.cloud.signed_in() {
                self.prefs_push_at = Some(now + 1.5);
            }
            self.prefs_seen = s.clone();
            if !self.shot {
                self.cam.prefs.save();
            }
        }
        if let Some(at) = self.prefs_push_at {
            if now >= at {
                self.prefs_push_at = None;
                self.cloud.push_prefs(ctx, serde_json::to_value(&self.cam.prefs.synced).unwrap_or_default());
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(at - now));
            }
        }
    }
}

// ---- documents in tabs ----
impl App {
    fn take_doc(&mut self) -> DocState {
        DocState {
            id: self.doc_id,
            canvas: std::mem::take(&mut self.canvas),
            name: std::mem::take(&mut self.name),
            cloud_id: std::mem::take(&mut self.cloud_id),
            path: self.project_path.take(),
            gen: self.gen.take(),
            gen_rev: self.gen_rev,
            gen_cov: std::mem::take(&mut self.gen_cov),
        }
    }

    fn put_doc(&mut self, mut d: DocState) {
        d.canvas.hit_scale = self.cam.prefs.hit_scale();
        self.doc_id = d.id;
        self.canvas = d.canvas;
        self.name = d.name;
        self.cloud_id = d.cloud_id;
        self.project_path = d.path;
        self.gen = d.gen;
        self.gen_rev = d.gen_rev;
        self.gen_cov = d.gen_cov;
        // per-document chrome
        self.shell.op_sel = None;
        self.shell.edit_for = None;
        let speed = self.player.speed;
        self.player = Player::default();
        self.player.speed = speed;
    }

    fn switch_tab(&mut self, i: usize) {
        if i == self.cur || i >= self.tabs.len() {
            return;
        }
        let d = self.take_doc();
        self.tabs[self.cur] = Some(d);
        self.cur = i;
        let next = self.tabs[i].take().expect("tab");
        self.put_doc(next);
    }

    fn new_tab(&mut self) {
        let d = self.take_doc();
        self.tabs[self.cur] = Some(d);
        self.tabs.push(None);
        self.cur = self.tabs.len() - 1;
        let id = self.next_doc_id;
        self.next_doc_id += 1;
        self.put_doc(DocState::new(id, new_canvas(&self.cam.prefs)));
        self.gen_rev = self.canvas.rev.wrapping_sub(1); // generate on the next frame
    }

    /// Open-into: the current tab when it is empty and untouched, else a new one (Qt openInTab).
    fn open_target(&mut self) {
        if !DocState::blank(&self.canvas) {
            self.new_tab();
        }
    }

    fn tab_index(&self, doc: u64) -> Option<usize> {
        if doc == self.doc_id {
            return Some(self.cur);
        }
        self.tabs.iter().position(|t| t.as_ref().is_some_and(|d| d.id == doc))
    }

    /// Close a tab, asking first when it has unsaved changes (Qt closeTab).
    fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        self.switch_tab(i);
        if self.canvas.dirty {
            self.confirm = Some(Pending::CloseTab(self.doc_id));
        } else {
            self.drop_current_tab();
        }
    }

    fn drop_current_tab(&mut self) {
        if self.tabs.len() == 1 {
            let id = self.doc_id;
            self.put_doc(DocState::new(id, new_canvas(&self.cam.prefs))); // the last tab becomes a fresh document
            self.gen_rev = self.canvas.rev.wrapping_sub(1);
            return;
        }
        self.tabs.remove(self.cur);
        self.cur = self.cur.min(self.tabs.len() - 1);
        let next = self.tabs[self.cur].take().expect("tab");
        self.put_doc(next);
    }

    /// The answer to "Save changes?" for the current document: `save` = save first (Save As when it has no path).
    fn resolve(&mut self, p: Pending, ctx: &egui::Context, save: bool) {
        if save && !self.save(ctx) {
            self.after_save = Some(p); // continues when the Save As lands
            return;
        }
        match p {
            Pending::CloseTab(id) => {
                if let Some(i) = self.tab_index(id) {
                    self.switch_tab(i);
                    self.drop_current_tab();
                }
            }
            Pending::Quit => {
                self.canvas.dirty = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close); // asks again for the next dirty tab
            }
        }
    }

    /// File ▸ Save: to the project's file (desktop), else Save As; the cloud on the web. true = saved now.
    fn save(&mut self, ctx: &egui::Context) -> bool {
        if Cloud::web() {
            self.save_to_cloud(ctx);
            return false;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(p) = self.project_path.clone() {
            match std::fs::write(&p, project::save_project(&self.canvas.doc)) {
                Ok(()) => {
                    self.canvas.dirty = false;
                    self.recent = files::add_recent(&p);
                    self.message = format!("Saved {}", p.display());
                    return true;
                }
                Err(e) => {
                    self.message = format!("Cannot save {}: {e}", p.display());
                    self.cloud.report("save", &files::scrub(&format!("Cannot save {}: {e}", p.display()), &p.display().to_string()));
                }
            }
        }
        self.save_as();
        false
    }

    fn save_as(&mut self) {
        let bytes = project::save_project(&self.canvas.doc).into_bytes();
        #[cfg(not(target_arch = "wasm32"))]
        self.files.save_project_as(self.doc_id, &self.name, bytes);
        #[cfg(target_arch = "wasm32")]
        files::save(&format!("{}.tlp", self.name), bytes, self.files.msg_tx.clone()); // browser download
    }

    /// A Save As landed: that document now lives at `path`.
    fn saved_as(&mut self, doc: u64, path: PathBuf, ctx: &egui::Context) {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.recent = files::add_recent(&path);
        }
        self.message = format!("Saved {}", path.display());
        match self.tab_index(doc) {
            Some(i) if i == self.cur => {
                self.canvas.dirty = false;
                self.name = name;
                self.project_path = Some(path);
            }
            Some(i) => {
                let d = self.tabs[i].as_mut().unwrap();
                d.canvas.dirty = false;
                d.name = name;
                d.path = Some(path);
            }
            None => {}
        }
        if let Some(p) = self.after_save.take() {
            if let Some(i) = self.tab_index(doc) {
                self.switch_tab(i);
                self.resolve(p, ctx, false);
            }
        }
    }

    /// A picked / dropped / recent file: .tlp opens (in a tab), .svg / .dxf open or import, .stl / .3mf join the current one.
    fn open_picked(&mut self, f: Picked) {
        let Picked { name, bytes, path, import } = f;
        if [".stl", ".3mf"].iter().any(|e| name.to_ascii_lowercase().ends_with(e)) {
            self.import_stl(&name, &bytes);
            return;
        }
        if !import {
            if let Some(i) = path.as_ref().and_then(|p| self.tab_with_path(p)) {
                self.switch_tab(i); // already open: switch instead of opening it twice
                return;
            }
            self.open_target();
        }
        match files::load(&mut self.canvas.doc, &name, &bytes) {
            Loaded::Project(mut d, note) => {
                dialogs::relief::rehydrate(&mut d);
                let (tool, requests, hook) = (self.canvas.tool, std::mem::take(&mut self.canvas.requests), std::mem::take(&mut self.canvas.shot_hook)); // dev hooks set before the load
                self.canvas = Canvas::with_doc(*d);
                self.canvas.shot_hook = hook;
                self.canvas.hit_scale = self.cam.prefs.hit_scale();
                self.canvas.tool = tool;
                self.canvas.requests = requests;
                self.canvas.apply_bindings(); // saved values may come from another sheet's board (Qt load)
                self.canvas.mutated();
                self.canvas.dirty = false;
                self.gen_rev = self.canvas.rev.wrapping_sub(1);
                let tlp = note.is_empty();
                self.name = name.trim_end_matches(".tlp").trim_end_matches(".crv").to_string();
                self.cloud_id.clear();
                self.message = if tlp { format!("Opened {name}: {} shapes, {} toolpaths", self.canvas.doc.shapes.len(), self.canvas.doc.ops.len()) } else { format!("Opened {name}: {note}") };
                if !tlp {
                    self.canvas.dirty = true; // never saved over the VCarve file: Save asks where the .tlp goes
                }
                self.offer_make_it();
                if let Some(p) = path.filter(|_| tlp) {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        self.recent = files::add_recent(&p);
                    }
                    self.project_path = Some(p);
                }
                if self.select_all_on_load {
                    self.canvas.select_all();
                }
                #[cfg(not(target_arch = "wasm32"))]
                if std::env::var("GCARVE_SELECT").is_ok_and(|v| v == "text") {
                    // dev hook: the first text's glyphs (inspector text row)
                    let t = self.canvas.doc.texts.first().map_or(-1, |t| t.id);
                    self.canvas.sel = self.canvas.doc.shapes.iter().filter(|s| s.text == t).map(|s| s.id).collect();
                    self.canvas.sel_changed();
                }
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(id) = std::env::var("GCARVE_SELECT").ok().and_then(|v| v.parse::<i32>().ok()) {
                    // dev hook: one shape by id (Qt --run canvas.selectIds([id]))
                    self.canvas.sel = [id].into_iter().collect();
                    self.canvas.sel_changed();
                }
            }
            Loaded::Imported(n, note) => {
                if self.name == "Untitled" {
                    self.name = name.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or(name.clone());
                }
                self.canvas.mutated();
                self.canvas.view = None;
                self.message = format!("{name}: {n} shapes. {note}");
            }
            Loaded::Image => match dialogs::trace::TraceDialog::new(&name, &bytes, &self.canvas.doc) {
                Ok(d) => self.cam.trace = Some(d),
                Err(e) => {
                    self.cloud.report("trace", &files::scrub(&e, &name));
                    self.message = e;
                }
            },
            Loaded::Gcode(text) => {
                if self.name == "Untitled" {
                    self.name = name.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or(name.clone());
                }
                self.cam.gimport = Some(dialogs::gimport::GcodeImportDialog::new(&name, text, &self.canvas.doc));
            }
            Loaded::Error(e) => {
                self.cloud.report("open", &files::scrub(&e, &name));
                self.message = e;
            }
        }
    }

    fn tab_with_path(&self, p: &std::path::Path) -> Option<usize> {
        if self.project_path.as_deref() == Some(p) {
            return Some(self.cur);
        }
        self.tabs.iter().position(|t| t.as_ref().is_some_and(|d| d.path.as_deref() == Some(p)))
    }

    /// (label, dirty) per tab for the strip
    fn tab_labels(&self) -> Vec<(String, bool)> {
        self.tabs.iter().map(|t| match t {
            Some(d) => (d.name.clone(), d.canvas.dirty),
            None => (self.name.clone(), self.canvas.dirty),
        }).collect()
    }
}

/// rail id of the current canvas tool (Main.qml railCurrent)
fn tool_id(t: Tool) -> &'static str {
    match t {
        Tool::Select => "tool:select",
        Tool::Nodes => "tool:nodes",
        Tool::Rect => "tool:rect",
        Tool::Ellipse => "tool:ellipse",
        Tool::Polygon => "tool:polygon",
        Tool::Star => "tool:star",
        Tool::Polyline => "tool:polyline",
        Tool::Pen => "tool:curve",
        Tool::Arc => "tool:arc",
        Tool::Measure => "tool:measure",
        Tool::Dimension => "tool:dimension",
        Tool::Text => "tool:text",
    }
}

impl App {
    /// A project saved with another machine, post or bits than this computer uses: offer to switch it (Make It).
    fn offer_make_it(&mut self) {
        if self.shot || self.canvas.doc.ops.is_empty() {
            return;
        }
        let reasons = dialogs::community::differences_here(&self.canvas.doc);
        if !reasons.is_empty() {
            self.cam.make_it = Some(dialogs::community::MakeIt::new(&self.canvas.doc, &self.name).with_reasons(reasons));
        }
    }

    fn simulating(&self) -> bool {
        self.mode == Mode::Simulate || self.show_sim
    }

    /// Which toolpaths need working out now (by index; a new one in its dialog is the last), and whether it can't
    /// wait for the edits to pause: simulating and exporting need every toolpath at once; drawing needs only the
    /// picked toolpaths and the one being set up, or all of them with View › Every Toolpath While Drawing.
    fn paths_wanted(&self, draft: Option<&(Option<usize>, gcarve_core::document::Op)>, n: usize) -> (Vec<bool>, bool) {
        let urgent = self.simulating() || self.export.is_some() || self.movie.is_some() || matches!(self.pending_dialog.as_str(), "export" | "aircut" | "setupsheet");
        if urgent || (self.canvas.show_paths && self.all_paths) {
            return (vec![true; n], urgent);
        }
        let mut w = vec![false; n];
        if self.canvas.show_paths {
            for i in self.shell.op_sel.into_iter().chain(self.shell.op_multi.iter().copied()).chain(draft.map(|d| d.0.unwrap_or(n.saturating_sub(1)))) {
                if let Some(x) = w.get_mut(i) {
                    *x = true;
                }
            }
        }
        (w, false)
    }

    /// Every toolpath of the document as it is now (export, setup sheet, Send to Control), worked out here if the
    /// previews held only some of them.
    fn full_gen(&mut self) {
        let n = self.canvas.doc.ops.len();
        if self.gen_rev == self.canvas.rev && self.gen_draft.is_none() && self.gen.is_some() && self.gen_cov.len() == n && self.gen_cov.iter().all(|c| *c) {
            return;
        }
        self.gen = Some(pipeline::regenerate(&self.canvas.doc));
        self.gen_cov = vec![true; n];
        self.gen_rev = self.canvas.rev;
        self.gen_draft = None; // an open dialog's preview is built again next frame
        self.canvas.gen_seq += 1;
    }

    fn set_mode(&mut self, m: Mode) {
        // the playback bar sits over the bottom of the canvas in Simulate: slide the view up by half its height
        // (same zoom) so a fitted board clears it, and back down again on the way out
        if (m == Mode::Simulate) != (self.mode == Mode::Simulate) {
            if let Some(v) = self.canvas.view.as_mut() {
                v.origin.y += if m == Mode::Simulate { -28.0 } else { 28.0 };
            }
        }
        self.mode = m;
        self.show_sim = false;
        match m {
            Mode::Design => self.shell.side_tab = 0,
            _ => {
                self.shell.side_tab = 2;
                self.canvas.set_tool(Tool::Select);
            }
        }
    }

    /// SpaceMouse, every frame: the 3D view orbits; the drawing pans with a slide and zooms with a lift / press.
    /// A device allowed earlier is looked for once at start.
    fn puck(&mut self, ctx: &egui::Context, in_3d: bool) {
        if !self.puck_tried {
            self.puck_tried = true;
            crate::spacemouse::connect(false);
        }
        let Some(raw) = crate::spacemouse::axes() else {
            return;
        };
        let (dt, now) = ctx.input(|i| (i.stable_dt.min(0.1), i.time));
        if raw.iter().any(|v| v.abs() > 0.02) {
            self.puck_trace.push(now, raw);
        }
        if std::mem::take(&mut self.view3d.want_trace) {
            if self.puck_trace.is_empty() {
                self.message = "No SpaceMouse movement recorded yet: move the puck, then save the trace".into();
            } else {
                files::save("gcarve-spacemouse-trace.csv", self.puck_trace.csv().into_bytes(), self.files.msg_tx.clone());
            }
        }
        let Some(a) = self.puck_filter.step(raw, dt) else {
            ctx.request_repaint_after(std::time::Duration::from_millis(50)); // the device doesn't wake the app
            return;
        };
        if in_3d {
            self.view3d.puck(a, dt, self.shell.canvas_rect.center());
        } else {
            if let Some(v) = self.canvas.view.as_mut() {
                v.origin += egui::vec2(a[0], a[1]) * 700.0 * dt;
            }
            self.zoom_by((-a[2] * dt * 1.6).exp());
        }
        ctx.request_repaint();
    }

    fn zoom_by(&mut self, k: f32) {
        let c = self.shell.canvas_rect.center();
        if let Some(v) = self.canvas.view.as_mut() {
            let k = (v.zoom * k).clamp(0.5, 5000.0) / v.zoom;
            v.origin = c + (v.origin - c) * k;
            v.zoom *= k;
        }
    }

    /// Shell command ids (Main.qml tbRun / railActivate). Unknown ids report "not available yet" in the status bar.
    fn run(&mut self, ctx: &egui::Context, id: &str) {
        let (head, arg) = id.split_once(':').unwrap_or((id, ""));
        match (head, arg) {
            ("new", _) => self.new_tab(),
            ("closetab", _) => self.close_tab(self.cur),
            ("open", _) if Cloud::web() && self.cloud.signed_in() => {
                self.projects.show(&self.name.clone());
                self.cloud.refresh(ctx);
            }
            ("open", _) => self.files.pick(ctx, false),
            // the showcase sign, built in (also on the web and signed out: nothing is uploaded)
            ("demo", _) => {
                if !self.staff {
                    self.cloud.event("demo");
                }
                self.open_picked(files::Picked { name: "GCarve demo.tlp".into(), bytes: std::env::var("GCARVE_DEMO_FILE").ok().and_then(|f| std::fs::read(f).ok()).unwrap_or_else(demo_project), path: None, import: false });
                self.cam.make_it = None; // our own demo: nothing to fit
                // a first visitor sees the carved sign, not a web of toolpath lines; key 1 goes back to the drawing
                self.set_mode(Mode::Simulate);
                self.view_3d = true;
                self.view3d.show.paths = crate::view3d::ShowPaths::Hidden; // the wood, not the lines (Show in the bar brings them back; not saved)
            }
            ("import", _) => self.files.pick(ctx, true),
            ("save", _) => {
                self.save(ctx);
            }
            ("saveas", _) if Cloud::web() && self.cloud.signed_in() => {
                self.projects.show(&self.name.clone());
                self.cloud.refresh(ctx);
            }
            ("saveas", _) => self.save_as(),
            // File ▸ Export Vectors: the selection, or every visible vector of this sheet
            ("vexport", kind) => {
                let doc = &self.canvas.doc;
                let picked = &self.canvas.sel;
                let shapes: Vec<&gcarve_core::document::Shape> = doc.shapes.iter().filter(|s| doc.shape_visible(s) && (picked.is_empty() || picked.contains(&s.id))).collect();
                if shapes.is_empty() {
                    self.message = "Nothing to export: draw or select some vectors first.".into();
                } else {
                    let items: Vec<(&gcarve_core::geom::Contour, &str)> = shapes.iter().map(|s| (&s.c, doc.layers.get(s.layer as usize).map_or("", |l| l.name.as_str()))).collect();
                    let text = if kind == "svg" { gcarve_core::svg::export_svg(&items, doc.material.w, doc.material.h) } else { gcarve_core::dxf::export_dxf(&items) };
                    match if kind == "dwg" { files::dxf_to_dwg(&text) } else { Ok(text.into_bytes()) } {
                        Ok(bytes) => files::save(&format!("{}.{kind}", self.name), bytes, self.files.msg_tx.clone()),
                        Err(e) => self.message = e,
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            ("recent", "clear") => {
                files::clear_recent();
                self.recent.clear();
            }
            #[cfg(not(target_arch = "wasm32"))]
            ("recent", i) => {
                if let Some(p) = i.parse::<usize>().ok().and_then(|i| self.recent.get(i)).cloned() {
                    self.files.open_path(std::path::Path::new(&p));
                }
            }
            ("post", pid) => {
                let doc = &mut self.canvas.doc;
                doc.checkpoint();
                doc.material.post = pid.to_string();
                self.canvas.mutated();
                self.message = format!("Post-processor: {}", self.posts.all().into_iter().find(|(p, _)| p.id == pid).map(|(p, _)| p.name).unwrap_or_else(|| pid.into()));
            }
            ("about", _) => self.about_open = true,
            ("guide", _) => self.guide.open = true,
            ("shortcuts", _) => self.guide.open_for("shortcuts"),
            ("manage", _) => self.cloud.open_web(ctx, "/dashboard/"),
            ("admin", _) => self.cloud.open_web(ctx, "/admin/"),
            ("control", what) => self.control(what),
            ("cutwidth", _) => self.canvas.show_width = !self.canvas.show_width,
            ("showsim", _) => {
                if self.mode == Mode::Simulate {
                    self.set_mode(Mode::Design);
                } else {
                    self.show_sim = !self.show_sim; // Qt View ▸ Show Simulation: the overlay, mode unchanged
                }
            }
            ("sim", "restart") => {
                self.player.pause();
                self.player.seek(0.0, ctx);
            }
            ("opedit", "") | ("opdelete", "") => match self.shell.op_sel {
                Some(i) => self.shell.push(format!("{head}:{i}")),
                None => self.message = "Select a toolpath first (Toolpaths tab)".into(),
            },
            ("opselect", v) => {
                if let Some(op) = v.parse::<usize>().ok().and_then(|i| self.canvas.doc.ops.get(i)) {
                    self.canvas.sel = self.canvas.doc.op_shape_ids(op).into_iter().collect(); // Qt selectOpShapes
                    self.canvas.sel_changed();
                }
            }
            ("savecloud", _) => self.save_to_cloud(ctx),
            ("cloud", _) => {
                if self.cloud.signed_in() { self.projects.show(&self.name.clone()); self.cloud.refresh(ctx) } else { self.account.open = true }
            }
            ("account", _) => self.account.open = true,
            ("shares", _) => {
                if self.cloud.signed_in() {
                    self.shares_open = true;
                    self.cloud.fetch_shares(ctx);
                } else {
                    self.account.open = true;
                }
            }
            ("undo", _) => self.canvas.undo(),
            ("redo", _) => self.canvas.redo(),
            ("export", _) => {
                self.full_gen();
                self.export_group = None;
                self.start_export(false);
            }
            ("exportgroup", g) => {
                self.full_gen();
                // a folder's Export button: only its toolpaths go into the files
                self.export_group = Some(g.to_string()).filter(|g| self.canvas.doc.ops.iter().any(|o| o.op_group == *g));
                self.start_export(false);
            }
            // ---- CAM dialogs / simulation ----
            ("aircut", _) => {
                self.full_gen();
                self.start_export(true);
            }
            ("setupsheet", _) => {
                self.full_gen();
                if let Some(html) = self.setup_sheet(0.0, &[]) {
                    self.message = crate::setupsheet::open(&self.name, &html);
                }
            }
            ("tools", _) => self.cam.tools = Some(dialogs::tools::ToolsDialog::new(&self.canvas.doc, 0)),
            ("posts", _) => self.cam.posts = Some(dialogs::posts::PostsDialog::default()),
            ("templates", _) => {
                self.templates.reload();
                self.cam.templates = Some(dialogs::templates::TemplatesDialog::new(&self.name));
            }
            ("board", _) => self.cam.job = Some(dialogs::job::JobDialog::new(&self.canvas.doc)),
            ("machine", _) => {
                let mut d = dialogs::job::JobDialog::new(&self.canvas.doc);
                d.browser.show(self.canvas.doc.material.machine.as_ref());
                d.machine_only = true;
                self.cam.job = Some(d);
            }
            ("reliefs" | "relief", _) => self.cam.relief = Some(dialogs::relief::ReliefDialog::default()),
            ("importstl", _) => self.files.pick_into(ctx, "3D models (.stl .3mf)", &["stl", "3mf"], "stl"),
            ("importgcode", _) => self.files.pick_into(ctx, "G-code", files::GCODE_EXTS, "gcode"),
            ("shapes", _) => self.cam.shapes = Some(dialogs::shapes::ShapesDialog::new(&self.canvas.doc)),
            ("boardphoto", "toggle") => {
                if let Some(b) = self.canvas.board_photo.as_mut() {
                    b.show = !b.show;
                }
            }
            ("boardphoto", "remove") => self.canvas.board_photo = None,
            ("boardphoto", _) => self.files.pick_into(ctx, "Photo of the board (PNG, JPEG)", files::IMAGE_EXTS, "boardphoto"),
            ("trace", _) => self.files.pick_into(ctx, "Images (.png, .jpg)", files::IMAGE_EXTS, "image"),
            ("prefs", _) => self.cam.prefs_open = true,
            ("vars", _) => self.cam.vars = Some(Default::default()),
            ("var", "add") => {
                self.canvas.add_variable();
                self.cam.vars = Some(Default::default());
            }
            ("link", _) if self.canvas.sel.len() == 2 => self.cam.constraint = Some(dialogs::vars::ConstraintDialog::new(&self.canvas)),
            ("link", _) => self.message = "Select exactly two shapes to link".into(),
            #[cfg(not(target_arch = "wasm32"))]
            ("jobs", _) => {
                self.jobs.refresh();
                self.cam.jobs = Some(dialogs::jobs::JobsDialog::default());
            }
            ("watch", _) => {
                self.show_sim = true; // Qt simulatePlayback: stays in the current mode
                self.watch_pending = true;
            }
            ("view3d", _) => {
                self.set_mode(Mode::Simulate);
                self.view_3d = !self.view_3d;
            }
            ("spacemouse", _) => crate::spacemouse::connect(true),
            ("makeit", _) => self.cam.make_it = Some(dialogs::community::MakeIt::new(&self.canvas.doc, &self.name)),
            ("community", _) => ctx.open_url(egui::OpenUrl::new_tab("https://gworkshop.app/projects/")),
            ("communityshare", _) => {
                if self.cloud.signed_in() {
                    self.cam.share = Some(dialogs::community::ShareDialog::new(&self.canvas.doc, &self.name));
                } else {
                    self.account.reason = "Sign in to share a project with the GCarve community.".into();
                    self.account.open = true;
                }
            }
            ("donate", _) => ctx.open_url(egui::OpenUrl::new_tab("https://gcarve.app/donate/")),
            ("website", _) => ctx.open_url(egui::OpenUrl::new_tab("https://gcarve.app/")),
            ("download", _) => ctx.open_url(egui::OpenUrl::new_tab("https://gcarve.app/download/")),
            ("support", _) => ctx.open_url(egui::OpenUrl::new_tab("https://gcarve.app/support/?from=app")),
            ("request", _) => self.help.open_report(&self.cloud, "suggestion"),
            ("bug", _) => self.help.open_report(&self.cloud, "bug"),
            ("update", _) => self.help.open_update(ctx),
            ("panels", _) => self.right_open = !self.right_open,
            ("fold", "right") => self.right_open = false,
            ("rail", "fold") => self.left_open = false,
            ("palette", _) => self.palette.open(ctx),
            ("mode", "Design") => self.set_mode(Mode::Design),
            ("tab", "toolpaths") => {
                self.shell.side_tab = 2; // no Toolpaths mode any more: key 2 shows the inspector's Toolpaths tab
                self.right_open = true;
            }
            ("mode", "Simulate") | ("sim", _) => self.set_mode(Mode::Simulate),
            ("tool", "text") => {
                self.canvas.set_tool(Tool::Text);
                self.dialogs.request(Request::Text(false), &self.canvas); // Qt: the Text tool opens the dialog
            }
            ("tool", t) => match Tool::ALL.iter().find(|(tool, _, _)| tool_id(*tool) == format!("tool:{t}")) {
                Some((tool, _, _)) => self.canvas.set_tool(*tool),
                None => self.message = format!("{id}: no such tool"),
            },
            // edit / vector ops (tools.rs, edit_ops.rs, popups.rs)
            ("cut", _) => self.canvas.cut(ctx),
            ("copy", _) => self.canvas.copy(ctx),
            ("paste", _) => {
                let clip = self.canvas.clip.clone();
                self.canvas.paste(&clip);
            }
            ("clone", _) => self.canvas.duplicate(),
            ("selectall", _) => self.canvas.select_all(),
            ("group", "") => self.canvas.group_selection(),
            ("ungroup", _) => self.canvas.ungroup_selection(),
            ("group", "toggle") if self.canvas.selection_has_group() => self.canvas.ungroup_selection(),
            ("group", "toggle") => self.canvas.group_selection(),
            ("fliph", _) => self.canvas.flip(true),
            ("flipv", _) => self.canvas.flip(false),
            ("mirror", v) => self.canvas.mirror_copy(v != "up"),
            ("rotate", _) => self.canvas.rotate90(),
            ("centre", _) => self.canvas.align_selection("center"),
            ("weld", _) => self.canvas.weld_selection(),
            ("subtract", _) => self.canvas.subtract_selection(),
            ("intersect", _) => self.canvas.intersect_selection(),
            ("join", _) => self.canvas.join_selection(),
            ("validate", _) => self.canvas.check_vectors(),
            ("extend", _) => self.canvas.extend_selection(),
            ("simplify", v) => self.canvas.simplify_selection(v.parse().unwrap_or(0.005)),
            ("close", _) => self.canvas.close_selection(),
            ("offset", _) => self.dialogs.request(Request::Offset, &self.canvas),
            ("dogbone", _) => self.dialogs.request(Request::Dogbone, &self.canvas),
            ("align", _) => self.dialogs.request(Request::Align, &self.canvas),
            ("array", _) => self.dialogs.request(Request::Array, &self.canvas),
            ("nest", _) => self.dialogs.request(Request::Nest, &self.canvas),
            ("text", "curve") => self.dialogs.request(Request::Text(true), &self.canvas),
            ("text", "edit") => {
                let t = self.canvas.selected_text();
                if t != 0 { self.dialogs.request(Request::EditText(t), &self.canvas) }
            }
            ("text", "curves") => self.canvas.convert_text_to_curves(),
            ("paths", v) => {
                // "" toggles; off / picked / all = the Toolpaths tab's Show switch
                (self.show_paths, self.all_paths) = match v {
                    "off" => (false, self.all_paths),
                    "picked" => (true, false),
                    "all" => (true, true),
                    _ => (!self.show_paths, self.all_paths),
                };
                self.canvas.show_paths = self.show_paths;
                crate::store::save("show-paths", if self.show_paths { "1" } else { "0" });
                crate::store::save("design-all-paths", if self.all_paths { "1" } else { "0" });
            }
            ("allpaths", _) => {
                self.all_paths = !self.all_paths;
                crate::store::save("design-all-paths", if self.all_paths { "1" } else { "0" });
            }
            ("allsheets", _) => {
                self.all_sheets = !self.all_sheets;
                crate::store::save("all-sheets", if self.all_sheets { "1" } else { "0" });
                self.canvas.all_sheets = self.all_sheets;
                self.canvas.zoom_fit();
                self.message = if !self.all_sheets {
                    "One sheet at a time: the tabs below switch sheets".into()
                } else if self.canvas.doc.sheets.len() > 1 {
                    "All sheets: click a sheet to work on it, drag vectors onto another sheet to move them there".into()
                } else {
                    "All sheets: add a sheet (+ by the tabs) and they show side by side".into()
                };
            }
            ("width", _) => self.canvas.show_width = !self.canvas.show_width,
            ("startcard", "close") => self.canvas.card_closed = true,
            ("guides", "clear") => {
                self.canvas.doc.guides.clear();
                self.canvas.touch();
            }
            ("grid", "toggle") => self.cam.prefs.synced.grid.show ^= true,
            ("rulers", "toggle") => {
                self.cam.prefs.synced.grid.rulers ^= true;
                self.canvas.grid.rulers = self.cam.prefs.synced.grid.rulers;
                self.canvas.view = None; // refit beside / without the rulers
            }
            ("snap", "settings") => egui::Popup::open_id(ctx, shell::snap_popup()),
            ("startcard", _) => {
                self.cam.prefs.start_card = !self.cam.prefs.start_card;
                self.cam.prefs.save();
            }
            ("ops", v) => {
                for op in &mut self.canvas.doc.ops {
                    op.visible = v == "showall";
                }
                self.canvas.rev += 1;
            }
            ("delete", _) => self.canvas.delete_selected(),
            ("select", v) => {
                self.canvas.sel = v.parse().into_iter().collect();
                self.canvas.sel_changed();
            }
            ("zoom", "in") => self.zoom_by(1.25),
            ("zoom", "out") => self.zoom_by(0.8),
            ("zoom", "board") => {
                // not `view = None`: an unset view fits the shapes first
                let r = self.canvas.last_rect;
                if r.is_positive() { self.canvas.zoom_board(r) } else { self.canvas.view = None }
            }
            ("zoom", "fit") => self.canvas.zoom_fit(),
            ("zoom", v) => {
                if let (Some(view), Ok(pct)) = (self.canvas.view.as_ref(), v.parse::<f32>()) {
                    let k = pct / 100.0 * 96.0 / view.zoom;
                    self.zoom_by(k);
                }
            }
            ("sheet", "add") => {
                let doc = &mut self.canvas.doc;
                doc.checkpoint();
                doc.sync_sheet();
                doc.sheets.push(gcarve_core::document::Material { name: String::new(), ..doc.material.clone() }); // a new sheet starts unnamed
                doc.set_active_sheet(doc.sheets.len() as i32 - 1);
                self.canvas.touch();
            }
            // Two-sided job: a new sheet with the same board and a mirrored copy of the vectors (the selection, or
            // everything on this sheet), so cuts made after flipping the board line up with the first side.
            ("sheet", v @ ("back-lr" | "back-tb")) => {
                let lr = v == "back-lr";
                let doc = &mut self.canvas.doc;
                doc.checkpoint();
                doc.sync_sheet();
                let from = doc.active_sheet;
                let (w, h) = (doc.material.w, doc.material.h);
                let flip = if lr { gcarve_core::geom::Affine { a: -1.0, d: 1.0, e: w, ..Default::default() } } else { gcarve_core::geom::Affine { a: 1.0, d: -1.0, f: h, ..Default::default() } };
                let picked = &self.canvas.sel;
                let copies: Vec<(gcarve_core::geom::Contour, i32)> = doc.shapes.iter().filter(|s| s.sheet == from && (picked.is_empty() || picked.contains(&s.id))).map(|s| (gcarve_core::geom::transformed(&s.c, &flip), s.layer)).collect();
                doc.sheets.push(gcarve_core::document::Material { name: String::new(), ..doc.material.clone() }); // a new sheet starts unnamed
                doc.set_active_sheet(doc.sheets.len() as i32 - 1);
                let n = copies.len();
                self.canvas.sel = copies.into_iter().map(|(c, layer)| doc.add(c, Some(layer))).collect();
                self.canvas.mutated();
                self.message = format!("Back side = Sheet {} ({n} mirrored). Cut side 1, flip the board {}, cut this sheet.", self.canvas.doc.sheets.len(), if lr { "left to right" } else { "top to bottom" });
            }
            ("sheet", "remove") => {
                let doc = &mut self.canvas.doc;
                if doc.sheets.len() > 1 {
                    let i = doc.active_sheet;
                    doc.checkpoint();
                    let to = (i - 1).max(0);
                    for s in &mut doc.shapes {
                        if s.sheet == i { s.sheet = to } else if s.sheet > i { s.sheet -= 1 }
                    }
                    doc.sheets.remove(i as usize);
                    doc.set_active_sheet(to);
                    self.canvas.touch();
                }
            }
            ("sheetname", v) => {
                // "<sheet>:<name>" from the sheet tabs (empty name = back to "Sheet N")
                if let Some((j, name)) = v.split_once(':').and_then(|(j, n)| j.parse::<usize>().ok().map(|j| (j, n))) {
                    if j < self.canvas.doc.sheets.len().max(1) && self.canvas.doc.sheet_name(j) != name {
                        self.canvas.doc.checkpoint();
                        self.canvas.doc.set_sheet_name(j, name);
                        self.canvas.touch();
                    }
                }
            }
            ("sheet", "next") => {
                let doc = &mut self.canvas.doc;
                if !self.canvas.sel.is_empty() && doc.sheets.len() > 1 {
                    let to = (doc.active_sheet + 1) % doc.sheets.len() as i32;
                    doc.checkpoint();
                    for id in self.canvas.sel.clone() {
                        if let Some(s) = doc.find_mut(id) { s.sheet = to }
                    }
                    self.canvas.mutated();
                }
            }
            ("sheet", v) => {
                if let Ok(i) = v.parse::<i32>() {
                    if self.canvas.showing_all_sheets() && (0..self.canvas.doc.sheets.len() as i32).contains(&i) {
                        self.canvas.switch_sheet_keep_view(i as usize); // all sheets shown: nothing moves on screen
                        return;
                    }
                    self.canvas.doc.set_active_sheet(i);
                    self.canvas.sel.clear();
                    self.canvas.apply_bindings(); // thickness / board-w are this sheet's (Qt switchSheet)
                    self.canvas.mutated();
                }
            }
            ("layersel", v) => {
                if let Ok(i) = v.parse::<i32>() {
                    self.canvas.select_layer_shapes(i);
                    self.message = format!("Selected {} vector(s) on the layer", self.canvas.sel.len());
                }
            }
            ("layerop", v) => {
                // "layer:kind": a toolpath for the whole layer
                let mut it = v.split(':').map(|x| x.parse::<usize>().ok());
                if let (Some(Some(l)), Some(Some(k))) = (it.next(), it.next()) {
                    if let Some(&kind) = inspector::KIND_TYPES.get(k) {
                        self.cam.op = Some(dialogs::op::OpDialog::new_for_layer(&self.canvas.doc, kind, l as i32));
                    }
                }
            }
            ("layer", "add") => self.canvas.add_layer(),
            ("layer", "remove") => self.canvas.remove_layer(self.canvas.doc.active_layer),
            ("kind", v) => {
                let k = v.parse::<usize>().unwrap_or(0);
                let sel: Vec<i32> = self.canvas.sel.iter().copied().collect();
                if let Some(&kind) = inspector::KIND_TYPES.get(k) {
                    self.cam.op = Some(dialogs::op::OpDialog::new(&self.canvas.doc, kind, &sel));
                }
            }
            ("opedit", v) => {
                if let Some(i) = v.parse::<usize>().ok().filter(|&i| i < self.canvas.doc.ops.len()) {
                    let sel: Vec<i32> = self.canvas.sel.iter().copied().collect();
                    self.cam.op = Some(dialogs::op::OpDialog::edit(&self.canvas.doc, i, &sel));
                }
            }
            ("opdelete", v) => {
                if let Ok(i) = v.parse::<usize>() {
                    if i < self.canvas.doc.ops.len() {
                        self.canvas.doc.checkpoint();
                        self.canvas.doc.ops.remove(i);
                        self.shell.op_sel = None;
                        self.canvas.touch();
                    }
                }
            }
            _ => self.message = format!("{id}: not available yet in this build"),
        }
    }
}

impl eframe::App for App {
    /// a normal quit (unsaved work was asked about) leaves no recovery copy behind; a crash does
    #[cfg(not(target_arch = "wasm32"))]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if !self.shot {
            crate::recover::clear();
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        self.canvas.bottom_inset = if self.mode == Mode::Simulate { 56.0 } else { 0.0 };
        #[cfg(target_arch = "wasm32")]
        self.smoke_test(&ctx);
        // anonymous usage counts: the app was opened; a first toolpath was added (the toolpath dialog closed with one more)
        // (a few seconds in, once the sign-in check has answered: the owner's and admins' own visits don't count)
        // the last run ended in a crash: tell the error log once (desktop; the report needs a signed-in account)
        #[cfg(not(target_arch = "wasm32"))]
        if self.cloud.checked && self.cloud.signed_in() {
            if let Some(c) = crate::store::load("crash.txt") {
                self.cloud.report("crash", c.trim());
                crate::store::remove("crash.txt");
            }
        }
        if !self.counted_open && ctx.input(|i| i.time) > 4.0 {
            self.counted_open = true;
            self.staff = self.cloud.user.as_ref().is_some_and(|u| u.role == "admin" || u.role == "superadmin");
            if !self.staff {
                self.cloud.event("open");
            }
        }
        if self.staff {
            self.counted_toolpath = true; // nothing else of theirs is counted either
        }
        // phones and narrow windows: start with the side panel folded so the drawing gets the width (the arrow at
        // its edge opens it); once per start, the user's own choice stands after that
        if self.frame == 2 && ctx.content_rect().width() < 760.0 {
            self.right_open = false;
            self.left_open &= ctx.content_rect().width() >= 600.0; // phones: the tool column folds too
            self.canvas.view = None; // fit the board into the wider canvas
        }
        let n_ops = self.canvas.doc.ops.len();
        if self.cam.op.is_some() {
            self.ops_before = Some(n_ops);
        } else if let Some(before) = self.ops_before.take() {
            if n_ops > before && !std::mem::replace(&mut self.counted_toolpath, true) {
                let kind = self.canvas.doc.ops.last().map_or(String::new(), |o| format!("{:?}", o.kind));
                self.cloud.event_detail("toolpath", &kind);
            }
        }
        #[cfg(all(feature = "screenshot", not(target_arch = "wasm32")))]
        crate::shot::tick(&ctx, self.frame);
        #[cfg(all(feature = "screenshot", not(target_arch = "wasm32")))]
        self.movie_tick(&ctx);
        self.canvas.show_paths = self.show_paths; // a new or switched-to drawing gets this device's choice
        let draft = self.cam.op.as_ref().and_then(|d| d.draft(&self.canvas.doc));
        let now = ctx.input(|i| i.time);
        if self.edit_seen.0 != self.canvas.rev || self.edit_seen.1 != draft {
            self.edit_seen = (self.canvas.rev, draft.clone());
            self.edit_at = now;
        }
        // only what is on screen is worked out, and while drawing only once the edits pause
        let n = self.canvas.doc.ops.len() + usize::from(matches!(draft, Some((None, _))));
        let (mut want, urgent) = self.paths_wanted(draft.as_ref(), n);
        let fresh = self.gen_rev == self.canvas.rev && draft == self.gen_draft;
        if fresh {
            for (w, c) in want.iter_mut().zip(&self.gen_cov) {
                *w |= *c; // keep what is already worked out
            }
        }
        let gen_due = want.iter().enumerate().any(|(i, &w)| w && !(fresh && self.gen_cov.get(i).copied().unwrap_or(false)));
        let pause = if self.shot { 0.0 } else { GEN_PAUSE - (now - self.edit_at) };
        self.canvas.all_paths = self.all_paths;
        self.canvas.paths_old = !fresh;
        self.canvas.paths_only = (!(urgent || self.all_paths)).then(|| want.clone());
        // a big drawing can take seconds (and the web build has no second thread): draw a card for one frame first
        let heavy = self.canvas.doc.shapes.len() > 150 && !self.canvas.doc.ops.is_empty();
        if gen_due && !urgent && pause > 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(pause));
        } else if gen_due && heavy && !self.gen_wait {
            self.gen_wait = true;
            ctx.request_repaint();
        } else if gen_due {
            self.gen_wait = false;
            // while a toolpath dialog is open its unsaved settings show on the canvas
            self.gen = Some(match &draft {
                Some((i, op)) => {
                    let mut doc = self.canvas.doc.clone();
                    match i.filter(|&i| i < doc.ops.len()) {
                        Some(i) => doc.ops[i] = op.clone(),
                        None => doc.ops.push(op.clone()),
                    }
                    pipeline::regenerate_some(&doc, &|i| want[i])
                }
                None => pipeline::regenerate_some(&self.canvas.doc, &|i| want[i]),
            });
            self.gen_cov = want;
            self.canvas.paths_old = false;
            if let (Some(g), Some((None, op))) = (self.gen.as_mut(), &draft) {
                g.draft_tool = op.tool;
            }
            self.gen_rev = self.canvas.rev;
            self.gen_draft = draft;
            self.canvas.gen_seq += 1;
        }
        if self.gen_wait {
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("gen-wait"));
            let p = ctx.layer_painter(layer);
            let card = egui::Rect::from_center_size(ctx.content_rect().center(), egui::vec2(280.0, 56.0));
            p.rect_filled(card, 8.0, theme::PANEL);
            p.text(card.center(), egui::Align2::CENTER_CENTER, "Generating toolpaths…", theme::font(14.0), theme::TEXT);
        }
        // /app/?share=1: 1 = start, 2 = sign-in box shown, 3 = project list shown (waiting for a project to open)
        if self.share_start > 0 && self.cloud.checked {
            if !self.cloud.signed_in() {
                if self.share_start == 1 {
                    self.account.reason = "Sign in to share a project with the GCarve community.".into();
                    self.account.open = true;
                    self.share_start = 2;
                }
            } else if !self.canvas.doc.ops.is_empty() {
                self.share_start = 0;
                self.cam.share = Some(dialogs::community::ShareDialog::new(&self.canvas.doc, &self.name));
            } else if self.share_start < 3 {
                if !self.account.open {
                    self.projects.show(&self.name.clone());
                    self.cloud.refresh(&ctx);
                    self.message = "Pick the project to share: it opens, then the Share box appears. A file on your computer? File › Open it first.".into();
                    self.share_start = 3;
                }
            } else if !self.projects.open && self.cloud.busy == 0 && self.opening.is_empty() {
                self.share_start = 0; // closed without opening one
                self.message = "Open the project you want to share, then File › Share to Community…".into();
            }
        }
        // a project handed over in the address (GSketch › Send to GCarve): open it as soon as the account is known
        if let Some(id) = self.open_at_start.clone() {
            if self.cloud.signed_in() {
                self.open_at_start = None;
                self.cloud.open_project(&ctx, &id);
            } else if self.cloud.checked {
                self.open_at_start = None;
                self.account.reason = "Sign in to open the project GSketch sent over.".into();
                self.account.open = true;
            }
        }
        for ev in self.cloud.poll(&ctx) {
            match ev {
                Reply::Saved { id, name } => {
                    self.cloud_id = id;
                    self.name = name.clone();
                    self.canvas.dirty = false;
                    self.message = format!("Saved to the cloud: {name}");
                }
                Reply::Prefs(Some(p)) => {
                    // account copy wins when it has one; an empty account gets the local settings
                    if p.as_object().is_some_and(|o| o.is_empty()) {
                        self.cloud.push_prefs(&ctx, serde_json::to_value(&self.cam.prefs.synced).unwrap_or_default());
                    } else {
                        self.cam.prefs.synced = self.cam.prefs.synced.merged(&p);
                        self.prefs_from_server = true;
                    }
                }
                Reply::Templates(list) => self.templates.on_list(list.into_iter().map(|p| p.id).collect()),
                Reply::TemplateData { id, json } => self.templates.on_data(id, &json),
                Reply::TemplatesChanged => self.templates.reload(),
                Reply::Library { bits, machines } => {
                    crate::library::merge_from_account(bits.as_ref(), machines.as_ref());
                    self.lib_ready = true; // new projects start with the merged library
                }
                Reply::Shares(l) => {
                    self.shares_open |= !l.is_empty() && self.shares.is_empty(); // new arrivals open the window once
                    self.shares = l;
                }
                Reply::Shared(m) => self.message = m,
                Reply::ShareTaken { kind, name, data } => {
                    if let Some(d) = data {
                        self.message = crate::library::accept(&kind, &name, &d);
                        if kind == "bits" {
                            // also into the open project, so the Tool Library shows them now
                            self.canvas.doc.checkpoint();
                            crate::toollib::merge(&mut self.canvas.doc.tools, crate::library::bits_from(&d));
                            self.canvas.touch();
                        }
                    }
                    self.cloud.fetch_shares(&ctx);
                }
                Reply::RequestSent(r) => {
                    self.help.on_reply(r);
                }
                Reply::CommunityShared(r) => {
                    if let Some(d) = self.cam.share.as_mut() {
                        d.answered(r);
                    }
                }
                Reply::CommunityOpened { slug, title, json } => match project::load_project(&json) {
                    Ok(mut d) => {
                        dialogs::relief::rehydrate(&mut d);
                        dialogs::community::use_my_machine(&mut d);
                        self.open_target();
                        self.canvas = Canvas::with_doc(d);
                        self.canvas.hit_scale = self.cam.prefs.hit_scale();
                        self.canvas.apply_bindings();
                        self.canvas.dirty = true; // someone else's project: saving makes it a copy of your own
                        self.gen_rev = self.canvas.rev.wrapping_sub(1);
                        self.project_path = None;
                        self.cloud_id = String::new();
                        self.name = title.clone();
                        let reasons = dialogs::community::differences_here(&self.canvas.doc);
                        self.cam.make_it = Some(dialogs::community::MakeIt::new(&self.canvas.doc, &title).with_reasons(reasons));
                        self.message = format!("Opened “{title}” from gworkshop.app/projects/{slug}/");
                        self.cloud.event_detail("community_open", &slug);
                    }
                    Err(e) => {
                        self.cloud.report("community open", &e);
                        self.message = e;
                    }
                },
                Reply::Opened { id, name, json } => match project::load_project(&json) {
                    Ok(mut d) => {
                        dialogs::relief::rehydrate(&mut d);
                        self.open_target();
                        self.canvas = Canvas::with_doc(d);
                        self.canvas.hit_scale = self.cam.prefs.hit_scale();
                        self.canvas.apply_bindings();
                        self.gen_rev = self.canvas.rev.wrapping_sub(1);
                        self.project_path = None;
                        self.cloud_id = id;
                        self.name = name.clone();
                        self.message = format!("Opened {name} from the cloud");
                        self.offer_make_it();
                    }
                    Err(e) => {
                        self.cloud.report("cloud open", &e);
                        self.message = e;
                    }
                },
                _ => {}
            }
        }
        self.sync_prefs(&ctx);
        self.recover_tick(&ctx);
        // toolpath templates: in the account on the web when signed in, else local (templates.rs)
        let account = Cloud::web() && self.cloud.signed_in();
        if account != self.templates.account || self.frame == 1 {
            self.templates.account = account;
            self.templates.reload();
        }
        for op in std::mem::take(&mut self.templates.pending) {
            self.cloud.template_op(&ctx, op);
        }
        if self.dev_measure && self.frame == 5 {
            self.canvas.set_tool(Tool::Measure);
            self.canvas.measure = vec![gcarve_core::geom::pt(1.5, 1.0), gcarve_core::geom::pt(9.25, 5.5)];
        }
        if self.canvas.tool != self.prev_tool {
            if self.canvas.tool == Tool::Measure {
                self.shell.side_tab = 0;
                self.right_open = true;
            }
            self.prev_tool = self.canvas.tool;
        }
        // the web app is open to everyone; bringing files in (open, import, drop, trace…) needs a free account
        self.files.locked.set(Cloud::web() && !self.cloud.signed_in() && !self.cloud.local());
        if self.files.wants_account.take() {
            self.account.reason = "Create a free account (or sign in) to open and import files. Drawing, toolpaths, the simulation and G-code export work without one.".into();
            self.account.open = true;
        }
        if std::mem::take(&mut self.cloud.expired) {
            self.account.reason = "You were signed out after a while without use. Sign in again to save or open projects; your drawing is still here.".into();
            self.account.open = true;
        }
        // no desktop licence gate: the desktop apps are free for everyone (2026-10-16), signing in is optional
        // the site can require an account for the web app too (admin overview ▸ Web app access): the same blocking box
        if Cloud::web() && self.cloud.require_signin && !self.shot && !self.cloud.local() && self.cloud.checked && !self.cloud.signed_in() && !self.account.gate {
            self.account.gate = true;
            self.account.open = true;
        }
        for m in self.files.msg_rx.try_iter() {
            if !m.is_empty() {
                self.message = m;
            }
        }
        crate::catalog::prefetch(&ctx); // web: the bit & machine database comes from the site (once)
        // opening a big file (a VCarve project, a large drawing) can take seconds, in the browser especially: the
        // "Opening…" card goes on screen for one frame first, then the file is read
        self.opening.extend(self.files.poll(&ctx));
        if !self.opening.is_empty() {
            let big = self.opening.iter().any(|f| f.bytes.len() > 300_000);
            if !big || std::mem::replace(&mut self.open_wait, true) {
                for f in std::mem::take(&mut self.opening) {
                    self.open_picked(f);
                }
                self.open_wait = false;
            } else {
                ctx.request_repaint();
            }
        }
        if self.open_wait {
            let name = self.opening.first().map_or(String::new(), |f| f.name.clone());
            busy_card(&ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("opening"))), ctx.content_rect(), &format!("Opening {name}…"), None);
        }
        for (doc, path) in self.files.saved_rx.try_iter().collect::<Vec<_>>() {
            self.saved_as(doc, path, &ctx);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.frame >= 2 && self.pending_dialog == "setupsheet" && self.gen.is_some() {
            self.pending_dialog.clear(); // dev: GCARVE_DIALOG=setupsheet writes the sheet (no browser) and prints its path
            let path = std::env::temp_dir().join("gcarve-shot-setup.html");
            if let Some(html) = self.setup_sheet(0.0, &[]) {
                let _ = std::fs::write(&path, html);
                println!("setup sheet: {}", path.display());
            }
        }
        if self.frame >= 2 && matches!(self.pending_dialog.as_str(), "export" | "aircut") && self.gen.is_some() {
            let air = std::mem::take(&mut self.pending_dialog) == "aircut"; // dev: GCARVE_DIALOG=export|aircut, once paths exist
            self.start_export(air);
        }
        // GCARVE_BOARD_PHOTO=photo.png:x1,y1,x2,y2,x3,y3,x4,y4 (corners in photo pixels, BL BR TR TL): dev, screenshots;
        // after the file given on the command line has opened
        #[cfg(not(target_arch = "wasm32"))]
        if self.frame == 4 {
            if let Ok(j) = std::env::var("GCARVE_SHEET") {
                self.shell.push(format!("sheet:{j}")); // dev: screenshots of another sheet (0-based)
            }
            if let Some((f, c)) = std::env::var("GCARVE_BOARD_PHOTO").ok().and_then(|v| v.split_once(':').map(|(a, b)| (a.to_string(), b.to_string()))) {
                let n: Vec<f32> = c.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if let (Ok(bytes), 8) = (std::fs::read(&f), n.len()) {
                    self.canvas.board_photo = crate::dialogs::boardphoto::dev_photo(&bytes, &n, &self.canvas.doc);
                }
            }
        }
        if self.frame == 2 && !self.pending_dialog.is_empty() && !matches!(self.pending_dialog.as_str(), "export" | "aircut" | "setupsheet") {
            let d = std::mem::take(&mut self.pending_dialog);
            if d == "snap" {
                egui::Popup::open_id(&ctx, shell::snap_popup()); // dev: the status-bar Snap popover
            }
            if matches!(d.as_str(), "update" | "bug" | "donate") {
                self.shell.push(&d); // dev: the Help dialogs (GCARVE_VERSION=0.1.0 shows "Update available")
            } else {
                self.cam.open_named(if d == "prefs-newdoc" { "prefs" } else { &d }, &self.canvas);
            }
        }
        // quitting with unsaved tabs: ask for each (Qt onClosing)
        if ctx.input(|i| i.viewport().close_requested()) && !self.shot {
            if let Some(i) = (0..self.tabs.len()).find(|&i| if i == self.cur { self.canvas.dirty } else { self.tabs[i].as_ref().is_some_and(|d| d.canvas.dirty) }) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.switch_tab(i);
                self.confirm = Some(Pending::Quit);
            }
        }
        self.shortcuts(&ctx);
        self.canvas.keys(&ctx); // tool keys, delete, undo/redo, nudge, clipboard (canvas.rs)
        if !self.canvas.status.is_empty() {
            self.message = std::mem::take(&mut self.canvas.status);
        }

        let has_ops = !self.canvas.doc.ops.is_empty();
        let menu = shell::MenuInfo {
            account: if self.cloud.signed_in() { format!("Account ({})…", self.cloud.user.as_ref().map(|u| u.email.as_str()).unwrap_or("")) } else { "Sign In…".to_string() },
            signed_in: self.cloud.signed_in(),
            admin: self.cloud.user.as_ref().is_some_and(|u| u.role == "admin" || u.role == "superadmin"),
            recent: self.recent.iter().map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()).collect(),
            sim: self.simulating(),
            view_3d: self.view_3d,
            start_card: self.cam.prefs.start_card,
            show_grid: self.cam.prefs.synced.grid.show,
            rulers: self.cam.prefs.synced.grid.rulers,
        };
        shell::menu_bar(ui, &mut self.shell, &mut self.canvas, &menu, &mut self.left_open, &mut self.right_open, has_ops);
        let dirty = if self.canvas.dirty { "• " } else { "" };
        #[cfg(not(target_arch = "wasm32"))]
        let job = self.project_path.as_ref().and_then(|p| self.jobs.job_for(p)).and_then(|j| j.file_name().map(|n| format!("{} › ", n.to_string_lossy()))).unwrap_or_default();
        #[cfg(target_arch = "wasm32")]
        let job = String::new();
        let title = if self.canvas.doc.sheets.len() > 1 { format!("{dirty}{job}{} · {}", self.name, self.canvas.doc.sheet_name(self.canvas.doc.active_sheet.max(0) as usize)) } else { format!("{dirty}{job}{}", self.name) };
        let window_title = format!("{dirty}{} — GCarve", self.name);
        if window_title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(window_title.clone()));
            self.title = window_title;
        }
        // previews may hold only some toolpaths while drawing: Export works them all out
        let can_export = !self.canvas.doc.ops.is_empty();
        shell::mode_bar(ui, &mut self.shell, self.mode, &title, &self.canvas, can_export);

        egui::Panel::bottom("status").frame(egui::Frame::new().fill(theme::BG)).show(ui, |ui| {
            let message = if self.simulating() && self.player.ready() { self.player.info.clone() } else { self.message.clone() };
            shell::status_bar(ui, &mut self.shell, &mut self.canvas, &mut self.cam.prefs.synced, &message, &self.status, has_ops);
        });

        let sel = self.canvas.selection_count();
        if self.left_open {
            egui::Panel::left("rail").resizable(false).exact_size(82.0).frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin { left: 4, right: 5, top: 4, bottom: 4 })).show(ui, |ui| {
                let tool = self.canvas.tool;
                let paths = self.canvas.show_paths;
                let canvas = &mut self.canvas;
                shell::rail(ui, &mut self.shell, self.mode, tool_id(tool), sel, paths, |ui| crate::tools::rail_count(ui, canvas));
            });
        } else {
            egui::Panel::left("rail-folded").resizable(false).exact_size(18.0).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| {
                if shell::folded(ui, true, "the tool rail", "") {
                    self.left_open = true;
                }
            });
        }

        if self.right_open {
            egui::Panel::right("inspector").default_size(310.0).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| {
                inspector::inspector(ui, &mut self.shell, &mut self.canvas, self.gen.as_ref());
            });
        } else {
            egui::Panel::right("inspector-folded").resizable(false).exact_size(18.0).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| {
                if shell::folded(ui, false, "the side panel", "⇧⌘P") {
                    self.right_open = true;
                }
            });
        }

        // ---- CAM: simulation playback (SimBar), 2D / 3D view ----
        let simulating = self.simulating();
        if simulating {
            if let Some(g) = &self.gen {
                // new toolpaths: draw one frame with the "Preparing" screen first, then build (it can take a moment)
                if self.player.rev != self.gen_rev && !std::mem::replace(&mut self.sim_wait, true) {
                    ctx.request_repaint();
                } else {
                    self.sim_wait = false;
                    self.player.update(&ctx, &self.canvas.doc, g, self.gen_rev, !self.watch_pending); // gen_rev: the paths `g` were built from
                }
            }
            self.player.finish_step(&ctx);
            if self.watch_pending && self.player.ready() {
                self.watch_pending = false;
                self.player.seek(0.0, &ctx);
                self.player.play();
            }
            self.player.tick(&ctx);
        }
        egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
            if self.tabs.len() > 1 {
                let labels = self.tab_labels();
                let mut act = None;
                egui::Panel::top("doctabs").resizable(false).exact_size(30.0).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| {
                    act = tabs::strip(ui, &labels, self.cur);
                });
                match act {
                    Some(TabAct::Switch(i)) => self.switch_tab(i),
                    Some(TabAct::Close(i)) => self.close_tab(i),
                    Some(TabAct::New) => self.new_tab(),
                    None => {}
                }
            }
            egui::Panel::bottom("sheets").resizable(false).exact_size(28.0).frame(egui::Frame::new().fill(theme::PANEL)).show(ui, |ui| {
                shell::sheet_tabs(ui, &mut self.shell, &self.canvas.doc, self.all_sheets);
            });
            egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                self.shell.canvas_rect = rect;
                if simulating && self.view_3d {
                    self.view3d.ui(ui, &self.player, self.canvas.show_paths);
                    self.puck(ui.ctx(), true);
                    self.status = format!("yaw {:.0}°  pitch {:.0}°  drag: orbit", self.view3d.yaw, self.view3d.pitch); // fits the 230 px slot
                } else {
                    // while simulating, the Show menu decides: all toolpath previews, or the moves cut so far drawn
                    // over the carved board (the tool's trail), or none
                    let show = self.view3d.show;
                    let previews = self.canvas.show_paths;
                    if simulating && show.paths != crate::view3d::ShowPaths::All {
                        self.canvas.show_paths = false;
                    }
                    self.canvas.all_sheets = self.all_sheets && !simulating; // the simulation is of the active sheet
                    self.status = self.canvas.ui(ui, self.gen.as_ref(), simulating.then_some(&self.player.view));
                    self.canvas.show_paths = previews;
                    self.puck(ui.ctx(), false);
                    if simulating && self.player.ready() {
                        if let Some(v) = &self.canvas.view {
                            let painter = ui.painter().with_clip_rect(rect);
                            if show.paths == crate::view3d::ShowPaths::Trail && previews {
                                crate::view3d::draw_moves(&painter, &self.player, show, |x, y, _| v.to_screen(gcarve_core::geom::pt(x, y)));
                            }
                            if show.cutter {
                                let (x, y) = self.player.tool_pos;
                                let c = v.to_screen(gcarve_core::geom::pt(x, y));
                                let r = (self.player.tool_dia as f32 / 2.0 * v.zoom).max(3.0);
                                painter.circle_stroke(c, r, egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 211, 77)));
                                painter.circle_filled(c, 2.0, egui::Color32::from_rgb(255, 211, 77));
                            }
                        }
                    }
                }
                if simulating {
                    self.sim_bar(ui);
                }
                // building the simulation can take a few seconds on a big job: say so, with how far it got
                let busy = if self.sim_wait { Some(0.0) } else { self.player.preparing() };
                if let (true, Some(f)) = (simulating, busy) {
                    busy_card(ui.painter(), rect, "Preparing the simulation…", Some(f));
                }
                if self.canvas.doc.shapes.is_empty() && self.canvas.doc.reliefs.is_empty() && !self.canvas.dirty && !simulating && self.cam.prefs.start_card && !self.canvas.card_closed {
                    let recent: Vec<String> = self.recent.iter().take(5).cloned().collect();
                    shell::empty_state(ui, &mut self.shell, rect, &self.canvas.doc, &recent);
                    // Esc closes the card like its ×, unless a dialog or a field has the keyboard
                    if !ctx.egui_wants_keyboard_input() && ctx.memory(|m| m.top_modal_layer().is_none()) && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.canvas.card_closed = true;
                    }
                }
            });
        });
        if self.palette.open {
            let posts: Vec<(String, String)> = self.posts.all().into_iter().map(|(p, _)| (p.id, p.name)).collect();
            let vars: Vec<(String, String)> = self.canvas.doc.variables.iter().map(|v| (v.name.clone(), v.expr.clone())).collect();
            let cmds = palette::commands(&posts, &self.canvas.doc.material.post, &vars);
            if let Some(id) = self.palette.ui(&ctx, &cmds, "Type a command… e.g. v-carve, nest, board, mach3") {
                self.shell.push(id);
            }
        }
        self.confirm_ui(&ctx);
        self.recover_ui(&ctx);
        self.about_ui(&ctx);
        if let Some(cmd) = self.guide.ui(&ctx, dialogs::guide::TOPICS, "Search: corners, long board, PDF, order…") {
            self.shell.push(cmd); // the answer's "Do it" button
        }
        // (a narrow window folds the side panel by itself: that is not the user's choice and is not remembered)
        if (self.left_open, self.right_open) != (self.ui_state.tool_rail, self.ui_state.side_panel) && ctx.content_rect().width() >= 760.0 {
            self.ui_state = UiState { tool_rail: self.left_open, side_panel: self.right_open };
            if !self.shot {
                crate::store::save("ui.json", &serde_json::to_string(&self.ui_state).unwrap_or_default());
            }
        }
        let shell = &mut self.shell;
        self.canvas.requests.retain(|r| if let Request::Cmd(id) = r { shell.push(*id); false } else { true }); // context-menu commands
        self.dialogs.ui(&ctx, &mut self.canvas); // text / offset / dogbone / array / align / nest (popups.rs)
        if std::mem::take(&mut self.dialogs.text.want_font) {
            self.files.pick_into(&ctx, "Fonts (.ttf, .otf)", &["ttf", "otf"], "font");
        }
        self.cam_ui(&ctx);
        self.export_ui(&ctx);
        self.account_ui(&ctx);
        let quiet = self.shot || self.cloud.local();
        if let Some(m) = self.help.ui(&ctx, &mut self.cloud, quiet) {
            self.message = m;
        }
        self.cloud_ui(&ctx);
        self.shares_ui(&ctx);
        self.sync_library(&ctx);
        crate::widgets::focus_ring(&ctx);
        for id in std::mem::take(&mut self.shell.cmds) {
            self.run(&ctx, &id);
        }
    }
}

impl App {
    /// Every menu shortcut of Main.qml. Shift variants first: ⌘S also matches ⇧⌘S.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers as M};
        let (cmd, sc) = (M::COMMAND, M::COMMAND | M::SHIFT);
        const TABLE: [(bool, Key, &str); 28] = [
            (true, Key::S, "saveas"), (true, Key::A, "align"), (true, Key::E, "vars"), (true, Key::K, "kind:1"), (true, Key::D, "kind:2"),
            (true, Key::V, "kind:3"), (true, Key::B, "kind:4"), (true, Key::I, "kind:5"), (true, Key::F, "kind:6"), (true, Key::P, "panels"),
            (true, Key::R, "watch"), (true, Key::T, "paths"),
            (false, Key::N, "new"), (false, Key::W, "closetab"), (false, Key::O, "open"), (false, Key::S, "save"), (false, Key::J, "jobs"),
            (false, Key::Comma, "prefs"), (false, Key::E, "export"), (false, Key::R, "sim"), (false, Key::Num3, "view3d"), (false, Key::Plus, "zoom:in"),
            (false, Key::Equals, "zoom:in"), (false, Key::Minus, "zoom:out"), (false, Key::Num0, "zoom:fit"), (false, Key::Num1, "zoom:board"), (false, Key::Num2, "zoom:100"),
            (false, Key::K, "palette"),
        ];
        let mut run = Vec::new();
        ctx.input_mut(|i| {
            for (shift, key, id) in TABLE {
                if i.consume_key(if shift { sc } else { cmd }, key) {
                    run.push(id);
                }
            }
        });
        if !ctx.egui_wants_keyboard_input() {
            ctx.input_mut(|i| {
                for (key, id) in [(Key::Num1, "mode:Design"), (Key::Num2, "tab:toolpaths"), (Key::Num3, "mode:Simulate"), (Key::T, "tool:text")] {
                    if i.consume_key(M::NONE, key) {
                        run.push(id);
                    }
                }
            });
        }
        for id in run {
            if id == "jobs" && Cloud::web() {
                continue;
            }
            self.shell.push(id);
        }
    }

    /// Web: tell the page about unsaved work, and keep a recovery copy 3 s after the last change (Qt Main.qml recoverTimer).
    fn recover_tick(&mut self, ctx: &egui::Context) {
        if self.shot {
            return;
        }
        let open = !self.canvas.doc.shapes.is_empty() || !self.canvas.doc.ops.is_empty();
        if open != self.last_open {
            crate::recover::set_open(open);
            self.last_open = open;
        }
        let dirty = self.canvas.dirty;
        if dirty != self.last_dirty {
            crate::recover::set_dirty(dirty);
            self.last_dirty = dirty;
            if !dirty {
                crate::recover::clear();
                self.recover_at = None;
            }
        }
        let now = ctx.input(|i| i.time);
        if dirty && self.canvas.rev != self.recover_rev {
            self.recover_rev = self.canvas.rev;
            self.recover_at = Some(now + 3.0);
            ctx.request_repaint_after(std::time::Duration::from_secs(3));
        }
        if let Some(at) = self.recover_at {
            if now >= at {
                crate::recover::save(&self.name, &project::save_project(&self.canvas.doc));
                self.recover_at = None;
            }
        }
    }

    /// "Restore unsaved work?" at start when a recovery copy exists.
    fn recover_ui(&mut self, ctx: &egui::Context) {
        let Some(r) = &self.recover else { return };
        let mut answer: Option<bool> = None;
        let text = format!("GCarve kept a copy of \"{}\" from {} that was never saved. Restore it?", r.name, crate::recover::when(r.time_ms));
        crate::widgets::Dialog::new("Restore unsaved work?").show(ctx, |ui| {
            ui.label(text);
            match crate::widgets::buttons(ui, &[("Restore", true), ("Discard", false)]) {
                Some(0) => answer = Some(true),
                Some(_) => answer = Some(false),
                None if ui.input(|i| i.key_pressed(egui::Key::Enter)) => answer = Some(true),
                None if ui.input(|i| i.key_pressed(egui::Key::Escape)) => answer = Some(false),
                None => {}
            }
        });
        let Some(restore) = answer else { return };
        let r = self.recover.take().unwrap();
        crate::recover::clear();
        if restore {
            match project::load_project(&r.json) {
                Ok(d) => {
                    self.canvas = Canvas::with_doc(d);
                    self.canvas.rev = self.gen_rev.wrapping_add(1);
                    self.canvas.dirty = true; // still unsaved
                    self.name = r.name;
                    self.message = "Restored your unsaved work".into();
                }
                Err(e) => {
                    self.cloud.report("restore", &e);
                    self.message = format!("Could not restore the saved copy: {e}");
                }
            }
        }
    }

    /// "Save changes to …?" (Qt discardDialog): Save / Discard / Cancel.
    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(p) = self.confirm else { return };
        let mut answer: Option<Option<bool>> = None; // Some(Some(save)) / Some(None) = cancel
        crate::widgets::Dialog::new("Unsaved changes").show(ctx, |ui| {
            ui.label(format!("Save changes to {}?", self.name));
            match crate::widgets::buttons(ui, &[("Save", true), ("Discard", false), ("Cancel", false)]) {
                Some(0) => answer = Some(Some(true)),
                Some(1) => answer = Some(Some(false)),
                Some(_) => answer = Some(None),
                None if ui.input(|i| i.key_pressed(egui::Key::Enter)) => answer = Some(Some(true)),
                None if ui.input(|i| i.key_pressed(egui::Key::Escape)) => answer = Some(None),
                None => {}
            }
        });
        match answer {
            Some(Some(save)) => {
                self.confirm = None;
                self.resolve(p, ctx, save);
            }
            Some(None) => self.confirm = None,
            None => {}
        }
    }

    /// Help ▸ About GCarve
    fn about_ui(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = true;
        crate::widgets::Dialog::new("About GCarve").open(&mut open).close_footer().width(340.0).show(ctx, |ui| {
            ui.label(RichText::new("GCarve").font(crate::widgets::bold(20.0)));
            ui.label(format!("Version {} · {}", env!("CARGO_PKG_VERSION"), if Cloud::web() { "web" } else { std::env::consts::OS }));
            ui.add_space(6.0);
            ui.label(format!("CAM for CNC routers: design, toolpaths, simulation and G-code for {} machine posts.", gcarve_core::postdef::post_library().len()));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.link("gcarve.app").clicked() {
                    self.cloud.open_web(ui.ctx(), "/");
                }
                if ui.link("♥ Donate").clicked() {
                    self.cloud.open_web(ui.ctx(), "/donate/");
                }
            });
        });
        self.about_open = open;
    }

    /// Toolpaths ▸ Send to / Open GCarve Control (desktop).
    fn control(&mut self, what: &str) {
        match what {
            "send" => {
                self.full_gen();
                self.message = crate::control::send(&self.canvas.doc, self.gen.as_ref(), &self.name);
            }
            _ => crate::control::open(None),
        }
    }

    // ---- CAM dialogs: every dialog lives in dialogs/*; this hands them the app state they may touch ----
    fn cam_ui(&mut self, ctx: &egui::Context) {
        let mut host = Host {
            canvas: &mut self.canvas,
            files: &self.files,
            message: &mut self.message,
            posts: &mut self.posts,
            templates: &mut self.templates,
            cloud: &mut self.cloud,
            #[cfg(not(target_arch = "wasm32"))]
            jobs: &mut self.jobs,
        };
        self.cam.ui(ctx, &mut host);
        if std::mem::take(&mut self.cam.go_simulate) {
            self.set_mode(Mode::Simulate);
        }
    }

    fn import_stl(&mut self, name: &str, bytes: &[u8]) {
        let mut host = Host {
            canvas: &mut self.canvas,
            files: &self.files,
            message: &mut self.message,
            posts: &mut self.posts,
            templates: &mut self.templates,
            cloud: &mut self.cloud,
            #[cfg(not(target_arch = "wasm32"))]
            jobs: &mut self.jobs,
        };
        dialogs::relief::import_file(name, bytes, &mut host);
        self.cam.relief = Some(dialogs::relief::ReliefDialog::default());
    }

    /// Playback bar over the canvas (Qt SimBar.qml): play/pause, scrubber, speed, machine time, 2D/3D.
    fn sim_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let bar = egui::Rect::from_min_max(egui::pos2(rect.left() + 12.0, rect.bottom() - 48.0), egui::pos2(rect.right() - 12.0, rect.bottom() - 8.0));
        let ctx = ui.ctx().clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            egui::Frame::new().fill(theme::PANEL).stroke(egui::Stroke::new(1.0, theme::BORDER)).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.set_width(bar.width() - 16.0);
                // SimBar.qml: flat ▶/❚❚ and ⏮, scrubber, mono time, speed, Finish, 3D, hide
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    use crate::widgets::Button as B;
                    let ready = self.player.ready();
                    let playing = self.player.playing;
                    let r = ui.add_enabled(ready, B::new(if playing { "" } else { "▶" }).flat().min_width(36.0)).tip(if playing { "Pause" } else { "Watch the cut (restart when finished)" });
                    if playing {
                        for dx in [-3.0, 3.0] {
                            ui.painter().rect_filled(egui::Rect::from_center_size(r.rect.center() + egui::vec2(dx, 0.0), egui::vec2(3.0, 11.0)), 1.0, theme::TEXT);
                        }
                    }
                    if r.clicked() {
                        self.player.toggle();
                    }
                    if ui.add_enabled(ready, B::new("⏮").flat().min_width(32.0)).tip("Back to the start").clicked() {
                        self.player.pause();
                        self.player.seek(0.0, &ctx);
                    }
                    let mut p = self.player.progress();
                    // phones: no time or speed in the bar (play, scrub, Finish, 3D, Show fit)
                    let phone = ctx.content_rect().width() < 600.0;
                    let w = (ui.available_width() - if phone { 290.0 } else { 500.0 }).max(40.0); // room for the Show menu
                    ui.spacing_mut().slider_width = w;
                    if ui.add_enabled(ready, egui::Slider::new(&mut p, 0.0..=1.0).show_value(false).handle_shape(egui::style::HandleShape::Circle)).tip("Drag to scrub through the job (rewinding re-cuts from the start)").changed() {
                        self.player.pause();
                        self.player.seek(p, &ctx);
                    }
                    if !phone {
                        ui.label(RichText::new(self.player.time_text()).font(egui::FontId::monospace(12.0)));
                        // any speed from real time to 200× (Finish = instant); faster would only stamp more moves per frame
                        ui.spacing_mut().slider_width = 80.0;
                        let mut sp = self.player.speed.clamp(1.0, MAX_SIM_SPEED);
                        if ui.add(egui::Slider::new(&mut sp, 1.0..=MAX_SIM_SPEED).logarithmic(true).handle_shape(egui::style::HandleShape::Circle).suffix("×").fixed_decimals(0)).tip("Playback speed: 1× is the machine's real speed").changed() {
                            self.player.speed = sp;
                        }
                    }
                    if ui.add_enabled(ready, B::new("Finish").flat()).tip("Jump to the finished part").clicked() {
                        self.player.pause();
                        self.player.seek(1.0, &ctx);
                    }
                    if ui.add(B::new(if self.view_3d { "2D" } else { "3D" }).flat().min_width(52.0)).tip(if self.view_3d { "Back to the drawing view" } else { "Rotate the carved board in 3D (⌘3)" }).clicked() {
                        self.view_3d = !self.view_3d;
                    }
                    self.show_menu(ui);
                    if crate::icons::IconButton::new("remove").size(14.0).padding(3.0).tip("Hide the simulation").show(ui).clicked() {
                        self.shell.push("showsim");
                    }
                });
            });
        });
    }

    /// Simulate bar › Show: toolpaths (all / cut so far / hidden), rapid moves, cutter; same in 2D and 3D, remembered.
    fn show_menu(&mut self, ui: &mut egui::Ui) {
        use crate::view3d::ShowPaths;
        let before = self.view3d.show;
        let s = &mut self.view3d.show;
        ui.menu_button("Show ▾", |ui| {
            ui.label(RichText::new("Toolpaths").color(theme::TEXT_DIM));
            ui.radio_value(&mut s.paths, ShowPaths::All, "All moves");
            ui.radio_value(&mut s.paths, ShowPaths::Trail, "Cut so far (tool trail)");
            ui.radio_value(&mut s.paths, ShowPaths::Hidden, "Hidden");
            ui.separator();
            ui.checkbox(&mut s.rapids, "Rapid moves (red)");
            ui.checkbox(&mut s.cutter, "Cutter");
            if !self.canvas.show_paths {
                ui.label(RichText::new("The status bar's Paths switch is off: toolpaths stay hidden").color(theme::TEXT_DIM).size(11.0));
            }
        })
        .response
        .on_hover_text("What the simulation draws over the board: toolpaths, the tool's trail, rapid moves, the cutter");
        if self.view3d.show != before {
            self.view3d.show.save();
        }
    }

    /// The export files for the current state: (files, safety issues, per-file (tools, toolpaths))
    fn export_files(&self, raise: f64, tiling: Option<pipeline::Tiling>) -> Option<pipeline::ExportResult> {
        let gen = self.gen.as_ref()?;
        let ts = chrono::Local::now().format("%I:%M %p, %A %B %-d %Y").to_string();
        let post = pipeline::active_post(&self.canvas.doc, &|id| self.posts.find(id));
        if let Some(g) = &self.export_group {
            // one folder: the document and its toolpaths with only that folder's toolpaths, files named after it
            let keep: Vec<bool> = self.canvas.doc.ops.iter().map(|o| o.op_group == *g).collect();
            let mut doc = self.canvas.doc.clone();
            let mut k = keep.iter();
            doc.ops.retain(|_| *k.next().unwrap_or(&false));
            let only = pipeline::Generated {
                paths: gen.paths.iter().zip(&keep).filter(|(_, k)| **k).map(|(x, _)| x.clone()).collect(),
                minutes: gen.minutes.iter().zip(&keep).filter(|(_, k)| **k).map(|(x, _)| *x).collect(),
                warnings: gen.warnings.clone(),
                draft_tool: -1,
            };
            let name = format!("{}_{}", self.name, g.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "-"));
            return Some(pipeline::export_tiled(&doc, &only, &name, &ts, raise, &post, tiling));
        }
        Some(pipeline::export_tiled(&self.canvas.doc, gen, &self.name, &ts, raise, &post, tiling))
    }

    fn export_tiling(&self, d: &ExportDlg) -> Option<pipeline::Tiling> {
        let doc = &self.canvas.doc;
        d.tile.then(|| pipeline::Tiling { along_x: !d.tile_y, len: units::get(doc, &d.tile_len, true).unwrap_or(0.0), overlap: units::get(doc, &d.tile_overlap, true).unwrap_or(0.0).max(0.0) })
    }

    /// The setup sheet for an export with these settings (File ▸ Setup Sheet…, the Export dialog's box).
    fn setup_sheet(&self, raise: f64, names: &[String]) -> Option<String> {
        let gen = self.gen.as_ref()?;
        let post = pipeline::active_post(&self.canvas.doc, &|id| self.posts.find(id));
        Some(crate::setupsheet::html(&self.canvas.doc, gen, &self.name, raise, &post, names))
    }

    /// Every export opens the Export dialog (Qt ExportGuard.qml): file names, folder, air-cut height, safety check.
    fn start_export(&mut self, air_cut: bool) {
        let raise = if air_cut { gcarve_core::cam::air_cut_raise(&self.canvas.doc.material) } else { 0.0 };
        let Some(r) = self.export_files(raise, None) else { return };
        if let Some(g) = self.gen.as_ref().filter(|g| !g.warnings.is_empty()) {
            self.message = format!("⚠ {}", g.warnings.join(" · "));
        }
        #[cfg(not(target_arch = "wasm32"))]
        let folder = self.project_path.as_ref().and_then(|p| self.jobs.job_for(p)).map(|j| j.join("gcode")).or_else(|| crate::store::load(EXPORT_FOLDER_KEY).map(PathBuf::from).filter(|p| p.is_dir()));
        self.export = Some(ExportDlg {
            air: air_cut,
            raise: units::text(&self.canvas.doc, raise),
            names: r.files.iter().map(|f| f.filename.clone()).collect(),
            plan: r.plan,
            issues: r.safety,
            sheet: true,
            zip: true,
            tile: false,
            tile_y: self.canvas.doc.material.h > self.canvas.doc.material.w,
            tile_len: units::text(&self.canvas.doc, 24.0),
            tile_overlap: units::text(&self.canvas.doc, 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            folder,
        });
    }

    fn export_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.export.take() else { return };
        let (mut ok, mut cancel, mut replan) = (false, false, false);
        let title = if d.air { "Export Air-Cut Test" } else { "Export G-code" };
        let mut open = true;
        crate::widgets::Dialog::new(title).id("export").open(&mut open).width(580.0).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            let dim = |t: String| RichText::new(t).size(11.0).color(theme::TEXT_DIM);
            if d.air {
                ui.add(egui::Label::new("Writes the job with every Z raised so the machine runs the whole program in the air. Zero Z on the material as usual. Files end in _AIRCUT and say AIR CUT TEST in the header.").wrap());
                ui.horizontal(|ui| {
                    ui.label(format!("Raise Z by ({})", units::name(&self.canvas.doc)));
                    replan = units::num_field(ui, "raise", &mut d.raise, &self.canvas.doc, true, 110.0).is_some();
                });
            }
            let head = if d.names.len() == 1 { "FILE".to_string() } else { format!("{} FILES (ONE PER BIT{}; CHANGE THE NAMES IF YOU LIKE)", d.names.len(), if d.tile { " AND TILE" } else { "" }) };
            ui.label(RichText::new(head).size(11.0).font(crate::widgets::bold(11.0)).color(theme::TEXT_DIM));
            // many files (several bits × sheets × tiles): the list scrolls, the buttons stay in view
            egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                for (i, n) in d.names.iter_mut().enumerate() {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(crate::widgets::field(n).id(egui::Id::new(("export-name", i))).desired_width(560.0));
                    ui.spacing_mut().item_spacing.y = 10.0;
                    if let Some((tool, paths)) = d.plan.get(i) {
                        ui.add(egui::Label::new(dim(if paths.is_empty() { tool.clone() } else { format!("{tool}  ·  {paths}") })).truncate());
                    }
                }
            });
            #[cfg(not(target_arch = "wasm32"))]
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label("Save to");
                let (text, col) = match &d.folder {
                    Some(f) => (f.display().to_string(), theme::TEXT),
                    None => ("Choose a folder…".to_string(), theme::WARNING),
                };
                ui.allocate_ui(egui::vec2(580.0 - 60.0 - 90.0, 20.0), |ui| ui.add(egui::Label::new(RichText::new(text).color(col)).truncate()));
                ui.add_space((580.0 - ui.min_rect().width() - 84.0).max(0.0));
                if ui.add(crate::widgets::Button::new("Choose…").min_width(84.0)).clicked() {
                    let mut pick = rfd::FileDialog::new().set_title("Export G-code to folder");
                    if let Some(f) = &d.folder {
                        pick = pick.set_directory(f);
                    }
                    if let Some(f) = pick.pick_folder() {
                        d.folder = Some(f);
                    }
                }
            });
            #[cfg(target_arch = "wasm32")]
            if d.names.len() + usize::from(d.sheet) > 1 {
                crate::widgets::checkbox(ui, &mut d.zip, "Download everything as one .zip file");
            } else {
                ui.add(egui::Label::new(RichText::new("The file downloads to your browser's Downloads folder.").size(12.0).color(theme::TEXT_DIM)).wrap());
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                replan |= crate::widgets::checkbox(ui, &mut d.tile, "Tile (long board)").tip("Cuts the job in sections. Each tile's files start at zero: cut tile 1, slide the board by the tile length minus the overlap, cut tile 2, and so on. XY zero must be the bottom-left corner.").changed();
                if d.tile {
                    let doc = &self.canvas.doc;
                    ui.label(format!("every ({})", units::name(doc)));
                    replan |= units::num_field(ui, "tile-len", &mut d.tile_len, doc, true, 96.0).is_some();
                    ui.label("overlap");
                    replan |= units::num_field(ui, "tile-ov", &mut d.tile_overlap, doc, true, 90.0).is_some();
                    replan |= crate::widgets::combo_box("tile-axis").selected_text(if d.tile_y { "along Y" } else { "along X" }).width(84.0).show_ui(ui, |ui| ui.selectable_value(&mut d.tile_y, false, "along X").changed() | ui.selectable_value(&mut d.tile_y, true, "along Y").changed()).inner.unwrap_or(false);
                }
            });
            crate::widgets::checkbox(ui, &mut d.sheet, format!("Setup sheet ({}_setup.html: printable job sheet for the operator)", self.name));
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Check every file before you cut: watch the simulation and run an air cut on new jobs. GCarve comes without warranty;").small().color(theme::TEXT_DIM));
                ui.hyperlink_to(RichText::new("terms").small(), "https://gcarve.app/terms/#gcode");
            });
            if self.canvas.doc.material.machine.is_none() && self.canvas.doc.material.auto_feeds {
                // same notice as the toolpath dialogs: suggested feeds assume a rigid industrial machine
                ui.add(egui::Label::new(RichText::new("No machine chosen: the suggested feeds suit a heavy industrial router. On a hobby or desktop machine pick yours first (Toolpaths › Choose Machine…) and export again.").size(12.0).color(theme::WARNING)).wrap());
            }
            if !d.issues.is_empty() {
                ui.add(egui::Label::new(RichText::new("The safety check found problems in this job. Fix them, or export anyway if you are sure:").color(theme::WARNING).font(crate::widgets::bold(theme::FONT_SIZE))).wrap());
                for i in &d.issues {
                    ui.add(egui::Label::new(format!("• {i}")).wrap());
                }
            }
            let go = if !d.issues.is_empty() { "Export anyway" } else if cfg!(target_arch = "wasm32") { "Download" } else { "Export" };
            #[cfg(not(target_arch = "wasm32"))]
            let can = d.folder.is_some();
            #[cfg(target_arch = "wasm32")]
            let can = true;
            ui.add_space(26.0 - 10.0);
            let right = ui.min_rect().right();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let w = ui.painter().layout_no_wrap(go.to_string(), theme::font(theme::FONT_SIZE), theme::TEXT).size().x + 28.0;
                ui.add_space((right - ui.cursor().left() - 84.0 - 8.0 - w.max(84.0)).max(0.0));
                cancel |= ui.add(crate::widgets::Button::new("Cancel").min_width(84.0)).clicked();
                ok |= ui.add_enabled(can, crate::widgets::Button::new(go).primary(d.issues.is_empty()).min_width(84.0)).clicked();
            });
            ok |= can && ui.input(|i| i.key_pressed(egui::Key::Enter)) && ctx.memory(|m| m.focused().is_none());
        });
        if replan {
            if let Some(r) = self.export_files(self.export_raise(&d), self.export_tiling(&d)) {
                d.names = r.files.iter().map(|f| f.filename.clone()).collect(); // Qt refreshPlan: names follow the new plan
                d.plan = r.plan;
                d.issues = r.safety;
            }
        }
        if ok {
            if let Some(r) = self.export_files(self.export_raise(&d), self.export_tiling(&d)) {
                if !self.staff {
                    self.cloud.event_detail("export", &format!("{} file(s), {}", r.files.len(), self.canvas.doc.material.post));
                }
                let mut files: Vec<(String, Vec<u8>)> = r.files.into_iter().enumerate().map(|(i, f)| (pipeline::clean_file_name(d.names.get(i).map(String::as_str).unwrap_or(""), &f.filename), f.text.into_bytes())).collect();
                if d.sheet {
                    // the sheet describes the whole job (its own file names when the export is tiled)
                    if let Some(html) = self.setup_sheet(self.export_raise(&d), if d.tile { &[] } else { &d.names }) {
                        files.push((format!("{}_setup.html", self.name), html.into_bytes()));
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(dir) = &d.folder {
                    self.write_export(dir, files);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    // several files: one .zip download instead of a browser prompt per file
                    let files = if d.zip && files.len() > 1 { vec![(format!("{}_gcode.zip", self.name), files::zip(&files))] } else { files };
                    files::save_many(files, self.files.msg_tx.clone());
                }
            }
        } else if !cancel && open {
            self.export = Some(d);
        }
    }

    fn export_raise(&self, d: &ExportDlg) -> f64 {
        if d.air { units::get(&self.canvas.doc, &d.raise, true).unwrap_or(0.0).max(0.001) } else { 0.0 }
    }

    /// Writes the files into `dir`; earlier versions move to `history/` (Qt archiveRevision on every export).
    #[cfg(not(target_arch = "wasm32"))]
    fn write_export(&mut self, dir: &std::path::Path, files: Vec<(String, Vec<u8>)>) {
        let mut names = Vec::new();
        for (n, b) in &files {
            let p = dir.join(n);
            let rev = crate::jobs::archive_revision(&p, &String::from_utf8_lossy(b));
            if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&p, b)) {
                self.message = format!("Cannot write {}: {e}", p.display());
                return;
            }
            names.push(if rev > 1 { format!("{n} (r{rev})") } else { n.clone() });
        }
        crate::store::save(EXPORT_FOLDER_KEY, &dir.to_string_lossy());
        self.jobs.refresh();
        self.message = format!("Exported {} to {}", names.join(", "), dir.display());
    }
}

impl App {
    fn save_to_cloud(&mut self, ctx: &egui::Context) {
        if !self.cloud.signed_in() {
            self.account.open = true;
            return;
        }
        if self.cloud_id.is_empty() {
            self.projects.show(&self.name.clone());
            self.cloud.refresh(ctx);
        } else {
            self.cloud.save_project(ctx, &self.cloud_id.clone(), &self.name.clone(), None, &project::save_project(&self.canvas.doc));
        }
    }

    fn account_ui(&mut self, ctx: &egui::Context) {
        self.account.show(ctx, &mut self.cloud);
    }

    /// Bits / machines changed locally: upload once the account copy has been merged in
    fn sync_library(&mut self, ctx: &egui::Context) {
        if !self.cloud.signed_in() {
            self.lib_ready = false;
            return;
        }
        if self.lib_ready && crate::library::rev() != self.lib_pushed {
            self.lib_pushed = crate::library::rev();
            self.cloud.push_library(ctx, "bits", crate::library::bits_json(&crate::toollib::load()));
            self.cloud.push_library(ctx, "machines", serde_json::to_value(crate::library::machines()).unwrap_or_default());
        }
    }

    /// Help ▸ Shared with Me: bits and machines other users sent, to add or turn down
    fn shares_ui(&mut self, ctx: &egui::Context) {
        if !self.shares_open {
            return;
        }
        let mut open = true;
        let mut take = None;
        crate::widgets::Dialog::new("Shared with me").id("shares").open(&mut open).close_footer().width(480.0).show(ctx, |ui| {
            if self.shares.is_empty() {
                ui.label(RichText::new("Nothing waiting. When someone shares bits or a machine with your email, it shows here.").color(theme::TEXT_DIM));
            }
            for s in &self.shares {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&s.name).family(crate::widgets::bold_family()));
                        ui.label(RichText::new(format!("{} from {}", if s.kind == "machines" { "Machine" } else { "Bits" }, s.from)).small().color(theme::TEXT_DIM));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("No thanks").clicked() {
                            take = Some((s.id, false));
                        }
                        if ui.add(egui::Button::new("Add to my library").fill(theme::ACCENT)).clicked() {
                            take = Some((s.id, true));
                        }
                    });
                });
                ui.separator();
            }
        });
        if let Some((id, accept)) = take {
            self.shares.retain(|s| s.id != id);
            self.cloud.take_share(ctx, id, accept);
        }
        if !open {
            self.shares_open = false;
        }
    }

    fn cloud_ui(&mut self, ctx: &egui::Context) {
        use dialogs::projects::Action;
        let Some(act) = self.projects.ui(ctx, &mut self.cloud, &self.cloud_id) else { return };
        let json = || project::save_project(&self.canvas.doc);
        match act {
            Action::Open(id) => self.cloud.open_project(ctx, &id),
            Action::SaveNew(name, folder) => self.cloud.save_project(ctx, "", &name, Some(&folder), &json()),
            Action::Replace(id) => {
                let name = self.projects.name.trim().to_string();
                self.cloud.save_project(ctx, &id, &name, None, &json());
            }
            Action::FromComputer => self.files.pick(ctx, false),
            Action::Download => files::save(&format!("{}.tlp", self.name), json().into_bytes(), self.files.msg_tx.clone()),
        }
    }

}

/// The showcase sign as the demo: the lettering's clearance bit doesn't fit its narrow flats (a warning in the
/// showcase, which the goldens keep), so the demo leaves the V-bit to clear them and opens without warnings.
/// The "please wait" card over `rect`: a title and a bar (`progress` 0..1, None = no number, the bar half full).
fn busy_card(p: &egui::Painter, rect: egui::Rect, title: &str, progress: Option<f32>) {
    p.rect_filled(rect, 0.0, theme::CANVAS_BG.gamma_multiply(0.85));
    let card = egui::Rect::from_center_size(rect.center(), egui::vec2(320.0, 84.0));
    p.rect_filled(card, 8.0, theme::PANEL);
    p.text(card.center_top() + egui::vec2(0.0, 24.0), egui::Align2::CENTER_CENTER, title, theme::font(14.0), theme::TEXT);
    let bar = egui::Rect::from_min_size(card.left_bottom() + egui::vec2(24.0, -34.0), egui::vec2(card.width() - 48.0, 6.0));
    p.rect_filled(bar, 3.0, theme::TEXT.gamma_multiply(0.15));
    let f = progress.unwrap_or(0.5);
    p.rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * f.clamp(0.02, 1.0), bar.height())), 3.0, theme::MODE_SIMULATE);
    if let Some(f) = progress {
        p.text(card.center_bottom() + egui::vec2(0.0, -14.0), egui::Align2::CENTER_CENTER, format!("{:.0}%", f * 100.0), theme::font(11.0), theme::TEXT_DIM);
    }
}

pub(crate) fn demo_project() -> Vec<u8> {
    let raw = include_bytes!("../../../samples/showcase.tlp");
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(raw) else { return raw.to_vec() };
    if let Some(ops) = v["ops"].as_array_mut() {
        for op in ops.iter_mut() {
            if op["type"] == "vcarve" {
                op["clearTool"] = (-1).into();
            }
        }
        // the showcase lists the cut-out first; a real job cuts the part free last
        ops.sort_by_key(|op| op["name"] == "Cut out");
    }
    serde_json::to_vec(&v).unwrap_or_else(|_| raw.to_vec())
}

#[cfg(test)]
mod demo_tests {
    #[test]
    fn demo_opens_without_warnings() {
        let doc = gcarve_core::project::load_project(std::str::from_utf8(&super::demo_project()).unwrap()).unwrap();
        let gen = gcarve_core::pipeline::regenerate(&doc);
        assert!(gen.warnings.is_empty(), "{:?}", gen.warnings);
        assert_eq!(doc.ops.len(), 7);
        assert_eq!(doc.ops[6].name, "Cut out");
        assert!(gcarve_core::pipeline::order_issues(&doc).is_empty(), "{:?}", gcarve_core::pipeline::order_issues(&doc));
        // the showcase lists the cut-out first but holds the part with tabs: fine. Without tabs it is flagged.
        let mut show = gcarve_core::project::load_project(include_str!("../../../samples/showcase.tlp")).unwrap();
        assert!(gcarve_core::pipeline::order_issues(&show).is_empty());
        show.ops[0].tabs = false;
        let issues = gcarve_core::pipeline::order_issues(&show);
        assert!(issues.len() == 1 && issues[0].contains("Pockets") && issues[0].contains("Cut Later"), "{issues:?}");
    }
}

/// Web build smoke test (deploy-web.sh runs it in headless Chrome before uploading): with `?smoke` in the address
/// the app opens the demo, generates every toolpath, exports and simulates nothing by hand, then writes the result
/// on the page's <body data-smoke>: "SMOKE OK …" or "SMOKE FAIL …". A crash in the browser build (things that only
/// break in wasm, like the system clock) leaves it unset, which the script treats as a failure.
#[cfg(target_arch = "wasm32")]
impl App {
    fn smoke_test(&mut self, ctx: &egui::Context) {
        let Some(win) = web_sys::window() else { return };
        if !win.location().search().is_ok_and(|q| q.contains("smoke")) {
            return;
        }
        ctx.request_repaint(); // keep the frames coming without any input
        let say = |t: String| {
            // on <body data-smoke="…">: the app keeps the page title for the project name
            if let Some(b) = win.document().and_then(|d| d.body()) {
                if b.get_attribute("data-smoke").is_some() {
                    return; // reported once
                }
                let _ = b.set_attribute("data-smoke", &t);
            }
            // and to the test's own web server, whose request log the script reads (headless Chrome can't be asked)
            let origin = win.location().origin().unwrap_or_default();
            ehttp::fetch(ehttp::Request::get(format!("{origin}/smoke-result?{}", t.replace(' ', "+").replace(',', ""))), |_| {});
        };
        if self.frame == 3 {
            self.shell.push("demo");
            return;
        }
        if self.frame <= 3 || self.frame % 20 != 0 || self.canvas.doc.ops.len() < 7 {
            return;
        }
        self.full_gen();
        let Some(gen) = self.gen.as_ref() else { return };
        let cut: usize = gen.paths.iter().flatten().map(|t| t.moves.len()).sum();
        let made = gen.paths.iter().filter(|p| !p.is_empty()).count();
        if self.smoke_step == 0 {
            // stage 1: the demo as it is, exported plain and tiled
            let tiled = self.export_files(0.0, Some(pipeline::Tiling { along_x: true, len: 5.0, overlap: 1.0 })).map_or(0, |r| r.files.len());
            let files = self.export_files(0.0, None).map_or(0, |r| r.files.iter().filter(|f| f.text.len() > 200).count());
            if made < 6 || files == 0 || tiled <= files {
                return say(format!("SMOKE FAIL stage 1: {made} toolpaths, {files} files, {tiled} tiled"));
            }
            self.smoke_note = format!("{made} toolpaths {cut} moves {files} files {tiled} tiled");
            // stage 2: the newer options on the same job (raster + two-bit pocket, lead-in + last pass, mirrored sheet)
            let doc = &mut self.canvas.doc;
            let small = doc.tools.iter().position(|t| t.diameter < 0.2 && t.kind == gcarve_core::document::ToolType::EndMill).unwrap_or(0) as i32;
            for op in &mut doc.ops {
                match op.kind {
                    gcarve_core::document::OpType::Pocket => {
                        op.raster = true;
                        op.raster_angle = 30;
                    }
                    gcarve_core::document::OpType::Profile => {
                        op.ramp_in = 0.2;
                        op.allowance = 0.02;
                    }
                    _ => {}
                }
            }
            let mut two = doc.ops.iter().find(|o| o.kind == gcarve_core::document::OpType::Pocket).cloned().unwrap_or_default();
            (two.raster, two.clear_tool, two.tool, two.name) = (false, two.tool, small, "Two-bit".into());
            // and the toolpath kinds the demo doesn't have, on the pocket's shapes
            let vbit = doc.tools.iter().position(|t| t.kind == gcarve_core::document::ToolType::VBit).unwrap_or(0) as i32;
            use gcarve_core::document::OpType as K;
            for (kind, name, tool, depth) in [(K::Inlay, "Inlay", vbit, 0.1), (K::Chamfer, "Chamfer", vbit, 0.05), (K::Prism, "Prism", vbit, 0.1), (K::Texture, "Texture", two.clear_tool, 0.04)] {
                doc.ops.push(gcarve_core::document::Op { kind, name: name.into(), tool, depth, flat_depth: if kind == K::Inlay { 0.1 } else { 0.0 }, glue_gap: 0.02, inlay_margin: 0.5, clear_tool: -1, raster: false, ramp_length: 0.5, stepover: 0.8, ..two.clone() });
            }
            doc.ops.push(two);
            self.canvas.touch();
            self.smoke_step = 1;
        } else if self.canvas.doc.ops.len() >= 12 && gen.paths.len() >= 12 {
            let two_bit = gen.paths.last().map_or(0, Vec::len);
            let files = self.export_files(0.0, None).map_or(0, |r| r.files.len());
            let n = gen.paths.len();
            let empty: Vec<&str> = (n - 5..n - 1).filter(|&i| gen.paths[i].is_empty()).map(|i| self.canvas.doc.ops[i].name.as_str()).collect();
            if made < 11 || two_bit < 2 || files == 0 || cut == 0 || !empty.is_empty() {
                return say(format!("SMOKE FAIL stage 2: {made} toolpaths, two-bit pocket {two_bit} paths, {files} files, nothing from: {}", empty.join(" ")));
            }
            // stage 3: how long the browser build takes on a busy drawing (250 star shapes, pocket + V-carve + cut out);
            // reported in the result line so a deploy shows the browser's speed, not the desktop's
            use gcarve_core::document::{Document, Op, OpType as K};
            let mut big = Document::default();
            big.tools = self.canvas.doc.tools.clone();
            big.material.w = 30.0;
            big.material.h = 12.0;
            big.material.t = 0.75;
            let mut ids = Vec::new();
            for k in 0..250 {
                let (cx, cy) = (1.0 + 1.1 * f64::from(k % 25), 1.0 + 1.1 * f64::from(k / 25));
                let pts: Vec<_> = (0..40)
                    .map(|i| {
                        let a = f64::from(i) / 40.0 * std::f64::consts::TAU;
                        let r = 0.4 * if i % 2 == 0 { 1.0 } else { 0.75 };
                        gcarve_core::geom::pt(cx + r * a.cos(), cy + r * a.sin())
                    })
                    .collect();
                ids.push(big.add(gcarve_core::geom::make_polyline(&pts, true), None));
            }
            let tool_of = |kind| big.tools.iter().position(|t| t.kind == kind).unwrap_or(0) as i32;
            let (em, vb) = (tool_of(gcarve_core::document::ToolType::EndMill), tool_of(gcarve_core::document::ToolType::VBit));
            for (kind, name, tool, depth) in [(K::Pocket, "Pocket", em, 0.25), (K::VCarve, "V-carve", vb, 0.3), (K::Profile, "Cut out", em, 0.75)] {
                big.ops.push(Op { kind, name: name.into(), tool, depth, shape_ids: ids.clone(), ..Default::default() });
            }
            let t0 = js_sys::Date::now();
            let g = pipeline::regenerate(&big);
            let ms = (js_sys::Date::now() - t0) as u64;
            let big_moves: usize = g.paths.iter().flatten().map(|t| t.moves.len()).sum();
            if big_moves == 0 {
                return say("SMOKE FAIL stage 3: the busy drawing made no toolpaths".into());
            }
            say(format!("SMOKE OK {} then options: {made} toolpaths {cut} moves {files} files; busy drawing 250 shapes {big_moves} moves in {ms} ms", self.smoke_note));
        }
    }
}

#[cfg(all(feature = "screenshot", not(target_arch = "wasm32")))]
impl App {
    /// Dev video: once the simulation is built, for each frame i seek to i / n, ask for a screenshot, wait for it, write
    /// `frame_NNNN.png`, orbit the 3D view a little, go on; exits after the last one. ffmpeg turns the folder into a video.
    fn movie_tick(&mut self, ctx: &egui::Context) {
        let Some((dir, i, n, waiting)) = &mut self.movie else { return };
        ctx.request_repaint();
        if !self.player.ready() || self.player.preparing().is_some() {
            return;
        }
        if *waiting {
            let shot = ctx.input(|inp| inp.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
            if let Some(img) = shot {
                let _ = std::fs::create_dir_all(&*dir);
                let _ = std::fs::write(dir.join(format!("frame_{:04}.png", *i)), crate::shot::png_bytes(&img));
                *i += 1;
                *waiting = false;
                if *i >= *n {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            return;
        }
        // pace: each toolpath gets a share of the video by the square root of its minutes, so the long pocket does not
        // swallow it; inside a toolpath the time runs evenly
        let u = (*i as f64 / (*n - 1).max(1) as f64).min(1.0);
        let mins: Vec<f64> = self.gen.as_ref().map_or_else(Vec::new, |g| g.minutes.clone());
        let (total, wsum): (f64, f64) = (mins.iter().sum(), mins.iter().map(|m| m.sqrt()).sum());
        let f = if total <= 0.0 || wsum <= 0.0 {
            u
        } else {
            let (mut w0, mut m0, mut f) = (0.0, 0.0, 1.0);
            for (k, m) in mins.iter().enumerate() {
                let w = m.sqrt() / wsum;
                if u <= w0 + w || k + 1 == mins.len() {
                    f = (m0 + ((u - w0) / w.max(1e-9)).clamp(0.0, 1.0) * m) / total;
                    break;
                }
                w0 += w;
                m0 += m;
            }
            f
        };
        self.player.pause();
        self.player.seek(f, ctx);
        self.view3d.yaw = -35.0 + 60.0 * u; // a slow turn round the board
        *waiting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }
}
