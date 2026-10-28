//! The window chrome of the Qt "modes" layout: menu bar, ModeBar, ModeRail, SheetTabs, status bar, the "Start a job"
//! card and the mode bar. Widgets push command ids (the Qt app's tbRun / railActivate ids) into `Shell::cmds`;
//! `App::run` dispatches them, so every piece here is layout only.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::canvas::Canvas;
use crate::widgets::TipExt;
use crate::icons::IconButton;
use crate::widgets;
use crate::theme;
use crate::units;
use eframe::egui::{self, Color32, RichText};
use gcarve_core::document::{Document, Op, OpType};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Design,
    Simulate,
}

impl Mode {
    pub fn color(self) -> Color32 {
        match self {
            Mode::Design => theme::MODE_DESIGN,
            Mode::Simulate => theme::MODE_SIMULATE,
        }
    }
}

const MODES: [(Mode, &str, &str, &str, &str); 2] = [ // i18n
    (Mode::Design, "pen", "Design", "1", "Draw, arrange vectors and add toolpaths (1; 2 = Toolpaths tab)"),
    (Mode::Simulate, "simulate", "Simulate", "3", "Carve the board and watch the cut (3)"),
];

/// Theme.kinds: name, icon, tip per OpType (index = OpType as usize)
pub const KINDS: [(&str, &str, &str); 13] = [ // i18n
    ("Profile", "profile", "Cut around / inside the selected shapes"),
    ("Pocket", "pocket", "Clear the inside of the selected shapes"),
    ("Drill", "drill", "Drill at the centre of each shape"),
    ("V-Carve", "vcarve", "V-carve the selected shapes"),
    ("Bore", "bore", "Helical bore: holes larger than the bit"),
    ("Inlay", "inlay", "V-carve inlay: female pocket + mirrored male plug"),
    ("Flute", "flute", "Flute along the selected vectors with ramps"),
    ("3D", "relief", "Carve an imported 3D model (import one first)"),
    ("Chamfer", "chamfer", "Chamfer the edges of the selected vectors with a V-bit"),
    ("Prism", "prism", "Prism carve: raised, bevelled shapes with a V-bit"),
    ("Texture", "texture", "Texture: random carved strokes filling the selected shapes (sign backgrounds)"),
    ("Photo", "photo", "Photo V-Carve: carve a picture with a V-bit, darker = deeper"),
    ("Imported", "open", "Moves read from a G-code file (File > Import G-code)"),
];

pub fn kind_index(k: OpType) -> usize {
    match k {
        OpType::Profile => 0,
        OpType::Pocket => 1,
        OpType::Drill => 2,
        OpType::VCarve => 3,
        OpType::Bore => 4,
        OpType::Inlay => 5,
        OpType::Flute => 6,
        OpType::Relief => 7,
        OpType::Chamfer => 8,
        OpType::Prism => 9,
        OpType::Texture => 10,
        OpType::Photo => 11,
        OpType::Imported => 12,
    }
}

/// Main.qml tbDefs: id → (icon, text, tip)
pub fn def(id: &str) -> (&'static str, &'static str, &'static str) {
    match id {
        "new" => ("new", i18n::t("New"), i18n::t("New project (⌘N)")),
        "open" => ("open", i18n::t("Open"), i18n::t("Open project or import SVG/DXF (⌘O)")),
        "jobs" => ("jobs", i18n::t("Jobs"), i18n::t("Job organizer: folders of projects with their G-code and revisions (⌘J)")),
        "save" => ("save", i18n::t("Save"), i18n::t("Save project (⌘S)")),
        "undo" => ("undo", i18n::t("Undo"), i18n::t("Undo (⌘Z)")),
        "redo" => ("redo", i18n::t("Redo"), i18n::t("Redo (⇧⌘Z)")),
        "copy" => ("copy", i18n::t("Copy"), i18n::t("Copy vectors with their toolpaths and bits (⌘C)")),
        "paste" => ("paste", i18n::t("Paste"), i18n::t("Paste (⌘V)")),
        "clone" => ("duplicate", i18n::t("Clone"), i18n::t("Duplicate with toolpaths (⌘D)")),
        "group" => ("group", i18n::t("Group"), i18n::t("Group selected vectors so they select and move together (⌘G); Ungroup (⇧⌘G)")),
        "delete" => ("delete", i18n::t("Delete"), i18n::t("Delete selection")),
        "fliph" => ("flip-h", i18n::t("Flip H"), i18n::t("Flip horizontal")),
        "flipv" => ("flip-v", i18n::t("Flip V"), i18n::t("Flip vertical")),
        "rotate" => ("rotate", i18n::t("Rotate"), i18n::t("Rotate 90°")),
        "align" => ("align", i18n::t("Align"), i18n::t("Align and distribute: to each other or to the board")),
        "array" => ("array", i18n::t("Array"), i18n::t("Array copy…")),
        "nest" => ("nest", i18n::t("Nest"), i18n::t("Pack the selected parts onto the board")),
        "link" => ("link", i18n::t("Link"), i18n::t("Keep two shapes positioned relative to each other (select exactly two)")),
        "weld" => ("weld", i18n::t("Weld"), i18n::t("Weld: merge overlapping shapes")),
        "subtract" => ("subtract", i18n::t("Minus"), i18n::t("Subtract: cut the smaller shapes out of the largest")),
        "intersect" => ("intersect", i18n::t("Overlap"), i18n::t("Intersect: keep only the overlap")),
        "offset" => ("offset", i18n::t("Offset"), i18n::t("Offset vectors…")),
        "dogbone" => ("offset", i18n::t("Corners"), i18n::t("Corners: dogbone / T-bone so square parts fit a round bit, or fillet (round) them…")),
        "join" => ("join", i18n::t("Join"), i18n::t("Join open vectors whose ends meet")),
        "close" => ("close", i18n::t("Close"), i18n::t("Close open vectors")),
        "paths" => ("eye", i18n::t("Paths"), i18n::t("Show / hide the toolpath previews so you can edit vectors (⇧⌘T)")),
        "width" => ("offset", i18n::t("Width"), i18n::t("Show / hide the cut-width bands")),
        "sim" => ("simulate", i18n::t("Carve"), i18n::t("Carve the board with all toolpaths (⌘R)")),
        "watch" => ("simulate", i18n::t("Watch"), i18n::t("Watch the cut at machine speed (⇧⌘R)")),
        "vars" => ("variable", i18n::t("Vars"), i18n::t("Variables (⇧⌘E): named values you can use in any size field")),
        "tools" => ("layers", i18n::t("Bits"), i18n::t("Tool library")),
        "posts" => ("export", i18n::t("Posts"), i18n::t("Post-processors")),
        "board" => ("board", i18n::t("Board"), i18n::t("Board / job setup")),
        "machine" => ("machine", i18n::t("Machine"), i18n::t("Choose your CNC machine: its limits keep feeds safe, its controller sets the post")),
        "export" => ("export", i18n::t("Export"), i18n::t("Export G-code (⌘E)")),
        "opedit" => ("edit", i18n::t("Edit"), i18n::t("Edit the selected toolpath")),
        "opdelete" => ("delete", i18n::t("Delete"), i18n::t("Delete the selected toolpath")),
        "view3d" => ("board", "3D", i18n::t("Orbit the carved board (⌘3)")),
        "import" => ("open", i18n::t("Import"), i18n::t("Import SVG / DXF / PDF / AI / STL into this document")),
        "edittext" => ("text", i18n::t("Edit Text"), i18n::t("Change the wording, font or size of the selected text (double-click it on the canvas)")),
        "tocurves" => ("nodes", i18n::t("To Curves"), i18n::t("Convert the selected text to plain vectors")),
        "restart" => ("undo", i18n::t("Restart"), i18n::t("Back to the start of the simulation")),
        _ => ("", "", ""),
    }
}


