//! Clock Maker (File › New Clock…): a round or square clock face in a new tab: numerals (1–12 or Roman) V-carved
//! round the dial, hour (and minute) ticks engraved, the hole for a quartz movement's shaft and the cut-out with tabs.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::sign::{bit, outline_of};
use super::{hint, unit_label, Host};
use crate::canvas::Canvas;
use eframe::egui;
use gcarve_core::document::{Op, OpType, Side, TextItem, ToolType};
use gcarve_core::geom::{make_ellipse, make_polyline, pt};

#[derive(Clone, Debug)]
pub struct ClockSpec {
    /// face size (diameter, or side of a square face), inches
    pub size: f64,
    pub t: f64,
    /// 0 numbers 1–12, 1 Roman, 2 ticks only
    pub numerals: u8,
    pub square: bool,
    pub minutes: bool,
    /// the movement's shaft hole
    pub hole: f64,
}

const NUMERALS: [&str; 3] = ["Numbers 1–12", "Roman (I–XII)", "Hour marks only"]; // i18n
const ROMAN: [&str; 12] = ["XII", "I", "II", "III", "IIII", "V", "VI", "VII", "VIII", "IX", "X", "XI"];

pub struct ClockDialog {
    /// size, thickness, hole
    size: [String; 3],
    numerals: u8,
    square: bool,
    minutes: bool,
    pub done: Option<ClockSpec>,
}

impl ClockDialog {
    pub fn new(doc: &gcarve_core::document::Document) -> Self {
        ClockDialog { size: [12.0, 0.75, 0.3125].map(|v| crate::units::text(doc, v)), numerals: 0, square: false, minutes: true, done: None }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let (mut open, mut ok, mut cancel) = (true, false, false);
        let doc = &host.canvas.doc;
        crate::widgets::Dialog::new(i18n::t("New Clock")).open(&mut open).width(400.0).show(ctx, |ui| {
            egui::Grid::new("clock").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                for (i, label) in ["Face size", i18n::t("Board thickness"), "Hole for the movement"].iter().enumerate() {
                    ui.label(unit_label(label, doc));
                    crate::units::num_field(ui, label, &mut self.size[i], doc, true, 110.0);
                    ui.end_row();
                }
                ui.label(i18n::t("Numerals"));
                egui::ComboBox::from_id_salt("clock-num").selected_text(i18n::t(NUMERALS[self.numerals as usize])).show_ui(ui, |ui| {
                    for (i, s) in NUMERALS.iter().enumerate() {
                        ui.selectable_value(&mut self.numerals, i as u8, i18n::t(s));
                    }
                });
                ui.end_row();
                ui.label(i18n::t("Shape"));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.square, false, i18n::t("Round"));
                    ui.selectable_value(&mut self.square, true, i18n::t("Square"));
                });
                ui.end_row();
            });
            crate::widgets::checkbox(ui, &mut self.minutes, i18n::t("Minute marks"));
            hint(ui, i18n::t("Opens as a new project: numerals V-carved round the dial, engraved marks, the hole for the movement's shaft (most quartz movements: 5/16 in) and the cut-out with tabs."));
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
                Ok(n) if n[0] < 4.0 || n[1] <= 0.0 => *host.message = i18n::t("New clock: at least 4 in across, with a thickness.").into(),
                Ok(n) if n[2] < 0.13 || n[2] > n[0] / 4.0 => *host.message = i18n::t("New clock: the hole must be bigger than 1/8 in and small next to the face.").into(),
                Ok(n) => {
                    self.done = Some(ClockSpec { size: n[0], t: n[1], numerals: self.numerals, square: self.square, minutes: self.minutes, hole: n[2] });
                    return false;
                }
            }
        }
        open
    }
}

