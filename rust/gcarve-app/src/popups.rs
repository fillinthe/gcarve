//! Popups and dialogs around the canvas (Qt app: Main.qml sizePopup / nodePopup / shapeMenu / dimPopup, OffsetDialog,
//! DogboneDialog, ArrayDialog, AlignDialog, NestDialog). The Edit and Vectors menus are built here too so the app
//! shell only adds one call per menu.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::canvas::{Canvas, CtxMenu, Request, Tool};
use crate::text::TextDialog;
use crate::units;
use crate::widgets::{self as W, TipExt};
use eframe::egui::{self, RichText};
use gcarve_core::geom::pt;

/// Click without dragging with a shape tool: type the size (centred at the click).
pub struct SizePopup {
    pub tool: Tool,
    pub at: egui::Pos2,
    pub w: String,
    pub h: String,
    /// centre X / Y text (the click, editable)
    pub cx: String,
    pub cy: String,
    /// Square / Circle: height = width
    pub lock: bool,
    /// first frame: Width takes the focus, selected (Qt openAt)
    pub fresh: bool,
}

pub struct NodePopup {
    pub shape: i32,
    pub node: i32,
    pub at: egui::Pos2,
    pub x: String,
    pub y: String,
    pub len: String,
    pub has_next: bool,
}

pub struct DimPopup {
    pub index: usize,
    pub at: egui::Pos2,
    pub label: String,
    pub drive: String,
}

fn popup_window<'a>(title: &'a str, at: egui::Pos2) -> egui::Window<'a> {
    egui::Window::new(title).collapsible(false).resizable(false).title_bar(false).fixed_pos(at + egui::vec2(8.0, 8.0)).frame(W::popup_frame())
}

impl Canvas {
    pub(crate) fn open_dim_popup(&mut self, i: usize, at: egui::Pos2) {
        let d = &self.doc.dimensions[i];
        self.dim_popup = Some(DimPopup { index: i, at, label: d.label.clone(), drive: d.drive.clone() });
    }
    pub(crate) fn open_node_popup(&mut self, shape: i32, node: i32, at: egui::Pos2) {
        let Some(n) = self.node_info(shape, node) else { return };
        let or = |b: String, v: f64| if b.is_empty() { units::text(&self.doc, v) } else { b };
        let (x, y, len) = (or(n.bx, n.x), or(n.by, n.y), or(n.blen, n.len));
        self.node_popup = Some(NodePopup { shape, node, at, x, y, len, has_next: n.has_next });
    }

