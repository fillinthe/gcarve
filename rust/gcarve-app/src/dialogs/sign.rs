//! Sign Wizard (File › New Sign…): type the words, the board size and a shape, and get a ready job in a new tab:
//! V-carved letters fitted to the board, an optional V-groove border and an outside cut-out with tabs, using bits
//! from the user's library (a 90° V-bit and a 1/4 in end mill are added when the library has none). The community
//! starter signs are made with it too.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::{hint, unit_label, Host};
use crate::canvas::Canvas;
use eframe::egui;
use gcarve_core::document::{Op, OpType, Side, TextItem, Tool, ToolType};
use gcarve_core::geom::{bounds, make_ellipse, make_rect, pt, transformed, Affine, Contour, Pt};
use gcarve_core::vecops::fillet_contours;

#[derive(Clone, Debug)]
pub struct SignSpec {
    pub words: String,
    /// board, inches
    pub w: f64,
    pub h: f64,
    pub t: f64,
    /// 0 rectangle, 1 rounded corners, 2 oval
    pub shape: u8,
    pub border: bool,
    pub family: String,
    pub bold: bool,
    /// one sign per line, laid out on stock sheets of this width × height (None = one sign, words on lines)
    pub batch: Option<(f64, f64)>,
}

const SHAPES: [&str; 3] = ["Rectangle", "Rounded corners", "Oval"]; // i18n
const DEFAULT_FONT: &str = "Liberation Sans";

pub struct SignDialog {
    words: String,
    size: [String; 3],
    shape: u8,
    border: bool,
    family: String,
    bold: bool,
    batch: bool,
    /// stock sheet width, height for a batch
    stock: [String; 2],
    /// OK pressed: the app makes the sign in a new tab
    pub done: Option<SignSpec>,
}

impl SignDialog {
    pub fn new(doc: &gcarve_core::document::Document) -> Self {
        let t = if doc.material.t > 0.0 { doc.material.t } else { 0.75 };
        SignDialog { words: "WELCOME".into(), size: [24.0, 8.0, t].map(|v| crate::units::text(doc, v)), shape: 1, border: true, family: DEFAULT_FONT.into(), bold: true, batch: false, stock: [48.0, 96.0].map(|v| crate::units::text(doc, v)), done: None }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let (mut open, mut ok, mut cancel) = (true, false, false);
        let doc = &host.canvas.doc;
        crate::widgets::Dialog::new(i18n::t("New Sign")).open(&mut open).width(420.0).show(ctx, |ui| {
            ui.label(if self.batch { i18n::t("Names (one sign per line)") } else { i18n::t("Words (one line each)") });
            ui.add(egui::TextEdit::multiline(&mut self.words).desired_rows(2).desired_width(f32::INFINITY));
            egui::Grid::new("sign").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                for (i, label) in ["Board width", i18n::t("Board height"), "Board thickness"].iter().enumerate() {
                    ui.label(unit_label(label, doc));
                    crate::units::num_field(ui, label, &mut self.size[i], doc, true, 110.0);
                    ui.end_row();
                }
                ui.label(i18n::t("Shape"));
                egui::ComboBox::from_id_salt("sign-shape").selected_text(i18n::t(SHAPES[self.shape as usize])).show_ui(ui, |ui| {
                    for (i, s) in SHAPES.iter().enumerate() {
                        ui.selectable_value(&mut self.shape, i as u8, i18n::t(s));
                    }
                });
                ui.end_row();
                if self.batch {
                    for (i, label) in ["Sheet width", "Sheet height"].iter().enumerate() {
                        ui.label(unit_label(label, doc));
                        crate::units::num_field(ui, label, &mut self.stock[i], doc, true, 110.0);
                        ui.end_row();
                    }
                }
                ui.label(i18n::t("Font"));
                egui::ComboBox::from_id_salt("sign-font").selected_text(self.family.as_str()).show_ui(ui, |ui| {
                    for f in std::iter::once(DEFAULT_FONT.to_string()).chain(crate::text::custom_families()) {
                        ui.selectable_value(&mut self.family, f.clone(), f);
                    }
                });
                ui.end_row();
            });
            crate::widgets::checkbox(ui, &mut self.bold, i18n::t("Bold letters"));
            crate::widgets::checkbox(ui, &mut self.border, i18n::t("V-groove border"));
            crate::widgets::checkbox(ui, &mut self.batch, i18n::t("One sign per line (a batch of names), laid out on sheets"));
            hint(ui, if self.batch { i18n::t("Opens as a new project: one sign per line, each with its letters sized to fit, laid out over as many sheets as it takes. The board size is one sign.") } else { i18n::t("Opens as a new project: carved letters sized to fit, the border, and a cut-out with tabs. Change anything afterwards, then Simulate and Export.") });
            let (o, c) = crate::widgets::ok_cancel(ui);
            ok = o;
            cancel = c;
        });
        if cancel {
            return false;
        }
        if ok {
            let v: Result<Vec<f64>, String> = self.size.iter().chain(&self.stock).map(|s| crate::units::get(&host.canvas.doc, s, true)).collect();
            match v {
                Err(e) => *host.message = e,
                Ok(n) if n[0] < 2.0 || n[1] < 1.0 || n[2] <= 0.0 => *host.message = i18n::t("New sign: the board must be at least 2 × 1 in, with a thickness.").into(),
                Ok(_) if self.words.trim().is_empty() => *host.message = i18n::t("New sign: type the words first.").into(),
                Ok(n) if self.batch && (n[0] + 0.5 > n[3] || n[1] + 0.5 > n[4]) => *host.message = i18n::t("New signs: one sign must fit on the sheet with room round it.").into(),
                Ok(n) => {
                    self.done = Some(SignSpec { words: self.words.trim().into(), w: n[0], h: n[1], t: n[2], shape: self.shape, border: self.border, family: self.family.clone(), bold: self.bold, batch: self.batch.then_some((n[3], n[4])) });
                    return false;
                }
            }
        }
        open
    }
}

