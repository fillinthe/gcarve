//! The right-hand inspector of the "modes" layout (Main.qml): Properties / Layers / Toolpaths tabs, scrolling
//! sections, pinned Edit / Delete at the bottom. Commands go through `Shell::cmds` like the rest of the shell.

use crate::canvas::{hex, kind_color, Canvas};
use crate::widgets::TipExt;
use crate::icons::{self, IconButton};
use crate::shell::{self, Shell};
use crate::theme;
use eframe::egui::{self, Color32, RichText};
use gcarve_core::document::OpType;
use gcarve_core::pipeline::Generated;

pub const KIND_TYPES: [OpType; 12] = [OpType::Profile, OpType::Pocket, OpType::Drill, OpType::VCarve, OpType::Bore, OpType::Inlay, OpType::Flute, OpType::Relief, OpType::Chamfer, OpType::Prism, OpType::Texture, OpType::Photo];

pub fn inspector(ui: &mut egui::Ui, sh: &mut Shell, canvas: &mut Canvas, gen: Option<&Generated>) {
    let warnings: Vec<&str> = gen.map(|g| g.warnings.iter().map(String::as_str).collect()).unwrap_or_default();
    // tab bar + collapse chevron
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    ui.horizontal(|ui| {
        let tabs = ["Properties", "Layers", if warnings.is_empty() { "Toolpaths" } else { "Toolpaths ⚠" }];
        let accents = [theme::MODE_DESIGN, theme::GROUP_SETUP, theme::MODE_TOOLPATHS];
        for (i, t) in tabs.iter().enumerate() {
            if theme::tab(ui, sh.side_tab == i, t, accents[i]).clicked() {
                sh.side_tab = i;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::shell::collapse_button(ui, false, true, "the side panel", "⇧⌘P") {
                sh.push("fold:right");
            }
        });
    });
    theme::hline(ui);
    // pinned bottom: warnings + Edit / Delete
    egui::Panel::bottom("insp-actions").frame(egui::Frame::new().inner_margin(10)).show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        if !warnings.is_empty() {
            ui.add(egui::Label::new(RichText::new(format!("⚠ {}", warnings.join("\n"))).size(12.0).color(theme::WARNING)).wrap());
        }
        ui.horizontal(|ui| {
            let on = sh.op_sel.is_some_and(|i| i < canvas.doc.ops.len());
            if ui.add_enabled_ui(on, |ui| IconButton::new("edit").text("Edit").show(ui)).inner.clicked() {
                sh.push(format!("opedit:{}", sh.op_sel.unwrap_or(0)));
            }
            if ui.add_enabled_ui(on, |ui| IconButton::new("delete").text("Delete").show(ui)).inner.clicked() {
                sh.push(format!("opdelete:{}", sh.op_sel.unwrap_or(0)));
            }
            if IconButton::new("save").size(16.0).padding(4.0).tip("Save as template… / apply a template: toolpaths, bits and variables, re-applied by layer name").show(ui).clicked() {
                sh.push("templates");
            }
        });
    });
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(10).show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
            match sh.side_tab {
                0 => properties(ui, sh, canvas),
                1 => layers(ui, sh, canvas),
                _ => toolpaths(ui, sh, canvas, gen),
            }
        });
    });
}

/// Measure tool: the measured segment in full (points, distance, ΔX / ΔY, angle, midpoint)
fn measure_panel(ui: &mut egui::Ui, canvas: &Canvas) {
    ui.label(RichText::new("Measure").family(crate::widgets::bold_family()).color(Color32::WHITE));
    let Some((a, b)) = canvas.measure_seg else {
        ui.add(egui::Label::new(RichText::new("Click two points on the drawing. It snaps to corners, midpoints, centres and edges, also with Snap off.").color(theme::TEXT_DIM)).wrap());
        return;
    };
    let d = &canvas.doc;
    let f = |v: f64| crate::units::fmt(d, v);
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let mut ang = dy.atan2(dx).to_degrees();
    if ang < 0.0 {
        ang += 360.0;
    }
    let done = canvas.measure.len() >= 2;
    egui::Grid::new("measure").num_columns(2).spacing([14.0, 5.0]).show(ui, |ui| {
        let mut row = |k: &str, v: String, strong: bool| {
            ui.label(RichText::new(k).color(theme::TEXT_DIM));
            ui.label(if strong { RichText::new(v).size(15.0).family(crate::widgets::bold_family()).color(Color32::WHITE) } else { RichText::new(v) });
            ui.end_row();
        };
        row("Distance", f(dx.hypot(dy)), true);
        row("ΔX", format!("{}{}", if dx < -1e-9 { "−" } else { "" }, f(dx.abs())), false);
        row("ΔY", format!("{}{}", if dy < -1e-9 { "−" } else { "" }, f(dy.abs())), false);
        row("Angle", format!("{ang:.2}° from +X"), false);
        row("Start", format!("X {}  Y {}", f(a.x), f(a.y)), false);
        row(if done { "End" } else { "Pointer" }, format!("X {}  Y {}", f(b.x), f(b.y)), false);
        row("Midpoint", format!("X {}  Y {}", f((a.x + b.x) / 2.0), f((a.y + b.y) / 2.0)), false);
    });
    ui.add(egui::Label::new(RichText::new(if done { "Click again to start a new measurement · Esc clears it" } else { "Click the second point" }).small().color(theme::TEXT_DIM)).wrap());
}

