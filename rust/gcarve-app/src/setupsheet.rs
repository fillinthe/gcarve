//! File ▸ Setup Sheet… and the Export dialog's "Setup sheet" box: the printable job sheet (gcarve-core setupsheet)
//! with the shaded simulation as its picture. Desktop: opened in the default browser to print; web: a new tab.

use gcarve_core::document::Document;
use gcarve_core::pipeline::{self, Generated, SheetPicture};
use gcarve_core::postdef::PostDef;
use gcarve_core::setupsheet::base64;
use gcarve_core::sim::{shade, simulate};

/// The shaded simulation of one sheet as a PNG data URI + where it sits on the board.
fn picture(d: &Document, g: &Generated) -> SheetPicture {
    let eff: Vec<_> = (0..d.tools.len()).map(|i| pipeline::tool_for_cut(d, i)).collect();
    let mut paths = Vec::new();
    for (op, ps) in d.ops.iter().zip(&g.paths) {
        for tp in ps {
            let ti = if tp.tool >= 0 { tp.tool } else { op.tool };
            if let Some(t) = usize::try_from(ti).ok().and_then(|i| eff.get(i)) {
                paths.push((tp, t));
            }
        }
    }
    if paths.is_empty() {
        return None;
    }
    let hf = simulate(&paths, &d.material, 400_000); // small enough to keep the HTML light
    let rgba = shade(&hf, &d.material);
    let mut png_bytes = Vec::new();
    let mut enc = png::Encoder::new(&mut png_bytes, hf.nx as u32, hf.ny as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().ok()?;
    w.write_image_data(&rgba).ok()?;
    w.finish().ok()?;
    Some((format!("data:image/png;base64,{}", base64(&png_bytes)), [hf.x0, hf.y0, f64::from(hf.nx) * hf.res, f64::from(hf.ny) * hf.res]))
}

/// The sheet for an export with these settings (`names` = the export dialog's file names, empty = the post's).
pub fn html(doc: &Document, gen: &Generated, project: &str, air_cut_raise: f64, post: &PostDef, names: &[String]) -> String {
    let date = chrono::Local::now().format("%-d %B %Y, %I:%M %p").to_string();
    pipeline::setup_sheet(doc, gen, project, &date, air_cut_raise, post, names, &mut picture)
}

/// Shows it for printing: the default browser on the desktop (a temp file), a new tab on the web (download when
/// the browser blocks the tab). Returns the status message.
pub fn open(project: &str, html: &str) -> String {
    let name = format!("{project}_setup.html");
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = std::env::temp_dir().join(&name);
        match std::fs::write(&path, html) {
            Ok(()) => {
                crate::files::open_with_os(&path);
                "Setup sheet opened in your browser".into()
            }
            Err(e) => format!("Cannot write {}: {e}", path.display()),
        }
    }
    #[cfg(target_arch = "wasm32")]
    match crate::files::open_html_tab(&name, html) {
        Ok(true) => "Setup sheet opened in a new tab".into(),
        Ok(false) => format!("Downloaded {name}"),
        Err(e) => e,
    }
}
