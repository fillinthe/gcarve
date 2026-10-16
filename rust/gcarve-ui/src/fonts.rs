//! UI fonts: the platform UI font like the Qt app (SF Pro on macOS, Segoe UI on Windows, the desktop default on
//! Linux) and Theme.mono for code; the browser can't read system fonts, so the web build (and any desktop without
//! them) uses the bundled Inter (assets/InterVariable.ttf, OFL). Bold text uses the family `theme::bold()` names.

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use eframe::epaint::text::{FontTweak, VariationCoords};
use std::sync::Arc;

const INTER: &[u8] = include_bytes!("../assets/InterVariable.ttf");
pub const BOLD: &str = "bold";

/// (regular, bold) font files to try, first readable wins; a variable font may serve both (weights set by `wght`)
#[cfg(not(target_arch = "wasm32"))]
fn system_ui() -> Option<(Vec<u8>, Vec<u8>)> {
    let pairs: &[(&str, &str)] = &[
        ("/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/SFNS.ttf"),
        ("C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\segoeuib.ttf"),
        ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf"),
        ("/usr/share/fonts/noto/NotoSans-Regular.ttf", "/usr/share/fonts/noto/NotoSans-Bold.ttf"),
        ("/usr/share/fonts/google-noto/NotoSans-Regular.ttf", "/usr/share/fonts/google-noto/NotoSans-Bold.ttf"),
        ("/usr/share/fonts/cantarell/Cantarell-VF.otf", "/usr/share/fonts/cantarell/Cantarell-VF.otf"),
        ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
        ("/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"),
    ];
    pairs.iter().find_map(|(r, b)| Some((std::fs::read(r).ok()?, std::fs::read(b).ok()?)))
}

#[cfg(not(target_arch = "wasm32"))]
fn system_mono() -> Option<Vec<u8>> {
    // Theme.mono: Menlo / Consolas / monospace
    ["/System/Library/Fonts/Menlo.ttc", "C:\\Windows\\Fonts\\consola.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", "/usr/share/fonts/TTF/DejaVuSansMono.ttf", "/usr/share/fonts/noto/NotoSansMono-Regular.ttf"]
        .iter()
        .find_map(|p| std::fs::read(p).ok())
}

/// Arrows and symbols the UI font lacks (⇤ ⇥ ⤒ ⇄ …, which Qt finds through font fallback)
#[cfg(not(target_arch = "wasm32"))]
fn system_symbols() -> Option<Vec<u8>> {
    ["/System/Library/Fonts/Apple Symbols.ttf", "C:\\Windows\\Fonts\\seguisym.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans.ttf"].iter().find_map(|p| std::fs::read(p).ok())
}

fn weighted(bytes: Vec<u8>, wght: f32) -> FontData {
    let mut d = FontData::from_owned(bytes);
    // static fonts have no axes: ignored. opsz 17 = SF Pro's smallest ("Text") optical size, what CoreText uses at UI sizes
    d.tweak = FontTweak { coords: VariationCoords::new([(b"wght", wght), (b"opsz", 17.0)]), ..Default::default() };
    d
}

pub fn install(ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    let (ui, mono) = (system_ui(), system_mono());
    #[cfg(target_arch = "wasm32")]
    let (ui, mono): (Option<(Vec<u8>, Vec<u8>)>, Option<Vec<u8>>) = (None, None);
    let (reg, bold) = ui.unwrap_or_else(|| (INTER.to_vec(), INTER.to_vec()));
    let mut f = FontDefinitions::default();
    f.font_data.insert("ui".into(), Arc::new(weighted(reg, 400.0)));
    f.font_data.insert("ui-bold".into(), Arc::new(weighted(bold, 700.0)));
    f.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "ui".into());
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(sym) = system_symbols() {
        f.font_data.insert("symbols".into(), Arc::new(FontData::from_owned(sym)));
        f.families.get_mut(&FontFamily::Proportional).unwrap().push("symbols".into());
    }
    // bold falls back to the regular chain (symbols, emoji) for glyphs it lacks
    let mut chain = vec!["ui-bold".to_owned()];
    chain.extend(f.families[&FontFamily::Proportional].iter().skip(1).cloned());
    f.families.insert(FontFamily::Name(BOLD.into()), chain);
    if let Some(m) = mono {
        f.font_data.insert("mono".into(), Arc::new(FontData::from_owned(m)));
        f.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "mono".into());
    }
    ctx.set_fonts(f);
}
