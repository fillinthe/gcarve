//! End-to-end UI checks: the real app driven through AccessKit (egui_kittest), the way a person uses it.
//! Every menu command is run; in whatever it opens, every field must keep what is typed into it and every checkbox
//! must toggle, and nothing may stop the app. (Owner 2026-11-03, after Properties X / Y / W / H took no typing.)

use eframe::egui;
use eframe::egui::accesskit;
use egui_kittest::kittest::{by, NodeT, Queryable};
use egui_kittest::Harness;
use std::collections::HashSet;

type H = Harness<'static, crate::app::App>;

/// a sandbox: no settings written, no pickers / browsers / downloads, no network, an empty config folder
fn sandbox() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        gcarve_ui::store::freeze();
        let home = std::env::temp_dir().join(format!("gcarve-ui-e2e-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&home);
        // ponytail: process-wide env, set once before any app starts; no other test reads the config folder
        std::env::set_var("HOME", &home);
        std::env::set_var("GCARVE_CLOUD_URL", "http://127.0.0.1:9");
    });
}

fn harness(demo: bool) -> H {
    sandbox();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(400).build_eframe(|cc| crate::app::App::new(cc));
    h.run_steps(4);
    if demo {
        // the showcase project (every toolpath type, layers, two sheets), everything selected, in Design
        h.state_mut().test_open("showcase.tlp", include_bytes!("../../../samples/showcase.tlp"));
        h.run_steps(4);
        h.state_mut().test_push("selectall");
        h.run_steps(3);
    }
    h
}

/// every command id in the menu bar: the last string of each `item(…, "id")` call or `(…, "id")` table row in
/// shell.rs `menu_bar` (a few plain words come along; they only give "Unknown command")
fn menu_ids() -> Vec<String> {
    let src = include_str!("shell.rs");
    let body = &src[src.find("pub fn menu_bar").unwrap()..];
    let body = &body[..body[1..].find("\npub fn ").unwrap_or(body.len() - 1)];
    let mut ids: Vec<String> = Vec::new();
    for (end, _) in body.match_indices("\")") {
        let Some(start) = body[..end].rfind('"') else { continue };
        let id = &body[start + 1..end];
        if id.starts_with(|c: char| c.is_ascii_lowercase()) && id.chars().all(|c| c.is_ascii_alphanumeric() || ":._-".contains(c)) && !ids.iter().any(|i| i == id) {
            ids.push(id.to_string());
        }
    }
    // and every "Do it" of Help › How Do I…? (the tasks the app tells people they can do)
    for (_, _, cmd) in crate::dialogs::guide::TOPICS {
        if !cmd.is_empty() && !ids.iter().any(|i| i == cmd) {
            ids.push(cmd.to_string());
        }
    }
    ids
}

#[derive(Clone)]
struct Seen {
    id: String,
    role: accesskit::Role,
    label: String,
    value: String,
    toggled: Option<bool>,
}

fn nodes(h: &H) -> Vec<Seen> {
    h.root()
        .query_all(by())
        .map(|n| {
            let a = n.accesskit_node();
            Seen { id: format!("{:?}", a.id()), role: a.role(), label: a.label().unwrap_or_default(), value: a.value().unwrap_or_default(), toggled: a.toggled().map(|t| t == accesskit::Toggled::True) }
        })
        .collect()
}

fn find(h: &H, id: &str) -> Option<Seen> {
    nodes(h).into_iter().find(|n| n.id == id)
}

