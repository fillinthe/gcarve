//! Sign Wizard (File › New Sign…): type the words, the board size and a shape, and get a ready job in a new tab:
//! V-carved letters fitted to the board, an optional V-groove border and an outside cut-out with tabs, using bits
//! from the user's library (a 90° V-bit and a 1/4 in end mill are added when the library has none). The community
//! starter signs are made with it too.

use super::{hint, unit_label, Host};
use crate::canvas::Canvas;
use eframe::egui;
use gcarve_core::document::{Op, OpType, Side, TextItem, Tool, ToolType};
use gcarve_core::geom::{bounds, make_ellipse, make_rect, pt, Contour};
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
}

const SHAPES: [&str; 3] = ["Rectangle", "Rounded corners", "Oval"];
const DEFAULT_FONT: &str = "Liberation Sans";

pub struct SignDialog {
    words: String,
    size: [String; 3],
    shape: u8,
    border: bool,
    family: String,
    bold: bool,
    /// OK pressed: the app makes the sign in a new tab
    pub done: Option<SignSpec>,
}

impl SignDialog {
    pub fn new(doc: &gcarve_core::document::Document) -> Self {
        let t = if doc.material.t > 0.0 { doc.material.t } else { 0.75 };
        SignDialog { words: "WELCOME".into(), size: [24.0, 8.0, t].map(|v| crate::units::text(doc, v)), shape: 1, border: true, family: DEFAULT_FONT.into(), bold: true, done: None }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let (mut open, mut ok, mut cancel) = (true, false, false);
        let doc = &host.canvas.doc;
        crate::widgets::Dialog::new("New Sign").open(&mut open).width(420.0).show(ctx, |ui| {
            ui.label("Words (one line each)");
            ui.add(egui::TextEdit::multiline(&mut self.words).desired_rows(2).desired_width(f32::INFINITY));
            egui::Grid::new("sign").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                for (i, label) in ["Board width", "Board height", "Board thickness"].iter().enumerate() {
                    ui.label(unit_label(label, doc));
                    crate::units::num_field(ui, label, &mut self.size[i], doc, true, 110.0);
                    ui.end_row();
                }
                ui.label("Shape");
                egui::ComboBox::from_id_salt("sign-shape").selected_text(SHAPES[self.shape as usize]).show_ui(ui, |ui| {
                    for (i, s) in SHAPES.iter().enumerate() {
                        ui.selectable_value(&mut self.shape, i as u8, *s);
                    }
                });
                ui.end_row();
                ui.label("Font");
                egui::ComboBox::from_id_salt("sign-font").selected_text(self.family.as_str()).show_ui(ui, |ui| {
                    for f in std::iter::once(DEFAULT_FONT.to_string()).chain(crate::text::custom_families()) {
                        ui.selectable_value(&mut self.family, f.clone(), f);
                    }
                });
                ui.end_row();
            });
            crate::widgets::checkbox(ui, &mut self.bold, "Bold letters");
            crate::widgets::checkbox(ui, &mut self.border, "V-groove border");
            hint(ui, "Opens as a new project: carved letters sized to fit, the border, and a cut-out with tabs. Change anything afterwards, then Simulate and Export.");
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
                Ok(n) if n[0] < 2.0 || n[1] < 1.0 || n[2] <= 0.0 => *host.message = "New sign: the board must be at least 2 × 1 in, with a thickness.".into(),
                Ok(_) if self.words.trim().is_empty() => *host.message = "New sign: type the words first.".into(),
                Ok(n) => {
                    self.done = Some(SignSpec { words: self.words.trim().into(), w: n[0], h: n[1], t: n[2], shape: self.shape, border: self.border, family: self.family.clone(), bold: self.bold });
                    return false;
                }
            }
        }
        open
    }
}

/// A bit of this kind from the project's tools (closest to `dia`, V-bits closest to 90°), added when there is none.
fn bit(c: &mut Canvas, kind: ToolType, dia: f64, name: &str) -> i32 {
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
    let (x1, y1) = (s.w - d, s.h - d);
    match s.shape {
        2 => make_ellipse(s.w / 2.0, s.h / 2.0, s.w / 2.0 - d, s.h / 2.0 - d),
        1 => {
            let r = (s.w.min(s.h) * 0.12 - (d - 0.3)).max(0.1);
            fillet_contours(&[make_rect(d, d, x1, y1)], r, true, true).into_iter().next().unwrap_or_else(|| make_rect(d, d, x1, y1))
        }
        _ => make_rect(d, d, x1, y1),
    }
}

