//! GCarve's dark look: the tokens of the Qt app's style/Theme.qml, applied to egui's visuals, plus the few
//! controls egui cannot be styled into (Qt-style checkbox, switch, tab button, unit field).

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, Visuals};

pub const fn c(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

// Photoshop-style neutral greys: panels a step lighter than the window, separators darker than both,
// one blue for the primary action; colour only where it carries meaning (modes, toolpath kinds, groups)
pub const BG: Color32 = c(0x2b2b2b); // window / bars
pub const PANEL: Color32 = c(0x323232); // panels, dialogs, popups
pub const FIELD: Color32 = c(0x1f1f1f); // inputs
pub const RAISED: Color32 = c(0x3d3d3d); // buttons
pub const RAISED_HOVER: Color32 = c(0x484848);
pub const RAISED_DOWN: Color32 = c(0x555555);
pub const ACTIVE: Color32 = c(0x4a4a4a); // selected tool / toggle tile
pub const BORDER: Color32 = c(0x1c1c1c);
pub const BORDER_STRONG: Color32 = c(0x4d4d4d);
pub const TEXT: Color32 = c(0xd9d9d9);
pub const TEXT_DIM: Color32 = c(0x9a9a9a);
pub const TEXT_DISABLED: Color32 = c(0x616161);
pub const ACCENT: Color32 = c(0x1473e6);
pub const ACCENT_HOVER: Color32 = c(0x2680eb);
pub const ACCENT_DOWN: Color32 = c(0x0d66d0);
pub const DANGER: Color32 = c(0xe0603c);
pub const WARNING: Color32 = c(0xf0a040);
pub const ORANGE: Color32 = c(0xff9a2e);
pub const CANVAS_BG: Color32 = c(0x262626);
pub const SEPARATOR: Color32 = c(0x1c1c1c);
pub const ROW_HOVER: Color32 = c(0x3e3e3e);
pub const MODE_DESIGN: Color32 = c(0x3d8ee6);
pub const MODE_TOOLPATHS: Color32 = c(0xff9a2e);
pub const MODE_SIMULATE: Color32 = c(0x3fbf7f);
// command families (Theme.groupColor)
pub const GROUP_ARRANGE: Color32 = c(0x6cb6ff);
pub const GROUP_BOOLEAN: Color32 = c(0x38c7d0);
pub const GROUP_VIEW: Color32 = c(0x7bd88f);
pub const GROUP_SETUP: Color32 = c(0xc9a9ff);
// rail tool families (Theme.toolColor)
pub const TOOL_SELECT: Color32 = c(0xe8eaee);
pub const TOOL_DRAW: Color32 = c(0x6cb6ff);
pub const TOOL_MEASURE: Color32 = c(0xffd34d);
pub const TOOL_TEXT: Color32 = c(0xff9f1a);
pub const TOOL_FILE: Color32 = c(0xb8bec8);

/// What a tool does, for its colour: every app's tool rail tints by this, so a draw tool looks the same in all of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolKind {
    Select,
    Draw,
    /// changes what is there (move, push/pull, booleans)
    Modify,
    Measure,
    Text,
    /// paint, materials, set-up
    Material,
    View,
}

/// (icon tint, active accent bar); Select keeps the blue accent bar
pub fn tool_color(k: ToolKind) -> (Color32, Color32) {
    let tint = match k {
        ToolKind::Select => return (TOOL_SELECT, ACCENT),
        ToolKind::Draw => TOOL_DRAW,
        ToolKind::Modify => GROUP_BOOLEAN,
        ToolKind::Measure => TOOL_MEASURE,
        ToolKind::Text => TOOL_TEXT,
        ToolKind::Material => GROUP_SETUP,
        ToolKind::View => GROUP_VIEW,
    };
    (tint, tint)
}

