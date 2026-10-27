//! New / Edit toolpath for every kind (Qt ProfileDialog.qml): profile, pocket, drill, v-carve, bore, inlay, flute,
//! 3D model, chamfer, prism. Length fields take expressions (variables stay bound in `Op::bind`).

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::widgets::TipExt;
use super::{combo, hint, tool_advice_ui, tool_combo, unit_label, Fields, Host};
use eframe::egui::{self, RichText};
use gcarve_core::document::{Document, Entry, Op, OpType, Side, ToolType};
use gcarve_core::geom::{pt, Pt};

pub const KIND_NAMES: [&str; 13] = ["Profile", "Pocket", "Drill", "V-Carve", "Bore", "Inlay", "Flute", "3D Model", "Chamfer", "Prism", "Imported", "Texture", "Photo V-Carve"]; // i18n

pub fn kind_name(k: OpType) -> &'static str {
    i18n::t(KIND_NAMES[k as usize])
}

const BIND_KEYS: [&str; 14] = ["depth", "stepdown", "tabWidth", "tabHeight", "allowance", "peck", "flatDepth", "startDepth", "glueGap", "inlayMargin", "rampIn", "rampOut", "rampLength", "chamferWidth"];

pub struct OpDialog {
    pub op: Op,
    pub index: Option<usize>,
    f: Fields,
    flat: bool,
    peck: bool,
    /// 0 selected shapes, 1 layer, 2 group
    scope: i32,
    scope_layer: i32,
    scope_group: i32,
    stepover_pct: i32,
    rough_pct: i32,
    rough_step: String,
    tab_count: i32,
    /// tabs one every so far (`Op::tab_spacing`) instead of a count per part
    by_spacing: bool,
    /// the hand-placed tab being dragged along the outline
    drag_tab: Option<usize>,
    /// Profile: clicks on the canvas place / remove tabs at the part's outline (Canvas::tab_pick)
    place_tabs: bool,
    /// chamfer: sized by width (else by V depth)
    by_width: bool,
    /// Bits… pressed this frame (Dialogs opens the Tool Library)
    pub open_bits: bool,
    /// Selector… pressed: the Vector Selector opens (its pick becomes this new toolpath's shapes)
    pub open_selector: bool,
    /// Photo: Picture… pressed this frame (Dialogs opens the file picker)
    pub pick_photo: bool,
    /// tool count when the library was opened from here: on close the bit added there is picked
    pub tools_before: Option<usize>,
}

/// Qt Canvas::newProfileParams defaults.
pub fn new_op(doc: &Document, kind: OpType, selected: &[i32]) -> Op {
    let mut op = Op { kind, tab_corners: true, ..Default::default() };
    let find = |k: ToolType| doc.tools.iter().position(|t| t.kind == k).map(|i| i as i32);
    match kind {
        OpType::Relief => {
            op.stepover = 0.1;
            op.rough = true;
            op.rough_stepdown = 0.2;
            op.rough_stepover = 0.5;
            op.allowance = 0.02;
            op.relief = doc.reliefs.last().map(|r| r.id).unwrap_or(-1);
            op.tool = find(ToolType::BallNose).unwrap_or(0);
            op.clear_tool = find(ToolType::EndMill).unwrap_or(0);
        }
        OpType::Flute => {
            op.ramp_in = 1.0;
            op.ramp_out = 1.0;
        }
        OpType::VCarve | OpType::Inlay | OpType::Chamfer | OpType::Prism => op.tool = find(ToolType::VBit).unwrap_or(0),
        _ => {}
    }
    if kind == OpType::Inlay {
        op.flat_depth = 0.25f64.min(doc.material.t / 2.0);
        op.glue_gap = op.flat_depth / 2.0;
    }
    op.name = format!("{} {}", kind_name(kind), doc.ops.len() + 1);
    op.depth = match kind {
        OpType::Pocket => doc.material.t / 2.0,
        OpType::Flute => 0.25f64.min(doc.material.t / 2.0),
        _ => doc.material.t,
    };
    if kind == OpType::Chamfer {
        op.chamfer_width = 0.125;
        op.depth = 0.125;
        op.side = Side::Inside;
    }
    if kind == OpType::Photo {
        op.tool = find(ToolType::VBit).unwrap_or(op.tool);
        op.depth = 0.06;
        op.raster_angle = 0;
    }
    if kind == OpType::Texture {
        op.tool = find(ToolType::BallNose).or_else(|| find(ToolType::VBit)).unwrap_or(op.tool);
        op.depth = 0.06;
        op.ramp_length = 0.6;
        op.stepover = 0.8;
        op.vary = 0.5;
        op.raster_angle = 0;
    }
    if kind == OpType::Prism {
        op.depth = 0.25f64.min(doc.material.t / 2.0);
    }
    op.shape_ids = selected.to_vec();
    op
}

impl OpDialog {
    pub fn new(doc: &Document, kind: OpType, selected: &[i32]) -> Self {
        Self::from_op(doc, new_op(doc, kind, selected), None, selected)
    }
    /// A new toolpath for everything on `layer` (Layers panel ▸ + Toolpath): shapes added to the layer later join it.
    pub fn new_for_layer(doc: &Document, kind: OpType, layer: i32) -> Self {
        let ids: Vec<i32> = doc.shapes.iter().filter(|s| s.layer == layer && s.sheet == doc.active_sheet).map(|s| s.id).collect();
        let mut op = new_op(doc, kind, &ids);
        op.shape_ids.clear();
        op.layer_scope = layer;
        if let Some(l) = usize::try_from(layer).ok().and_then(|i| doc.layers.get(i)) {
            op.name = format!("{} {}", l.name, kind_name(kind));
        }
        Self::from_op(doc, op, None, &ids)
    }
    /// Surface the board: a raster Pocket over `shape` with the widest end mill, shallow, along the board's long side
    pub fn surface(doc: &Document, shape: i32) -> Self {
        let mut op = new_op(doc, OpType::Pocket, &[shape]);
        op.name = i18n::t("Surface the board").into();
        op.raster = true;
        op.raster_angle = if doc.material.h > doc.material.w { 90 } else { 0 };
        op.depth = 0.04;
        op.stepdown = op.depth;
        op.stepover = 0.8;
        let widest = doc.tools.iter().enumerate().filter(|(_, t)| t.kind == ToolType::EndMill).max_by(|a, b| a.1.diameter.total_cmp(&b.1.diameter));
        if let Some((i, _)) = widest {
            op.tool = i as i32;
        }
        Self::from_op(doc, op, None, &[shape])
    }
    pub fn edit(doc: &Document, i: usize, selected: &[i32]) -> Self {
        Self::from_op(doc, doc.ops[i].clone(), Some(i), selected)
    }
    fn from_op(doc: &Document, op: Op, index: Option<usize>, selected: &[i32]) -> Self {
        let sel = selected.first().and_then(|&id| doc.find(id));
        let mut f = Fields::new(doc);
        let b = &op.bind;
        f.set_bound("depth", b, op.depth);
        f.set_bound("stepdown", b, op.stepdown);
        f.set_bound("tabWidth", b, op.tab_width);
        f.set_bound("tabHeight", b, op.tab_height);
        f.set_bound("allowance", b, op.allowance);
        f.set_bound("threadPitch", b, op.thread_pitch);
        f.set_bound("countersink", b, op.countersink);
        f.set_bound("skin", b, op.skin);
        f.set_bound("peck", b, op.peck);
        f.set_bound("flatDepth", b, if op.flat_depth > 0.0 { op.flat_depth } else { op.depth / 2.0 });
        f.set_bound("startDepth", b, op.start_depth);
        f.set_bound("glueGap", b, op.glue_gap);
        f.set_bound("inlayMargin", b, op.inlay_margin);
        f.set_bound("rampIn", b, op.ramp_in);
        f.set("dragKnife", if op.drag_knife > 0.0 { op.drag_knife } else { 0.25 });
        f.set("rounding", if op.rounding > 0.0 { op.rounding } else { doc.material.rotary.unwrap_or(3.0) });
        f.set_bound("rampOut", b, op.ramp_out);
        f.set_bound("rampLength", b, op.ramp_length);
        f.set_bound("chamferWidth", b, if op.chamfer_width > 0.0 { op.chamfer_width } else { 0.125 });
        f.set_bound("tabSpacing", b, op.tab_spacing);
        f.set_bound("arrayDx", b, op.array_dx);
        f.set_bound("arrayDy", b, op.array_dy);
        let scope = if op.all_vectors { 3 } else if op.layer_scope >= 0 { 1 } else if op.group_scope > 0 { 2 } else { 0 };
        OpDialog {
            flat: op.flat_depth > 0.0 || op.kind == OpType::Inlay,
            peck: op.peck > 0.0,
            scope,
            scope_layer: if scope == 1 { op.layer_scope } else { sel.map(|s| s.layer).unwrap_or(doc.active_layer) },
            scope_group: if scope == 2 { op.group_scope } else { sel.map(|s| s.group).unwrap_or(0) },
            stepover_pct: (op.stepover * 100.0).round() as i32,
            rough_pct: (op.rough_stepover * 100.0).round() as i32,
            rough_step: crate::units::text(doc, op.rough_stepdown),
            tab_count: op.tab_count,
            by_spacing: op.tab_spacing > 0.0,
            by_width: op.chamfer_width > 0.0 || op.kind != OpType::Chamfer,
            open_bits: false,
            open_selector: false,
            pick_photo: false,
            place_tabs: false,
            drag_tab: None,
            tools_before: None,
            f,
            op,
            index,
        }
    }