pub const ARRANGE: [&str; 7] = ["fliph", "flipv", "rotate", "align", "array", "nest", "link"];
pub const BOOLEAN: [&str; 7] = ["weld", "subtract", "intersect", "offset", "dogbone", "join", "close"];

/// Main.qml tbEnabled
pub fn enabled(id: &str, sel: usize, ops: usize, can_undo: bool, can_redo: bool) -> bool {
    match id {
        "undo" => can_undo,
        "redo" => can_redo,
        "copy" | "clone" | "delete" | "fliph" | "flipv" | "rotate" | "align" | "array" | "nest" | "offset" | "dogbone" | "close" => sel > 0,
        "group" | "weld" | "subtract" | "intersect" | "join" => sel > 1,
        "link" => sel == 2,
        "sim" | "watch" | "export" | "opedit" | "opdelete" => ops > 0,
        _ => true,
    }
}

/// Design-mode tools: id, icon, name, key
const TOOLS: [(&str, &str, &str, &str); 13] = [ // i18n
    ("tool:select", "select", "Select", "V"),
    ("tool:nodes", "nodes", "Nodes", "N"),
    ("tool:rect", "rect", "Rect", "R"),
    ("tool:ellipse", "ellipse", "Ellipse", "E"),
    ("tool:polygon", "polygon", "Polygon", "G"),
    ("tool:star", "star", "Star", "S"),
    ("tool:polyline", "polyline", "Polyline", "P"),
    ("tool:curve", "pen", "Curve", "B"),
    ("tool:arc", "arc", "Arc", "A"),
    ("tool:freehand", "edit", "Freehand", "H"),
    ("tool:measure", "measure", "Measure", "M"),
    ("tool:dimension", "dimension", "Dimension", "D"),
    ("tool:text", "text", "Text", "T"),
];

fn tool_color(id: &str) -> (Color32, Color32) {
    use theme::ToolKind as K;
    theme::tool_color(match id {
        "tool:select" | "tool:nodes" => K::Select,
        "tool:measure" | "tool:dimension" | "tool:angle" => K::Measure,
        "tool:text" => K::Text,
        _ => K::Draw,
    })
}

/// project unit from Document::bare_unit_in: (name, multiplier from inches, decimals)
pub fn unit(doc: &Document) -> (&'static str, f64, usize) {
    (units::name(doc), 1.0 / doc.bare_unit_in, units::decimals(doc))
}

/// Canvas::boardSummary
pub fn board_summary(doc: &Document) -> String {
    let m = &doc.material;
    let (name, u, dt) = unit(doc);
    let d = if name == "mm" { 1 } else { 2 };
    let origins = ["the bottom-left corner", i18n::t("the bottom-right corner"), i18n::t("the top-left corner"), i18n::t("the top-right corner"), "the centre"];
    let sheet = if doc.sheets.len() > 1 { format!("{} ({} of {})  ·  ", doc.sheet_name(doc.active_sheet.max(0) as usize), doc.active_sheet + 1, doc.sheets.len()) } else { String::new() };
    format!(
        "{sheet}{}  ·  {:.d$} × {:.d$} × {:.dt$} {name}  ·  zero at {}, {}",
        m.kind,
        m.w * u,
        m.h * u,
        m.t * u,
        origins[(m.origin.max(0) as usize).min(4)],
        if m.z_bottom { i18n::t("bit touched off on the table") } else { i18n::t("bit touched off on top") }
    ) + &m.machine.as_ref().map(|x| format!("  ·  {}", x.name)).unwrap_or_default()
}

/// Canvas::feedsSummary
pub fn feeds_summary(doc: &Document) -> String {
    if doc.tools.is_empty() {
        return String::new();
    }
    let mat = &doc.material.kind;
    let n = doc.tools.iter().filter(|t| t.presets.iter().any(|p| p.material.eq_ignore_ascii_case(mat))).count();
    let rest = if doc.material.auto_feeds { i18n::t("suggested feeds") } else { i18n::t("their default feeds") };
    if n == 0 {
        format!("No feed presets for {mat}: bits use {rest}")
    } else if n == doc.tools.len() {
        format!("All {n} bits have feeds for {mat}")
    } else {
        format!("{n} of {} bits have feeds for {mat}, the rest use {rest}", doc.tools.len())
    }
}

/// Canvas::opList side + info strings for a toolpath row
pub fn op_summary(doc: &Document, op: &Op) -> (String, String) {
    let (name, u, dec) = unit(doc);
    let num = |v: f64| format!("{:.dec$}", v * u);
    let side = match op.kind {
        OpType::Profile => match op.side {
            gcarve_core::document::Side::Outside => i18n::t("Outside"),
            gcarve_core::document::Side::Inside => i18n::t("Inside"),
            gcarve_core::document::Side::On => i18n::t("On"),
        },
        k => i18n::t(KINDS[kind_index(k)].0),
    }
    .to_string();
    let mut info = Vec::new();
    match op.kind {
        OpType::VCarve => {
            info.push(if op.flat_depth > 0.0 { format!("flat {}", num(op.flat_depth)) } else { i18n::t("to full depth").into() });
        }
        OpType::Relief => info.push(i18n::t("3D model").into()),
        OpType::Imported => info.push(i18n::t("from G-code").into()),
        OpType::Chamfer => {
            info.push(if op.chamfer_width > 0.0 { format!("width {}", num(op.chamfer_width)) } else { format!("V depth {}", num(op.depth)) });
            info.push(["outside", "inside", "on"][op.side as usize].into());
        }
        OpType::Prism => info.push(format!("max depth {}", num(op.depth))),
        OpType::Texture => info.push(format!("depth {} · strokes {}", num(op.depth), num(op.ramp_length))),
        OpType::Photo => info.push(if op.photo.is_empty() { i18n::t("no picture").into() } else { format!("max depth {} · {}×{} px", num(op.depth), op.photo_w, op.photo_h) }),
        _ => info.push(format!("depth {}", num(op.depth))),
    }
    if op.kind == OpType::Pocket {
        info.push(format!("stepover {}%", (op.stepover * 100.0).round() as i32));
    }
    if op.kind == OpType::Profile && op.tabs {
        info.push(format!("{} tabs {} × {}", op.tab_count, num(op.tab_width), num(op.tab_height)));
    }
    if op.kind == OpType::Drill && op.peck > 0.0 {
        info.push(format!("peck {}", num(op.peck)));
    }
    if op.kind == OpType::Bore {
        info.push(format!("helix {}/rev", num(op.stepdown.max(0.0))));
    }
    // the unit only where there is a length ("depth 0.375 · stepover 40% in"), not after "to full depth", "3D model"
    let text = info.join(" · ");
    let unit = if text.contains(|c: char| c.is_ascii_digit()) && !text.ends_with("px") && !text.starts_with("3D") { format!(" {name}") } else { String::new() };
    (side, text + &unit)
}

pub struct Shell {
    /// the first-time guide (onboarding.rs)
    pub tour: crate::onboarding::Tour,
    pub cmds: Vec<String>,
    pub side_tab: usize,
    pub op_sel: Option<usize>,
    /// toolpaths picked with ⌘ / Shift-click in the list (Group puts them in a folder)
    pub op_multi: std::collections::BTreeSet<usize>,
    /// toolpath folders shut in the list
    pub folded: std::collections::BTreeSet<String>,
    /// a sheet tab being renamed: (sheet, text)
    pub sheet_rename: Option<(usize, String)>,
    /// the active sheet the tab strip last scrolled to
    pub tabs_seen: Option<usize>,
    /// the name typed for a new folder (Group…)
    pub new_folder: Option<String>,
    /// folders made with + Folder that hold nothing yet (they live here until a toolpath is dropped in)
    pub empty_folders: std::collections::BTreeSet<String>,
    /// toolpath list filters: only the active sheet's, one layer (-1 = all)
    pub tp_this_sheet: bool,
    pub tp_layer: i32,
    /// (canvas rev, per toolpath: vectors cut per sheet and their layers)
    pub tp_usage: (u64, Vec<crate::inspector::OpUsage>),
    /// toolpath list grouping: 0 none, 1 by bit, 2 by operation
    pub tp_group: u8,
    pub lock_aspect: bool,
    /// inspector Rotate field, degrees
    pub rot_text: String,
    pub edit: [String; 4], // inspector X/Y/W/H text being edited
    pub edit_for: Option<i32>,
    pub zoom_text: String,
    pub canvas_rect: egui::Rect,
    /// the menu bar last frame (menus close on a press anywhere else)
    pub menu_rect: egui::Rect,
    /// the status-bar Snap ▾ arrow last frame: a press on it is the popover's own toggle, not a click elsewhere
    pub snap_arrow: egui::Rect,
}

