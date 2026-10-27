//! The design canvas: board, grid, shapes, pan/zoom, painting (Qt app: app/canvas_view.cpp). Pointer tools live in
//! tools.rs, document edits in edit_ops.rs, text in text.rs, popups/dialogs in popups.rs (all `impl Canvas`).
//! Document units are inches with Y up; the screen has Y down, so every mapping goes through View.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::dialogs::prefs::{GridPrefs, SnapPrefs};
use crate::theme;
use crate::units;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke};
use gcarve_core::document::{Dimension, Document, OpType, Shape as DocShape};
use gcarve_core::pipeline::Generated;
use gcarve_core::geom::{self, dist, flatten, pt, Contour, Pt};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Nodes,
    Rect,
    Ellipse,
    Polygon,
    Star,
    Polyline,
    Pen,
    Arc,
    /// drag to draw; the stroke is smoothed into arcs and lines on release
    Freehand,
    Measure,
    Dimension,
    /// angular dimension: corner, then a point on each arm
    Angle,
    Text,
}

impl Tool {
    /// (tool, label, key) for the rail and the key handler (same keys as the Qt app)
    pub const ALL: [(Tool, &'static str, egui::Key); 14] = [
        (Tool::Select, "Select", egui::Key::V),
        (Tool::Nodes, "Nodes", egui::Key::N),
        (Tool::Rect, "Rect", egui::Key::R),
        (Tool::Ellipse, "Ellipse", egui::Key::E),
        (Tool::Polygon, "Polygon", egui::Key::G),
        (Tool::Star, "Star", egui::Key::S),
        (Tool::Polyline, "Polyline", egui::Key::P),
        (Tool::Pen, "Curve", egui::Key::B),
        (Tool::Arc, "Arc", egui::Key::A),
        (Tool::Freehand, "Freehand", egui::Key::H),
        (Tool::Measure, "Measure", egui::Key::M),
        (Tool::Dimension, "Dimension", egui::Key::D),
        (Tool::Angle, "Angle", egui::Key::L),
        (Tool::Text, "Text", egui::Key::T),
    ];
    pub fn draws(self) -> bool {
        !matches!(self, Tool::Select | Tool::Nodes)
    }
}

/// Things the canvas asks the app shell to open (context menus and tools can't reach the dialogs directly).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Request {
    Offset,
    Dogbone,
    Array,
    Align,
    Nest,
    /// Edit › Vector Selector…
    Selector,
    /// Vectors › Vector Texture…
    Texture,
    /// Vectors › Plate Production…
    Plates,
    /// Vectors › Shelf-Pin Holes…
    Pins,
    /// Vectors › Rotary Layout…
    RotaryLayout,
    BoxMaker,
    Puzzle,
    Inlay,
    Hinge,
    Qr,
    /// new text; true = along the selected curve
    Text(bool),
    EditText(i32),
    /// an app command id (shell ids: "kind:0", "link", "zoom:fit"…), run by the app at the end of the frame
    Cmd(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Drag {
    #[default]
    None,
    Move,
    Marquee,
    Node,
    DrawBox,
    DrawRadial,
    Pen,
    /// Select tool on a 3D model (reliefs.rs)
    ReliefMove,
    /// transform frame (transform.rs): scale handle 0..8, rotate handle, pivot
    Scale(u8),
    Rotate,
    Pivot,
}

/// What the context menu was opened on (tools.rs sets it on right-click, popups.rs draws it).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) enum CtxMenu {
    #[default]
    None,
    Shape,
    Node(i32, usize),
    Dimension(usize),
}

/// A snapped point: kind 0 none, 1 grid, 2 object (`what`: point, midpoint, centre, quadrant, on edge);
/// `node` >= 0 only for a shape node (dimensions anchor to those)
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Snap {
    pub kind: u8,
    pub to: Pt,
    pub d: f64,
    pub shape: i32,
    pub node: i32,
    pub what: &'static str,
}

pub struct View {
    pub zoom: f32,      // pixels per inch
    pub origin: Pos2,   // screen position of document (0,0)
}

impl View {
    pub fn to_screen(&self, p: Pt) -> Pos2 {
        Pos2::new(self.origin.x + p.x as f32 * self.zoom, self.origin.y - p.y as f32 * self.zoom)
    }
    pub fn to_doc(&self, s: Pos2) -> Pt {
        pt(((s.x - self.origin.x) / self.zoom) as f64, ((self.origin.y - s.y) / self.zoom) as f64)
    }
}

/// hit tolerances in screen pixels (Qt kHitPx / kSnapPx / kNodePx)
pub(crate) const HIT_PX: f64 = 6.0;
pub(crate) const SNAP_PX: f64 = 8.0;
pub(crate) const NODE_PX: f64 = 7.0;
/// ruler strip (Qt kRulerPx)
pub(crate) const RULER_PX: f32 = 20.0;

pub struct Canvas {
    pub doc: Document,
    /// every sheet side by side (allsheets.rs)
    pub all_sheets: bool,
    /// a straightened photo of the board under the drawing (dialogs/boardphoto.rs), this session only
    pub board_photo: Option<crate::dialogs::boardphoto::BoardPhoto>,
    /// a one-finger touch that began on empty canvas: it pans instead of drawing a selection box
    pub(crate) touch_pan: bool,
    /// a finger is on the screen this frame (touch devices)
    pub(crate) touching: bool,
    pub sel: BTreeSet<i32>, // shape ids (groups expand in sel_changed)
    pub tool: Tool,
    pub snap: bool,
    pub view: Option<View>,
    pub dirty: bool,
    /// bumped on every document change: the app regenerates toolpaths when it moves
    pub rev: u64,
    pub show_paths: bool,
    /// every toolpath shown while drawing (View menu; the app copies its setting in each frame), else only the picked
    /// one / the one being set up
    pub all_paths: bool,
    /// the previews are from before the last edit (worked out again shortly): drawn faded
    pub paths_old: bool,
    /// which toolpaths' previews to draw (by index into the generated paths); None = all
    pub paths_only: Option<Vec<bool>>,
    /// cut-width bands under the toolpath lines (status-bar Width toggle)
    pub show_width: bool,
    pub bands: crate::bands::Bands,
    /// result of the last command for the status bar (the app takes it)
    pub status: String,
    pub requests: Vec<Request>,
    pub polygon_sides: i32,
    pub star_points: i32,
    pub star_ratio: f64,
    /// selected dimension (Select tool), -1 none
    pub sel_dim: i32,
    /// selected node in the Nodes tool
    pub node_shape: i32,
    pub node_idx: i32,
    pub(crate) node_part: u8, // 0 point, 1 in handle, 2 out handle
    pub(crate) sel_rect: geom::Rect,
    pub(crate) drag: Drag,
    pub(crate) press_w: Pt,
    pub(crate) box_a: Pt,
    pub(crate) box_b: Pt,
    pub(crate) poly: Vec<Pt>,
    pub(crate) pen: Contour,
    pub(crate) arc_pts: Vec<Pt>,
    /// freehand stroke being drawn (world points)
    pub(crate) freehand: Vec<Pt>,
    /// Nest dialog "Mirror parts for a better fit" (set by the dialog before nesting)
    pub nest_mirror: bool,
    /// Nest dialog "Inside the largest selected shape" (an offcut's outline)
    pub nest_boundary: bool,
    /// Nest dialog "Fill the rest with copies of the smallest part"
    pub nest_filler: bool,
    pub(crate) measure: Vec<Pt>,
    pub(crate) measure_text: String,
    /// the measured segment now (second point = the click, or the pointer before it): the Properties panel shows it
    pub(crate) measure_seg: Option<(Pt, Pt)>,
    pub(crate) dim_pts: Vec<Pt>,
    pub(crate) dim_anchors: Vec<(i32, i32)>,
    pub(crate) orig: Vec<DocShape>,
    pub(crate) orig_c: Contour,
    pub(crate) move_rect: geom::Rect,
    /// rotation centre set by dragging the pivot, valid for this selection (transform.rs)
    pub(crate) pivot: Option<(Pt, BTreeSet<i32>)>,
    pub(crate) xf_pivot: Pt,
    pub(crate) xf_angle: f64,
    /// the handle drag's transform so far (texts take its turn and scale when the drag ends)
    pub(crate) xf_last: gcarve_core::geom::Affine,
    pub(crate) xf_scale: (f64, f64),
    pub(crate) snap_mark: Option<Snap>,
    /// guide being dragged out of / along a ruler (index into doc.guides)
    pub(crate) guide_drag: Option<usize>,
    /// height of a bar the app draws over the bottom of the canvas (the Simulate playback bar): fits stay above it
    pub bottom_inset: f32,
    /// status-bar cursor readout (blank until the pointer has been over the canvas)
    pub(crate) cursor_text: String,
    pub(crate) gen_stale: bool,
    /// set by the app on every regenerate (preview cache key)
    pub gen_seq: u64,
    pub(crate) clip: String, // internal clipboard (the OS clipboard gets the same JSON)
    pub(crate) ctx_menu: CtxMenu,
    /// document point the context menu was opened at (Trim Here)
    pub(crate) ctx_at: Pt,
    pub(crate) size_popup: Option<crate::popups::SizePopup>,
    /// By-hand tabs (dialogs/op.rs): frames left in which clicks are handed to the toolpath dialog (tab_clicks)
    /// instead of the tool; tab_marks are drawn on the canvas
    pub(crate) tab_pick: u8,
    pub(crate) tab_clicks: Vec<Pt>,
    /// a drag while placing tabs: (where it started, where the pointer is, released)
    pub(crate) tab_drag: Option<(Pt, Pt, bool)>,
    pub(crate) tab_marks: Vec<[f64; 2]>,
    /// dev hook for screenshots (GCARVE_DIALOG=shapemenu|sizepopup), resolved once the canvas rect is known
    pub(crate) shot_hook: String,
    /// the start card's ✕ hides it for this document only
    pub(crate) card_closed: bool,
    pub(crate) node_popup: Option<crate::popups::NodePopup>,
    pub(crate) dim_popup: Option<crate::popups::DimPopup>,
    pub(crate) last_rect: Rect,
    /// Preferences ▸ Larger handles: hit / snap / node radii × 1.7 (Qt hitScale)
    pub hit_scale: f64,
    /// app-wide snap kinds / distance and grid / ruler settings (the app copies them in every frame)
    pub snap_prefs: SnapPrefs,
    pub grid: GridPrefs,
    pub(crate) relief_tex: crate::reliefs::ReliefTextures,
    /// model being dragged: id, its corner at the press
    pub(crate) relief_drag: (i32, Pt),
}

impl Canvas {
    pub fn with_doc(doc: Document) -> Self {
        Canvas { doc, ..Default::default() }
    }
    pub fn selected(&self) -> Option<i32> {
        self.sel.iter().next().copied()
    }
    pub fn selection_count(&self) -> usize {
        self.sel.len()
    }
    pub fn selection_has_group(&self) -> bool {
        self.sel.iter().any(|id| self.doc.find(*id).is_some_and(|s| s.group != 0))
    }
    /// Document point at the middle of the canvas
    pub fn view_centre(&self) -> Pt {
        self.view.as_ref().map(|v| v.to_doc(self.last_rect.center())).unwrap_or(pt(self.doc.material.w / 2.0, self.doc.material.h / 2.0))
    }
    pub fn zoom(&self) -> f64 {
        self.view.as_ref().map(|v| v.zoom as f64).unwrap_or(20.0)
    }
    pub fn set_tool(&mut self, t: Tool) {
        if t == self.tool {
            return;
        }
        self.tool = t;
        self.poly.clear();
        self.pen = Contour::default();
        self.arc_pts.clear();
        self.measure.clear();
        self.measure_text.clear();
        self.dim_pts.clear();
        self.dim_anchors.clear();
        if t != Tool::Nodes {
            self.node_idx = -1;
        }
        if t != Tool::Select {
            self.sel_dim = -1;
        }
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas {
            doc: Document::default(),
            sel: BTreeSet::new(),
            tool: Tool::Select,
            snap: false,
            view: None,
            dirty: false,
            rev: 1,
            show_paths: true,
            all_paths: true,
            paths_old: false,
            paths_only: None,
            all_sheets: false,
            board_photo: None,
            touch_pan: false,
            touching: false,
            show_width: true,
            bands: Default::default(),
            status: String::new(),
            requests: Vec::new(),
            polygon_sides: 6,
            star_points: 5,
            star_ratio: 0.5,
            sel_dim: -1,
            node_shape: -1,
            node_idx: -1,
            node_part: 0,
            sel_rect: geom::Rect::default(),
            drag: Drag::None,
            press_w: Pt::default(),
            box_a: Pt::default(),
            box_b: Pt::default(),
            poly: Vec::new(),
            pen: Contour::default(),
            arc_pts: Vec::new(),
            freehand: Vec::new(),
            nest_mirror: false,
            nest_boundary: false,
            nest_filler: false,
            measure: Vec::new(),
            measure_text: String::new(),
            measure_seg: None,
            dim_pts: Vec::new(),
            dim_anchors: Vec::new(),
            orig: Vec::new(),
            orig_c: Contour::default(),
            move_rect: geom::Rect::default(),
            pivot: None,
            xf_pivot: Pt::default(),
            xf_angle: 0.0,
            xf_last: Default::default(),
            xf_scale: (1.0, 1.0),
            snap_mark: None,
            guide_drag: None,
            bottom_inset: 0.0,
            cursor_text: String::new(),
            gen_stale: false,
            gen_seq: 0,
            clip: String::new(),
            ctx_menu: CtxMenu::None,
            ctx_at: Pt::default(),
            size_popup: None,
            tab_pick: 0,
            tab_clicks: Vec::new(),
            tab_drag: None,
            tab_marks: Vec::new(),
            shot_hook: String::new(),
            card_closed: false,
            node_popup: None,
            dim_popup: None,
            last_rect: Rect::NOTHING,
            hit_scale: 1.0,
            snap_prefs: SnapPrefs::default(),
            grid: GridPrefs { rulers: false, ..GridPrefs::default() }, // the app turns rulers on (View ▸ Rulers); tests keep the bare view
            relief_tex: Default::default(),
            relief_drag: (-1, Pt::default()),
        }
    }
}

/// "#rrggbb" -> colour (layer colours in .tlp files)
pub fn hex(c: &str) -> Color32 {
    let v = u32::from_str_radix(c.trim_start_matches('#'), 16).unwrap_or(0xd7dae0);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

use theme::c;
/// selected shapes and dimensions (Qt canvas_view.cpp)
pub(crate) const SELECTED: Color32 = c(0xff9f1a);
/// measure readout, snap ring, datum (Theme.highlightText)
pub(crate) const HIGHLIGHT: Color32 = c(0xffd34d);
pub(crate) const DIM_COLOR: Color32 = Color32::from_rgb(0x9a, 0xd0, 0xff);

impl Canvas {
    /// Zoom to the visible shapes (the board when there are none).
    pub fn zoom_fit(&mut self) {
        let mut b: Option<geom::Rect> = self.all_boards(); // all sheets shown: fit every board
        for s in self.doc.shapes.iter().filter(|_| !self.showing_all_sheets()) {
            // Qt Document::bounds: every shape, all sheets
            let r = geom::bounds(&s.c);
            b = Some(match b { Some(a) => geom::Rect { x0: a.x0.min(r.x0), y0: a.y0.min(r.y0), x1: a.x1.max(r.x1), y1: a.y1.max(r.y1) }, None => r });
        }
        let mut rect = self.view_area(self.last_rect);
        rect.max.y -= self.bottom_inset;
        match b {
            Some(b) if rect.width() > 0.0 && b.w() > 0.0 && b.h() > 0.0 && b.w().is_finite() && b.h().is_finite() => {
                let m = if rect.width() < 500.0 { 24.0 } else { 80.0 }; // phones: every pixel counts
                let zoom = (((rect.width() - m) as f64 / b.w()).min((rect.height() - m) as f64 / b.h())).clamp(1.0, MAX_ZOOM as f64) as f32;
                let c = rect.center();
                let mid = pt((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
                self.view = Some(View { zoom, origin: c + egui::vec2(-(mid.x as f32) * zoom, mid.y as f32 * zoom) });
            }
            _ => self.view = None,
        }
    }

    pub fn zoom_board(&mut self, rect: Rect) {
        if !(self.doc.material.w > 0.0 && self.doc.material.h > 0.0) {
            return self.zoom_fit(); // no board to fit (a size bound to 0): an infinite zoom broke the rulers
        }
        let mut rect = self.view_area(rect);
        rect.max.y -= self.bottom_inset;
        let (w, h) = (self.doc.material.w, self.doc.material.h);
        let m = if rect.width() < 500.0 { 24.0 } else { 80.0 }; // Qt zoomBoard: 40 px margin (phones: 12)
        let zoom = ((rect.width() - m) / w as f32).min((rect.height() - m) / h as f32).clamp(1.0, MAX_ZOOM);
        let origin = Pos2::new(rect.center().x - w as f32 * zoom / 2.0, rect.center().y + h as f32 * zoom / 2.0);
        self.view = Some(View { zoom, origin });
    }

    /// ruler strip width (0 when off)
    pub fn ruler_px(&self) -> f32 {
        if self.grid.rulers { RULER_PX } else { 0.0 }
    }
    /// the canvas right of / below the rulers (Qt fitRect keeps content out from under them)
    pub fn view_area(&self, rect: Rect) -> Rect {
        Rect::from_min_max(rect.min + egui::vec2(self.ruler_px(), self.ruler_px()), rect.max)
    }

    /// Grid pitch for snapping / nudging (Qt gridStep): the fixed spacing from Grid settings, else a decimal step in
    /// the project unit that is at least 10 px.
    pub fn grid_step(&self) -> f64 {
        if self.grid.spacing > 0.0 {
            return self.grid.spacing;
        }
        let mut step = self.doc.bare_unit_in * 0.001;
        let zoom = self.zoom();
        while step * zoom < 10.0 {
            step *= 10.0;
        }
        step
    }

    pub(crate) fn screen_pts(&self, view: &View, c: &Contour) -> Vec<Pos2> {
        flatten(c, 0.5 / view.zoom as f64).into_iter().map(|p| view.to_screen(p)).collect()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, gen: Option<&Generated>, sim: Option<&crate::simview::SimView>) -> String {
        let (full, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        // clicks on the rulers do nothing: the pointer tools only sense the area beside them
        let rect = self.view_area(full);
        let resp = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
        // the web canvas can be a few pixels on the first frame (page layout not done): a fit made then gives the 1 px/in
        // floor and a board off-screen, so fit again once there is room
        let tiny = |r: Rect| r.width() < 120.0 || r.height() < 120.0;
        let refit = tiny(self.last_rect) && !tiny(full);
        self.last_rect = full;
        if self.view.is_none() || refit {
            self.zoom_fit(); // Qt: open and F fit the shapes, the board when there are none (new documents too)
            if self.view.is_none() {
                self.zoom_board(full);
            }
        }
        let painter = ui.painter_at(full);
        painter.rect_filled(full, 0.0, theme::CANVAS_BG);
        if let Some(bp) = self.board_photo.as_mut().filter(|b| b.show && b.tex.is_none()) {
            bp.tex = Some(ui.ctx().load_texture("board-photo", egui::ColorImage::from_rgba_unmultiplied([bp.w, bp.h], &bp.rgba), egui::TextureOptions::LINEAR));
        }

        // pan / zoom: a middle or right drag does what Preferences › Mouse says (orbit has no meaning in 2D: it pans),
        // space+drag pans, the wheel / pinch zooms round the cursor
        let view = self.view.as_mut().unwrap();
        let mouse = crate::mouse::get();
        let (space, shift, ctrl) = ui.input(|i| (i.key_down(egui::Key::Space), i.modifiers.shift, i.modifiers.ctrl || i.modifiers.command));
        let button = [egui::PointerButton::Middle, egui::PointerButton::Secondary].into_iter().find(|b| resp.dragged_by(*b));
        match button.map(|b| mouse.drag(b, shift, ctrl)) {
            Some(gcarve_ui::nav::Act::Zoom) => {
                if let Some(at) = resp.interact_pointer_pos() {
                    let k = (-resp.drag_delta().y * 0.01 * if mouse.wheel_reverse { -1.0 } else { 1.0 } * mouse.zoom_speed).exp(); // up = closer
                    let new_zoom = (view.zoom * k).clamp(0.5, 5000.0);
                    view.origin = at + (view.origin - at) * (new_zoom / view.zoom);
                    view.zoom = new_zoom;
                }
            }
            Some(_) => view.origin += resp.drag_delta(),
            None if space && resp.dragged() => view.origin += resp.drag_delta(),
            None => {}
        }
        self.touching = ui.input(|i| i.any_touches());
        // touch screens: two fingers pan (and pinch zooms below); one finger on empty space pans too (`touch_pan`)
        if let Some(t) = ui.input(|i| i.multi_touch()).filter(|_| resp.hovered() || resp.dragged()) {
            view.origin += t.translation_delta;
        } else if self.touch_pan && resp.dragged_by(egui::PointerButton::Primary) {
            view.origin += resp.drag_delta();
        }
        if resp.hovered() {
            let (scroll, zoom_delta, hover) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
            if let Some(h) = hover {
                let factor = zoom_delta * (scroll.y * 0.0015 * if mouse.wheel_reverse { -1.0 } else { 1.0 } * mouse.zoom_speed).exp();
                if (factor - 1.0).abs() > 1e-4 {
                    let new_zoom = (view.zoom * factor).clamp(0.5, 5000.0);
                    let k = new_zoom / view.zoom;
                    view.origin = h + (view.origin - h) * k;
                    view.zoom = new_zoom;
                }
            }
        }
        let view = self.view.as_ref().unwrap();

        // simulation: the shaded cut beneath everything painted (Qt SimOverlay z: -1)
        if let Some(sv) = sim {
            if let (Some(tex), Some([x0, y0, w, h])) = (&sv.texture, sv.rect) {
                let r = Rect::from_two_pos(view.to_screen(pt(x0, y0 + h)), view.to_screen(pt(x0 + w, y0)));
                painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
        }
        self.paint_grid(&painter, view, full);
        let hover_doc = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)).map(|p| view.to_doc(p));
        self.paint_other_sheets(&painter, self.drag == Drag::Move, hover_doc);
        self.paint_front_side(&painter);
        if self.drag == Drag::None && self.tool == Tool::Select && hover_doc.is_some_and(|w| self.other_sheet_at(w).is_some() && self.hit_shape(w).is_none()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let view = self.view.as_ref().unwrap();
        let (bw, bh) = (self.doc.material.w, self.doc.material.h);
        let board = Rect::from_two_pos(view.to_screen(pt(0.0, 0.0)), view.to_screen(pt(bw, bh)));
        // the photo of the board, straightened onto it (dimmed so the vectors stay readable)
        if let Some((bp, tex)) = self.board_photo.as_ref().filter(|b| b.show && b.sheet == self.doc.active_sheet).and_then(|b| b.tex.as_ref().map(|t| (b, t))) {
            let _ = bp;
            painter.image(tex.id(), board, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::from_white_alpha(170));
        }
        painter.rect(board, 0.0, Color32::from_rgba_unmultiplied(138, 122, 90, 28), Stroke::new(1.0, c(0x8a7a5a)), egui::StrokeKind::Middle);
        self.paint_reliefs(ui.ctx(), &painter);
        let view = self.view.as_ref().unwrap();

        // toolpath previews (built once per regenerate in bands.rs): rapids dashed red, cut-width bands, cuts by depth
        if self.show_paths && !self.gen_stale {
            if let Some(g) = gen {
                self.bands.update(ui.ctx(), &self.doc, g, self.rev, self.gen_seq, self.paths_only.as_deref());
                let to = |c: &Vec<Pt>| -> Vec<Pos2> { c.iter().map(|p| view.to_screen(*p)).collect() };
                let fade = if self.paths_old { 0.35 } else { 1.0 };
                let rapid = Stroke::new(1.0, c(0xff6b6b).gamma_multiply(fade));
                for ch in &self.bands.rapids {
                    painter.extend(Shape::dashed_line(&to(ch), rapid, 4.0, 2.0));
                }
                if self.show_width {
                    if let (Some(tex), Some([x0, y0, w, h])) = (&self.bands.texture, self.bands.rect) {
                        let r = Rect::from_two_pos(view.to_screen(pt(x0, y0 + h)), view.to_screen(pt(x0 + w, y0)));
                        painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE.gamma_multiply(fade));
                    }
                }
                for (b, chains) in self.bands.cuts.iter().enumerate() {
                    let stroke = Stroke::new(1.0, crate::bands::cut_color(b).gamma_multiply(fade));
                    for ch in chains {
                        painter.add(Shape::line(to(ch), stroke));
                    }
                }
            }
        }
        // shapes: layer colour (dotted when locked), selected orange 2 px
        for s in self.doc.shapes.iter().filter(|s| self.doc.shape_visible(s)) {
            let pts = self.screen_pts(view, &s.c);
            let layer = self.doc.layers.get(s.layer as usize);
            if self.sel.contains(&s.id) {
                stroke_path(&painter, pts, s.c.closed, Stroke::new(2.0, SELECTED));
            } else if layer.is_some_and(|l| l.keep_out) && s.c.closed {
                // keep-out zones (clamps, screws): thick red, so nobody mistakes them for parts
                stroke_path(&painter, pts, true, Stroke::new(2.0, crate::theme::DANGER));
            } else {
                let stroke = Stroke::new(1.0, layer.map(|l| hex(&l.color)).unwrap_or(c(0xd7dae0)));
                if layer.is_some_and(|l| l.locked) {
                    let mut pts = pts;
                    if s.c.closed && !pts.is_empty() {
                        pts.push(pts[0]);
                    }
                    painter.extend(Shape::dotted_line(&pts, stroke.color, 3.0, 0.6));
                } else {
                    stroke_path(&painter, pts, s.c.closed, stroke);
                }
            }
        }

        // dimensions
        for (i, d) in self.doc.dimensions.iter().enumerate() {
            self.paint_dimension(&painter, view, d, i as i32 == self.sel_dim, false);
        }

        self.paint_constraints(&painter, view);
        self.paint_tab_marks(&painter, view);

        // tools: pointer handling + previews (tools.rs), then popups and context menus (popups.rs)
        let status = self.input(ui, &resp, rect, &painter);
        self.paint_nodes(&painter);
        self.paint_transform_frame(&painter);
        if let Some(m) = self.snap_mark {
            let s = self.view.as_ref().unwrap().to_screen(m.to);
            painter.circle_stroke(s, 6.0, Stroke::new(1.5, HIGHLIGHT));
            if !m.what.is_empty() {
                painter.text(s + egui::vec2(9.0, -8.0), egui::Align2::LEFT_BOTTOM, m.what, egui::FontId::proportional(11.0), HIGHLIGHT);
            }
        }
        self.guides_ui(ui, &painter, full);
        self.paint_rulers(&painter, full, ui.input(|i| i.pointer.hover_pos()));
        self.paint_guide_markers(&painter, full);
        self.popups_ui(ui, &resp);
        status
    }

    /// Top and left rulers in project units (Qt drawRulers): 1 / 2 / 5 x 10^n labels at least 70 px apart, minor ticks,
    /// the board's extent shaded, the pointer marked on both.
    fn paint_rulers(&self, painter: &egui::Painter, rect: Rect, pointer: Option<Pos2>) {
        let r = self.ruler_px();
        if r <= 0.0 {
            return;
        }
        let view = self.view.as_ref().unwrap();
        let k = 1.0 / self.doc.bare_unit_in; // display units per inch
        let Some((step, sub, dec)) = ruler_scale(view.zoom as f64 / k) else { return };
        let minor = step / sub as f64;
        let (bg, tick, text) = (c(0x2b2b2b), c(0x8a8a8a), c(0xb0b0b0));
        let top = Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + r));
        let left = Rect::from_min_max(rect.min, egui::pos2(rect.left() + r, rect.bottom()));
        painter.rect_filled(top, 0.0, bg);
        painter.rect_filled(left, 0.0, bg);
        let board = Color32::from_rgba_unmultiplied(138, 122, 90, 70);
        let (b0, b1) = (view.to_screen(pt(0.0, 0.0)), view.to_screen(pt(self.doc.material.w, self.doc.material.h)));
        let (ex, ey) = (left.right(), top.bottom());
        painter.rect_filled(Rect::from_min_max(egui::pos2(b0.x.max(ex), rect.top()), egui::pos2(b1.x.max(ex), ey)), 0.0, board);
        painter.rect_filled(Rect::from_min_max(egui::pos2(rect.left(), b1.y.max(ey)), egui::pos2(ex, b0.y.max(ey))), 0.0, board);
        let font = theme::font(9.0);
        let label = |v: f64| format!("{:.*}", dec, if v.abs() < minor / 2.0 { 0.0 } else { v });
        let len = |i: i64| if i % sub == 0 { r } else if sub == 10 && i % 5 == 0 { 7.0 } else { 4.0 };
        // top: x
        let (x0, x1) = (view.to_doc(egui::pos2(ex, 0.0)).x * k, view.to_doc(egui::pos2(rect.right(), 0.0)).x * k);
        for i in (x0 / minor).floor() as i64..=(x1 / minor).ceil() as i64 {
            let sx = view.to_screen(pt(i as f64 * minor / k, 0.0)).x.floor() + 0.5;
            if sx < ex {
                continue;
            }
            painter.vline(sx, (ey - len(i))..=ey, Stroke::new(1.0, tick));
            if i % sub == 0 {
                painter.text(egui::pos2(sx + 3.0, rect.top() + 1.0), egui::Align2::LEFT_TOP, label(i as f64 * minor), font.clone(), text);
            }
        }
        // left: y, labels reading upwards
        let (y0, y1) = (view.to_doc(egui::pos2(0.0, rect.bottom())).y * k, view.to_doc(egui::pos2(0.0, ey)).y * k);
        for i in (y0 / minor).floor() as i64..=(y1 / minor).ceil() as i64 {
            let sy = view.to_screen(pt(0.0, i as f64 * minor / k)).y.floor() + 0.5;
            if sy < ey {
                continue;
            }
            painter.hline((ex - len(i))..=ex, sy, Stroke::new(1.0, tick));
            if i % sub == 0 {
                let g = painter.layout_no_wrap(label(i as f64 * minor), font.clone(), text);
                painter.add(egui::epaint::TextShape::new(egui::pos2(rect.left() + 1.0, sy - 3.0), g, text).with_angle(-std::f32::consts::FRAC_PI_2));
            }
        }
        painter.hline(ex..=rect.right(), ey, Stroke::new(1.0, theme::BORDER));
        painter.vline(ex, ey..=rect.bottom(), Stroke::new(1.0, theme::BORDER));
        painter.rect_filled(Rect::from_min_size(rect.min, egui::vec2(r, r)), 0.0, bg); // corner: the unit
        painter.text(rect.min + egui::vec2(r / 2.0, r / 2.0), egui::Align2::CENTER_CENTER, units::name(&self.doc), font, text);
        // pointer markers
        if let Some(p) = pointer.filter(|p| rect.contains(*p)) {
            if p.x >= ex {
                painter.vline(p.x, rect.top()..=ey, Stroke::new(1.0, HIGHLIGHT));
            }
            if p.y >= ey {
                painter.hline(rect.left()..=ex, p.y, Stroke::new(1.0, HIGHLIGHT));
            }
        }
    }

    /// Qt drawGrid: grid in the project unit (Grid settings: shown, spacing, major every N), the X/Y axes, the
    /// machine XY zero.
    fn paint_grid(&self, painter: &egui::Painter, view: &View, rect: Rect) {
        let snap = |v: f32| v.floor() + 0.5; // crisp 1 px lines
        if self.grid.show {
            let mut step = self.grid_step();
            let n = self.grid.major.clamp(1, 100) as i64;
            while step * (view.zoom as f64) < 6.0 {
                step *= n.max(2) as f64; // a fixed spacing zoomed far out: every Nth line only
            }
            let sx = step as f32 * view.zoom;
            let (w0, w1) = (view.to_doc(rect.left_bottom()), view.to_doc(rect.right_top()));
            let (minor, major) = (Stroke::new(1.0, c(0x2e2e2e)), Stroke::new(1.0, c(0x3a3a3a)));
            for i in (w0.x / step).floor() as i64..=(w1.x / step).ceil() as i64 {
                let x = snap(view.origin.x + i as f32 * sx);
                painter.vline(x, rect.y_range(), if i % n == 0 { major } else { minor });
            }
            for i in (w0.y / step).floor() as i64..=(w1.y / step).ceil() as i64 {
                let y = snap(view.origin.y - i as f32 * sx);
                painter.hline(rect.x_range(), y, if i % n == 0 { major } else { minor });
            }
        }
        painter.hline(rect.x_range(), snap(view.origin.y), Stroke::new(1.0, c(0x8a3f3f)));
        painter.vline(snap(view.origin.x), rect.y_range(), Stroke::new(1.0, c(0x3f8a52)));
        // machine XY zero (Material::origin 0 BL, 1 BR, 2 TL, 3 TR, 4 centre)
        let m = &self.doc.material;
        let (fx, fy) = ([0.0, 1.0, 0.0, 1.0, 0.5], [0.0, 0.0, 1.0, 1.0, 0.5]);
        let k = m.origin.clamp(0, 4) as usize;
        let o = view.to_screen(pt(m.w * fx[k], m.h * fy[k]));
        let st = Stroke::new(2.0, HIGHLIGHT);
        painter.line_segment([o - egui::vec2(10.0, 0.0), o + egui::vec2(10.0, 0.0)], st);
        painter.line_segment([o - egui::vec2(0.0, 10.0), o + egui::vec2(0.0, 10.0)], st);
        painter.circle_stroke(o, 6.0, st);
        let font = theme::font(theme::FONT_SIZE);
        let base = o + egui::vec2(10.0, -8.0 - painter.fonts_mut(|f| f.row_height(&font)) * 0.8); // Qt drawText: y is the baseline
        painter.text(base, egui::Align2::LEFT_TOP, if m.z_bottom { i18n::t("XY0 · Z0 bed") } else { i18n::t("XY0 · Z0 top") }, font, HIGHLIGHT);
    }

    /// dashed links for the constraints touching the selection, labelled with their type and expression
    /// Tabs placed by hand: an orange square on the outline where each one goes.
    fn paint_tab_marks(&self, painter: &egui::Painter, view: &View) {
        for p in &self.tab_marks {
            let at = view.to_screen(pt(p[0], p[1]));
            painter.rect(Rect::from_center_size(at, egui::vec2(10.0, 10.0)), 1.0, c(0xff9a2e), Stroke::new(1.5, c(0x1b1c1f)), egui::StrokeKind::Middle);
        }
    }

    fn paint_constraints(&self, painter: &egui::Painter, view: &View) {
        let col = c(0x5aa1ec);
        let centre = |id: i32| self.doc.find(id).map(|s| { let b = geom::bounds(&s.c); view.to_screen(pt((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0)) });
        for k in &self.doc.constraints {
            if !self.sel.contains(&k.anchor) && !self.sel.contains(&k.target) {
                continue;
            }
            let (Some(a), Some(b)) = (centre(k.anchor), centre(k.target)) else { continue };
            painter.extend(Shape::dashed_line(&[a, b], Stroke::new(1.0, col), 4.0, 2.0));
            painter.circle_filled(b, 3.0, col);
            let label = if k.expr.is_empty() { k.kind.clone() } else { format!("{} {}", k.kind, k.expr) };
            painter.text(a + (b - a) / 2.0 + egui::vec2(6.0, -4.0), egui::Align2::LEFT_BOTTOM, label, theme::font(10.0), col);
        }
    }

    /// Node tool overlay: nodes of the selected shapes, handles around the selected node.
    fn paint_nodes(&self, painter: &egui::Painter) {
        if self.tool != Tool::Nodes {
            return;
        }
        let view = self.view.as_ref().unwrap();
        for id in &self.sel {
            let Some(s) = self.doc.find(*id) else { continue };
            for (i, n) in s.c.nodes.iter().enumerate() {
                let p = view.to_screen(n.p);
                let cur = *id == self.node_shape && i as i32 == self.node_idx;
                if self.node_shown(*id, i) {
                    for (has, h) in [(n.has_in, n.inh), (n.has_out, n.out)] {
                        if has {
                            let hs = view.to_screen(h);
                            painter.line_segment([p, hs], Stroke::new(1.0, theme::TOOL_DRAW));
                            painter.circle(hs, 3.5, theme::TOOL_DRAW, Stroke::new(1.0, theme::TOOL_DRAW));
                        }
                    }
                }
                let ni = i.to_string();
                let bound = ["n{}.x", "n{}.y", "s{}.len"].iter().any(|k| s.bind.contains_key(&k.replace("{}", &ni)));
                let fill = if cur { SELECTED } else if bound { c(0x5aa1ec) } else { Color32::WHITE };
                painter.rect(Rect::from_center_size(p, egui::vec2(7.0, 7.0)), 0.0, fill, Stroke::new(1.0, c(0x1b1d21)), egui::StrokeKind::Middle);
            }
        }
    }

    /// handles are shown for the selected node and its neighbours
    pub(crate) fn node_shown(&self, shape: i32, idx: usize) -> bool {
        if shape != self.node_shape || self.node_idx < 0 {
            return false;
        }
        let Some(s) = self.doc.find(shape) else { return false };
        let n = s.c.nodes.len() as i32;
        let d = (idx as i32 - self.node_idx).abs();
        d <= 1 || (s.c.closed && d == n - 1)
    }

    pub fn dimension_text(&self, d: &Dimension) -> String {
        let v = if d.axis == 3 { format!("{:.1}°", d.degrees()) } else { units::fmt(&self.doc, d.length()) };
        if d.label.is_empty() {
            v
        } else if d.label.contains("{v}") {
            d.label.replace("{v}", &v)
        } else {
            d.label.clone()
        }
    }

    pub(crate) fn paint_dimension(&self, painter: &egui::Painter, view: &View, dm: &Dimension, selected: bool, preview: bool) {
        if dm.axis == 3 {
            return self.paint_angle(painter, view, dm, selected, preview);
        }
        let (ea, eb) = dm.ends();
        let d = eb - ea;
        let l = dist(ea, eb);
        if l < 1e-9 {
            return;
        }
        let n = pt(-d.y / l, d.x / l);
        let off = n * dm.offset;
        let ext = n * (dm.offset + if dm.offset >= 0.0 { 0.08 } else { -0.08 });
        let (a, b) = (view.to_screen(dm.a), view.to_screen(dm.b));
        let (a2, b2) = (view.to_screen(ea + off), view.to_screen(eb + off));
        let (ae, be) = (view.to_screen(ea + ext), view.to_screen(eb + ext));
        let mut col = if selected { SELECTED } else { DIM_COLOR };
        let stroke = Stroke::new(1.0, col);
        let line = |p: Pos2, q: Pos2| {
            if preview {
                painter.add(Shape::dashed_line(&[p, q], stroke, 4.0, 3.0));
            } else {
                painter.line_segment([p, q], stroke);
            }
        };
        line(a, ae);
        line(b, be);
        line(a2, b2);
        if dm.shape_a >= 0 {
            painter.circle_stroke(a, 3.0, stroke);
        }
        if dm.shape_b >= 0 {
            painter.circle_stroke(b, 3.0, stroke);
        }
        let dir = b2 - a2;
        let sl = dir.length();
        if sl > 1e-6 {
            let dir = dir / sl;
            let nn = egui::vec2(-dir.y, dir.x);
            painter.add(Shape::convex_polygon(vec![a2, a2 + dir * 9.0 + nn * 3.0, a2 + dir * 9.0 - nn * 3.0], col, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![b2, b2 - dir * 9.0 + nn * 3.0, b2 - dir * 9.0 - nn * 3.0], col, Stroke::NONE));
        }
        if !dm.drive.is_empty() && !selected {
            col = Color32::from_rgb(0x5a, 0xa1, 0xec); // driving: accent
        }
        let text = self.dimension_text(dm);
        let galley = painter.layout_no_wrap(text, theme::font(theme::FONT_SIZE), col);
        let mid = a2 + (b2 - a2) / 2.0;
        let mut ang = (b2.y - a2.y).atan2(b2.x - a2.x);
        if ang > std::f32::consts::FRAC_PI_2 {
            ang -= std::f32::consts::PI; // keep text upright
        }
        if ang < -std::f32::consts::FRAC_PI_2 {
            ang += std::f32::consts::PI;
        }
        let (s, c) = ang.sin_cos();
        let size = galley.size();
        let rot = |v: egui::Vec2| mid + egui::vec2(v.x * c - v.y * s, v.x * s + v.y * c);
        // Qt: box (-w/2 - 5, -h - 6, w + 10, h + 4) above the line, rgba(27,29,33,230), radius 3, text centred
        let (bx, by, bw, bh) = (-size.x / 2.0 - 5.0, -size.y - 6.0, size.x + 10.0, size.y + 4.0);
        if ang.abs() < 1e-3 {
            painter.rect_filled(Rect::from_min_size(rot(egui::vec2(bx, by)), egui::vec2(bw, bh)), 3.0, Color32::from_rgba_unmultiplied(27, 29, 33, 230));
        } else {
            let pts = vec![rot(egui::vec2(bx, by)), rot(egui::vec2(bx + bw, by)), rot(egui::vec2(bx + bw, by + bh)), rot(egui::vec2(bx, by + bh))];
            painter.add(Shape::convex_polygon(pts, Color32::from_rgba_unmultiplied(27, 29, 33, 230), Stroke::NONE));
        }
        painter.add(egui::epaint::TextShape::new(rot(egui::vec2(-size.x / 2.0, by + 2.0)), galley, col).with_angle(ang));
    }

    /// angular dimension: both arms out to the arc, the arc with arrow ends, the angle on a label at its middle
    fn paint_angle(&self, painter: &egui::Painter, view: &View, dm: &Dimension, selected: bool, preview: bool) {
        let col = if selected { SELECTED } else { DIM_COLOR };
        let stroke = Stroke::new(1.0, col);
        let (a0, sweep) = dm.arc();
        let r = dm.offset.abs().max(1e-3);
        let at = |t: f64| view.to_screen(dm.vertex + pt(t.cos(), t.sin()) * r);
        let v = view.to_screen(dm.vertex);
        let arm = |p: Pt, t: f64| {
            let end = at(t);
            // the drawn arm reaches past the arc a little, or to the point when that is further out
            let far = if dist(dm.vertex, p) > r { view.to_screen(p) } else { end + (end - v).normalized() * 6.0 };
            painter.line_segment([v, far], stroke);
        };
        arm(dm.a, a0);
        arm(dm.b, a0 + sweep);
        let n = ((sweep.abs() * r * f64::from(view.zoom)) / 4.0).clamp(8.0, 128.0) as usize;
        let pts: Vec<Pos2> = (0..=n).map(|k| at(a0 + sweep * k as f64 / n as f64)).collect();
        if preview {
            painter.add(Shape::dashed_line(&pts, stroke, 4.0, 3.0));
        } else {
            painter.add(Shape::line(pts.clone(), stroke));
        }
        for (tip, before) in [(pts[0], pts[1]), (pts[n], pts[n - 1])] {
            let dir = (before - tip).normalized();
            let nn = egui::vec2(-dir.y, dir.x);
            painter.add(Shape::convex_polygon(vec![tip, tip + dir * 9.0 + nn * 3.0, tip + dir * 9.0 - nn * 3.0], col, Stroke::NONE));
        }
        let galley = painter.layout_no_wrap(self.dimension_text(dm), theme::font(theme::FONT_SIZE), col);
        let mid = at(a0 + sweep / 2.0);
        let out = (mid - v).normalized() * (galley.size().length() / 2.0 + 6.0);
        let rect = Rect::from_center_size(mid + out, galley.size() + egui::vec2(10.0, 4.0));
        painter.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(27, 29, 33, 230));
        painter.galley(rect.min + egui::vec2(5.0, 2.0), galley, col);
    }

    pub fn touch(&mut self) {
        self.dirty = true;
        self.rev += 1;
    }

    pub fn undo(&mut self) {
        if self.doc.undo() {
            self.mutated();
        }
    }
    pub fn redo(&mut self) {
        if self.doc.redo() {
            self.mutated();
        }
    }

    /// Keyboard: tool keys, delete, escape, arrows, undo/redo, select all, duplicate, group, plus the Copy/Cut/Paste
    /// events egui raises for the platform shortcuts (which also carry the OS clipboard text).
    pub fn keys(&mut self, ctx: &egui::Context) {
        // not while typing in a field, and not behind a dialog: letters typed at a dialog must never pick tools
        if ctx.egui_wants_keyboard_input() || ctx.memory(|m| m.top_modal_layer().is_some()) {
            return;
        }
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        for e in &events {
            match e {
                egui::Event::Copy => self.copy(ctx),
                egui::Event::Cut => self.cut(ctx),
                egui::Event::Paste(text) => {
                    self.paste(text);
                }
                _ => {}
            }
        }
        let pressed = |k: egui::Key| events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, .. } if *key == k));
        if mods.command {
            if pressed(egui::Key::Z) {
                if mods.shift { self.redo() } else { self.undo() }
            }
            if pressed(egui::Key::A) { self.select_all() }
            if pressed(egui::Key::D) { self.duplicate() }
            if pressed(egui::Key::G) {
                if mods.shift { self.ungroup_selection() } else { self.group_selection() }
            }
            return;
        }
        if pressed(egui::Key::Delete) || pressed(egui::Key::Backspace) {
            // while drawing a polyline or a curve: take back the last point clicked
            if self.tool == Tool::Polyline && !self.poly.is_empty() {
                self.poly.pop();
            } else if self.tool == Tool::Pen && !self.pen.nodes.is_empty() {
                self.pen.nodes.pop();
            } else {
                self.delete_selected();
            }
        }
        if pressed(egui::Key::Escape) {
            self.escape();
        }
        if pressed(egui::Key::Enter) {
            if self.tool == Tool::Pen { self.finish_pen(false) } else { self.finish_poly(false) }
        }
        let st = self.grid_step() * if mods.shift { 10.0 } else { 1.0 };
        let nudge = [(egui::Key::ArrowLeft, -st, 0.0), (egui::Key::ArrowRight, st, 0.0), (egui::Key::ArrowUp, 0.0, st), (egui::Key::ArrowDown, 0.0, -st)];
        for (k, dx, dy) in nudge {
            if pressed(k) {
                self.transform_selection(&geom::Affine::translate(dx, dy));
            }
        }
        if mods.alt || mods.ctrl {
            return;
        }
        for (t, _, k) in Tool::ALL {
            if pressed(k) {
                self.set_tool(t);
            }
        }
        if pressed(egui::Key::F) {
            self.view = None;
        }
    }

    fn escape(&mut self) {
        if !self.poly.is_empty() {
            self.poly.clear();
        } else if !self.arc_pts.is_empty() {
            self.arc_pts.clear();
        } else if !self.measure.is_empty() {
            self.measure.clear();
            self.measure_text.clear();
        } else if !self.dim_pts.is_empty() {
            self.dim_pts.clear();
            self.dim_anchors.clear();
        } else if self.sel_dim >= 0 {
            self.sel_dim = -1;
        } else if !self.pen.nodes.is_empty() {
            self.pen = Contour::default();
        } else {
            self.sel.clear();
            self.node_idx = -1;
            self.sel_changed();
        }
    }

    pub fn selection_size(&self) -> Option<geom::Rect> {
        (!self.sel_rect.is_empty()).then_some(self.sel_rect)
    }
}