/// Fills an empty canvas with the clock job. Returns what was made, for the status line.
pub fn build(c: &mut Canvas, s: &ClockSpec) -> String {
    let side = s.size + 1.0; // half an inch of board round the face
    let m = &mut c.doc.material;
    (m.w, m.h, m.t) = (side, side, s.t);
    let a = c.doc.active_sheet.max(0) as usize;
    if let Some(sh) = c.doc.sheets.get_mut(a) {
        *sh = c.doc.material.clone();
    }
    let vbit = bit(c, ToolType::VBit, 0.5, i18n::t("90° V-Bit (0.5 inch)"));
    let mill = bit(c, ToolType::EndMill, 0.25, i18n::t("1/4 End Mill (0.25 inch)"));
    let small = bit(c, ToolType::EndMill, 0.125, i18n::t("1/8 End Mill (0.125 inch)"));
    let (cx, cy, r) = (side / 2.0, side / 2.0, s.size / 2.0);
    let cut = c.doc.add(if s.square { outline_of(side, side, 1, 0.5) } else { make_ellipse(cx, cy, r, r) }, None);
    let hole = c.doc.add(make_ellipse(cx, cy, s.hole / 2.0, s.hole / 2.0), None);
    // marks: hour marks longer, minute marks short, from the rim inwards
    let at = |k: f64, rad: f64| { let a = std::f64::consts::FRAC_PI_2 - k * std::f64::consts::TAU / 60.0; pt(cx + rad * a.cos(), cy + rad * a.sin()) };
    let rim = r - 0.08 * r;
    let (mut hours, mut minutes) = (Vec::new(), Vec::new());
    for k in 0..60 {
        let hour = k % 5 == 0;
        if !hour && !s.minutes {
            continue;
        }
        let len = if hour { 0.08 * r } else { 0.035 * r };
        let id = c.doc.add(make_polyline(&[at(k as f64, rim), at(k as f64, rim - len)], false), None);
        if hour { hours.push(id) } else { minutes.push(id) }
    }
    // a V-bit groove is twice as wide as it is deep: hour marks bold, minute marks fine
    let deep = (0.03 * r).clamp(0.04, 0.1);
    let mut ops = vec![Op { kind: OpType::Profile, name: i18n::t("Hour marks").into(), tool: vbit, shape_ids: hours, depth: deep, side: Side::On, ..Default::default() }];
    if !minutes.is_empty() {
        ops.push(Op { kind: OpType::Profile, name: i18n::t("Minute marks").into(), tool: vbit, shape_ids: minutes, depth: deep * 0.4, side: Side::On, ..Default::default() });
    }
    let mut made = i18n::t("engraved marks").to_string();
    if s.numerals < 2 {
        // numerals centred on a ring inside the marks; Roman ones are wider, so smaller
        let h = if s.numerals == 1 { 0.11 * r } else { 0.15 * r };
        let ring = rim - 0.1 * r - h * if s.numerals == 1 { 1.0 } else { 0.8 };
        let mut letters = Vec::new();
        for (k, roman) in ROMAN.iter().enumerate() {
            let text = if s.numerals == 1 { roman.to_string() } else { (if k == 0 { 12 } else { k }).to_string() };
            c.place_text(TextItem { text, height: h, align: 1, bold: true, ..Default::default() }, at(5.0 * k as f64, ring), None);
            let id = c.doc.texts.last().map_or(0, |t| t.id);
            letters.extend(c.doc.shapes.iter().filter(|g| g.text == id).map(|g| g.id));
        }
        ops.insert(0, Op { kind: OpType::VCarve, name: i18n::t("Numerals").into(), tool: vbit, shape_ids: letters, depth: (s.t / 4.0).min(0.2), flat_depth: (s.t / 4.0).min(0.2), ..Default::default() });
        made = format!("{} V-carved, {made}", if s.numerals == 1 { i18n::t("Roman numerals") } else { "numbers" });
    }
    ops.push(Op { kind: OpType::Profile, name: i18n::t("Movement hole").into(), tool: small, shape_ids: vec![hole], depth: s.t, side: Side::Inside, ..Default::default() });
    ops.push(Op { kind: OpType::Profile, name: i18n::t("Cut out").into(), tool: mill, shape_ids: vec![cut], depth: s.t, side: Side::Outside, tabs: true, tab_count: 4, tab_corners: true, ..Default::default() });
    c.doc.ops = ops;
    c.sel.clear();
    c.mutated();
    format!("New clock: {made}, the movement hole and a cut-out with tabs. Check it in Simulate, then Export.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::geom::{bounds, Rect};

    #[test]
    fn a_clock_has_twelve_numerals_round_the_dial_inside_the_face_and_clean_toolpaths() {
        for (numerals, square) in [(0, false), (1, false), (2, true)] {
            let mut c = Canvas::default();
            let s = ClockSpec { size: 12.0, t: 0.75, numerals, square, minutes: true, hole: 0.3125 };
            build(&mut c, &s);
            assert_eq!(c.doc.ops.last().unwrap().name, "Cut out");
            if numerals < 2 {
                assert_eq!(c.doc.texts.len(), 12);
                let mut all = Rect::default();
                for t in &c.doc.texts {
                    let mut b = Rect::default();
                    c.doc.shapes.iter().filter(|g| g.text == t.id).for_each(|g| b.add_rect(&bounds(&g.c)));
                    let (mx, my) = ((b.x0 + b.x1) / 2.0 - 6.5, (b.y0 + b.y1) / 2.0 - 6.5);
                    assert!(mx.hypot(my) < 6.0 * 0.85 && mx.hypot(my) > 6.0 * 0.5, "{numerals}: {:?} at {:.2}", t.text, mx.hypot(my));
                    all.add_rect(&b);
                }
                // 12 at the top, 6 at the bottom
                let centre_of = |s: &str| { let id = c.doc.texts.iter().find(|t| t.text == s).unwrap().id; let mut b = Rect::default(); c.doc.shapes.iter().filter(|g| g.text == id).for_each(|g| b.add_rect(&bounds(&g.c))); (b.x0 + b.x1) / 2.0 };
                let top = if numerals == 1 { "XII" } else { "12" };
                assert!((centre_of(top) - 6.5).abs() < 0.05, "12 centred at the top");
                assert!(all.x0 > 1.0 && all.x1 < 12.0);
            } else {
                assert!(c.doc.texts.is_empty());
            }
            let marks = |n: &str| c.doc.ops.iter().find(|o| o.name == n).map_or(0, |o| o.shape_ids.len());
            assert_eq!((marks("Hour marks"), marks("Minute marks")), (12, 48));
            let gen = gcarve_core::pipeline::regenerate(&c.doc);
            assert!(gen.warnings.is_empty(), "{numerals}: {:?}", gen.warnings);
            assert!(gcarve_core::pipeline::order_issues(&c.doc).is_empty());
        }
    }

    /// look at one: GCARVE_CLOCK_OUT=x.png cargo test -p gcarve-app clock_picture -- --ignored
    #[test]
    #[ignore]
    fn clock_picture() {
        let Ok(out) = std::env::var("GCARVE_CLOCK_OUT") else { return };
        let mut c = Canvas::default();
        build(&mut c, &ClockSpec { size: 12.0, t: 0.75, numerals: 1, square: false, minutes: true, hole: 0.3125 });
        let gen = gcarve_core::pipeline::regenerate(&c.doc);
        let (rgba, w, h, _) = crate::setupsheet::carved(&c.doc, &gen, 600_000).unwrap();
        image::RgbaImage::from_raw(w, h, rgba).unwrap().save(out).unwrap();
    }
}
