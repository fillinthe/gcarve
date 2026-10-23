//! Opening files: the picker (native dialog or the browser's file input) and drag-and-drop, both delivering
//! (name, bytes); load() turns them into the document (.tlp = project, .svg / .dxf = import into the current one).

#[cfg(target_arch = "wasm32")]
use gcarve_ui::webfile::{blob_url, download};
use gcarve_core::document::Document;
use gcarve_core::{dxf, project, svg};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

/// A file to open: picked, dropped or given on the command line.
pub struct Picked {
    pub name: String,
    pub bytes: Vec<u8>,
    /// where it came from (desktop; the web has no paths)
    pub path: Option<PathBuf>,
    /// true = import into the current document (File ▸ Import), false = open (in a new tab unless the current one is empty)
    pub import: bool,
}

pub struct Files {
    tx: Sender<Picked>,
    rx: Receiver<Picked>,
    pub msg_tx: Sender<String>,
    pub msg_rx: Receiver<String>,
    // ---- tagged picks (tool libraries, posts, STL models): (tag, name, bytes), consumed by the dialogs
    aux_tx: Sender<(&'static str, String, Vec<u8>)>,
    pub aux_rx: Receiver<(&'static str, String, Vec<u8>)>,
    /// Save As finished: (document id, chosen path)
    pub saved_tx: Sender<(u64, PathBuf)>,
    pub saved_rx: Receiver<(u64, PathBuf)>,
    /// web, signed out: bringing files in needs a free account (the app sets this every frame)
    pub locked: std::cell::Cell<bool>,
    /// a pick or drop was refused because of `locked` (the app opens the account box)
    pub wants_account: std::cell::Cell<bool>,
}

impl Default for Files {
    fn default() -> Self {
        let (tx, rx) = channel();
        let (msg_tx, msg_rx) = channel();
        let (aux_tx, aux_rx) = channel();
        let (saved_tx, saved_rx) = channel();
        Files { tx, rx, msg_tx, msg_rx, aux_tx, aux_rx, saved_tx, saved_rx, locked: Default::default(), wants_account: Default::default() }
    }
}

impl Files {
    /// Shows the picker; the file arrives later through `poll` (the web picker is asynchronous).
    /// true (and asks for an account) when files can't come in yet
    fn refuse(&self) -> bool {
        if self.locked.get() {
            self.wants_account.set(true);
        }
        self.locked.get()
    }

    pub fn pick(&self, ctx: &eframe::egui::Context, import: bool) {
        if self.refuse() {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let exts: &[&str] = if import { IMPORT_EXTS } else { OPEN_EXTS };
            let tx = self.tx.clone();
            web_pick(exts, ctx.clone(), self.msg_tx.clone(), move |name, bytes| {
                let _ = tx.send(Picked { name, bytes, path: None, import });
            });
            return;
        }
        #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
        let dialog = if import {
            rfd::AsyncFileDialog::new().set_title("Import into this document").add_filter("All supported", IMPORT_EXTS).add_filter("Vector files", VECTOR_EXTS).add_filter("LightBurn project", &["lbrn2"]).add_filter("GSketch model", &["gsk"]).add_filter("G-code", GCODE_EXTS).add_filter("Images to trace", IMAGE_EXTS).add_filter("3D models", &["stl", "3mf"])
        } else {
            rfd::AsyncFileDialog::new().add_filter("All supported", OPEN_EXTS).add_filter("GCarve project", &["tlp"]).add_filter("Job template", &["tlpt"]).add_filter("VCarve project", &["crv"]).add_filter("GSketch model", &["gsk"]).add_filter("Vector files", VECTOR_EXTS).add_filter("LightBurn project", &["lbrn2"]).add_filter("G-code", GCODE_EXTS)
        };
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let task = async move {
            if let Some(f) = dialog.pick_file().await {
                #[cfg(not(target_arch = "wasm32"))]
                let path = Some(f.path().to_path_buf());
                #[cfg(target_arch = "wasm32")]
                let path = None;
                let _ = tx.send(Picked { name: f.file_name(), bytes: f.read().await, path, import });
                ctx.request_repaint();
            }
        };
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(task);
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || pollster_block_on(task));
    }

    /// Picker for one kind of side file (bit library, post, STL); the file arrives on `aux_rx` with `tag`.
    pub fn pick_into(&self, ctx: &eframe::egui::Context, filter: &str, exts: &[&str], tag: &'static str) {
        if self.refuse() {
            return;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = filter;
            let tx = self.aux_tx.clone();
            web_pick(exts, ctx.clone(), self.msg_tx.clone(), move |name, bytes| {
                let _ = tx.send((tag, name, bytes));
            });
            return;
        }
        #[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
        let dialog = rfd::AsyncFileDialog::new().add_filter(filter, exts);
        let tx = self.aux_tx.clone();
        let ctx = ctx.clone();
        let task = async move {
            if let Some(f) = dialog.pick_file().await {
                let _ = tx.send((tag, f.file_name(), f.read().await));
                ctx.request_repaint();
            }
        };
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(task);
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || pollster_block_on(task));
    }

