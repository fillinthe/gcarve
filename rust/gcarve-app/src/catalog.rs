//! The embedded machine + bit databases and their two pickers (Board dialog: machine; Tool Library: bits).

use eframe::egui;
use gcarve_core::catalog::Catalog;
use gcarve_core::document::MachineLimits;
use std::sync::OnceLock;

mod files {
    include!(concat!(env!("OUT_DIR"), "/catalog_files.rs"));
}

static LOADED: OnceLock<Catalog> = OnceLock::new();

/// Desktop: the embedded files, parsed on first use (~0.1 s). Web: the copy fetched by `prefetch` (empty until it
/// arrives; the pickers say so).
pub fn get() -> &'static Catalog {
    #[cfg(not(target_arch = "wasm32"))]
    return LOADED.get_or_init(|| Catalog::load(files::MACHINES, files::BITS, files::CHIPLOAD).0);
    #[cfg(target_arch = "wasm32")]
    {
        static EMPTY: OnceLock<Catalog> = OnceLock::new();
        let _ = files::MACHINES;
        LOADED.get().unwrap_or_else(|| EMPTY.get_or_init(Catalog::default))
    }
}

/// Web: "loading" / an error while the database isn't here yet; None = ready (always on the desktop).
pub fn status() -> Option<String> {
    if LOADED.get().is_some() {
        return None;
    }
    #[cfg(target_arch = "wasm32")]
    return Some(FETCH_ERR.get().cloned().unwrap_or_else(|| "Loading the bit & machine database…".into()));
    #[cfg(not(target_arch = "wasm32"))]
    None
}

#[cfg(target_arch = "wasm32")]
static FETCH_ERR: OnceLock<String> = OnceLock::new();

/// Web: start downloading /data/catalog.json (same origin as the app) once; repaint when it lands. The URL carries
/// the version from /data/catalog.ver, so browsers and Cloudflare keep the big file until the data changes.
pub fn prefetch(ctx: &egui::Context) {
    #[cfg(target_arch = "wasm32")]
    {
        static STARTED: OnceLock<()> = OnceLock::new();
        if STARTED.set(()).is_err() {
            return;
        }
        let origin = web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default();
        let ctx = ctx.clone();
        let base = format!("{origin}/data/");
        ehttp::fetch(ehttp::Request::get(format!("{base}catalog.ver")), move |v| {
            let ver = v.ok().filter(|v| v.ok).and_then(|v| v.text().map(|t| t.trim().to_string())).filter(|t| t.len() <= 40 && t.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or_default();
            fetch_catalog(format!("{base}catalog.json?v={ver}"), ctx);
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = ctx;
}

#[cfg(target_arch = "wasm32")]
fn fetch_catalog(url: String, ctx: egui::Context) {
    {
        ehttp::fetch(ehttp::Request::get(url), move |r| {
            let parsed = r.map_err(|e| e.to_string()).and_then(|r| if r.ok { Catalog::from_published(r.text().unwrap_or_default()) } else { Err(format!("HTTP {}", r.status)) });
            match parsed {
                Ok(c) => {
                    let _ = LOADED.set(c);
                }
                Err(e) => {
                    let _ = FETCH_ERR.set(format!("Bit & machine database not available ({e}): reload to try again"));
                }
            }
            ctx.request_repaint();
        });
    }
}


/// One line describing the limits in force.
pub fn limits_line(m: &MachineLimits) -> String {
    let mut parts = Vec::new();
    if m.rpm_max > 0.0 {
        parts.push(if m.rpm_min > 0.0 { format!("{:.0}–{:.0} RPM", m.rpm_min, m.rpm_max) } else { format!("≤ {:.0} RPM", m.rpm_max) });
    }
    if m.feed_xy > 0.0 {
        parts.push(format!("≤ {:.0} in/min", m.feed_xy));
    }
    if m.travel_x > 0.0 {
        parts.push(format!("travel {:.0} × {:.0} × {:.0} in", m.travel_x, m.travel_y, m.travel_z));
    }
    if m.tool_slots > 0 {
        parts.push(format!("ATC {} slots", m.tool_slots));
    }
    let (kc, _) = gcarve_core::pipeline::class_factor(&m.class);
    if kc < 1.0 {
        parts.push(format!("{} machine: suggested feeds at {:.0}% of the industrial chip load", m.class, kc * 100.0));
    }
    parts.join(" · ")
}