/// A bit of this kind from the project's tools (closest to `dia`, V-bits closest to 90°), added when there is none.
pub(crate) fn bit(c: &mut Canvas, kind: ToolType, dia: f64, name: &str) -> i32 {
    let score = |t: &Tool| if kind == ToolType::VBit { (t.angle - 90.0).abs() } else { (t.diameter - dia).abs() };
    let best = c.doc.tools.iter().enumerate().filter(|(_, t)| t.kind == kind).min_by(|a, b| score(a.1).total_cmp(&score(b.1)));
    if let Some((i, _)) = best {
        return i as i32;
    }
    c.doc.tools.push(Tool { name: name.into(), kind, diameter: dia, angle: 90.0, stepdown: if kind == ToolType::VBit { 0.1 } else { 0.125 }, feed_xy: 100.0, feed_z: 30.0, rpm: 18000, ..Default::default() });
    c.doc.tools.len() as i32 - 1
}

/// The outline of the shape inset by `d` from the board edge.
fn outline(s: &SignSpec, d: f64) -> Contour {
    outline_of(s.w, s.h, s.shape, d)
}

/// A w × h board's shape (0 rectangle, 1 rounded corners, 2 oval) inset by `d` from its edge (the Tray Maker too).
pub(crate) fn outline_of(w: f64, h: f64, shape: u8, d: f64) -> Contour {
    let (x1, y1) = (w - d, h - d);
    match shape {
        2 => make_ellipse(w / 2.0, h / 2.0, w / 2.0 - d, h / 2.0 - d),
        1 => {
            let r = (w.min(h) * 0.12 - (d - 0.3)).max(0.1);
            fillet_contours(&[make_rect(d, d, x1, y1)], r, true, true).into_iter().next().unwrap_or_else(|| make_rect(d, d, x1, y1))
        }
        _ => make_rect(d, d, x1, y1),
    }
}