    /// Files picked or dropped since the last frame (a drop opens, like the Qt app).
    pub fn poll(&self, ctx: &eframe::egui::Context) -> Vec<Picked> {
        let mut out: Vec<_> = self.rx.try_iter().collect();
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() && self.refuse() {
            return out;
        }
        for f in dropped {
            #[cfg(not(target_arch = "wasm32"))]
            self.open_path(f.path());
            #[cfg(target_arch = "wasm32")]
            {
                let name = f.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let tx = self.tx.clone();
                let ctx = ctx.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    if let Ok(bytes) = f.bytes_async().await {
                        let _ = tx.send(Picked { name, bytes, path: None, import: false });
                        ctx.request_repaint();
                    }
                });
            }
        }
        out.extend(self.rx.try_iter());
        out
    }

    /// Queue a file read from disk (command line, recent files, jobs, drops); `import` = into the current document.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_path(&self, path: &std::path::Path) {
        match std::fs::read(path) {
            Ok(bytes) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let _ = self.tx.send(Picked { name, bytes, path: Some(path.to_path_buf()), import: false });
            }
            Err(e) => {
                let _ = self.msg_tx.send(format!("Cannot open {}: {e}", path.display()));
            }
        }
    }

    /// Save As (desktop): native dialog on its own thread; the chosen path comes back on `saved_rx` with `doc`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_project_as(&self, doc: u64, name: &str, bytes: Vec<u8>) {
        let dialog = rfd::AsyncFileDialog::new().set_file_name(format!("{name}.tlp")).add_filter("GCarve project", &["tlp"]).add_filter("VCarve project", &["crv"]);
        let (done, msg) = (self.saved_tx.clone(), self.msg_tx.clone());
        std::thread::spawn(move || {
            let Some(f) = pollster_block_on(dialog.save_file()) else { return };
            let mut p = f.path().to_path_buf();
            if p.extension().is_none() {
                p.set_extension("tlp");
            }
            match std::fs::write(&p, &bytes) {
                Ok(()) => {
                    let _ = done.send((doc, p));
                }
                Err(e) => {
                    let _ = msg.send(format!("Cannot save {}: {e}", p.display()));
                }
            }
        });
    }
}

/// File ▸ Open Recent (desktop): newest first, ten at most, kept in the config store.
/// A message with the file's name (or full path) taken out, so it can go to the error log: "report.pdf: Nothing to
/// import" -> "file.pdf: Nothing to import".
pub fn scrub(msg: &str, name: &str) -> String {
    let ext = name.rsplit_once('.').map_or(String::new(), |(_, e)| format!(".{e}"));
    let mut s = msg.to_string();
    for needle in [name, name.rsplit(['/', '\\']).next().unwrap_or(name)] {
        if !needle.is_empty() {
            s = s.replace(needle, &format!("file{ext}"));
        }
    }
    s
}

/// the largest file the importers are given
const MAX_IMPORT_BYTES: usize = 100 << 20;

