//! The account box shared by GCarve and GSketch: sign in, create an account, the plan, and the desktop licence gate.
//! Names in it come from `store::app()`; the server and the account are the same for both apps.

use crate::cloud::Cloud;
use crate::{theme, widgets};
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct Account {
    pub open: bool,
    /// the first session check at start: no flash of the sign-in box while the server answers
    pub startup: bool,
    /// the website sign-in page is open in another tab: poll the session until it is signed in
    pub waiting: bool,
    polled: f64,
    /// email, password, (unused), two-step code
    pub fields: [String; 4],
    /// blocking: the desktop licence (or the site's sign-in rule) holds the app until signed in / licensed
    pub gate: bool,
    /// why the box opened (shown above the fields)
    pub reason: String,
}

impl Account {
    /// The account box: sign in / create an account, or the signed-in summary (plan, links, sign out). Blocking while
    /// `gate` is set (desktop licence, or the site's sign-in rule), else a closable dialog.
    pub fn show(&mut self, ctx: &egui::Context, cloud: &mut Cloud) {
        if !self.open {
            return;
        }
        if self.startup {
            if !cloud.checked {
                return; // still asking the server: no flash of the sign-in box
            }
            self.startup = false;
            if cloud.signed_in() {
                self.open = false; // signed in on the site already: straight into the app
                return;
            }
        }
        if self.gate && (if Cloud::web() { cloud.signed_in() } else { cloud.licensed() }) {
            self.gate = false; // licensed now: straight into the app
            self.open = false;
            return;
        }
        let gate = self.gate; // desktop licence only: the web app is open
        let mut open = true;
        let app = crate::store::app();
        let title = if cloud.signed_in() { format!("{app} account") } else if self.reason.starts_with("You were signed out") { crate::i18n::t("Sign in again").into() } else { format!("Sign in to {app}") };
        let reason = self.reason.clone();
        let body = |ui: &mut egui::Ui| {
            if !reason.is_empty() && !cloud.signed_in() {
                ui.label(RichText::new(&reason).color(theme::ACCENT));
                ui.add_space(4.0);
            }
            if let Some(u) = cloud.user.clone() {
                self.fields[1].clear(); // signed in: forget the typed password
                ui.label(RichText::new(format!("Signed in as {}", u.email)).family(widgets::bold_family()));
                let plan = if cloud.ent.plan == "pro" { format!("{app} Pro") } else { format!("Free plan · {} cloud projects", cloud.ent.max_projects) };
                let desk = if !Cloud::web() && !cloud.ent.desktop { format!(" · the desktop app is part of {app} Pro") } else { String::new() };
                let role = match u.role.as_str() { "superadmin" => crate::i18n::t(" · super admin"), "admin" => crate::i18n::t(" · admin"), _ => "" };
                ui.label(RichText::new(format!("{plan}{role}{desk}")).color(theme::TEXT_DIM));
                if self.gate {
                    let grace = cloud.grace_days_left();
                    let note = if grace > 0 { format!(" (Licence still valid offline for {grace} day(s).)") } else { String::new() };
                    ui.label(RichText::new(format!("The desktop app is part of {app} Pro. Upgrade on your account page, then press Check again.{note}")).color(theme::ORANGE));
                    if ui.add_enabled(cloud.busy == 0, egui::Button::new(crate::i18n::t("Check again"))).clicked() {
                        cloud.check_session(ui.ctx());
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if cloud.ent.plan != "pro" && ui.button(crate::i18n::t("Upgrade to Pro")).clicked() {
                        cloud.open_web(ui.ctx(), "/account/");
                    }
                    if ui.button(crate::i18n::t("Dashboard")).clicked() {
                        cloud.open_web(ui.ctx(), "/dashboard/");
                    }
                    if (u.role == "admin" || u.role == "superadmin") && ui.button(crate::i18n::t("User management")).clicked() {
                        cloud.open_web(ui.ctx(), "/admin/");
                    }
                    if ui.button(crate::i18n::t("♥ Donate")).clicked() {
                        cloud.open_web(ui.ctx(), "/donate/");
                    }
                    if ui.button(crate::i18n::t("Sign out")).clicked() {
                        cloud.sign_out(ui.ctx());
                    }
                });
            } else {
                // one way in for everyone: the website handles sign-in AND sign-up (email link, passkey, password); the
                // password fields below are only a shortcut for people who already have a password
                if let Some(d) = cloud.device.as_ref().filter(|_| !Cloud::web()) {
                    let (code, url) = (d.code.clone(), d.url.clone());
                    ui.label(crate::i18n::t("Finish in your browser. Allow the sign-in there if it shows this code:"));
                    ui.label(RichText::new(code).size(theme::FONT_SIZE * 1.8).family(widgets::bold_family()));
                    ui.horizontal(|ui| {
                        if ui.button(crate::i18n::t("Open the page again")).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        }
                        if ui.button(crate::i18n::t("Cancel")).clicked() {
                            cloud.cancel_browser_sign_in();
                        }
                    });
                    if !cloud.error.is_empty() && cloud.error != "sign in required" {
                        ui.add(egui::Label::new(RichText::new(&cloud.error).color(theme::DANGER)).wrap());
                    }
                    return;
                }
                if Cloud::web() && self.waiting {
                    ui.label(crate::i18n::t("Finish in the tab that just opened. This box closes by itself when you are signed in."));
                    // the site's cookie is shared with this tab: ask the server every 3 s
                    let now = ui.input(|i| i.time);
                    if cloud.busy == 0 && now - self.polled > 3.0 {
                        self.polled = now;
                        cloud.check_session(ui.ctx());
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                    ui.add_space(4.0);
                }
                let go = if Cloud::web() { if self.waiting { crate::i18n::t("Open the sign-in page again") } else { crate::i18n::t("Sign in or create a free account") } } else { crate::i18n::t("Sign in with your browser") };
                if ui.add_enabled(cloud.busy == 0, widgets::Button::new(go).primary(true)).on_hover_text(crate::i18n::t("Sign in or sign up on the website with your email link, passkey or password")).clicked() {
                    if Cloud::web() {
                        self.waiting = true;
                        cloud.open_web(ui.ctx(), "/signin/?signup&next=/app/");
                    } else {
                        cloud.browser_sign_in(ui.ctx());
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new(crate::i18n::t("Already have a password? Sign in here:")).color(theme::TEXT_DIM).small());
                let w = ui.available_width();
                ui.label(crate::i18n::t("Email"));
                ui.add(widgets::field(&mut self.fields[0]).desired_width(w));
                ui.label(crate::i18n::t("Password"));
                ui.add(widgets::field(&mut self.fields[1]).password(true).desired_width(w));
                if cloud.need_code {
                    ui.label(crate::i18n::t("Code from your authenticator app (or a backup code)"));
                    ui.add(widgets::field(&mut self.fields[3]).desired_width(w));
                }
                // "sign in required" is only the answer to the session check of a signed-out visitor, not a mistake of theirs
                if !cloud.error.is_empty() && cloud.error != "sign in required" {
                    ui.add(egui::Label::new(RichText::new(&cloud.error).color(theme::DANGER)).wrap());
                }
                ui.horizontal(|ui| {
                    let ready = cloud.busy == 0 && self.fields[0].len() > 3 && !self.fields[1].is_empty();
                    if ui.add_enabled(ready, widgets::Button::new(if cloud.busy > 0 { "…" } else { crate::i18n::t("Sign in") })).clicked() {
                        cloud.sign_in(ui.ctx(), &self.fields[0].clone(), &self.fields[1].clone(), &self.fields[3].clone());
                        self.fields[3].clear(); // codes are single-use; the password stays for the code step
                    }
                    if ui.add(egui::Link::new(RichText::new(crate::i18n::t("Forgot password?")).small().color(theme::GROUP_ARRANGE))).clicked() {
                        cloud.open_web(ui.ctx(), "/signin/?forgot");
                    }
                });
                if self.gate {
                    ui.add_space(4.0);
                    let text = if Cloud::web() { format!("{app} online needs an account: a free one is enough, and your work is kept for you.") } else { format!("The desktop app needs a {app} Pro account (it keeps working offline for 14 days after each sign-in check).") };
                    ui.label(RichText::new(text).color(theme::TEXT_DIM).small());
                }
            }
        };
        if gate {
            // blocking: nothing behind it reacts until signed in / licensed
            widgets::Dialog::new(title.clone()).id("account").width(380.0).show(ctx, body);
        } else {
            widgets::Dialog::new(title.clone()).id("account").open(&mut open).close_footer().width(380.0).show(ctx, body);
        }
        if !open {
            self.open = false;
            self.reason.clear();
        }
    }
}
