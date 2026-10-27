//! Cabinet Door Maker (File › New Cabinet Doors…): paste a cut list ("W x H" or "W x H x qty", one size a line) and
//! get every door nested across sheets in a new tab. Shaker = the panel V-carved to a flat floor (bevelled edge, sharp
//! inside corners: the MDF shaker look; an end mill clears the floor), slab = the cut-out only. Each door is one group
//! (outline + panel), the toolpaths work by layer ("Door panels", "Door outlines"), so they cover every sheet.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::sign::bit;
use super::{hint, unit_label, Host};
use crate::canvas::Canvas;
use eframe::egui;
use gcarve_core::document::{Op, OpType, Side, ToolType};
use gcarve_core::geom::make_rect;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct DoorSpec {
    /// (width, height) per door, inches, quantities expanded
    pub doors: Vec<(f64, f64)>,
    pub shaker: bool,
    /// frame (stile and rail) width
    pub frame: f64,
    /// panel depth
    pub depth: f64,
    /// sheet w, h, t
    pub sheet: [f64; 3],
    /// space between doors on the sheet
    pub gap: f64,
    /// don't turn doors (wood grain runs up the door)
    pub grain: bool,
}

/// "W x H" or "W x H x qty" per line (x, X, ×, * or commas between; blank lines and # comments skipped); sizes in
/// the project unit. Err = the first line that doesn't read.
pub fn parse_list(text: &str, doc: &gcarve_core::document::Document) -> Result<Vec<(f64, f64)>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(['x', 'X', '×', '*', ',']).map(str::trim).filter(|p| !p.is_empty()).collect();
        let bad = || format!("Line {}: \"{line}\" should be width x height, or width x height x how many", n + 1);
        if !(2..=3).contains(&parts.len()) {
            return Err(bad());
        }
        let w = crate::units::get(doc, parts[0], true).map_err(|_| bad())?;
        let h = crate::units::get(doc, parts[1], true).map_err(|_| bad())?;
        let q: usize = if parts.len() == 3 { parts[2].parse().map_err(|_| bad())? } else { 1 };
        if w <= 0.0 || h <= 0.0 || q == 0 || q > 500 {
            return Err(bad());
        }
        out.extend(std::iter::repeat_n((w, h), q));
    }
    if out.is_empty() {
        return Err(i18n::t("Type the door sizes, one a line: width x height (x how many).").into());
    }
    Ok(out)
}

pub struct DoorDialog {
    list: String,
    shaker: bool,
    grain: bool,
    /// frame, depth, sheet w, h, t, gap
    size: [String; 6],
    pub done: Option<DoorSpec>,
}

