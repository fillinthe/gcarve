//! The Qt app's controls (app/style/*.qml) as egui widgets: one implementation each, used by every screen.
//! Most of the look comes from theme::apply (egui visuals); these cover what egui can't be styled into:
//! Dialog (style/Dialog.qml + DialogButtonBox), Button (primary / flat), TextField sizing, ComboBox chevron,
//! CheckBox, Switch, RadioButton, SpinBox, TabButton, NumField badge, Menu items, ToolTip, Popup frame.

use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Color32, FontFamily, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, Widget, WidgetText};

/// The bold family: `RichText::family(bold_family())` keeps the size (egui's `strong()` only brightens)
pub fn bold_family() -> FontFamily {
    FontFamily::Name(crate::fonts::BOLD.into())
}

/// Bold UI font (fonts.rs installs the family)
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(crate::fonts::BOLD.into()))
}

fn galley(ui: &Ui, text: &str, f: FontId) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), f, TEXT)
}

// ---- Dialog -------------------------------------------------------------------------------------------------------

/// style/Dialog.qml: centred panel (radius 10, strong border), bold title left at 20 px over a separator, content with
/// 20 px sides, buttons bottom right (`buttons` / `ok_cancel`). Modal by default (dark backdrop, Esc closes when
/// `open` is given); `modeless` = a movable window without backdrop (Variables, Align).
pub struct Dialog<'o> {
    title: String,
    id: egui::Id,
    open: Option<&'o mut bool>,
    modeless: Option<(egui::Pos2, egui::Align2)>,
    width: Option<f32>,
    close: bool,
    flush: bool,
}

impl<'o> Dialog<'o> {
    pub fn new(title: impl Into<String>) -> Self {
        let title = title.into();
        Dialog { id: egui::Id::new(("dialog", title.clone())), title, open: None, modeless: None, width: None, close: false, flush: false }
    }
    /// stable id when the title changes (e.g. "New …" / "Edit …")
    pub fn id(mut self, id: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id = egui::Id::new(("dialog", id));
        self
    }
    /// Esc sets it to false
    pub fn open(mut self, open: &'o mut bool) -> Self {
        self.open = Some(open);
        self
    }
    /// `pivot`: which corner of the dialog sits at `pos` (Qt x/y bindings, e.g. right-anchored)
    pub fn modeless(mut self, pos: egui::Pos2, pivot: egui::Align2) -> Self {
        self.modeless = Some((pos, pivot));
        self
    }
    /// standardButtons: Dialog.Close (clears `open`)
    pub fn close_footer(mut self) -> Self {
        self.close = true;
        self
    }
    /// `padding: 0` (JobsDialog.qml): content edge to edge under the title; the footer keeps its margins
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the Jobs dialog is desktop-only
    pub fn flush(mut self) -> Self {
        self.flush = true;
        self
    }
    /// content width (without the 20 px sides)
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }

    pub fn show<R>(self, ctx: &egui::Context, add: impl FnOnce(&mut Ui) -> R) -> R {
        let Dialog { title, id, open, modeless, width, close, flush } = self;
        let closed = std::cell::Cell::new(false);
        // a dialog that can be closed always shows a × in its title bar (a modeless one has no Esc and maybe no footer)
        let closable = open.is_some();
        let frame = egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER_STRONG)).shadow(ctx.global_style().visuals.window_shadow);
        let title2 = title.clone();
        let body = |ui: &mut Ui| {
            frame.show(ui, |ui| {
                let top = ui.cursor().min;
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.allocate_exact_size(vec2(0.0, 46.0), Sense::hover());
                let m = if flush { egui::Margin { left: 1, right: 1, top: 14, bottom: 0 } } else { egui::Margin { left: 20, right: 20, top: 14, bottom: 16 } };
                let r = egui::Frame::new().inner_margin(m).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    if let Some(w) = width {
                        ui.set_width(w);
                    }
                    // never taller than the window: the content scrolls, the Close footer stays
                    // (a modeless dialog starts below the top of the window: that much less room)
                    let down = modeless.map_or(0.0, |(pos, _)| (pos.y - ui.ctx().content_rect().top()).max(0.0));
                    let max_h = ui.ctx().content_rect().height() - down - 46.0 - 30.0 - if close { 62.0 } else { 0.0 } - 24.0;
                    let r = egui::ScrollArea::vertical().max_height(max_h.max(120.0)).min_scrolled_height(max_h.max(120.0)).show(ui, add).inner;
                    if close {
                        let hit = if flush {
                            egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 19, top: 2, bottom: 11 }).show(ui, |ui| {
                                ui.set_width(ui.available_width()); // button_row aligns to the right edge of the content
                                button_row(ui, &[("Close", false)], 84.0).is_some()
                            }).inner
                        } else {
                            close_button(ui)
                        };
                        if hit {
                            closed.set(true);
                        }
                    }
                    r
                });
                let w = ui.min_rect().width();
                let g = ui.painter().layout_no_wrap(title2, bold(FONT_SIZE + 3.0), TEXT);
                ui.painter().galley(pos2(top.x + 20.0, top.y + 23.0 - g.size().y / 2.0), g, TEXT);
                if closable {
                    let x = Rect::from_center_size(pos2(top.x + w - 25.0, top.y + 23.0), vec2(28.0, 28.0));
                    let resp = ui.interact(x, id.with("close×"), Sense::click()).on_hover_text("Close");
                    if resp.hovered() {
                        ui.painter().rect_filled(x, 6.0, RAISED);
                    }
                    let (c, r) = (x.center(), 5.0);
                    let col = if resp.hovered() { TEXT } else { TEXT_DIM };
                    for d in [vec2(r, r), vec2(r, -r)] {
                        ui.painter().line_segment([c - d, c + d], Stroke::new(1.6, col));
                    }
                    if resp.clicked() {
                        closed.set(true);
                    }
                }
                ui.painter().rect_filled(Rect::from_min_size(pos2(top.x + 1.0, top.y + 45.0), vec2(w - 2.0, 1.0)), 0.0, BORDER);
                r.inner
            })
            .inner
        };
        match modeless {
            None => {
                let r = egui::Modal::new(id).backdrop_color(Color32::from_black_alpha(0x66)).frame(egui::Frame::NONE).show(ctx, body);
                if let Some(o) = open {
                    if closed.get() || (r.is_top_modal && !r.any_popup_open && ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
                        *o = false;
                    }
                }
                r.inner
            }
            Some((pos, pivot)) => {
                let mut out = None;
                egui::Window::new(&title).id(id).title_bar(false).resizable(false).frame(egui::Frame::NONE).pivot(pivot).fixed_pos(pos).show(ctx, |ui| out = Some(body(ui)));
                if let (Some(o), true) = (open, closed.get()) {
                    *o = false;
                }
                // a window that was not shown this frame (never happens with title_bar(false)) has no result
                out.unwrap_or_else(|| panic!("dialog {title} not shown"))
            }
        }
    }
}