    /// Context menus (opened by tools.rs on right-click) and the small popups.
    pub(crate) fn popups_ui(&mut self, ui: &mut egui::Ui, resp: &egui::Response) {
        let ctx = ui.ctx().clone();
        match self.shot_hook.as_str() {
            // Qt Main.qml --dialog hooks: shapeMenu.popup(canvas, 420, 260); sizePopup.openAt(ellipse, 3, 2, 500, 400)
            "shapemenu" => {
                egui::Area::new(egui::Id::new("shot-menu")).fixed_pos(resp.rect.min + egui::vec2(420.0, 260.0)).order(egui::Order::Foreground).show(&ctx, |ui| {
                    egui::Frame::menu(ui.style()).show(ui, |ui| {
                        ui.set_width(200.0 - 12.0);
                        self.shape_menu(ui)
                    })
                });
            }
            "sizepopup" => {
                self.shot_hook.clear();
                self.tool = Tool::Ellipse;
                self.open_size_popup(resp.rect.min + egui::vec2(500.0, 400.0), gcarve_core::geom::pt(3.0, 2.0));
            }
            _ => {}
        }
        let menu = self.ctx_menu;
        if menu != CtxMenu::None {
            let r = resp.context_menu(|ui| match menu {
                CtxMenu::Node(shape, node) => self.node_menu(ui, shape, node as i32),
                CtxMenu::Dimension(i) => self.dim_menu(ui, i),
                _ => self.shape_menu(ui),
            });
            if r.is_none() {
                self.ctx_menu = CtxMenu::None;
            }
        }

        if let Some(mut p) = self.size_popup.take() {
            let mut done = false;
            let mut ok = false;
            let radial = matches!(p.tool, Tool::Polygon | Tool::Star);
            let was_fresh = p.fresh;
            let shown = popup_window("size", p.at).show(&ctx, |ui| {
                ui.label(RichText::new(match p.tool { Tool::Rect => i18n::t("Rectangle"), Tool::Ellipse => i18n::t("Ellipse"), Tool::Polygon => i18n::t("Polygon"), _ => i18n::t("Star") }).font(W::bold(crate::theme::FONT_SIZE)));
                egui::Grid::new("size").num_columns(2).spacing([10.0, 6.0]).min_col_width(81.0).show(ui, |ui| {
                    ui.label(if radial { i18n::t("Diameter") } else { i18n::t("Width") });
                    if std::mem::take(&mut p.fresh) {
                        units::focus_next(ui.ctx());
                    }
                    // locked = keep proportions: type in either field and the other follows (both stay editable)
                    let before_w = p.w.clone();
                    units::num_field(ui, "w", &mut p.w, &self.doc, true, 120.0);
                    ui.end_row();
                    if p.lock && p.w != before_w {
                        p.h = p.w.clone();
                    }
                    if !radial {
                        ui.label(i18n::t("Height"));
                        let before_h = p.h.clone();
                        units::num_field(ui, "h", &mut p.h, &self.doc, true, 120.0);
                        ui.end_row();
                        if p.lock && p.h != before_h {
                            p.w = p.h.clone();
                        }
                    }
                });
                if !radial && W::checkbox(ui, &mut p.lock, if p.tool == Tool::Ellipse { i18n::t("Circle (keep 1:1)") } else { i18n::t("Square (keep 1:1)") }).changed() && p.lock {
                    p.h = p.w.clone();
                }
                if radial {
                    p.h = p.w.clone();
                }
                egui::Grid::new("size-c").num_columns(2).spacing([10.0, 6.0]).min_col_width(81.0).show(ui, |ui| {
                    ui.label(i18n::t("Centre X"));
                    units::num_field(ui, "cx", &mut p.cx, &self.doc, true, 120.0);
                    ui.end_row();
                    ui.label(i18n::t("Centre Y"));
                    units::num_field(ui, "cy", &mut p.cy, &self.doc, true, 120.0);
                    ui.end_row();
                });
                ui.memory_mut(|m| {
                    if m.focused().is_none() {
                        m.request_focus(ui.id().with("w"));
                    }
                });
                if p.tool == Tool::Polygon {
                    ui.horizontal(|ui| {
                        ui.label(i18n::t("Sides"));
                        ui.add(W::SpinBox::new(&mut self.polygon_sides, 3..=64));
                    });
                }
                if p.tool == Tool::Star {
                    ui.horizontal(|ui| {
                        ui.label(i18n::t("Points"));
                        ui.add(W::SpinBox::new(&mut self.star_points, 3..=64));
                        ui.label(i18n::t("Inner"));
                        ui.add(W::SpinBox::new(&mut self.star_ratio, 0.05..=0.95).step(0.05).decimals(2));
                    });
                }
                ui.label(RichText::new(i18n::t("Math, units (1in + 5mm) and variables work here.")).size(11.0).color(crate::theme::TEXT.gamma_multiply(0.5)));
                let hit = W::button_row(ui, &[(i18n::t("Cancel"), false), (i18n::t("Add"), true)], 0.0);
                ok = hit == Some(1) || ui.input(|i| i.key_pressed(egui::Key::Enter));
                done = hit == Some(0) || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
            // a click anywhere else, or another tool, puts the box away
            let outside = shown.is_some_and(|r| ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|q| !r.response.rect.contains(q))));
            if !was_fresh && (outside || self.tool != p.tool) {
                done = true;
                ok = false;
            }
            if ok {
                let v: Result<Vec<f64>, String> = [&p.w, &p.h, &p.cx, &p.cy].iter().map(|t| units::get(&self.doc, t, true)).collect();
                match v {
                    Ok(v) => {
                        self.add_sized_shape(p.tool, v[2], v[3], v[0], v[1]);
                        done = true;
                    }
                    Err(e) => self.status = e,
                }
            }
            if !done {
                self.size_popup = Some(p);
            }
        }

        if let Some(mut p) = self.node_popup.take() {
            let mut done = false;
            let mut ok = false;
            popup_window("node", p.at).show(&ctx, |ui| {
                ui.label(RichText::new(i18n::t("Node position")).font(W::bold(crate::theme::FONT_SIZE)));
                egui::Grid::new("node").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    let u = units::name(&self.doc);
                    ui.label(format!("X ({u})"));
                    units::num_field(ui, "x", &mut p.x, &self.doc, true, 110.0);
                    ui.end_row();
                    ui.label(format!("Y ({u})"));
                    units::num_field(ui, "y", &mut p.y, &self.doc, true, 110.0);
                    ui.end_row();
                    ui.add_enabled(p.has_next, egui::Label::new(format!("Length to next ({u})")));
                    ui.add_enabled_ui(p.has_next, |ui| units::num_field(ui, "len", &mut p.len, &self.doc, true, 110.0));
                    ui.end_row();
                });
                ui.label(RichText::new(i18n::t("Type a variable to keep it linked.")).size(11.0).color(crate::theme::TEXT.gamma_multiply(0.5)));
                let hit = W::button_row(ui, &[(i18n::t("Cancel"), false), (i18n::t("Apply"), true)], 0.0);
                ok = hit == Some(1) || ui.input(|i| i.key_pressed(egui::Key::Enter));
                done = hit == Some(0) || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
            if ok {
                let len_used = p.has_next && !p.len.trim().is_empty();
                match [&p.x, &p.y].into_iter().chain(len_used.then_some(&p.len)).map(|t| units::get(&self.doc, t, true)).find_map(Result::err) {
                    Some(e) => self.status = e,
                    None => {
                        self.set_node_field(p.shape, p.node, "x", &p.x);
                        self.set_node_field(p.shape, p.node, "y", &p.y);
                        if len_used {
                            self.set_node_field(p.shape, p.node, "len", &p.len);
                        }
                        done = true;
                    }
                }
            }
            if !done {
                self.node_popup = Some(p);
            }
        }

        if let Some(mut p) = self.dim_popup.take() {
            let mut done = false;
            let mut ok = false;
            let Some(d) = self.doc.dimensions.get(p.index).cloned() else { return };
            popup_window("dim", p.at).show(&ctx, |ui| {
                ui.label(RichText::new(format!("Dimension: {}", self.dimension_text(&d))).font(W::bold(crate::theme::FONT_SIZE)));
                let anchored = d.shape_a >= 0 && d.shape_b >= 0;
                egui::Grid::new("dim").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label(i18n::t("Label"));
                    ui.add(W::field(&mut p.label).desired_width(220.0).hint_text("{v} = measured value"));
                    ui.end_row();
                    ui.label(i18n::t("Drive length"));
                    ui.add_enabled(anchored, W::field(&mut p.drive).desired_width(220.0).hint_text(i18n::t("e.g. box-w, 4in, 2*gap")));
                    ui.end_row();
                });
                let note = if !anchored { i18n::t("Snap both ends to shape nodes to let this dimension drive geometry.") } else if d.shape_a == d.shape_b { i18n::t("Both ends on one shape: driving moves the second node.") } else { i18n::t("Ends on two shapes: driving moves the second shape.") };
                ui.add(egui::Label::new(RichText::new(note).size(11.0).color(crate::theme::TEXT.gamma_multiply(0.6))).wrap());
                ui.horizontal(|ui| {
                    if ui.add(W::Button::new(i18n::t("Delete"))).clicked() {
                        self.remove_dimension(p.index);
                        done = true;
                    }
                    let hit = W::button_row(ui, &[(i18n::t("Cancel"), false), (i18n::t("Apply"), true)], 0.0);
                    ok = hit == Some(1) || ui.input(|i| i.key_pressed(egui::Key::Enter));
                    done |= hit == Some(0) || ui.input(|i| i.key_pressed(egui::Key::Escape));
                });
            });
            if ok {
                self.set_dimension(p.index, &p.label, &p.drive);
                done = true;
            }
            if !done {
                self.dim_popup = Some(p);
            }
        }
    }

    /// Main.qml shapeMenu: the same items and submenus; each runs the app command of the same id.
    fn shape_menu(&mut self, ui: &mut egui::Ui) {
        let n = self.sel.len();
        let has = n > 0;
        let text = self.selected_text();
        let group = self.selection_has_group();
        let sheets = self.doc.sheets.len();
        // layers the selection could move to (current one ticked when all selected shapes are on it)
        let layers: Vec<(String, bool)> = self.doc.layers.iter().enumerate().map(|(i, l)| (l.name.clone(), has && self.sel.iter().all(|id| self.doc.find(*id).is_some_and(|s| s.layer == i as i32)))).collect();
        let mut to_layer: Option<i32> = None; // -1 = a new layer
        let mut run: Option<&'static str> = None;
        let mut item = |ui: &mut egui::Ui, enabled: bool, label: &str, id: &'static str| {
            if ui.add_enabled_ui(enabled, |ui| crate::widgets::menu_item(ui, label, "")).inner.clicked() {
                run = Some(id);
            }
        };
        item(ui, has, i18n::t("Cut"), "cut");
        item(ui, has, i18n::t("Copy"), "copy");
        item(ui, true, i18n::t("Paste"), "paste");
        item(ui, has, i18n::t("Duplicate"), "clone");
        item(ui, has, i18n::t("Delete"), "delete");
        crate::widgets::menu_separator(ui);
        item(ui, group || n > 1, if group { i18n::t("Ungroup") } else { i18n::t("Group") }, "group:toggle");
        item(ui, text > 0, i18n::t("Edit Text…"), "text:edit");
        item(ui, text > 0, i18n::t("Convert Text to Curves"), "text:curves");
        item(ui, has, i18n::t("Edit Nodes"), "tool:nodes");
        crate::widgets::menu_separator(ui);
        crate::widgets::submenu(ui, i18n::t("Move to Layer"), |ui| {
            for (i, (name, here)) in layers.iter().enumerate() {
                if ui.add_enabled_ui(has && !here, |ui| crate::widgets::menu_item(ui, &if *here { format!("✓ {name}") } else { name.clone() }, "")).inner.clicked() {
                    to_layer = Some(i as i32);
                }
            }
            crate::widgets::menu_separator(ui);
            if ui.add_enabled_ui(has, |ui| crate::widgets::menu_item(ui, i18n::t("New Layer"), "")).inner.clicked() {
                to_layer = Some(-1);
            }
        });
        crate::widgets::submenu(ui, i18n::t("Arrange"), |ui| {
            item(ui, has, i18n::t("Flip Horizontal"), "fliph");
            item(ui, has, i18n::t("Flip Vertical"), "flipv");
            item(ui, has, i18n::t("Rotate 90°"), "rotate");
            item(ui, has, i18n::t("Centre on Board"), "centre");
            crate::widgets::menu_separator(ui);
            item(ui, has, i18n::t("Align & Distribute…"), "align");
            item(ui, has, i18n::t("Array Copy…"), "array");
            item(ui, has, i18n::t("Nest on Board…"), "nest");
            item(ui, n == 2, i18n::t("Link Two Shapes…"), "link");
            item(ui, has && sheets > 1, i18n::t("Move to Next Sheet"), "sheet:next");
        });
        crate::widgets::submenu(ui, i18n::t("Combine"), |ui| {
            item(ui, n > 1, i18n::t("Weld"), "weld");
            item(ui, n > 1, i18n::t("Subtract"), "subtract");
            item(ui, n > 1, i18n::t("Intersect"), "intersect");
            crate::widgets::menu_separator(ui);
            item(ui, has, i18n::t("Offset…"), "offset");
            item(ui, has, i18n::t("Corners: Dogbone / Fillet…"), "dogbone");
            item(ui, n > 1, i18n::t("Join Open Vectors"), "join");
            item(ui, has, i18n::t("Close Vectors"), "close");
        });
        item(ui, true, i18n::t("Trim Here (to the crossings)"), "trim");
        crate::widgets::submenu(ui, i18n::t("Toolpath"), |ui| {
            for (label, id) in [(i18n::t("Profile…"), "kind:0"), (i18n::t("Pocket…"), "kind:1"), (i18n::t("Drill…"), "kind:2"), (i18n::t("V-Carve…"), "kind:3"), (i18n::t("Bore…"), "kind:4"), (i18n::t("Inlay…"), "kind:5"), (i18n::t("Flute…"), "kind:6"), (i18n::t("Chamfer…"), "kind:8"), (i18n::t("Prism…"), "kind:9"), (i18n::t("Texture…"), "kind:10"), (i18n::t("Photo V-Carve…"), "kind:11")] {
                item(ui, has, label, id);
            }
        });
        crate::widgets::menu_separator(ui);
        item(ui, true, i18n::t("Zoom to Fit"), "zoom:fit");
        item(ui, true, i18n::t("Zoom to Board"), "zoom:board");
        if let Some(id) = run {
            if id == "trim" {
                self.trim_here(); // needs the point the menu opened at
            } else {
                self.requests.push(Request::Cmd(id));
            }
            ui.close();
        }
        if let Some(i) = to_layer {
            if i < 0 {
                self.add_layer();
            }
            let target = if i < 0 { self.doc.active_layer } else { i }; // add_layer made the new one active
            self.move_selection_to_layer(target);
            ui.close();
        }
    }

    fn node_menu(&mut self, ui: &mut egui::Ui, shape: i32, node: i32) {
        let at = ui.ctx().input(|i| i.pointer.hover_pos()).unwrap_or_default();
        let smooth = self.doc.find(shape).and_then(|s| s.c.nodes.get(node as usize)).map(|n| n.has_in || n.has_out).unwrap_or(false);
        let mut close = false;
        if crate::widgets::menu_item(ui, i18n::t("Insert Node After"), "").clicked() {
            self.insert_node_after(shape, node);
            close = true;
        }
        if crate::widgets::menu_item(ui, i18n::t("Remove Node"), "").clicked() {
            self.remove_node(shape, node);
            close = true;
        }
        if crate::widgets::menu_item(ui, i18n::t("Make Start Point"), "").clicked() {
            self.make_start_point(shape, node);
            close = true;
        }
        if crate::widgets::menu_item(ui, if smooth { i18n::t("Make Corner") } else { i18n::t("Make Smooth") }, "").clicked() {
            self.toggle_node_smooth(shape, node);
            close = true;
        }
        crate::widgets::menu_separator(ui);
        if crate::widgets::menu_item(ui, i18n::t("Set Position…"), "").clicked() {
            self.open_node_popup(shape, node, at);
            close = true;
        }
        if close {
            ui.close();
        }
    }

    fn dim_menu(&mut self, ui: &mut egui::Ui, i: usize) {
        let at = ui.ctx().input(|i| i.pointer.hover_pos()).unwrap_or_default();
        let mut close = false;
        if crate::widgets::menu_item(ui, i18n::t("Edit Dimension…"), "").clicked() {
            self.open_dim_popup(i, at);
            close = true;
        }
        crate::widgets::menu_separator(ui);
        let cur = self.doc.dimensions.get(i).map_or(0, |d| d.axis);
        // the other two kinds (a tick glyph is missing from the web font)
        for (axis, label) in [(0, i18n::t("Make Aligned (straight distance)")), (1, i18n::t("Make Horizontal (X only)")), (2, i18n::t("Make Vertical (Y only)"))].into_iter().filter(|(a, _)| *a != cur) {
            if crate::widgets::menu_item(ui, label, "").clicked() {
                self.set_dimension_axis(i, axis);
                close = true;
            }
        }
        crate::widgets::menu_separator(ui);
        if crate::widgets::menu_item(ui, i18n::t("Delete Dimension"), "").clicked() {
            self.remove_dimension(i);
            close = true;
        }
        if close {
            ui.close();
        }
    }
}