/// Toolpath kind colours (Theme.kindColor in the Qt app)
pub fn kind_color(k: OpType) -> Color32 {
    match k {
        OpType::Profile => Color32::from_rgb(0x6c, 0xb6, 0xff),
        OpType::Pocket => Color32::from_rgb(0x38, 0xc7, 0xd0),
        OpType::Drill => Color32::from_rgb(0xff, 0x6b, 0x6b),
        OpType::VCarve => Color32::from_rgb(0xff, 0xb3, 0x47),
        OpType::Bore => Color32::from_rgb(0xc9, 0xa9, 0xff),
        OpType::Inlay => Color32::from_rgb(0xf0, 0x8f, 0xd0),
        OpType::Flute => Color32::from_rgb(0x7b, 0xd8, 0x8f),
        OpType::Relief => Color32::from_rgb(0xd7, 0xb2, 0x6a),
        OpType::Chamfer => Color32::from_rgb(0xff, 0xd3, 0x4d),
        OpType::Prism => Color32::from_rgb(0xff, 0x8f, 0x6b),
        OpType::Imported => Color32::from_rgb(0xb0, 0xb4, 0xbc),
        OpType::Texture => Color32::from_rgb(0xe0, 0xa8, 0x7a),
        OpType::Photo => Color32::from_rgb(0xa8, 0xc8, 0xe8),
    }
}

