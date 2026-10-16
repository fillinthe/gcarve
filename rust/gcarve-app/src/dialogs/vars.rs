//! Variables & Constraints (Qt VariablesDialog.qml, modeless) and Constrain Shapes (Qt ConstraintDialog.qml).

use super::{hint, Host};
use crate::widgets::TipExt;
use crate::canvas::Canvas;
use crate::params::variable_usage;
use crate::theme;
use crate::units;
use eframe::egui::{self, RichText};
use gcarve_core::document::Document;
use std::collections::HashMap;

/// Edit buffers of the open dialog; a field shows the document value until it has focus.
#[derive(Default)]
pub struct VarsDialog {
    bufs: HashMap<egui::Id, String>,
}

/// Single-line field bound to `value`: returns the committed text when focus leaves with a change.
fn field(ui: &mut egui::Ui, bufs: &mut HashMap<egui::Id, String>, id: egui::Id, value: &str, width: f32, color: egui::Color32, hint_text: &str) -> Option<String> {
    let buf = bufs.entry(id).or_default();
    if !ui.memory(|m| m.has_focus(id)) {
        *buf = value.to_string();
    }
    let r = ui.add(crate::widgets::field(buf).id(id).desired_width(width).text_color(color).hint_text(hint_text));
    (r.lost_focus() && buf.trim() != value).then(|| buf.clone())
}

impl VarsDialog {
    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut open = true;
        let c = &mut *host.canvas;
        type Act = Box<dyn FnOnce(&mut Canvas)>;
        let mut act: Option<Act> = None;
        let unit = units::name(&c.doc);
        crate::widgets::Dialog::new("Variables & Constraints").open(&mut open).close_footer().modeless(ctx.content_rect().right_top() + egui::vec2(-330.0, 80.0), egui::Align2::RIGHT_TOP).show(ctx, |ui| {
            ui.set_width(520.0);
            hint(ui, "Use a variable name in any size field (shapes, toolpaths, board, constraints) to link it. thickness, board-w and board-h follow the Board. Math and units work: 2*gap + 1in.");
            let doc = &c.doc;
            egui::ScrollArea::vertical().max_height(320.0).id_salt("vars").show(ui, |ui| {
                egui::Grid::new("vars").num_columns(5).spacing([6.0, 4.0]).show(ui, |ui| {
                    for (name, desc) in Document::builtin_variables() {
                        if doc.find_variable(name).is_some() {
                            continue; // shadowed by a user variable
                        }
                        let v = doc.eval_variable(name).unwrap_or(0.0);
                        let mut uses = variable_usage(doc, name);
                        if *name == "thickness" {
                            uses.extend(variable_usage(doc, "z"));
                            uses.extend(variable_usage(doc, "board-t"));
                        }
                        name_cell(ui, name, &uses, true);
                        ui.label(RichText::new("=").color(theme::TEXT_DIM));
                        ui.label(RichText::new(units::fmt(doc, v)).italics().color(theme::TEXT_DIM)).tip(format!("Built in ({desc}): follows the Board setup."));
                        ui.label(RichText::new(units::fmt(doc, v)).size(12.0).color(theme::TEXT_DIM));
                        ui.label("");
                        ui.end_row();
                    }
                    for (i, var) in doc.variables.iter().enumerate() {
                        let uses = variable_usage(doc, &var.name);
                        let used = !uses.is_empty();
                        let r = doc.eval_dim(&var.expr);
                        let name_col = if used { theme::ACCENT } else { theme::TEXT };
                        ui.horizontal(|ui| {
                            if let Some(n) = field(ui, &mut self.bufs, egui::Id::new(("var-name", i)), &var.name, 120.0, name_col, "") {
                                let from = var.name.clone();
                                act = Some(Box::new(move |c: &mut Canvas| c.rename_variable(&from, &n)));
                            }
                            badge(ui, &uses);
                        });
                        ui.label(RichText::new("=").color(theme::TEXT_DIM));
                        let col = if r.is_err() { theme::ORANGE } else if gcarve_core::expr::identifiers(&var.expr).is_empty() { theme::TEXT } else { theme::ACCENT };
                        let resp = ui.horizontal(|ui| field(ui, &mut self.bufs, egui::Id::new(("var-expr", i)), &var.expr, 200.0, col, "")).inner;
                        if let Some(e) = resp {
                            let n = var.name.clone();
                            act = Some(Box::new(move |c: &mut Canvas| c.set_variable(&n, &e)));
                        }
                        match &r {
                            // plain numbers (counts, ratios) show without a unit
                            Ok((v, len)) => ui.label(RichText::new(if *len { units::fmt(doc, *v) } else { format!("{}", (v * 1e4).round() / 1e4) }).size(12.0).color(theme::TEXT_DIM)),
                            Err(e) => ui.label(RichText::new("?").color(theme::ORANGE)).tip(e),
                        };
                        if crate::icons::IconButton::new("dismiss").size(12.0).padding(3.0).tip("Remove variable").show(ui).clicked() {
                            let n = var.name.clone();
                            act = Some(Box::new(move |c: &mut Canvas| c.remove_variable(&n)));
                        }
                        ui.end_row();
                    }
                });
            });
            if ui.button("Add variable").clicked() {
                act = Some(Box::new(|c: &mut Canvas| {
                    c.add_variable();
                }));
            }
            ui.separator();
            ui.label(RichText::new("Constraints").family(crate::widgets::bold_family()));
            if doc.constraints.is_empty() {
                hint(ui, "None yet. Select two shapes and use Link to keep one positioned relative to the other.");
            }
            egui::Grid::new("cons").num_columns(5).spacing([6.0, 4.0]).show(ui, |ui| {
                for (i, k) in doc.constraints.iter().enumerate() {
                    let selected = c.sel.contains(&k.anchor) || c.sel.contains(&k.target);
                    if ui.add(egui::Label::new(RichText::new(format!("{} → {}", k.target, k.anchor)).color(if selected { theme::ACCENT } else { theme::TEXT })).sense(egui::Sense::click())).tip("target → anchor. Click to select both shapes.").clicked() {
                        act = Some(Box::new(move |c: &mut Canvas| c.select_constraint_shapes(i)));
                    }
                    let mut kind = k.kind.clone();
                    let label = Document::constraint_types().iter().find(|(t, _)| *t == kind).map(|(_, l)| *l).unwrap_or("");
                    crate::widgets::combo_box(("con-kind", i)).selected_text(label).width(140.0).show_ui(ui, |ui| {
                        for (t, l) in Document::constraint_types() {
                            ui.selectable_value(&mut kind, t.to_string(), *l);
                        }
                    });
                    if kind != k.kind {
                        let e = k.expr.clone();
                        act = Some(Box::new(move |c: &mut Canvas| c.update_constraint(i, &kind, &e)));
                    }
                    let r = if k.expr.is_empty() { Ok(0.0) } else { doc.eval(&k.expr) };
                    let col = if r.is_err() { theme::ORANGE } else if gcarve_core::expr::identifiers(&k.expr).is_empty() { theme::TEXT } else { theme::ACCENT };
                    if let Some(e) = field(ui, &mut self.bufs, egui::Id::new(("con-expr", i)), &k.expr, 120.0, col, "0") {
                        act = Some(Box::new(move |c: &mut Canvas| c.update_constraint(i, "", &e)));
                    }
                    ui.label(RichText::new(units::fmt(doc, r.unwrap_or(0.0))).size(12.0).color(theme::TEXT_DIM));
                    if crate::icons::IconButton::new("dismiss").size(12.0).padding(3.0).tip("Remove constraint").show(ui).clicked() {
                        act = Some(Box::new(move |c: &mut Canvas| c.remove_constraint(i)));
                    }
                    ui.end_row();
                }
            });
            let _ = unit;
        });
        if let Some(f) = act {
            f(c);
        }
        open
    }
}

