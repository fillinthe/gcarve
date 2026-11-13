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
    // GCARVE_E2E_SIZE=390x780: the same checks in a phone-sized window
    let size = std::env::var("GCARVE_E2E_SIZE").ok().and_then(|v| v.split_once('x').and_then(|(w, h)| Some(egui::vec2(w.parse().ok()?, h.parse().ok()?)))).unwrap_or(egui::vec2(1400.0, 900.0));
    let mut h = Harness::builder().with_size(size).with_max_steps(400).build_eframe(|cc| crate::app::App::new(cc));
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
/// shell.rs `menus` (a few plain words come along; they only give "Unknown command")
fn menu_ids() -> Vec<String> {
    let src = include_str!("shell.rs");
    let body = &src[src.find("pub fn menus").unwrap()..];
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
        // wider than the screen (a phone): tap the part that shows, as a person would
        let shows = r.intersect(h.ctx.content_rect());
        if h.ctx.content_rect().contains_rect(r) {
            node.click();
        } else if shows.width() >= 12.0 && shows.height() >= 12.0 {
            let at = shows.center();
            h.event(egui::Event::PointerMoved(at));
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(1);
            }
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

/// a value typed into a Properties field (it changes the drawing at once) comes back off with one Undo
#[test]
#[ignore]
fn typing_in_the_properties_panel_can_be_undone() {
    let mut bad = Vec::new();
    let harness = |demo| {
        let mut h = harness(demo);
        if h.ctx.content_rect().width() < 760.0 {
            h.state_mut().test_push("panels"); // folded at start on a narrow window: open it, as a person would
            h.run_steps(4);
        }
        h
    };
    let fields: Vec<Seen> = nodes(&harness(true)).into_iter().filter(|n| n.role == accesskit::Role::TextInput).collect();
    let mut tried = 0;
    for f in &fields {
        let mut h = harness(true);
        let was = gcarve_core::project::save_project(&h.state_mut().test_doc());
        if !click(&mut h, &f.id) {
            continue;
        }
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        if let Some(node) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == f.id) {
            node.type_text("3");
        }
        h.key_press(egui::Key::Enter);
        h.run_steps(4);
        if gcarve_core::project::save_project(&h.state_mut().test_doc()) == was {
            continue; // not a drawing field (search, zoom)
        }
        tried += 1;
        undo_redo(&mut h, &was, &format!("Properties field {:?}", f.label), &mut bad);
    }
    assert!(tried >= 4, "only {tried} of {} fields changed the drawing", fields.len());
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

/// press the OK (or Add / Import) of whatever opened since `before`; false when there is none
fn press_ok(h: &mut H, before: &HashSet<String>) -> bool {
    let Some(ok) = nodes(h).into_iter().find(|n| !before.contains(&n.id) && n.role == accesskit::Role::Button && matches!(n.label.as_str(), "OK" | "Add" | "Import")) else {
        if std::env::var("GCARVE_E2E_VERBOSE").is_ok() {
            eprintln!("no OK among: {:?} / windows {:?}", nodes(h).into_iter().filter(|n| !before.contains(&n.id)).map(|n| (n.role, n.label)).collect::<Vec<_>>(), nodes(h).into_iter().filter(|n| n.role == accesskit::Role::Window).map(|n| n.label).collect::<Vec<_>>());
        }
        return false;
    };
    let hit = click(h, &ok.id);
    h.run_steps(10);
    hit
}

/// run a command and press OK in what it opens
fn command_ok(h: &mut H, id: &str) -> bool {
    let before: HashSet<_> = nodes(h).into_iter().map(|n| n.id).collect();
    h.state_mut().test_push(id);
    h.run_steps(std::env::var("GCARVE_E2E_STEPS").ok().and_then(|v| v.parse().ok()).unwrap_or(6));
    press_ok(h, &before)
}

/// the job as File › Export writes it: some files, no safety findings, no toolpath warnings (`foreign`: a file made
/// elsewhere may carry its own, and cut a little into the spoilboard on purpose)
fn exports_cleanly_from(h: &mut H, what: &str, foreign: bool, bad: &mut Vec<String>) {
    let doc = h.state_mut().test_doc();
    let (r, warnings) = h.state_mut().test_export();
    let Some(r) = r else { return bad.push(format!("{what}: nothing to export")) };
    if r.files.is_empty() || r.files.iter().any(|f| !f.text.contains("G1") && !f.text.contains("G01")) {
        bad.push(format!("{what}: {} files, some without cuts ({} toolpaths)", r.files.len(), doc.ops.len()));
    }
    // saved and opened again, the project must give the same files (nothing lost in the save)
    let gcode = |d: &gcarve_core::document::Document| {
        use gcarve_core::pipeline;
        let post = pipeline::active_post(d, &|_| None);
        pipeline::export(d, &pipeline::regenerate(d), "p", "", 0.0, &post).files.into_iter().map(|f| (f.filename, f.text)).collect::<Vec<_>>()
    };
    match gcarve_core::project::load_project(&gcarve_core::project::save_project(&doc)) {
        Ok(back) if gcode(&back) != gcode(&doc) => bad.push(format!("{what}: saved and opened again, it exports different G-code")),
        Ok(_) => {}
        Err(e) => bad.push(format!("{what}: the saved project does not open: {e}")),
    }
    let safety: Vec<&String> = r.safety.iter().filter(|f| !(foreign && f.contains("into the spoilboard"))).collect();
    if !safety.is_empty() {
        bad.push(format!("{what}: export safety: {safety:?}"));
    }
    if !warnings.is_empty() && !foreign {
        bad.push(format!("{what}: toolpath warnings: {warnings:?}"));
    }
}

/// pick a drawing tool and click the middle of the canvas; type `words` into the box that opens; press OK / Add
fn draw_by_click(h: &mut H, tool: &str, words: Option<&str>) -> bool {
    let before: HashSet<_> = nodes(h).into_iter().map(|n| n.id).collect();
    h.state_mut().test_push(tool);
    h.run_steps(4);
    // the Text tool opens its box at once; the shape tools wait for a click
    if !nodes(h).iter().any(|n| !before.contains(&n.id) && n.role == accesskit::Role::Button && n.label == "OK") {
        let at = h.ctx.content_rect().center();
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(2);
        }
        h.run_steps(4);
    }
    if let Some(words) = words {
        let Some(f) = nodes(h).into_iter().find(|n| !before.contains(&n.id) && matches!(n.role, accesskit::Role::TextInput | accesskit::Role::MultilineTextInput)) else { return false };
        click(h, &f.id);
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        if let Some(node) = h.root().query_all(by()).find(|n| format!("{:?}", n.accesskit_node().id()) == f.id) {
            node.type_text(words);
        }
        h.run_steps(3);
    }
    press_ok(h, &before)
}