fn properties(ui: &mut egui::Ui, sh: &mut Shell, canvas: &mut Canvas) {
    if canvas.tool == crate::canvas::Tool::Measure {
        measure_panel(ui, canvas);
        ui.add_space(8.0);
        theme::hline(ui);
        ui.add_space(4.0);
    }
    let sel = canvas.selection_count();
    let (uname, u, dec) = shell::unit(&canvas.doc);
    ui.label(RichText::new(if sel > 0 { format!("{sel} selected") } else { "Nothing selected".into() }).family(crate::widgets::bold_family()).color(Color32::WHITE));
    // X Y / W H fields with the unit badge
    let bounds = canvas.selection_size();
    let focused = ["x", "y", "w", "h"].iter().any(|k| ui.memory(|m| m.has_focus(ui.id().with(*k))));
    if let Some(b) = bounds {
        if sh.edit_for != canvas.selected() || !focused {
            let bound = canvas.selection_bindings(); // a bound field shows its expression
            sh.edit = [("x", b.x0), ("y", b.y0), ("w", b.w()), ("h", b.h())].map(|(k, v)| bound.get(k).cloned().unwrap_or_else(|| format!("{:.dec$}", v * u)));
            sh.edit_for = canvas.selected();
        }
    } else {
        sh.edit = Default::default();
        sh.edit_for = None;
    }
    let mut commit = None;
    if sel == 0 {
        next_step(ui, sh, canvas);
    }
    ui.add_enabled_ui(sel > 0, |ui| {
        if sel == 0 {
            return; // nothing to size: the fields come back with a selection
        }
        ui.spacing_mut().interact_size.y = theme::CONTROL_HEIGHT - 2.0; // 28 px field rows, 6 apart (Qt GridLayout)
        let fw = (ui.available_width() - 2.0 * 14.0 - 3.0 * 6.0) / 2.0;
        for row in [[0usize, 1], [2, 3]] {
            ui.horizontal(|ui| {
                for i in row {
                    let (label, key) = [("X", "x"), ("Y", "y"), ("W", "w"), ("H", "h")][i];
                    ui.add_sized([14.0, 20.0], egui::Label::new(RichText::new(label).color(if sel > 0 { theme::TEXT } else { theme::TEXT_DISABLED })));
                    let id = ui.id().with(key);
                    let r = ui.push_id(id, |ui| theme::unit_field(ui, &mut sh.edit[i], uname, fw)).inner;
                    if r.lost_focus() && ui.input(|inp| inp.key_pressed(egui::Key::Enter)) {
                        commit = Some(i);
                    }
                }
            });
        }
    });
    if let Some(i) = commit {
        canvas.set_selection_field(["x", "y", "w", "h"][i], &sh.edit[i].clone(), sh.lock_aspect); // a variable binds, a value unbinds (params.rs)
        sh.edit_for = None;
    }
    // selected text: wording / font at a glance, edit or convert (double-click on the canvas edits too)
    let t = canvas.selected_text();
    if let Some(ti) = canvas.doc.texts.iter().find(|x| x.id == t) {
        ui.horizontal(|ui| {
            icons::paint(ui, "text", egui::Rect::from_min_size(ui.cursor().min + egui::vec2(0.0, 2.0), egui::vec2(16.0, 16.0)), 16.0, theme::TEXT.gamma_multiply(0.7));
            ui.add_space(20.0);
            let style = format!("{}{}", if ti.bold { " Bold" } else { "" }, if ti.italic { " Italic" } else { "" });
            let line = format!("{}   {}{style} · {}", ti.text.replace('\n', " "), ti.family, crate::units::fmt(&canvas.doc, ti.height));
            let w = (ui.available_width() - 56.0).max(40.0);
            ui.allocate_ui(egui::vec2(w, 20.0), |ui| ui.add(egui::Label::new(RichText::new(line).size(12.0)).truncate()));
            if IconButton::new("pen").size(15.0).padding(3.0).tip("Convert text to curves (plain vectors)").show(ui).clicked() {
                sh.push("text:curves");
            }
            if IconButton::new("edit").size(15.0).padding(3.0).tip("Edit the text (wording, font, size)").show(ui).clicked() {
                sh.push("text:edit");
            }
        });
    }
    if sel > 0 {
        theme::checkbox(ui, &mut sh.lock_aspect, "Keep proportions").tip("Changing W also scales H (and the reverse)");
    }
    // exact rotation around the pivot (the frame's crosshair: drag it to turn around another point)
    if sel > 0 {
        ui.horizontal(|ui| {
            ui.add_sized([44.0, 20.0], egui::Label::new(RichText::new("Rotate").color(if sel > 0 { theme::TEXT } else { theme::TEXT_DISABLED })));
            let r = theme::unit_field(ui, &mut sh.rot_text, "°", 70.0).tip("Angle in degrees; Enter turns counter-clockwise. Drag the round handle above the selection to rotate by hand (Shift = 15° steps).");
            let deg = sh.rot_text.trim().trim_end_matches('°').trim().parse::<f64>().ok().filter(|d| d.is_finite());
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ccw = ui.add_enabled(deg.is_some(), egui::Button::new("↺")).tip("Rotate counter-clockwise by this angle around the pivot").clicked();
            let cw = ui.add_enabled(deg.is_some(), egui::Button::new("↻")).tip("Rotate clockwise by this angle around the pivot").clicked();
            if let Some(d) = deg {
                if enter || ccw {
                    canvas.rotate_selection(d);
                } else if cw {
                    canvas.rotate_selection(-d);
                }
            }
        });
    }
    // ARRANGE / BOOLEAN command grids (CommandGroup.qml)
    let ops = canvas.doc.ops.len();
    let (cu, cr) = (canvas.doc.can_undo(), canvas.doc.can_redo());
    for (label, ids, color) in [("ARRANGE", &shell::ARRANGE, theme::GROUP_ARRANGE), ("BOOLEAN", &shell::BOOLEAN, theme::GROUP_BOOLEAN)] {
        let tiles: Vec<Tile> = ids.iter().map(|id| {
            let (icon, text, tip) = shell::def(id);
            Tile { id: id.to_string(), icon, text, tip: tip.to_string(), tint: color, on: shell::enabled(id, sel, ops, cu, cr), why: "" }
        }).collect();
        if !tiles.iter().any(|t| t.on) {
            continue; // a grid of greyed-out commands says nothing: shown again once something is selected
        }
        ui.add_space(6.0);
        section(ui, label, color);
        tile_grid(ui, sh, &tiles);
    }
    ui.add_space(6.0);
    theme::hline(ui);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Board").family(crate::widgets::bold_family()).color(Color32::WHITE));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if IconButton::new("edit").tip("Board / job setup").show(ui).clicked() {
                sh.push("board");
            }
        });
    });
    ui.add(egui::Label::new(RichText::new(shell::board_summary(&canvas.doc)).size(12.0)).wrap());
    let binds = board_bindings(&canvas.doc);
    if !binds.is_empty() {
        ui.add(egui::Label::new(RichText::new(binds).size(11.0).italics().color(theme::ACCENT)).wrap());
    }
    ui.add(egui::Label::new(RichText::new(shell::feeds_summary(&canvas.doc)).size(11.0).color(theme::TEXT.gamma_multiply(0.65))).wrap());
}