/// like a person: scrolled into view, then clicked with the pointer (by AccessKit where the click can't land);
/// false when the node is gone or sits under another window (a floating dialog over the side panel)
fn click(h: &mut H, id: &str) -> bool {
    let Some(node) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == id) else { return false };
    node.scroll_to_me();
    h.run_steps(12); // the scroll animates: click once it has settled
    for _ in 0..2 {
        let Some(node) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == id) else { return false };
        let r = node.rect();
        // its own window, if any; any other window over its middle hides it
        let mut own = None;
        let mut p = node.accesskit_node().parent();
        while let Some(a) = p {
            if a.role() == accesskit::Role::Window {
                own = Some(a.id());
                break;
            }
            p = a.parent();
        }
        if h.root().query_all(by()).any(|w| w.accesskit_node().role() == accesskit::Role::Window && Some(w.accesskit_node().id()) != own && w.rect().contains(r.center())) {
            return false;
        }
        // only partly shown (the bottom edge of a scrolled list, under the dialog's OK / Cancel row): scroll on a bit
        if r.height() < 18.0 && h.ctx.content_rect().contains(r.center()) {
            h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -60.0), modifiers: egui::Modifiers::NONE, phase: egui::TouchPhase::Move });
            h.run_steps(8);
            continue;
        }
        if h.ctx.content_rect().contains_rect(r) {
            node.click();
        } else {
            node.click_accesskit();
        }
        return true;
    }
    false
}