/// Fills an empty canvas with the sign job. Returns what was made, for the status line.
pub fn build(c: &mut Canvas, s: &SignSpec) -> String {
    let m = &mut c.doc.material;
    (m.w, m.h, m.t) = (s.w, s.h, s.t);
    let a = c.doc.active_sheet.max(0) as usize;
    if let Some(sh) = c.doc.sheets.get_mut(a) {
        *sh = c.doc.material.clone();
    }
    let vbit = bit(c, ToolType::VBit, 0.5, "90° V-Bit (0.5 inch)");
    let mill = bit(c, ToolType::EndMill, 0.25, "1/4 End Mill (0.25 inch)");
    // cut-out 0.3 in in from the board edge, border 0.55 in further in, letters inside that with a little air
    let edge = 0.3;
    let cut = c.doc.add(outline(s, edge), None);
    let mut ops = Vec::new();
    let inner = if s.border {
        let b = c.doc.add(outline(s, edge + 0.55), None);
        ops.push(Op { kind: OpType::Profile, name: "Border groove".into(), tool: vbit, shape_ids: vec![b], depth: 0.1, side: Side::On, ..Default::default() });
        edge + 1.0
    } else {
        edge + 0.5
    };
    // the room for the letters: the inner box (an oval's largest box is 1/√2 of it)
    let k = if s.shape == 2 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
    let (aw, ah) = ((s.w / 2.0 - inner) * 2.0 * k, (s.h / 2.0 - inner) * 2.0 * k);
    let mut t = TextItem { text: s.words.clone(), family: s.family.clone(), height: 1.0, bold: s.bold, align: 1, ..Default::default() };
    let (glyphs, _) = crate::text::build_text(&t, pt(0.0, 0.0), None);
    let mut b = gcarve_core::geom::Rect::default();
    glyphs.iter().for_each(|g| b.add_rect(&bounds(g)));
    if glyphs.is_empty() || aw <= 0.0 || ah <= 0.0 {
        return "New sign: the board is too small for the words.".into();
    }
    t.height = (aw / (b.x1 - b.x0)).min(ah / (b.y1 - b.y0));
    c.place_text(t, pt(s.w / 2.0, s.h / 2.0), None);
    let id = c.doc.texts.last().map_or(0, |t| t.id);
    let letters: Vec<i32> = c.doc.shapes.iter().filter(|g| g.text == id).map(|g| g.id).collect();
    // V-carved to a flat floor no deeper than a third of the board: big letters don't go through thin boards
    let depth = (s.t / 3.0).min(0.25);
    ops.insert(0, Op { kind: OpType::VCarve, name: "Letters".into(), tool: vbit, shape_ids: letters, depth, flat_depth: depth, ..Default::default() });
    ops.push(Op { kind: OpType::Profile, name: "Cut out".into(), tool: mill, shape_ids: vec![cut], depth: s.t, side: Side::Outside, tabs: true, tab_count: 4, tab_corners: true, ..Default::default() });
    c.doc.ops = ops;
    c.sel.clear();
    c.mutated();
    format!("New sign: {} letters V-carved, {}a cut-out with tabs. Check it in Simulate, then Export.", c.doc.texts.last().map_or(0, |t| t.text.chars().filter(|ch| !ch.is_whitespace()).count()), if s.border { "a border groove, " } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sign_fits_its_letters_inside_the_border_and_makes_clean_toolpaths() {
        for shape in 0..3u8 {
            let mut c = Canvas::default();
            let s = SignSpec { words: "THE SMITHS\nEST 2026".into(), w: 20.0, h: 10.0, t: 0.75, shape, border: true, family: DEFAULT_FONT.into(), bold: true };
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

    /// look at one: GCARVE_SIGN_OUT=x.png cargo test -p gcarve-app sign_picture -- --ignored
    #[test]
    #[ignore]
    fn sign_picture() {
        let Ok(out) = std::env::var("GCARVE_SIGN_OUT") else { return };
        let mut c = Canvas::default();
        build(&mut c, &SignSpec { words: "THE SMITHS\nEST 2026".into(), w: 20.0, h: 10.0, t: 0.75, shape: 1, border: true, family: DEFAULT_FONT.into(), bold: true });
        let gen = gcarve_core::pipeline::regenerate(&c.doc);
        let (rgba, w, h, _) = crate::setupsheet::carved(&c.doc, &gen, 600_000).unwrap();
        image::RgbaImage::from_raw(w, h, rgba).unwrap().save(out).unwrap();
    }
}