/// Nothing selected: what to do next for this job, with the buttons for it
fn next_step(ui: &mut egui::Ui, sh: &mut Shell, canvas: &Canvas) {
    let doc = &canvas.doc;
    let (text, buttons): (&str, &[(&str, &str)]) = if doc.shapes.is_empty() {
        ("Draw with the tools on the left, or open a VCarve project (.crv) or an SVG, DXF, PDF, image or G-code file.", &[("Import…", "import"), ("Board…", "board")])
    } else if doc.ops.is_empty() {
        ("Click shapes to select them (drag a box for several), then add a toolpath.", &[("Select all", "selectall"), ("Add toolpath", "tab:toolpaths")])
    } else {
        ("Toolpaths are set: watch the cut, then export the G-code.", &[("Simulate", "mode:Simulate"), ("Export…", "export")])
    };
    egui::Frame::new().fill(theme::FIELD).corner_radius(6).inner_margin(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Next").size(10.0).family(crate::widgets::bold_family()).color(theme::ACCENT));
        ui.add(egui::Label::new(RichText::new(text).size(12.0)).wrap());
        ui.horizontal(|ui| {
            for (label, cmd) in buttons {
                if ui.button(*label).clicked() {
                    sh.push(*cmd);
                }
            }
        });
    });
}