/// Fills an empty canvas with the sign job. Returns what was made, for the status line.
pub fn build(c: &mut Canvas, s: &SignSpec) -> String {
    let (bw, bh) = s.batch.unwrap_or((s.w, s.h));
    let m = &mut c.doc.material;
    (m.w, m.h, m.t) = (bw, bh, s.t);
    let a = c.doc.active_sheet.max(0) as usize;
    if let Some(sh) = c.doc.sheets.get_mut(a) {
        *sh = c.doc.material.clone();
    }
    let vbit = bit(c, ToolType::VBit, 0.5, i18n::t("90° V-Bit (0.5 inch)"));
    let mill = bit(c, ToolType::EndMill, 0.25, i18n::t("1/4 End Mill (0.25 inch)"));
    // V-carved to a flat floor no deeper than a third of the board: big letters don't go through thin boards
    let depth = (s.t / 3.0).min(0.25);
    let Some((bw, bh)) = s.batch else {
        let Some((letters, border, cut)) = sign_at(c, s, &s.words, pt(0.0, 0.0), [None; 3]) else {
            return i18n::t("New sign: the board is too small for the words.").into();
        };
        let mut ops = vec![Op { kind: OpType::VCarve, name: i18n::t("Letters").into(), tool: vbit, shape_ids: letters, depth, flat_depth: depth, ..Default::default() }];
        if let Some(b) = border {
            ops.push(Op { kind: OpType::Profile, name: i18n::t("Border groove").into(), tool: vbit, shape_ids: vec![b], depth: 0.1, side: Side::On, ..Default::default() });
        }
        ops.push(Op { kind: OpType::Profile, name: i18n::t("Cut out").into(), tool: mill, shape_ids: vec![cut], depth: s.t, side: Side::Outside, tabs: true, tab_count: 4, tab_corners: true, ..Default::default() });
        c.doc.ops = ops;
        c.sel.clear();
        c.mutated();
        return format!("New sign: {} letters V-carved, {}a cut-out with tabs. Check it in Simulate, then Export.", c.doc.texts.last().map_or(0, |t| t.text.chars().filter(|ch| !ch.is_whitespace()).count()), if s.border { i18n::t("a border groove, ") } else { "" });
    };
    // a batch: signs in a grid, 1/2 in apart (the cut-out bit runs in the gap), as many sheets as it takes
    let layers = ["Sign letters", i18n::t("Sign borders"), "Sign outlines"].map(|n| Some(c.doc.layer_named(n)));
    let gap = 0.5;
    let (cols, rows) = (((bw - gap) / (s.w + gap)).floor().max(1.0) as usize, ((bh - gap) / (s.h + gap)).floor().max(1.0) as usize);
    let names: Vec<&str> = s.words.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut skipped = Vec::new();
    for (k, name) in names.iter().enumerate() {
        let (sheet, slot) = (k / (cols * rows), k % (cols * rows));
        while c.doc.sheets.len() <= sheet {
            let mut m = c.doc.material.clone();
            m.name.clear();
            c.doc.sheets.push(m);
        }
        c.doc.set_active_sheet(sheet as i32);
        let at = pt(gap + (slot % cols) as f64 * (s.w + gap), bh - (gap + s.h) * (1 + slot / cols) as f64);
        if sign_at(c, s, name, at, layers).is_none() {
            skipped.push(*name);
        }
    }
    c.doc.set_active_sheet(0);
    let scope = |l: Option<i32>| l.unwrap_or(-1);
    let mut ops = vec![Op { kind: OpType::VCarve, name: i18n::t("Letters").into(), tool: vbit, layer_scope: scope(layers[0]), depth, flat_depth: depth, ..Default::default() }];
    if s.border {
        ops.push(Op { kind: OpType::Profile, name: i18n::t("Border groove").into(), tool: vbit, layer_scope: scope(layers[1]), depth: 0.1, side: Side::On, ..Default::default() });
    }
    ops.push(Op { kind: OpType::Profile, name: i18n::t("Cut out").into(), tool: mill, layer_scope: scope(layers[2]), depth: s.t, side: Side::Outside, tabs: true, tab_count: 4, tab_corners: true, ..Default::default() });
    c.doc.ops = ops;
    c.sel.clear();
    c.mutated();
    let n = c.doc.sheets.len();
    format!("New signs: {} sign{} on {n} sheet{}{}. Check them in Simulate, then Export (one file set per sheet).", names.len() - skipped.len(), if names.len() - skipped.len() == 1 { "" } else { "s" }, if n == 1 { "" } else { "s" }, if skipped.is_empty() { String::new() } else { format!("; too small for: {}", skipped.join(", ")) })
}