impl Shell {
    pub fn new() -> Self {
        Shell { tour: crate::onboarding::Tour::load(), cmds: Vec::new(), side_tab: 0, op_sel: None, op_multi: Default::default(), folded: Default::default(), new_folder: None, sheet_rename: None, tabs_seen: None, empty_folders: Default::default(), tp_this_sheet: false, tp_layer: -1, tp_usage: (u64::MAX, Vec::new()), tp_group: 0, lock_aspect: true, rot_text: "15".into(), edit: Default::default(), edit_for: None, zoom_text: String::new(), canvas_rect: egui::Rect::ZERO, menu_rect: egui::Rect::NOTHING, snap_arrow: egui::Rect::NOTHING }
    }
    pub fn push(&mut self, id: impl Into<String>) {
        self.cmds.push(id.into());
    }
}

fn icon_btn(ui: &mut egui::Ui, sh: &mut Shell, id: &str, on: bool) {
    let (icon, _, tip) = def(id);
    if ui.add_enabled_ui(on, |ui| IconButton::new(icon).tip(tip).show(ui)).inner.clicked() {
        sh.push(id);
    }
}

/// ModeBar.qml: 46 px bar under the menus
pub fn mode_bar(ui: &mut egui::Ui, sh: &mut Shell, mode: Mode, title: &str, canvas: &Canvas, can_export: bool) {
    let (can_undo, can_redo) = (canvas.doc.can_undo(), canvas.doc.can_redo());
    egui::Panel::top("modebar").frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin { left: 8, right: 8, top: 1, bottom: 2 })).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            // phones: no name or title, icon-only buttons, so undo / mode / export fit in one row
            let phone = ui.ctx().content_rect().width() < 600.0;
            if !phone {
                ui.label(RichText::new(i18n::t("GCarve")).family(crate::widgets::bold_family()).size(14.0).color(Color32::WHITE));
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(title).color(theme::TEXT.gamma_multiply(0.6))).truncate());
                ui.add_space(6.0);
                theme::vsep(ui, 22.0);
                ui.add_space(4.0);
            }
            icon_btn(ui, sh, "undo", can_undo); // new / open / jobs / save live in the File menu (user decision)
            icon_btn(ui, sh, "redo", can_redo);
            ui.add_space(10.0);
            let wide = ui.ctx().content_rect().width();
            // right-aligned group, then the mode switch in what is left
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if ui.add_enabled_ui(can_export, |ui| IconButton::new("export").text(if phone { "" } else if wide < 760.0 { i18n::t("Export") } else { i18n::t("Export G-code") }).tip(i18n::t("Export G-code (⌘E)")).primary(true).show(ui)).inner.clicked() {
                    sh.push("export");
                }
                ui.add_space(6.0);
                if !phone && IconButton::new("heart").tint(theme::ORANGE).tip(i18n::t("Support GCarve: donate (opens gcarve.app)")).show(ui).clicked() {
                    sh.push("donate");
                }
                // mode switch pill, centred in the remaining space
                let avail = ui.available_rect_before_wrap();
                let pill_w = 6.0 + MODES.iter().map(|(_, _, t, k, _)| 16.0 + 12.0 + if phone { 0.0 } else { 2.0 + text_w(ui, &format!("{t}  {k}"), 12.0) }).sum::<f32>() + 2.0 * 2.0;
                // centred on the window when that leaves it clear of the title and the buttons, else in the space left
                let mid = ui.ctx().content_rect().center().x;
                let x = if mid - pill_w / 2.0 > avail.left() + 8.0 && mid + pill_w / 2.0 < avail.right() - 8.0 { mid } else { avail.center().x };
                let pill = egui::Rect::from_center_size(egui::pos2(x, avail.center().y), egui::vec2(pill_w, 32.0));
                ui.painter().rect(pill, 8.0, theme::FIELD, egui::Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(pill.shrink2(egui::vec2(3.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                child.spacing_mut().item_spacing.x = 2.0;
                for (m, icon, text, key, tip) in MODES {
                    let (text, tip) = (i18n::t(text), i18n::t(tip));
                    let label = if phone { String::new() } else { format!("{text}  {key}") };
                    if IconButton::new(icon).text(&label).tip(tip).size(16.0).padding(6.0).tint(m.color()).active(mode == m).show(&mut child).clicked() {
                        sh.push(format!("mode:{text}"));
                    }
                }
            });
        });
    });
}

fn text_w(ui: &egui::Ui, s: &str, size: f32) -> f32 {
    ui.painter().layout_no_wrap(s.to_owned(), theme::font(size), theme::TEXT).size().x
}

/// The one collapse / expand control for every panel (Qt CollapseButton.qml): same size everywhere, the chevron
/// points the way the panel will move, same wording. `left` = the panel lives on the left edge.
pub fn collapse_button(ui: &mut egui::Ui, left: bool, open: bool, what: &str, key: &str) -> bool {
    let icon = if left == open { "chevron-left" } else { "chevron-right" };
    let tip = format!("{} {}{}", if open { i18n::t("Collapse") } else { i18n::t("Show") }, what, if key.is_empty() { String::new() } else { format!(" ({key})") });
    IconButton::new(icon).size(14.0).padding(3.0).tip(&crate::theme::keys(&tip)).show(ui).clicked()
}

/// 18 px folded edge with the shared collapse button (Main.qml `Rail`)
pub fn folded(ui: &mut egui::Ui, left: bool, what: &str, key: &str) -> bool {
    ui.centered_and_justified(|ui| collapse_button(ui, left, false, what, key)).inner
}

/// ModeRail.qml: 82 px, the tools of the current mode with labels under the icons
/// `extra`: drawn under the Design tools (polygon sides / star points, Qt ModeRail's child column)
pub fn rail(ui: &mut egui::Ui, sh: &mut Shell, mode: Mode, current: &str, sel: usize, paths_on: bool, extra: impl FnOnce(&mut egui::Ui)) {
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 1.0);
    let w = 73.0;
    // same control and place as the side panel's: top row, on the edge facing the canvas
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        if collapse_button(ui, true, true, i18n::t("the tool rail"), "") {
            sh.push("rail:fold");
        }
    });
    let item = |ui: &mut egui::Ui, sh: &mut Shell, id: &str, icon: &str, name: &str, tip: &str, tint: Color32, accent: Color32, on: bool, active: bool| {
        if ui.add_enabled_ui(on, |ui| IconButton::new(icon).text(name).tip(tip).label_below().tint(tint).accent(accent).active(active).width(w).show(ui)).inner.clicked() {
            sh.push(id);
        }
    };
    // short windows (laptops, tablets): the rail scrolls instead of cutting off its last tools
    egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| match mode {
        Mode::Design => {
            for (id, icon, name, key) in TOOLS {
                let name = i18n::t(name);
                let (tint, accent) = tool_color(id);
                item(ui, sh, id, icon, name, &format!("{name} ({key})"), tint, accent, true, current == id);
            }
            item(ui, sh, "import", "open", i18n::t("Import"), i18n::t("Import SVG / DXF / PDF / AI / STL into this document"), theme::TOOL_FILE, theme::TOOL_FILE, true, false);
            item(ui, sh, "nest", "nest", i18n::t("Nest"), i18n::t("Pack the selected parts onto the board"), theme::TOOL_FILE, theme::TOOL_FILE, sel > 0, false);
            extra(ui);
        }
        Mode::Simulate => {
            let g = theme::MODE_SIMULATE;
            item(ui, sh, "sim", "simulate", i18n::t("Carve"), i18n::t("Carve the board with all toolpaths (⌘R)"), g, g, true, false);
            item(ui, sh, "watch", "simulate", i18n::t("Watch"), i18n::t("Watch the cut at machine speed (⇧⌘R)"), g, g, true, false);
            item(ui, sh, "view3d", "board", "3D", i18n::t("Orbit the carved board (⌘3)"), theme::GROUP_VIEW, theme::GROUP_VIEW, true, false);
            item(ui, sh, "paths", "eye", i18n::t("Paths"), i18n::t("Show / hide the toolpath lines over the simulation"), theme::GROUP_VIEW, theme::GROUP_VIEW, true, paths_on);
            item(ui, sh, "tool:measure", "measure", i18n::t("Measure"), i18n::t("Measure on the carved board (M)"), theme::TOOL_MEASURE, theme::TOOL_MEASURE, true, current == "tool:measure");
        }
    });
}

