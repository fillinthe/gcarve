//! ⌘K command palette shared by GCarve and GSketch (Qt CommandPalette.qml): the app hands in its commands; type to
//! filter, ↑/↓ to pick, Enter to run, Esc or a click outside to close.

use crate::theme;
use eframe::egui::{self, Color32};

pub struct Command {
    pub cat: &'static str,
    pub text: String,
    /// shortcut, Mac style (theme::keys converts)
    pub key: String,
    /// app command id (App::run)
    pub id: String,
}

/// Qt ranking: text starts with the query, then a word starts with it, then any match (category included); 12 hits.
pub fn filter(cmds: &[Command], q: &str) -> Vec<usize> {
    let q = q.trim().to_lowercase();
    let mut scored: Vec<(i32, usize)> = cmds
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let t = c.text.to_lowercase();
            let r = if q.is_empty() { 3 } else if t.starts_with(&q) { 0 } else if format!(" {t}").contains(&format!(" {q}")) { 1 } else if format!("{} {t}", c.cat.to_lowercase()).contains(&q) { 2 } else { return None };
            Some((r, i))
        })
        .collect();
    scored.sort();
    scored.into_iter().take(12).map(|(_, i)| i).collect()
}

fn box_id() -> egui::Id {
    egui::Id::new("palette-query")
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    pub query: String,
    cur: usize,
}

impl Palette {
    pub fn show(&mut self) {
        *self = Palette { open: true, ..Default::default() };
    }

    /// Open with the search box focused at once, so letters typed in the frames before the box is drawn still reach
    /// it (and not the single-key tool shortcuts).
    pub fn open(&mut self, ctx: &egui::Context) {
        self.show();
        ctx.memory_mut(|m| m.request_focus(box_id()));
    }

    /// Draws the open palette; returns the command id to run. `hint` = the search box's placeholder.
    pub fn ui(&mut self, ctx: &egui::Context, cmds: &[Command], hint: &str) -> Option<String> {
        if !self.open {
            return None;
        }
        let hits = filter(cmds, &self.query);
        let (down, up, enter, esc) = ctx.input_mut(|i| {
            (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, egui::Key::Enter), i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        });
        if down {
            self.cur = (self.cur + 1).min(hits.len().saturating_sub(1));
        }
        if up {
            self.cur = self.cur.saturating_sub(1);
        }
        let mut run = enter.then(|| hits.get(self.cur).map(|&i| cmds[i].id.clone())).flatten();
        let before = self.query.clone();
        // focus the box before this frame's text events are handed out: letters typed right after ⌘K (or in the same
        // input batch, as a fast typist or the browser does) went to the single-key tool shortcuts and never arrived here
        let box_id = box_id();
        if !ctx.memory(|m| m.has_focus(box_id)) {
            ctx.memory_mut(|m| m.request_focus(box_id));
        }
        let win = egui::Window::new("palette").title_bar(false).resizable(false).fixed_size([560.0, 0.0]).anchor(egui::Align2::CENTER_TOP, [0.0, 90.0]).frame(egui::Frame::window(&ctx.global_style()).inner_margin(0)).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.add_sized([ui.available_width() - 8.0, 34.0], egui::TextEdit::singleline(&mut self.query).id(box_id).hint_text(hint).font(theme::font(15.0)));
            });
            ui.add_space(8.0);
            theme::hline(ui);
            ui.add_space(4.0);
            if hits.is_empty() {
                ui.vertical_centered(|ui| ui.label(egui::RichText::new(crate::i18n::t("No matching command")).color(theme::TEXT.gamma_multiply(0.5))));
            }
            for (row, &i) in hits.iter().enumerate() {
                let c = &cmds[i];
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width() - 8.0, 34.0), egui::Sense::click());
                let rect = rect.translate(egui::vec2(4.0, 0.0));
                if resp.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO) {
                    self.cur = row; // hover moves the highlight (Qt onHoveredChanged)
                }
                let fill = if row == self.cur { Color32::from_rgba_unmultiplied(61, 142, 230, 64) } else { Color32::TRANSPARENT };
                ui.painter().rect_filled(rect, 4.0, fill);
                ui.painter().text(rect.left_center() + egui::vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, c.cat, theme::font(11.0), theme::TEXT.gamma_multiply(0.45));
                ui.painter().text(rect.left_center() + egui::vec2(98.0, 0.0), egui::Align2::LEFT_CENTER, &c.text, theme::font(theme::FONT_SIZE), theme::TEXT);
                ui.painter().text(rect.right_center() - egui::vec2(10.0, 0.0), egui::Align2::RIGHT_CENTER, theme::keys(&c.key), theme::font(11.0), theme::TEXT.gamma_multiply(0.5));
                if resp.clicked() {
                    run = Some(c.id.clone());
                }
            }
            ui.add_space(4.0);
        });
        if self.query != before {
            self.cur = 0;
        }
        let outside = win.is_some_and(|w| ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !w.response.rect.contains(p))));
        if esc || run.is_some() || outside {
            self.open = false;
        }
        run
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_prefers_a_starting_match_then_a_word_then_the_category() {
        let c = |cat: &'static str, text: &str| Command { cat, text: text.into(), key: String::new(), id: text.to_lowercase() };
        let cmds = [c("Tool", "Push/Pull"), c("View", "Zoom Extents"), c("Tool", "Zoom"), c("Variable", "width = 24")];
        assert_eq!(filter(&cmds, "zoom"), vec![1, 2], "both start with it: list order");
        assert_eq!(filter(&cmds, "extents"), vec![1]);
        assert_eq!(filter(&cmds, "variable"), vec![3]);
        assert_eq!(filter(&cmds, "").len(), 4);
        assert!(filter(&cmds, "zzzz").is_empty());
    }
}