/// DialogButtonBox: right-aligned buttons in the given order (min 84 px, `true` = the blue primary one), 26 px above.
/// Returns the index clicked.
pub fn buttons(ui: &mut Ui, list: &[(&str, bool)]) -> Option<usize> {
    ui.add_space(26.0 - ui.spacing().item_spacing.y);
    // In a dialog taller than the window the content scrolls: when this row would be below the visible part it is
    // drawn stuck to the bottom edge instead (its place in the content is kept), so OK / Cancel are always in reach.
    let (clip, at, right) = (ui.clip_rect(), ui.cursor().min, ui.min_rect().right());
    if at.y + CONTROL_HEIGHT > clip.bottom() + 0.5 && clip.height() > 3.0 * CONTROL_HEIGHT {
        ui.allocate_exact_size(vec2(1.0, CONTROL_HEIGHT), Sense::hover());
        let strip = Rect::from_min_max(pos2(clip.left(), clip.bottom() - CONTROL_HEIGHT - 12.0), clip.right_bottom());
        ui.painter().rect_filled(strip, 0.0, PANEL);
        ui.painter().hline(strip.x_range(), strip.top(), Stroke::new(1.0, BORDER));
        let mut pinned = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(at.x, strip.top() + 8.0), pos2(right, clip.bottom()))));
        return button_row_to(&mut pinned, list, 84.0, right);
    }
    button_row(ui, list, 84.0)
}

/// Right-aligned row of buttons in the given order (`true` = primary), each at least `min_w` wide
pub fn button_row(ui: &mut Ui, list: &[(&str, bool)], min_w: f32) -> Option<usize> {
    // right edge = the content above (auto-sized dialogs and popups have no fixed width)
    let right = ui.min_rect().right();
    button_row_to(ui, list, min_w, right)
}

fn button_row_to(ui: &mut Ui, list: &[(&str, bool)], min_w: f32, right: f32) -> Option<usize> {
    let ws: Vec<f32> = list.iter().map(|(t, _)| (galley(ui, t, font(FONT_SIZE)).size().x + 28.0).max(min_w)).collect();
    let total = ws.iter().sum::<f32>() + 8.0 * (list.len().saturating_sub(1)) as f32;
    let mut hit = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_space((right - ui.cursor().left() - total).max(0.0));
        for (i, (t, primary)) in list.iter().enumerate() {
            if Button::new(*t).primary(*primary).min_width(ws[i]).ui(ui).clicked() {
                hit = Some(i);
            }
        }
    });
    hit
}

/// A `w` px wide item flush with the right edge of the content above (right-to-left layouts would grow an
/// auto-sized dialog to the whole screen)
pub fn right<R>(ui: &mut Ui, w: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let right = ui.min_rect().right();
    ui.horizontal(|ui| {
        ui.add_space((right - ui.cursor().left() - w).max(0.0));
        add(ui)
    })
    .inner
}