/// Points along a G2/G3 arc from a to b with centre offset (i, j) from a.
pub fn arc_points(a: Pt, b: Pt, i: f64, j: f64, ccw: bool) -> Vec<Pt> {
    let c = pt(a.x + i, a.y + j);
    let r = (i * i + j * j).sqrt();
    let a0 = (a.y - c.y).atan2(a.x - c.x);
    let mut sweep = (b.y - c.y).atan2(b.x - c.x) - a0;
    if ccw {
        while sweep <= 1e-12 {
            sweep += std::f64::consts::TAU;
        }
    } else {
        while sweep >= -1e-12 {
            sweep -= std::f64::consts::TAU;
        }
    }
    let n = ((sweep.abs() * r / 0.01).ceil() as usize).clamp(4, 400);
    (0..=n).map(|k| {
        let t = a0 + sweep * k as f64 / n as f64;
        pt(c.x + r * t.cos(), c.y + r * t.sin())
    }).collect()
}

/// A shape's outline. Where the path turns straight back on itself (a speck in a VCarve or SVG file) egui's corner
/// join has no direction and shoots a spike across the screen, so the outline is drawn in pieces split there.
pub(crate) fn stroke_path(painter: &egui::Painter, pts: Vec<Pos2>, closed: bool, stroke: Stroke) {
    match split_at_reversals(pts, closed) {
        Ok(pts) => {
            painter.add(if closed { Shape::closed_line(pts, stroke) } else { Shape::line(pts, stroke) });
        }
        Err(pieces) => painter.extend(pieces.into_iter().map(|p| Shape::line(p, stroke))),
    }
}

