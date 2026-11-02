//! Language packs for the UI text. `t("Open GCarve")` gives the string in the chosen language, or the English when the
//! pack has no entry (so a new string shows up in English until it is translated: nothing breaks). Packs are
//! `lang/<code>.json` ({English: translated}) embedded at build time; `set(code)` loads one, the choice is kept in the
//! store under `language`. Keys are the English UI strings themselves: the English build has no pack to maintain, and
//! `tools/i18n_keys.py` lists every `t("…")` in the sources so the packs can be filled.
//!
//! Format strings cannot go through `t` (format! needs a literal): messages go through `tf` (a runtime
//! `{name}` substitution), and the engine's English warnings are read back into a pattern by `msg`.

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
    /// the keys with `{name}`s, most fixed text first (so "{name}: {x}" never beats a fuller pattern), for `msg`
    patterns: Vec<(&'static str, &'static str)>,
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
    let vals: Vec<(&str, String)> = args.iter().map(|(k, v)| (*k, v.to_string())).collect();
    fill(t(s), |n| vals.iter().find(|(k, _)| *k == n).map(|(_, v)| v.as_str()))
}

/// `text` with each `{name}` that `val` knows replaced, in one pass (a value that itself holds "{x}" stays as it is)
pub fn fill<'v>(text: &str, val: impl Fn(&str) -> Option<&'v str>) -> String {
    let Some(ps) = pieces(text) else { return text.into() };
    let mut out = String::from(ps[0].1);
    for (n, lit) in &ps[1..] {
        match val(n) {
            Some(v) => out.push_str(v),
            None => {
                out.push('{');
                out.push_str(n);
                out.push('}');
            }
        }
        out.push_str(lit);
    }
    out
}

/// The reverse of `tf` for text the engine writes in English (warnings, safety checks): a pack key whose `{name}`s
/// match `s` gives its translation with the matched pieces put back; no match = `s` as it is
pub fn msg(s: &str) -> String {
    let Ok(p) = PACK.read() else { return s.into() };
    let Some(p) = p.as_ref() else { return s.into() };
    if let Some(v) = p.map.get(s) {
        return v.to_string();
    }
    for (k, v) in &p.patterns {
        if let Some(caps) = match_pattern(k, s) {
            return fill(v, |n| caps.iter().find(|(c, _)| *c == n).map(|(_, v)| *v));
        }
    }
    s.into()
}

/// `{name}` placeholders of a key with the text between them: [("", "lit"), ("name", "lit after it"), …]
fn pieces(k: &str) -> Option<Vec<(&str, &str)>> {
    let mut out = Vec::new();
    let (mut name, mut rest) = ("", k);
    loop {
        // the next `{ident}` (other braces are plain text)
        let mut at = None;
        let mut from = 0;
        while let Some(i) = rest[from..].find('{').map(|i| i + from) {
            if let Some(j) = rest[i + 1..].find('}') {
                let n = &rest[i + 1..i + 1 + j];
                if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    at = Some((i, n));
                    break;
                }
            }
            from = i + 1;
        }
        match at {
            Some((i, n)) => {
                out.push((name, &rest[..i]));
                name = n;
                rest = &rest[i + n.len() + 2..];
            }
            None => {
                out.push((name, rest));
                return (out.len() > 1).then_some(out);
            }
        }
    }
}

/// what each `{name}` of pattern `k` stands for in `s`, or None when `s` isn't made from it. Each piece takes the
/// shortest text up to a following fixed part that lets the rest match too; the plural bits `{s}`, `{s2}` … are one
/// word ("are", "them"); two placeholders side by side never match.
pub fn match_pattern<'a>(k: &'a str, s: &'a str) -> Option<Vec<(&'a str, &'a str)>> {
    let ps = pieces(k)?;
    let rest = s.strip_prefix(ps[0].1)?;
    let mut caps = Vec::new();
    match_rest(&ps[1..], rest, &mut caps).then_some(caps)
}

fn match_rest<'a>(ps: &[(&'a str, &'a str)], rest: &'a str, caps: &mut Vec<(&'a str, &'a str)>) -> bool {
    let Some(&(n, lit)) = ps.first() else { return rest.is_empty() };
    let word = n.starts_with('s') && n.len() <= 2;
    let ok = |c: &str| !word || !c.contains(char::is_whitespace);
    if ps.len() == 1 {
        let Some(c) = rest.strip_suffix(lit) else { return false };
        caps.push((n, c));
        return ok(c);
    }
    if lit.is_empty() {
        return false;
    }
    for (at, _) in rest.match_indices(lit) {
        let c = &rest[..at];
        if !ok(c) {
            break; // longer captures only add more words
        }
        caps.push((n, c));
        if match_rest(&ps[1..], &rest[at + lit.len()..], caps) {
            return true;
        }
        caps.truncate(caps.len() - 1);
    }
    false
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
    let mut patterns: Vec<(&'static str, &'static str)> = map.iter().filter(|(k, _)| pieces(k).is_some()).map(|(k, v)| (*k, *v)).collect();
    patterns.sort_by_cached_key(|(k, _)| (std::cmp::Reverse(pieces(k).map_or(0, |p| p.iter().map(|(_, l)| l.len()).sum::<usize>())), *k));
    if let Ok(mut p) = PACK.write() {
        *p = Some(Pack { code, map, patterns });
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
            // every {name} of a tf string survives (the English plural {s}, {s2} may go), and no new ones appear
            let names = |s: &str| -> std::collections::BTreeSet<String> {
                s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string())).filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).collect()
            };
            for (k, v) in &map {
                let (want, got) = (names(k), names(v));
                assert!(got.is_subset(&want), "{code}: {v:?} has a placeholder {k:?} lacks");
                assert!(want.iter().all(|n| got.contains(n) || n.starts_with('s') && n.len() <= 2), "{code}: {v:?} lost a placeholder of {k:?}");
            }
        }
        set("es");
        assert_eq!(t("not a string of the app"), "not a string of the app");
        assert_eq!(tf("{n} of {m}", &[("n", &1), ("m", &"two")]), "1 of two");
        // engine text read back through the patterns
        assert_eq!(match_pattern("{name}: depth must be > 0", "Cut out: depth must be > 0"), Some(vec![("name", "Cut out")]));
        assert_eq!(match_pattern("{a} of {b}", "1 of 2 of 3"), Some(vec![("a", "1"), ("b", "2 of 3")]));
        assert_eq!(match_pattern("{name}: depth must be > 0", "Cut out: depth is fine"), None);
        assert_eq!(match_pattern("Total {n} min", "Total 4 min"), Some(vec![("n", "4")]));
        assert_eq!(match_pattern("{a}{b}", "xy"), None);
        let k = "\"{name}\" cuts the part free before {later} {s} cut: move it below {s2} in the list.";
        assert_eq!(
            match_pattern(k, "\"Cut out\" cuts the part free before Pocket 2, V-Carve 1 are cut: move it below them in the list."),
            Some(vec![("name", "Cut out"), ("later", "Pocket 2, V-Carve 1"), ("s", "are"), ("s2", "them")])
        );
        assert_eq!(fill("{a} and {b}", |n| if n == "a" { Some("{b}") } else { Some("x") }), "{b} and x", "a value is never filled in again");
        assert!(pieces("no {braces here} at all").is_none());
        assert_eq!(msg("not a string of the app"), "not a string of the app");
        set("en");
        assert_eq!(current(), "en");
        assert_eq!(msg("Cut out: depth must be > 0"), "Cut out: depth must be > 0");
    }
}