pub fn recent() -> Vec<String> {
    let all: Vec<String> = crate::store::load("recent.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    // files moved or deleted since, and the same file under two spellings (relative / absolute), drop out
    let mut seen = std::collections::HashSet::new();
    all.into_iter().filter_map(|p| std::path::absolute(&p).ok()).filter(|p| p.exists()).map(|p| p.to_string_lossy().into_owned()).filter(|p| seen.insert(p.clone())).collect()
}

pub fn add_recent(path: &std::path::Path) -> Vec<String> {
    // screenshot / test runs (GCARVE_SHOT) open scratch files: they don't belong in the user's Recent list
    if std::env::var_os("GCARVE_SHOT").is_some() {
        return recent();
    }
    let p = std::path::absolute(path).unwrap_or(path.to_path_buf()).to_string_lossy().into_owned();
    let mut r = recent();
    r.retain(|x| *x != p);
    r.insert(0, p);
    r.truncate(10);
    crate::store::save("recent.json", &serde_json::to_string(&r).unwrap_or_default());
    r
}

pub fn clear_recent() {
    crate::store::save("recent.json", "[]");
}

/// The browser's own file chooser (web build). rfd's web picker wraps it in a pop-up whose "cancel" handler removes
/// its element a second time after a pick and panics (rfd 0.17.2 wasm.rs:166), which took the app down: this one
/// clicks a hidden <input type=file> and only removes it if it's still there.
#[cfg(target_arch = "wasm32")]
/// Files are read in the browser, never uploaded; past this a big file would just freeze the tab.
#[cfg(target_arch = "wasm32")]
const MAX_PICK_BYTES: f64 = 50.0 * 1024.0 * 1024.0;

#[cfg(target_arch = "wasm32")]
fn web_pick(exts: &[&str], ctx: eframe::egui::Context, msg: Sender<String>, done: impl FnOnce(String, Vec<u8>) + 'static) {
    use eframe::wasm_bindgen::{closure::Closure, JsCast};
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let Some(input) = doc.create_element("input").ok().and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok()) else { return };
    input.set_type("file");
    input.set_accept(&exts.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(","));
    let _ = input.style().set_property("display", "none");
    if let Some(b) = doc.body() {
        let _ = b.append_child(&input);
    }
    let remove = {
        let i = input.clone();
        move || {
            if let Some(p) = i.parent_node() {
                let _ = p.remove_child(&i);
            }
        }
    };
    let done = std::rc::Rc::new(std::cell::RefCell::new(Some(done)));
    let on_change = {
        let (input, remove) = (input.clone(), remove.clone());
        Closure::<dyn FnMut()>::new(move || {
            let file = input.files().and_then(|f| f.get(0));
            remove();
            let (Some(file), Some(done)) = (file, done.borrow_mut().take()) else { return };
            if file.size() > MAX_PICK_BYTES {
                let _ = msg.send(format!("{} is {:.0} MB: the web app opens files up to 50 MB", file.name(), file.size() / 1048576.0));
                ctx.request_repaint();
                return;
            }
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if let Ok(buf) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    done(file.name(), js_sys::Uint8Array::new(&buf).to_vec());
                    ctx.request_repaint();
                }
            });
        })
    };
    let on_cancel = Closure::<dyn FnMut()>::new(remove);
    let _ = input.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
    let _ = input.add_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref());
    on_change.forget(); // ponytail: two small closures per pick stay alive; picks are rare
    on_cancel.forget();
    input.click(); // inside the click's user activation (egui runs the command in the same frame)
}

/// Minimal executor for rfd's future on a desktop thread (it only awaits the native dialog).
#[cfg(not(target_arch = "wasm32"))]
fn pollster_block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Save bytes: a native "Save as" dialog, or a browser download on the web. Returns at once; the native dialog
/// runs on its own thread.
pub fn save(name: &str, bytes: Vec<u8>, done: Sender<String>) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = done.send(match download(name, &bytes) { Ok(()) => format!("Downloaded {name}"), Err(e) => e });
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dialog = rfd::AsyncFileDialog::new().set_file_name(name);
        let name = name.to_string();
        std::thread::spawn(move || {
            let r = pollster_block_on(dialog.save_file());
            let msg = match r {
                Some(f) => match std::fs::write(f.path(), &bytes) {
                    Ok(()) => format!("Saved {}", f.path().display()),
                    Err(e) => format!("Cannot save {name}: {e}"),
                },
                None => String::new(),
            };
            let _ = done.send(msg);
        });
    }
}

/// Opens a file or folder with its default app (folder: the file manager; .html: the browser).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_with_os(path: &std::path::Path) {
    let cmd = if cfg!(target_os = "macos") { "open" } else if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
    let _ = std::process::Command::new(cmd).arg(path).spawn();
}

/// An HTML page in a new browser tab (blob URL, so it can print); Ok(false) = the tab was blocked, downloaded instead.
#[cfg(target_arch = "wasm32")]
pub fn open_html_tab(name: &str, html: &str) -> Result<bool, String> {
    let window = web_sys::window().ok_or("no window")?;
    let url = blob_url(html.as_bytes(), "text/html;charset=utf-8")?;
    if window.open_with_url_and_target(&url, "_blank").ok().flatten().is_some() {
        return Ok(true); // the tab keeps the blob URL alive: not revoked
    }
    let _ = web_sys::Url::revoke_object_url(&url);
    download(name, html.as_bytes()).map(|_| false)
}

/// Browser downloads, one per file (the desktop writes into the Export dialog's folder: App::write_export)
#[cfg(target_arch = "wasm32")]
pub fn save_many(files: Vec<(String, Vec<u8>)>, done: Sender<String>) {
    let mut names = Vec::new();
    for (n, b) in &files {
        if let Err(e) = download(n, b) {
            let _ = done.send(e);
            return;
        }
        names.push(n.clone());
    }
    let _ = done.send(format!("Downloaded {}", names.join(", ")));
}

