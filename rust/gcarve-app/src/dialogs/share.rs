//! "Share…" with another GCarve user by email (bits from the Tool Library, machines from Job Setup, the open
//! project from File › Share Project…); the recipient adds / opens it from Shared with me (app.rs shares_ui).
//! Server: /api/shares (library.go).

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::cloud::Cloud;
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct ShareBox {
    open: bool,
    to: String,
}

impl ShareBox {
    /// already showing the email row
    pub fn opened() -> Self {
        ShareBox { open: true, to: String::new() }
    }

    /// the email row is showing (closed again once sent or cancelled)
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// A Share… button that opens an email row; `data` is built only when sent.
    pub fn ui(&mut self, ui: &mut egui::Ui, cloud: &mut Cloud, what: &str, kind: &str, name: &str, data: impl FnOnce() -> serde_json::Value) {
        if !cloud.signed_in() {
            return;
        }
        if !self.open {
            if ui.button(format!("Share {what}…")).on_hover_text(format!("Send {what} to another GCarve user by email")).clicked() {
                self.open = true;
            }
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Share {what} with")).small());
            let r = ui.add(crate::widgets::field(&mut self.to).hint_text(i18n::t("their email")).desired_width(170.0));
            let ok = self.to.contains('@');
            if ui.add_enabled(ok, egui::Button::new(i18n::t("Send"))).clicked() || (ok && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                cloud.share(ui.ctx(), self.to.trim(), kind, name, data());
                self.open = false;
                self.to.clear();
            }
            if ui.button(i18n::t("Cancel")).clicked() {
                self.open = false;
            }
        });
    }
}
