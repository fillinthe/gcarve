//! Project units (Qt app: Units.qml + NumField.qml): the document is inches; fields show/accept the project unit
//! (in / ft / mm, from `Document::bare_unit_in`) and any length expression ("1in + 5mm", variables).

use eframe::egui;
use crate::widgets::TipExt;
use gcarve_core::document::Document;

/// index 0 in, 1 ft, 2 mm (Qt Canvas::unitIndex)
pub const BARE: [f64; 3] = [1.0, 12.0, 1.0 / 25.4];
pub const NAMES: [&str; 3] = ["in", "ft", "mm"];

pub fn index(doc: &Document) -> usize {
    if doc.bare_unit_in > 10.0 {
        1
    } else if doc.bare_unit_in < 0.5 {
        2
    } else {
        0
    }
}
pub fn name(doc: &Document) -> &'static str {
    NAMES[index(doc)]
}
pub fn decimals(doc: &Document) -> usize {
    [3, 4, 2][index(doc)]
}
/// inches -> project unit
pub fn to_unit(doc: &Document, inches: f64) -> f64 {
    inches / doc.bare_unit_in
}
pub fn fmt(doc: &Document, inches: f64) -> String {
    format!("{:.*} {}", decimals(doc), to_unit(doc, inches), name(doc))
}
/// inches -> field text (no suffix)
pub fn text(doc: &Document, inches: f64) -> String {
    format!("{:.*}", decimals(doc), to_unit(doc, inches))
}
/// Length expression in the project unit -> inches (variables resolved from the document).
pub fn parse(doc: &Document, s: &str) -> Result<f64, String> {
    doc.eval(s)
}

/// Text field with a unit badge; Enter or focus loss commits. Returns the new value in inches when it was
/// committed and evaluates (`length = false`: a plain number such as a count or a feed, no unit).
/// The next `num_field` drawn takes the keyboard focus with its text selected (Qt `forceActiveFocus(); selectAll()`)
pub fn focus_next(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("num-field-focus"), true));
}

pub fn num_field(ui: &mut egui::Ui, id: &str, buf: &mut String, doc: &Document, length: bool, width: f32) -> Option<f64> {
    let mut committed = None;
    ui.horizontal(|ui| {
        let suffix = if length { name(doc) } else { "" };
        let r = ui.push_id(id, |ui| crate::widgets::unit_field(ui, buf, suffix, width)).inner;
        if ui.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("num-field-focus"))).is_some() {
            r.request_focus();
            let mut st = egui::TextEdit::load_state(ui.ctx(), r.id).unwrap_or_default();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(buf.chars().count()))));
            st.store(ui.ctx(), r.id);
        }
        let value = if length { parse(doc, buf) } else { buf.trim().parse::<f64>().map_err(|e| e.to_string()) };
        let r = match &value {
            Ok(v) if length => r.tip(format!("= {}", fmt(doc, *v))),
            Err(e) if !buf.trim().is_empty() => r.tip(egui::RichText::new(e).color(crate::theme::ORANGE)),
            _ => r,
        };
        if r.lost_focus() {
            committed = value.ok(); // a single-line edit drops focus on Enter too
        }
    });
    committed
}

/// Evaluate a field at OK time: Ok(inches) or the message for the status line.
pub fn get(doc: &Document, buf: &str, length: bool) -> Result<f64, String> {
    if length {
        parse(doc, buf).map_err(|e| format!("Can't evaluate \"{}\": {e}", buf.trim()))
    } else {
        buf.trim().parse::<f64>().map_err(|_| format!("\"{}\" is not a number", buf.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::document::Variable;

    #[test]
    fn unit_switch_and_expressions() {
        let mut d = Document::default();
        assert_eq!(name(&d), "in");
        assert_eq!(parse(&d, "1in + 5mm").unwrap(), 1.0 + 5.0 / 25.4);
        d.bare_unit_in = BARE[2];
        assert_eq!(name(&d), "mm");
        assert!((parse(&d, "25.4").unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(text(&d, 1.0), "25.40");
        d.variables.push(Variable { name: "gap".into(), expr: "2".into() });
        let v = parse(&d, "gap + 2"); // bare numbers are lengths too: 2 mm + 2 mm
        assert!((v.clone().unwrap() - 4.0 / 25.4).abs() < 1e-12, "{v:?}");
        assert!(get(&d, "abc", true).unwrap_err().contains("abc"));
    }
}
