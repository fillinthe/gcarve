//! UI pieces shared by GCarve and GSketch (../../gsketch): theme, widgets, fonts, tinted icons, the prefs store, the
//! SpaceMouse reader, recovery copies, the dev screenshot and browser downloads. App-specific things (icon set, store name) are handed in at start.

pub mod account;
pub mod cloud;
pub mod color;
pub mod fonts;
pub mod glyphs;
pub mod guide;
pub mod help;
pub mod i18n;
pub mod icons;
pub mod nav;
pub mod palette;
pub mod recover;
pub mod shot;
pub mod spacemouse;
pub mod store;
pub mod strokes;
pub mod theme;
pub mod menubar;
pub mod widgets;
#[cfg(target_arch = "wasm32")]
pub mod webfile;
#[cfg(target_arch = "wasm32")]
pub mod webserial;