pub const RADIUS: u8 = 6;
pub const CONTROL_HEIGHT: f32 = 30.0;
pub const FONT_SIZE: f32 = 13.0;

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Qt.lighter(c, f): scale the colour's brightness
pub fn lighter(col: Color32, f: f32) -> Color32 {
    let [r, g, b, a] = col.to_array();
    let s = |v: u8| ((v as f32) * f).min(255.0) as u8;
    Color32::from_rgba_unmultiplied(s(r), s(g), s(b), a)
}

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(RADIUS);
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = BG;
    v.code_bg_color = FIELD;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = c(0x6cb6ff);
    v.warn_fg_color = WARNING;
    v.error_fg_color = DANGER;
    v.override_text_color = None;
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.weak_bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.weak_bg_fill = RAISED;
    v.widgets.inactive.bg_fill = FIELD;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.weak_bg_fill = RAISED_HOVER;
    v.widgets.hovered.bg_fill = FIELD;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.weak_bg_fill = RAISED_DOWN;
    v.widgets.active.bg_fill = FIELD;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.expansion = 0.0;
    v.widgets.open.weak_bg_fill = RAISED_DOWN;
    v.widgets.open.bg_fill = FIELD;
    v.widgets.open.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.open.fg_stroke = Stroke::new(1.0, TEXT);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(RADIUS);
    }
    v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(90) };
    v.window_shadow = egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(120) };
    v.text_cursor.stroke.color = TEXT;
    // the apps are dark by design (panels, canvas and text colours are fixed): a computer or browser in light mode
    // must not swap in egui's light style, which put light-grey text on light menus (reported 2026-10-15)
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.all_styles_mut(|s| {
        s.spacing.button_padding = egui::vec2(14.0, 7.0); // 30 px buttons / combos (Theme.controlHeight)
        s.spacing.item_spacing = egui::vec2(6.0, 6.0);
        s.spacing.interact_size = egui::vec2(40.0, 26.0);
        s.spacing.menu_margin = egui::Margin::same(6); // style/Menu.qml padding 6
        s.spacing.window_margin = egui::Margin::same(12);
        s.spacing.text_edit_width = 100.0;
        s.spacing.combo_width = 100.0;
        s.spacing.icon_width = 16.0; // 30 px combos (text 16 + 2 × 7)
        s.spacing.icon_width_inner = 10.0;
        s.visuals.menu_corner_radius = CornerRadius::same(RADIUS + 2);
        s.interaction.tooltip_delay = 0.5;
        use egui::TextStyle::*;
        s.text_styles.insert(Body, font(FONT_SIZE));
        s.text_styles.insert(Button, font(FONT_SIZE));
        s.text_styles.insert(Small, font(11.0));
        s.text_styles.insert(Heading, font(14.0));
        s.text_styles.insert(Monospace, FontId::monospace(12.0));
        s.text_styles.insert(Name("Tip".into()), font(12.0));
        s.text_styles.insert(Name("Caption".into()), font(10.0));
    });
}

/// Qt-style separator line (Theme.border) across the current width
pub fn hline(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, BORDER);
}

/// 1 px vertical separator (Theme.separator) of the given height, inside a horizontal row
pub fn vsep(ui: &mut egui::Ui, h: f32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(1.0, h), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, SEPARATOR);
}

// the Qt controls live in widgets.rs (one implementation each)
pub use crate::widgets::{checkbox, switch, tab, unit_field};

// ---- shortcut labels (Qt Theme.keys) -------------------------------------------------------------------------------

/// Apple users see ⌘ ⇧ ⌥: macOS, and in the web build Mac / iPhone / iPad browsers (navigator.userAgent + platform).
pub fn apple() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        static APPLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *APPLE.get_or_init(|| {
            let Some(n) = web_sys::window().map(|w| w.navigator()) else { return false };
            let ua = format!("{} {}", n.user_agent().unwrap_or_default(), n.platform().unwrap_or_default());
            ["Mac", "iPhone", "iPad"].iter().any(|k| ua.contains(k))
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        cfg!(any(target_os = "macos", target_os = "ios"))
    }
}

/// Shortcut text written Mac-style ("⇧⌘Z", "Undo (⌘Z)") in the user's style: unchanged on Apple devices,
/// "Ctrl+Shift+Z" for everyone else. Menus, tooltips, the palette and hints all go through it.
pub fn keys(s: &str) -> std::borrow::Cow<'_, str> {
    if apple() { s.into() } else { keys_pc(s) }
}

/// Qt Theme.keys for non-Apple users: each run of modifier glyphs plus the key after it becomes "Ctrl+Shift+K".
pub fn keys_pc(s: &str) -> std::borrow::Cow<'_, str> {
    const MODS: [char; 4] = ['⌃', '⌥', '⇧', '⌘'];
    if !s.contains(MODS) {
        return s.into();
    }
    let mut out = String::with_capacity(s.len() + 8);
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if !MODS.contains(&ch) {
            out.push(ch);
            continue;
        }
        let mut mods = String::from(ch);
        while let Some(m) = chars.next_if(|m| MODS.contains(m)) {
            mods.push(m);
        }
        let mut key = String::new();
        while let Some(k) = chars.next_if(|k| !k.is_whitespace() && *k != ')' && !MODS.contains(k)) {
            key.push(k);
        }
        if key.is_empty() {
            out.push_str(&mods); // a lone glyph stays
            continue;
        }
        for (g, name) in [('⌘', "Ctrl+"), ('⌃', "Meta+"), ('⌥', "Alt+"), ('⇧', "Shift+")] {
            if mods.contains(g) {
                out.push_str(name);
            }
        }
        out.push_str(&key);
    }
    out.into()
}

#[cfg(test)]
mod key_tests {
    use super::keys_pc;

    #[test]
    fn mac_glyphs_become_ctrl_shift() {
        assert_eq!(keys_pc("⇧⌘Z"), "Ctrl+Shift+Z");
        assert_eq!(keys_pc("Redo (⇧⌘Z)"), "Redo (Ctrl+Shift+Z)");
        assert_eq!(keys_pc("Group (⌘G); Ungroup (⇧⌘G)"), "Group (Ctrl+G); Ungroup (Ctrl+Shift+G)");
        assert_eq!(keys_pc("Tip: press ⌘K to find"), "Tip: press Ctrl+K to find");
        assert_eq!(keys_pc("⌥⌘,"), "Ctrl+Alt+,");
        assert_eq!(keys_pc("⌘−"), "Ctrl+−");
        assert_eq!(keys_pc("T"), "T");
        assert_eq!(keys_pc("⌫"), "⌫");
    }
}