/// G-code file extensions File ▸ Open / Import read (dialogs/gimport.rs).
pub const GCODE_EXTS: &[&str] = &["tap", "nc", "gcode", "ngc", "cnc"];
/// drawings read as vectors (the "Vector files" filter)
pub const VECTOR_EXTS: &[&str] = &["svg", "dxf", "dwg", "pdf", "ai", "eps", "plt", "hpgl"];
/// what Import takes (everything Open does but whole projects), and Open
pub const IMPORT_EXTS: &[&str] = &["svg", "dxf", "dwg", "pdf", "ai", "eps", "plt", "hpgl", "lbrn2", "gsk", "stl", "3mf", "tap", "nc", "gcode", "ngc", "cnc", "png", "jpg", "jpeg", "webp", "csv"];
pub const OPEN_EXTS: &[&str] = &["tlp", "tlpt", "crv", "csv", "svg", "dxf", "dwg", "pdf", "ai", "eps", "plt", "hpgl", "lbrn2", "gsk", "stl", "3mf", "tap", "nc", "gcode", "ngc", "cnc", "png", "jpg", "jpeg", "webp"];
/// Pictures File ▸ Open / Import send to Trace Image (dialogs/trace.rs).
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];

pub enum Loaded {
    /// G-code: the app shows the import dialog (bits confirmed by the user)
    Gcode(String),
    /// a picture: the app shows Trace Image
    Image,
    /// a whole project; the note says what came in (a VCarve file), "" for .tlp
    Project(Box<Document>, String),
    Imported(usize, String), // shapes added, note
    Error(String),
}

/// AutoCAD .dwg → DXF text for the DXF importer (acadrust; records it can't decode are skipped, not fatal).
/// A damaged file can make the reader ask for petabytes, which aborts the process: on desktop the read runs in a
/// copy of this program (`--dwg-to-dxf`, bytes in on stdin, DXF out on stdout), so the worst case is an error message.
fn dwg_to_dxf(bytes: &[u8]) -> Result<String, String> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(exe) = helper_exe() {
        use std::io::{Read, Write};
        let child = std::process::Command::new(exe).arg("--dwg-to-dxf").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn();
        if let Ok(mut child) = child {
            let (mut input, mut out_pipe) = (child.stdin.take(), child.stdout.take());
            let data = bytes.to_vec();
            let writer = std::thread::spawn(move || input.as_mut().map(|i| i.write_all(&data)));
            let mut out = Vec::new();
            let _ = out_pipe.as_mut().map(|o| o.read_to_end(&mut out));
            let _ = writer.join();
            return match child.wait() {
                Ok(st) if st.success() => String::from_utf8(out).map_err(|_| "This DWG file could not be converted. Save it as DXF and open that.".to_string()),
                Ok(_) if !out.is_empty() => Err(String::from_utf8_lossy(&out).into_owned()), // the helper's own message
                _ => Err("This DWG file looks damaged: reading it failed. Save it as DXF from the program that made it and open that.".into()),
            };
        }
    }
    dwg_to_dxf_inproc(bytes)
}

/// The program to run the DWG read in: this one (in tests only when GCARVE_EXE names the built program).
#[cfg(not(target_arch = "wasm32"))]
fn helper_exe() -> Option<PathBuf> {
    match std::env::var_os("GCARVE_EXE") {
        Some(p) => Some(p.into()),
        None if cfg!(test) => None,
        None => std::env::current_exe().ok(),
    }
}

/// The in-process read (the helper process and the web build use it).
pub(crate) fn dwg_to_dxf_inproc(bytes: &[u8]) -> Result<String, String> {
    let read = std::panic::catch_unwind(|| acadrust::DwgReader::from_stream_with_options(std::io::Cursor::new(bytes), acadrust::DwgReadOptions::failsafe()).read_with_stats())
        .map_err(|_| "This DWG file could not be read. Save it as DXF from the program that made it and open that.".to_string())?
        .map_err(|e| format!("This DWG file could not be read ({e}). Save it as DXF and open that."))?;
    if !read.stats.record_stream_read {
        return Err("This DWG file looks damaged or isn't a drawing. Save it as DXF and open that.".into());
    }
    let mut out = Vec::new();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| acadrust::DxfWriter::new(&read.document).write_to_writer(&mut out)))
        .map_err(|_| "This DWG file could not be converted. Save it as DXF and open that.".to_string())?
        .map_err(|e| format!("DWG: {e}"))?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// DXF text → AutoCAD .dwg (2004 format; acadrust's writer).
