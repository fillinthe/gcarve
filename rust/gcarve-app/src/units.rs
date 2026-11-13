//! Project units (Qt app: Units.qml + NumField.qml): the document is inches; fields show/accept the project unit
//! (in / ft / mm, from `Document::bare_unit_in`) and any length expression ("1in + 5mm", variables).

use gcarve_ui::i18n;
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

/// The name being typed at the end of `buf` (letters, digits, _ and -, starting with a letter or _), and where it
/// starts. A name may hold "-" (board-w), so "a-b" is read as one name, as the expression parser does.
fn typed_name(buf: &str) -> Option<(usize, &str)> {
    let start = buf.char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '-').last().map(|(i, _)| i)?;
    let w = &buf[start..];
    let w = w.trim_start_matches(|c: char| c == '-' || c.is_ascii_digit());
    let start = buf.len() - w.len();
    w.chars().next().filter(|c| c.is_alphabetic() || *c == '_').map(|_| (start, w))
}

/// The variables whose name starts with (else holds) `typed`, best first, with their value: the project's own, then
/// the board's built-ins. Units words (in, mm, ft…) alone don't ask.
pub fn variable_matches(doc: &Document, typed: &str) -> Vec<(String, String)> {
    let t = typed.to_lowercase();
    if matches!(t.as_str(), "in" | "inch" | "mm" | "cm" | "m" | "ft") {
        return Vec::new();
    }
    let names: Vec<String> = doc.variables.iter().map(|v| v.name.clone()).chain(Document::builtin_variables().iter().map(|(n, _)| n.to_string())).collect();
    let mut hits: Vec<(u8, String)> = names.into_iter().filter_map(|n| {
        let l = n.to_lowercase();
        if l == t {
            None // typed in full already
        } else if l.starts_with(&t) {
            Some((0, n))
        } else if l.contains(&t) {
            Some((1, n))
        } else {
            None
        }
    }).collect();
    hits.sort();
    hits.dedup_by(|a, b| a.1 == b.1);
    hits.into_iter().take(8).map(|(_, n)| {
        let v = doc.eval_variable(&n).map(|v| fmt(doc, v)).unwrap_or_else(|e| e);
        (n, v)
    }).collect()
}

/// A suggestion list was shown last frame: (Tab, ↓, ↑) taken from this frame's input before the text box under it
/// can use them (a text box moves focus on Tab).
const SUGGEST_OPEN: &str = "var-suggest-open";
const SUGGEST_KEYS: &str = "var-suggest-keys";

/// Call first thing each frame.
pub fn take_suggest_keys(ctx: &egui::Context) {
    let open = ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new(SUGGEST_OPEN))).unwrap_or(false);
    let keys = if open {
        ctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::Tab), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)))
    } else {
        (false, false, false)
    };
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(SUGGEST_KEYS), keys));
}

