//! Documents in tabs (Qt Main.qml docStack + DocCanvas): the current document lives in the App's fields; the others
//! wait here as `DocState`. The strip over the canvas shows only with two or more tabs, like the Qt app.

use crate::canvas::Canvas;
use crate::icons::IconButton;
use crate::theme;
use eframe::egui::{self, Color32};
use gcarve_core::pipeline::Generated;
use std::path::PathBuf;

/// Everything that belongs to one document.
pub struct DocState {
    /// stable id (Save As results find their tab by it)
    pub id: u64,
    pub canvas: Canvas,
    pub name: String,
    pub cloud_id: String,
    pub path: Option<PathBuf>,
    pub gen: Option<Generated>,
    pub gen_rev: u64,
}

impl DocState {
    pub fn new(id: u64, canvas: Canvas) -> Self {
        DocState { id, canvas, name: "Untitled".into(), cloud_id: String::new(), path: None, gen: None, gen_rev: 0 }
    }
    /// empty and untouched: an open reuses it instead of adding a tab
    pub fn blank(c: &Canvas) -> bool {
        c.doc.shapes.is_empty() && c.doc.ops.is_empty() && c.doc.reliefs.is_empty() && !c.dirty
    }
}

pub enum TabAct {
    Switch(usize),
    Close(usize),
    New,
}

/// Tab strip: (label, dirty) per tab, `cur` highlighted; × or middle-click closes, + adds.
pub fn strip(ui: &mut egui::Ui, labels: &[(String, bool)], cur: usize) -> Option<TabAct> {
    let mut act = None;
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, (name, dirty)) in labels.iter().enumerate() {
            let text = format!("{}{name}", if *dirty { "• " } else { "" });
            let galley = ui.painter().layout_no_wrap(text, theme::font(12.0), theme::TEXT);
            let w = (galley.size().x + 44.0).min(220.0);
            let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 26.0), egui::Sense::click());
            let on = i == cur;
            let fill = if on { theme::CANVAS_BG } else if resp.hovered() { theme::ROW_HOVER } else { Color32::TRANSPARENT };
            ui.painter().rect_filled(r, 5.0, fill);
            let clip = egui::Rect::from_min_max(r.min, egui::pos2(r.right() - 26.0, r.bottom()));
            ui.painter().with_clip_rect(clip).galley(egui::pos2(r.left() + 10.0, r.center().y - galley.size().y / 2.0), galley, if on { theme::TEXT } else { theme::TEXT.gamma_multiply(0.7) });
            let x = egui::Rect::from_center_size(egui::pos2(r.right() - 13.0, r.center().y), egui::vec2(18.0, 18.0));
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(x).layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)));
            if IconButton::new("remove").size(11.0).padding(3.0).tip("Close (⌘W)").show(&mut c).clicked() || resp.middle_clicked() {
                act = Some(TabAct::Close(i));
            } else if resp.clicked() {
                act = Some(TabAct::Switch(i));
            }
        }
        if IconButton::new("add").size(14.0).padding(4.0).tip("New document tab (⌘N)").show(ui).clicked() {
            act = Some(TabAct::New);
        }
    });
    act
}