/// CommandGroup.qml section label: small bold caps in the group colour
fn section(ui: &mut egui::Ui, label: &str, color: Color32) {
    ui.label(RichText::new(label).size(10.0).family(crate::widgets::bold_family()).color(color.gamma_multiply(0.85)));
}

/// One tile of a command grid: command id, icon, label, tooltip, colour, enabled, tooltip while disabled
struct Tile {
    id: String,
    icon: &'static str,
    text: &'static str,
    tip: String,
    tint: Color32,
    on: bool,
    why: &'static str,
}

/// CommandGroup.qml Flow: labelled icon tiles, as many per row as fit (4 at 310 px), 2 px apart
fn tile_grid(ui: &mut egui::Ui, sh: &mut Shell, tiles: &[Tile]) {
    let per_row = ((ui.available_width() + 2.0) / 60.0).max(1.0) as usize;
    ui.spacing_mut().item_spacing.y = 2.0;
    for row in tiles.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
            for t in row {
                let r = ui.add_enabled_ui(t.on, |ui| IconButton::new(t.icon).text(t.text).tip(&t.tip).label_below().size(20.0).font(10.0).tint(t.tint).width(58.0).show(ui)).inner;
                let r = if t.why.is_empty() { r } else { r.disabled_tip(t.why) };
                if r.clicked() {
                    sh.push(t.id.as_str());
                }
            }
        });
    }
    ui.spacing_mut().item_spacing.y = 6.0;
}

