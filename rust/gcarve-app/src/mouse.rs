//! The mouse setup for the drawing and the 3D view (`gcarve_ui::nav::MouseMap`): one per device, kept in the store as
//! "mouse". Read every frame by both views (`get`, a Copy out of a cell); changed by Preferences › Mouse and the 3D
//! view's Mouse menu (`set`). Older installs had only a style and a reverse-zoom switch: those become the first map.

use gcarve_ui::nav::{MouseMap, Nav};
use std::cell::Cell;

const KEY: &str = "mouse";

thread_local! {
    static MAP: Cell<Option<MouseMap>> = const { Cell::new(None) };
    static WANT_PREFS: Cell<bool> = const { Cell::new(false) };
}

pub fn get() -> MouseMap {
    MAP.with(|m| {
        m.get().unwrap_or_else(|| {
            let shot = std::env::var("GCARVE_SHOT").is_ok(); // screenshots: the stock setup
            let map = crate::store::load(KEY).filter(|_| !shot).and_then(|s| MouseMap::decode(&s)).unwrap_or_else(|| {
                let nav = crate::store::load("view3d-nav").filter(|_| !shot).and_then(|n| Nav::from_key(&n)).unwrap_or(Nav::Fusion);
                MouseMap { wheel_reverse: !shot && crate::store::load("view3d-zoom-reverse").as_deref() == Some("1"), ..MouseMap::from_nav(nav) }
            });
            m.set(Some(map));
            map
        })
    })
}

pub fn set(map: MouseMap) {
    if get() != map {
        MAP.with(|m| m.set(Some(map)));
        crate::store::save(KEY, &map.encode());
    }
}

/// the 3D view's "Fine-tune…" item: the app opens Preferences on the next frame
pub fn ask_prefs() {
    WANT_PREFS.with(|w| w.set(true));
}
pub fn take_prefs_request() -> bool {
    WANT_PREFS.with(|w| w.replace(false))
}
