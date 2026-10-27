//! Help › How Do I…? shared by GCarve and GSketch: short answers to "where is that?", searchable. The app hands in
//! its topics: (question, answer, command to run from the "Do it" button or "").

use crate::{theme, widgets};
use eframe::egui::{self, RichText};

pub type Topic = (&'static str, &'static str, &'static str);

#[derive(Default)]
pub struct Guide {
    pub open: bool,
    query: String,
}

impl Guide {
    /// Open with a search already typed (Help › Keyboard Shortcuts).
    pub fn open_for(&mut self, query: &str) {
        self.open = true;
        self.query = query.to_string();
    }

    /// Draws the open guide; returns a command to run (the "Do it" button of an answer).
    pub fn ui(&mut self, ctx: &egui::Context, topics: &[Topic], hint: &str) -> Option<&'static str> {
        if !self.open {
            return None;
        }
        let mut open = true;
        let mut run = None;
        widgets::Dialog::new(crate::i18n::t("How Do I…?")).id("guide").open(&mut open).close_footer().width(560.0).show(ctx, |ui| {
            let r = ui.add(widgets::field(&mut self.query).hint_text(hint).desired_width(560.0));
            widgets::focus_first(ui, &r);
            let words: Vec<String> = self.query.to_lowercase().split_whitespace().map(str::to_string).collect();
            egui::ScrollArea::vertical().max_height(widgets::list_height(ui, 60.0)).show(ui, |ui| {
                ui.set_width(560.0);
                let mut any = false;
                for &(q, a, cmd) in topics {
                    let (q, a) = (crate::i18n::t(q), crate::i18n::t(a));
                    let hay = format!("{q} {a}").to_lowercase();
                    if !words.iter().all(|w| hay.contains(w.as_str())) {
                        continue;
                    }
                    any = true;
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(q).font(widgets::bold(theme::FONT_SIZE)).color(egui::Color32::WHITE));
                        if !cmd.is_empty() {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button(crate::i18n::t("Do it")).clicked() {
                                    run = Some(cmd);
                                }
                            });
                        }
                    });
                    ui.add(egui::Label::new(RichText::new(theme::keys(a)).color(theme::TEXT.gamma_multiply(0.8))).wrap());
                }
                if !any {
                    ui.add_space(8.0);
                    ui.label(RichText::new(crate::i18n::t("Nothing matches. Try one word.")).color(theme::TEXT_DIM));
                }
            });
        });
        self.open = open && run.is_none();
        run
    }
}

/// Every topic filled in and asked once (each app's list runs this in a test).
pub fn check_topics(topics: &[Topic]) {
    let mut seen = std::collections::HashSet::new();
    for (q, a, _) in topics {
        assert!(q.len() > 5 && a.len() > 20, "{q}");
        assert!(seen.insert(q), "twice: {q}");
    }
}