fn exports_cleanly(h: &mut H, what: &str, bad: &mut Vec<String>) {
    exports_cleanly_from(h, what, false, bad)
}

/// Undo must give back `was` exactly, Redo what was there before the Undo
/// (id counters such as "nextText" may stay ahead: they only keep ids unique)
fn undo_redo(h: &mut H, was: &str, what: &str, bad: &mut Vec<String>) {
    let plain = |t: &str| t.lines().filter(|l| !l.trim_start().starts_with("\"next")).collect::<Vec<_>>().join("\n");
    let was = plain(was);
    let save = |h: &mut H| plain(&gcarve_core::project::save_project(&h.state_mut().test_doc()));
    let after = save(h);
    h.state_mut().test_push("undo");
    h.run_steps(3);
    let undone = save(h);
    if undone != was {
        bad.push(format!("{what}: Undo did not give the drawing back ({})", first_difference(&was, &undone)));
    }
    h.state_mut().test_push("redo");
    h.run_steps(3);
    let redone = save(h);
    if redone != after {
        bad.push(format!("{what}: Redo did not bring the change back ({})", first_difference(&after, &redone)));
    }
}

/// where two project texts part: a little of each, for the message
fn first_difference(a: &str, b: &str) -> String {
    let i = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let i = (0..=i).rev().find(|&k| a.is_char_boundary(k) && b.is_char_boundary(k)).unwrap_or(0);
    let s = i.saturating_sub(60);
    let s = (0..=s).rev().find(|&k| a.is_char_boundary(k) && b.is_char_boundary(k)).unwrap_or(0);
    let cut = |t: &str| t[s..].chars().take(140).collect::<String>();
    format!("expected …{:?}, got …{:?}", cut(a), cut(b))
}

/// select everything, make each toolpath kind with its dialog's defaults, then the export must be clean
fn toolpaths_then_export(h: &mut H, kinds: &[&str], what: &str, bad: &mut Vec<String>) {
    for k in kinds {
        let n = h.state_mut().test_doc().ops.len();
        h.state_mut().test_push("selectall");
        h.run_steps(3);
        let was = gcarve_core::project::save_project(&h.state_mut().test_doc());
        if !command_ok(h, k) || h.state_mut().test_doc().ops.len() <= n {
            return bad.push(format!("{what}: {k}: no toolpath after OK"));
        }
        undo_redo(h, &was, &format!("{what}: {k}"), bad);
    }
    exports_cleanly(h, what, bad);
}