/// type into every field and toggle every checkbox among the nodes not in `before`; problems go to `bad`
fn exercise(h: &mut H, what: &str, before: &HashSet<String>, bad: &mut Vec<String>) {
    use accesskit::Role;
    let fresh: Vec<Seen> = nodes(h).into_iter().filter(|n| !before.contains(&n.id)).collect();
    if std::env::var("GCARVE_E2E_VERBOSE").is_ok() {
        let count = |r: &[Role]| fresh.iter().filter(|n| r.contains(&n.role)).count();
        eprintln!("{what}: {} fields, {} checkboxes, {} drop-downs, {} buttons", count(&[Role::TextInput, Role::MultilineTextInput]), count(&[Role::CheckBox]), count(&[Role::ComboBox]), count(&[Role::Button]));
    }
    for f in fresh.iter().filter(|n| n.role == Role::TextInput || n.role == Role::MultilineTextInput) {
        let Some(node) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == f.id) else { continue };
        if node.accesskit_node().is_disabled() {
            continue;
        }
        if !click(h, &f.id) {
            continue; // gone, or under another window
        }
        h.run_steps(2);
        let focused = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == f.id).is_some_and(|n| n.is_focused());
        let hit = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == f.id).map(|n| n.rect());
        if !focused && std::env::var("GCARVE_E2E_VERBOSE").is_ok() {
            let p = hit.map(|r| r.center()).unwrap_or_default();
            let over: Vec<String> = h.root().query_all(by()).filter(|n| n.accesskit_node().bounding_box().is_some() && n.rect().contains(p)).map(|n| format!("{:?} {:?} {:?}", n.accesskit_node().role(), n.accesskit_node().label(), n.rect())).collect();
            eprintln!("{what}: click at {p:?} found {over:#?}");
        }
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(1);
        let Some(node) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == f.id) else { continue };
        node.type_text("7");
        h.run_steps(3);
        match find(h, &f.id) {
            Some(n) if n.value.contains('7') => {}
            Some(n) => bad.push(format!("{what}: field {:?} (was {:?}) did not keep typing: shows {:?} (focused after the click: {focused}, at {hit:?})", f.label, f.value, n.value)),
            None => {} // typing changed the dialog (a search filtered it): fine
        }
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
    }
    for f in fresh.iter().filter(|n| n.role == Role::ComboBox) {
        let Some(node) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == f.id) else { continue };
        if node.accesskit_node().is_disabled() {
            continue;
        }
        let was = node.accesskit_node().value().unwrap_or_default();
        if was == "English" {
            continue; // the language list: another language would stay set for the rest of the run
        }
        node.scroll_to_me();
        h.run_steps(12);
        let Some(node) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == f.id) else { continue };
        let shown: HashSet<String> = nodes(h).into_iter().map(|n| n.id).collect();
        let at = node.rect();
        node.click_accesskit();
        h.run_steps(3);
        // the list's choices: new buttons, the last one that isn't the current value
        let choice = h
            .root()
            .query_all(by())
            .filter(|n| {
                let a = n.accesskit_node();
                !shown.contains(&format!("{:?}", a.id())) && a.role() == Role::Button && a.label().is_some_and(|l| !l.is_empty() && l != was) && n.rect().min.x < at.max.x && n.rect().max.x > at.min.x
            })
            .map(|n| Seen { id: format!("{:?}", n.accesskit_node().id()), role: Role::Button, label: n.accesskit_node().label().unwrap_or_default(), value: String::new(), toggled: None })
            .collect::<Vec<_>>();
        // a menu (3D view's View / Mouse) runs commands and keeps its title: only lists that offer the current value
        // are choices; a choice that is the current value with a note ("✓ same size") is no change
        let listed = nodes(h).into_iter().any(|n| !shown.contains(&n.id) && n.label == was);
        if std::env::var("GCARVE_E2E_DUMP").is_ok() {
            eprintln!("{was:?} at {at:?}: {} candidates {:?}, listed {listed}", choice.len(), choice.iter().map(|c| c.label.clone()).collect::<Vec<_>>());
            eprintln!("{was:?} opened: {:?}", nodes(h).into_iter().filter(|n| !shown.contains(&n.id)).map(|n| format!("{:?} {:?} {:?}", n.role, n.label, n.value)).collect::<Vec<_>>());
        }
        let choice = if listed { choice.into_iter().rfind(|c| !c.label.starts_with(&was)) } else { None };
        let Some(choice) = choice else {
            if std::env::var("GCARVE_E2E_VERBOSE").is_ok() {
                eprintln!("{what}: drop-down {was:?} showed no other choice");
            }
            if nodes(h).into_iter().any(|n| !shown.contains(&n.id)) {
                h.key_press(egui::Key::Escape); // close what opened
                h.run_steps(2);
            }
            continue;
        };
        // the list floats over everything: AccessKit click (the window check in `click` would think it hidden)
        if let Some(c) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == choice.id) {
            c.click_accesskit();
        }
        h.run_steps(3);
        if find(h, &choice.id).is_some() {
            h.key_press(egui::Key::Escape); // the list is still open (the choice was scrolled away): close it
            h.run_steps(2);
        }
        if std::env::var("GCARVE_E2E_VERBOSE").is_ok() {
            eprintln!("{what}: drop-down {was:?} → picked {:?} → shows {:?}", choice.label, find(h, &f.id).map(|n| n.value));
        }
        if let Some(n) = find(h, &f.id) {
            if n.value == was {
                bad.push(format!("{what}: drop-down showing {was:?} did not change when {:?} was picked", choice.label));
            }
        }
    }
    for f in fresh.iter().filter(|n| n.role == Role::CheckBox) {
        let Some(node) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == f.id) else { continue };
        if node.accesskit_node().is_disabled() {
            continue;
        }
        let was = node.accesskit_node().toggled().map(|t| t == accesskit::Toggled::True);
        if !click(h, &f.id) {
            continue;
        }
        h.run_steps(3);
        if let Some(n) = find(h, &f.id) {
            if n.toggled == was {
                let r = h.root().query_all(by()).find(|x| format!("{:?}", x.accesskit_node().id()) == f.id).map(|x| x.rect());
                let p = r.map(|r| r.center()).unwrap_or_default();
                let over: Vec<String> = h.root().query_all(by()).filter(|n| n.accesskit_node().bounding_box().is_some() && n.rect().contains(p)).map(|n| format!("{:?} {:?}", n.accesskit_node().role(), n.accesskit_node().label())).collect();
                bad.push(format!("{what}: checkbox {:?} did not toggle (at {r:?}, under {over:?})", f.label));
            }
        }
    }
}

fn run_command(demo: bool, id: &str, bad: &mut Vec<String>) {
    let id = id.to_string();
    let mut problems = Vec::new();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut h = harness(demo);
        let before: HashSet<_> = nodes(&h).into_iter().map(|n| n.id).collect();
        h.state_mut().test_push(&id);
        h.run_steps(6);
        exercise(&mut h, &format!("{id} ({})", if demo { "showcase" } else { "empty drawing" }), &before, &mut problems);
    }));
    bad.extend(problems);
    if let Err(e) = r {
        let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
        bad.push(format!("{id} ({}): the app stopped: {msg}", if demo { "showcase" } else { "empty drawing" }));
    }
}