/// SheetTabs.qml: 28 px strip under the canvas
/// The sheet tabs under the drawing: named tabs (double-click or right-click › Rename), which scroll sideways when
/// they don't fit (wheel or trackpad over them), then + / − / All sheets / board setup at the right.
pub fn sheet_tabs(ui: &mut egui::Ui, sh: &mut Shell, doc: &Document, all: bool) {
    ui.spacing_mut().item_spacing.x = 2.0;
    ui.horizontal(|ui| {
        ui.set_height(27.0);
        ui.add_space(6.0);
        // the buttons keep their place at the right; the tabs get the rest
        let buttons_w = 230.0;
        let tabs_w = (ui.available_width() - buttons_w).max(80.0);
        ui.allocate_ui_with_layout(egui::vec2(tabs_w, 27.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            egui::ScrollArea::horizontal().id_salt("sheet-tabs").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).max_width(tabs_w).show(ui, |ui| {
                ui.horizontal(|ui| {
                    // a mouse wheel over the tabs scrolls them sideways
                    if ui.rect_contains_pointer(ui.max_rect()) {
                        let dy = ui.input(|i| i.smooth_scroll_delta.y);
                        if dy != 0.0 {
                            ui.scroll_with_delta(egui::vec2(dy, 0.0));
                        }
                    }
                    for i in 0..doc.sheets.len().max(1) {
                        let current = i as i32 == doc.active_sheet;
                        if let Some((j, name)) = sh.sheet_rename.as_mut().filter(|(j, _)| *j == i) {
                            let _ = j;
                            let r = ui.add(egui::TextEdit::singleline(name).desired_width(110.0).hint_text(format!("Sheet {}", i + 1)));
                            r.request_focus();
                            if r.lost_focus() {
                                if !ui.input(|k| k.key_pressed(egui::Key::Escape)) {
                                    sh.cmds.push(format!("sheetname:{i}:{}", name.trim()));
                                }
                                sh.sheet_rename = None;
                            }
                            continue;
                        }
                        let count = doc.shapes.iter().filter(|s| s.sheet as usize == i).count();
                        let name = doc.sheet_name(i);
                        let label = if count > 0 { format!("{name}  {count}") } else { name.clone() };
                        let galley = ui.painter().layout_no_wrap(label, theme::font(12.0), theme::TEXT);
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(galley.size().x + 24.0, 24.0), egui::Sense::click());
                        let r = r.translate(egui::vec2(0.0, 1.5));
                        let fill = if current { theme::RAISED } else if resp.hovered() { theme::ROW_HOVER } else { Color32::TRANSPARENT };
                        ui.painter().rect(r, 5.0, fill, egui::Stroke::new(1.0, if current { theme::BORDER_STRONG } else { Color32::TRANSPARENT }), egui::StrokeKind::Inside);
                        ui.painter().galley(r.center() - galley.size() / 2.0, galley, if current { theme::TEXT } else { theme::TEXT.gamma_multiply(0.7) });
                        // the active tab comes into view when it changes (another sheet picked elsewhere)
                        if current && sh.tabs_seen != Some(i) {
                            resp.scroll_to_me(None);
                            sh.tabs_seen = Some(i);
                        }
                        if resp.double_clicked() {
                            sh.sheet_rename = Some((i, if name.starts_with(i18n::t("Sheet ")) && name[6..].parse::<usize>().is_ok() { String::new() } else { name.clone() }));
                        } else if resp.clicked() {
                            sh.push(format!("sheet:{i}"));
                        }
                        let resp = resp.tip(i18n::t("Click to work on this sheet · double-click to rename"));
                        resp.context_menu(|ui| {
                            if crate::widgets::menu_item(ui, i18n::t("Rename…"), "").clicked() {
                                sh.sheet_rename = Some((i, name.clone()));
                                ui.close();
                            }
                            if !current && crate::widgets::menu_item(ui, i18n::t("Work on this sheet"), "").clicked() {
                                sh.push(format!("sheet:{i}"));
                                ui.close();
                            }
                        });
                    }
                });
            });
        });
        if IconButton::new("add").size(13.0).padding(3.0).tip(i18n::t("Add a sheet (another board; parts that don't fit go on the next)")).show(ui).clicked() {
            sh.push("sheet:add");
        }
        if ui.add_enabled_ui(doc.sheets.len() > 1, |ui| IconButton::new("remove").size(13.0).padding(3.0).tip(i18n::t("Remove this sheet (its shapes move to the previous one)")).show(ui)).inner.clicked() {
            sh.push("sheet:remove");
        }
        ui.add_space(6.0);
        if IconButton::new("board").text(i18n::t("All sheets")).size(13.0).padding(3.0).active(all).tip(i18n::t("Show every sheet side by side: click a sheet to work on it, drag vectors onto another sheet to move them there")).show(ui).clicked() {
            sh.push("allsheets");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            if IconButton::new("edit").size(13.0).padding(3.0).tip(&format!("Board / job setup: {}", board_summary(doc))).show(ui).clicked() {
                sh.push("board");
            }
        });
    });
}

/// the status-bar Snap popover (View ▸ Snap Settings… opens it too)
pub fn snap_popup() -> egui::Id {
    egui::Id::new("snap-popover")
}