/// Modeless vector-op dialogs owned by the app shell.
#[derive(Default)]
pub struct Dialogs {
    pub text: TextDialog,
    offset: Option<[String; 1]>,
    // OffsetDialog.qml: Outwards, Round, Keep original checked at first; later opens keep the last choice
    offset_inwards: bool,
    offset_sharp: bool,
    offset_replace: bool,
    dogbone: Option<[String; 1]>,
    dogbone_inside: bool,
    /// 0 dogbone, 1 T-bone, 2 fillet (rounded corners)
    corner_style: u8,
    /// fillet: 0 all corners, 1 outer (sticking out), 2 inner
    fillet_which: u8,
    array: Option<[String; 4]>, // gap x, gap y, centre x, centre y
    array_rows: i32,
    array_cols: i32,
    array_count: i32,
    /// 0 grid, 1 circular, 2 along the longest selected vector
    array_mode: u8,
    array_rotate: bool,
    pub align_open: bool,
    align_gap: String,
    nest: Option<[String; 2]>,
    nest_rot: usize,
    nest_overflow: bool,
    /// 0 bottom-left, 1 bottom-right, 2 top-left, 3 top-right
    nest_corner: u8,
    nest_mirror: bool,
    nest_boundary: bool,
    nest_filler: bool,
    /// Shelf-pin holes: diameter, pitch, inset, end margin texts
    pins: Option<[String; 4]>,
    /// rotary layout: starts, pitch, from X, to X
    rlayout: Option<[String; 4]>,
    /// Box Maker: width, depth, height, thickness, finger, play, bit texts + lid
    boxm: Option<([String; 7], bool)>,
    /// Jigsaw Puzzle: width, height, columns, rows texts + the knob seed
    puzzle: Option<([String; 4], u64)>,
    /// Inlay Wizard: depth, gap, margin texts + flat (end mill) instead of V-bit
    inlay: Option<([String; 3], bool)>,
    /// Living Hinge: column spacing, slot length, bridge texts + slots run up the board
    hinge: Option<([String; 3], bool)>,
    /// Stencil Bridges: width text + bridges run up and down
    stencil: Option<(String, bool)>,
    /// QR Code: text, size text, invert
    qr: Option<(String, String, bool)>,
    /// Plate Production: the list (one plate per line), first line = headings, gap text
    plates: Option<(String, bool, String)>,
    /// Vector Texture: spacing, amplitude, wavelength texts + angle, randomness
    texture: Option<[String; 3]>,
    texture_angle: f64,
    texture_vary: f64,
    /// Vector Selector: filter + larger / smaller texts
    selector: Option<[String; 2]>,
    sel_filter: crate::edit_ops::VecFilter,
}