pub fn dxf_to_dwg(dxf: &str) -> Result<Vec<u8>, String> {
    std::panic::catch_unwind(|| {
        let mut doc = acadrust::DxfReader::from_reader(std::io::Cursor::new(dxf.as_bytes().to_vec())).and_then(|r| r.read()).map_err(|e| format!("DWG: {e}"))?;
        if doc.version < acadrust::DxfVersion::AC1018 {
            doc.version = acadrust::DxfVersion::AC1018; // R12 / 2000 DWG has no true colour etc.; 2004 opens everywhere
        }
        acadrust::DwgWriter::write_to_vec(&doc).map_err(|e| format!("DWG: {e}"))
    })
    .map_err(|_| "Could not write the DWG file. Export DXF instead.".to_string())?
}

/// .tlp and VCarve .crv replace the document; .svg / .dxf / .pdf / .ai add to it (layers from the file).
pub fn load(doc: &mut Document, name: &str, bytes: &[u8]) -> Loaded {
    // a browser tab has a few GB at most, and the importers copy the text: refuse the absurd before reading it
    if bytes.len() > MAX_IMPORT_BYTES {
        return Loaded::Error(format!("That file is {} MB; GCarve opens files up to {} MB. A drawing this big is usually full of pictures or far too many points: simplify it in the program that made it.", bytes.len() >> 20, MAX_IMPORT_BYTES >> 20));
    }
    let mut ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let dwg;
    let text = if ext == "dwg" {
        dwg = match dwg_to_dxf(bytes) {
            Ok(t) => t,
            Err(e) => return Loaded::Error(e),
        };
        ext = "dxf".into();
        std::borrow::Cow::Borrowed(dwg.as_str())
    } else {
        String::from_utf8_lossy(bytes)
    };
    match ext.as_str() {
        "tlp" => match project::load_project(&text) {
            Ok(d) => Loaded::Project(Box::new(d), String::new()),
            Err(e) => Loaded::Error(e),
        },
        // a job template (VCarve .crvt): a project that always opens as a new one, so it is never saved over
        // a cut list: parts as labelled rectangles above the board; the app nests them onto sheets (cutlist.rs)
        "csv" => match crate::cutlist::parse(doc, &text) {
            Ok(parts) => {
                let n = crate::cutlist::add(doc, &parts).len();
                Loaded::Imported(n, format!("cut list: {} parts", parts.iter().map(|p| p.qty).sum::<usize>()))
            }
            Err(e) => Loaded::Error(format!("Cut list: {e}")),
        },
        "tlpt" => match project::load_project(&text) {
            Ok(d) => Loaded::Project(Box::new(d), "a job template, opened as a new project (Save asks where it goes)".into()),
            Err(e) => Loaded::Error(e),
        },
        "crv" => match gcarve_core::crv::import_crv(bytes) {
            Ok((d, note)) => Loaded::Project(Box::new(d), note),
            Err(e) => Loaded::Error(e),
        },
        "svg" | "dxf" | "pdf" | "ai" | "eps" | "lbrn2" | "plt" | "hpgl" => {
            let mut r = match ext.as_str() {
                "svg" => svg::import_svg_with_text(&text),
                "dxf" => dxf::import_dxf_with_text(&text),
                "eps" => gcarve_core::pdf::import_eps(bytes),
                "lbrn2" => gcarve_core::lbrn::import_lbrn(&text),
                "plt" | "hpgl" => gcarve_core::hpgl::import_hpgl(&text),
                _ => gcarve_core::pdf::import_pdf(bytes), // .ai files carry a PDF copy of the artwork
            };
            if ext != name.rsplit('.').next().unwrap_or("").to_ascii_lowercase() {
                r.note = r.note.replacen("DXF", "DWG", 1); // read through its DXF form
            }
            if r.contours.is_empty() && r.texts.is_empty() {
                return Loaded::Error(format!("Nothing to import. {}", r.note));
            }
            doc.checkpoint();
            let n = r.contours.len();
            for (c, layer) in r.contours.into_iter().zip(r.layers) {
                let l = doc.layer_named(&layer);
                doc.add(c, Some(l));
            }
            let placed = crate::text::import_texts(doc, &r.texts);
            if placed > 0 {
                r.note += &format!("; {placed} text kept editable (double-click to change it or its font)");
            }
            Loaded::Imported(n, r.note)
        }
        "gsk" => match import_gsk(doc, &text) {
            Ok((n, note)) => Loaded::Imported(n, note),
            Err(e) => Loaded::Error(e),
        },
        e if GCODE_EXTS.contains(&e) => Loaded::Gcode(text.into_owned()),
        e if IMAGE_EXTS.contains(&e) => Loaded::Image,
        "step" | "stp" | "sldprt" | "sldasm" | "iges" | "igs" | "f3d" | "ipt" | "3dm" => Loaded::Error(format!(
            "GCarve can't read .{ext} (3D CAD) files. In your CAD program export the outline you want to cut as DXF, DWG or SVG (or the shape as STL) and open that."
        )),
        _ => Loaded::Error(format!("GCarve can't open .{ext} files. It opens GCarve projects (.tlp) and job templates (.tlpt), VCarve projects (.crv), SVG, DXF, AutoCAD DWG, PDF, Illustrator (.ai) and EPS, HPGL plot files (.plt), LightBurn (.lbrn2), 3D models (.stl .3mf), G-code (.tap .nc .gcode) and pictures (PNG, JPEG, WebP) to trace.")),
    }
}