fn name_cell(ui: &mut egui::Ui, name: &str, uses: &[String], builtin: bool) {
    ui.horizontal(|ui| {
        let col = if !uses.is_empty() { theme::ACCENT } else { theme::TEXT };
        let mut t = RichText::new(name).color(col);
        if !uses.is_empty() {
            t = t.family(crate::widgets::bold_family());
        }
        if builtin {
            t = t.italics();
        }
        ui.add_sized([120.0, 20.0], egui::Label::new(t));
        badge(ui, uses);
    });
}

/// "×n" usage badge with the list of uses as its tooltip.
fn badge(ui: &mut egui::Ui, uses: &[String]) {
    if uses.is_empty() {
        return;
    }
    let text = format!("×{}", uses.len());
    let g = ui.painter().layout_no_wrap(text, theme::font(11.0), egui::Color32::WHITE);
    let (r, resp) = ui.allocate_exact_size(egui::vec2(g.size().x + 10.0, 18.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 9.0, theme::ACCENT);
    ui.painter().galley(r.center() - g.size() / 2.0, g, egui::Color32::WHITE);
    resp.tip(format!("Used by:\n{}", uses.join("\n")));
}

/// Vectors ▸ Constrain Two Shapes… (Link): anchor stays, target follows.
pub struct ConstraintDialog {
    anchor: i32,
    target: i32,
    kind: String,
    expr: String,
}

impl ConstraintDialog {
    pub fn new(canvas: &Canvas) -> Self {
        let ids: Vec<i32> = canvas.sel.iter().copied().collect();
        ConstraintDialog { anchor: ids.first().copied().unwrap_or(0), target: ids.get(1).copied().unwrap_or(0), kind: "right-of".into(), expr: String::new() }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let mut keep = true;
        let mut ok = false;
        let doc = &host.canvas.doc;
        crate::widgets::Dialog::new("Constrain Shapes").width(400.0).show(ctx, |ui| {
            hint(ui, "Keeps one shape positioned relative to another. Drag the anchor and the target follows; the gap can be a number, math or a variable.");
            egui::Grid::new("con").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                ui.label("Anchor (stays)");
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Shape {}", self.anchor)).family(crate::widgets::bold_family()));
                    if ui.add(crate::widgets::Button::new("Swap").icon("swap", false)).clicked() {
                        std::mem::swap(&mut self.anchor, &mut self.target);
                    }
                });
                ui.end_row();
                ui.label("Target (moves)");
                ui.label(RichText::new(format!("Shape {}", self.target)).family(crate::widgets::bold_family()));
                ui.end_row();
                ui.label("Relation");
                let label = Document::constraint_types().iter().find(|(t, _)| *t == self.kind).map(|(_, l)| *l).unwrap_or("");
                crate::widgets::combo_box("con-type").selected_text(label).width(280.0).show_ui(ui, |ui| {
                    for (t, l) in Document::constraint_types() {
                        ui.selectable_value(&mut self.kind, t.to_string(), *l);
                    }
                });
                ui.end_row();
                ui.label(format!("Gap / offset ({})", units::name(doc)));
                ui.add(crate::widgets::field(&mut self.expr).desired_width(280.0).hint_text("0, 0.5, gap, 2*gap + 1in"));
                ui.end_row();
            });
            let (o, c) = crate::widgets::ok_cancel(ui);
            ok |= o;
            keep &= !c;
        });
        if ok {
            host.canvas.add_constraint(self.anchor, self.target, &self.kind, &self.expr);
            keep = false;
        }
        keep
    }
}
