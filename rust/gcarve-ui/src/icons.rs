//! The Qt app's monochrome icons (app/icons/*.svg, black strokes) embedded and tinted at draw time, and the
//! IconButton widget (app/IconButton.qml): idle = icon in its tint colour, active = lighter tile + accent bar
//! underneath, `primary` = the one filled blue button, optional label beside or below, tooltip. Each app hands its own
//! icon set (name, svg) to `install`.

use crate::theme;
use crate::widgets::TipExt;
use eframe::egui::{self, load::Bytes, Color32, ImageSource, Sense, Stroke, Vec2};
use std::collections::HashMap;
use std::sync::OnceLock;

static CACHE: OnceLock<HashMap<&'static str, ImageSource<'static>>> = OnceLock::new();

fn sources() -> &'static HashMap<&'static str, ImageSource<'static>> {
    CACHE.get_or_init(HashMap::new)
}

/// The icons are drawn in black; egui's tint multiplies, so they are re-coloured white once and tinted from there.
fn load(raw: &'static [(&'static str, &'static str)]) -> HashMap<&'static str, ImageSource<'static>> {
    raw.iter()
            .map(|(name, svg)| {
                let bytes: &'static [u8] = Box::leak(svg.replace("#000", "#fff").into_boxed_str().into_boxed_bytes());
                let uri: &'static str = Box::leak(format!("bytes://icon-{name}.svg").into_boxed_str());
                (*name, ImageSource::Bytes { uri: uri.into(), bytes: Bytes::Static(bytes) })
            })
        .collect()
}

/// `raw` = the app's icons (name, svg text, black strokes); the first call's set is kept
pub fn install(ctx: &egui::Context, raw: &'static [(&'static str, &'static str)]) {
    CACHE.get_or_init(|| load(raw));
    egui_extras::install_image_loaders(ctx);
}

/// Draw one icon tinted, centred in `rect` at `size` px
pub fn paint(ui: &egui::Ui, name: &str, rect: egui::Rect, size: f32, color: Color32) {
    if let Some(src) = sources().get(name) {
        let r = egui::Rect::from_center_size(rect.center().round(), Vec2::splat(size));
        egui::Image::new(src.clone()).tint(color).paint_at(ui, r);
    }
}

pub struct IconButton<'a> {
    icon: &'a str,
    text: &'a str,
    tip: &'a str,
    size: f32,
    padding: f32,
    font: f32,
    label_below: bool,
    primary: bool,
    active: bool,
    tint: Option<Color32>,
    accent: Color32,
    width: Option<f32>,
}

impl<'a> IconButton<'a> {
    pub fn new(icon: &'a str) -> Self {
        IconButton { icon, text: "", tip: "", size: 18.0, padding: 5.0, font: 12.0, label_below: false, primary: false, active: false, tint: None, accent: theme::ACCENT, width: None }
    }
    pub fn text(mut self, t: &'a str) -> Self {
        self.text = t;
        self
    }
    pub fn tip(mut self, t: &'a str) -> Self {
        self.tip = t;
        self
    }
    /// icon px (IconButton.qml `size`)
    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }
    pub fn padding(mut self, p: f32) -> Self {
        self.padding = p;
        self
    }
    pub fn font(mut self, f: f32) -> Self {
        self.font = f;
        self
    }
    /// label under the icon (rails, command grids): icon 24, font 11, padding 6
    pub fn label_below(mut self) -> Self {
        self.label_below = true;
        self.size = 24.0;
        self.font = 11.0;
        self.padding = 6.0;
        self
    }
    pub fn primary(mut self, on: bool) -> Self {
        self.primary = on;
        self
    }
    /// checked / highlighted
    pub fn active(mut self, on: bool) -> Self {
        self.active = on;
        self
    }
    /// idle icon colour (colour coding); also the default accent bar colour
    pub fn tint(mut self, c: Color32) -> Self {
        self.tint = Some(c);
        self.accent = c;
        self
    }
    pub fn accent(mut self, c: Color32) -> Self {
        self.accent = c;
        self
    }
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let enabled = ui.is_enabled();
        let text_col = if !enabled { theme::TEXT_DISABLED } else if self.primary { Color32::WHITE } else if self.active { theme::TEXT } else { theme::TEXT_DIM };
        let galley = (!self.text.is_empty()).then(|| ui.painter().layout_no_wrap(self.text.to_owned(), theme::font(self.font), text_col));
        let (tw, th) = galley.as_ref().map(|g| (g.size().x, g.size().y)).unwrap_or((0.0, 0.0));
        let pad = self.padding;
        let inner = if galley.is_none() {
            Vec2::splat(self.size)
        } else if self.label_below {
            Vec2::new(tw.max(self.size), self.size + 2.0 + th)
        } else {
            Vec2::new(self.size + 2.0 + tw, self.size.max(th))
        };
        let mut want = inner + Vec2::splat(pad * 2.0);
        if let Some(w) = self.width {
            want.x = w;
        }
        let (rect, resp) = ui.allocate_exact_size(want, Sense::click());
        let resp = if self.tip.is_empty() { resp } else { resp.tip(self.tip) };
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let hovered = resp.hovered() && enabled;
        let down = resp.is_pointer_button_down_on() && enabled;
        let idle = self.tint.unwrap_or(theme::TEXT);
        let icon_col = if !enabled { theme::TEXT_DISABLED } else if self.primary { Color32::WHITE } else if self.active { theme::lighter(idle, 1.1) } else if hovered { theme::lighter(idle, 1.12) } else { idle };
        let bg = if self.primary {
            if !enabled { theme::RAISED } else if down { theme::ACCENT_DOWN } else if hovered { theme::ACCENT_HOVER } else { theme::ACCENT }
        } else if !enabled {
            Color32::TRANSPARENT
        } else if self.active {
            if hovered { theme::lighter(theme::ACTIVE, 1.08) } else { theme::ACTIVE }
        } else if down {
            theme::RAISED_DOWN
        } else if hovered {
            theme::RAISED_HOVER
        } else {
            Color32::TRANSPARENT
        };
        let p = ui.painter();
        let border = if self.primary { Stroke::new(1.0, theme::ACCENT_HOVER) } else if self.active && enabled { Stroke::new(1.0, theme::BORDER_STRONG) } else { Stroke::NONE };
        p.rect(rect, 4.0, bg, border, egui::StrokeKind::Inside);
        if self.active && !self.primary && enabled {
            let w = (rect.width() - 14.0).max(10.0);
            p.rect_filled(egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.bottom() - 3.0), Vec2::new(w, 2.0)), 1.0, self.accent);
        }
        let content = egui::Rect::from_center_size(rect.center(), inner);
        match galley {
            None => paint(ui, self.icon, content, self.size, icon_col),
            Some(g) if self.label_below => {
                let ir = egui::Rect::from_min_size(egui::pos2(content.center().x - self.size / 2.0, content.top()), Vec2::splat(self.size));
                paint(ui, self.icon, ir, self.size, icon_col);
                ui.painter().galley(egui::pos2(content.center().x - tw / 2.0, content.bottom() - th), g, text_col);
            }
            Some(g) => {
                let ir = egui::Rect::from_min_size(egui::pos2(content.left(), content.center().y - self.size / 2.0), Vec2::splat(self.size));
                paint(ui, self.icon, ir, self.size, icon_col);
                ui.painter().galley(egui::pos2(content.left() + self.size + 2.0, content.center().y - th / 2.0), g, text_col);
            }
        }
        resp
    }
}