/// A GSketch model (.gsk): the flat panels GSketch writes into every save (outline + holes in inches, laid out
/// apart, with the stock thickness) come in as closed vectors, one layer per thickness ("0.75in"). The 3D model
/// itself stays GSketch's. Returns (vectors added, note).
pub fn import_gsk(doc: &mut Document, text: &str) -> Result<(usize, String), String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|_| "That .gsk file is damaged.".to_string())?;
    if v.get("app").and_then(|a| a.as_str()) != Some("GSketch") {
        return Err("That isn't a GSketch model.".into());
    }
    let panels = v.get("panels").and_then(|p| p.as_array()).filter(|p| !p.is_empty()).ok_or("This GSketch model has no flat panels to cut (or it was saved by an older GSketch: open it there and save it again).")?;
    let pts = |l: &serde_json::Value| -> Option<Vec<gcarve_core::geom::Pt>> {
        let a = l.as_array()?;
        let out: Option<Vec<_>> = a.iter().map(|q| Some(gcarve_core::geom::pt(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?))).collect();
        out.filter(|o| o.len() >= 3 && o.iter().all(|p| p.x.is_finite() && p.y.is_finite()))
    };
    doc.checkpoint();
    let mut n = 0;
    for p in panels {
        let t = p.get("thickness").and_then(|t| t.as_f64()).unwrap_or(0.0);
        let layer = doc.layer_named(&format!("{}in", format!("{t:.4}").trim_end_matches('0').trim_end_matches('.')));
        for l in std::iter::once(p.get("outline")).chain(p.get("holes").and_then(|h| h.as_array()).into_iter().flatten().map(Some)).flatten() {
            if let Some(q) = pts(l) {
                doc.add(gcarve_core::geom::make_polyline(&q, true), Some(layer));
                n += 1;
            }
        }
    }
    Ok((n, format!("{} panels from GSketch, one layer per stock thickness", panels.len())))
}

/// A .zip holding `files` as they are (stored, not compressed: G-code is small and every unzip tool reads it).
/// The web export hands over one download instead of one per file.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let le16 = |v: usize| (v as u16).to_le_bytes();
    let le32 = |v: usize| (v as u32).to_le_bytes();
    for (name, data) in files {
        let crc = crc32fast::hash(data);
        let offset = out.len();
        // shared part of the local and the central header: version 2.0, UTF-8 names, stored, 1980-01-01, crc, sizes, name length
        let mut head: Vec<u8> = Vec::new();
        head.extend(le16(20));
        head.extend(le16(0x0800));
        head.extend(le16(0));
        head.extend(le16(0));
        head.extend(le16(0x0021));
        head.extend(crc.to_le_bytes());
        head.extend(le32(data.len()));
        head.extend(le32(data.len()));
        head.extend(le16(name.len()));
        head.extend(le16(0));
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend(&head);
        out.extend(name.as_bytes());
        out.extend(data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(le16(20));
        central.extend(&head);
        central.extend([0u8; 10]); // comment length (2), disk (2), internal (2) + external (4) attributes
        central.extend(le32(offset));
        central.extend(name.as_bytes());
    }
    let dir = out.len();
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(le16(files.len()));
    out.extend(le16(files.len()));
    out.extend(le32(central.len()));
    out.extend(le32(dir));
    out.extend(le16(0));
    out
}

