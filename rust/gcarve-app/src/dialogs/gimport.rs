//! File ▸ Import G-code: one row per tool section with what the file says about its bit, and the bit to use
//! (one already in the project, one from the bit library, or a new one made from the file). Nothing is guessed
//! silently: the user sees the evidence and confirms.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use super::{combo, hint, Host};
use eframe::egui::{self, RichText};
use gcarve_core::document::Tool;
use gcarve_core::gimport::{apply_import, guess_text, match_tool, read_program, ImportedProgram};

/// "Remember my choice": tool comment (or toolpath name when the file has none) -> the bit's name, local store.
const MEMORY_KEY: &str = "gcode-bits.json";

fn memory() -> std::collections::BTreeMap<String, String> {
    crate::store::load(MEMORY_KEY).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn memory_key(s: &gcarve_core::gimport::ImportSection) -> String {
    let k = if s.tool_text.trim().is_empty() { &s.name } else { &s.tool_text };
    k.trim().to_lowercase()
}

#[derive(Clone, Copy, PartialEq)]
enum Bit {
    New,
    Doc(usize),
    Lib(usize),
}

pub struct GcodeImportDialog {
    name: String,
    stem: String,
    text: String,
    prog: ImportedProgram,
    bits: Vec<Bit>,
    library: Vec<Tool>,
    set_board: bool,
    vectors: bool,
}

impl GcodeImportDialog {
    pub fn new(name: &str, text: String, host_doc: &gcarve_core::document::Document) -> Self {
        let stem = name.rsplit_once('.').map_or(name, |(a, _)| a).to_string();
        let prog = read_program(&text, &stem, &host_doc.material);
        let library = crate::toollib::load();
        let mem = memory();
        let bits = prog
            .sections
            .iter()
            .map(|s| {
                // a bit picked for this tool comment before wins, then the usual name / size match
                if let Some(name) = mem.get(&memory_key(s)) {
                    if let Some(i) = host_doc.tools.iter().position(|t| &t.name == name) {
                        return Bit::Doc(i);
                    }
                    if let Some(i) = library.iter().position(|t| &t.name == name) {
                        return Bit::Lib(i);
                    }
                }
                match (match_tool(s, &host_doc.tools), match_tool(s, &library)) {
                    (Some(i), _) => Bit::Doc(i),
                    (None, Some(i)) => Bit::Lib(i),
                    _ => Bit::New,
                }
            })
            .collect();
        let empty = host_doc.shapes.is_empty() && host_doc.ops.is_empty();
        GcodeImportDialog { name: name.into(), stem, text, set_board: empty && prog.board.is_some(), vectors: true, prog, bits, library }
    }

    /// false = closed
    pub fn ui(&mut self, ctx: &egui::Context, host: &mut Host) -> bool {
        let (mut ok, mut keep) = (false, true);
        let doc = &host.canvas.doc;
        let n = self.prog.sections.len();
        crate::widgets::Dialog::new(format!("Import G-code: {}", self.name)).id("gimport").width(620.0).show(ctx, |ui| {
            if n == 0 {
                ui.label(RichText::new(i18n::t("No cutting moves found in this file.")).color(crate::theme::ORANGE));
            } else {
                hint(ui, i18n::t("Each tool in the file becomes an imported toolpath you can simulate, check and post for any machine. Pick the bit each one used: GCarve shows what the file says about it."));
            }
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(ctx.input(|i| i.content_rect().height()) - 260.0).show(ui, |ui| {
                for (k, s) in self.prog.sections.iter().enumerate() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&s.name).family(crate::widgets::bold_family()));
                            if s.tnum > 0 {
                                ui.label(RichText::new(format!("T{}", s.tnum)).color(crate::theme::TEXT_DIM));
                            }
                        });
                        let depth = (-s.min_z).max(0.0);
                        ui.label(
                            RichText::new(format!("{:.3} in deep · {:.0} in/min · plunge {:.0} · {:.0} RPM · {:.1} min", depth, s.feed_xy, s.feed_z, s.rpm, s.minutes))
                                .small()
                                .color(crate::theme::TEXT_DIM),
                        );
                        ui.horizontal_wrapped(|ui| {
                            ui.label(i18n::t("File says:"));
                            ui.label(RichText::new(if s.tool_text.is_empty() { i18n::t("(no tool comment)").to_string() } else { format!("“{}”", s.tool_text) }).monospace());
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label(i18n::t("Reads as:"));
                            ui.label(RichText::new(guess_text(&s.guess)).family(crate::widgets::bold_family()));
                            if !s.guess.source.is_empty() {
                                ui.label(RichText::new(format!("from the {}", s.guess.source)).small().color(crate::theme::TEXT_DIM));
                            }
                        });
                        for h in &s.hints {
                            ui.label(RichText::new(h).small().color(crate::theme::ORANGE));
                        }
                        ui.horizontal(|ui| {
                            ui.label(i18n::t("Bit"));
                            let mut items: Vec<(Bit, String)> = vec![(Bit::New, format!("New bit from the file: {}", gcarve_core::gimport::tool_from_section(s, "").name))];
                            items.extend(doc.tools.iter().enumerate().map(|(i, t)| (Bit::Doc(i), format!("Project: {}", t.name))));
                            items.extend(self.library.iter().enumerate().map(|(i, t)| (Bit::Lib(i), format!("Library: {}", t.name))));
                            let refs: Vec<(Bit, &str)> = items.iter().map(|(b, t)| (*b, t.as_str())).collect();
                            combo(ui, &format!("bit{k}"), &mut self.bits[k], &refs, 480.0);
                        });
                    });
                    ui.add_space(4.0);
                }
            });
            ui.add_space(6.0);
            if let Some((w, h, t)) = self.prog.board {
                crate::widgets::checkbox(ui, &mut self.set_board, format!("Set the board to {w} × {h} × {t} in (from the file)"));
            }
            crate::widgets::checkbox(ui, &mut self.vectors, i18n::t("Bring in the cut paths as vectors (centrelines, one layer per toolpath)"));
            if !self.prog.errors.is_empty() {
                ui.label(RichText::new(format!("{} line(s) GCarve can't read will be skipped, first: {}", self.prog.errors.len(), self.prog.errors[0])).small().color(crate::theme::ORANGE));
            }
            hint(ui, i18n::t("Positions are read relative to the board's XY origin and Z zero (Board settings)."));
            let (o, c) = crate::widgets::ok_cancel_text(ui, i18n::t("Import"));
            ok = o && n > 0;
            keep = !c;
        });
        if ok {
            let doc = &mut host.canvas.doc;
            doc.checkpoint();
            let mut lib_index: Vec<(usize, usize)> = Vec::new(); // library bit -> its index once in the project
            let chosen: Vec<Option<usize>> = self
                .bits
                .iter()
                .map(|b| match *b {
                    Bit::New => None,
                    Bit::Doc(i) => Some(i),
                    Bit::Lib(i) => Some(match lib_index.iter().find(|(l, _)| *l == i) {
                        Some(&(_, d)) => d,
                        None => {
                            let d = gcarve_core::document::tool_index_for(&mut doc.tools, &self.library[i].name, &self.library) as usize;
                            lib_index.push((i, d));
                            d
                        }
                    }),
                })
                .collect();
            // remember picked bits (not "new from the file") for the next file with the same tool comment
            let mut mem = memory();
            for (s, b) in self.prog.sections.iter().zip(&chosen) {
                if let Some(t) = b.and_then(|i| doc.tools.get(i)) {
                    mem.insert(memory_key(s), t.name.clone());
                }
            }
            crate::store::save(MEMORY_KEY, &serde_json::to_string(&mem).unwrap_or_default());
            *host.message = apply_import(doc, &self.text, &self.stem, &chosen, self.set_board, self.vectors);
            host.canvas.mutated();
            host.canvas.view = None;
            return false;
        }
        keep
    }
}