fn layers(ui: &mut egui::Ui, sh: &mut Shell, canvas: &mut Canvas) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Layers").family(crate::widgets::bold_family()).color(Color32::WHITE));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled_ui(canvas.doc.layers.len() > 1, |ui| IconButton::new("remove").tip("Remove active layer").show(ui)).inner.clicked() {
                sh.push("layer:remove");
            }
            if IconButton::new("add").tip("Add layer").show(ui).clicked() {
                sh.push("layer:add");
            }
        });
    });
    let active = canvas.doc.active_layer;
    let counts: Vec<usize> = (0..canvas.doc.layers.len()).map(|i| canvas.doc.shapes.iter().filter(|s| s.layer as usize == i).count()).collect();
    enum Act {
        Active(usize),
        Select(usize),
        Rename(usize, String),
        Color(usize, String, bool),
        Visible(usize),
        Locked(usize),
        KeepOut(usize),
    }
    let mut acts = Vec::new();
    let w = ui.available_width();
    ui.spacing_mut().item_spacing.y = 0.0;
    for (i, l) in canvas.doc.layers.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 28.0), egui::Sense::click());
        let on = active == i as i32;
        let fill = if on { theme::ACCENT } else if resp.hovered() { theme::ROW_HOVER } else { Color32::TRANSPARENT };
        ui.painter().rect_filled(rect, 4.0, fill);
        if resp.double_clicked() {
            acts.push(Act::Select(i)); // Qt selectLayerShapes
        } else if resp.clicked() {
            acts.push(Act::Active(i));
        }
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(6.0, 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        row.spacing_mut().item_spacing.x = 4.0;
        // colour swatch = picker; ponytail: the press that starts a pick records the undo step, drags inside the
        // picker recolour live without more steps
        let mut col = hex(&l.color);
        row.spacing_mut().interact_size = egui::vec2(14.0, 14.0);
        if egui::color_picker::color_edit_button_srgba(&mut row, &mut col, egui::color_picker::Alpha::Opaque).tip("Layer colour").changed() {
            acts.push(Act::Color(i, format!("#{:02x}{:02x}{:02x}", col.r(), col.g(), col.b()), row.input(|inp| inp.pointer.any_pressed())));
        }
        // name: edited in a buffer, renamed (one undo step) when the field loses focus
        let id = row.id().with(("layer-name", i));
        let mut name = row.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| l.name.clone());
        let r = row.add(egui::TextEdit::singleline(&mut name).id(id.with("edit")).frame(egui::Frame::NONE).font(theme::font(12.0)).desired_width(w - 190.0).text_color(if on { Color32::WHITE } else { theme::TEXT }));
        if r.gained_focus() {
            acts.push(Act::Active(i));
        }
        if r.has_focus() {
            row.data_mut(|d| d.insert_temp(id, name));
        } else {
            if r.lost_focus() {
                acts.push(Act::Rename(i, name));
            }
            row.data_mut(|d| d.remove::<String>(id));
        }
        row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |row| {
            if IconButton::new(if l.locked { "lock" } else { "unlock" }).size(15.0).padding(3.0).tip("Lock / unlock").show(row).clicked() {
                acts.push(Act::Locked(i));
            }
            if IconButton::new(if l.visible { "eye" } else { "eye-off" }).size(15.0).padding(3.0).tip("Show / hide").show(row).clicked() {
                acts.push(Act::Visible(i));
            }
            // a toolpath for the whole layer (new shapes on it join), and select everything on it
            let add = IconButton::new("add").size(15.0).padding(3.0).tip("Add a toolpath for everything on this layer").show(row);
            egui::Popup::menu(&add).show(|ui| {
                let mut ko = l.keep_out;
                if crate::widgets::menu_check(ui, "Keep out (clamps, screws)", "", &mut ko).on_hover_text("Shapes on this layer mark hold-downs: export warns when a toolpath comes down near one").clicked() {
                    acts.push(Act::KeepOut(i));
                    ui.close();
                }
                crate::widgets::menu_separator(ui);
                ui.label(RichText::new("Toolpath for the whole layer").small().color(theme::TEXT_DIM));
                for (k, kind) in KIND_TYPES.iter().enumerate() {
                    if *kind == OpType::Relief {
                        continue;
                    }
                    if crate::widgets::menu_item(ui, shell::KINDS[k].0, "").clicked() {
                        sh.push(format!("layerop:{i}:{k}"));
                        ui.close();
                    }
                }
            });
            if IconButton::new("select").size(15.0).padding(3.0).tip("Select every vector on this layer (or double-click the row)").show(row).clicked() {
                sh.push(format!("layersel:{i}"));
            }
            if l.keep_out {
                row.label(RichText::new("keep out").size(11.0).color(theme::DANGER)).tip("Clamps / screws: export warns when a toolpath comes down near these shapes");
            }
            let tps = canvas.doc.ops.iter().filter(|o| o.layer_scope == i as i32).count();
            if tps > 0 {
                row.label(RichText::new(format!("{tps} tp")).size(11.0).color(theme::MODE_TOOLPATHS)).tip(format!("{tps} toolpath(s) cover this whole layer"));
            }
            row.label(RichText::new(counts[i].to_string()).size(11.0).color(theme::TEXT.gamma_multiply(0.5)));
        });
    }
    ui.spacing_mut().item_spacing.y = 8.0;
    if canvas.selection_count() > 0 && canvas.doc.layers.len() > 1 {
        ui.add_space(6.0);
        if ui.button("Move selection to active layer").clicked() {
            canvas.move_selection_to_layer(canvas.doc.active_layer);
        }
    }
    for a in acts {
        match a {
            Act::Active(i) => canvas.doc.active_layer = i as i32,
            Act::Select(i) => canvas.select_layer_shapes(i as i32),
            Act::Rename(i, n) => canvas.rename_layer(i, &n),
            Act::Color(i, c, undo) => canvas.set_layer_color(i, &c, undo),
            Act::Visible(i) => {
                canvas.doc.layers[i].visible ^= true; // view state: not an undo step (Qt)
                canvas.mutated();
            }
            Act::Locked(i) => {
                canvas.doc.layers[i].locked ^= true;
                canvas.mutated();
            }
            Act::KeepOut(i) => {
                canvas.doc.checkpoint(); // changes what export checks: an undo step
                canvas.doc.layers[i].keep_out ^= true;
                canvas.mutated();
            }
        }
    }
}

