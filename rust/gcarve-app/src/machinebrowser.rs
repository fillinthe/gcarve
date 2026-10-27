//! Machine browser (Job Setup ▸ Choose machine…): every machine in the database, filtered by maker, kind, size and
//! tool changer; the detail panel picks the spindle and controller (= the post) and applies the machine.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::catalog::{get, status};
use crate::theme;
use eframe::egui::{self, pos2, vec2, Color32, RichText};
use gcarve_core::catalog::Machine;
use gcarve_core::document::MachineLimits;

const CLASSES: &[(&str, &str)] = &[("desktop", "Desktop"), ("hobby", "Hobby"), ("prosumer", "Prosumer"), ("industrial", "Industrial")];

/// Work area filter: label, smallest short side, smallest long side (inches)
const SIZES: &[(&str, f64, f64)] = &[("Any size", 0.0, 0.0), ("Up to 2 × 2 ft", -1.0, -1.0), ("2 × 2 ft+", 22.0, 22.0), ("4 × 4 ft+", 46.0, 46.0), ("4 × 8 ft+", 46.0, 94.0), ("5 × 10 ft+", 58.0, 118.0)];

#[derive(Clone, PartialEq, Default)]
struct Filter {
    query: String,
    maker: String,
    class: Option<usize>,
    size: usize,
    atc: bool,
}

#[derive(Default)]
pub struct MachineBrowser {
    pub open: bool,
    filter: Filter,
    cache: Option<(Filter, Vec<usize>)>,
    makers: Vec<(String, usize)>,
    selected: Option<usize>,
    spindle: usize,
    controller: usize,
}

/// (short, long) side of the work area in inches, 0 when not published
fn sides(m: &Machine) -> (f64, f64) {
    let (x, y) = (m.work_area_in.x.unwrap_or(0.0), m.work_area_in.y.unwrap_or(0.0));
    (x.min(y), x.max(y))
}

