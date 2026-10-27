//! Language packs for the UI text. `t("Open GCarve")` gives the string in the chosen language, or the English when the
//! pack has no entry (so a new string shows up in English until it is translated: nothing breaks). Packs are
//! `lang/<code>.json` ({English: translated}) embedded at build time; `set(code)` loads one, the choice is kept in the
//! store under `language`. Keys are the English UI strings themselves: the English build has no pack to maintain, and
//! `tools/i18n_keys.py` lists every `t("…")` in the sources so the packs can be filled.
//!
//! Format strings cannot go through `t` (format! needs a literal): status messages built with format! stay English
//! for now, except through `tf` (a runtime `{name}` substitution for the few that matter).

use std::collections::HashMap;
use std::sync::RwLock;

/// (code, name in its own language, the embedded pack)
pub const LANGS: [(&str, &str, &str); 8] = [
    ("en", "English", ""),
    ("es", "Español", include_str!("../lang/es.json")),
    ("de", "Deutsch", include_str!("../lang/de.json")),
    ("fr", "Français", include_str!("../lang/fr.json")),
    ("it", "Italiano", include_str!("../lang/it.json")),
    ("pt", "Português", include_str!("../lang/pt.json")),
    ("ja", "日本語", include_str!("../lang/ja.json")),
    ("zh", "中文", include_str!("../lang/zh.json")),
];

struct Pack {
    code: &'static str,
    map: HashMap<&'static str, &'static str>,
}

static PACK: RwLock<Option<Pack>> = RwLock::new(None);

/// The UI string in the current language (the English itself when there is no translation)
pub fn t(s: &'static str) -> &'static str {
    match PACK.read() {
        Ok(p) => p.as_ref().and_then(|p| p.map.get(s).copied()).unwrap_or(s),
        _ => s,
    }
}

/// `t` for a string that is not a literal (a table entry): same lookup, same fallback
pub fn tr(s: &str) -> String {
    match PACK.read() {
        Ok(p) => p.as_ref().and_then(|p| p.map.get(s)).map_or_else(|| s.to_string(), |v| v.to_string()),
        _ => s.to_string(),
    }
}

/// A translated format string: `tf("Added the font {name}", &[("name", &family)])` substitutes every `{key}`
pub fn tf(s: &'static str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = t(s).to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), &v.to_string());
    }
    out
}

pub fn current() -> &'static str {
    PACK.read().ok().and_then(|p| p.as_ref().map(|p| p.code)).unwrap_or("en")
}

/// Switch the UI language (unknown codes = English) and remember it
pub fn set(code: &str) {
    let code = code.get(..2).unwrap_or("en");
    let (code, _, json) = LANGS.iter().copied().find(|(c, _, _)| *c == code).unwrap_or(LANGS[0]);
    let map: HashMap<&'static str, &'static str> = if json.is_empty() {
        HashMap::new()
    } else {
        // the pack lives for the program (strings with escapes cannot be borrowed from the JSON text)
        serde_json::from_str::<HashMap<String, String>>(json).unwrap_or_default().into_iter().map(|(k, v)| (&*k.leak(), &*v.leak())).collect()
    };
    if let Ok(mut p) = PACK.write() {
        *p = Some(Pack { code, map });
    }
    if !cfg!(test) {
        crate::store::save("language", code);
    }
}

/// At start: `?lang=xx` in the page address (the website's language hint links here), else the stored choice, else
/// the system / browser language when we have it, else English
pub fn init() {
    #[cfg(target_arch = "wasm32")]
    let asked = web_sys::window().and_then(|w| w.location().search().ok()).and_then(|q| q.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix("lang=").map(str::to_string)));
    #[cfg(not(target_arch = "wasm32"))]
    let asked: Option<String> = std::env::var("GCARVE_LANG").ok();
    let code = asked.or_else(|| crate::store::load("language")).unwrap_or_else(system_language);
    if code != "en" {
        set(&code);
    }
}

fn system_language() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(l) = web_sys::window().and_then(|w| w.navigator().language()) {
            return l.to_lowercase();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(v) = std::env::var(k) {
                if v.len() >= 2 && v != "C" && v != "POSIX" {
                    return v.to_lowercase();
                }
            }
        }
    }
    "en".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pack_parses_and_an_unknown_string_falls_back_to_english() {
        for (code, _, json) in LANGS.iter().skip(1) {
            let map: HashMap<String, String> = serde_json::from_str(json).unwrap_or_else(|e| panic!("{code}: {e}"));
            assert!(!map.is_empty(), "{code} is empty");
            assert!(map.values().all(|v| !v.is_empty()), "{code} has empty translations");
        }
        set("es");
        assert_eq!(t("not a string of the app"), "not a string of the app");
        assert_eq!(tf("{n} of {m}", &[("n", &1), ("m", &"two")]), "1 of two");
        set("en");
        assert_eq!(current(), "en");
    }
}
