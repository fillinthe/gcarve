//! Cabinet Maker (File › New Cabinet…): type a frameless cabinet box's outside size and get every part nested on
//! sheets in a new tab, with its toolpaths. Two sides (mirror images: dados for the bottom and top, shelf-pin holes,
//! the toe-kick notch), bottom, top, shelves, a toe-kick board, and the nailed-on back on a sheet of its own
//! thickness. Each part is one group; the toolpaths work by layer, so they cover every sheet.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::sign::bit;
use super::{hint, unit_label, Host};
use crate::canvas::Canvas;
use eframe::egui;
use gcarve_core::document::{Material, Op, OpType, Side, ToolType};
use gcarve_core::geom::{make_ellipse, make_polyline, make_rect, pt};
use std::collections::BTreeSet;

/// shelf-pin holes: 5 mm, 32 mm apart, 37 mm in from the front and back edges (the 32 mm system)
const PIN: [f64; 3] = [5.0 / 25.4, 32.0 / 25.4, 37.0 / 25.4];

#[derive(Clone, Debug)]
pub struct CabinetSpec {
    /// outside width, height, depth
    pub size: [f64; 3],
    /// board thickness, back thickness
    pub t: f64,
    pub back: f64,
    /// toe kick height and depth (0 = none: a wall cabinet)
    pub toe: [f64; 2],
    pub shelves: usize,
    pub count: usize,
    /// bottom and top sit in dados half the board deep (else butt joints)
    pub dados: bool,
    /// dado wider than the board by this
    pub fit: f64,
    /// sheet w, h
    pub sheet: [f64; 2],
}

/// The parts of one cabinet as (name, width, height) of the board thickness, then the back: the cut list.
pub fn cut_list(s: &CabinetSpec) -> Vec<(&'static str, f64, f64)> {
    let [w, h, d] = s.size;
    let side_d = d - s.back;
    let dd = if s.dados { s.t / 2.0 } else { 0.0 };
    let mut v = vec![("Side", side_d, h), ("Side", side_d, h), ("Bottom", w - 2.0 * s.t + 2.0 * dd, side_d), ("Top", w - 2.0 * s.t + 2.0 * dd, side_d)];
    v.extend(std::iter::repeat_n(("Shelf", w - 2.0 * s.t - 0.125, side_d - 0.5), s.shelves));
    if s.toe[0] > 0.0 {
        v.push(("Toe kick", w - 2.0 * s.t, s.toe[0]));
    }
    v
}

pub struct CabinetDialog {
    base: bool,
    /// width, height, depth, board, back, toe height, toe depth, fit, sheet w, sheet h
    size: [String; 10],
    shelves: i32,
    count: i32,
    dados: bool,
    pub done: Option<CabinetSpec>,
}

const BASE: [f64; 10] = [24.0, 34.5, 24.0, 0.75, 0.25, 4.0, 3.0, 0.01, 49.0, 97.0];