/// Main.qml footer: cursor readout · status · Paths/Width · zoom · Snap ▾ (snap & grid popover) · units
pub fn status_bar(ui: &mut egui::Ui, sh: &mut Shell, canvas: &mut Canvas, synced: &mut crate::dialogs::prefs::Synced, message: &str, status: &str, has_ops: bool) {
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.horizontal(|ui| {
        ui.set_height(27.0);
        ui.add_space(10.0);
        // narrow windows: the pointer read-out gives way to the buttons on the right
        if ui.ctx().content_rect().width() >= 760.0 {
            ui.add_sized([230.0, 20.0], egui::Label::new(RichText::new(status).monospace()).truncate());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            // phones: zoom, fit and Paths only (units and snap live in the menus: View, Edit › Preferences)
            let phone = ui.ctx().content_rect().width() < 600.0;
            if !phone {
            // units
            let (uname, _, _) = unit(&canvas.doc);
            let names = [i18n::t("inches"), i18n::t("feet"), "mm"];
            let cur = match uname { "ft" => 1, "mm" => 2, _ => 0 };
            let mut pick = cur;
            crate::widgets::combo_box("units").width(100.0).selected_text(names[cur]).show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    ui.selectable_value(&mut pick, i, *n);
                }
            }).response.tip(i18n::t("Project units: what numbers mean and how they display. Type 4ft, 48in or 1200mm anywhere to override."));
            if pick != cur {
                canvas.doc.bare_unit_in = units::BARE[pick];
                canvas.touch();
            }
            let chev = IconButton::new("chevron-down").size(12.0).padding(4.0).tip(i18n::t("Snap & grid settings")).active(egui::Popup::is_id_open(ui.ctx(), snap_popup())).show(ui);
            sh.snap_arrow = chev.rect;
            egui::Popup::from_toggle_button_response(&chev).id(snap_popup()).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).align(egui::RectAlign::TOP_END).frame(widgets::popup_frame().inner_margin(14)).show(|ui| {
                crate::dialogs::prefs::snap_grid_ui(ui, synced, &canvas.doc);
            });
            let mut snap = canvas.snap;
            if theme::switch(ui, &mut snap, i18n::t("Snap")).changed() {
                canvas.snap = snap;
            }
            theme::vsep(ui, 18.0);
            if IconButton::new("board").tip(i18n::t("Zoom to board (⌘1)")).show(ui).clicked() {
                sh.push("zoom:board");
            }
            }
            if IconButton::new("fit").tip(i18n::t("Zoom to fit shapes (⌘0 / F)")).show(ui).clicked() {
                sh.push("zoom:fit");
            }
            if IconButton::new("zoom-in").tip(i18n::t("Zoom in (⌘+)")).show(ui).clicked() {
                sh.push("zoom:in");
            }
            let pct = canvas.view.as_ref().map(|v| (v.zoom / 96.0 * 100.0).round() as i32).unwrap_or(100);
            let shown = format!("{pct}%");
            let id = ui.id().with("zoom");
            if !ui.memory(|m| m.has_focus(id)) {
                sh.zoom_text = shown;
            }
            let r = ui.add_sized([if phone { 48.0 } else { 64.0 }, theme::CONTROL_HEIGHT - 2.0], crate::widgets::field(&mut sh.zoom_text).id(id).horizontal_align(egui::Align::Center));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Ok(v) = sh.zoom_text.trim().trim_end_matches('%').parse::<f32>() {
                    sh.push(format!("zoom:{v}"));
                }
            }
            if IconButton::new("zoom-out").tip(i18n::t("Zoom out (⌘−)")).show(ui).clicked() {
                sh.push("zoom:out");
            }
            theme::vsep(ui, 18.0);
            if has_ops {
                if !phone && IconButton::new("offset").text(i18n::t("Width")).active(canvas.show_width).tip(i18n::t("Show / hide cut-width bands")).show(ui).clicked() {
                    canvas.show_width = !canvas.show_width;
                }
                // which toolpaths the drawing shows (and works out) while you edit: Paths off / Picked / All (right to
                // left here, so they read Paths · Picked · All); Simulate always cuts every toolpath
                if !phone {
                    let opts = [(i18n::t("All"), "all", canvas.all_paths, i18n::t("Every toolpath, worked out again a moment after each edit")), (i18n::t("Picked"), "picked", !canvas.all_paths, i18n::t("Only the toolpath you click in the Toolpaths list, or the one open in its dialog"))];
                    for (label, cmd, on, tip) in opts {
                        let r = ui.add_enabled(canvas.show_paths, egui::Button::selectable(canvas.show_paths && on, RichText::new(label).size(12.0)));
                        if r.on_hover_text(tip).on_disabled_hover_text("Paths are off: nothing is drawn or worked out while you edit").clicked() {
                            sh.push(format!("paths:{cmd}"));
                        }
                    }
                }
                if IconButton::new(if canvas.show_paths { "eye" } else { "eye-off" }).text(i18n::t("Paths")).active(canvas.show_paths).tip(i18n::t("Toolpaths on the drawing (⇧⌘T). Off: nothing is drawn or worked out while you edit; Simulate still cuts them all")).show(ui).clicked() {
                    sh.push("paths");
                }
            }
            // Qt: the message fills the space after the cursor readout, left-aligned
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                // a problem was just shown: one click to tell us (the error goes along)
                if gcarve_ui::help::recent_problem(ui.input(|i| i.time)).is_some() && ui.add(egui::Button::new(RichText::new(i18n::t("Report this…")).size(11.0).color(theme::DANGER)).small()).on_hover_text(i18n::t("Send this problem to the people building GCarve, with the error and your version")).clicked() {
                    sh.push("reportproblem");
                }
                ui.add(egui::Label::new(RichText::new(message).color(theme::TEXT.gamma_multiply(0.8))).truncate());
            });
        });
    });
}

/// EmptyState.qml: the "Start a job" card over an empty, untouched document
pub fn empty_state(ui: &mut egui::Ui, sh: &mut Shell, rect: egui::Rect, doc: &Document, recent: &[String]) {
    // on a phone the top bar makes the layout wider than the screen: keep the card on the visible part
    let rect = rect.intersect(ui.ctx().content_rect());
    // never wider / taller than the canvas (phones, tablets) and clipped to it: never over the rail or inspector
    let w = (rect.width() - 24.0).min(460.0);
    if w < 120.0 {
        return;
    }
    let id = ui.id().with("start-card");
    let h = ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or(150.0).min(rect.height() - 24.0);
    let card = egui::Rect::from_center_size(rect.center(), egui::vec2(w, h));
    let mut outer = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(card.left_top(), egui::vec2(w, rect.bottom() - card.top()))).layout(egui::Layout::top_down(egui::Align::Min)));
    outer.set_clip_rect(card.intersect(rect));
    let r = egui::Frame::new().fill(theme::PANEL.gamma_multiply(0.97)).stroke(egui::Stroke::new(1.0, theme::BORDER)).corner_radius(10).inner_margin(20).show(&mut outer, |c| {
        c.set_width(w - 40.0);
        c.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
        c.horizontal(|c| {
            c.label(RichText::new(i18n::t("Start a job")).font(crate::widgets::bold(18.0)).color(Color32::WHITE));
            c.with_layout(egui::Layout::right_to_left(egui::Align::Center), |c| {
                if IconButton::new("dismiss").tip(i18n::t("Close")).show(c).clicked() {
                    sh.push("startcard:close");
                }
            });
        });
        let (uname, u, _) = unit(doc);
        let m = &doc.material;
        let d = if uname == "mm" { 1 } else { 2 };
        c.add(egui::Label::new(RichText::new(format!("{}raw with the tools on the left or bring vectors in. Set the board size first if it isn't {:.d$} × {:.d$} × {:.3} {uname}.", if m.machine.is_some() { "D" } else { i18n::t("Choose your machine, then d") }, m.w * u, m.h * u, m.t * u)).color(theme::TEXT.gamma_multiply(0.65))).wrap());
        c.add_space(6.0);
        c.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            if IconButton::new("simulate").text(i18n::t("Try the demo project")).primary(true).tip(i18n::t("Opens a sample sign with every toolpath type: look around, simulate it, export it")).show(ui).clicked() {
                sh.push("demo");
            }
            if doc.material.machine.is_none() && IconButton::new("machine").text(i18n::t("Choose machine…")).tip(i18n::t("Pick your CNC: feeds are scaled to it, the board starts at its table size and the G-code comes out in its dialect")).show(ui).clicked() {
                sh.push("machine");
            }
            for (id, icon, text, tip) in [("open", "open", i18n::t("Open…"), i18n::t("Open a project (GCarve .tlp, VCarve .crv) or a drawing: SVG / DXF / PDF (⌘O)")), ("import", "relief", i18n::t("Import"), i18n::t("Import SVG / DXF / PDF / AI / STL into this document")), ("tool:text", "text", i18n::t("Add Text"), i18n::t("Add text (T)")), ("board", "board", i18n::t("Board…"), i18n::t("Board size, material, XY zero, post-processor"))] {
                if IconButton::new(icon).text(text).tip(tip).show(ui).clicked() {
                    sh.push(id);
                }
            }
        });
        c.add_space(10.0);
        c.label(RichText::new(i18n::t("OR CHANGE A READY PROJECT")).size(11.0).color(theme::TEXT.gamma_multiply(0.45)));
        crate::inspector::starter_buttons(c, sh);
        if !recent.is_empty() {
            c.add_space(10.0);
            c.label(RichText::new(i18n::t("RECENT")).size(11.0).color(theme::TEXT.gamma_multiply(0.45)));
            for (i, path) in recent.iter().enumerate() {
                let mut parts = path.rsplit(['/', '\\']);
                let name = parts.next().unwrap_or(path);
                let folder = parts.next().unwrap_or("");
                let (r, resp) = c.allocate_exact_size(egui::vec2(c.available_width(), 30.0), egui::Sense::click());
                if resp.hovered() {
                    c.painter().rect_filled(r, 4.0, theme::ROW_HOVER);
                }
                let g = c.painter().text(r.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, name, theme::font(12.0), theme::TEXT);
                // same file name in several folders: say which folder
                if recent.iter().filter(|p| p.rsplit(['/', '\\']).next() == Some(name)).count() > 1 {
                    c.painter().text(egui::pos2(g.right() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, format!("in {folder}"), theme::font(11.0), theme::TEXT_DIM);
                }
                if resp.tip(path.as_str()).clicked() {
                    sh.push(format!("recent:{i}"));
                }
            }
        }
        c.add_space(8.0);
        // right to left: the link keeps its place, a long tip (other languages) is cut short instead of overlapping it
        c.horizontal(|c| c.with_layout(egui::Layout::right_to_left(egui::Align::Center), |c| {
            {
                let id = c.id().with("dont-show");
                let hot = c.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
                let r = c.add(egui::Label::new(RichText::new(i18n::t("Don't show again")).size(11.0).color(theme::ACCENT.gamma_multiply(if hot { 1.0 } else { 0.75 }))).sense(egui::Sense::click()));
                c.ctx().data_mut(|d| d.insert_temp(id, r.hovered()));
                let r = r.on_hover_cursor(egui::CursorIcon::PointingHand).tip(i18n::t("View › Start Card brings it back"));
                if r.clicked() {
                    sh.push("startcard");
                }
            }
            c.add(egui::Label::new(RichText::new(theme::keys(i18n::t("Tip: press ⌘K to find any command by name."))).size(11.0).color(theme::TEXT.gamma_multiply(0.45))).truncate());
        }));
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, r.response.rect.height()));
}

