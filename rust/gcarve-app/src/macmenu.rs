//! The macOS menu bar at the top of the screen (owner 2026-11-03): the same menus as the window's own bar
//! (`shell::menus`), recorded each frame and handed to AppKit through `muda`. Only a change in the menus' layout or
//! wording builds them again; enabled / ticked states are set in place. A choice comes back as a command id.
//! Shortcuts stay with the app's own key handling (a native ⌘C / ⌘V would take copy and paste away from text
//! fields), so the menus show none, except ⌘Q, which the app menu needs.

use crate::canvas::Canvas;
use crate::shell::{self, MenuInfo, MenuSink};
use eframe::egui;
use gcarve_ui::i18n;
use muda::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::sync::mpsc;

#[derive(PartialEq)]
enum Node {
    Item { text: String, on: bool, id: String },
    Check { text: String, on: bool, ticked: bool, id: String },
    Sep,
    Sub { text: String, on: bool, kids: Vec<Node> },
}

/// `menus` written down as nodes; About and Preferences go to the app menu instead
#[derive(Default)]
struct Rec(Vec<Node>);

impl MenuSink for Rec {
    fn item(&mut self, text: &str, _key: &str, on: bool, id: &str) {
        if !matches!(id, "about" | "prefs") {
            self.0.push(Node::Item { text: text.into(), on, id: id.into() });
        }
    }
    fn check(&mut self, text: &str, _key: &str, on: bool, ticked: bool, id: &str) {
        self.0.push(Node::Check { text: text.into(), on, ticked, id: id.into() });
    }
    fn sep(&mut self) {
        self.0.push(Node::Sep);
    }
    fn sub(&mut self, title: &str, on: bool, add: &mut dyn FnMut(&mut dyn MenuSink)) {
        let mut r = Rec::default();
        add(&mut r);
        self.0.push(Node::Sub { text: title.into(), on, kids: r.0 });
    }
}

/// what makes a rebuild: everything but the enabled / ticked states
fn layout(nodes: &[Node], out: &mut String) {
    for n in nodes {
        match n {
            Node::Item { text, id, .. } => out.push_str(&format!("i{text}\u{1}{id}\u{2}")),
            Node::Check { text, id, .. } => out.push_str(&format!("c{text}\u{1}{id}\u{2}")),
            Node::Sep => out.push('-'),
            Node::Sub { text, kids, .. } => {
                out.push_str(&format!("s{text}\u{1}"));
                layout(kids, out);
                out.push('\u{3}');
            }
        }
    }
}

enum Handle {
    Item(MenuItem),
    Check(CheckMenuItem),
    Sub(Submenu),
}

#[derive(Default)]
pub struct MacMenu {
    menu: Option<Menu>,
    layout: String,
    /// every item in build order, with the state it has now
    handles: Vec<(Handle, bool, bool)>,
    rx: Option<mpsc::Receiver<String>>,
}

/// AppKit shows '&' only doubled (muda reads a single one as a Windows access key)
fn label(t: &str) -> String {
    t.replace('&', "&&")
}

fn build(parent: &Submenu, nodes: &[Node], handles: &mut Vec<(Handle, bool, bool)>) {
    // no separator first, last or twice in a row (About / Preferences left the Edit and Help menus)
    let mut last_sep = true;
    let n = nodes.iter().rposition(|n| *n != Node::Sep).map_or(0, |i| i + 1);
    for node in &nodes[..n] {
        match node {
            Node::Sep => {
                if !last_sep {
                    let _ = parent.append(&PredefinedMenuItem::separator());
                }
                last_sep = true;
                continue;
            }
            Node::Item { text, on, id } => {
                let it = MenuItem::with_id(id.as_str(), label(text), *on, None);
                let _ = parent.append(&it);
                handles.push((Handle::Item(it), *on, false));
            }
            Node::Check { text, on, ticked, id } => {
                let it = CheckMenuItem::with_id(id.as_str(), label(text), *on, *ticked, None);
                let _ = parent.append(&it);
                handles.push((Handle::Check(it), *on, *ticked));
            }
            Node::Sub { text, on, kids } => {
                let sub = Submenu::new(label(text), *on);
                build(&sub, kids, handles);
                let _ = parent.append(&sub);
                handles.push((Handle::Sub(sub), *on, false));
            }
        }
        last_sep = false;
    }
}