    /// The dialog's result applied to `op` (Qt opFromMap).
    fn apply(&self, doc: &Document) -> Op {
        let mut op = self.op.clone();
        let f = &self.f;
        let k = op.kind;
        op.tool = op.tool.clamp(0, doc.tools.len() as i32 - 1);
        op.depth = f.get_or("depth", doc, op.depth).max(0.0);
        op.stepdown = f.get_or("stepdown", doc, op.stepdown).max(0.0);
        op.ramp_length = f.get_or("rampLength", doc, op.ramp_length).max(0.0);
        op.layer_scope = if self.scope == 1 { self.scope_layer.clamp(0, doc.layers.len() as i32 - 1) } else { -1 };
        op.group_scope = if self.scope == 2 { self.scope_group } else { 0 };
        op.all_vectors = self.scope == 3;
        op.sheet_scope = op.sheet_scope.clamp(-1, doc.sheets.len() as i32 - 1);
        op.tab_count = self.tab_count.max(1);
        op.tab_spacing = if self.by_spacing { f.get_or("tabSpacing", doc, op.tab_spacing).max(0.0) } else { 0.0 };
        op.tab_width = f.get_or("tabWidth", doc, op.tab_width).max(0.0);
        op.tab_height = f.get_or("tabHeight", doc, op.tab_height).max(0.0);
        op.stepover = if k == OpType::Texture { (self.stepover_pct as f64 / 100.0).clamp(0.2, 3.0) } else if k == OpType::Relief { (self.stepover_pct as f64 / 100.0).clamp(0.02, 1.0) } else if k == OpType::Pocket && op.adaptive { (self.stepover_pct as f64 / 100.0).clamp(0.03, 0.5) } else { (self.stepover_pct as f64 / 100.0).clamp(0.05, 1.0) };
        op.allowance = f.get_or("allowance", doc, op.allowance).max(0.0);
        op.skin = if k == OpType::Profile { f.get_or("skin", doc, op.skin).max(0.0) } else { 0.0 };
        op.fit_holes = k == OpType::Drill && op.fit_holes;
        op.countersink = if k == OpType::Drill { f.get_or("countersink", doc, op.countersink).max(0.0) } else { 0.0 };
        op.thread_pitch = if k == OpType::Bore { f.get_or("threadPitch", doc, op.thread_pitch).max(0.0) } else { 0.0 };
        op.peck = if self.peck { f.get_or("peck", doc, op.peck).max(0.0) } else { 0.0 };
        let v_like = k == OpType::VCarve || k == OpType::Inlay;
        op.flat_depth = if v_like && (self.flat || k == OpType::Inlay) { f.get_or("flatDepth", doc, op.flat_depth).max(0.0) } else { 0.0 };
        if v_like && !(self.flat || k == OpType::Inlay) {
            op.clear_tool = -1;
        }
        op.chamfer_width = if k == OpType::Chamfer && self.by_width { f.get_or("chamferWidth", doc, op.chamfer_width).max(0.0) } else { 0.0 };
        op.start_depth = if k == OpType::VCarve || k == OpType::Chamfer { f.get_or("startDepth", doc, op.start_depth).max(0.0) } else { 0.0 };
        op.glue_gap = f.get_or("glueGap", doc, op.glue_gap).max(0.0);
        op.inlay_margin = f.get_or("inlayMargin", doc, op.inlay_margin).max(0.0);
        op.ramp_in = f.get_or("rampIn", doc, op.ramp_in).max(0.0);
        if op.drag_knife > 0.0 {
            op.drag_knife = f.get_or("dragKnife", doc, op.drag_knife).max(0.001);
        }
        if op.rounding > 0.0 {
            op.rounding = f.get_or("rounding", doc, op.rounding).max(0.01);
        }
        op.ramp_out = f.get_or("rampOut", doc, op.ramp_out).max(0.0);
        op.rough = k == OpType::Relief && op.rough;
        op.rough_stepdown = super::eval_len(&self.rough_step, doc).unwrap_or(op.rough_stepdown).max(0.0);
        op.rough_stepover = (self.rough_pct as f64 / 100.0).clamp(0.05, 1.0);
        op.raster_angle = if matches!(k, OpType::Texture | OpType::Photo) || (k == OpType::Pocket && op.raster) { op.raster_angle.rem_euclid(180) } else if op.raster_angle == 90 { 90 } else { 0 };
        op.array_rows = op.array_rows.clamp(1, 500);
        op.array_cols = op.array_cols.clamp(1, 500);
        op.array_dx = f.get_or("arrayDx", doc, op.array_dx);
        op.array_dy = f.get_or("arrayDy", doc, op.array_dy);
        op.bind = f.bindings(&BIND_KEYS);
        op
    }

    /// false = closed
    /// Photo V-Carve: the picked picture (8-bit gray, top row first)
    pub fn set_photo(&mut self, gray: &[u8], w: usize, h: usize) {
        self.op.photo = gcarve_core::photo::b64_encode(gray);
        (self.op.photo_w, self.op.photo_h) = (w as u32, h as u32);
    }

    /// The toolpath as currently set (for the live preview): its index when editing, None = new; None when it has no shapes yet
    pub fn draft(&self, doc: &Document) -> Option<(Option<usize>, Op)> {
        let op = self.apply(doc);
        (matches!(op.kind, OpType::Relief | OpType::Imported) || !doc.op_shape_ids(&op).is_empty()).then_some((self.index, op))
    }