/// Ok(the points) when nothing doubles back, else Err(open pieces that each run from one reversal to the next).
fn split_at_reversals(mut pts: Vec<Pos2>, closed: bool) -> Result<Vec<Pos2>, Vec<Vec<Pos2>>> {
    pts.dedup_by(|a, b| a.distance(*b) < 0.01);
    let n = pts.len();
    let reverses = |i: usize| {
        let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let (u, v) = (b - a, c - b);
        u.dot(v) < -0.999 * u.length() * v.length()
    };
    let cuts: Vec<usize> = (0..n).filter(|&i| n > 2 && (closed || (i > 0 && i + 1 < n)) && reverses(i)).collect();
    if cuts.is_empty() {
        return Ok(pts);
    }
    let start = if closed { cuts[0] } else { 0 };
    let mut ring = pts.clone();
    ring.rotate_left(start); // closed: start at a reversal, so every piece runs reversal to reversal
    if closed {
        ring.push(ring[0]);
    }
    let mut pieces = Vec::new();
    let mut piece = vec![ring[0]];
    for (k, q) in ring.iter().enumerate().skip(1) {
        piece.push(*q);
        if k + 1 < ring.len() && cuts.contains(&((k + start) % n)) {
            pieces.push(std::mem::replace(&mut piece, vec![*q]));
        }
    }
    pieces.push(piece);
    Err(pieces)
}