/// Under a field that takes sizes: while a name is being typed, the variables it could be, to click (or Tab / ↓ then
/// Enter) so it is written in full. Returns true when the text changed. Any field that accepts an expression calls
/// this after drawing its TextEdit (owner 2026-11-13: "it should search for variables to use … check all fields").
pub fn suggest_variables(ui: &mut egui::Ui, r: &egui::Response, buf: &mut String, doc: &Document) -> bool {
    let key = r.id.with("var-suggest");
    // (the list stays while the pointer is on it, so a click on it lands although the field lost focus on the press)
    let (was_open, pick) = ui.data(|d| d.get_temp::<(egui::Rect, usize)>(key)).map_or((false, 0), |(rect, i)| (ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p)), i));
    // (egui moves focus on Tab before this runs: the field that just lost it still owns its list this frame)
    let mine = r.has_focus() || r.lost_focus();
    if !(mine || was_open) {
        ui.data_mut(|d| d.remove::<(egui::Rect, usize)>(key));
        return false;
    }
    let Some((start, typed)) = typed_name(buf) else {
        ui.data_mut(|d| d.remove::<(egui::Rect, usize)>(key));
        return false;
    };
    let hits = variable_matches(doc, typed);
    if hits.is_empty() {
        ui.data_mut(|d| d.remove::<(egui::Rect, usize)>(key));
        return false;
    }
    let mut pick = pick.min(hits.len() - 1);
    // the keys were taken at the start of the frame (take_suggest_keys); only the field with focus uses them
    let (take, down, up) = if mine { ui.data(|d| d.get_temp::<(bool, bool, bool)>(egui::Id::new(SUGGEST_KEYS))).unwrap_or_default() } else { (false, false, false) };
    ui.data_mut(|d| d.insert_temp(egui::Id::new(SUGGEST_OPEN), true));
    // while the list shows, Tab and the arrows belong to the field (egui would move focus on them before any code
    // of ours runs); take_suggest_keys hands them to the list next frame
    if r.has_focus() {
        ui.memory_mut(|m| m.set_focus_lock_filter(r.id, egui::EventFilter { tab: true, vertical_arrows: true, horizontal_arrows: true, escape: false }));
    }
    if down {
        pick = (pick + 1) % hits.len();
    }
    if up {
        pick = (pick + hits.len() - 1) % hits.len();
    }
    let mut chosen = take.then_some(pick);
    let area = egui::Area::new(key).order(egui::Order::Tooltip).fixed_pos(r.rect.left_bottom() + egui::vec2(0.0, 2.0)).show(ui.ctx(), |ui| {
        crate::widgets::popup_frame().show(ui, |ui| {
            ui.set_min_width(r.rect.width().max(180.0));
            ui.label(egui::RichText::new(i18n::t("Variables (click, or Tab)")).small().color(crate::theme::TEXT_DIM));
            for (k, (n, v)) in hits.iter().enumerate() {
                let row = ui.horizontal(|ui| {
                    let l = ui.selectable_label(k == pick, egui::RichText::new(n).strong());
                    ui.label(egui::RichText::new(format!("= {v}")).small().color(crate::theme::TEXT_DIM));
                    l
                }).inner;
                if row.clicked() {
                    chosen = Some(k);
                }
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(key, (area.response.rect, pick)));
    if let Some(k) = chosen {
        buf.replace_range(start.., &hits[k].0);
        ui.data_mut(|d| d.remove::<(egui::Rect, usize)>(key));
        r.request_focus(); // carry on typing after the name
        let end = buf.chars().count();
        let mut st = egui::TextEdit::load_state(ui.ctx(), r.id).unwrap_or_default();
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
        st.store(ui.ctx(), r.id);
        return true;
    }
    false
}

pub fn num_field(ui: &mut egui::Ui, id: &str, buf: &mut String, doc: &Document, length: bool, width: f32) -> Option<f64> {
    let mut committed = None;
    ui.horizontal(|ui| {
        let suffix = if length { name(doc) } else { "" };
        let r = ui.push_id(id, |ui| crate::widgets::unit_field(ui, buf, suffix, width)).inner;
        let picked = length && suggest_variables(ui, &r, buf, doc);
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
        // a single-line edit drops focus on Enter too; not while a name is being picked from the list under it
        let list = ui.data(|d| d.get_temp::<(egui::Rect, usize)>(r.id.with("var-suggest")));
        let picking = list.is_some_and(|(rect, _)| ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p)));
        if r.lost_focus() && !picking && !picked {
            committed = value.ok();
        }
    });
    committed
}

/// Evaluate a field at OK time: Ok(inches) or the message for the status line.
pub fn get(doc: &Document, buf: &str, length: bool) -> Result<f64, String> {
    if length {
        parse(doc, buf).map_err(|e| i18n::tf("Can't evaluate \"{buf}\": {e}", &[("buf", &(buf.trim())), ("e", &e)]))
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

#[cfg(test)]
mod suggest_tests {
    use super::{typed_name, variable_matches};
    use gcarve_core::document::{Document, Variable};

    #[test]
    fn the_name_being_typed_is_offered_the_variables_it_could_be() {
        assert_eq!(typed_name("thick"), Some((0, "thick")));
        assert_eq!(typed_name("2*board"), Some((2, "board")));
        assert_eq!(typed_name("1in + gap"), Some((6, "gap")));
        assert_eq!(typed_name("12"), None);
        assert_eq!(typed_name("3 "), None);
        let mut d = Document::default();
        d.variables.push(Variable { name: "shelf-gap".into(), expr: "2".into() });
        d.variables.push(Variable { name: "thick2".into(), expr: "thickness*2".into() });
        let names = |t: &str| variable_matches(&d, t).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names("thi"), vec!["thick2", "thickness"]); // starts-with, the project's first
        assert_eq!(names("board"), vec!["board-h", "board-w"]);
        assert_eq!(names("gap"), vec!["shelf-gap"]); // found inside the name too
        assert!(names("mm").is_empty() && names("in").is_empty()); // units words don't ask
        assert!(names("thickness").is_empty()); // typed in full: nothing left to offer
        assert_eq!(variable_matches(&d, "thick")[0].1, "1.500 in"); // shown with its value
    }
}