/// App state the menus show (account line, admin items, Open Recent, check marks).
pub struct MenuInfo {
    pub account: String,
    pub signed_in: bool,
    pub admin: bool,
    /// file names, newest first (desktop)
    pub recent: Vec<String>,
    pub sim: bool,
    pub view_3d: bool,
    pub start_card: bool,
    pub show_grid: bool,
    pub rulers: bool,
}

/// Main.qml menuBar: the same menus, wording and shortcuts
pub fn menu_bar(ui: &mut egui::Ui, sh: &mut Shell, canvas: &mut Canvas, m: &MenuInfo, rail_open: &mut bool, panel_open: &mut bool, has_ops: bool) {
    let sel = canvas.selection_count();
    let (can_undo, can_redo) = (canvas.doc.can_undo(), canvas.doc.can_redo());
    let web = cfg!(target_arch = "wasm32");
    let text = canvas.selected_text() != 0;
    let keep = sh.snap_arrow;
    let mut search = false;
    sh.menu_rect = gcarve_ui::menubar::bar(ui, keep, |ui: &mut egui::Ui, tops: &mut Vec<egui::Response>| {
            // item: label, shortcut text, enabled, command id
            fn item(ui: &mut egui::Ui, sh: &mut Shell, text: &str, key: &str, on: bool, id: &str) {
                if ui.add_enabled_ui(on, |ui| widgets::menu_item(ui, text, key)).inner.clicked() {
                    sh.push(id);
                    ui.close();
                }
            }
            fn check(ui: &mut egui::Ui, text: &str, v: &mut bool) {
                if widgets::menu_check(ui, text, "", v).clicked() {
                    ui.close();
                }
            }
            tops.push(ui.menu_button(i18n::t("File"), |ui| {
                item(ui, sh, i18n::t("New"), "⌘N", true, "new");
                item(ui, sh, i18n::t("New Sign…"), "", true, "sign");
                item(ui, sh, i18n::t("New Tray or Cutting Board…"), "", true, "tray");
                item(ui, sh, i18n::t("New Cabinet Doors…"), "", true, "doors");
                item(ui, sh, i18n::t("New Clock…"), "", true, "clock");
                item(ui, sh, i18n::t("New Spoilboard…"), "", true, "spoilboard");
                item(ui, sh, i18n::t("Close Tab"), "⌘W", true, "closetab");
                item(ui, sh, if web { i18n::t("Open / Import from Computer…") } else { i18n::t("Open Project / Import…") }, "⌘O", true, "open");
                item(ui, sh, i18n::t("Import G-code…"), "", true, "importgcode");
                item(ui, sh, i18n::t("Cloud Projects…"), "", true, "cloud");
                item(ui, sh, i18n::t("Free Community Projects…"), "", true, "community");
                item(ui, sh, i18n::t("Make It on My Machine…"), "", has_ops, "makeit");
                if !web {
                    item(ui, sh, i18n::t("Jobs…"), "⌘J", true, "jobs");
                    item(ui, sh, i18n::t("Import 3D Model (STL)…"), "", true, "importstl");
                    ui.add_enabled_ui(!m.recent.is_empty(), |ui| {
                        widgets::submenu(ui, i18n::t("Open Recent"), |ui| {
                            for (i, r) in m.recent.iter().enumerate() {
                                // "name — folder" (the full path made the submenu as wide as the screen)
                                let p = std::path::Path::new(r);
                                let name = p.file_name().map_or(r.clone(), |n| n.to_string_lossy().into_owned());
                                let dir = p.parent().and_then(|d| d.file_name()).map(|d| format!(" — {}", d.to_string_lossy())).unwrap_or_default();
                                item(ui, sh, &format!("{name}{dir}"), "", true, &format!("recent:{i}"));
                            }
                            widgets::menu_separator(ui);
                            item(ui, sh, i18n::t("Clear Menu"), "", true, "recent:clear");
                        });
                    });
                }
                widgets::menu_separator(ui);
                item(ui, sh, if web { i18n::t("Save to Cloud") } else { i18n::t("Save") }, "⌘S", true, "save");
                if web {
                    item(ui, sh, i18n::t("Download Project (.tlp)"), "", true, "saveas");
                } else {
                    item(ui, sh, i18n::t("Save As…"), "⇧⌘S", true, "saveas");
                    item(ui, sh, i18n::t("Save to Cloud"), "", true, "savecloud");
                }
                item(ui, sh, i18n::t("Save as Job Template…"), "", true, "jobtemplate");
                widgets::menu_separator(ui);
                widgets::submenu(ui, i18n::t("Export Vectors"), |ui| {
                    // the selection, or every visible vector of this sheet, at real size
                    item(ui, sh, i18n::t("As SVG (curves kept)"), "", true, "vexport:svg");
                    item(ui, sh, i18n::t("As DXF (for CAD / other CAM)"), "", true, "vexport:dxf");
                    item(ui, sh, i18n::t("As DWG (AutoCAD)"), "", true, "vexport:dwg");
                    item(ui, sh, i18n::t("As PDF (print, full size)"), "", true, "vexport:pdf");
                });
                item(ui, sh, i18n::t("Send to GBurn (laser)…"), "", true, "sendgburn");
                item(ui, sh, i18n::t("Setup Sheet…"), "", has_ops, "setupsheet"); // printable job sheet for the operator
                item(ui, sh, i18n::t("Save Preview Image (PNG)"), "", has_ops, "previewimage");
                item(ui, sh, i18n::t("Share Project with Someone…"), "", true, "shareproject");
                item(ui, sh, i18n::t("Share to Community…"), "", has_ops, "communityshare");
                widgets::menu_separator(ui);
                item(ui, sh, &m.account, "", true, "account");
            }).response);
            tops.push(ui.menu_button(i18n::t("Edit"), |ui| {
                item(ui, sh, i18n::t("Undo"), "⌘Z", can_undo, "undo");
                item(ui, sh, i18n::t("Redo"), "⇧⌘Z", can_redo, "redo");
                widgets::menu_separator(ui);
                item(ui, sh, i18n::t("Cut"), "⌘X", sel > 0, "cut");
                item(ui, sh, i18n::t("Copy"), "⌘C", sel > 0, "copy");
                item(ui, sh, i18n::t("Paste"), "⌘V", true, "paste");
                widgets::menu_separator(ui);
                item(ui, sh, i18n::t("Select All"), "⌘A", true, "selectall");
                item(ui, sh, i18n::t("Vector Selector…"), "", true, "selector");
                item(ui, sh, i18n::t("Duplicate (with toolpaths)"), "⌘D", sel > 0, "clone");
                widgets::menu_separator(ui);
                item(ui, sh, i18n::t("Group"), "⌘G", sel > 1, "group");
                item(ui, sh, i18n::t("Ungroup"), "⇧⌘G", canvas.selection_has_group(), "ungroup");
                item(ui, sh, i18n::t("Delete"), "⌫", sel > 0 || canvas.sel_dim >= 0, "delete");
                widgets::menu_separator(ui);
                item(ui, sh, i18n::t("Add Text…"), "T", true, "tool:text");
                item(ui, sh, i18n::t("Text on Curve…"), "", sel == 1, "text:curve");
                item(ui, sh, i18n::t("Edit Text…"), "", text, "text:edit");
                item(ui, sh, i18n::t("Convert Text to Curves"), "", text, "text:curves");
                widgets::menu_separator(ui);
                item(ui, sh, i18n::t("Preferences…"), "⌘,", true, "prefs");
            }).response);
            tops.push(ui.menu_button(i18n::t("Vectors"), |ui| {
                for (t, k, on, id) in [(i18n::t("Weld"), "", sel > 1, "weld"), (i18n::t("Subtract"), "", sel > 1, "subtract"), (i18n::t("Intersect"), "", sel > 1, "intersect")] {
                    item(ui, sh, t, k, on, id);
                }
                widgets::menu_separator(ui);
                item(ui, sh, i18n::t("Shapes Library…"), "", true, "shapes");
                item(ui, sh, i18n::t("Trace Image…"), "", true, "trace");
                widgets::menu_separator(ui);
                for (t, k, on, id) in [(i18n::t("Offset…"), "", sel > 0, "offset"), (i18n::t("Corners: Dogbone / Fillet…"), "", sel > 0, "dogbone"), (i18n::t("Join Open Vectors"), "", sel > 1, "join"), (i18n::t("Close Vectors"), "", sel > 0, "close"), (i18n::t("Extend Ends to Next Vector"), "", sel > 0, "extend"), (i18n::t("Vector Texture…"), "", sel > 0, "vtexture"), (i18n::t("Plate Production…"), "", sel > 0, "plates"), (i18n::t("Shelf-Pin Holes…"), "", sel > 0, "shelfpins"), (i18n::t("Box Maker (finger joints)…"), "", true, "boxmaker"), (i18n::t("Jigsaw Puzzle…"), "", true, "puzzle"), (i18n::t("Living Hinge…"), "", true, "hinge"), (i18n::t("Stencil Bridges…"), "", sel > 0, "stencil"), (i18n::t("Halftone from Picture…"), "", true, "halftone"), (i18n::t("QR Code…"), "", true, "qr"), (i18n::t("Rotary Layout (spiral, flutes)…"), "", canvas.doc.material.rotary.is_some(), "rotarylayout"), (i18n::t("Check Vectors (open, duplicate, tiny)"), "", true, "validate")] {
                    item(ui, sh, t, k, on, id);
                }
                ui.add_enabled_ui(sel > 1, |ui| {
                    widgets::submenu(ui, i18n::t("Distort"), |ui| {
                        item(ui, sh, i18n::t("Bend Along Curve"), "", true, "distort:1");
                        item(ui, sh, i18n::t("Fit Into Four Corners"), "", true, "distort:4");
                        item(ui, sh, i18n::t("Fit Between Two Curves"), "", true, "distort:2");
                    });
                });
                ui.add_enabled_ui(sel > 0, |ui| {
                    widgets::submenu(ui, i18n::t("Simplify (fewer nodes)"), |ui| {
                        let u = if crate::units::name(&canvas.doc) == "mm" { ["0.03 mm", i18n::t("0.13 mm"), "0.5 mm"] } else { ["0.001 in", i18n::t("0.005 in"), "0.02 in"] };
                        for (label, tol) in [(format!("Fine: within {}", u[0]), "0.001"), (format!("Medium: within {}", u[1]), "0.005"), (format!("Coarse: within {}", u[2]), "0.02")] {
                            item(ui, sh, &label, "", true, &format!("simplify:{tol}"));
                        }
                        widgets::menu_separator(ui);
                        item(ui, sh, i18n::t("Fit Arcs and Curves (smooth)"), "", true, "fitcurves:0.005");
                    });
                });
                widgets::menu_separator(ui);
                for (t, k, on, id) in [(i18n::t("Flip Horizontal"), "", sel > 0, "fliph"), (i18n::t("Flip Vertical"), "", sel > 0, "flipv"), (i18n::t("Rotate 90°"), "", sel > 0, "rotate"), (i18n::t("Centre on Board"), "", sel > 0, "centre"), (i18n::t("Align & Distribute…"), "⇧⌘A", sel > 0, "align"), (i18n::t("Array Copy…"), "", sel > 0, "array"), (i18n::t("Constrain Two Shapes…"), "", sel == 2, "link"), (i18n::t("Nest on Board…"), "", sel > 0, "nest")] {
                    item(ui, sh, t, k, on, id);
                }
                item(ui, sh, i18n::t("Mirror Copy to the Right"), "", sel > 0, "mirror:right");
                item(ui, sh, i18n::t("Mirror Copy Upwards"), "", sel > 0, "mirror:up");
                item(ui, sh, i18n::t("Move to Next Sheet"), "", sel > 0 && canvas.doc.sheets.len() > 1, "sheet:next");
                item(ui, sh, i18n::t("New Sheet"), "", true, "sheet:add");
                widgets::submenu(ui, i18n::t("New Back Side (two-sided job)"), |ui| {
                    item(ui, sh, i18n::t("Flip the board left to right"), "", true, "sheet:back-lr");
                    item(ui, sh, i18n::t("Flip the board top to bottom"), "", true, "sheet:back-tb");
                });
            }).response);
            tops.push(ui.menu_button(i18n::t("Variables"), |ui| {
                item(ui, sh, i18n::t("Variables & Constraints…"), "⇧⌘E", true, "vars");
                item(ui, sh, i18n::t("Add Variable"), "", true, "var:add");
            }).response);
            tops.push(ui.menu_button(i18n::t("Toolpaths"), |ui| {
                for (i, key) in ["⇧⌘P", "⇧⌘K", "⇧⌘D", "⇧⌘V", "⇧⌘B", "⇧⌘I", "⇧⌘F"].iter().enumerate() {
                    let key = if i == 0 { "" } else { key }; // ⇧⌘P is the side panel (Qt binds it twice)
                    item(ui, sh, &i18n::tf("New {kind}…", &[("kind", &i18n::t(KINDS[i].0))]), key, sel > 0, &format!("kind:{i}"));
                }
                for i in [8, 9, 10, 11] {
                    item(ui, sh, &i18n::tf("New {kind}…", &[("kind", &i18n::t(KINDS[i].0))]), "", sel > 0, &format!("kind:{i}"));
                }
                item(ui, sh, i18n::t("Export G-code…"), "⌘E", has_ops, "export");
                item(ui, sh, i18n::t("Export Air-Cut Test…"), "", has_ops, "aircut");
                widgets::menu_separator(ui);
                item(ui, sh, i18n::t("Save as Template…"), "", has_ops, "templates");
                item(ui, sh, i18n::t("Apply Template…"), "", true, "templates");
                widgets::menu_separator(ui);
                if !web {
                    item(ui, sh, i18n::t("Send to GCarve Control"), "", has_ops, "control:send");
                    item(ui, sh, i18n::t("Open GCarve Control"), "", true, "control:open");
                    widgets::menu_separator(ui);
                }
                item(ui, sh, i18n::t("Tool Library…"), "", true, "tools");
                item(ui, sh, i18n::t("Board / Job Setup…"), "", true, "board");
                item(ui, sh, i18n::t("Choose Machine…"), "", true, "machine");
                item(ui, sh, i18n::t("Surface the Board…"), "", true, "surface");
                item(ui, sh, i18n::t("Inlay Wizard…"), "", true, "inlay");
                item(ui, sh, i18n::t("3D Models…"), "", true, "reliefs");
                item(ui, sh, i18n::t("Post-Processors…"), "", true, "posts");
            }).response);
            tops.push(ui.menu_button(i18n::t("View"), |ui| {
                let mut paths = canvas.show_paths;
                if widgets::menu_check(ui, i18n::t("Show Toolpaths"), "⇧⌘T", &mut paths).clicked() {
                    sh.push("paths");
                    ui.close();
                }
                let mut every = canvas.all_paths;
                if widgets::menu_check(ui, i18n::t("Every Toolpath While Drawing"), "", &mut every).on_hover_text(i18n::t("Off: while drawing, only the toolpath picked in the list (or open in its dialog) is shown and worked out, so edits stay quick. Simulate always shows them all.")).clicked() {
                    sh.push("allpaths");
                    ui.close();
                }
                item(ui, sh, i18n::t("Show All Toolpaths"), "", has_ops, "ops:showall");
                item(ui, sh, i18n::t("Hide All Toolpaths"), "", has_ops, "ops:hideall");
                check(ui, i18n::t("Show Cut Width"), &mut canvas.show_width);
                item(ui, sh, i18n::t("Simulate Cut"), "⌘R", has_ops, "sim");
                item(ui, sh, i18n::t("Watch the Cut"), "⇧⌘R", has_ops, "watch");
                let mut v3 = m.view_3d;
                if widgets::menu_check(ui, i18n::t("3D View"), "⌘3", &mut v3).clicked() {
                    sh.push("view3d");
                    ui.close();
                }
                let mut s = m.sim;
                if ui.add_enabled_ui(has_ops, |ui| widgets::menu_check(ui, i18n::t("Show Simulation"), "", &mut s)).inner.clicked() {
                    sh.push("showsim");
                    ui.close();
                }
                for (t, k, id) in [(i18n::t("Zoom In"), "⌘+", "zoom:in"), (i18n::t("Zoom Out"), "⌘−", "zoom:out"), (i18n::t("Zoom to Fit"), "⌘0", "zoom:fit"), (i18n::t("Zoom to Board"), "⌘1", "zoom:board")] {
                    item(ui, sh, t, k, true, id);
                }
                widgets::menu_separator(ui);
                item(ui, sh, i18n::t("Command Palette…"), "⌘K", true, "palette");
                if widgets::menu_check(ui, i18n::t("Side Panel"), "⇧⌘P", panel_open).clicked() {
                    ui.close();
                }
                check(ui, i18n::t("Tool Rail"), rail_open);
                let mut card = m.start_card;
                if widgets::menu_check(ui, i18n::t("Start Card"), "", &mut card).clicked() {
                    sh.push("startcard");
                    ui.close();
                }
                item(ui, sh, i18n::t("Actual Size (100%)"), "⌘2", true, "zoom:100");
                widgets::menu_separator(ui);
                check(ui, i18n::t("Snap"), &mut canvas.snap);
                item(ui, sh, i18n::t("Snap Settings…"), "", true, "snap:settings");
                item(ui, sh, i18n::t("Clear Guides"), "", !canvas.doc.guides.is_empty(), "guides:clear");
                widgets::menu_separator(ui);
                item(ui, sh, i18n::t("Photo of the Board…"), "", true, "boardphoto");
                if let Some(b) = &canvas.board_photo {
                    item(ui, sh, if b.show { i18n::t("Hide the Board Photo") } else { i18n::t("Show the Board Photo") }, "", true, "boardphoto:toggle");
                    item(ui, sh, i18n::t("Remove the Board Photo"), "", true, "boardphoto:remove");
                }
                item(ui, sh, if canvas.all_sheets { i18n::t("One Sheet at a Time") } else { i18n::t("All Sheets Side by Side") }, "", true, "allsheets");
                let (line, can) = crate::spacemouse::status();
                item(ui, sh, if can || line.is_empty() { i18n::t("Connect a SpaceMouse…") } else { line.split(':').next().unwrap_or(i18n::t("SpaceMouse")) }, "", can, "spacemouse");
                for (t, on, id) in [(i18n::t("Show Grid"), m.show_grid, "grid:toggle"), (i18n::t("Rulers"), m.rulers, "rulers:toggle")] {
                    let mut on = on;
                    if widgets::menu_check(ui, t, "", &mut on).clicked() {
                        sh.push(id);
                        ui.close();
                    }
                }
                widgets::submenu(ui, i18n::t("Units"), |ui| {
                    let (uname, _, _) = unit(&canvas.doc);
                    for (t, n, v) in [(i18n::t("Inches"), "in", units::BARE[0]), (i18n::t("Feet"), "ft", units::BARE[1]), (i18n::t("Millimetres"), "mm", units::BARE[2])] {
                        let mut on = uname == n;
                        if widgets::menu_check(ui, t, "", &mut on).clicked() {
                            canvas.doc.bare_unit_in = v;
                            canvas.touch();
                            ui.close();
                        }
                    }
                });
            }).response);
            tops.push(ui.menu_button(i18n::t("Help"), |ui| {
                item(ui, sh, i18n::t("♥ Donate to GCarve…"), "", true, "donate");
                if m.signed_in {
                    item(ui, sh, i18n::t("Dashboard (account, requests)…"), "", true, "manage");
                } else {
                    item(ui, sh, i18n::t("Sign In…"), "", true, "account");
                }
                if m.admin {
                    item(ui, sh, i18n::t("User Management…"), "", true, "admin");
                }
                item(ui, sh, i18n::t("Getting Started Guide"), "", true, "tour:show");
                item(ui, sh, i18n::t("How Do I…?"), "", true, "guide");
                item(ui, sh, i18n::t("Keyboard Shortcuts"), "", true, "shortcuts");
                item(ui, sh, i18n::t("Open the Demo Project"), "", true, "demo");
                item(ui, sh, i18n::t("Request a Change…"), "", true, "request");
                item(ui, sh, i18n::t("Shared with Me…"), "", true, "shares");
                item(ui, sh, i18n::t("Support Page"), "", true, "support");
                item(ui, sh, i18n::t("GCarve Website"), "", true, "website");
                item(ui, sh, i18n::t("Report a Bug…"), "", true, "bug");
                // web: the free desktop apps; desktop: check for a newer version
                if cfg!(target_arch = "wasm32") {
                    item(ui, sh, i18n::t("Download the Desktop App"), "", true, "download");
                } else {
                    let up = gcarve_ui::help::available().map_or(i18n::t("Check for Updates…").to_string(), |(v, _)| format!("Update available: {v}…"));
                    item(ui, sh, &up, "", true, "update");
                }
                widgets::menu_separator(ui);
                item(ui, sh, i18n::t("About GCarve"), "", true, "about");
            }).response);
    }, |ui: &mut egui::Ui| search = gcarve_ui::menubar::search_box(ui));
    if search {
        sh.push("palette");
    }
}

