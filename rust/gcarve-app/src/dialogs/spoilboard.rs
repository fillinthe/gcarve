//! Spoilboard Maker (File › New Spoilboard…): a board the size of the machine's table with a grid of holes for
//! threaded inserts (clamps screw into them), optional engraved grid lines every so many inches, and a pass that
//! surfaces the top flat, in a new tab. Holes are cut with a 1/4 end mill, so any insert size works.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::sign::bit;
use super::{hint, unit_label, Host};
use crate::canvas::Canvas;
use eframe::egui;
use gcarve_core::document::{Op, OpType, Side, ToolType};
use gcarve_core::geom::{make_ellipse, make_polyline, make_rect, pt};

#[derive(Clone, Debug)]
pub struct SpoilSpec {
    pub w: f64,
    pub h: f64,
    pub t: f64,
    /// distance between insert holes
    pub spacing: f64,
    /// insert hole diameter and depth
    pub hole: f64,
    pub depth: f64,
    /// engraved grid lines every `spacing`, between the holes
    pub lines: bool,
    pub surface: bool,
}

pub struct SpoilDialog {
    /// w, h, t, spacing, hole, depth
    size: [String; 6],
    lines: bool,
    surface: bool,
    pub done: Option<SpoilSpec>,
}

impl SpoilDialog {
    pub fn new(doc: &gcarve_core::document::Document) -> Self {
        // the table of the machine last chosen on this device, else the open board
        let (w, h) = match super::prefs::last_machine() {
            Some((m, _)) if m.travel_x > 0.0 && m.travel_y > 0.0 => (m.travel_x, m.travel_y),
            _ => (doc.material.w, doc.material.h),
        };
        // 1/4-20 inserts mostly want a 3/8 in hole, 1/2 in deep
        SpoilDialog { size: [w, h, 0.75, 4.0, 0.375, 0.5].map(|v| crate::units::text(doc, v)), lines: false, surface: true, done: None }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let (mut open, mut ok, mut cancel) = (true, false, false);
        let doc = &host.canvas.doc;
        crate::widgets::Dialog::new(i18n::t("New Spoilboard")).open(&mut open).width(420.0).show(ctx, |ui| {
            egui::Grid::new("spoil").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                for (i, label) in ["Board width", i18n::t("Board height"), i18n::t("Board thickness"), i18n::t("Hole spacing"), i18n::t("Insert hole size"), "Insert hole depth"].iter().enumerate() {
                    ui.label(unit_label(label, doc));
                    crate::units::num_field(ui, label, &mut self.size[i], doc, true, 110.0);
                    ui.end_row();
                }
            });
            crate::widgets::checkbox(ui, &mut self.lines, i18n::t("Engrave grid lines between the holes"));
            crate::widgets::checkbox(ui, &mut self.surface, i18n::t("Surface the top flat first"));
            hint(ui, i18n::t("Opens as a new project sized to your machine's table: a grid of holes for threaded inserts (1/4-20 inserts mostly take a 3/8 in hole), cut with a 1/4 end mill. Screw the board down first and zero on the table corner."));
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
                Ok(n) if n[0] < 6.0 || n[1] < 6.0 || n[2] <= 0.0 => *host.message = i18n::t("New spoilboard: at least 6 × 6 in, with a thickness.").into(),
                Ok(n) if n[3] < 1.0 || n[3] > n[0].min(n[1]) => *host.message = i18n::t("New spoilboard: holes 1 in apart or more, and fitting on the board.").into(),
                Ok(n) if n[4] < 0.27 || n[4] > n[3] / 2.0 => *host.message = i18n::t("New spoilboard: the hole must be wider than the 1/4 in bit and well under the spacing.").into(),
                Ok(n) if n[5] <= 0.0 || n[5] > n[2] => *host.message = i18n::t("New spoilboard: the hole depth must be within the board.").into(),
                Ok(n) => {
                    self.done = Some(SpoilSpec { w: n[0], h: n[1], t: n[2], spacing: n[3], hole: n[4], depth: n[5], lines: self.lines, surface: self.surface });
                    return false;
                }
            }
        }
        open
    }
}