/// OK (primary, Enter) / Cancel (Esc) footer: (ok, cancel)
pub fn ok_cancel(ui: &mut Ui) -> (bool, bool) {
    ok_cancel_text(ui, "OK")
}
pub fn ok_cancel_text(ui: &mut Ui, ok_text: &str) -> (bool, bool) {
    let hit = buttons(ui, &[(ok_text, true), ("Cancel", false)]);
    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter) && !i.modifiers.shift, i.key_pressed(egui::Key::Escape)));
    let multiline = ui.ctx().memory(|m| m.focused()).and_then(|id| ui.ctx().read_response(id)).is_some_and(|r| r.rect.height() > 40.0);
    let popup = egui::Popup::is_any_open(ui.ctx()) || multiline;
    (hit == Some(0) || (enter && !popup), hit == Some(1) || (esc && !popup))
}
/// single Close button (Esc)
pub fn close_button(ui: &mut Ui) -> bool {
    buttons(ui, &[("Close", false)]).is_some()
}

/// style/Popup.qml frame: panel, radius 8, strong border, 12 px padding
pub fn popup_frame() -> egui::Frame {
    egui::Frame::new().fill(PANEL).corner_radius(RADIUS + 2).stroke(Stroke::new(1.0, BORDER_STRONG)).inner_margin(12).shadow(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(90) })
}

/// style/ItemDelegate.qml row: full-width, radius 5, accent when selected (white text), raised on hover; returns the
/// row's click response (widgets inside keep their own clicks)
pub fn list_row(ui: &mut Ui, selected: bool, add: impl FnOnce(&mut Ui)) -> Response {
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = ui.scope(|ui| {
        if selected {
            ui.visuals_mut().override_text_color = Some(Color32::WHITE);
        }
        egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 6 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
    });
    let rect = inner.inner.response.rect;
    let r = ui.interact(rect, inner.response.id.with("row"), Sense::click());
    let fill = if selected { ACCENT } else if r.hovered() { RAISED_HOVER } else { Color32::TRANSPARENT };
    ui.painter().set(bg, egui::epaint::RectShape::filled(rect, 5.0, fill));
    r
}

// ---- Button -------------------------------------------------------------------------------------------------------

/// style/Button.qml: 30 px, raised grey with a dark border; `primary` = blue; `flat` = no fill until hovered
pub struct Button {
    text: String,
    primary: bool,
    flat: bool,
    min_width: f32,
    icon: Option<(&'static str, bool)>,
    /// a colour of its own: (colour, solid). Outlined = a tint and a border in it; solid = filled with it
    tone: Option<(Color32, bool)>,
}

impl Button {
    pub fn new(text: impl Into<String>) -> Self {
        Button { text: text.into(), primary: false, flat: false, min_width: 0.0, icon: None, tone: None }
    }
    /// app/icons glyph before the text (`after`: behind it), in the text colour: arrows the web fonts lack (⇤ ⤒ ⇄)
    pub fn icon(mut self, name: &'static str, after: bool) -> Self {
        self.icon = Some((name, after));
        self
    }
    pub fn primary(mut self, on: bool) -> Self {
        self.primary = on;
        self
    }
    pub fn flat(mut self) -> Self {
        self.flat = true;
        self
    }
    /// a colour of its own (GBurn: amber outline = moves the machine, solid = fires the laser or stops it)
    pub fn tone(mut self, colour: Color32, solid: bool) -> Self {
        self.tone = Some((colour, solid));
        self
    }
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }
}