/// The tasks the app offers, done through the UI from start to G-code: each wizard with its own defaults and OK,
/// and a rectangle drawn with the mouse and cut out with a Profile. (`cargo test --release … -- --ignored`)
#[test]
#[ignore]
fn the_tasks_the_app_offers_end_in_clean_g_code() {
    let mut bad = Vec::new();
    // new-project wizards: OK must give a whole job
    for id in ["sign", "clock", "spoilboard", "doors", "cabinet", "tray"] {
        let mut h = harness(false);
        if !command_ok(&mut h, id) {
            bad.push(format!("{id}: no OK button"));
            continue;
        }
        if h.state_mut().test_doc().ops.is_empty() {
            bad.push(format!("{id}: OK made no toolpaths"));
            continue;
        }
        exports_cleanly(&mut h, id, &mut bad);
    }
    // drawing generators on an empty drawing: OK must add vectors
    for id in ["qr", "puzzle", "boxmaker", "hinge", "joinery"] {
        let mut h = harness(false);
        let n = h.state_mut().test_doc().shapes.len();
        if !command_ok(&mut h, id) {
            bad.push(format!("{id}: no OK button"));
        } else if h.state_mut().test_doc().shapes.len() <= n {
            bad.push(format!("{id}: OK added no vectors"));
        }
    }
    // plain jobs: a rectangle by a click with the Rectangle tool, then toolpaths on it, then Export
    for (kinds, what) in [(&["kind:0"][..], "rectangle + profile"), (&["kind:1"][..], "rectangle + pocket"), (&["inlay"][..], "rectangle + Inlay Wizard"),
        (&["kind:8"][..], "rectangle + chamfer"), (&["kind:9"][..], "rectangle + prism"),
        (&["kind:10"][..], "rectangle + texture"), (&["kind:6"][..], "rectangle + flute"), (&["kind:4"][..], "circle + bore")] {
        let mut h = harness(false);
        let tool = if what.starts_with("circle") { "tool:ellipse" } else { "tool:rect" };
        if !draw_by_click(&mut h, tool, None) || h.state_mut().test_doc().shapes.is_empty() {
            bad.push(format!("{tool} by a click: no size box, or no shape"));
            continue;
        }
        toolpaths_then_export(&mut h, kinds, what, &mut bad);
    }
    // words: the Text tool, a click, type them, OK, then V-carve them
    let mut h = harness(false);
    if !draw_by_click(&mut h, "tool:text", Some("OAK")) || h.state_mut().test_doc().texts.is_empty() {
        bad.push("text by a click: no text dialog, or no text".into());
    } else {
        toolpaths_then_export(&mut h, &["kind:3"], "text + V-carve", &mut bad);
    }
    // files opened as File › Open does: drawings get a Profile; projects, Carbide Create and G-code export as they are
    let files: [(&str, &[u8]); 5] = [
        ("demo.svg", include_bytes!("../../../samples/demo.svg")),
        ("demo.dxf", include_bytes!("../../../samples/demo.dxf")),
        ("cabinet.tlp", include_bytes!("../../../samples/cabinet.tlp")),
        ("small.c2d", include_bytes!("../../testdata/c2d/small.c2d")),
        ("01_V90.tap", include_bytes!("../../../reference/01_V90.tap")),
    ];
    for (name, bytes) in files {
        let mut h = harness(false);
        let before: HashSet<_> = nodes(&h).into_iter().map(|n| n.id).collect();
        h.state_mut().test_open(name, bytes);
        h.run_steps(6);
        press_ok(&mut h, &before); // G-code opens the import box first
        if h.state_mut().test_doc().shapes.is_empty() && h.state_mut().test_doc().ops.is_empty() {
            bad.push(format!("{name}: opened nothing"));
        } else if h.state_mut().test_doc().ops.is_empty() {
            toolpaths_then_export(&mut h, &["kind:0"], &format!("{name} + profile"), &mut bad);
        } else {
            exports_cleanly_from(&mut h, name, true, &mut bad);
        }
    }
    // the vector tools on a drawing (a rectangle and the word OAK, all selected): OK must change it
    // (Align acts from its own buttons: the button sweep presses those)
    for id in ["offset", "dogbone", "vtexture", "plates", "shelfpins", "stencil", "array", "nest"] {
        let mut h = harness(false);
        let words = if id == "plates" { "{1}" } else { "OAK" }; // a plate's text names a field of the list
        if !draw_by_click(&mut h, "tool:rect", None) || !draw_by_click(&mut h, "tool:text", Some(words)) {
            bad.push(format!("{id}: could not draw the rectangle and text"));
            continue;
        }
        h.state_mut().test_push("selectall");
        h.run_steps(3);
        let was = gcarve_core::project::save_project(&h.state_mut().test_doc());
        if !command_ok(&mut h, id) {
            bad.push(format!("{id}: no OK button"));
        } else if gcarve_core::project::save_project(&h.state_mut().test_doc()) == was {
            bad.push(format!("{id}: OK left the drawing as it was"));
        } else {
            undo_redo(&mut h, &was, id, &mut bad);
        }
    }
    assert!(bad.is_empty(), "{} problems:\n{}", bad.len(), bad.join("\n"));
}

