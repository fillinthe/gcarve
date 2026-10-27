//! Small persisted key -> text store for the bit library, user posts and preferences: a file per key in the GCarve
//! config folder on the desktop (same folder as the account token in cloud.rs), browser localStorage on the web.
//! The folder / key prefix is the app's name (`set_app`, default "GCarve").

static APP: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Call once at start, before any load/save ("GSketch" → ~/Library/Application Support/GSketch, `gsketch.<key>`).
pub fn set_app(name: &'static str) {
    let _ = APP.set(name);
}
/// The app's name given to `set_app`
pub fn app() -> &'static str {
    APP.get().copied().unwrap_or(crate::i18n::t("GCarve"))
}

/// %APPDATA%\<App>, ~/Library/Application Support/<App>, or ~/.config/<app>
#[cfg(not(target_arch = "wasm32"))]
pub fn config_dir() -> Option<std::path::PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var("APPDATA").ok().map(|a| std::path::PathBuf::from(a).join(app()))
    } else if cfg!(target_os = "macos") {
        std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join("Library/Application Support").join(app()))
    } else {
        std::env::var("XDG_CONFIG_HOME").ok().map(std::path::PathBuf::from).or_else(|| std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".config"))).map(|c| c.join(app().to_lowercase()))
    }
}

/// `key` is a file name like "tools.json"; env `<APP>_<KEY>_FILE` (desktop) overrides the path, as the Qt app does for tools.
pub fn load(key: &str) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let p = path_for(key)?;
        std::fs::read_to_string(p).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()?.local_storage().ok()??.get_item(&format!("{}.{key}", app().to_lowercase())).ok()?
    }
}

static FROZEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Stop writing preferences (tests and smoke runs that press every button must not change this computer's settings).
pub fn freeze() {
    FROZEN.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn save(key: &str, text: &str) {
    if FROZEN.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(p) = path_for(key) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // write beside it, then swap in: a crash or power cut mid-write left a cut-short tools.json / recovery copy
        let tmp = p.with_extension("tmp-write");
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &p).is_err() {
            let _ = std::fs::remove_file(&tmp);
            let _ = std::fs::write(&p, text);
        }
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(&format!("{}.{key}", app().to_lowercase()), text);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn path_for(key: &str) -> Option<std::path::PathBuf> {
    let var = format!("{}_{}_FILE", app().to_uppercase(), key.split('.').next().unwrap_or(key).to_ascii_uppercase());
    if let Ok(p) = std::env::var(var) {
        return Some(p.into());
    }
    config_dir().map(|d| d.join(key))
}

pub fn remove(key: &str) {
    if FROZEN.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(p) = path_for(key) {
        let _ = std::fs::remove_file(p);
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.remove_item(&format!("{}.{key}", app().to_lowercase()));
    }
}