impl Widget for Button {
    fn ui(self, ui: &mut Ui) -> Response {
        let g = galley(ui, &self.text, font(FONT_SIZE));
        // an icon alone: just the icon with narrow sides (fits a 36 px cell), no gap for text
        let iw = if self.icon.is_some() { 14.0 + if self.text.is_empty() { 0.0 } else { 5.0 } } else { 0.0 };
        let pad = if self.text.is_empty() && self.icon.is_some() { 14.0 } else { 28.0 };
        let (rect, resp) = ui.allocate_exact_size(vec2((g.size().x + iw + pad).max(self.min_width), CONTROL_HEIGHT), Sense::click());
        let on = ui.is_enabled();
        let (hov, down) = (resp.hovered() && on, resp.is_pointer_button_down_on() && on);
        let fill = if !on {
            RAISED
        } else if self.primary {
            if down { ACCENT_DOWN } else if hov { ACCENT_HOVER } else { ACCENT }
        } else if down {
            RAISED_DOWN
        } else if hov {
            RAISED_HOVER
        } else if self.flat {
            Color32::TRANSPARENT
        } else {
            RAISED
        };
        let mut border = if self.flat && !hov { Stroke::NONE } else { Stroke::new(1.0, if resp.has_focus() { ACCENT } else if self.primary { ACCENT_HOVER } else { BORDER }) };
        let mut fill = fill;
        let mut col = if !on { TEXT_DISABLED } else if self.primary { Color32::WHITE } else { TEXT };
        if let (Some((c, solid)), true) = (self.tone, on) {
            let k = if down { 0.75 } else if hov { 0.9 } else { 1.0 };
            if solid {
                fill = Color32::from_rgb((c.r() as f32 * k) as u8, (c.g() as f32 * k) as u8, (c.b() as f32 * k) as u8);
                border = Stroke::new(1.0, fill);
                col = if c.r() as u32 + c.g() as u32 + c.b() as u32 > 480 { Color32::from_gray(24) } else { Color32::WHITE };
            } else {
                fill = c.gamma_multiply(if down { 0.38 } else if hov { 0.28 } else { 0.16 });
                border = Stroke::new(1.0, c.gamma_multiply(0.85));
                col = c;
            }
        }
        let p = ui.painter();
        p.rect(rect, RADIUS, fill, border, StrokeKind::Inside);
        let left = rect.center().x - (g.size().x + iw) / 2.0;
        let text_x = match self.icon {
            Some((name, after)) => {
                let ix = if after { left + g.size().x + 5.0 } else { left };
                crate::icons::paint(ui, name, Rect::from_min_size(pos2(ix, rect.center().y - 7.0), vec2(14.0, 14.0)), 14.0, col);
                if after { left } else { left + iw }
            }
            None => left,
        };
        p.galley(pos2(text_x, rect.center().y - g.size().y / 2.0), g, col);
        resp
    }
}

// ---- TextField / ComboBox ------------------------------------------------------------------------------------------

/// style/TextField.qml metrics: 28 px tall, 9 px sides (colours come from the egui visuals)
pub fn field<'t>(text: &'t mut dyn egui::TextBuffer) -> egui::TextEdit<'t> {
    egui::TextEdit::singleline(text).margin(egui::Margin { left: 9, right: 9, top: 5, bottom: 5 }).min_size(vec2(0.0, CONTROL_HEIGHT - 2.0)).font(font(FONT_SIZE))
}

/// Height for the result list of a browser dialog (bit catalog, machines): everything the window leaves after the
/// dialog's title, margins and Close row (about 170 px) and `above` px of filters over the list. The dialog then
/// fills the window instead of leaving it half empty.
pub fn list_height(ui: &Ui, above: f32) -> f32 {
    (ui.ctx().content_rect().height() - 170.0 - above).max(220.0)
}

/// A dialog's search box takes the keyboard as soon as the dialog shows (nothing else focused yet), so typing
/// filters straight away.
pub fn focus_first(ui: &Ui, r: &egui::Response) {
    if ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
}

/// Focused text fields get the accent border (Qt activeFocus); egui draws focus with the white selection stroke,
/// which also colours selected text, so the ring is painted over once per frame here.
pub fn focus_ring(ctx: &egui::Context) {
    let Some(id) = ctx.memory(|m| m.focused()) else { return };
    if let Some(r) = ctx.read_response(id) {
        // text fields sense click + drag (buttons only click); frameless edits are shorter than a field
        if r.sense.senses_drag() && r.rect.height() >= CONTROL_HEIGHT - 4.0 {
            ctx.layer_painter(r.layer_id).with_clip_rect(r.interact_rect.expand(1.0)).rect_stroke(r.rect, RADIUS - 1, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
        }
    }
}

/// style/ComboBox.qml: 30 px, 10 px text inset, chevron at the right. Wraps egui's ComboBox so its full 30 px
/// height counts in grids and rows (egui registers only the text height).
pub struct Combo {
    inner: egui::ComboBox,
    width: f32,
}

pub fn combo_box(id: impl std::hash::Hash + std::fmt::Debug) -> Combo {
    // a long choice (another language) is cut short in the closed box instead of widening it
    Combo { inner: egui::ComboBox::from_id_salt(id).height(320.0).truncate().icon(|ui, rect, _v, _open| chevron(ui.painter(), rect, if ui.is_enabled() { TEXT_DIM } else { TEXT_DISABLED })), width: 100.0 }
}

impl Combo {
    pub fn selected_text(mut self, t: impl Into<WidgetText>) -> Self {
        self.inner = self.inner.selected_text(t);
        self
    }
    pub fn width(mut self, w: f32) -> Self {
        self.inner = self.inner.width(w);
        self.width = w;
        self
    }
    pub fn show_ui<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<Option<R>> {
        let layout = if ui.layout().prefer_right_to_left() { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) };
        ui.allocate_ui_with_layout(vec2(self.width, CONTROL_HEIGHT), layout, |ui| {
            ui.spacing_mut().button_padding = vec2(10.0, 7.0);
            ui.spacing_mut().interact_size.y = CONTROL_HEIGHT; // egui centres an interact-height row inside: keep it flush
            self.inner.show_ui(ui, add)
        })
        .inner
    }
}

