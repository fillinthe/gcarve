//! The menu bar shared by GCarve and its sister apps: 27 px row, grey 12 px titles with 9 px padding, hovering
//! another title while a menu is open switches to it, a press anywhere else closes menus, phones get one ☰ menu,
//! and the right end holds the command search (⌘K).

use crate::widgets::{self, TipExt};
use crate::{icons, theme};
use eframe::egui::{self, Rect, Response, Ui};

/// egui closes a menu only on a full click (press + release without moving); a press that drags a little (the
/// canvas) left it open. Any press outside menus / popups and the menu bar closes them, as does losing focus.
/// `keep`: another rect whose presses must not close menus (a popup's own toggle).
fn close_on_press_elsewhere(ctx: &egui::Context, bar: Rect, keep: Rect) {
    if !ctx.input(|i| i.focused) && egui::Popup::is_any_open(ctx) {
        egui::Popup::close_all(ctx); // another window, or the site bar above the web app, took the click
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten()) else { return };
    if egui::Popup::is_any_open(ctx) && !bar.contains(p) && !keep.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l.order != egui::Order::Foreground) {
        egui::Popup::close_all(ctx);
    }
}

/// The bar. `menus` adds the top-level `ui.menu_button`s and pushes each one's response to `tops`; `right` draws
/// what sits at the right end (usually [`search_box`]). Returns the bar's rect.
pub fn bar(ui: &mut Ui, keep: Rect, mut menus: impl FnMut(&mut Ui, &mut Vec<Response>), right: impl FnOnce(&mut Ui)) -> Rect {
    let id = egui::Id::new("gc-menubar-rect");
    let last = ui.ctx().data(|d| d.get_temp::<Rect>(id)).unwrap_or(Rect::NOTHING);
    close_on_press_elsewhere(ui.ctx(), last, keep);
    let rect = egui::Panel::top("menu").frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin { left: 6, right: 6, top: 2, bottom: 2 })).show(ui, |ui| {
        ui.style_mut().text_styles.insert(egui::TextStyle::Button, theme::font(12.0));
        ui.spacing_mut().button_padding = egui::vec2(9.0, 3.0);
        ui.spacing_mut().interact_size.y = 23.0; // 27 px menu bar like the Qt one
        egui::MenuBar::new().config(egui::containers::menu::MenuConfig::new().style(widgets::menu_style)).style(|s: &mut egui::Style| {
            egui::containers::menu::menu_style(s);
            s.spacing.button_padding = egui::vec2(9.0, 4.0);
            s.spacing.item_spacing.x = 0.0;
        }).ui(ui, |ui| {
            let mut tops: Vec<Response> = Vec::new();
            // phones: every menu under one ☰ button (the titles don't fit in a row)
            if ui.ctx().content_rect().width() < 600.0 {
                let mut inner = Vec::new();
                tops.push(ui.menu_button("☰ Menu", |ui| menus(ui, &mut inner)).response);
            } else {
                menus(ui, &mut tops);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                right(ui);
            });
            // like a desktop menu bar: while one menu is open, hovering another title opens that one
            let ctx = ui.ctx().clone();
            if tops.iter().any(|r| egui::Popup::is_id_open(&ctx, egui::Popup::default_response_id(r))) {
                if let Some(h) = tops.iter().find(|r| r.hovered() && !egui::Popup::is_id_open(&ctx, egui::Popup::default_response_id(r))) {
                    egui::Popup::open_id(&ctx, egui::Popup::default_response_id(h));
                }
            }
        });
    }).response.rect;
    ui.ctx().data_mut(|d| d.insert_temp(id, rect));
    rect
}

/// The command search box (⌘K opens the same palette); narrow windows get only its icon. True when clicked.
pub fn search_box(ui: &mut Ui) -> bool {
    let wide = ui.ctx().content_rect().width() >= 1000.0;
    let (r, resp) = ui.allocate_exact_size(egui::vec2(if wide { 240.0 } else { 30.0 }, 22.0), egui::Sense::click());
    let resp = resp.tip("Search every command (⌘K)");
    let p = ui.painter();
    p.rect(r, theme::RADIUS as f32, theme::FIELD, egui::Stroke::new(1.0, if resp.hovered() { theme::BORDER_STRONG } else { theme::BORDER }), egui::StrokeKind::Inside);
    icons::paint(ui, "search", Rect::from_min_size(r.left_top() + egui::vec2(8.0, 4.0), egui::vec2(14.0, 14.0)), 14.0, theme::TEXT_DIM);
    if wide {
        ui.painter().text(r.left_center() + egui::vec2(28.0, 0.0), egui::Align2::LEFT_CENTER, "Search commands…", theme::font(12.0), theme::TEXT.gamma_multiply(0.55));
        let kb = Rect::from_min_max(egui::pos2(r.right() - 34.0, r.top() + 4.0), egui::pos2(r.right() - 5.0, r.bottom() - 4.0));
        ui.painter().rect_stroke(kb, 4.0, egui::Stroke::new(1.0, theme::BORDER_STRONG), egui::StrokeKind::Inside);
        ui.painter().text(kb.center(), egui::Align2::CENTER_CENTER, theme::keys("⌘K"), theme::font(10.0), theme::TEXT.gamma_multiply(0.6));
    }
    resp.clicked()
}