impl CabinetDialog {
    pub fn new(doc: &gcarve_core::document::Document) -> Self {
        CabinetDialog { base: true, size: BASE.map(|v| crate::units::text(doc, v)), shelves: 1, count: 1, dados: true, done: None }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let (mut open, mut ok, mut cancel) = (true, false, false);
        let doc = &host.canvas.doc;
        crate::widgets::Dialog::new(i18n::t("New Cabinet")).open(&mut open).width(440.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let was = self.base;
                ui.selectable_value(&mut self.base, true, i18n::t("Base cabinet"));
                ui.selectable_value(&mut self.base, false, i18n::t("Wall cabinet"));
                if was != self.base {
                    // the usual sizes of each kind; a wall cabinet has no toe kick
                    let v = if self.base { [24.0, 34.5, 24.0, 4.0, 3.0] } else { [30.0, 30.0, 12.0, 0.0, 0.0] };
                    for (i, x) in [0, 1, 2, 5, 6].into_iter().zip(v) {
                        self.size[i] = crate::units::text(doc, x);
                    }
                }
            });
            egui::Grid::new("cabinet").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                let labels = ["Width (outside)", "Height (outside)", "Depth (outside)", "Board thickness (measure it)", "Back thickness", "Toe kick height", "Toe kick depth", "Dado wider than the board by", "Sheet width", "Sheet height"];
                for (i, label) in labels.iter().enumerate() {
                    if (!self.base && (i == 5 || i == 6)) || (!self.dados && i == 7) {
                        continue;
                    }
                    ui.label(unit_label(&i18n::tr(label), doc));
                    crate::units::num_field(ui, label, &mut self.size[i], doc, true, 110.0);
                    ui.end_row();
                }
                ui.label(i18n::t("Shelves"));
                ui.add(crate::widgets::SpinBox::new(&mut self.shelves, 0..=12));
                ui.end_row();
                ui.label(i18n::t("How many cabinets"));
                ui.add(crate::widgets::SpinBox::new(&mut self.count, 1..=40));
                ui.end_row();
            });
            crate::widgets::checkbox(ui, &mut self.dados, i18n::t("Dado joints: the bottom and top sit in grooves half the board deep"));
            hint(ui, i18n::t("Opens as a new project: a frameless box (two sides, bottom, top, shelves, toe kick) nested over as many sheets as it needs, with dados, 5 mm shelf-pin holes and the cut-outs, and the nailed-on back on a sheet of its own thickness."));
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
            match n {
                Err(e) => *host.message = e,
                Ok(n) if n[3] <= 0.0 || n[4] <= 0.0 || n[8] < 6.0 || n[9] < 6.0 => *host.message = i18n::t("New cabinet: give the boards a thickness and the sheet a size.").into(),
                Ok(n) if n[0] < 4.0 * n[3] + 2.0 || n[1] < n[5] + 4.0 * n[3] + 2.0 || n[2] < n[4] + n[6] + 3.0 => *host.message = i18n::t("New cabinet: too small for its boards and toe kick.").into(),
                Ok(n) => {
                    let toe = if self.base { [n[5].max(0.0), n[6].max(0.0)] } else { [0.0, 0.0] };
                    self.done = Some(CabinetSpec { size: [n[0], n[1], n[2]], t: n[3], back: n[4], toe, shelves: self.shelves.max(0) as usize, count: self.count.max(1) as usize, dados: self.dados, fit: n[7].max(0.0), sheet: [n[8], n[9]] });
                    return false;
                }
            }
        }
        open
    }
}