/// One sign with its bottom-left corner at `at` on the active sheet: letters fitted inside the border (an oval's
/// largest box is 1/√2 of it), the border groove 0.55 in inside the cut-out, the cut-out 0.3 in in from the board
/// edge. `layers` = letters, border, cut-out (None = the active layer). Returns (letters, border, cut-out) ids, None
/// when the words don't fit.
fn sign_at(c: &mut Canvas, s: &SignSpec, words: &str, at: Pt, layers: [Option<i32>; 3]) -> Option<(Vec<i32>, Option<i32>, i32)> {
    let edge = 0.3;
    let inner = if s.border { edge + 1.0 } else { edge + 0.5 };
    let k = if s.shape == 2 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
    let (aw, ah) = ((s.w / 2.0 - inner) * 2.0 * k, (s.h / 2.0 - inner) * 2.0 * k);
    let mut t = TextItem { text: words.to_string(), family: s.family.clone(), height: 1.0, bold: s.bold, align: 1, ..Default::default() };
    let (glyphs, _) = crate::text::build_text(&t, pt(0.0, 0.0), None);
    let mut b = gcarve_core::geom::Rect::default();
    glyphs.iter().for_each(|g| b.add_rect(&bounds(g)));
    if glyphs.is_empty() || aw <= 0.0 || ah <= 0.0 {
        return None;
    }
    let m = Affine::translate(at.x, at.y);
    let cut = c.doc.add(transformed(&outline(s, edge), &m), layers[2]);
    let border = s.border.then(|| c.doc.add(transformed(&outline(s, edge + 0.55), &m), layers[1]));
    t.height = (aw / (b.x1 - b.x0)).min(ah / (b.y1 - b.y0));
    c.place_text(t, pt(at.x + s.w / 2.0, at.y + s.h / 2.0), layers[0]);
    let id = c.doc.texts.last().map_or(0, |t| t.id);
    Some((c.doc.shapes.iter().filter(|g| g.text == id).map(|g| g.id).collect(), border, cut))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sign_fits_its_letters_inside_the_border_and_makes_clean_toolpaths() {
        for shape in 0..3u8 {
            let mut c = Canvas::default();
            let s = SignSpec { words: "THE SMITHS\nEST 2026".into(), w: 20.0, h: 10.0, t: 0.75, shape, border: true, family: DEFAULT_FONT.into(), bold: true, batch: None };
            build(&mut c, &s);
            assert_eq!(c.doc.ops.len(), 3);
            assert_eq!(c.doc.ops[2].name, "Cut out", "cut free last");
            let letters = &c.doc.ops[0].shape_ids;
            assert!(!letters.is_empty());
            let border = bounds(&c.doc.find(c.doc.ops[1].shape_ids[0]).unwrap().c);
            for id in letters {
                let g = bounds(&c.doc.find(*id).unwrap().c);
                assert!(g.x0 > border.x0 && g.x1 < border.x1 && g.y0 > border.y0 && g.y1 < border.y1, "shape {shape}: {g:?} inside {border:?}");
            }
            let gen = gcarve_core::pipeline::regenerate(&c.doc);
            assert!(gen.warnings.is_empty(), "shape {shape}: {:?}", gen.warnings);
            assert!(gcarve_core::pipeline::order_issues(&c.doc).is_empty());
        }
    }

    #[test]
    fn a_batch_makes_one_sign_per_name_in_a_grid_over_as_many_sheets_as_it_takes() {
        let names = (1..=7).map(|i| format!("NAME {i}")).collect::<Vec<_>>().join("\n\n");
        let mut c = Canvas::default();
        // 10 × 4 signs on 21.5 × 14 sheets: 2 across, 3 down = 6 a sheet → 2 sheets
        let s = SignSpec { words: names, w: 10.0, h: 4.0, t: 0.5, shape: 1, border: true, family: DEFAULT_FONT.into(), bold: true, batch: Some((21.5, 14.0)) };
        let msg = build(&mut c, &s);
        assert!(msg.starts_with("New signs: 7 signs on 2 sheets"), "{msg}");
        assert_eq!(c.doc.texts.len(), 7);
        let cuts: Vec<_> = c.doc.shapes.iter().filter(|g| g.layer == c.doc.ops[2].layer_scope).collect();
        assert_eq!(cuts.len(), 7);
        for g in &cuts {
            let b = bounds(&g.c);
            assert!(b.x0 > 0.0 && b.y0 > 0.0 && b.x1 < 21.5 && b.y1 < 14.0, "{b:?} on the sheet");
            // each sign's letters sit inside its own cut-out, on its sheet
            let inside = c.doc.shapes.iter().filter(|l| l.text != 0 && l.sheet == g.sheet).filter(|l| { let lb = bounds(&l.c); lb.x0 > b.x0 && lb.x1 < b.x1 && lb.y0 > b.y0 && lb.y1 < b.y1 }).count();
            assert!(inside > 0);
        }
        for j in 0..2 {
            c.doc.set_active_sheet(j);
            let gen = gcarve_core::pipeline::regenerate(&c.doc);
            assert!(gen.warnings.is_empty(), "sheet {j}: {:?}", gen.warnings);
            assert!(gen.paths.iter().all(|p| !p.is_empty()), "sheet {j} cuts every toolpath");
        }
    }

    /// look at one: GCARVE_SIGN_OUT=x.png cargo test -p gcarve-app sign_picture -- --ignored
    #[test]
    #[ignore]
    fn sign_picture() {
        let Ok(out) = std::env::var("GCARVE_SIGN_OUT") else { return };
        let mut c = Canvas::default();
        build(&mut c, &SignSpec { words: "THE SMITHS\nEST 2026".into(), w: 20.0, h: 10.0, t: 0.75, shape: 1, border: true, family: DEFAULT_FONT.into(), bold: true, batch: None });
        let gen = gcarve_core::pipeline::regenerate(&c.doc);
        let (rgba, w, h, _) = crate::setupsheet::carved(&c.doc, &gen, 600_000).unwrap();
        image::RgbaImage::from_raw(w, h, rgba).unwrap().save(out).unwrap();
    }
}