/// the enabled / ticked states in build order (the same walk as `build`)
fn states(nodes: &[Node], out: &mut Vec<(bool, bool)>) {
    let n = nodes.iter().rposition(|n| *n != Node::Sep).map_or(0, |i| i + 1);
    for node in &nodes[..n] {
        match node {
            Node::Sep => {}
            Node::Item { on, .. } => out.push((*on, false)),
            Node::Check { on, ticked, .. } => out.push((*on, *ticked)),
            Node::Sub { on, kids, .. } => {
                states(kids, out);
                out.push((*on, false));
            }
        }
    }
}

impl MacMenu {
    /// record the menus for this frame and bring the screen's menu bar up to date
    pub fn update(&mut self, ctx: &egui::Context, canvas: &Canvas, m: &MenuInfo, rail_open: bool, panel_open: bool, has_ops: bool) {
        if self.rx.is_none() {
            let (tx, rx) = mpsc::channel();
            let ctx = ctx.clone();
            // set once per process; a choice wakes the app so it runs at once
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let _ = tx.send(e.id.0);
                ctx.request_repaint();
            }));
            self.rx = Some(rx);
        }
        let mut tops: Vec<(String, Vec<Node>)> = Vec::new();
        shell::menus(&mut |title, add| {
            let mut r = Rec::default();
            add(&mut r);
            tops.push((title.to_string(), r.0));
        }, canvas, m, rail_open, panel_open, has_ops);
        let mut key = String::new();
        for (t, n) in &tops {
            key.push_str(t);
            key.push('\u{4}');
            layout(n, &mut key);
        }
        if self.menu.is_none() || key != self.layout {
            self.rebuild(&tops);
            self.layout = key;
            return;
        }
        let mut now = Vec::new();
        for (_, n) in &tops {
            states(n, &mut now);
        }
        for ((h, on, ticked), (want_on, want_ticked)) in self.handles.iter_mut().zip(now) {
            if *on != want_on {
                match h {
                    Handle::Item(i) => i.set_enabled(want_on),
                    Handle::Check(c) => c.set_enabled(want_on),
                    Handle::Sub(s) => s.set_enabled(want_on),
                }
                *on = want_on;
            }
            if let (Handle::Check(c), true) = (&*h, *ticked != want_ticked) {
                c.set_checked(want_ticked);
                *ticked = want_ticked;
            }
        }
    }

    fn rebuild(&mut self, tops: &[(String, Vec<Node>)]) {
        let menu = Menu::new();
        // the app menu (its title is the app's name, whatever is given here)
        let app = Submenu::new(i18n::t("GCarve"), true);
        let quit = MenuItem::with_id("quit", i18n::t("Quit GCarve"), true, Some(muda::accelerator::Accelerator::new(muda::accelerator::Modifiers::META, muda::accelerator::Code::KeyQ)));
        let items: [&dyn IsMenuItem; 10] = [
            &MenuItem::with_id("about", i18n::t("About GCarve"), true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("prefs", i18n::t("Preferences…"), true, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
        ];
        let _ = app.append_items(&items);
        let _ = app.append(&quit);
        let _ = menu.append(&app);
        let mut handles = Vec::new();
        for (title, nodes) in tops {
            let sub = Submenu::new(label(title), true);
            build(&sub, nodes, &mut handles);
            let _ = menu.append(&sub);
        }
        menu.init_for_nsapp();
        // GCarve has its own tabs: no "Show Tab Bar" / "Show All Tabs" added to the View menu by AppKit
        // SAFETY: a class method taking a BOOL, on the main thread (the frame runs there)
        unsafe {
            let _: () = objc2::msg_send![objc2::class!(NSWindow), setAllowsAutomaticWindowTabbing: false];
        }
        self.menu = Some(menu); // the old one goes once it is no longer the app's
        self.handles = handles;
    }

    /// the next menu choice, as a command id
    pub fn take(&mut self) -> Option<String> {
        self.rx.as_ref()?.try_recv().ok()
    }
}