/// Centres of the insert grid: `spacing` apart, the grid centred on the board, at least half a spacing (and 1 in) from
/// the edges.
fn grid(s: &SpoilSpec) -> (Vec<f64>, Vec<f64>) {
    let axis = |len: f64| {
        let n = ((len - 2.0 * (s.spacing / 2.0).max(1.0)) / s.spacing).floor().max(0.0) as usize + 1;
        let x0 = (len - (n - 1) as f64 * s.spacing) / 2.0;
        (0..n).map(|i| x0 + i as f64 * s.spacing).collect::<Vec<_>>()
    };
    (axis(s.w), axis(s.h))
}

/// Fills an empty canvas with the spoilboard job. Returns what was made, for the status line.
pub fn build(c: &mut Canvas, s: &SpoilSpec) -> String {
    let m = &mut c.doc.material;
    (m.w, m.h, m.t) = (s.w, s.h, s.t);
    let a = c.doc.active_sheet.max(0) as usize;
    if let Some(sh) = c.doc.sheets.get_mut(a) {
        *sh = c.doc.material.clone();
    }
    let mill = bit(c, ToolType::EndMill, 0.25, i18n::t("1/4 End Mill (0.25 inch)"));
    let (xs, ys) = grid(s);
    let mut ops = Vec::new();
    if s.surface {
        let layer = c.doc.layer_named(i18n::t("Surface"));
        let id = c.doc.add(make_rect(0.0, 0.0, s.w, s.h), Some(layer));
        if let Some((_, op)) = super::op::OpDialog::surface(&c.doc, id).draft(&c.doc) {
            ops.push(op);
        }
    }
    let layer = c.doc.layer_named(i18n::t("Inserts"));
    let holes: Vec<i32> = ys.iter().flat_map(|&y| xs.iter().map(move |&x| (x, y))).map(|(x, y)| c.doc.add(make_ellipse(x, y, s.hole / 2.0, s.hole / 2.0), Some(layer))).collect();
    let count = holes.len();
    ops.push(Op { kind: OpType::Pocket, name: i18n::t("Insert holes").into(), tool: mill, shape_ids: holes, depth: s.depth, ..Default::default() });
    if s.lines {
        // lines half way between the holes, so they never run through one
        let layer = c.doc.layer_named(i18n::t("Grid lines"));
        let mut ids = Vec::new();
        for x in xs.windows(2).map(|p| (p[0] + p[1]) / 2.0) {
            ids.push(c.doc.add(make_polyline(&[pt(x, 0.5), pt(x, s.h - 0.5)], false), Some(layer)));
        }
        for y in ys.windows(2).map(|p| (p[0] + p[1]) / 2.0) {
            ids.push(c.doc.add(make_polyline(&[pt(0.5, y), pt(s.w - 0.5, y)], false), Some(layer)));
        }
        if !ids.is_empty() {
            let vbit = bit(c, ToolType::VBit, 0.5, i18n::t("90° V-Bit (0.5 inch)"));
            ops.push(Op { kind: OpType::Profile, name: i18n::t("Grid lines").into(), tool: vbit, shape_ids: ids, depth: 0.02, side: Side::On, ..Default::default() });
        }
    }
    c.doc.ops = ops;
    c.sel.clear();
    c.mutated();
    format!("New spoilboard: {count} insert holes ({} × {}){}. Screw the board down, zero on its corner, then Export.", xs.len(), ys.len(), if s.surface { i18n::t(", surfaced first") } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spoilboard_puts_its_insert_grid_centred_inside_the_board_with_clean_toolpaths() {
        for (lines, surface) in [(false, true), (true, false), (true, true)] {
            let mut c = Canvas::default();
            let s = SpoilSpec { w: 25.0, h: 17.0, t: 0.75, spacing: 4.0, hole: 0.375, depth: 0.5, lines, surface };
            let msg = build(&mut c, &s);
            assert!(msg.starts_with("New spoilboard: 24 insert holes (6 × 4)"), "{msg}");
            let (xs, ys) = grid(&s);
            assert!((xs[0] + xs[5] - 25.0).abs() < 1e-9 && (ys[0] + ys[3] - 17.0).abs() < 1e-9, "centred");
            assert!(xs[0] >= 2.0 && ys[0] >= 2.0, "half a spacing from the edge");
            assert_eq!(c.doc.ops.len(), 1 + lines as usize + surface as usize);
            if surface {
                assert!(c.doc.ops[0].raster, "surfaced first");
            }
            let gen = gcarve_core::pipeline::regenerate(&c.doc);
            assert!(gen.warnings.is_empty(), "{:?}", gen.warnings);
            assert!(gen.paths.iter().all(|p| !p.is_empty()));
        }
    }
}