/// 10×6 down chevron centred in `rect`
pub fn chevron(p: &egui::Painter, rect: Rect, col: Color32) {
    let c = rect.center();
    p.add(egui::Shape::line(vec![c + vec2(-4.2, -2.2), c + vec2(0.0, 2.0), c + vec2(4.2, -2.2)], Stroke::new(1.6, col)));
}

// ---- CheckBox / Switch / RadioButton -------------------------------------------------------------------------------

/// style/CheckBox.qml: 18 px rounded box, blue when ticked with a white tick
pub struct CheckBox<'a> {
    on: &'a mut bool,
    text: WidgetText,
}

impl<'a> CheckBox<'a> {
    pub fn new(on: &'a mut bool, text: impl Into<WidgetText>) -> Self {
        CheckBox { on, text: text.into() }
    }
}

impl Widget for CheckBox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let enabled = ui.is_enabled();
        let g = self.text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::FontSelection::FontId(font(FONT_SIZE)));
        let tw = if g.size().x > 0.0 { 8.0 + g.size().x + 4.0 } else { 0.0 };
        let (rect, mut resp) = ui.allocate_exact_size(vec2(4.0 + 18.0 + tw, 30.0), Sense::click());
        if resp.clicked() {
            *self.on = !*self.on;
            resp.mark_changed();
        }
        let on = *self.on;
        let p = ui.painter();
        let bx = Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 9.0), vec2(18.0, 18.0));
        let down = resp.is_pointer_button_down_on();
        let fill = if !enabled { BG } else if on { if down { ACCENT_DOWN } else { ACCENT } } else if down { RAISED_DOWN } else { FIELD };
        let border = if !enabled { BORDER } else if on { ACCENT_HOVER } else if resp.hovered() { ACCENT } else { BORDER_STRONG };
        p.rect(bx, 4.0, fill, Stroke::new(1.0, border), StrokeKind::Inside);
        if on {
            let o = bx.left_top();
            p.add(egui::Shape::line(vec![o + vec2(4.0, 9.5), o + vec2(7.5, 13.0), o + vec2(14.0, 5.5)], Stroke::new(2.0, if enabled { Color32::WHITE } else { TEXT_DISABLED })));
        }
        let col = if enabled { TEXT } else { TEXT_DISABLED };
        p.galley(pos2(bx.right() + 8.0, rect.center().y - g.size().y / 2.0), g, col);
        resp
    }
}

pub fn checkbox(ui: &mut Ui, on: &mut bool, label: impl Into<WidgetText>) -> Response {
    ui.add(CheckBox::new(on, label))
}

/// style/Switch.qml: 36×20 pill with a 16 px knob, label after it
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    // a long label (another language, a narrow panel) wraps under itself instead of widening the row
    let room = (ui.available_width() - 52.0).max(60.0);
    let g = ui.painter().layout(label.to_owned(), font(FONT_SIZE), TEXT, room);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(36.0 + 8.0 + g.size().x + 8.0, (g.size().y + 4.0).max(22.0)), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let p = ui.painter();
    let track = Rect::from_min_size(pos2(rect.left() + 4.0, rect.center().y - 10.0), vec2(36.0, 20.0));
    p.rect(track, 10.0, if *on { ACCENT } else { RAISED }, Stroke::new(1.0, if *on { ACCENT_HOVER } else { BORDER_STRONG }), StrokeKind::Inside);
    let kx = if *on { track.right() - 10.0 } else { track.left() + 10.0 };
    p.circle_filled(pos2(kx, track.center().y), 8.0, if *on { Color32::WHITE } else { TEXT_DIM });
    p.galley(pos2(track.right() + 8.0, rect.center().y - g.size().y / 2.0), g, TEXT);
    resp
}

/// style/RadioButton.qml: 18 px ring, 8 px accent dot when checked
pub fn radio(ui: &mut Ui, checked: bool, label: &str) -> Response {
    let enabled = ui.is_enabled();
    let g = galley(ui, label, font(FONT_SIZE));
    let (rect, resp) = ui.allocate_exact_size(vec2(4.0 + 18.0 + 8.0 + g.size().x + 4.0, 30.0), Sense::click());
    let p = ui.painter();
    let c = pos2(rect.left() + 13.0, rect.center().y);
    p.circle(c, 8.5, if enabled { FIELD } else { BG }, Stroke::new(1.0, if checked || resp.hovered() { ACCENT } else { BORDER_STRONG }));
    if checked {
        p.circle_filled(c, 4.0, ACCENT);
    }
    p.galley(pos2(rect.left() + 30.0, rect.center().y - g.size().y / 2.0), g, if enabled { TEXT } else { TEXT_DISABLED });
    resp
}
pub fn radio_value<T: PartialEq>(ui: &mut Ui, cur: &mut T, v: T, label: &str) -> Response {
    let mut r = radio(ui, *cur == v, label);
    if r.clicked() && *cur != v {
        *cur = v;
        r.mark_changed();
    }
    r
}

