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

/// A system font with Japanese / Chinese glyphs, for the ja and zh packs (the UI font has none); the web has no
/// system fonts, so there the packs show boxes until a web font is added. (collection index 0 everywhere)
#[cfg(not(target_arch = "wasm32"))]
fn system_cjk(lang: &str) -> Option<Vec<u8>> {
    let (ja, zh): (&[&str], &[&str]) = (
        &["/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "C:\\Windows\\Fonts\\meiryo.ttc", "C:\\Windows\\Fonts\\YuGothM.ttc", "C:\\Windows\\Fonts\\msgothic.ttc", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc"],
        &["/System/Library/Fonts/Hiragino Sans GB.ttc", "/System/Library/Fonts/PingFang.ttc", "C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\simhei.ttf", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc"],
    );
    let list = match lang {
        "ja" => ja,
        "zh" => zh,
        _ => return None,
    };
    list.iter().chain(if lang == "ja" { zh } else { ja }).find_map(|p| std::fs::read(p).ok())
}

/// The web has no system fonts: a Noto Sans CJK subset (web/fonts/cjk-<lang>.otf, OFL) is fetched when Japanese or
/// Chinese is chosen, kept here, and `install` runs again with it.
#[cfg(target_arch = "wasm32")]
static WEB_CJK: std::sync::Mutex<Option<(&'static str, Vec<u8>)>> = std::sync::Mutex::new(None);

/// Web: fetch the CJK font the current language needs (nothing for the others, nothing twice), then reinstall the fonts
pub fn fetch_cjk(ctx: &egui::Context) {
    #[cfg(target_arch = "wasm32")]
    {
        let lang = crate::i18n::current();
        if !matches!(lang, "ja" | "zh") || WEB_CJK.lock().map(|g| g.as_ref().is_some_and(|(l, _)| *l == lang)).unwrap_or(true) {
            return;
        }
        // next to the app's own files (/app/fonts/…), whatever the host
        let href = web_sys::window().and_then(|w| w.location().href().ok()).unwrap_or_default();
        let dir = href.split(['?', '#']).next().unwrap_or("").rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
        let ctx = ctx.clone();
        ehttp::fetch(ehttp::Request::get(format!("{dir}fonts/cjk-{lang}.otf")), move |r| {
            if let Ok(r) = r {
                if r.ok && !r.bytes.is_empty() {
                    if let Ok(mut g) = WEB_CJK.lock() {
                        *g = Some((lang, r.bytes));
                    }
                    install(&ctx);
                    ctx.request_repaint();
                }
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = ctx;
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
    #[cfg(not(target_arch = "wasm32"))]
    let cjk = system_cjk(crate::i18n::current());
    #[cfg(target_arch = "wasm32")]
    let cjk = WEB_CJK.lock().ok().and_then(|g| g.as_ref().filter(|(l, _)| *l == crate::i18n::current()).map(|(_, b)| b.clone()));
    if let Some(cjk) = cjk {
        f.font_data.insert("cjk".into(), Arc::new(FontData::from_owned(cjk)));
        f.families.get_mut(&FontFamily::Proportional).unwrap().push("cjk".into());
    }
    // bold falls back to the regular chain (symbols, emoji, cjk) for glyphs it lacks
    let mut chain = vec!["ui-bold".to_owned()];
    chain.extend(f.families[&FontFamily::Proportional].iter().skip(1).cloned());
    f.families.insert(FontFamily::Name(BOLD.into()), chain);
    if let Some(m) = mono {
        f.font_data.insert("mono".into(), Arc::new(FontData::from_owned(m)));
        f.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "mono".into());
    }
    ctx.set_fonts(f);
}