/// most pixels per inch a fit gives (a speck or a 0-size board made it infinite)
const MAX_ZOOM: f32 = 1e6;

/// ruler labels: (step, minor ticks per step, decimals) for 1 / 2 / 5 × 10^n labels at least 70 px apart;
/// None when the scale is not a usable number (a decimals count from an infinite zoom crashed the web app)
fn ruler_scale(px_per_unit: f64) -> Option<(f64, i64, usize)> {
    if !(px_per_unit.is_finite() && px_per_unit > 0.0) {
        return None;
    }
    let mut step = 10f64.powf((70.0 / px_per_unit).log10().floor());
    for f in [1.0, 2.0, 5.0, 10.0] {
        if f * step * px_per_unit >= 70.0 {
            step *= f;
            break;
        }
    }
    if !(step.is_finite() && step > 0.0) {
        return None;
    }
    let sub: i64 = if step * px_per_unit / 10.0 >= 5.0 { 10 } else if step * px_per_unit / 5.0 >= 5.0 { 5 } else { 2 };
    Some((step, sub, ((-(step.log10() + 1e-9).floor()).max(0.0) as usize).min(9)))
}

#[cfg(test)]
mod stroke_tests {
    use super::*;

    #[test]
    fn a_board_or_drawing_of_no_size_never_breaks_the_zoom_or_the_rulers() {
        for z in [f64::INFINITY, 0.0, f64::NAN, -3.0] {
            assert!(ruler_scale(z).is_none(), "{z}");
        }
        let (step, _, dec) = ruler_scale(1e12).unwrap();
        assert!(step > 0.0 && dec <= 9);
        assert_eq!(ruler_scale(20.0).map(|r| (r.0, r.2)), Some((5.0, 0)));
        let mut c = Canvas::default();
        c.doc.material.w = 0.0;
        c.zoom_board(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
        assert!(c.view.as_ref().is_none_or(|v| v.zoom.is_finite() && v.zoom <= MAX_ZOOM));
        c.doc.material.w = 1e-30;
        c.zoom_board(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
        assert!(c.view.as_ref().is_some_and(|v| v.zoom <= MAX_ZOOM));
    }

    #[test]
    fn delete_takes_back_the_last_polyline_point() {
        let mut c = Canvas { tool: Tool::Polyline, ..Default::default() };
        c.poly = vec![pt(0.0, 0.0), pt(1.0, 0.0), pt(1.0, 1.0)];
        let id = c.doc.add(gcarve_core::geom::make_rect(0.0, 0.0, 1.0, 1.0), None);
        c.sel.insert(id);
        let ctx = egui::Context::default();
        let key = |k| egui::RawInput { events: vec![egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }], ..Default::default() };
        ctx.run_ui(key(egui::Key::Backspace), |ui| c.keys(ui.ctx())).textures_delta.clear();
        assert_eq!(c.poly.len(), 2, "the last point goes");
        assert!(c.doc.find(id).is_some(), "the selection stays while drawing");
        ctx.run_ui(key(egui::Key::Delete), |ui| c.keys(ui.ctx())).textures_delta.clear();
        assert_eq!(c.poly.len(), 1);
    }

    #[test]
    fn outlines_that_double_back_are_drawn_in_pieces() {
        let p = |x: f32, y: f32| Pos2::new(x, y);
        // a square has no reversal: one closed line, as before
        assert!(split_at_reversals(vec![p(0., 0.), p(10., 0.), p(10., 10.), p(0., 10.)], true).is_ok());
        // out and straight back (a speck): open pieces that start and end at the turn, so no join is drawn there
        let Err(pieces) = split_at_reversals(vec![p(0., 0.), p(10., 0.), p(5., 0.), p(5., 3.)], true) else { panic!("not split") };
        assert_eq!(pieces, vec![vec![p(10., 0.), p(5., 0.), p(5., 3.), p(0., 0.), p(10., 0.)]]);
        let Err(open) = split_at_reversals(vec![p(0., 0.), p(10., 0.), p(2., 0.)], false) else { panic!("open not split") };
        assert_eq!(open, vec![vec![p(0., 0.), p(10., 0.)], vec![p(10., 0.), p(2., 0.)]]);
    }
}