// ---- SpinBox ------------------------------------------------------------------------------------------------------

/// style/SpinBox.qml: field with − / + buttons at the right, typed value clamped on Enter / focus loss
pub struct SpinBox<'a, N: egui::emath::Numeric> {
    v: &'a mut N,
    lo: f64,
    hi: f64,
    step: f64,
    decimals: usize,
    width: f32,
}

impl<'a, N: egui::emath::Numeric> SpinBox<'a, N> {
    pub fn new(v: &'a mut N, range: std::ops::RangeInclusive<N>) -> Self {
        SpinBox { v, lo: range.start().to_f64(), hi: range.end().to_f64(), step: 1.0, decimals: 0, width: 110.0 }
    }
    pub fn step(mut self, s: f64) -> Self {
        self.step = s;
        self
    }
    pub fn decimals(mut self, d: usize) -> Self {
        self.decimals = d;
        self
    }
    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }
}

impl<N: egui::emath::Numeric> Widget for SpinBox<'_, N> {
    fn ui(self, ui: &mut Ui) -> Response {
        let enabled = ui.is_enabled();
        let fmt = |v: f64| format!("{v:.*}", self.decimals);
        // never narrower than the value it shows needs beside the − + buttons (64 px of padding and buttons)
        let widest: String = format!("{:>2}", fmt(self.v.to_f64())).chars().take(10).collect();
        let need = 64.0 + ui.painter().layout_no_wrap(widest, font(FONT_SIZE), TEXT).size().x + 8.0;
        let (rect, mut resp) = ui.allocate_exact_size(vec2(self.width.max(need), CONTROL_HEIGHT), Sense::hover());
        let id = resp.id;
        let p = ui.painter().clone();
        let edit_id = id.with("edit");
        let focused = ui.memory(|m| m.has_focus(edit_id));
        p.rect(rect, RADIUS, if enabled { FIELD } else { BG }, Stroke::new(1.0, if focused { ACCENT } else { BORDER }), StrokeKind::Inside);
        let mut text = if focused { ui.data(|d| d.get_temp::<String>(edit_id)).unwrap_or_else(|| fmt(self.v.to_f64())) } else { fmt(self.v.to_f64()) };
        let tr = Rect::from_min_max(pos2(rect.left() + 10.0, rect.top() + 1.0), pos2(rect.right() - 54.0, rect.bottom() - 1.0));
        // a child Ui, not ui.put: put moves the parent's cursor back to the field's right edge (labels after the spin box
        // would land on its buttons)
        let te = ui.new_child(egui::UiBuilder::new().max_rect(tr).layout(egui::Layout::centered_and_justified(egui::Direction::TopDown))).add(egui::TextEdit::singleline(&mut text).id(edit_id).frame(egui::Frame::NONE).margin(egui::Margin::ZERO).vertical_align(egui::Align::Center).font(font(FONT_SIZE)).text_color(if enabled { TEXT } else { TEXT_DISABLED }));
        let (lo, hi) = (self.lo, self.hi);
        let set = |cur: &mut N, v: f64, resp: &mut Response| {
            let v = v.clamp(lo, hi);
            let v = if N::INTEGRAL { v.round() } else { v };
            if v != cur.to_f64() {
                *cur = N::from_f64(v);
                resp.mark_changed();
            }
        };
        if te.has_focus() {
            ui.data_mut(|d| d.insert_temp(edit_id, text.clone()));
        }
        if te.lost_focus() {
            if let Ok(v) = text.trim().parse::<f64>() {
                set(self.v, v, &mut resp);
            }
            ui.data_mut(|d| d.remove::<String>(edit_id));
        }
        for (k, glyph, dir) in [(0, "+", 1.0), (1, "−", -1.0)] {
            let x = rect.right() - 4.0 - 22.0 * (k as f32 + 1.0) - 2.0 * k as f32;
            let br = Rect::from_min_size(pos2(x, rect.top() + 4.0), vec2(22.0, rect.height() - 8.0));
            let b = ui.interact(br, id.with(k), Sense::click());
            let fill = if b.is_pointer_button_down_on() && enabled { ACCENT } else if b.hovered() && enabled { RAISED_HOVER } else { RAISED };
            p.rect(br, 4.0, fill, Stroke::new(1.0, BORDER), StrokeKind::Inside);
            p.text(br.center(), egui::Align2::CENTER_CENTER, glyph, bold(14.0), if enabled { TEXT } else { TEXT_DISABLED });
            if b.clicked() {
                let nv = self.v.to_f64() + dir * self.step;
                set(self.v, nv, &mut resp);
            }
        }
        resp
    }
}

// ---- TabButton / NumField -----------------------------------------------------------------------------------------

