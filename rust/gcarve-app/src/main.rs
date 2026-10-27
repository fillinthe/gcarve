//! GCarve app entry: native window (Windows, macOS, Linux) or the browser canvas (wasm32). Same App either way.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
mod app;
mod app_icons;
mod allsheets;
// shared with GSketch (../gcarve-ui): `crate::theme` etc. keep working
use gcarve_ui::{cloud, fonts, icons, recover, spacemouse, store, theme, widgets};
#[cfg(all(feature = "screenshot", not(target_arch = "wasm32")))]
use gcarve_ui::shot;
mod bands;
mod bitbrowser;
mod canvas;
mod catalog;
mod control;
mod cutlist; // GCarve Control (machine screen): `gcarve --control [file]`, `?control` on the web
mod edit_ops;
mod files;
mod guides;
mod palette;
mod params;
mod popups;
mod reliefs;
mod setupsheet;
mod inspector;
mod layers;
mod machinebrowser;
mod mouse;
mod onboarding;
mod library;
mod shell;
mod tabs;
mod templates;
mod simview;
// ---- CAM dialogs / simulation ----
mod dialogs;
#[cfg(not(target_arch = "wasm32"))]
mod jobs;
mod playback;
mod posts;
mod toollib;
mod vtdb;
mod view3d;
mod text;
mod tools;
mod transform;
mod units;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    if std::env::args().nth(1).as_deref() == Some("--dwg-to-dxf") {
        // helper for files::dwg_to_dxf: success = DXF text on stdout, failure = exit 1 with the message on stdout
        use std::io::{Read, Write};
        let mut bytes = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut bytes);
        let (text, code) = match files::dwg_to_dxf_inproc(&bytes) {
            Ok(t) => (t, 0),
            Err(e) => (e, 1),
        };
        let _ = std::io::stdout().write_all(text.as_bytes());
        std::process::exit(code);
    }
    if std::env::args().any(|a| a == "--control") {
        return control::run_native();
    }
    // a crash can't reach the network from a dying thread: it is written to the config folder and sent at the next
    // start (signed in only, see app.rs `report_last_crash`)
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        store::save("crash.txt", &format!("v{} {}: {info} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, app::command_trail()));
    }));
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title(i18n::t("GCarve")).with_inner_size(dev_size().unwrap_or([1500.0, 900.0])).with_min_inner_size(if dev_size().is_some() { [320.0, 400.0] } else { [900.0, 500.0] }).with_icon(window_icon()),
        ..Default::default()
    };
    eframe::run_native(i18n::t("GCarve"), options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast;
    // a crash is reported to the site's error log (admin overview) as well as the browser console
    std::panic::set_hook(Box::new(|info| {
        let trail = app::command_trail();
        let msg = if trail.is_empty() { info.to_string() } else { format!("{info} ({trail})") };
        web_sys::console::error_1(&msg.clone().into());
        store::save("crash.txt", &format!("v{} web: {msg}", env!("CARGO_PKG_VERSION"))); // offered as a report after Reload
        // the canvas stops answering after a panic: say so, and offer the reload (unsaved work is restored by recover.rs)
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if doc.get_element_by_id("gcarve-crash").is_none() {
                if let Ok(el) = doc.create_element("div") {
                    el.set_id("gcarve-crash");
                    let _ = el.set_attribute("style", i18n::t("position:fixed;left:50%;top:16px;transform:translateX(-50%);z-index:9999;background:#3a1f1a;color:#fff;border:1px solid #e0603c;border-radius:8px;padding:12px 16px;font:14px system-ui,sans-serif;max-width:90vw"));
                    el.set_inner_html(i18n::t("GCarve hit a problem and stopped. It has been reported. <a href=\"\" style=\"color:#8ec5ff\">Reload</a> to continue: a copy of your drawing from a few seconds ago is offered back, and you can tell us what you were doing."));
                    if let Some(body) = doc.body() {
                        let _ = body.append_child(&el);
                    }
                }
            }
        }
        gcarve_ui::cloud::report_panic(&msg);
    }));
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document.get_element_by_id("gcarve").unwrap().dyn_into::<web_sys::HtmlCanvasElement>().unwrap();
        let result = if control::requested() {
            eframe::WebRunner::new().start(canvas, eframe::WebOptions::default(), Box::new(|cc| Ok(Box::new(control::ControlApp::new(cc, None))))).await
        } else {
            eframe::WebRunner::new().start(canvas, eframe::WebOptions::default(), Box::new(|cc| Ok(Box::new(app::App::new(cc))))).await
        };
        if let Some(loading) = document.get_element_by_id("loading") {
            match result {
                Ok(_) => loading.remove(),
                Err(e) => loading.set_inner_html(&format!("GCarve couldn't start in this browser: {e:?}")),
            }
        }
    });
}

/// dev: `GCARVE_SIZE=1024x530` opens the window at that size (screenshots of small-screen layouts)
#[cfg(not(target_arch = "wasm32"))]
/// The window / taskbar / Dock icon. Without one eframe shows its own default icon: on macOS it sets that as the
/// Dock icon at start-up, hiding the bundle's appicon.icns (the owner saw a black hexagon, 2026-10-17).
fn window_icon() -> std::sync::Arc<eframe::egui::IconData> {
    let (width, height, rgba) = gcarve_ui::shot::png_rgba(include_bytes!("../../../app/branding/appicon-512.png")).unwrap_or((1, 1, vec![0; 4]));
    std::sync::Arc::new(eframe::egui::IconData { rgba, width, height })
}

fn dev_size() -> Option<[f32; 2]> {
    let v = std::env::var("GCARVE_SIZE").ok()?;
    let (w, h) = v.split_once('x')?;
    Some([w.parse().ok()?, h.parse().ok()?])
}