fn toolpaths(ui: &mut egui::Ui, sh: &mut Shell, canvas: &mut Canvas, gen: Option<&Generated>) {
    let w = ui.available_width();
    let doc = &canvas.doc;
    let mut acts = Vec::new();
    // NEW TOOLPATH / SETUP tiles (Main.qml, same CommandGroup grid as ARRANGE / BOOLEAN; was the Toolpaths-mode rail)
    section(ui, "NEW TOOLPATH", theme::MODE_TOOLPATHS);
    let kinds: Vec<Tile> = shell::KINDS.iter().take(KIND_TYPES.len()).enumerate().map(|(i, (name, icon, tip))| {
        let relief = i == 7;
        let on = if relief { !doc.reliefs.is_empty() } else { !doc.shapes.is_empty() }; // nothing selected: the dialog asks for shapes
        Tile { id: format!("kind:{i}"), icon, text: name, tip: tip.to_string(), tint: kind_color(KIND_TYPES[i]), on, why: if relief { "Import a 3D model first" } else { "Draw or import shapes first" } }
    }).collect();
    tile_grid(ui, sh, &kinds);
    ui.add_space(6.0);
    section(ui, "SETUP", theme::GROUP_SETUP);
    let setup: Vec<Tile> = ["tools", "machine", "posts", "board"].iter().map(|id| {
        let (icon, text, tip) = shell::def(id);
        Tile { id: id.to_string(), icon, text, tip: tip.to_string(), tint: theme::GROUP_SETUP, on: true, why: "" }
    }).collect();
    tile_grid(ui, sh, &setup);
    ui.add_space(6.0);
    section(ui, "TOOLPATHS", theme::TEXT_DIM);
    if doc.ops.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.add(egui::Label::new(RichText::new("Select shapes, then pick a toolpath type above.").color(theme::TEXT.gamma_multiply(0.6))).wrap());
        });
    }
    // Group: none / by bit (= one file per bit at export, with the tool-changer number) / by the user's operations
    ui.horizontal(|ui| {
        ui.label(RichText::new("Group").size(11.0).color(theme::TEXT_DIM));
        for (g, name) in ["None", "Bit", "Operation"].into_iter().enumerate() {
            if ui.selectable_label(sh.tp_group == g as u8, name).clicked() {
                sh.tp_group = g as u8;
            }
        }
        if sh.tp_group == 1 && !doc.ops.is_empty() && ui.small_button("Number bits").tip("Give each bit here without a tool-changer number the lowest free one (T1, T2…)").clicked() {
            acts.push(TpAct::AutoSlots);
        }
    });
    let groups: Vec<(String, Vec<usize>)> = match sh.tp_group {
        1 => group_ops(doc, |o| o.tool.to_string()),
        2 => group_ops(doc, |o| o.op_group.clone()),
        _ => vec![(String::new(), (0..doc.ops.len()).collect())],
    };
    let names: Vec<String> = group_ops(doc, |o| o.op_group.clone()).into_iter().map(|(k, _)| k).filter(|k| !k.is_empty()).collect();
    ui.spacing_mut().item_spacing.y = 0.0;
    for (key, ops) in &groups {
        if sh.tp_group == 1 {
            let t = key.parse::<usize>().unwrap_or(0);
            if let Some(tool) = doc.tools.get(t) {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&tool.name).size(11.0).family(crate::widgets::bold_family()).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut v = tool.slot;
                        let r = ui.add(egui::DragValue::new(&mut v).range(0..=99).prefix("T").custom_formatter(|n, _| if n < 1.0 { "–".into() } else { format!("{n:.0}") }))
                            .tip("Tool-changer number for this bit (T in the G-code); – = numbered in cutting order");
                        if v != tool.slot {
                            acts.push(TpAct::Slot(t, v, r.drag_started() || !r.dragged()));
                        }
                    });
                });
            }
        } else if sh.tp_group == 2 {
            ui.add_space(6.0);
            let r = ui.label(RichText::new(if key.is_empty() { "No operation" } else { key }).size(11.0).family(crate::widgets::bold_family()).color(theme::TEXT));
            if !key.is_empty() {
                r.tip("Right-click to rename").context_menu(|ui| {
                    let id = ui.id().with(("rename-op-group", key));
                    let mut name = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| key.clone());
                    ui.label("Rename operation");
                    let e = ui.text_edit_singleline(&mut name);
                    if e.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !name.trim().is_empty() {
                        acts.push(TpAct::Rename(key.clone(), name.trim().into()));
                        ui.close();
                    }
                    ui.data_mut(|d| d.insert_temp(id, name));
                });
            }
        }
        for &i in ops {
            op_row(ui, sh, doc, i, w, &names, &mut acts, gen.and_then(|g| g.minutes.get(i)).copied().unwrap_or(0.0));
        }
    }
    for a in acts {
        match a {
            TpAct::Toggle(i) => {
                canvas.doc.ops[i].visible = !canvas.doc.ops[i].visible;
                canvas.rev += 1; // preview only; the document itself is unchanged
            }
            TpAct::Edit(i) => sh.push(format!("opedit:{i}")),
            TpAct::Move(i, d) => {
                let j = i as i64 + d as i64;
                if j >= 0 && (j as usize) < canvas.doc.ops.len() {
                    canvas.doc.checkpoint();
                    canvas.doc.ops.swap(i, j as usize);
                    sh.op_sel = Some(j as usize);
                    canvas.touch();
                }
            }
            TpAct::Duplicate(i) => {
                canvas.doc.checkpoint();
                let mut op = canvas.doc.ops[i].clone();
                op.name += " copy";
                canvas.doc.ops.insert(i + 1, op);
                sh.op_sel = Some(i + 1);
                canvas.touch();
            }
            TpAct::Group(i, g) => {
                canvas.doc.checkpoint();
                canvas.doc.ops[i].op_group = g;
                canvas.touch();
            }
            TpAct::Rename(old, new) => {
                canvas.doc.checkpoint();
                canvas.doc.ops.iter_mut().filter(|o| o.op_group == old).for_each(|o| o.op_group = new.clone());
                canvas.touch();
            }
            TpAct::Slot(t, v, undo) => {
                if undo {
                    canvas.doc.checkpoint();
                }
                canvas.doc.tools[t].slot = v;
                canvas.touch();
            }
            TpAct::AutoSlots => {
                canvas.doc.checkpoint();
                let used: Vec<usize> = group_ops(&canvas.doc, |o| o.tool.to_string()).iter().filter_map(|(k, _)| k.parse().ok()).collect();
                for t in used {
                    if canvas.doc.tools.get(t).is_some_and(|x| x.slot == 0) {
                        let taken: Vec<i32> = canvas.doc.tools.iter().map(|x| x.slot).collect();
                        canvas.doc.tools[t].slot = (1..).find(|s| !taken.contains(s)).unwrap_or(1);
                    }
                }
                canvas.touch();
            }
        }
    }
}