/// style/TabButton.qml: 36 px tall, raised when selected with a 2 px accent bar along the bottom
pub fn tab(ui: &mut Ui, selected: bool, text: &str, accent: Color32) -> Response {
    let g = galley(ui, text, font(FONT_SIZE));
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 24.0, 36.0), Sense::click());
    let p = ui.painter();
    let fill = if selected { RAISED } else if resp.hovered() { ROW_HOVER } else { Color32::TRANSPARENT };
    p.rect_filled(rect, 0.0, fill);
    if selected {
        p.rect_filled(Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.right_bottom()), 0.0, accent);
    }
    let col = if selected || resp.hovered() { TEXT } else { TEXT_DIM };
    p.galley(rect.center() - g.size() / 2.0, g, col);
    resp
}

/// NumField.qml: a text field with the unit badge inside at the right (empty text shows the unit as a placeholder)
pub fn unit_field(ui: &mut Ui, text: &mut String, suffix: &str, width: f32) -> Response {
    let badge = ui.painter().layout_no_wrap(suffix.to_owned(), font(11.0), TEXT_DIM);
    let bw = if suffix.is_empty() { 0 } else { badge.size().x as i8 + 4 };
    let r = ui.add_sized([width, CONTROL_HEIGHT - 2.0], field(text).hint_text(suffix).margin(egui::Margin { left: 9, right: 8 + bw, top: 5, bottom: 5 }));
    if !suffix.is_empty() {
        ui.painter().galley(pos2(r.rect.right() - 7.0 - badge.size().x, r.rect.center().y - badge.size().y / 2.0), badge, TEXT_DIM);
    }
    r
}

// ---- Menus --------------------------------------------------------------------------------------------------------

/// row rect for a 28 px menu row that ignores the menu's item spacing (Qt rows touch)
fn menu_row(ui: &mut Ui, w: f32, h: f32) -> (Rect, Response) {
    let gap = ui.spacing().item_spacing.y;
    // fill the menu's width, except while egui measures a new menu (sizing pass: the space is unbounded)
    let w = if ui.is_sizing_pass() { w } else { w.max(ui.available_width()) };
    let (r, _) = ui.allocate_exact_size(vec2(w, h - gap), Sense::hover());
    let full = r.expand2(vec2(0.0, gap / 2.0));
    let resp = ui.interact(full, ui.next_auto_id(), Sense::click());
    ui.skip_ahead_auto_ids(1);
    (full, resp)
}

/// style/MenuItem.qml: 28 px row, text at 30 px, shortcut dim at the right, blue rounded highlight; `check` draws
/// the 16 px tick box at 8 px
fn menu_row_paint(ui: &mut Ui, text: &str, shortcut: &str, check: Option<bool>, arrow: bool) -> Response {
    let enabled = ui.is_enabled();
    let g = galley(ui, text, font(FONT_SIZE));
    let shortcut = &*crate::theme::keys(shortcut);
    let sg = galley(ui, shortcut, font(FONT_SIZE - 1.0));
    let want = 30.0 + g.size().x + if shortcut.is_empty() { 0.0 } else { 28.0 + sg.size().x } + 12.0 + if arrow { 14.0 } else { 0.0 };
    let (rect, resp) = menu_row(ui, want.max(180.0), 28.0);
    let hl = enabled && (resp.hovered() || resp.has_focus());
    let p = ui.painter();
    if hl {
        p.rect_filled(Rect::from_min_max(pos2(rect.left() + 2.0, rect.top()), pos2(rect.right() - 2.0, rect.bottom())), 5.0, ACCENT);
    }
    if let Some(on) = check {
        let b = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 8.0), vec2(16.0, 16.0));
        p.rect(b, 4.0, if on { ACCENT } else { FIELD }, Stroke::new(1.0, if on { ACCENT_HOVER } else { BORDER_STRONG }), StrokeKind::Inside);
        if on {
            let o = b.left_top();
            p.add(egui::Shape::line(vec![o + vec2(3.5, 8.5), o + vec2(6.5, 11.5), o + vec2(12.5, 4.5)], Stroke::new(1.8, Color32::WHITE)));
        }
    }
    let col = if !enabled { TEXT_DISABLED } else if hl { Color32::WHITE } else { TEXT };
    p.galley(pos2(rect.left() + 30.0, rect.center().y - g.size().y / 2.0), g, col);
    if !shortcut.is_empty() {
        let sc = if !enabled { TEXT_DISABLED } else if hl { c(0xdfe9f5) } else { TEXT_DIM };
        p.galley(pos2(rect.right() - 12.0 - sg.size().x, rect.center().y - sg.size().y / 2.0), sg, sc);
    }
    if arrow {
        p.text(pos2(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, "›", font(16.0), if hl { Color32::WHITE } else { TEXT_DIM });
    }
    resp
}