impl Dialogs {
    pub fn request(&mut self, r: Request, canvas: &Canvas) {
        let doc = &canvas.doc;
        match r {
            Request::Offset => {
                // Qt distF.set(mm ? 3 : 0.125): a value in the project unit
                let mm = units::name(doc) == "mm";
                self.offset = Some([format!("{:.*}", units::decimals(doc), if mm { 3.0 } else { 0.125 })]);
            }
            Request::Dogbone => {
                self.dogbone = Some([units::text(doc, 0.25)]);
                self.dogbone_inside = true;
            }
            Request::Array => {
                let c = canvas.sel_rect;
                let (cx, cy) = if c.is_empty() { (doc.material.w / 2.0, doc.material.h / 2.0) } else { ((c.x0 + c.x1) / 2.0, (c.y0 + c.y1) / 2.0) };
                self.array = Some([units::text(doc, 0.5), units::text(doc, 0.5), units::text(doc, cx), units::text(doc, cy)]);
                self.array_rows = self.array_rows.max(2);
                self.array_cols = self.array_cols.max(2);
                self.array_count = self.array_count.max(6);
            }
            Request::Align => {
                self.align_open = true;
                if self.align_gap.is_empty() {
                    self.align_gap = units::text(doc, 0.5);
                }
            }
            Request::Nest => {
                self.nest = Some([units::text(doc, 0.25), units::text(doc, 0.5)]);
                self.nest_rot = 1;
                self.nest_overflow = true;
            }
            Request::Selector => self.selector = Some([String::new(), String::new()]),
            Request::RotaryLayout => self.rlayout = Some([String::from("1"), units::text(doc, 4.0), units::text(doc, 0.0), units::text(doc, doc.material.w)]),
            Request::Inlay => self.inlay = Some(([0.2, 0.1, 0.25].map(|v| units::text(doc, v)), false)),
            Request::Stencil => self.stencil = Some((units::text(doc, 0.125), true)),
            Request::Hinge => self.hinge = Some(([0.25, 1.5, 0.25].map(|v| units::text(doc, v)), true)),
            Request::Qr => self.qr = Some(("https://".into(), units::text(doc, 4.0), false)),
            Request::Puzzle => {
                let (w, h) = ((doc.material.w - 1.0).clamp(2.0, 16.0), (doc.material.h - 1.0).clamp(2.0, 12.0));
                self.puzzle = Some(([units::text(doc, w), units::text(doc, h), "5".into(), "4".into()], 7));
            }
            Request::BoxMaker => {
                let t = doc.material.t;
                let tool = doc.tools.iter().filter(|k| k.kind == gcarve_core::document::ToolType::EndMill).map(|k| k.diameter).fold(0.0, f64::max);
                let v = [8.0, 5.0, 3.5, t, (2.0 * t).max(0.5), 0.005, if tool > 0.0 { tool } else { 0.25 }];
                self.boxm = Some((v.map(|x| units::text(doc, x)), true));
            }
            Request::Pins => self.pins = Some([units::text(doc, 5.0 / 25.4), units::text(doc, 32.0 / 25.4), units::text(doc, 37.0 / 25.4), units::text(doc, 3.0)]),
            Request::Plates => self.plates = Some(("name,room\nAnn,101\nBo,102".into(), true, units::text(doc, 0.25))),
            Request::Texture => {
                self.texture = Some([units::text(doc, 0.25), units::text(doc, 0.06), units::text(doc, 1.5)]);
                if self.texture_vary == 0.0 {
                    self.texture_vary = 0.3;
                }
            }
            Request::Text(on_path) => self.text.open_new(canvas, on_path),
            Request::EditText(id) => self.text.open_for(canvas, id),
            Request::Cmd(_) => {} // taken by the app (App::ui)
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, canvas: &mut Canvas) {
        for r in std::mem::take(&mut canvas.requests) {
            self.request(r, canvas);
        }
        self.text.ui(ctx, canvas);
        let doc_unit = units::name(&canvas.doc);

        if let Some(mut f) = self.offset.take() {
            let mut keep = true;
            let mut ok = false;
            W::Dialog::new(i18n::t("Offset Vectors")).show(ctx, |ui| {
                egui::Grid::new("offset").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    ui.label(format!("Distance ({doc_unit})"));
                    units::num_field(ui, "d", &mut f[0], &canvas.doc, true, 110.0);
                    ui.end_row();
                    ui.label(i18n::t("Direction"));
                    ui.horizontal(|ui| {
                        W::radio_value(ui, &mut self.offset_inwards, false, i18n::t("Outwards"));
                        W::radio_value(ui, &mut self.offset_inwards, true, i18n::t("Inwards"));
                    });
                    ui.end_row();
                    ui.label(i18n::t("Corners"));
                    let mut round = !self.offset_sharp;
                    W::checkbox(ui, &mut round, i18n::t("Round (else sharp)"));
                    self.offset_sharp = !round;
                    ui.end_row();
                    ui.label("");
                    let mut keep_orig = !self.offset_replace;
                    W::checkbox(ui, &mut keep_orig, i18n::t("Keep original vectors"));
                    self.offset_replace = !keep_orig;
                    ui.end_row();
                });
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                match units::get(&canvas.doc, &f[0], true) {
                    Ok(d) => {
                        canvas.offset_selection(if self.offset_inwards { -d } else { d }, !self.offset_sharp, !self.offset_replace);
                        keep = false;
                    }
                    Err(e) => canvas.status = e,
                }
            }
            if keep {
                self.offset = Some(f);
            }
        }

        if let Some(mut f) = self.pins.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("Shelf-Pin Holes")).show(ctx, |ui| {
                egui::Grid::new("pins").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    for (i, label) in ["Hole diameter", i18n::t("Hole spacing"), i18n::t("In from the front and back edges"), "Clear of the ends"].iter().enumerate() {
                        ui.label(format!("{label} ({doc_unit})"));
                        units::num_field(ui, label, &mut f[i], &canvas.doc, true, 110.0);
                        ui.end_row();
                    }
                });
                ui.label(egui::RichText::new(i18n::t("Two rows of holes along each selected cabinet side (system 32: 5 mm holes 32 mm apart, 37 mm in). They are added as circles on the \"Shelf pins\" layer: drill them with a Drill toolpath on that layer.")).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                let v: Vec<Result<f64, String>> = f.iter().map(|t| units::get(&canvas.doc, t, true)).collect();
                match v.iter().find_map(|r| r.as_ref().err()) {
                    Some(e) => canvas.status = e.clone(),
                    None => {
                        let n: Vec<f64> = v.into_iter().map(Result::unwrap).collect();
                        canvas.shelf_pin_holes(n[0], n[1], n[2], n[3]);
                        keep = false;
                    }
                }
            }
            if keep {
                self.pins = Some(f);
            }
        }

        if let Some(mut f) = self.rlayout.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("Rotary Layout")).show(ctx, |ui| {
                egui::Grid::new("rlayout").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    for (i, label) in ["Starts (lines round the blank)", i18n::t("Pitch: length per turn, 0 = straight"), i18n::t("From X"), "To X"].iter().enumerate() {
                        ui.label(if i == 0 { label.to_string() } else { format!("{label} ({doc_unit})") });
                        units::num_field(ui, label, &mut f[i], &canvas.doc, i > 0, 110.0);
                        ui.end_row();
                    }
                });
                ui.label(egui::RichText::new(i18n::t("Lines on the unrolled blank: a spiral (barley twist, rope) or flutes along it, evenly round. A negative pitch twists the other way. Put a Flute or V-Carve toolpath on them.")).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                let v: Vec<Result<f64, String>> = f.iter().enumerate().map(|(i, t)| units::get(&canvas.doc, t, i > 0)).collect();
                match v.iter().find_map(|r| r.as_ref().err()) {
                    Some(e) => canvas.status = e.clone(),
                    None => {
                        let n: Vec<f64> = v.into_iter().map(Result::unwrap).collect();
                        canvas.rotary_layout(n[0].round().max(0.0) as usize, n[1], n[2], n[3]);
                        keep = false;
                    }
                }
            }
            if keep {
                self.rlayout = Some(f);
            }
        }

        if let Some((mut text, mut size, mut invert)) = self.qr.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("QR Code")).show(ctx, |ui| {
                ui.label(i18n::t("Text or web address"));
                ui.add(crate::widgets::field(&mut text).desired_width(320.0));
                ui.horizontal(|ui| {
                    ui.label(format!("Size ({doc_unit})"));
                    units::num_field(ui, "qr-size", &mut size, &canvas.doc, true, 110.0);
                });
                W::checkbox(ui, &mut invert, i18n::t("Light squares instead (pocket them: the dark squares stand up)"));
                ui.label(egui::RichText::new(i18n::t("A scannable code centred on the board. Carve it about 3 in or bigger in a light wood, fill the pockets with dark paint, and leave a plain border four squares wide. Test it with your phone in Simulate.")).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                match units::get(&canvas.doc, &size, true) {
                    Err(e) => canvas.status = e,
                    Ok(s) => {
                        canvas.qr_code(&text, s, invert);
                        keep = false;
                    }
                }
            }
            if keep {
                self.qr = Some((text, size, invert));
            }
        }

        if let Some((mut f, mut flat)) = self.inlay.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("Inlay Wizard")).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut flat, false, i18n::t("V-bit inlay"));
                    ui.selectable_value(&mut flat, true, i18n::t("Flat (end mill) inlay"));
                });
                egui::Grid::new("inlay").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    let labels = if flat { ["Pocket depth", i18n::t("Fit gap (per side)"), ""] } else { ["Pocket depth", i18n::t("Glue gap"), "Plug margin"] };
                    for (i, label) in labels.iter().enumerate().filter(|(_, l)| !l.is_empty()) {
                        ui.label(format!("{label} ({doc_unit})"));
                        units::num_field(ui, label, &mut f[i], &canvas.doc, true, 110.0);
                        ui.end_row();
                    }
                });
                let say = if flat { i18n::t("The selected shapes' corners are rounded to a 1/8 in end mill, pocketed here; the plug goes on a new sheet, mirrored and a fit gap smaller, cut 1/16 in deeper than the pocket. 0.005 in is a snug fit.") } else { i18n::t("The selected shapes are V-carved here to a flat floor; the plug goes on a new sheet, mirrored, carved from the glue gap down so it wedges in tight. A 1/4 in end mill clears the floors.") };
                ui.label(egui::RichText::new(say).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                let v: Vec<Result<f64, String>> = f.iter().map(|t| units::get(&canvas.doc, t, true)).collect();
                match v.iter().find_map(|r| r.as_ref().err()) {
                    Some(e) => canvas.status = e.clone(),
                    None => {
                        let n: Vec<f64> = v.into_iter().map(Result::unwrap).collect();
                        canvas.inlay(flat, n[0], n[1], n[2]);
                        keep = false;
                    }
                }
            }
            if keep {
                self.inlay = Some((f, flat));
            }
        }

        if let Some((mut f, mut along_y)) = self.stencil.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("Stencil Bridges")).show(ctx, |ui| {
                egui::Grid::new("stencil").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    ui.label(format!("{} ({doc_unit})", i18n::t("Bridge width")));
                    units::num_field(ui, "stencil-w", &mut f, &canvas.doc, true, 110.0);
                    ui.end_row();
                    ui.label(i18n::t("Bridges run"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut along_y, true, i18n::t("Up and down"));
                        ui.selectable_value(&mut along_y, false, i18n::t("Across"));
                    });
                    ui.end_row();
                });
                ui.label(egui::RichText::new(i18n::t("Every island inside the selected shapes (the middle of an O, A or R) gets a bridge through its centre, so it stays joined to the plate when the letters are cut out.")).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                match units::get(&canvas.doc, &f, true) {
                    Ok(w) if w > 0.0 => {
                        canvas.stencil_bridges(w, along_y);
                        keep = false;
                    }
                    Ok(_) => canvas.status = i18n::t("Stencil Bridges: the bridge width must be more than 0.").into(),
                    Err(e) => canvas.status = e,
                }
            }
            if keep {
                self.stencil = Some((f, along_y));
            }
        }

        if let Some((mut f, mut along_y)) = self.hinge.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("Living Hinge")).show(ctx, |ui| {
                egui::Grid::new("hinge").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    for (i, label) in ["Space between rows of slots", i18n::t("Slot length"), "Wood between slots"].iter().enumerate() {
                        ui.label(format!("{label} ({doc_unit})"));
                        units::num_field(ui, label, &mut f[i], &canvas.doc, true, 110.0);
                        ui.end_row();
                    }
                    ui.label(i18n::t("Slots run"));
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut along_y, true, i18n::t("Up the board"));
                        ui.selectable_value(&mut along_y, false, i18n::t("Across"));
                    });
                    ui.end_row();
                });
                ui.label(egui::RichText::new(i18n::t("Fills the selection's box (or the board, 1 in in from the edges) with staggered slots so the wood bends. Closer rows and shorter wood between slots bend tighter.")).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                let v: Vec<Result<f64, String>> = f.iter().map(|t| units::get(&canvas.doc, t, true)).collect();
                match v.iter().find_map(|r| r.as_ref().err()) {
                    Some(e) => canvas.status = e.clone(),
                    None => {
                        let n: Vec<f64> = v.into_iter().map(Result::unwrap).collect();
                        canvas.living_hinge(n[0], n[1], n[2], along_y);
                        keep = false;
                    }
                }
            }
            if keep {
                self.hinge = Some((f, along_y));
            }
        }

        if let Some((mut f, mut seed)) = self.puzzle.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("Jigsaw Puzzle")).show(ctx, |ui| {
                egui::Grid::new("puzzle").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    for (i, label) in ["Width", i18n::t("Height"), i18n::t("Pieces across"), "Pieces down"].iter().enumerate() {
                        ui.label(if i < 2 { format!("{label} ({doc_unit})") } else { label.to_string() });
                        units::num_field(ui, label, &mut f[i], &canvas.doc, i < 2, 110.0);
                        ui.end_row();
                    }
                    ui.label(i18n::t("Knob shapes"));
                    if ui.button(i18n::t("Shuffle")).tip(i18n::t("Other knob directions and shapes (the same puzzle comes back with the same number)")).clicked() {
                        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 1;
                    }
                    ui.end_row();
                });
                ui.label(egui::RichText::new(i18n::t("Interlocking pieces centred on the board, on the \"Puzzle\" layer. Cut them with a Profile on the line; a thin board (1/4 in) and a 1/16 in bit work well.")).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                let v: Vec<Result<f64, String>> = f.iter().enumerate().map(|(i, t)| units::get(&canvas.doc, t, i < 2)).collect();
                match v.iter().find_map(|r| r.as_ref().err()) {
                    Some(e) => canvas.status = e.clone(),
                    None => {
                        let n: Vec<f64> = v.into_iter().map(Result::unwrap).collect();
                        canvas.jigsaw(n[0], n[1], n[2].round().max(1.0) as usize, n[3].round().max(1.0) as usize, seed);
                        keep = false;
                    }
                }
            }
            if keep {
                self.puzzle = Some((f, seed));
            }
        }

        if let Some((mut f, mut lid)) = self.boxm.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("Box Maker")).show(ctx, |ui| {
                egui::Grid::new("boxm").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    for (i, label) in ["Width (outside)", i18n::t("Depth (outside)"), i18n::t("Height (outside)"), i18n::t("Board thickness"), i18n::t("Finger width"), i18n::t("Fit play (slots wider by)"), "Bit diameter (dogbones, 0 = none)"].iter().enumerate() {
                        ui.label(format!("{label} ({doc_unit})"));
                        units::num_field(ui, label, &mut f[i], &canvas.doc, true, 110.0);
                        ui.end_row();
                    }
                });
                W::checkbox(ui, &mut lid, i18n::t("Lid (jointed like the bottom)"));
                ui.label(egui::RichText::new(i18n::t("Six panels (five without a lid) with finger joints, laid flat on the \"Box\" layer. Measure the board: the thickness sets the finger depth. Cut them with a Profile outside, through the board.")).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                let v: Vec<Result<f64, String>> = f.iter().map(|t| units::get(&canvas.doc, t, true)).collect();
                match v.iter().find_map(|r| r.as_ref().err()) {
                    Some(e) => canvas.status = e.clone(),
                    None => {
                        let n: Vec<f64> = v.into_iter().map(Result::unwrap).collect();
                        canvas.box_maker(gcarve_core::boxjoint::BoxSpec { w: n[0], d: n[1], h: n[2], t: n[3], finger: n[4], play: n[5].max(0.0), lid, bit: n[6].max(0.0) });
                        keep = false;
                    }
                }
            }
            if keep {
                self.boxm = Some((f, lid));
            }
        }

        if let Some((mut list, mut heads, mut gap)) = self.plates.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("Plate Production")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(egui::RichText::new("Select the master plate (its outline, text, anything). In its text, {1} is a row's first field, {2} the second, or use a heading like {name}. One plate is made per line below, beside and then below the master.").small().color(crate::theme::TEXT_DIM));
                ui.add(egui::TextEdit::multiline(&mut list).desired_width(440.0).desired_rows(8).hint_text(i18n::t("Ann,101\nBo,102")));
                W::checkbox(ui, &mut heads, i18n::t("First line is headings"));
                ui.horizontal(|ui| {
                    ui.label(format!("Gap between plates ({doc_unit})"));
                    units::num_field(ui, "plgap", &mut gap, &canvas.doc, true, 110.0);
                });
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                // fields split on tabs (pasted from a spreadsheet) or commas
                let mut rows: Vec<Vec<String>> = list.lines().filter(|l| !l.trim().is_empty()).map(|l| l.split(if l.contains('\t') { '\t' } else { ',' }).map(str::to_string).collect()).collect();
                let headings = if heads && !rows.is_empty() { rows.remove(0) } else { Vec::new() };
                match units::get(&canvas.doc, &gap, true) {
                    Ok(g) => {
                        canvas.plate_production(&rows, &headings, g);
                        keep = false;
                    }
                    Err(e) => canvas.status = e,
                }
            }
            if keep {
                self.plates = Some((list, heads, gap));
            }
        }

        if let Some(mut f) = self.texture.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("Vector Texture")).show(ctx, |ui| {
                egui::Grid::new("vtexture").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    for (i, label) in ["Line spacing", i18n::t("Wave height"), "Wave length"].iter().enumerate() {
                        ui.label(format!("{label} ({doc_unit})"));
                        units::num_field(ui, label, &mut f[i], &canvas.doc, true, 110.0);
                        ui.end_row();
                    }
                    ui.label(i18n::t("Angle"));
                    ui.add(egui::DragValue::new(&mut self.texture_angle).range(0.0..=179.0).suffix("°"));
                    ui.end_row();
                    ui.label(i18n::t("Randomness"));
                    ui.add(egui::Slider::new(&mut self.texture_vary, 0.0..=1.0).fixed_decimals(2));
                    ui.end_row();
                });
                ui.label(egui::RichText::new(i18n::t("Fills the selected closed shapes with wavy lines (holes stay clear), added as one group. Cut them with a V-bit or ball nose: Profile on the line, or Flute.")).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                let v: Vec<Result<f64, String>> = f.iter().map(|t| units::get(&canvas.doc, t, true)).collect();
                match v.iter().find_map(|r| r.as_ref().err()) {
                    Some(e) => canvas.status = e.clone(),
                    None => {
                        let n: Vec<f64> = v.into_iter().map(Result::unwrap).collect();
                        canvas.texture_selection(n[0], n[1], n[2], self.texture_angle, self.texture_vary);
                        keep = false;
                    }
                }
            }
            if keep {
                self.texture = Some(f);
            }
        }

        if let Some(mut f) = self.selector.take() {
            let (mut keep, mut ok) = (true, false);
            W::Dialog::new(i18n::t("Vector Selector")).show(ctx, |ui| {
                egui::Grid::new("selector").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    ui.label(i18n::t("Kind"));
                    ui.horizontal(|ui| {
                        for (v, t) in [(0, i18n::t("Any")), (1, i18n::t("Open")), (2, i18n::t("Closed")), (3, i18n::t("Circles"))] {
                            W::radio_value(ui, &mut self.sel_filter.kind, v, t);
                        }
                    });
                    ui.end_row();
                    ui.label(format!("Larger than ({doc_unit})"));
                    units::num_field(ui, "lg", &mut f[0], &canvas.doc, true, 110.0);
                    ui.end_row();
                    ui.label(format!("Smaller than ({doc_unit})"));
                    units::num_field(ui, "sm", &mut f[1], &canvas.doc, true, 110.0);
                    ui.end_row();
                    ui.label(i18n::t("Layer"));
                    let name = |l: Option<i32>| l.and_then(|i| canvas.doc.layers.get(i as usize)).map_or(i18n::t("Any layer").to_string(), |l| l.name.clone());
                    egui::ComboBox::from_id_salt("sel_layer").selected_text(name(self.sel_filter.layer)).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sel_filter.layer, None, i18n::t("Any layer"));
                        for i in 0..canvas.doc.layers.len() as i32 {
                            ui.selectable_value(&mut self.sel_filter.layer, Some(i), name(Some(i)));
                        }
                    });
                    ui.end_row();
                });
                ui.label(egui::RichText::new(i18n::t("Size = the longest side. Leave a size blank for no limit. Only this sheet's unlocked vectors are picked.")).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                let size = |t: &str| if t.trim().is_empty() { Ok(None) } else { units::get(&canvas.doc, t, true).map(Some) };
                match (size(&f[0]), size(&f[1])) {
                    (Ok(larger), Ok(smaller)) => {
                        canvas.select_matching(&crate::edit_ops::VecFilter { larger, smaller, ..self.sel_filter.clone() });
                        keep = false;
                    }
                    (Err(e), _) | (_, Err(e)) => canvas.status = e,
                }
            }
            if keep {
                self.selector = Some(f);
            }
        }

        if let Some(mut f) = self.dogbone.take() {
            let mut keep = true;
            let mut ok = false;
            let fillet = self.corner_style == 2;
            W::Dialog::new(i18n::t("Corners")).show(ctx, |ui| {
                egui::Grid::new("dogbone").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    ui.label(i18n::t("Style"));
                    ui.horizontal(|ui| {
                        W::radio_value(ui, &mut self.corner_style, 0, i18n::t("Dogbone"));
                        W::radio_value(ui, &mut self.corner_style, 1, i18n::t("T-bone"));
                        W::radio_value(ui, &mut self.corner_style, 2, i18n::t("Fillet (round)"));
                    });
                    ui.end_row();
                    ui.label(if fillet { format!("Radius ({doc_unit})") } else { format!("Bit diameter ({doc_unit})") });
                    units::num_field(ui, "d", &mut f[0], &canvas.doc, true, 110.0);
                    ui.end_row();
                    if fillet {
                        ui.label(i18n::t("Corners"));
                        ui.horizontal(|ui| {
                            W::radio_value(ui, &mut self.fillet_which, 0, i18n::t("All"));
                            W::radio_value(ui, &mut self.fillet_which, 1, i18n::t("Sticking out"));
                            W::radio_value(ui, &mut self.fillet_which, 2, i18n::t("Going in"));
                        });
                    } else {
                        ui.label(i18n::t("Vectors are"));
                        ui.horizontal(|ui| {
                            W::radio_value(ui, &mut self.dogbone_inside, true, i18n::t("Holes / pockets"));
                            W::radio_value(ui, &mut self.dogbone_inside, false, i18n::t("Parts (cut outside)"));
                        });
                    }
                    ui.end_row();
                });
                ui.label(egui::RichText::new(if fillet { i18n::t("Rounds the sharp corners of closed vectors. A going-in corner rounded to your bit's radius is one the bit can cut exactly.") } else { i18n::t("Adds a small lobe at each corner the round bit can't reach, so square parts fit.") }).small().color(crate::theme::TEXT_DIM));
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                match units::get(&canvas.doc, &f[0], true) {
                    Ok(d) => {
                        match self.corner_style {
                            2 => canvas.fillet_selection(d, self.fillet_which != 2, self.fillet_which != 1),
                            s => canvas.dogbone_selection(d, self.dogbone_inside, s == 1),
                        }
                        keep = false;
                    }
                    Err(e) => canvas.status = e,
                }
            }
            if keep {
                self.dogbone = Some(f);
            }
        }

        if let Some(mut f) = self.array.take() {
            let mut keep = true;
            let mut ok = false;
            W::Dialog::new(i18n::t("Array Copy")).show(ctx, |ui| {
                egui::Grid::new("array").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    ui.label(i18n::t("Layout"));
                    ui.horizontal(|ui| {
                        W::radio_value(ui, &mut self.array_mode, 0, i18n::t("Grid"));
                        W::radio_value(ui, &mut self.array_mode, 1, i18n::t("Circular"));
                        W::radio_value(ui, &mut self.array_mode, 2, i18n::t("Along a vector"));
                    });
                    ui.end_row();
                    if self.array_mode == 2 {
                        ui.label(i18n::t("Copies"));
                        ui.add(W::SpinBox::new(&mut self.array_count, 2..=1000));
                        ui.end_row();
                        ui.label("");
                        W::checkbox(ui, &mut self.array_rotate, i18n::t("Turn copies to follow the vector"));
                        ui.end_row();
                    } else if self.array_mode == 0 {
                        ui.label(i18n::t("Columns (X)"));
                        ui.add(W::SpinBox::new(&mut self.array_cols, 1..=200));
                        ui.end_row();
                        ui.label(i18n::t("Rows (Y)"));
                        ui.add(W::SpinBox::new(&mut self.array_rows, 1..=200));
                        ui.end_row();
                        ui.label(format!("Gap X ({doc_unit})"));
                        units::num_field(ui, "gx", &mut f[0], &canvas.doc, true, 110.0);
                        ui.end_row();
                        ui.label(format!("Gap Y ({doc_unit})"));
                        units::num_field(ui, "gy", &mut f[1], &canvas.doc, true, 110.0);
                        ui.end_row();
                    } else {
                        ui.label(i18n::t("Copies"));
                        ui.add(W::SpinBox::new(&mut self.array_count, 2..=360));
                        ui.end_row();
                        ui.label(format!("Centre X ({doc_unit})"));
                        units::num_field(ui, "cx", &mut f[2], &canvas.doc, true, 110.0);
                        ui.end_row();
                        ui.label(format!("Centre Y ({doc_unit})"));
                        units::num_field(ui, "cy", &mut f[3], &canvas.doc, true, 110.0);
                        ui.end_row();
                        ui.label("");
                        W::checkbox(ui, &mut self.array_rotate, i18n::t("Rotate copies"));
                        ui.end_row();
                    }
                });
                let note = match self.array_mode {
                    0 => i18n::t("Gap is the space between copies, edge to edge."),
                    2 => i18n::t("Select the object(s) and the vector to follow: the longest selected vector is the path, the rest is copied evenly along it (an open path gets a copy at both ends)."),
                    _ => "",
                };
                if !note.is_empty() {
                    ui.add(egui::Label::new(RichText::new(note).color(crate::theme::TEXT.gamma_multiply(0.6))).wrap());
                }
                let (o, c) = W::ok_cancel(ui);
                ok |= o;
                keep &= !c;
            });
            if ok {
                let v: Vec<Result<f64, String>> = f.iter().map(|t| units::get(&canvas.doc, t, true)).collect();
                match v.iter().find(|r| r.is_err()) {
                    Some(Err(e)) => canvas.status = e.clone(),
                    _ => {
                        let v: Vec<f64> = v.into_iter().map(|r| r.unwrap()).collect();
                        match self.array_mode {
                            1 => canvas.array_circular(self.array_count, pt(v[2], v[3]), self.array_rotate),
                            2 => canvas.array_along(self.array_count, self.array_rotate),
                            _ => canvas.array_copy(self.array_rows, self.array_cols, v[0], v[1]),
                        }
                        keep = false;
                    }
                }
            }
            if keep {
                self.array = Some(f);
            }
        }

        if self.align_open {
            let mut open = true;
            W::Dialog::new(i18n::t("Align & Distribute")).open(&mut open).close_footer().modeless(ctx.content_rect().right_top() + egui::vec2(-330.0, 120.0), egui::Align2::RIGHT_TOP).show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 10.0);
                let n = canvas.sel.len();
                let bold = |t: &str| RichText::new(t).font(W::bold(crate::theme::FONT_SIZE));
                ui.label(RichText::new(format!("{n} selected")).color(crate::theme::TEXT.gamma_multiply(0.7)));
                ui.label(bold(i18n::t("Align to each other")));
                egui::Grid::new("align-each").spacing([6.0, 6.0]).show(ui, |ui| {
                    for (i, (label, mode, icon)) in [(i18n::t("Left"), "left", "align-left"), (i18n::t("Centre"), "centerH", "arrows-h"), (i18n::t("Right"), "right", "align-right"), (i18n::t("Top"), "top", "align-top"), (i18n::t("Middle"), "centerV", "arrows-v"), (i18n::t("Bottom"), "bottom", "align-bottom")].into_iter().enumerate() {
                        // Qt "⇤ Left" … "Right ⇥": the arrows are icons (the web fonts lack ⤒)
                        if ui.add_enabled(n >= 2, W::Button::new(label).icon(icon, label == "Right").min_width(100.0)).clicked() {
                            canvas.align_objects(mode);
                        }
                        if i % 3 == 2 {
                            ui.end_row();
                        }
                    }
                });
                ui.label(bold(i18n::t("Align to the board")));
                egui::Grid::new("align-board").spacing([6.0, 6.0]).show(ui, |ui| {
                    let cells = [("top-left", "left", "top"), ("top", "centerH", "top"), ("top-right", "right", "top"), ("left", "left", "centerV"), ("centre", "center", ""), ("right", "right", "centerV"), ("bottom-left", "left", "bottom"), ("bottom", "centerH", "bottom"), ("bottom-right", "right", "bottom")];
                    for (i, (label, a, b)) in cells.into_iter().enumerate() {
                        if ui.add_enabled(n >= 1, W::Button::new(label).min_width(100.0)).clicked() {
                            canvas.align_selection(a);
                            if !b.is_empty() {
                                canvas.align_selection(b);
                            }
                        }
                        if i % 3 == 2 {
                            ui.end_row();
                        }
                    }
                });
                ui.label(bold(i18n::t("Distribute")));
                ui.horizontal(|ui| {
                    if ui.add_enabled(n >= 3, W::Button::new(i18n::t("Evenly ↔"))).clicked() {
                        canvas.distribute_objects(true, -1.0);
                    }
                    if ui.add_enabled(n >= 3, W::Button::new(i18n::t("Evenly ↕"))).clicked() {
                        canvas.distribute_objects(false, -1.0);
                    }
                    ui.label(i18n::t("  Gap"));
                    units::num_field(ui, "gap", &mut self.align_gap, &canvas.doc, true, 80.0);
                    for (label, h) in [(i18n::t("Space ↔"), true), (i18n::t("Space ↕"), false)] {
                        if ui.add_enabled(n >= 2, W::Button::new(label)).clicked() {
                            match units::get(&canvas.doc, &self.align_gap, true) {
                                Ok(g) => canvas.distribute_objects(h, g),
                                Err(e) => canvas.status = e,
                            }
                        }
                    }
                });
                ui.label(RichText::new(i18n::t("Gap may use variables (e.g. gap). Items keep their order; the first one stays put.")).size(11.0).color(crate::theme::TEXT.gamma_multiply(0.5)));
            });
            self.align_open = open;
        }

        if let Some(mut f) = self.nest.take() {
            let mut keep = true;
            let mut ok = false;
            W::Dialog::new(i18n::t("Nest on Board")).show(ctx, |ui| {
                ui.set_max_width(400.0);
                ui.add(egui::Label::new(RichText::new(i18n::t("True-shape nesting: parts are packed by their real outlines from the chosen corner, largest first, so curved and angled parts interlock. Shapes inside another selected shape (holes, inner cuts) move with it.")).color(crate::theme::TEXT.gamma_multiply(0.6))).wrap());
                egui::Grid::new("nest").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    ui.label(format!("Gap between parts ({doc_unit})"));
                    units::num_field(ui, "gap", &mut f[0], &canvas.doc, true, 110.0);
                    ui.end_row();
                    ui.label(format!("Margin from board edge ({doc_unit})"));
                    units::num_field(ui, "margin", &mut f[1], &canvas.doc, true, 110.0);
                    ui.end_row();
                    ui.label(i18n::t("Rotations to try"));
                    let items = ["None", i18n::t("90° steps"), i18n::t("45° steps"), "15° steps (slower)"];
                    W::combo_box("rot").selected_text(items[self.nest_rot]).width(200.0).show_ui(ui, |ui| {
                        for (i, l) in items.iter().enumerate() {
                            ui.selectable_value(&mut self.nest_rot, i, *l);
                        }
                    });
                    ui.end_row();
                    ui.label(i18n::t("Nest from"));
                    let corners = ["Bottom-left", i18n::t("Bottom-right"), i18n::t("Top-left"), "Top-right"];
                    W::combo_box("corner").selected_text(corners[self.nest_corner as usize]).width(200.0).show_ui(ui, |ui| {
                        for (i, l) in corners.iter().enumerate() {
                            ui.selectable_value(&mut self.nest_corner, i as u8, *l);
                        }
                    });
                    ui.end_row();
                });
                W::checkbox(ui, &mut self.nest_boundary, i18n::t("Inside the largest selected shape (an offcut)")).tip(i18n::t("Draw or trace the offcut's outline, select it with the parts: it stays where it is and the parts nest inside it"));
                W::checkbox(ui, &mut self.nest_filler, i18n::t("Fill the rest with copies of the smallest part")).tip(i18n::t("Filler part: after the selection is placed, copies of its smallest part go wherever they still fit"));
                W::checkbox(ui, &mut self.nest_mirror, i18n::t("Mirror parts for a better fit")).tip(i18n::t("Only for parts that can be cut either way round (no text, no one-sided edge profile)"));
                W::checkbox(ui, &mut self.nest_overflow, i18n::t("Overflow onto more sheets")).tip(i18n::t("Parts that don't fit go to the next sheet (a new one is added with the same board)"));
                ui.label(RichText::new(if self.nest_overflow { i18n::t("Parts that don't fit this sheet are nested on following sheets.") } else { i18n::t("One sheet; parts that don't fit are left where they are.") }).size(11.0).color(crate::theme::TEXT.gamma_multiply(0.5)));
                let (o, c) = W::ok_cancel(ui);
                ok |= o && !canvas.sel.is_empty();
                keep &= !c;
            });
            if ok {
                match (units::get(&canvas.doc, &f[0], true), units::get(&canvas.doc, &f[1], true)) {
                    (Ok(g), Ok(m)) => {
                        let rot = [0, 90, 45, 15][self.nest_rot];
                        canvas.nest_mirror = self.nest_mirror;
                        canvas.nest_boundary = self.nest_boundary;
                        canvas.nest_filler = self.nest_filler;
                        if self.nest_overflow {
                            canvas.nest_selection_sheets(g, m, rot, self.nest_corner);
                        } else {
                            canvas.nest_selection(g, m, rot, self.nest_corner);
                        }
                        keep = false;
                    }
                    (Err(e), _) | (_, Err(e)) => canvas.status = e,
                }
            }
            if keep {
                self.nest = Some(f);
            }
        }
    }
}
