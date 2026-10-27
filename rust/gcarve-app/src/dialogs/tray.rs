//! Tray Maker (File › New Tray…): a serving tray (a pocket with a bowl bit, so the floor meets the walls in a round,
//! optional hand holds through the end rims) or a cutting board (a juice groove with a ball nose), cut out with tabs,
//! in a new tab. Bits from the user's library; a 1 in bowl bit with a 1/4 in corner radius, a 1/2 in ball nose and a
//! 1/4 in end mill are added when missing.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::sign::{bit, outline_of};
use super::{hint, unit_label, Host};
use crate::canvas::Canvas;
use eframe::egui;
use gcarve_core::document::{Op, OpType, Side, Tool, ToolType};
use gcarve_core::geom::make_rect;
use gcarve_core::vecops::fillet_contours;

#[derive(Clone, Debug)]
pub struct TraySpec {
    /// false = serving tray, true = cutting board with a juice groove
    pub board: bool,
    pub w: f64,
    pub h: f64,
    pub t: f64,
    /// 0 rectangle, 1 rounded corners, 2 oval
    pub shape: u8,
    /// tray: pocket depth; board: groove depth
    pub depth: f64,
    /// tray: rim width; board: groove distance from the edge
    pub rim: f64,
    /// tray: hand holds through the end rims
    pub handles: bool,
}

const SHAPES: [&str; 3] = ["Rectangle", "Rounded corners", "Oval"]; // i18n

pub struct TrayDialog {
    board: bool,
    /// w, h, t, depth, rim
    size: [String; 5],
    shape: u8,
    handles: bool,
    /// OK pressed: the app makes it in a new tab
    pub done: Option<TraySpec>,
}

impl TrayDialog {
    pub fn new(doc: &gcarve_core::document::Document) -> Self {
        TrayDialog { board: false, size: [18.0, 12.0, 1.0, 0.5, 1.0].map(|v| crate::units::text(doc, v)), shape: 1, handles: true, done: None }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let (mut open, mut ok, mut cancel) = (true, false, false);
        let doc = &host.canvas.doc;
        crate::widgets::Dialog::new(i18n::t("New Tray or Board")).open(&mut open).width(420.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.board, false, i18n::t("Serving tray"));
                ui.selectable_value(&mut self.board, true, i18n::t("Cutting board"));
            });
            egui::Grid::new("tray").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                let labels = ["Board width", i18n::t("Board height"), i18n::t("Board thickness"), if self.board { i18n::t("Groove depth") } else { i18n::t("Tray depth") }, if self.board { i18n::t("Groove in from the edge") } else { i18n::t("Rim width") }];
                for (i, label) in labels.iter().enumerate() {
                    ui.label(unit_label(label, doc));
                    crate::units::num_field(ui, label, &mut self.size[i], doc, true, 110.0);
                    ui.end_row();
                }
                ui.label(i18n::t("Shape"));
                egui::ComboBox::from_id_salt("tray-shape").selected_text(i18n::t(SHAPES[self.shape as usize])).show_ui(ui, |ui| {
                    for (i, s) in SHAPES.iter().enumerate() {
                        ui.selectable_value(&mut self.shape, i as u8, i18n::t(s));
                    }
                });
                ui.end_row();
            });
            if !self.board {
                crate::widgets::checkbox(ui, &mut self.handles, i18n::t("Hand holds through the ends"));
            }
            hint(ui, if self.board { i18n::t("Opens as a new project: a juice groove (1/2 in ball nose) and a cut-out with tabs.") } else { i18n::t("Opens as a new project: the tray pocketed with a bowl bit (round where the floor meets the walls), hand holds, and a cut-out with tabs.") });
            let (o, c) = crate::widgets::ok_cancel(ui);
            ok = o;
            cancel = c;
        });
        if cancel {
            return false;
        }
        if ok {
            let v: Result<Vec<f64>, String> = self.size.iter().map(|s| crate::units::get(&host.canvas.doc, s, true)).collect();
            match v {
                Err(e) => *host.message = e,
                Ok(n) if n[0] < 4.0 || n[1] < 3.0 || n[2] <= 0.0 => *host.message = i18n::t("New tray: the board must be at least 4 × 3 in, with a thickness.").into(),
                Ok(n) if n[3] <= 0.0 || n[3] > n[2] - 0.25 => *host.message = i18n::t("New tray: the depth must leave at least 1/4 in of floor.").into(),
                Ok(n) if n[4] < 0.5 || 2.0 * n[4] + 1.0 > n[0].min(n[1]) => *host.message = i18n::t("New tray: the rim must be at least 1/2 in and leave room inside.").into(),
                Ok(n) => {
                    self.done = Some(TraySpec { board: self.board, w: n[0], h: n[1], t: n[2], shape: self.shape, depth: n[3], rim: n[4], handles: self.handles && !self.board });
                    return false;
                }
            }
        }
        open
    }
}