enum TpAct {
    Toggle(usize),
    Edit(usize),
    Group(usize, String),
    /// op index, -1 up / +1 down in the cutting order
    Move(usize, i32),
    Duplicate(usize),
    Rename(String, String),
    /// tool index, new slot, start of an edit (= undo step)
    Slot(usize, i32, bool),
    AutoSlots,
}

/// Toolpath indices grouped by `key`, groups in order of first use
fn group_ops(doc: &gcarve_core::document::Document, key: impl Fn(&gcarve_core::document::Op) -> String) -> Vec<(String, Vec<usize>)> {
    let mut out: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, op) in doc.ops.iter().enumerate() {
        let k = key(op);
        match out.iter_mut().find(|(g, _)| *g == k) {
            Some((_, v)) => v.push(i),
            None => out.push((k, vec![i])),
        }
    }
    out
}

/// One toolpath row: kind icon, name · side, bit · summary, eye; right-click = put it in an operation
#[allow(clippy::too_many_arguments)]
fn op_row(ui: &mut egui::Ui, sh: &mut Shell, doc: &gcarve_core::document::Document, i: usize, w: f32, groups: &[String], acts: &mut Vec<TpAct>, minutes: f64) {
    let op = &doc.ops[i];
    let k = shell::kind_index(op.kind);
    let col = kind_color(op.kind);
    let (side, info) = shell::op_summary(doc, op);
    let tool = doc.tools.get(op.tool as usize).map(|t| t.name.as_str()).unwrap_or("?");
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 36.0), egui::Sense::click());
    let on = sh.op_sel == Some(i);
    let fill = if on { theme::ACCENT } else if resp.hovered() { theme::ROW_HOVER } else { Color32::TRANSPARENT };
    let p = ui.painter();
    p.rect_filled(rect, 4.0, fill);
    p.rect_filled(egui::Rect::from_min_max(rect.left_top() + egui::vec2(3.0, 3.0), egui::pos2(rect.left() + 6.0, rect.bottom() - 3.0)), 1.5, col);
    let alpha = if op.visible { 1.0 } else { 0.55 };
    let icon_col = if on { Color32::WHITE } else { col };
    icons::paint(ui, shell::KINDS[k].1, egui::Rect::from_min_size(rect.left_top() + egui::vec2(12.0, 10.0), egui::vec2(16.0, 16.0)), 16.0, icon_col.gamma_multiply(alpha));
    let tc = if on { Color32::WHITE } else { theme::TEXT }.gamma_multiply(alpha);
    let tx = rect.left() + 34.0;
    let p = ui.painter_at(egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right() - 30.0, rect.bottom())));
    p.text(egui::pos2(tx, rect.top() + 4.0), egui::Align2::LEFT_TOP, format!("{}  ·  {side}", op.name), theme::font(12.0), tc);
    if minutes > 0.0 {
        // rough cutting time, right-aligned on the name line
        let t = if minutes < 1.0 { "<1 min".to_string() } else if minutes < 90.0 { format!("{minutes:.0} min") } else { format!("{:.1} h", minutes / 60.0) };
        let g = p.layout_no_wrap(t, theme::font(11.0), tc.gamma_multiply(0.6));
        let at = egui::pos2(rect.right() - 34.0 - g.size().x, rect.top() + 5.0);
        p.rect_filled(egui::Rect::from_min_size(at - egui::vec2(6.0, 1.0), g.size() + egui::vec2(6.0, 2.0)), 0.0, if on { theme::ACCENT } else if resp.hovered() { theme::ROW_HOVER } else { theme::PANEL });
        p.galley(at, g, tc);
    }
    p.text(egui::pos2(tx, rect.top() + 19.0), egui::Align2::LEFT_TOP, format!("{tool}  ·  {info}"), theme::font(11.0), tc.gamma_multiply(0.75));
    let eye = egui::Rect::from_center_size(egui::pos2(rect.right() - 14.0, rect.center().y), egui::vec2(21.0, 21.0));
    let mut e = ui.new_child(egui::UiBuilder::new().max_rect(eye).layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight))); // new_child: `put` would advance the row cursor
    let er = IconButton::new(if op.visible { "eye" } else { "eye-off" }).size(15.0).padding(3.0).tip(if op.visible { "Hide this toolpath's preview" } else { "Show this toolpath's preview" }).show(&mut e);
    if er.clicked() {
        acts.push(TpAct::Toggle(i));
    } else if resp.double_clicked() {
        acts.push(TpAct::Edit(i));
    } else if resp.clicked() {
        sh.op_sel = Some(i);
        sh.push(format!("opselect:{i}")); // every shape of the toolpath (layer / group scope too), Qt selectOpShapes
    }
    resp.context_menu(|ui| {
        // the list order is the cutting order (per bit): pockets and carving before the cut-out
        for (label, act) in [("Edit…", TpAct::Edit(i)), ("Duplicate", TpAct::Duplicate(i)), ("Cut Earlier (move up)", TpAct::Move(i, -1)), ("Cut Later (move down)", TpAct::Move(i, 1))] {
            if crate::widgets::menu_item(ui, label, "").clicked() {
                acts.push(act);
                ui.close();
            }
        }
        crate::widgets::menu_separator(ui);
        ui.label(RichText::new("Operation").small().color(theme::TEXT_DIM));
        for g in groups.iter().map(String::as_str).chain([""]) {
            let label = if g.is_empty() { "None" } else { g };
            if ui.radio(op.op_group == g, label).clicked() {
                acts.push(TpAct::Group(i, g.into()));
                ui.close();
            }
        }
        let id = ui.id().with("new-op-group");
        let mut name = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
        let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text("New operation…").desired_width(160.0));
        if r.lost_focus() && ui.input(|inp| inp.key_pressed(egui::Key::Enter)) && !name.trim().is_empty() {
            acts.push(TpAct::Group(i, name.trim().into()));
            name.clear();
            ui.close();
        }
        ui.data_mut(|d| d.insert_temp(id, name));
    });
}

/// Board fields driven by variables (Qt Canvas::boardBindings): "W = board-w  ·  Thickness = t2"
pub fn board_bindings(doc: &gcarve_core::document::Document) -> String {
    [("w", "W"), ("h", "H"), ("t", "Thickness"), ("safeZ", "Safe Z"), ("retractZ", "Retract Z")]
        .iter()
        .filter_map(|(k, label)| doc.material.bind.get(*k).map(|e| format!("{label} = {e}")))
        .collect::<Vec<_>>()
        .join("  ·  ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn board_bindings_line() {
        let mut d = gcarve_core::document::Document::default();
        assert_eq!(super::board_bindings(&d), "");
        d.material.bind.insert("t".into(), "ply".into());
        d.material.bind.insert("w".into(), "2 * side".into());
        assert_eq!(super::board_bindings(&d), "W = 2 * side  ·  Thickness = ply");
    }
}