pub fn menu_item(ui: &mut Ui, text: &str, shortcut: &str) -> Response {
    menu_row_paint(ui, text, shortcut, None, false)
}
/// checkable item; flips `on` when clicked
pub fn menu_check(ui: &mut Ui, text: &str, shortcut: &str, on: &mut bool) -> Response {
    let mut r = menu_row_paint(ui, text, shortcut, Some(*on), false);
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    r
}
/// style/MenuSeparator.qml: 9 px with a 1 px line
pub fn menu_separator(ui: &mut Ui) {
    let (r, _) = menu_row(ui, 0.0, 9.0);
    ui.painter().rect_filled(Rect::from_min_size(pos2(r.left() + 4.0, r.center().y.round()), vec2(r.width() - 8.0, 1.0)), 0.0, BORDER);
}
/// item that opens a submenu on hover (› at the right)
pub fn submenu<R>(ui: &mut Ui, text: &str, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let r = menu_row_paint(ui, text, "", None, true);
    egui::containers::menu::SubMenu::new().show(ui, &r, add).map(|i| i.inner)
}

/// Menu style for egui's menu popups: Qt rows touch, no button frames
pub fn menu_style(s: &mut egui::Style) {
    egui::containers::menu::menu_style(s);
    s.spacing.item_spacing.y = 0.0;
}

// ---- ToolTip ------------------------------------------------------------------------------------------------------

/// style/ToolTip.qml: near-black, radius 5, 12 px text. `resp.tip(text)` replaces `on_hover_text`.
pub trait TipExt {
    fn tip(self, text: impl Into<WidgetText>) -> Self;
    /// tooltip shown only while the widget is disabled (why it is greyed out)
    fn disabled_tip(self, text: impl Into<WidgetText>) -> Self;
}

/// paint over egui's popup frame (6 px margin) with the Qt tooltip colours
fn tip_ui(ui: &mut Ui, text: WidgetText) {
    let text = match text {
        WidgetText::Text(t) => WidgetText::Text(crate::theme::keys(&t).into_owned()), // "Undo (⌘Z)" -> "Undo (Ctrl+Z)" off Apple
        t => t,
    };
    let r = ui.max_rect().expand(6.0);
    ui.painter().rect(r, 5.0, c(0x101216).gamma_multiply(0.96), Stroke::new(1.0, BORDER_STRONG), StrokeKind::Inside);
    ui.add_space(0.0);
    ui.style_mut().override_font_id = Some(font(FONT_SIZE - 1.0));
    ui.add(egui::Label::new(text).wrap());
}

impl TipExt for Response {
    fn tip(self, text: impl Into<WidgetText>) -> Self {
        let text = text.into();
        self.on_hover_ui(|ui| tip_ui(ui, text))
    }
    fn disabled_tip(self, text: impl Into<WidgetText>) -> Self {
        let text = text.into();
        self.on_disabled_hover_ui(|ui| tip_ui(ui, text))
    }
}

const SHAPE_TILE: f32 = 86.0;

/// A shapes-library thumbnail: the outlines fitted in an 86 px square, the name under them (GCarve's Shapes
/// dialog, GBurn's Shapes Library).
pub fn shape_tile(ui: &mut egui::Ui, cs: &[gcarve_core::geom::Contour], name: &str, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(SHAPE_TILE, SHAPE_TILE + 26.0), egui::Sense::click());
    let hovered = resp.hovered();
    let p = ui.painter_at(rect);
    p.rect(rect.shrink(1.0), 6.0, if hovered { ROW_HOVER } else { FIELD }, Stroke::new(1.0, if hovered { ACCENT } else { BORDER }), egui::StrokeKind::Inside);
    let area = egui::Rect::from_min_size(rect.min + egui::vec2(12.0, 10.0), egui::vec2(SHAPE_TILE - 24.0, SHAPE_TILE - 24.0));
    let b = gcarve_core::clipart::bounds_all(cs);
    if !b.is_empty() {
        let k = (area.width() as f64 / b.w().max(1e-9)).min(area.height() as f64 / b.h().max(1e-9));
        let off = egui::vec2(((area.width() as f64 - b.w() * k) / 2.0) as f32, ((area.height() as f64 - b.h() * k) / 2.0) as f32);
        for c in cs {
            let pts: Vec<egui::Pos2> = gcarve_core::geom::flatten(c, b.w().max(b.h()) / 200.0).iter().map(|q| area.left_bottom() + off * egui::vec2(1.0, -1.0) + egui::vec2(((q.x - b.x0) * k) as f32, -((q.y - b.y0) * k) as f32)).collect();
            p.add(if c.closed { egui::Shape::closed_line(pts, Stroke::new(1.3, Color32::from_rgb(0x6c, 0xb6, 0xff))) } else { egui::Shape::line(pts, Stroke::new(1.3, Color32::from_rgb(0x6c, 0xb6, 0xff))) });
        }
    }
    let g = p.layout(name.to_string(), font(11.0), TEXT, SHAPE_TILE - 6.0);
    p.galley(egui::pos2(rect.center().x - g.size().x / 2.0, rect.bottom() - 32.0), g, TEXT); // up to two lines
    resp.tip(tip)
}