/// Fills an empty canvas with the cabinets' parts, nested over sheets. Returns what was made, for the status line.
pub fn build(c: &mut Canvas, s: &CabinetSpec) -> String {
    let [sw, sh] = s.sheet;
    let [w, h, _] = s.size;
    let gap = 0.5;
    let m = &mut c.doc.material;
    (m.w, m.h, m.t) = (sw, sh, s.t);
    let a = c.doc.active_sheet.max(0) as usize;
    if let Some(sheet) = c.doc.sheets.get_mut(a) {
        *sheet = c.doc.material.clone();
    }
    let list = cut_list(s);
    let fits = |pw: f64, ph: f64| (pw <= sw - 2.0 * gap && ph <= sh - 2.0 * gap) || (ph <= sw - 2.0 * gap && pw <= sh - 2.0 * gap);
    if !list.iter().all(|p| fits(p.1, p.2)) || !fits(w, h - s.toe[0]) {
        return i18n::t("New cabinet: a part is bigger than the sheet.").into();
    }
    let mill = bit(c, ToolType::EndMill, 0.25, i18n::t("1/4 End Mill (0.25 inch)"));
    let small = (s.shelves > 0).then(|| bit(c, ToolType::EndMill, 0.125, i18n::t("1/8 End Mill (0.125 inch)")));
    let parts = c.doc.layer_named(i18n::t("Cabinet parts"));
    let dado_l = c.doc.layer_named(i18n::t("Cabinet dados"));
    let pins = c.doc.layer_named(i18n::t("Shelf pin holes"));
    let backs = c.doc.layer_named(i18n::t("Cabinet backs"));
    let dd = if s.dados { s.t / 2.0 } else { 0.0 };
    let ext = 0.13; // dados run this far past the part's edge, inside the cut-out bit's own path, so they have no round ends
    let mut sel = BTreeSet::new();
    let mut x0 = 0.0;
    for _ in 0..s.count {
        let mut side_no = 0;
        for &(name, pw, ph) in &list {
            // laid out in a row first; nesting places them
            let group = c.doc.new_group_id();
            let mut add = |c: &mut Canvas, shape, layer| {
                let id = c.doc.add(shape, Some(layer));
                if let Some(sh) = c.doc.find_mut(id) {
                    sh.group = group;
                }
                sel.insert(id);
            };
            if name == "Side" {
                // the left side has its front at the left, the right side is its mirror image: both show their inside face
                let flip = side_no == 1;
                side_no += 1;
                let fx = |x: f64| x0 + if flip { pw - x } else { x };
                let mut outline = vec![pt(fx(0.0), s.toe[0]), pt(fx(s.toe[1]), s.toe[0]), pt(fx(s.toe[1]), 0.0), pt(fx(pw), 0.0), pt(fx(pw), ph), pt(fx(0.0), ph)];
                if s.toe[0] <= 0.0 || s.toe[1] <= 0.0 {
                    outline = vec![pt(fx(0.0), 0.0), pt(fx(pw), 0.0), pt(fx(pw), ph), pt(fx(0.0), ph)];
                }
                add(c, make_polyline(&outline, true), parts);
                if s.dados {
                    let across = |c: &mut Canvas, add: &mut dyn FnMut(&mut Canvas, gcarve_core::geom::Contour, i32), y0: f64, y1: f64| add(c, make_rect(x0 - ext, y0, x0 + pw + ext, y1), dado_l);
                    across(c, &mut add, s.toe[0], s.toe[0] + s.t + s.fit); // the bottom
                    across(c, &mut add, ph - s.t - s.fit, ph + ext); // the top: a rabbet at the edge
                }
                if s.shelves > 0 {
                    let (lo, hi) = (s.toe[0] + s.t + 6.0, ph - s.t - 6.0);
                    let mut y = lo;
                    while y <= hi + 1e-9 {
                        for x in [PIN[2], pw - PIN[2]] {
                            add(c, make_ellipse(fx(x), y, PIN[0] / 2.0, PIN[0] / 2.0), pins);
                        }
                        y += PIN[1];
                    }
                }
            } else {
                add(c, make_rect(x0, 0.0, x0 + pw, ph), parts);
            }
            x0 += pw + gap + 2.0 * ext;
        }
    }
    let mut ops = Vec::new();
    if s.dados {
        ops.push(Op { kind: OpType::Pocket, name: i18n::t("Dados").into(), tool: mill, layer_scope: dado_l, depth: dd, stepover: 0.4, ..Default::default() });
    }
    if let Some(b) = small {
        ops.push(Op { kind: OpType::Bore, name: i18n::t("Shelf pin holes").into(), tool: b, layer_scope: pins, depth: (s.t - 0.125).min(0.5), ..Default::default() });
    }
    ops.push(Op { kind: OpType::Profile, name: i18n::t("Cut out parts").into(), tool: mill, layer_scope: parts, depth: s.t, side: Side::Outside, tabs: true, tab_count: 4, tab_corners: true, ..Default::default() });
    ops.push(Op { kind: OpType::Profile, name: i18n::t("Cut out backs").into(), tool: mill, layer_scope: backs, depth: s.back, side: Side::Outside, tabs: true, tab_count: 4, tab_height: (s.back / 2.0).min(0.125), tab_corners: true, ..Default::default() });
    c.doc.ops = ops;
    c.sel = sel;
    c.sel_changed();
    c.nest_selection_sheets(gap, gap, 90, 0);
    // the backs: a sheet of their own thickness after the last one
    c.doc.sync_sheet();
    c.doc.sheets.push(Material { name: i18n::t("Backs").into(), back_of: None, t: s.back, ..c.doc.material.clone() });
    let first_back = c.doc.sheets.len() as i32 - 1;
    c.doc.set_active_sheet(first_back);
    let mut sel = BTreeSet::new();
    for k in 0..s.count {
        sel.insert(c.doc.add(make_rect(k as f64 * (w + gap), 0.0, k as f64 * (w + gap) + w, h - s.toe[0]), Some(backs)));
    }
    c.sel = sel;
    c.sel_changed();
    c.nest_selection_sheets(gap, gap, 90, 0);
    c.doc.set_active_sheet(0);
    c.sel.clear();
    c.mutated();
    let u = |v: f64| crate::units::text(&c.doc, v);
    let mut sizes: Vec<String> = Vec::new();
    for (name, pw, ph) in &list {
        let line = format!("{} {} × {}", i18n::tr(name), u(*pw), u(*ph));
        if !sizes.contains(&line) {
            sizes.push(line);
        }
    }
    i18n::tf("New cabinet: {n} cabinet(s), {parts} parts on {sheets} sheet(s), the last for the backs ({back}). Parts: {sizes}. Check them in Simulate, then Export (one file set per sheet).", &[("n", &s.count), ("parts", &(s.count * (list.len() + 1))), ("sheets", &c.doc.sheets.len()), ("back", &format!("{} × {}", u(w), u(h - s.toe[0]))), ("sizes", &sizes.join(", "))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::geom::bounds;

    fn spec() -> CabinetSpec {
        CabinetSpec { size: [24.0, 34.5, 24.0], t: 0.75, back: 0.25, toe: [4.0, 3.0], shelves: 1, count: 2, dados: true, fit: 0.01, sheet: [49.0, 97.0] }
    }

    #[test]
    fn the_cut_list_adds_up_to_the_box() {
        let s = spec();
        let l = cut_list(&s);
        assert_eq!(l.len(), 6, "two sides, bottom, top, a shelf, the toe kick");
        // sides are the depth less the back; the bottom spans between the sides plus both dados
        assert_eq!((l[0].1, l[0].2), (23.75, 34.5));
        assert!((l[2].1 - (24.0 - 1.5 + 0.75)).abs() < 1e-9 && l[2].2 == 23.75);
        assert!((l[4].1 - (24.0 - 1.5 - 0.125)).abs() < 1e-9, "a shelf slides in with 1/8 in of play");
    }

    #[test]
    fn cabinets_nest_on_sheets_with_dados_pin_holes_and_clean_toolpaths() {
        let mut c = Canvas::default();
        let s = spec();
        let msg = build(&mut c, &s);
        assert!(msg.starts_with("New cabinet: 2 cabinet(s), 14 parts"), "{msg}");
        let layer = |sh: &gcarve_core::document::Shape| c.doc.layers[sh.layer as usize].name.clone();
        let parts: Vec<_> = c.doc.shapes.iter().filter(|sh| layer(sh) == "Cabinet parts").collect();
        assert_eq!(parts.len(), 12);
        // every part is on its board, and each dado and pin hole travelled with its side
        for p in &parts {
            let (b, m) = (bounds(&p.c), &c.doc.sheets[p.sheet as usize]);
            assert!(b.x0 >= 0.3 && b.y0 >= 0.3 && b.x1 <= m.w - 0.3 && b.y1 <= m.h - 0.3, "{b:?} on sheet {}", p.sheet);
        }
        for f in c.doc.shapes.iter().filter(|sh| ["Cabinet dados", "Shelf pin holes"].contains(&layer(sh).as_str())) {
            let side = parts.iter().find(|p| p.group == f.group).expect("its side");
            let (fb, sb) = (bounds(&f.c), bounds(&side.c));
            assert!(f.sheet == side.sheet && fb.x1 > sb.x0 && fb.x0 < sb.x1 && fb.y1 > sb.y0 && fb.y0 < sb.y1, "{fb:?} on {sb:?}");
        }
        // the two sides of a cabinet are mirror images: their toe-kick notches face opposite ways
        let sides: Vec<_> = parts.iter().filter(|p| p.c.nodes.len() == 6).collect();
        assert_eq!(sides.len(), 4);
        // backs: thin sheet(s) at the end, cut only as deep as they are
        let last = c.doc.sheets.last().unwrap();
        assert_eq!(last.t, 0.25);
        assert_eq!(c.doc.shapes.iter().filter(|sh| layer(sh) == "Cabinet backs").count(), 2);
        for j in 0..c.doc.sheets.len() as i32 {
            c.doc.set_active_sheet(j);
            let gen = gcarve_core::pipeline::regenerate(&c.doc);
            assert!(gen.warnings.is_empty(), "sheet {j}: {:?}", gen.warnings);
            let issues = gcarve_core::pipeline::order_issues(&c.doc);
            assert!(issues.is_empty(), "sheet {j}: {issues:?}");
        }
    }
}