fn size_text(m: &Machine) -> String {
    let w = &m.work_area_in;
    match (w.x, w.y) {
        (Some(x), Some(y)) => format!("{} × {} in", trim(x), trim(y)),
        _ => i18n::t("size not published").into(),
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.1}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn spindle_text(s: &gcarve_core::catalog::Spindle) -> String {
    let rpm = match (s.rpm_min, s.rpm_max) {
        (Some(a), Some(b)) => format!(" · {a:.0}–{b:.0} RPM"),
        (None, Some(b)) => format!(" · max {b:.0} RPM"),
        _ => String::new(),
    };
    let kw = s.power_kw.map(|k| format!(" · {} kW", trim(k))).unwrap_or_default();
    format!("{}{kw}{rpm}", if s.name.is_empty() { i18n::t("Spindle") } else { &s.name })
}

fn post_name(id: &str) -> String {
    gcarve_core::postdef::post_library().iter().find(|p| p.id == id).map_or_else(|| id.to_string(), |p| p.name.clone())
}

impl MachineBrowser {
    pub fn show(&mut self, current: Option<&MachineLimits>) {
        self.open = true;
        // start on the machine in use
        if let (Some(cur), None) = (current, status()) {
            self.selected = get().machines.iter().position(|m| m.id == cur.id);
            if let Some(m) = self.selected.and_then(|i| get().machines.get(i)) {
                self.spindle = m.spindles.iter().position(|s| s.name == cur.spindle).unwrap_or(m.default_spindle());
                self.controller = m.controllers.iter().position(|c| c.default == Some(true)).unwrap_or(0);
            }
        }
    }

    /// dev screenshots (GCARVE_DIALOG=machines + GCARVE_SEARCH): filter and show the first hit
    pub fn preset(&mut self, query: &str) {
        self.filter.query = query.into();
        if status().is_none() {
            self.selected = self.hits().first().copied();
            if let Some(m) = self.selected.and_then(|i| get().machines.get(i)) {
                self.spindle = m.default_spindle();
            }
        }
    }

    fn hits(&mut self) -> &[usize] {
        if self.cache.as_ref().is_none_or(|(f, _)| *f != self.filter) {
            let f = &self.filter;
            let words: Vec<String> = f.query.split_whitespace().map(|w| w.to_ascii_lowercase()).collect();
            let (min_s, min_l) = (SIZES[f.size].1, SIZES[f.size].2);
            let hits = get()
                .machines
                .iter()
                .enumerate()
                .filter(|(_, m)| f.maker.is_empty() || m.maker == f.maker)
                .filter(|(_, m)| f.class.is_none_or(|c| m.class == CLASSES[c].0))
                .filter(|(_, m)| !f.atc || m.tool_change.kind.starts_with("atc"))
                .filter(|(_, m)| {
                    let (s, l) = sides(m);
                    match min_s {
                        x if x < 0.0 => l > 0.0 && l <= 26.0,
                        x if x > 0.0 => s >= min_s && l >= min_l,
                        _ => true,
                    }
                })
                .filter(|(_, m)| {
                    words.is_empty() || {
                        let hay = format!("{} {} {}", m.title(), m.id, m.controllers.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(" ")).to_ascii_lowercase();
                        words.iter().all(|w| hay.contains(w.as_str()))
                    }
                })
                .map(|(i, _)| i)
                .collect();
            self.cache = Some((self.filter.clone(), hits));
        }
        &self.cache.as_ref().unwrap().1
    }

    /// Some((limits, post)) when the user picks a machine
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<(MachineLimits, Option<String>)> {
        if !self.open {
            return None;
        }
        let mut pick = None;
        let mut open = true;
        crate::widgets::Dialog::new(i18n::t("Choose your machine")).id("machine-browser").open(&mut open).close_footer().width(960.0).show(ctx, |ui| {
            if let Some(s) = status() {
                ui.label(RichText::new(s).color(theme::TEXT_DIM));
                return;
            }
            let cat = get();
            if self.makers.is_empty() {
                let mut m: std::collections::BTreeMap<&str, usize> = Default::default();
                for x in &cat.machines {
                    *m.entry(x.maker.as_str()).or_default() += 1;
                }
                self.makers = m.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            }
            ui.horizontal(|ui| {
                let r = ui.add(crate::widgets::field(&mut self.filter.query).hint_text(i18n::t("Search: Shapeoko, Panther 5x10, ShopBot PRS…")).desired_width(280.0));
                crate::widgets::focus_first(ui, &r);
                let maker_label = if self.filter.maker.is_empty() { i18n::t("All makers").to_string() } else { self.filter.maker.clone() };
                egui::ComboBox::from_id_salt("mb-maker").selected_text(maker_label).width(180.0).height(420.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.filter.maker, String::new(), i18n::t("All makers"));
                    for (m, n) in &self.makers {
                        ui.selectable_value(&mut self.filter.maker, m.clone(), format!("{m}  ({n})"));
                    }
                });
                egui::ComboBox::from_id_salt("mb-size").selected_text(SIZES[self.filter.size].0).width(120.0).show_ui(ui, |ui| {
                    for (i, s) in SIZES.iter().enumerate() {
                        ui.selectable_value(&mut self.filter.size, i, s.0);
                    }
                });
                crate::widgets::checkbox(ui, &mut self.filter.atc, i18n::t("Tool changer"));
            });
            ui.horizontal(|ui| {
                for (i, (_, label)) in CLASSES.iter().enumerate() {
                    let on = self.filter.class == Some(i);
                    if ui.selectable_label(on, *label).clicked() {
                        self.filter.class = if on { None } else { Some(i) };
                    }
                }
            });
            ui.add_space(4.0);
            let hits = self.hits().to_vec();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(560.0);
                    ui.label(RichText::new(format!("{} machine(s)", hits.len())).small().color(theme::TEXT_DIM));
                    let list_h = crate::widgets::list_height(ui, 112.0); // search row, class chips, the count
                    egui::ScrollArea::vertical().id_salt("mb-list").max_height(list_h).min_scrolled_height(list_h).show_rows(ui, 44.0, hits.len(), |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for &i in &hits[range] {
                            let m = &cat.machines[i];
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());
                            let on = self.selected == Some(i);
                            let p = ui.painter();
                            if on || resp.hovered() {
                                p.rect_filled(r, 4.0, if on { theme::ACTIVE } else { theme::ROW_HOVER });
                            }
                            let pc = ui.painter_at(egui::Rect::from_min_max(r.left_top(), pos2(r.right() - 110.0, r.bottom())));
                            pc.text(pos2(r.left() + 10.0, r.top() + 6.0), egui::Align2::LEFT_TOP, m.title(), theme::font(12.0), Color32::WHITE);
                            let sp = m.spindles.get(m.default_spindle()).map(spindle_text).unwrap_or_default();
                            let ctl = m.controllers.iter().find(|c| c.default == Some(true)).or(m.controllers.first()).map(|c| c.name.clone()).unwrap_or_default();
                            let sub = [size_text(m), sp, ctl].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                            pc.text(pos2(r.left() + 10.0, r.top() + 24.0), egui::Align2::LEFT_TOP, sub, theme::font(11.0), theme::TEXT_DIM);
                            let cls = CLASSES.iter().find(|c| c.0 == m.class).map_or("", |c| c.1);
                            p.text(pos2(r.right() - 10.0, r.top() + 8.0), egui::Align2::RIGHT_TOP, cls, theme::font(11.0), theme::TEXT);
                            if m.tool_change.kind.starts_with("atc") {
                                let slots = m.tool_change.slots.map(|n| format!("ATC · {n}")).unwrap_or_else(|| "ATC".into());
                                p.text(pos2(r.right() - 10.0, r.top() + 25.0), egui::Align2::RIGHT_TOP, slots, theme::font(10.0), theme::MODE_TOOLPATHS);
                            }
                            if resp.clicked() || resp.double_clicked() {
                                if self.selected != Some(i) {
                                    self.spindle = m.default_spindle();
                                    self.controller = m.controllers.iter().position(|c| c.default == Some(true)).unwrap_or(0);
                                }
                                self.selected = Some(i);
                            }
                            if resp.double_clicked() {
                                pick = Some(self.choice(m));
                            }
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_width(360.0);
                    let Some(m) = self.selected.and_then(|i| cat.machines.get(i)) else {
                        ui.add_space(60.0);
                        ui.label(RichText::new(i18n::t("Pick your machine to see its details.\nIts spindle and feed limits keep suggested feeds safe, and its controller sets the post-processor.")).color(theme::TEXT_DIM));
                        return;
                    };
                    ui.label(RichText::new(m.title()).size(15.0).family(crate::widgets::bold_family()).color(Color32::WHITE));
                    ui.label(RichText::new(CLASSES.iter().find(|c| c.0 == m.class).map_or(m.class.as_str(), |c| c.1)).small().color(theme::TEXT_DIM));
                    ui.add_space(6.0);
                    egui::Grid::new("mb-spec").num_columns(2).spacing(vec2(14.0, 4.0)).show(ui, |ui| {
                        ui.label(RichText::new(i18n::t("Work area")).color(theme::TEXT_DIM));
                        let z = m.work_area_in.z.map(|z| format!(" · Z {} in", trim(z))).unwrap_or_default();
                        ui.label(format!("{}{z}", size_text(m)));
                        ui.end_row();
                        if let Some(f) = m.max_feed_ipm.xy.or(m.rapid_ipm.xy) {
                            ui.label(RichText::new(i18n::t("Max feed")).color(theme::TEXT_DIM));
                            ui.label(format!("{f:.0} in/min"));
                            ui.end_row();
                        }
                        ui.label(RichText::new(i18n::t("Tool changes")).color(theme::TEXT_DIM));
                        ui.label(if m.tool_change.kind.starts_with("atc") { m.tool_change.slots.map_or(i18n::t("Automatic (ATC)").into(), |n| format!("Automatic, {n} slots")) } else { i18n::t("Manual").into() });
                        ui.end_row();
                    });
                    if m.spindles.len() > 1 {
                        ui.add_space(6.0);
                        ui.label(RichText::new(i18n::t("Spindle")).family(crate::widgets::bold_family()));
                        for (k, s) in m.spindles.iter().enumerate() {
                            ui.radio_value(&mut self.spindle, k, spindle_text(s));
                        }
                    } else if let Some(s) = m.spindles.first() {
                        ui.add_space(4.0);
                        ui.label(format!("Spindle: {}", spindle_text(s)));
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new(i18n::t("Controller")).family(crate::widgets::bold_family()));
                    if m.controllers.is_empty() {
                        ui.label(RichText::new(i18n::t("Not published: the post stays as it is (choose one in Job Setup).")).small().color(theme::TEXT_DIM));
                    }
                    for (k, c) in m.controllers.iter().enumerate() {
                        let post = c.gcarve_post.as_deref().map(post_name).unwrap_or_else(|| i18n::t("no built-in post: keeps yours").into());
                        ui.radio_value(&mut self.controller, k, format!("{} → {post}", c.name));
                    }
                    if let Some(src) = m.sources.first() {
                        ui.add_space(4.0);
                        ui.hyperlink_to(RichText::new(i18n::t("Maker's specs")).small(), src);
                    }
                    ui.add_space(10.0);
                    if ui.add(egui::Button::new(RichText::new(i18n::t("Use this machine")).color(Color32::WHITE)).fill(theme::ACCENT)).clicked() {
                        pick = Some(self.choice(m));
                    }
                });
            });
        });
        if !open || pick.is_some() {
            self.open = false;
        }
        pick
    }

    fn choice(&self, m: &Machine) -> (MachineLimits, Option<String>) {
        let post = m.controllers.get(self.controller).and_then(|c| c.gcarve_post.clone()).or_else(|| m.post().map(str::to_string));
        (m.limits(self.spindle.min(m.spindles.len().saturating_sub(1))), post)
    }
}