#[cfg(test)]
mod zip_tests {
    /// `unzip` reads it back byte for byte (skipped where there is no unzip)
    #[test]
    fn zip_is_read_by_unzip() {
        let files = vec![("a_01_End_Mill.tap".to_string(), b"G0 X0 Y0\nM5\n".to_vec()), ("sheet_setup.html".to_string(), vec![b'x'; 70_000])];
        let z = super::zip(&files);
        let dir = std::env::temp_dir().join(format!("gcarve-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.zip");
        std::fs::write(&path, &z).unwrap();
        let Ok(out) = std::process::Command::new("unzip").arg("-o").arg(&path).arg("-d").arg(&dir).output() else { return };
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(std::fs::read(dir.join("a_01_End_Mill.tap")).unwrap(), files[0].1);
        assert_eq!(std::fs::read(dir.join("sheet_setup.html")).unwrap().len(), 70_000);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod scrub_tests {
    #[test]
    fn file_names_are_taken_out_of_reported_messages() {
        assert_eq!(super::scrub("Customer Smith sign.pdf: Nothing to import.", "Customer Smith sign.pdf"), "file.pdf: Nothing to import.");
        assert_eq!(super::scrub("Cannot save /Users/jo/Jobs/kitchen.tlp: denied", "/Users/jo/Jobs/kitchen.tlp"), "Cannot save file.tlp: denied");
        assert_eq!(super::scrub("Cannot save C:\\Jobs\\kitchen.tlp: denied", "kitchen.tlp"), "Cannot save C:\\Jobs\\file.tlp: denied");
    }
}

#[cfg(test)]
mod dwg_tests {
    /// a DXF made into a DWG (acadrust's writer) opens with the same shapes; real files: GCARVE_DWG=a.dwg
    #[test]
    fn dwg_opens_like_its_dxf() {
        let dxf = "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1018\n9\n$INSUNITS\n70\n1\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\nCuts\n10\n0\n20\n0\n30\n0\n11\n4\n21\n2\n31\n0\n0\nCIRCLE\n8\nHoles\n10\n2\n20\n1\n30\n0\n40\n0.5\n0\nENDSEC\n0\nEOF\n";
        let doc = acadrust::DxfReader::from_reader(std::io::Cursor::new(dxf.as_bytes().to_vec())).and_then(|r| r.read()).expect("dxf");
        let dwg = acadrust::DwgWriter::write_to_vec(&doc).expect("dwg");
        let mut d = gcarve_core::document::Document::default();
        match super::load(&mut d, "part.DWG", &dwg) {
            super::Loaded::Imported(n, _) => assert_eq!(n, 2),
            super::Loaded::Error(e) => panic!("{e}"),
            _ => panic!("not imported"),
        }
        let back = super::dxf_to_dwg(&gcarve_core::dxf::export_dxf(&[(&gcarve_core::geom::make_rect(0.0, 0.0, 3.0, 2.0), "Cuts")])).expect("export");
        let mut d = gcarve_core::document::Document::default();
        assert!(matches!(super::load(&mut d, "out.dwg", &back), super::Loaded::Imported(1, _)));
        let mut d = gcarve_core::document::Document::default();
        assert!(matches!(super::load(&mut d, "x.dwg", b"AC1032 not really"), super::Loaded::Error(_)));
        if let Ok(p) = std::env::var("GCARVE_DWG_FUZZ") { // damaged copies of a real DWG: no hang, no abort
            let good = std::fs::read(&p).expect("read");
            let mut seed = 0x9E37_79B9_7F4A_7C15u64;
            let mut rnd = move || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
            let worst = (0..300).map(|_| { // set GCARVE_EXE=target/release/gcarve: a damaged file must come back as an error
                let mut b = good.clone();
                for _ in 0..1 + rnd() % 40 { let at = (rnd() % b.len() as u64) as usize; b[at] = rnd() as u8; }
                if rnd() % 4 == 0 { b.truncate((rnd() % b.len() as u64) as usize); }
                let t = std::time::Instant::now();
                let mut d = gcarve_core::document::Document::default();
                let _ = super::load(&mut d, "x.dwg", &b);
                t.elapsed().as_millis()
            }).max().unwrap_or(0);
            println!("dwg fuzz: slowest damaged file {worst} ms");
        }
        if let Ok(dir) = std::env::var("GCARVE_CAD_DIR") { // survey: every .dxf / .dwg under a folder
            fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
                for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                    let p = e.path();
                    if p.is_dir() { walk(&p, out) } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dxf") || x.eq_ignore_ascii_case("dwg")) { out.push(p) }
                }
            }
            let mut all = Vec::new();
            walk(std::path::Path::new(&dir), &mut all);
            let (mut ok, mut bad) = (0, 0);
            for p in all {
                let Ok(b) = std::fs::read(&p) else { continue };
                let mut d = gcarve_core::document::Document::default();
                let t = std::time::Instant::now();
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| super::load(&mut d, &p.to_string_lossy(), &b))) {
                    Ok(super::Loaded::Imported(n, note)) => { ok += 1; println!("OK   {n:5} {:5}ms {} {note}", t.elapsed().as_millis(), p.display()) }
                    Ok(super::Loaded::Error(e)) => { bad += 1; println!("FAIL {} {e}", p.display()) }
                    Ok(_) => {}
                    Err(_) => { bad += 1; println!("PANIC {}", p.display()) }
                }
            }
            println!("survey: {ok} opened, {bad} failed");
        }
        if let Ok(p) = std::env::var("GCARVE_DWG") {
            let mut d = gcarve_core::document::Document::default();
            match super::load(&mut d, &p, &std::fs::read(&p).expect("read")) {
                super::Loaded::Imported(n, note) => println!("{p}: {n} shapes, layers {:?}; {note}", d.layers.iter().map(|l| &l.name).collect::<Vec<_>>()),
                super::Loaded::Error(e) => panic!("{p}: {e}"),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod gsk_tests {
    use super::*;

    #[test]
    fn a_gsketch_model_opens_as_vectors_by_thickness() {
        // what GSketch writes: two 3/4 panels (one with a hole) and a 1/4 back, laid out apart
        let gsk = r#"{"app":"GSketch","version":2,"mesh":{"verts":[],"faces":[]},
            "panels":[{"name":"Side","thickness":0.75,"outline":[[0,0],[24,0],[24,12],[0,12]],"holes":[[[4,4],[6,4],[6,6],[4,6]]]},
                      {"name":"Side","thickness":0.75,"outline":[[25,0],[49,0],[49,12],[25,12]],"holes":[]},
                      {"name":"Back","thickness":0.25,"outline":[[0,13],[30,13],[30,43],[0,43]],"holes":[]}]}"#;
        let mut doc = Document::default();
        match load(&mut doc, "cabinet.gsk", gsk.as_bytes()) {
            Loaded::Imported(n, note) => assert!(n == 4 && note.contains("3 panels"), "{n} {note}"),
            _ => panic!("not imported"),
        }
        let layer = |name: &str| doc.layers.iter().position(|l| l.name == name).map(|i| i as i32);
        let (thick, thin) = (layer("0.75in").expect("0.75in layer"), layer("0.25in").expect("0.25in layer"));
        assert_eq!(doc.shapes.iter().filter(|s| s.layer == thick).count(), 3);
        assert_eq!(doc.shapes.iter().filter(|s| s.layer == thin).count(), 1);
        assert!(doc.shapes.iter().all(|s| s.c.closed));
        // not a GSketch file, or one without panels: a plain refusal
        assert!(matches!(load(&mut Document::default(), "x.gsk", br#"{"app":"Other"}"#), Loaded::Error(_)));
        assert!(matches!(load(&mut Document::default(), "x.gsk", br#"{"app":"GSketch","mesh":{}}"#), Loaded::Error(_)));
        assert!(matches!(load(&mut Document::default(), "x.gsk", b"\xff\xfe junk"), Loaded::Error(_)));
    }

    #[test]
    fn a_plot_file_opens_one_layer_per_pen() {
        // pen 1: a 1 x 1 in square (40 units a mm), pen 2: an open line; .PLT in capitals like CorelDRAW writes it
        let plt = b"IN;SP1;PU0,0;PD1016,0,1016,1016,0,1016,0,0;SP2;PU2032,0;PD3048,0;";
        let mut doc = Document::default();
        assert!(matches!(load(&mut doc, "SIGN.PLT", plt), Loaded::Imported(2, _)));
        let layer = |name: &str| doc.layers.iter().position(|l| l.name == name).map(|i| i as i32).expect(name);
        let (p1, p2) = (layer("Pen 1"), layer("Pen 2"));
        let sq = doc.shapes.iter().find(|s| s.layer == p1).expect("square");
        let r = gcarve_core::geom::bounds(&sq.c);
        assert!(sq.c.closed && (r.w() - 1.0).abs() < 1e-6 && (r.h() - 1.0).abs() < 1e-6);
        assert!(doc.shapes.iter().any(|s| s.layer == p2 && !s.c.closed));
        assert!(OPEN_EXTS.contains(&"plt") && IMPORT_EXTS.contains(&"hpgl"));
    }

    #[test]
    fn a_job_template_opens_as_a_new_project() {
        let mut src = Document::default();
        src.material.w = 24.0;
        src.add(gcarve_core::geom::make_rect(1.0, 1.0, 2.0, 2.0), None);
        let text = gcarve_core::project::save_project(&src);
        let mut doc = Document::default();
        // the note marks it as not a plain .tlp, so the app opens it unsaved with no path (never saved over)
        match load(&mut doc, "sign.tlpt", text.as_bytes()) {
            Loaded::Project(d, note) => assert!(d.material.w == 24.0 && d.shapes.len() == 1 && note.contains("job template")),
            _ => panic!("not opened as a project"),
        }
    }
}