/// Slow in a debug build: `cargo test --release -p gcarve-app ui_e2e -- --ignored` (before every release)
#[test]
#[ignore]
fn every_menu_command_opens_and_its_fields_take_typing() {
    let ids = menu_ids();
    assert!(ids.len() > 80, "menu ids not found: {ids:?}");
    if std::env::var("GCARVE_E2E_VERBOSE").is_ok() {
        eprintln!("{} commands: {ids:?}", ids.len());
    }
    let only = std::env::var("GCARVE_E2E_ONLY").ok();
    let mut bad = Vec::new();
    for id in &ids {
        if matches!(id.as_str(), "quit" | "closetab") || only.as_deref().is_some_and(|o| !o.split(',').any(|o| id == o)) {
            continue;
        }
        run_command(true, id, &mut bad); // the showcase project, everything selected
        if id != "mcp" {
            run_command(false, id, &mut bad); // a new empty drawing (MCP: the first run holds port 47110, and says so)
        }
    }
    assert!(bad.is_empty(), "{} problems:\n{}", bad.len(), bad.join("\n"));
}

/// the side panel with the demo selected: Properties (X / Y / W / H: user report #16), Layers, Toolpaths
#[test]
fn the_side_panel_fields_take_typing() {
    let mut bad = Vec::new();
    let mut h = harness(true);
    exercise(&mut h, "side panel › Properties", &HashSet::new(), &mut bad);
    for tab in ["Layers", "Toolpaths"] {
        let before: HashSet<_> = nodes(&h).into_iter().map(|n| n.id).collect();
        let id = nodes(&h).into_iter().find(|n| n.label == tab).map(|n| n.id).unwrap_or_else(|| panic!("no {tab} tab"));
        click(&mut h, &id);
        h.run_steps(4);
        exercise(&mut h, &format!("side panel › {tab}"), &before, &mut bad);
    }
    assert!(bad.is_empty(), "{} problems:\n{}", bad.len(), bad.join("\n"));
}

/// Every button in whatever each menu command opens is pressed, one per fresh app, and the app must keep going (and
/// still save a project that loads). Slow in a debug build: `cargo test --release -p gcarve-app ui_e2e -- --ignored`.
#[test]
#[ignore]
fn every_button_in_every_dialog_can_be_pressed() {
    let only = std::env::var("GCARVE_E2E_ONLY").ok();
    let mut bad = Vec::new();
    let mut pressed = 0;
    for id in menu_ids() {
        if matches!(id.as_str(), "quit" | "closetab") || only.as_deref().is_some_and(|o| !o.split(',').any(|o| id == o)) {
            continue;
        }
        let open = |id: &str| {
            let mut h = harness(true);
            let before: HashSet<_> = nodes(&h).into_iter().map(|n| n.id).collect();
            h.state_mut().test_push(id);
            h.run_steps(6);
            let buttons: Vec<Seen> = nodes(&h).into_iter().filter(|n| !before.contains(&n.id) && n.role == accesskit::Role::Button).collect();
            (h, buttons)
        };
        let Ok((_, buttons)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| open(&id))) else {
            bad.push(format!("{id}: the app stopped opening it"));
            continue;
        };
        for b in &buttons {
            pressed += 1;
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let (mut h, _) = open(&id);
                if let Some(node) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == b.id) {
                    if !node.accesskit_node().is_disabled() {
                        node.click_accesskit();
                    }
                }
                h.run_steps(8);
                let doc = gcarve_core::project::save_project(&h.state_mut().test_doc());
                gcarve_core::project::load_project(&doc).map(|_| ())
            }));
            match r {
                Ok(Ok(())) => {}
                Ok(Err(e)) => bad.push(format!("{id} › {:?}: the project no longer loads: {e}", b.label)),
                Err(e) => {
                    let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    bad.push(format!("{id} › {:?}: the app stopped: {msg}", b.label));
                }
            }
        }
    }
    eprintln!("{pressed} buttons pressed");
    assert!(bad.is_empty(), "{} problems:\n{}", bad.len(), bad.join("\n"));
}
