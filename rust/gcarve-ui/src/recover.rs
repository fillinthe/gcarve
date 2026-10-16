//! Never lose work to a refresh or a crash. Web: `set_dirty` sets window.<app>Dirty (gcarveDirty, gsketchDirty) (index.html's beforeunload
//! prompt reads it). Both: a recovery copy of the project goes to browser storage / the config folder 3 s after the
//! last change, offered back at the next start; a normal quit (`App::on_exit`) or a save removes it.

use crate::store;

pub struct Recovery {
    pub name: String,
    pub time_ms: f64,
    pub json: String,
}

const KEY: &str = "recover";

pub fn set_dirty(dirty: bool) {
    #[cfg(target_arch = "wasm32")]
    if let Some(w) = web_sys::window() {
        let _ = js_sys::Reflect::set(&w, &format!("{}Dirty", store::app().to_lowercase()).into(), &dirty.into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = dirty;
}

pub fn save(name: &str, json: &str) {
    if cfg!(test) && std::env::var_os(format!("{}_RECOVER_FILE", store::app().to_uppercase())).is_none() {
        return; // tests never write into the real config folder
    }
    let time_ms = js_now();
    let v = serde_json::json!({"name": name, "time": time_ms, "json": json});
    store::save(KEY, &v.to_string());
}

pub fn load() -> Option<Recovery> {
    let v: serde_json::Value = serde_json::from_str(&store::load(KEY)?).ok()?;
    let json = v.get("json")?.as_str()?.to_string();
    if json.is_empty() {
        return None;
    }
    Some(Recovery { name: v.get("name").and_then(|n| n.as_str()).unwrap_or("Untitled").to_string(), time_ms: v.get("time").and_then(|t| t.as_f64()).unwrap_or(0.0), json })
}

pub fn clear() {
    store::remove(KEY);
}

fn js_now() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
    }
}

/// the time a recovery copy was written, in the local time zone
pub fn when(time_ms: f64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        if time_ms > 0.0 {
            return js_sys::Date::new(&time_ms.into()).to_locale_string("default", &eframe::wasm_bindgen::JsValue::UNDEFINED).into();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(t) = chrono::DateTime::from_timestamp_millis(time_ms as i64) {
        return t.with_timezone(&chrono::Local).format("%b %-d, %-I:%M %p").to_string();
    }
    String::new()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #[test]
    fn a_recovery_copy_comes_back_until_it_is_cleared() {
        let f = std::env::temp_dir().join(format!("gcarve-recover-{}.json", std::process::id()));
        std::env::set_var("GCARVE_RECOVER_FILE", &f);
        super::save("Door sign", "{\"x\":1}");
        let r = super::load().expect("copy");
        assert_eq!((r.name.as_str(), r.json.as_str()), ("Door sign", "{\"x\":1}"));
        assert!(!super::when(r.time_ms).is_empty());
        super::clear();
        assert!(super::load().is_none());
    }
}
