//! File ▸ Setup Sheet… and the Export dialog's "Setup sheet" box: the printable job sheet (gcarve-core setupsheet)
//! with the shaded simulation as its picture. Desktop: opened in the default browser to print; web: a new tab.

use gcarve_core::document::Document;
use gcarve_core::pipeline::{self, Generated, SheetPicture};
use gcarve_core::postdef::PostDef;
use gcarve_core::setupsheet::base64;
use gcarve_core::sim::{shade, simulate};

/// The carved board of the active sheet as straight RGBA (top row first; through cuts transparent), simulated on
/// about `cells` grid cells, + where it sits on the board. None when nothing cuts.
pub fn carved(d: &Document, g: &Generated, cells: usize) -> Option<(Vec<u8>, u32, u32, [f64; 4])> {
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
    let hf = simulate(&paths, &d.material, cells);
    let rgba = shade(&hf, &d.material);
    Some((rgba, hf.nx as u32, hf.ny as u32, [hf.x0, hf.y0, f64::from(hf.nx) * hf.res, f64::from(hf.ny) * hf.res]))
}

/// The shaded simulation of one sheet as a PNG data URI + where it sits on the board.
fn picture(d: &Document, g: &Generated) -> SheetPicture {
    let (rgba, nx, ny, at) = carved(d, g, 400_000)?; // small enough to keep the HTML light
    Some((format!("data:image/png;base64,{}", base64(&png(&rgba, nx, ny)?)), at))
}

/// File › Save Preview Image (VCarve's): the carved board of this sheet, finely simulated, as a PNG
/// (`cells` = simulation grid size: 4 M for a crisp picture)
pub fn preview_png(d: &Document, g: &Generated, cells: usize) -> Option<Vec<u8>> {
    let (rgba, nx, ny, _) = carved(d, g, cells)?;
    png(&rgba, nx, ny)
}

fn png(rgba: &[u8], nx: u32, ny: u32) -> Option<Vec<u8>> {
    let mut png_bytes = Vec::new();
    let mut enc = png::Encoder::new(&mut png_bytes, nx, ny);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().ok()?;
    w.write_image_data(rgba).ok()?;
    w.finish().ok()?;
    Some(png_bytes)
}

/// The sheet for an export with these settings (`names` = the export dialog's file names, empty = the post's).
pub fn html(doc: &Document, gen: &Generated, project: &str, air_cut_raise: f64, post: &PostDef, names: &[String]) -> String {
    let date = chrono::Local::now().format("%-d %B %Y, %I:%M %p").to_string();
    let shop = crate::store::load(SHOP_KEY).unwrap_or_default();
    branded(&pipeline::setup_sheet(doc, gen, project, &date, air_cut_raise, post, names, &mut picture), shop.trim(), &crate::store::load(LOGO_KEY).unwrap_or_default())
}

/// Preferences › Setup sheet (VCarve's Job Setup Sheet Editor): the shop's name and logo, kept on this device
pub const SHOP_KEY: &str = "sheet-shop";
pub const LOGO_KEY: &str = "sheet-logo";

/// the shop's logo (a data URI) and name at the top of every page; the shared sheet itself stays as it is
fn branded(html: &str, shop: &str, logo: &str) -> String {
    if shop.is_empty() && !logo.starts_with("data:image/") {
        return html.to_string();
    }
    let esc = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let img = if logo.starts_with("data:image/") { format!("<img src=\"{}\" alt=\"\" style=\"max-height:52px;max-width:220px;display:block;margin-bottom:6px\">", esc(logo)) } else { String::new() };
    let name = if shop.is_empty() { String::new() } else { format!("<p style=\"margin:0 0 4px;font-weight:600\">{}</p>", esc(shop)) };
    html.replace("<header><div><h1>", &format!("<header><div>{img}{name}<h1>"))
}

/// a picked logo → a PNG data URI at most 440 px wide (the sheet shows it 220 px wide, sharp on print)
pub fn logo_uri(name: &str, bytes: &[u8]) -> Result<String, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("{name}: not a picture GCarve can read ({e})"))?;
    let img = if img.width() > 440 { img.resize(440, 440 * img.height() / img.width().max(1), image::imageops::FilterType::Triangle) } else { img };
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(format!("data:image/png;base64,{}", base64(png.get_ref())))
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

#[cfg(test)]
mod brand_test {
    #[test]
    fn the_preview_image_is_a_png_of_the_carving() {
        use gcarve_core::document::{Document, Op, OpType, Tool};
        let mut d = Document::default();
        let id = d.add(gcarve_core::geom::make_rect(2.0, 2.0, 6.0, 5.0), None);
        d.tools = vec![Tool { name: "E".into(), diameter: 0.25, stepdown: 0.25, ..Default::default() }];
        d.ops.push(Op { kind: OpType::Pocket, name: "P".into(), depth: 0.2, shape_ids: vec![id], ..Default::default() });
        let g = gcarve_core::pipeline::regenerate(&d);
        let png = super::preview_png(&d, &g, 20_000).expect("a picture");
        assert_eq!(&png[1..4], b"PNG");
    }

    #[test]
    fn the_shop_name_and_logo_head_every_page() {
        let sheet = "<section><header><div><h1>A</h1></div></header></section><section><header><div><h1>B</h1></div></header></section>";
        assert_eq!(super::branded(sheet, "", ""), sheet);
        let b = super::branded(sheet, "Pete's <Shop>", "data:image/png;base64,AAAA");
        assert_eq!(b.matches("Pete's &lt;Shop&gt;").count(), 2);
        assert_eq!(b.matches("<img src=\"data:image/png;base64,AAAA\"").count(), 2);
        assert!(super::branded(sheet, "", "javascript:x").eq(sheet));
    }
}