impl DoorDialog {
    pub fn new(doc: &gcarve_core::document::Document) -> Self {
        DoorDialog { list: i18n::t("15 x 30 x 2\n18 x 24\n12 x 30 x 4").into(), shaker: true, grain: false, size: [2.25, 0.25, 49.0, 97.0, 0.75, 0.5].map(|v| crate::units::text(doc, v)), done: None }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let (mut open, mut ok, mut cancel) = (true, false, false);
        let doc = &host.canvas.doc;
        crate::widgets::Dialog::new(i18n::t("New Cabinet Doors")).open(&mut open).width(440.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.shaker, true, i18n::t("Shaker (recessed panel)"));
                ui.selectable_value(&mut self.shaker, false, i18n::t("Slab"));
            });
            ui.label(format!("Doors, one size a line: width x height (x how many), in {}", crate::units::name(doc)));
            ui.add(egui::TextEdit::multiline(&mut self.list).desired_rows(5).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
            egui::Grid::new("doors").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                for (i, label) in ["Frame width", i18n::t("Panel depth"), i18n::t("Sheet width"), i18n::t("Sheet height"), i18n::t("Sheet thickness"), "Space between doors"].iter().enumerate() {
                    if i < 2 && !self.shaker {
                        continue;
                    }
                    ui.label(unit_label(label, doc));
                    crate::units::num_field(ui, label, &mut self.size[i], doc, true, 110.0);
                    ui.end_row();
                }
            });
            crate::widgets::checkbox(ui, &mut self.grain, i18n::t("Keep the grain running up every door (don't turn them)"));
            hint(ui, if self.shaker { i18n::t("Opens as a new project: the doors nested over as many sheets as they need, each panel V-carved to a flat floor (a 90° V-bit bevels the edge and leaves sharp corners, a 1/4 in end mill clears the floor), then cut out with tabs.") } else { i18n::t("Opens as a new project: the doors nested over as many sheets as they need, cut out with tabs.") });
            let (o, c) = crate::widgets::ok_cancel(ui);
            ok = o;
            cancel = c;
        });
        if cancel {
            return false;
        }
        if ok {
            let doc = &host.canvas.doc;
            let n: Result<Vec<f64>, String> = self.size.iter().map(|s| crate::units::get(doc, s, true)).collect();
            match (parse_list(&self.list, doc), n) {
                (Err(e), _) | (_, Err(e)) => *host.message = e,
                (Ok(_), Ok(n)) if n[2] < 6.0 || n[3] < 6.0 || n[4] <= 0.0 => *host.message = i18n::t("New doors: give the sheet a size and thickness.").into(),
                (Ok(_), Ok(n)) if self.shaker && (n[1] <= 0.0 || n[1] >= n[4] - 0.125) => *host.message = i18n::t("New doors: the panel depth must leave at least 1/8 in.").into(),
                (Ok(_), Ok(n)) if n[5] < 0.3 => *host.message = i18n::t("New doors: leave at least 0.3 in between doors for the cut-out bit.").into(),
                (Ok(doors), Ok(n)) => {
                    self.done = Some(DoorSpec { doors, shaker: self.shaker, frame: n[0], depth: n[1], sheet: [n[2], n[3], n[4]], gap: n[5], grain: self.grain });
                    return false;
                }
            }
        }
        open
    }
}