/// A bowl bit from the project's tools (a ball nose with a corner radius under half its diameter, the widest), else
/// one is added.
fn bowl_bit(c: &mut Canvas) -> i32 {
    let bowl = |t: &Tool| t.kind == ToolType::BallNose && t.radius > 0.0 && 2.0 * t.radius < t.diameter - 1e-6;
    if let Some((i, _)) = c.doc.tools.iter().enumerate().filter(|(_, t)| bowl(t)).max_by(|a, b| a.1.diameter.total_cmp(&b.1.diameter)) {
        return i as i32;
    }
    c.doc.tools.push(Tool { name: i18n::t("Bowl Bit 1 in, 1/4 in radius").into(), kind: ToolType::BallNose, diameter: 1.0, radius: 0.25, stepdown: 0.25, feed_xy: 120.0, feed_z: 30.0, rpm: 14000, flutes: 2, ..Default::default() });
    c.doc.tools.len() as i32 - 1
}

/// Fills an empty canvas with the tray or board job. Returns what was made, for the status line.
pub fn build(c: &mut Canvas, s: &TraySpec) -> String {
    let m = &mut c.doc.material;
    (m.w, m.h, m.t) = (s.w, s.h, s.t);
    let a = c.doc.active_sheet.max(0) as usize;
    if let Some(sh) = c.doc.sheets.get_mut(a) {
        *sh = c.doc.material.clone();
    }
    let mill = bit(c, ToolType::EndMill, 0.25, i18n::t("1/4 End Mill (0.25 inch)"));
    let edge = 0.3; // the cut-out, in from the board edge
    let cut = c.doc.add(outline_of(s.w, s.h, s.shape, edge), None);
    let mut ops = Vec::new();
    let mut made = Vec::new();
    if s.board {
        let ball = bit(c, ToolType::BallNose, 0.5, i18n::t("1/2 Ball Nose (0.5 inch)"));
        let groove = c.doc.add(outline_of(s.w, s.h, s.shape.max(1), edge + s.rim), None);
        ops.push(Op { kind: OpType::Profile, name: i18n::t("Juice groove").into(), tool: ball, shape_ids: vec![groove], depth: s.depth, side: Side::On, ..Default::default() });
        made.push(i18n::t("a juice groove"));
    } else {
        let bowl = bowl_bit(c);
        let br = c.doc.tools[bowl as usize].diameter / 2.0;
        // the ends get room for the hand holds: a 3.5 × 0.875 in slot with 0.6 in of wood either side
        let hold = (3.5f64.min(s.h * 0.5), 0.875);
        let end = if s.handles { s.rim.max(hold.1 + 1.2) } else { s.rim };
        let (x0, x1, y0, y1) = (edge + end, s.w - edge - end, edge + s.rim, s.h - edge - s.rim);
        if x1 - x0 < 2.0 * br || y1 - y0 < 2.0 * br {
            return i18n::t("New tray: too small inside for the bowl bit.").into();
        }
        let inner = if s.shape == 2 {
            gcarve_core::geom::make_ellipse((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0)
        } else {
            let r = if s.shape == 1 { (x1 - x0).min(y1 - y0) * 0.15 } else { br };
            let rect = make_rect(x0, y0, x1, y1);
            fillet_contours(std::slice::from_ref(&rect), r.max(br), true, false).into_iter().next().unwrap_or(rect)
        };
        let pocket = c.doc.add(inner, None);
        ops.push(Op { kind: OpType::Pocket, name: i18n::t("Tray").into(), tool: bowl, shape_ids: vec![pocket], depth: s.depth, stepover: 0.4, ..Default::default() });
        made.push(i18n::t("the tray pocketed with a bowl bit"));
        if s.handles {
            let cy = s.h / 2.0;
            let mut ids = Vec::new();
            for cx in [edge + end / 2.0, s.w - edge - end / 2.0] {
                let slot = make_rect(cx - hold.1 / 2.0, cy - hold.0 / 2.0, cx + hold.1 / 2.0, cy + hold.0 / 2.0);
                let slot = fillet_contours(std::slice::from_ref(&slot), hold.1 / 2.0 - 0.01, true, false).into_iter().next().unwrap_or(slot);
                ids.push(c.doc.add(slot, None));
            }
            ops.push(Op { kind: OpType::Profile, name: i18n::t("Hand holds").into(), tool: mill, shape_ids: ids, depth: s.t, side: Side::Inside, ..Default::default() });
            made.push(i18n::t("hand holds"));
        }
    }
    ops.push(Op { kind: OpType::Profile, name: i18n::t("Cut out").into(), tool: mill, shape_ids: vec![cut], depth: s.t, side: Side::Outside, tabs: true, tab_count: 4, tab_corners: true, ..Default::default() });
    c.doc.ops = ops;
    c.sel.clear();
    c.mutated();
    format!("New {}: {} and a cut-out with tabs. Check it in Simulate, then Export.", if s.board { i18n::t("cutting board") } else { "tray" }, made.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::geom::bounds;

    #[test]
    fn a_tray_and_a_board_come_out_as_clean_jobs_inside_the_board() {
        for (board, shape, handles) in [(false, 0, true), (false, 1, true), (false, 2, false), (true, 1, false), (true, 2, false)] {
            let mut c = Canvas::default();
            let s = TraySpec { board, w: 18.0, h: 12.0, t: 1.0, shape, depth: 0.5, rim: 1.0, handles };
            let msg = build(&mut c, &s);
            assert!(msg.starts_with("New "), "{msg}");
            assert_eq!(c.doc.ops.last().unwrap().name, "Cut out", "cut free last");
            assert_eq!(c.doc.ops.len(), if handles { 3 } else { 2 });
            let cut = bounds(&c.doc.find(c.doc.ops.last().unwrap().shape_ids[0]).unwrap().c);
            for op in &c.doc.ops[..c.doc.ops.len() - 1] {
                for id in &op.shape_ids {
                    let b = bounds(&c.doc.find(*id).unwrap().c);
                    assert!(b.x0 > cut.x0 + 0.5 && b.x1 < cut.x1 - 0.5 && b.y0 > cut.y0 + 0.5 && b.y1 < cut.y1 - 0.5, "{board} {shape}: {b:?} inside {cut:?}");
                }
            }
            if !board {
                let t = &c.doc.tools[c.doc.ops[0].tool as usize];
                assert!(t.kind == ToolType::BallNose && t.radius > 0.0 && t.radius < t.diameter / 2.0, "a bowl bit");
            }
            let gen = gcarve_core::pipeline::regenerate(&c.doc);
            assert!(gen.warnings.is_empty(), "{board} {shape}: {:?}", gen.warnings);
            assert!(gcarve_core::pipeline::order_issues(&c.doc).is_empty());
        }
    }

    /// look at one: GCARVE_TRAY_OUT=x.png cargo test -p gcarve-app tray_picture -- --ignored
    #[test]
    #[ignore]
    fn tray_picture() {
        let Ok(out) = std::env::var("GCARVE_TRAY_OUT") else { return };
        let mut c = Canvas::default();
        build(&mut c, &TraySpec { board: false, w: 18.0, h: 12.0, t: 1.0, shape: 1, depth: 0.5, rim: 1.0, handles: true });
        let gen = gcarve_core::pipeline::regenerate(&c.doc);
        let (rgba, w, h, _) = crate::setupsheet::carved(&c.doc, &gen, 600_000).unwrap();
        image::RgbaImage::from_raw(w, h, rgba).unwrap().save(out).unwrap();
    }
}