    /// The nearest point on the outline of a shape this toolpath cuts: (distance, point).
    fn on_outline(&self, canvas: &crate::canvas::Canvas, c: Pt) -> Option<(f64, Pt)> {
        let doc = &canvas.doc;
        let mut best: Option<(f64, Pt)> = None;
        for id in doc.op_shape_ids(&self.apply(doc)) {
            let Some(s) = doc.find(id) else { continue };
            let h = gcarve_core::geom::nearest(&s.c, c);
            if best.is_none_or(|(d, _)| h.d < d) {
                best = Some((h.d, gcarve_core::geom::point_on(&s.c, &h)));
            }
        }
        best
    }

    /// By-hand tabs: while the box is ticked the canvas hands its clicks over (not selecting anything); a click near the
    /// outline of the shapes this toolpath cuts adds a tab there, a click on a tab takes it away.
    fn tab_clicks(&mut self, host: &mut Host) {
        let on = self.place_tabs && self.op.tabs && self.op.kind == OpType::Profile;
        host.canvas.tab_marks = if self.op.kind == OpType::Profile && self.op.tabs { self.op.tab_points.clone() } else { Vec::new() };
        if !on {
            host.canvas.tab_clicks.clear();
            return;
        }
        host.canvas.tab_pick = 3;
        // a tab dragged along: it follows the pointer, kept on the outline
        if let Some((start, now, done)) = host.canvas.tab_drag {
            if self.drag_tab.is_none() {
                self.drag_tab = self.op.tab_points.iter().position(|p| gcarve_core::geom::dist(pt(p[0], p[1]), start) < 0.3).or(Some(usize::MAX));
            }
            if let Some(i) = self.drag_tab.filter(|&i| i < self.op.tab_points.len()) {
                if let Some((d, q)) = self.on_outline(host.canvas, now) {
                    if d < 1.5 {
                        self.op.tab_points[i] = [q.x, q.y];
                    }
                }
            }
            if done {
                self.drag_tab = None;
                host.canvas.tab_drag = None;
            }
        }
        let clicks = std::mem::take(&mut host.canvas.tab_clicks);
        for c in clicks {
            if let Some(i) = self.op.tab_points.iter().position(|p| gcarve_core::geom::dist(pt(p[0], p[1]), c) < 0.3) {
                self.op.tab_points.remove(i);
                continue;
            }
            match self.on_outline(host.canvas, c) {
                Some((d, q)) if d < 1.5 => self.op.tab_points.push([q.x, q.y]),
                _ => *host.message = i18n::t("Click on the outline of a part this toolpath cuts").into(),
            }
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut keep = true;
        let mut ok = false;
        self.tab_clicks(host);
        let doc = &host.canvas.doc;
        let k = self.op.kind;
        let (is_profile, is_pocket, is_drill, is_vcarve, is_bore, is_inlay, is_flute, is_relief) = (k == OpType::Profile, k == OpType::Pocket, k == OpType::Drill, k == OpType::VCarve, k == OpType::Bore, k == OpType::Inlay, k == OpType::Flute, k == OpType::Relief);
        let v_like = is_vcarve || is_inlay;
        let (is_chamfer, is_prism) = (k == OpType::Chamfer, k == OpType::Prism);
        let fixed = k == OpType::Imported;
        let needs_v = v_like || is_chamfer || is_prism;
        let title = format!("{} {} Toolpath", if self.index.is_some() { i18n::t("Edit") } else { i18n::t("New") }, kind_name(k));
        let selected: Vec<i32> = host.canvas.sel.iter().copied().collect();
        let have_sel = !selected.is_empty();
        // the dialog floats at the right, the canvas stays live: a new toolpath follows the canvas selection
        if self.index.is_none() && self.scope == 0 && have_sel && selected != self.op.shape_ids {
            self.op.shape_ids = selected.clone();
        }
        // every sheet counts: a toolpath kept to another sheet still has its shapes there
        let applied = self.apply(doc);
        let shapes: usize = (0..doc.sheets.len().max(1) as i32).map(|j| doc.op_shape_ids_on(&applied, j).len()).sum();
        let needs_shapes = !matches!(k, OpType::Relief | OpType::Imported);
        let at = egui::pos2(ctx.content_rect().right() - 12.0, 70.0);
        crate::widgets::Dialog::new(title).id("op").width(464.0).modeless(at, egui::Align2::RIGHT_TOP).show(ctx, |ui| {
            // the fields scroll, OK / Cancel stay in view: the dialog starts 70 px down and its frame, title and
            // button row take about 160 px more
            egui::ScrollArea::vertical().max_height((ctx.input(|i| i.content_rect().height()) - 232.0).max(160.0)).show(ui, |ui| {
                egui::Grid::new("op").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                    ui.label(i18n::t("Name"));
                    ui.add(crate::widgets::field(&mut self.op.name).desired_width(300.0));
                    ui.end_row();
                    if !is_relief && !fixed {
                        ui.label(i18n::t("Applies to"));
                        let layer_name = doc.layers.get(self.scope_layer.max(0) as usize).map(|l| l.name.clone()).unwrap_or_default();
                        let group_ok = self.scope_group > 0;
                        let _ = layer_name;
                        let items = [(0, i18n::t("Selected shapes only").to_string()), (1, i18n::t("A layer (new shapes on it join)").to_string()), (2, if group_ok { i18n::t("The selection’s group (new members inherit)").into() } else { i18n::t("The selection’s group (select a group first)").into() }), (3, i18n::t("All vectors, every layer (new shapes join)").to_string())];
                        let refs: Vec<(i32, &str)> = items.iter().map(|(i, s)| (*i, s.as_str())).collect();
                        combo(ui, "scope", &mut self.scope, &refs, 360.0);
                        if self.scope == 2 && !group_ok {
                            self.scope = 0;
                        }
                        ui.end_row();
                        if self.scope == 1 {
                            ui.label(i18n::t("Layer"));
                            let layers: Vec<(i32, String)> = doc.layers.iter().enumerate().filter(|(_, l)| !l.keep_out).map(|(i, l)| (i as i32, l.name.clone())).collect();
                            let refs: Vec<(i32, &str)> = layers.iter().map(|(i, n)| (*i, n.as_str())).collect();
                            combo(ui, "scope-layer", &mut self.scope_layer, &refs, 260.0);
                            ui.end_row();
                        }
                        if doc.sheets.len() > 1 {
                            ui.label(i18n::t("Sheets")).tip(i18n::t("Which sheets it cuts on. All sheets: the same toolpath cuts its shapes on every sheet they're on (one setup, every sheet's file)."));
                            let sheets: Vec<(i32, String)> = std::iter::once((-1, i18n::t("All sheets").to_string())).chain((0..doc.sheets.len()).map(|j| (j as i32, format!("{} only", doc.sheet_name(j))))).collect();
                            let refs: Vec<(i32, &str)> = sheets.iter().map(|(i, n)| (*i, n.as_str())).collect();
                            combo(ui, "scope-sheet", &mut self.op.sheet_scope, &refs, 200.0);
                            ui.end_row();
                        }
                        ui.label(i18n::t("Shapes"));
                        ui.horizontal(|ui| {
                            if shapes == 0 {
                                ui.label(RichText::new(i18n::t("Click shapes on the canvas (Shift adds), or pick a layer")).color(crate::theme::ORANGE));
                            } else {
                                ui.label(format!("{shapes} shape(s)"));
                            }
                            if ui.button(i18n::t("Selector…")).on_hover_text(i18n::t("Vector Selector: pick shapes by kind (open, closed, circles), size and layer")).clicked() {
                                self.open_selector = true;
                            }
                            if self.index.is_some() && ui.add_enabled(have_sel, egui::Button::new(i18n::t("Use current selection"))).clicked() {
                                self.op.shape_ids = selected.clone();
                                self.scope = 0;
                            }
                        });
                        ui.end_row();
                    }
                    ui.label(i18n::t("Tool"));
                    ui.horizontal(|ui| {
                        tool_combo(ui, "tool", &mut self.op.tool, &doc.tools, None, 280.0);
                        if ui.button(i18n::t("Bits…")).on_hover_text(i18n::t("Open the Tool Library: add a bit from the database or a new one")).clicked() {
                            self.open_bits = true;
                        }
                        // the tool-changer pocket: prominent on ATC machines, a quiet note otherwise
                        let slot = doc.tools.get(self.op.tool.max(0) as usize).map_or(0, |t| t.slot);
                        if crate::toollib::machine_slots(doc) > 0 {
                            let text = if slot > 0 { format!("ATC slot {}", super::tools::slot_text(slot)) } else { i18n::t("No ATC slot: set it in the Tool Library").into() };
                            ui.label(RichText::new(text).family(crate::widgets::bold_family()).color(if slot > 0 { crate::theme::ACCENT } else { crate::theme::ORANGE }));
                        } else if slot > 0 {
                            ui.label(RichText::new(format!("slot {}", super::tools::slot_text(slot))).small().color(crate::theme::TEXT_DIM));
                        }
                    });
                    ui.end_row();
                    if !matches!(self.op.kind, OpType::Relief | OpType::Photo | OpType::Imported) {
                        ui.label(i18n::t("Order")).on_hover_text(i18n::t("Normally the shapes are cut in the order with the least travel between them. Tick this to cut them in the order they were drawn (VCarve: use vector selection order)."));
                        crate::widgets::checkbox(ui, &mut self.op.keep_order, i18n::t("Cut shapes in the order they were drawn"));
                        ui.end_row();
                    }
                    // project onto a 3D model: only when the drawing has one, and not for the 3D toolpath itself
                    if !doc.reliefs.is_empty() && self.op.kind != OpType::Relief {
                        ui.label(i18n::t("Follow 3D model")).on_hover_text(i18n::t("Project the toolpath onto the model: every cut is measured down from the model's surface instead of the flat board top (engraving or V-carving on a carved sign)"));
                        let name = |id: i32| if id <= 0 { i18n::t("No (flat board)").to_string() } else { doc.find_relief(id).map_or(format!("Model #{id} (gone)"), |r| r.name.clone()) };
                        egui::ComboBox::from_id_salt("project-on").selected_text(name(self.op.project_on)).width(280.0).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.op.project_on, 0, name(0));
                            for r in &doc.reliefs {
                                ui.selectable_value(&mut self.op.project_on, r.id, &r.name);
                            }
                        });
                        ui.end_row();
                    }
                    if self.op.tool >= 0 {
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.set_width(360.0);
                            tool_advice_ui(ui, doc, self.op.tool as usize, 360.0);
                        });
                        ui.end_row();
                    }
                    if fixed {
                        ui.label("");
                        hint(ui, i18n::t("The moves come from the imported G-code file: only the name and the bit change. The bit sets the RPM, the simulation and the file each toolpath goes into."));
                        ui.end_row();
                        return;
                    }
                    if k == OpType::Photo {
                        ui.label(i18n::t("Picture"));
                        ui.horizontal(|ui| {
                            if ui.button(if self.op.photo.is_empty() { i18n::t("Choose picture…") } else { i18n::t("Change picture…") }).clicked() {
                                self.pick_photo = true;
                            }
                            // VCarve's sketch carving from a 3D model: the model lit from the upper left becomes the picture
                            let models: Vec<&gcarve_core::document::Relief> = doc.reliefs.iter().filter(|r| !r.height.is_empty()).collect();
                            if !models.is_empty() {
                                ui.menu_button(i18n::t("From a 3D model"), |ui| {
                                    for r in &models {
                                        if ui.button(&r.name).on_hover_text(i18n::t("Its shading as a sketch: draw a rectangle over the model and select it, so the picture lands on the model")).clicked() {
                                            let (px, w, h) = gcarve_core::photo::shaded_relief(r, 600);
                                            self.set_photo(&px, w, h);
                                            self.op.sketch = true;
                                            ui.close();
                                        }
                                    }
                                });
                            }
                            if !self.op.photo.is_empty() {
                                hint(ui, &format!("{} × {} px", self.op.photo_w, self.op.photo_h));
                            }
                        });
                        ui.end_row();
                        ui.label(unit_label(i18n::t("Max depth"), doc));
                        self.f.ui(ui, "depth", doc);
                        ui.end_row();
                        ui.label(i18n::t("Direction"));
                        ui.add(egui::DragValue::new(&mut self.op.raster_angle).range(0..=179).suffix("°"));
                        ui.end_row();
                        ui.label(i18n::t("Tones"));
                        crate::widgets::checkbox(ui, &mut self.op.photo_invert, i18n::t("Invert: light parts carve deep (dark wood, light picture)"));
                        ui.end_row();
                        ui.label(i18n::t("Style"));
                        ui.horizontal(|ui| {
                            crate::widgets::radio_value(ui, &mut self.op.sketch, false, i18n::t("Photo (depth follows the tone)"));
                            crate::widgets::radio_value(ui, &mut self.op.sketch, true, i18n::t("Sketch (cross-hatching)"));
                        });
                        ui.end_row();
                        ui.label(i18n::t("Brightness"));
                        ui.add(egui::Slider::new(&mut self.op.photo_brightness, -0.5..=0.5).fixed_decimals(2)).tip(i18n::t("Higher = lighter picture, shallower carving"));
                        ui.end_row();
                        ui.label(i18n::t("Contrast"));
                        ui.add(egui::Slider::new(&mut self.op.photo_contrast, 0.5..=2.5).fixed_decimals(2)).tip(i18n::t("Higher = darks deeper and lights shallower"));
                        ui.end_row();
                        ui.label(i18n::t("Gamma"));
                        ui.add(egui::Slider::new(&mut self.op.photo_gamma, 0.4..=2.5).fixed_decimals(2)).tip(i18n::t("Higher = lighter mid tones; lower = darker mid tones"));
                        ui.end_row();
                        ui.label("");
                        hint(ui, i18n::t("Carves the picture with a V-bit, row by row: darker means deeper and wider. It fills the selected shape (draw a rectangle the size you want). Rows are spaced so full black grooves just touch; a 60° or 90° bit and a fine-grained, light-coloured board work best."));
                        ui.end_row();
                        return;
                    }
                    if k == OpType::Texture {
                        ui.label(unit_label(i18n::t("Depth"), doc));
                        self.f.ui(ui, "depth", doc);
                        ui.end_row();
                        ui.label(unit_label(i18n::t("Stroke length"), doc));
                        self.f.ui(ui, "rampLength", doc);
                        ui.end_row();
                        ui.label(i18n::t("Row spacing (%)"));
                        ui.horizontal(|ui| {
                            ui.add(crate::widgets::SpinBox::new(&mut self.stepover_pct, 20..=300).step(10.0));
                            hint(ui, i18n::t("of the bit diameter"));
                        });
                        ui.end_row();
                        ui.label(i18n::t("Variation (%)"));
                        let mut pct = (self.op.vary * 100.0).round() as i32;
                        ui.horizontal(|ui| {
                            ui.add(crate::widgets::SpinBox::new(&mut pct, 0..=100).step(10.0));
                            hint(ui, i18n::t("random change of length and depth"));
                        });
                        self.op.vary = pct as f64 / 100.0;
                        ui.end_row();
                        ui.label(i18n::t("Direction"));
                        ui.add(egui::DragValue::new(&mut self.op.raster_angle).range(0..=179).suffix("°"));
                        ui.end_row();
                        ui.label("");
                        hint(ui, i18n::t("Short strokes that dip to the depth and back up, row by row, filling the shapes: hand-carved look for sign backgrounds. A ball nose or V-bit works best."));
                        ui.end_row();
                        return;
                    }
                    if is_relief {
                        ui.label(i18n::t("3D model"));
                        let names: Vec<(i32, String)> = doc.reliefs.iter().map(|r| (r.id, r.name.clone())).collect();
                        let refs: Vec<(i32, &str)> = names.iter().map(|(i, s)| (*i, s.as_str())).collect();
                        if refs.is_empty() {
                            ui.label(RichText::new(i18n::t("No 3D models: import an STL in Toolpaths > 3D Models")).color(crate::theme::ORANGE));
                        } else {
                            if !refs.iter().any(|(i, _)| *i == self.op.relief) {
                                self.op.relief = refs[0].0;
                            }
                            combo(ui, "relief", &mut self.op.relief, &refs, 300.0);
                        }
                        ui.end_row();
                        ui.label(i18n::t("Finish stepover (%)"));
                        ui.horizontal(|ui| {
                            ui.add(crate::widgets::SpinBox::new(&mut self.stepover_pct, 2..=100));
                            hint(ui, i18n::t("of the bit diameter between raster lines (8–12% for a smooth finish)"));
                        });
                        ui.end_row();
                        ui.label(i18n::t("Direction"));
                        combo(ui, "angle", &mut self.op.raster_angle, &[(0, i18n::t("Along X")), (90, i18n::t("Along Y"))], 160.0);
                        ui.end_row();
                        ui.label(i18n::t("Roughing"));
                        crate::widgets::checkbox(ui, &mut self.op.rough, i18n::t("Rough out in layers first"));
                        ui.end_row();
                        let rough = self.op.rough;
                        ui.add_enabled(rough, egui::Label::new(i18n::t("Roughing tool")));
                        ui.add_enabled_ui(rough, |ui| tool_combo(ui, "rough-tool", &mut self.op.clear_tool, &doc.tools, None, 300.0));
                        ui.end_row();
                        ui.add_enabled(rough, egui::Label::new(unit_label(i18n::t("Rough pass depth"), doc)));
                        ui.add_enabled_ui(rough, |ui| {
                            ui.horizontal(|ui| {
                                crate::units::num_field(ui, "rough-step", &mut self.rough_step, doc, true, 110.0);
                                hint(ui, i18n::t("0 = tool's"));
                            });
                        });
                        ui.end_row();
                        ui.add_enabled(rough, egui::Label::new(i18n::t("Rough stepover (%)")));
                        ui.add_enabled(rough, crate::widgets::SpinBox::new(&mut self.rough_pct, 10..=100));
                        ui.end_row();
                        ui.add_enabled(rough, egui::Label::new(unit_label(i18n::t("Rough allowance"), doc)));
                        ui.add_enabled_ui(rough, |ui| {
                            ui.horizontal(|ui| {
                                self.f.ui(ui, "allowance", doc);
                                hint(ui, i18n::t("left for the finishing pass"));
                            });
                        });
                        ui.end_row();
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            hint(ui, i18n::t("Finishing follows the model surface with the bit's true shape (ball, flat or V) so it never gouges. Set the model's size, position and depth in Toolpaths > 3D Models."));
                        });
                        ui.end_row();
                        // selected closed vectors limit the machined area (core: generate_relief boundary)
                        ui.label(i18n::t("Boundary"));
                        ui.vertical(|ui| {
                            let closed = self.op.shape_ids.iter().filter(|id| doc.find(**id).is_some_and(|s| s.c.closed)).count();
                            ui.horizontal(|ui| {
                                ui.label(if closed > 0 { format!("{closed} closed vector(s): only the area inside is machined") } else { i18n::t("Whole model").into() });
                                if ui.add_enabled(have_sel, egui::Button::new(i18n::t("Use selection"))).clicked() {
                                    self.op.shape_ids = selected.clone();
                                }
                                if closed > 0 && ui.button(i18n::t("Clear")).clicked() {
                                    self.op.shape_ids.clear();
                                }
                            });
                            if closed == 0 {
                                hint(ui, i18n::t("Select closed vectors (and press Use selection) to machine only inside them."));
                            }
                        });
                        ui.end_row();
                    }
                    if needs_v && doc.tools.get(self.op.tool.max(0) as usize).is_some_and(|t| t.kind != ToolType::VBit) {
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            ui.label(RichText::new(i18n::t("⚠ This toolpath needs a V-bit: pick one above (nothing is cut with this tool).")).color(crate::theme::ORANGE));
                        });
                        ui.end_row();
                    }
                    if is_profile {
                        ui.label(i18n::t("Cut"));
                        combo(ui, "side", &mut self.op.side, &[(Side::Outside, i18n::t("Outside")), (Side::Inside, i18n::t("Inside")), (Side::On, i18n::t("On the line"))], 180.0);
                        ui.end_row();
                    }
                    if is_chamfer {
                        ui.label(i18n::t("Bevel"));
                        combo(ui, "side", &mut self.op.side, &[(Side::Outside, i18n::t("Outside the vector")), (Side::Inside, i18n::t("Inside the vector")), (Side::On, i18n::t("Centred on the vector"))], 220.0);
                        ui.end_row();
                        ui.label(i18n::t("Size by"));
                        combo(ui, "size-by", &mut self.by_width, &[(true, i18n::t("Chamfer width")), (false, i18n::t("V depth"))], 180.0);
                        ui.end_row();
                        if self.by_width {
                            ui.label(unit_label(i18n::t("Chamfer width"), doc));
                            ui.horizontal(|ui| {
                                self.f.ui(ui, "chamferWidth", doc);
                                hint(ui, i18n::t("depth = width ÷ tan(half the bit angle)"));
                            });
                            ui.end_row();
                        }
                    }
                    if !v_like && !is_relief && !(is_chamfer && self.by_width) {
                        ui.label(unit_label(if is_prism { i18n::t("Max depth") } else if is_chamfer { i18n::t("V depth") } else { i18n::t("Cut depth") }, doc));
                        self.f.ui(ui, "depth", doc);
                        ui.end_row();
                    }
                    if is_chamfer || is_prism {
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            hint(ui, if is_chamfer { i18n::t("The V-bit tip rides the vector, so corners stay sharp. Outside / inside: the bevel face lies on that side (holes flip, like profile); centred: a V-groove of that width.") } else { i18n::t("Raised, bevelled shapes: the V-bit runs on each outline as deep as the shape is wide, so its flank cuts a facet up to the centre line (left at the surface) and an equal groove outside. Max depth caps the bevel: wider parts keep a flat top.") });
                        });
                        ui.end_row();
                    }
                    if is_vcarve {
                        ui.label(i18n::t("Flat depth"));
                        crate::widgets::checkbox(ui, &mut self.flat, i18n::t("Limit depth, flat bottom"));
                        ui.end_row();
                    }
                    if v_like {
                        let on = self.flat || is_inlay;
                        ui.add_enabled(on, egui::Label::new(unit_label(if is_inlay { i18n::t("Pocket depth") } else { i18n::t("Flat depth") }, doc)));
                        ui.add_enabled_ui(on, |ui| self.f.ui(ui, "flatDepth", doc));
                        ui.end_row();
                    }
                    if is_vcarve || is_chamfer {
                        ui.label(unit_label(i18n::t("Start depth"), doc));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "startDepth", doc);
                            hint(ui, i18n::t("carve below an existing surface"));
                        });
                        ui.end_row();
                    }
                    if is_inlay {
                        ui.label(unit_label(i18n::t("Glue gap"), doc));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "glueGap", doc);
                            hint(ui, i18n::t("male start depth; plug height = pocket − gap"));
                        });
                        ui.end_row();
                        ui.label(unit_label(i18n::t("Male margin"), doc));
                        self.f.ui(ui, "inlayMargin", doc);
                        ui.end_row();
                    }
                    if v_like {
                        let on = self.flat || is_inlay;
                        ui.add_enabled(on, egui::Label::new(i18n::t("Clearance tool")));
                        ui.add_enabled_ui(on, |ui| tool_combo(ui, "clear", &mut self.op.clear_tool, &doc.tools, Some(i18n::t("None (V-bit clears the floor)")), 300.0));
                        ui.end_row();
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            hint(ui, if is_inlay { i18n::t("Makes two sets of paths: the female pocket of the design, and the male plug carved from the inverted, mirrored design starting at the glue gap. Cut the male from a second board, flip it and glue it in.") } else { i18n::t("Carves with the selected V-bit; the floor depth follows the stroke width.") });
                        });
                        ui.end_row();
                    }
                    if !is_relief {
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            hint(ui, i18n::t("Tip: type a variable (e.g. thickness + 0.02) in any size to keep it linked."));
                        });
                        ui.end_row();
                    }
                    if !is_drill && !v_like && !is_relief {
                        ui.label(unit_label(if is_bore { i18n::t("Depth per revolution") } else { i18n::t("Pass depth") }, doc));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "stepdown", doc);
                            hint(ui, i18n::t("0 = tool's"));
                        });
                        ui.end_row();
                    }
                    if is_flute {
                        ui.label(unit_label(i18n::t("Ramp in"), doc));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "rampIn", doc);
                            hint(ui, i18n::t("length from the start; 0 = plunge"));
                        });
                        ui.end_row();
                        ui.label(unit_label(i18n::t("Ramp out"), doc));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "rampOut", doc);
                            hint(ui, i18n::t("length before the end; 0 = none"));
                        });
                        ui.end_row();
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            hint(ui, i18n::t("Cuts along each vector with the bit (round-nose bits make classic flutes). Set both ramps longer than the vector for a full-length lens shape."));
                        });
                        ui.end_row();
                        // moulding: an open vector of this sheet as the profile swept along the selected rails
                        ui.label(i18n::t("Cross-section")).on_hover_text(i18n::t("Moulding: draw the profile as an open vector (its width across the rail, lower = deeper below the top), pick it here, and select the rails. The bit carves that shape along every rail; a ball nose gives the smoothest result."));
                        let open: Vec<(i32, String)> = doc.shapes.iter().filter(|s| !s.c.closed && s.sheet == doc.active_sheet).map(|s| {
                            let b = gcarve_core::geom::bounds(&s.c);
                            (s.id, format!("Vector #{} ({} × {})", s.id, crate::units::text(doc, b.w()), crate::units::text(doc, b.h())))
                        }).collect();
                        let name = |id: i32| if id <= 0 { i18n::t("None (plain flute)").to_string() } else { open.iter().find(|(i, _)| *i == id).map_or(format!("Vector #{id} (missing)"), |(_, n)| n.clone()) };
                        egui::ComboBox::from_id_salt("section").selected_text(name(self.op.section)).width(300.0).show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.op.section, 0, name(0));
                            for (id, n) in &open {
                                ui.selectable_value(&mut self.op.section, *id, n);
                            }
                        });
                        ui.end_row();
                        if self.op.section > 0 {
                            ui.label(i18n::t("Clearing bit")).tip(i18n::t("A larger end mill roughs the shape first, staying the allowance above it; the bit above then finishes in one pass at full depth. Two files, one per bit: run the clearing bit's first."));
                            tool_combo(ui, "mould-clear", &mut self.op.clear_tool, &doc.tools, Some(i18n::t("None (the bit above does it all)")), 300.0);
                            ui.end_row();
                            ui.label(unit_label(i18n::t("Leave for the finish"), doc));
                            ui.horizontal(|ui| {
                                self.f.ui(ui, "allowance", doc);
                                hint(ui, i18n::t("with a clearing bit; 0 = 0.02 in"));
                            });
                            ui.end_row();
                        }
                    }
                    if is_bore {
                        ui.label(unit_label(i18n::t("Bore allowance"), doc));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "allowance", doc);
                            hint(ui, i18n::t("material left on the hole wall"));
                        });
                        ui.end_row();
                        ui.label(unit_label(i18n::t("Thread pitch"), doc));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "threadPitch", doc);
                            hint(ui, i18n::t("0 = plain bore. For a thread mill: distance per turn (1/20 in = 20 TPI, 1.5 mm for M10)"));
                        });
                        ui.end_row();
                        ui.label("");
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            hint(ui, i18n::t("Helical cut inside each selected circle: hole size comes from the shape, the bit spirals down then finishes a flat lap. Holes no bigger than the bit are drilled straight. With a thread pitch it climbs from the bottom up, one turn per pitch."));
                        });
                        ui.end_row();
                    }
                    if is_pocket && doc.tools.get(self.op.tool as usize).is_some_and(|t| t.kind == ToolType::VBit) {
                        ui.label("");
                        ui.scope(|ui| {
                            ui.set_width(300.0);
                            hint(ui, i18n::t("Engraving fill: with a V-bit or engraving bit the lines are spaced by how wide the bit cuts at this depth. Keep the depth shallow (0.01 to 0.03 in)."));
                        });
                        ui.end_row();
                    }
                    if is_pocket && doc.material.rotary.is_some() {
                        let mut on = self.op.rounding > 0.0;
                        ui.label(i18n::t("Rounding"));
                        ui.horizontal(|ui| {
                            if crate::widgets::checkbox(ui, &mut on, i18n::t("Square blank, side")).tip(i18n::t("Rotary: turn a square blank round. The bit spirals the length of the selected shape from the square's corners down to the job's diameter, a stepdown per pass. Draw a rectangle across the part of the blank to round and pick it. Travel heights must clear the corners.")).changed() {
                                self.op.rounding = if on { self.f.get_or("rounding", doc, 3.0).max(0.01) } else { 0.0 };
                            }
                            ui.add_enabled_ui(on, |ui| self.f.ui(ui, "rounding", doc));
                        });
                        ui.end_row();
                    }
                    if is_pocket {
                        ui.label(i18n::t("Clearing bit")).tip(i18n::t("A larger end mill clears most of the pocket first; the bit above then cuts only the corners and narrow parts it couldn't reach. Two files, one per bit: run the clearing bit's first."));
                        tool_combo(ui, "pocket-clear", &mut self.op.clear_tool, &doc.tools, Some(i18n::t("None (the bit above does it all)")), 300.0);
                        ui.end_row();
                        ui.label(i18n::t("Clearing")).tip(i18n::t("Rings follow the pocket's outline inward. Rows go back and forth across it, then once round the walls: quicker on big open pockets. Adaptive keeps the bit's sideways load the same everywhere (no spikes in corners and narrow necks), so it can cut deep and fast and the bit lasts: it starts with a helix and works outward."));
                        let mut mode: i32 = if self.op.adaptive { 2 } else { i32::from(self.op.raster) };
                        ui.horizontal(|ui| {
                            let before = mode;
                            combo(ui, "clearing", &mut mode, &[(0, i18n::t("Rings (offset)")), (1, i18n::t("Rows (raster)")), (2, i18n::t("Adaptive (constant load)"))], 190.0);
                            if mode != before {
                                (self.op.raster, self.op.adaptive) = (mode == 1, mode == 2);
                                if mode == 2 && self.stepover_pct > 30 {
                                    self.stepover_pct = 12; // a rings-sized stepover would be a heavy cut here
                                }
                            }
                            if self.op.raster {
                                ui.label("at");
                                ui.add(egui::DragValue::new(&mut self.op.raster_angle).range(0..=179).suffix("°"));
                            }
                        });
                        ui.end_row();
                        if self.op.adaptive {
                            ui.label(i18n::t("Load (% of bit)")).tip(i18n::t("How wide each cut is, as a share of the bit's diameter, kept the same everywhere. 10–15 % is typical in wood: deep cuts (a stepdown of one bit diameter or more) at a good feed. Larger = faster but heavier."));
                            ui.horizontal(|ui| {
                                ui.add(crate::widgets::SpinBox::new(&mut self.stepover_pct, 3..=50));
                                hint(ui, i18n::t("end mills only; a V-bit or ball nose cuts rings"));
                            });
                            ui.end_row();
                            ui.label("");
                            ui.checkbox(&mut self.op.feed_boost, i18n::t("Feed up for the thin cut")).tip(i18n::t("Chip thinning: a thin cut leaves a thinner chip, so the feed can rise up to 1.5× (within the machine's limit). Off = the bit's own feed."));
                        } else {
                            ui.label(i18n::t("Stepover (%)"));
                            ui.add(crate::widgets::SpinBox::new(&mut self.stepover_pct, 5..=100).step(5.0));
                        }
                        ui.end_row();
                        ui.label(unit_label(i18n::t("Pocket allowance"), doc));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "allowance", doc);
                            hint(ui, i18n::t("material left on walls"));
                        });
                        ui.end_row();
                    }
                    if is_profile {
                        ui.label(unit_label(i18n::t("Lead in / out"), doc)).tip(i18n::t("The bit plunges in the waste and curves onto the line (and off it again), so there is no plunge mark on the part edge. The value is the arc radius; 0 = off. Not used when cutting on the line."));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "rampIn", doc);
                            hint(ui, i18n::t("0 = off: arc radius in the waste"));
                        });
                        ui.end_row();
                        let mut knife = self.op.drag_knife > 0.0;
                        ui.label(i18n::t("Drag knife"));
                        ui.horizontal(|ui| {
                            if crate::widgets::checkbox(ui, &mut knife, i18n::t("Blade offset")).tip(i18n::t("For a drag knife in the spindle: one pass on the line, the holder swings round each sharp corner so the blade turns in place. Set the knife's RPM to 0 in the Tool Library: the G-code then never turns the spindle on.")).changed() {
                                self.op.drag_knife = if knife { self.f.get_or("dragKnife", doc, 0.25).max(0.001) } else { 0.0 };
                            }
                            ui.add_enabled_ui(knife, |ui| self.f.ui(ui, "dragKnife", doc));
                        });
                        ui.end_row();
                        ui.label(i18n::t("Keyhole slot"));
                        crate::widgets::checkbox(ui, &mut self.op.keyhole, i18n::t("Cut as a keyhole slot")).tip(i18n::t("For a keyhole cutter: draw a line from where the screw goes in to where it should hang. The bit plunges once to full depth at the line's start, runs along it, comes back and lifts where it went in."));
                        ui.end_row();
                        ui.label(i18n::t("Cutter compensation"));
                        ui.add_enabled_ui(self.op.side != Side::On, |ui| {
                            crate::widgets::checkbox(ui, &mut self.op.cutter_comp, i18n::t("G41 / G42 for trimming the size at the machine")).tip(i18n::t("Writes G41 (climb) or G42 round each cut so the operator can make the part a hair bigger or smaller with the machine's D offset. The path is already the bit's centre: D must hold only the wear (0 = the size drawn), not the bit's radius. Only Mach / LinuxCNC / Fanuc-style posts write it; GRBL and WinCNC files leave it out."));
                        });
                        ui.end_row();
                        ui.label(i18n::t("Start point"));
                        crate::widgets::checkbox(ui, &mut self.op.start_first, i18n::t("At the vector's first node")).tip(i18n::t("Each cut starts at the point of the ring nearest the shape's first node. Set it in the Node tool: right-click a node ▸ Make Start Point. Off = the lowest point."));
                        ui.end_row();
                        ui.label(unit_label(i18n::t("Last pass allowance"), doc)).tip(i18n::t("Separate last pass: the stepped passes stay this far off the line, then one full-depth pass on the line cleans the edge. 0 = off; 0.02 in is typical."));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "allowance", doc);
                            hint(ui, i18n::t("0 = off: extra clean-up pass on the line"));
                        });
                        ui.end_row();
                        ui.label(unit_label(i18n::t("Onion skin"), doc)).tip(i18n::t("Every pass stops this far above the full depth, so the parts stay held by a thin skin (good on a vacuum table, no tabs to sand). One last lap round each part, after all the others, cuts the skin. Tabs are left out. 0 = off; 0.01–0.02 in is typical."));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "skin", doc);
                            hint(ui, i18n::t("0 = off: parts held by a thin skin until the end"));
                        });
                        ui.end_row();
                    }
                    if is_profile || is_pocket {
                        ui.label(i18n::t("Entry"));
                        combo(ui, "entry", &mut self.op.entry, &[(Entry::Plunge, i18n::t("Plunge straight down")), (Entry::Spiral, i18n::t("Spiral: ramp over the whole pass")), (Entry::Ramp, i18n::t("Ramp over a distance"))], 300.0);
                        ui.end_row();
                        if self.op.entry == Entry::Ramp {
                            ui.label(unit_label(i18n::t("Ramp length"), doc));
                            self.f.ui(ui, "rampLength", doc);
                            ui.end_row();
                        }
                    }
                    if !is_drill && !v_like && !is_bore && !is_flute && !is_relief && !is_prism {
                        ui.label(i18n::t("Direction"));
                        crate::widgets::checkbox(ui, &mut self.op.climb, i18n::t("Climb (else conventional)"));
                        ui.end_row();
                    }
                    // VCarve's Array Copy Toolpath: the same cut repeated across a grid, no extra vectors
                    ui.label(i18n::t("Array copies")).tip(i18n::t("Cut this toolpath again in a grid: rows × columns copies, each moved by the spacing from the last (copy to copy, not the gap between parts). 1 × 1 = off."));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.op.array_rows).range(1..=500).suffix(i18n::t(" rows")));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut self.op.array_cols).range(1..=500).suffix(i18n::t(" columns")));
                    });
                    ui.end_row();
                    if self.op.array_rows * self.op.array_cols > 1 {
                        ui.label(unit_label(i18n::t("Spacing X / Y"), doc)).tip(i18n::t("How far each copy moves from the last: X across the columns, Y up the rows (centre to centre)."));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "arrayDx", doc);
                            self.f.ui(ui, "arrayDy", doc);
                        });
                        ui.end_row();
                    }
                    if is_drill {
                        ui.label(i18n::t("Peck drilling"));
                        crate::widgets::checkbox(ui, &mut self.peck, i18n::t("Retract between pecks"));
                        ui.end_row();
                        ui.add_enabled(self.peck, egui::Label::new(unit_label(i18n::t("Peck depth"), doc)));
                        let peck = self.peck;
                        ui.add_enabled_ui(peck, |ui| self.f.ui(ui, "peck", doc));
                        ui.end_row();
                        ui.label(unit_label(i18n::t("Countersink width"), doc));
                        ui.horizontal(|ui| {
                            self.f.ui(ui, "countersink", doc);
                            hint(ui, i18n::t("0 = plain hole. With a V-bit: the width of the cone at the surface; the depth follows from the bit's angle"));
                        });
                        ui.end_row();
                        ui.label(i18n::t("V-bit holes"));
                        crate::widgets::checkbox(ui, &mut self.op.fit_holes, i18n::t("Each as wide as its circle")).tip(i18n::t("With a V-bit: every hole is plunged until the cone is as wide as the circle drawn (halftone dots, countersinks of several sizes), never deeper than the depth above."));
                        ui.end_row();
                    }
                    if is_profile {
                        ui.label(i18n::t("Tabs"));
                        crate::widgets::checkbox(ui, &mut self.op.tabs, i18n::t("Add tabs"));
                        ui.end_row();
                        let tabs = self.op.tabs;
                        ui.add_enabled(tabs, egui::Label::new(i18n::t("How many")));
                        ui.add_enabled_ui(tabs, |ui| {
                            ui.horizontal(|ui| {
                                combo(ui, "tab-mode", &mut self.by_spacing, &[(false, i18n::t("Same on every part")), (true, i18n::t("One every"))], 150.0);
                                if self.by_spacing {
                                    self.f.ui(ui, "tabSpacing", doc);
                                } else {
                                    ui.add(crate::widgets::SpinBox::new(&mut self.tab_count, 1..=50));
                                }
                            });
                        });
                        ui.end_row();
                        ui.add_enabled(tabs, egui::Label::new(unit_label(i18n::t("Tab width"), doc)));
                        ui.add_enabled_ui(tabs, |ui| self.f.ui(ui, "tabWidth", doc));
                        ui.end_row();
                        ui.add_enabled(tabs, egui::Label::new(unit_label(i18n::t("Tab height"), doc)));
                        ui.add_enabled_ui(tabs, |ui| self.f.ui(ui, "tabHeight", doc));
                        ui.end_row();
                        ui.add_enabled(tabs, egui::Label::new(i18n::t("Tab shape")));
                        ui.add_enabled(tabs, crate::widgets::CheckBox::new(&mut self.op.tab_ramped, i18n::t("3D tabs: ramp over the tab instead of stepping")));
                        ui.end_row();
                        ui.add_enabled(tabs, egui::Label::new(i18n::t("Spread evenly")));
                        ui.add_enabled(tabs, crate::widgets::CheckBox::new(&mut self.op.tab_corners, i18n::t("Keep tabs off corners"))).tip(i18n::t("Evenly spaced tabs that would land on a corner slide along to the nearest straight part: a tab on a corner is hard to sand off."));
                        ui.end_row();
                        ui.add_enabled(tabs, egui::Label::new(i18n::t("Place by hand"))).tip(i18n::t("Tabs you place replace the evenly spaced ones on the parts they are on; other parts keep the even ones."));
                        ui.vertical(|ui| {
                            ui.add_enabled_ui(tabs, |ui| {
                                ui.horizontal(|ui| {
                                    let label = if self.place_tabs { i18n::t("Done placing tabs") } else { i18n::t("Place tabs on the drawing…") };
                                    if ui.add(crate::widgets::Button::new(label).primary(self.place_tabs)).clicked() {
                                        self.place_tabs = !self.place_tabs;
                                    }
                                    if !self.op.tab_points.is_empty() && ui.small_button(format!("Clear {}", self.op.tab_points.len())).tip(i18n::t("Remove the tabs placed by hand: the parts go back to evenly spaced tabs")).clicked() {
                                        self.op.tab_points.clear();
                                    }
                                });
                                if self.place_tabs {
                                    hint(ui, i18n::t("Click an outline to add a tab, click a tab to remove it, drag a tab to move it"));
                                } else if !self.op.tab_points.is_empty() {
                                    hint(ui, &format!("{} placed by hand", self.op.tab_points.len()));
                                }
                            });
                        });
                        ui.end_row();
                    }
                });
            });
            let (o, c) = crate::widgets::ok_cancel(ui);
            if o && needs_shapes && shapes == 0 {
                *host.message = i18n::t("Pick the shapes to cut first: click them on the canvas").into();
            } else {
                ok |= o;
            }
            keep &= !c;
        });
        if ok {
            let op = self.apply(&host.canvas.doc);
            host.canvas.doc.checkpoint();
            match self.index {
                Some(i) => host.canvas.doc.ops[i] = op,
                None => host.canvas.doc.ops.push(op),
            }
            host.canvas.touch();
            return false;
        }
        keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolpath_for_a_whole_layer() {
        let mut doc = Document::default();
        let l = doc.layer_named("Letters");
        doc.add(gcarve_core::geom::make_rect(0.0, 0.0, 1.0, 1.0), Some(l));
        let d = OpDialog::new_for_layer(&doc, OpType::VCarve, l);
        assert_eq!((d.op.layer_scope, d.op.shape_ids.len(), d.scope, d.op.name.as_str()), (l, 0, 1, "Letters V-Carve"));
        // a shape drawn on the layer later is covered too
        let id = doc.add(gcarve_core::geom::make_rect(2.0, 0.0, 3.0, 1.0), Some(l));
        assert!(doc.op_shape_ids(&d.op).contains(&id));
    }

    #[test]
    fn defaults_follow_the_qt_app() {
        let mut doc = Document::default();
        doc.material.t = 0.5;
        let p = new_op(&doc, OpType::Pocket, &[7]);
        assert_eq!((p.depth, p.shape_ids.clone(), p.name.as_str()), (0.25, vec![7], "Pocket 1"));
        let v = new_op(&doc, OpType::VCarve, &[]);
        assert_eq!(doc.tools[v.tool as usize].kind, ToolType::VBit);
        let i = new_op(&doc, OpType::Inlay, &[]);
        assert_eq!((i.flat_depth, i.glue_gap), (0.25, 0.125));
        let c = new_op(&doc, OpType::Chamfer, &[]);
        assert!(doc.tools[c.tool as usize].kind == ToolType::VBit && c.chamfer_width == 0.125 && c.side == Side::Inside);
        let cd = OpDialog::from_op(&doc, c, None, &[]).apply(&doc);
        assert!((cd.chamfer_width - 0.125).abs() < 1e-9);
        let pr = new_op(&doc, OpType::Prism, &[]);
        assert!(doc.tools[pr.tool as usize].kind == ToolType::VBit && pr.depth == 0.25);
        let r = new_op(&doc, OpType::Relief, &[]);
        assert!(r.rough && r.stepover == 0.1 && doc.tools[r.tool as usize].kind == ToolType::BallNose);
        // round trip through the dialog keeps bindings and scope
        let mut op = new_op(&doc, OpType::Profile, &[1]);
        op.bind.insert("depth".into(), "thickness".into());
        op.layer_scope = 0;
        let d = OpDialog::from_op(&doc, op, Some(0), &[1]);
        let back = d.apply(&doc);
        assert_eq!(back.bind.get("depth").map(String::as_str), Some("thickness"));
        assert!((back.depth - 0.5).abs() < 1e-9 && back.layer_scope == 0);
    }
}