/// Fills an empty canvas with the doors, nested over sheets. Returns what was made, for the status line.
pub fn build(c: &mut Canvas, s: &DoorSpec) -> String {
    let [w, h, t] = s.sheet;
    let m = &mut c.doc.material;
    (m.w, m.h, m.t) = (w, h, t);
    let a = c.doc.active_sheet.max(0) as usize;
    if let Some(sh) = c.doc.sheets.get_mut(a) {
        *sh = c.doc.material.clone();
    }
    let mill = bit(c, ToolType::EndMill, 0.25, i18n::t("1/4 End Mill (0.25 inch)"));
    let vbit = s.shaker.then(|| bit(c, ToolType::VBit, 0.5, i18n::t("90° V-Bit (0.5 inch)")));
    let outlines = c.doc.layer_named(i18n::t("Door outlines"));
    let panels = c.doc.layer_named(i18n::t("Door panels"));
    let fits = |dw: f64, dh: f64| (dw <= w - 2.0 * s.gap && dh <= h - 2.0 * s.gap) || (!s.grain && dh <= w - 2.0 * s.gap && dw <= h - 2.0 * s.gap);
    let too_big: Vec<String> = s.doors.iter().filter(|(dw, dh)| !fits(*dw, *dh)).map(|(dw, dh)| format!("{} × {}", crate::units::text(&c.doc, *dw), crate::units::text(&c.doc, *dh))).collect();
    if !too_big.is_empty() {
        return format!("New doors: bigger than the sheet: {}", too_big.join(", "));
    }
    let narrow = s.shaker && s.doors.iter().any(|(dw, dh)| dw.min(*dh) <= 2.0 * s.frame + 0.5);
    let mut sel = BTreeSet::new();
    let mut x = 0.0;
    for &(dw, dh) in &s.doors {
        // laid out in a row first; nesting places them
        let group = c.doc.new_group_id();
        let mut add = |c: &mut Canvas, r, layer| {
            let id = c.doc.add(r, Some(layer));
            if let Some(sh) = c.doc.find_mut(id) {
                sh.group = group;
            }
            sel.insert(id);
        };
        add(c, make_rect(x, 0.0, x + dw, dh), outlines);
        if s.shaker && dw.min(dh) > 2.0 * s.frame + 0.5 {
            add(c, make_rect(x + s.frame, s.frame, x + dw - s.frame, dh - s.frame), panels);
        }
        x += dw + s.gap;
    }
    let mut ops = Vec::new();
    if let Some(v) = vbit {
        ops.push(Op { kind: OpType::VCarve, name: i18n::t("Door panels").into(), tool: v, clear_tool: mill, layer_scope: panels, depth: s.depth, flat_depth: s.depth, ..Default::default() });
    }
    ops.push(Op { kind: OpType::Profile, name: i18n::t("Cut out doors").into(), tool: mill, layer_scope: outlines, depth: t, side: Side::Outside, tabs: true, tab_count: 4, tab_corners: true, ..Default::default() });
    c.doc.ops = ops;
    c.sel = sel;
    c.sel_changed();
    // the cut-out bit runs in the gap: half of it each side is enough, with the same at the sheet edge
    c.nest_selection_sheets(s.gap, s.gap, if s.grain { 0 } else { 90 }, 0);
    c.doc.set_active_sheet(0);
    c.sel.clear();
    c.mutated();
    format!("New doors: {} door{} on {} sheet{}{}. Check them in Simulate, then Export (one file set per sheet).", s.doors.len(), if s.doors.len() == 1 { "" } else { "s" }, c.doc.sheets.len(), if c.doc.sheets.len() == 1 { "" } else { "s" }, if narrow { i18n::t("; doors too narrow for a panel are left plain") } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::geom::bounds;

    #[test]
    fn a_cut_list_reads_sizes_and_counts() {
        let doc = gcarve_core::document::Document::default();
        assert_eq!(parse_list("15 x 30 x 2\n# uppers\n18X24\n\n12×30", &doc).unwrap(), vec![(15.0, 30.0), (15.0, 30.0), (18.0, 24.0), (12.0, 30.0)]);
        assert!(parse_list("15 x", &doc).unwrap_err().starts_with("Line 1"));
        assert!(parse_list("", &doc).is_err());
    }

    #[test]
    fn doors_nest_over_sheets_with_their_panels_and_clean_toolpaths() {
        let mut c = Canvas::default();
        let doc = gcarve_core::document::Document::default();
        let doors = parse_list("15 x 30 x 6\n18 x 24 x 4\n12 x 30 x 4\n4 x 4", &doc).unwrap();
        let s = DoorSpec { doors, shaker: true, frame: 2.25, depth: 0.25, sheet: [49.0, 97.0, 0.75], gap: 0.5, grain: false };
        let msg = build(&mut c, &s);
        assert!(msg.starts_with("New doors: 15 doors on"), "{msg}");
        assert!(c.doc.sheets.len() >= 2, "more than one sheet's worth");
        assert!(msg.contains("left plain"), "the 4 × 4 door has no room for a panel");
        let outlines: Vec<_> = c.doc.shapes.iter().filter(|sh| c.doc.layers[sh.layer as usize].name == "Door outlines").collect();
        assert_eq!(outlines.len(), 15);
        for j in 0..c.doc.sheets.len() {
            let m = &c.doc.sheets[j];
            let on: Vec<_> = outlines.iter().filter(|o| o.sheet == j as i32).map(|o| bounds(&o.c)).collect();
            for (k, a) in on.iter().enumerate() {
                assert!(a.x0 >= 0.25 && a.y0 >= 0.25 && a.x1 <= m.w - 0.25 && a.y1 <= m.h - 0.25, "sheet {j}: {a:?} on the board");
                for b in &on[k + 1..] {
                    assert!(a.x1 + 0.45 <= b.x0 || b.x1 + 0.45 <= a.x0 || a.y1 + 0.45 <= b.y0 || b.y1 + 0.45 <= a.y0, "doors keep the gap: {a:?} {b:?}");
                }
            }
        }
        // every panel sits inside its door, on the same sheet
        for p in c.doc.shapes.iter().filter(|sh| c.doc.layers[sh.layer as usize].name == "Door panels") {
            let pb = bounds(&p.c);
            assert!(outlines.iter().any(|o| o.sheet == p.sheet && o.group == p.group && bounds(&o.c).contains(&pb)));
        }
        for j in 0..c.doc.sheets.len() as i32 {
            c.doc.set_active_sheet(j);
            let gen = gcarve_core::pipeline::regenerate(&c.doc);
            assert!(gen.warnings.is_empty(), "sheet {j}: {:?}", gen.warnings);
            assert!(gen.paths.iter().all(|p| !p.is_empty()), "both toolpaths cut on sheet {j}");
        }
    }
}
