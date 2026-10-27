//! Bit catalog browser (Tool Library ▸ Browse bit catalog…): every bit in the database with a drawn icon of its
//! type, filters by type / maker / size, and a detail panel with the maker's specs and chip loads.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::catalog::{get, status};
use crate::theme;
use eframe::egui::{self, pos2, vec2, Color32, Painter, Pos2, Rect, RichText, Shape, Stroke};
use gcarve_core::catalog::Bit;
use gcarve_core::document::{Tool, ToolType};

/// Type filter chips: label, icon family, families it covers
const TYPES: &[(&str, &str, &[&str])] = &[
    ("Up-cut", "end-mill-up", &["end-mill-up"]),
    ("Down-cut", "end-mill-down", &["end-mill-down"]),
    ("Compression", "compression", &["compression", "mortise-compression"]),
    ("Straight", "straight", &["straight"]),
    ("Roughing", "chipbreaker", &["chipbreaker"]),
    ("O-flute", "o-flute", &["o-flute"]),
    ("Ball nose", "ball-nose", &["ball-nose"]),
    ("Tapered ball", "tapered-ball", &["tapered-ball"]),
    ("V-bit", "v-bit", &["v-bit", "chamfer"]),
    ("Engraving", "engraving", &["engraving"]),
    ("Surfacing", "surfacing", &["surfacing"]),
    ("Drill", "drill", &["drill"]),
    ("Dovetail", "dovetail", &["dovetail"]),
    ("Profile", "roundover", &["roundover", "cove", "ogee", "bowl"]),
    ("T-slot", "t-slot", &["t-slot", "keyhole"]),
    ("Other", "other", &["other", "thread-mill"]),
];

/// Diameter filter (inches): label, min, max
const SIZES: &[(&str, f64, f64)] = &[("Any size", 0.0, 1e9), ("Under 1/8\"", 0.0, 0.124), ("1/8\" – 1/4\"", 0.124, 0.251), ("1/4\" – 1/2\"", 0.251, 0.501), ("Over 1/2\"", 0.501, 1e9)];

#[derive(Clone, PartialEq, Default)]
struct Filter {
    query: String,
    kind: Option<usize>,
    maker: String,
    size: usize,
    feeds_only: bool,
}

#[derive(Default)]
pub struct BitBrowser {
    pub open: bool,
    filter: Filter,
    cache: Option<(Filter, Vec<usize>)>,
    makers: Vec<(String, usize)>,
    selected: Option<usize>,
    added: Vec<usize>,
}

const MAKERS_KEY: &str = "bit-makers";

/// Makers the user added bits from, newest first (kept on the device): listed first in the maker filter
fn my_makers() -> Vec<String> {
    crate::store::load(MAKERS_KEY).map(|t| t.lines().filter(|l| !l.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

impl BitBrowser {
    fn sort_makers(&mut self) {
        let mine = my_makers();
        self.makers.sort_by(|a, b| {
            let rank = |m: &str| mine.iter().position(|x| x == m).unwrap_or(usize::MAX);
            rank(&a.0).cmp(&rank(&b.0)).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
        });
    }
}

fn inches(b: &Bit, v: f64) -> f64 {
    if b.units == "mm" { v / 25.4 } else { v }
}

fn size_text(b: &Bit, v: f64) -> String {
    if b.units == "mm" { format!("{} mm", trim(v)) } else { format!("{}\"", trim(v)) }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The icon family of a library tool (no catalog family on it)
pub fn tool_family(t: &Tool) -> &'static str {
    match t.kind {
        ToolType::VBit => "v-bit",
        ToolType::BallNose => "ball-nose",
        ToolType::RoundOver => "roundover",
        _ => "end-mill-up",
    }
}

impl BitBrowser {
    fn hits(&mut self) -> &[usize] {
        if self.cache.as_ref().is_none_or(|(f, _)| *f != self.filter) {
            let f = &self.filter;
            let words: Vec<String> = f.query.split_whitespace().map(|w| w.to_ascii_lowercase()).collect();
            let fams = f.kind.map(|k| TYPES[k].2);
            let (lo, hi) = (SIZES[f.size].1, SIZES[f.size].2);
            let cat = get();
            let hits = cat
                .bits
                .iter()
                .enumerate()
                .filter(|(_, b)| fams.is_none_or(|fs| fs.contains(&b.family.as_str())))
                .filter(|(_, b)| f.maker.is_empty() || b.maker == f.maker)
                .filter(|(_, b)| (lo..=hi).contains(&inches(b, b.diameter)))
                .filter(|(_, b)| !f.feeds_only || !cat.chip_loads(b).is_empty())
                .filter(|(_, b)| {
                    words.is_empty() || {
                        let hay = format!("{} {} {} {} {}", b.maker, b.part_number, b.name, b.family, b.id).to_ascii_lowercase();
                        words.iter().all(|w| hay.contains(w.as_str()))
                    }
                })
                .map(|(i, _)| i)
                .collect();
            self.cache = Some((self.filter.clone(), hits));
        }
        &self.cache.as_ref().unwrap().1
    }

    /// dev screenshots (GCARVE_DIALOG=bits + GCARVE_SEARCH): filter and show the first hit's details
    pub fn preset(&mut self, query: &str) {
        self.filter.query = query.into();
        if status().is_none() {
            self.selected = self.hits().first().copied();
        }
    }

    /// A bit the user added (dialog stays open so several can be added)
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Tool> {
        if !self.open {
            return None;
        }
        let mut add = None;
        let mut open = true;
        crate::widgets::Dialog::new(i18n::t("Bit catalog")).id("bit-browser").open(&mut open).close_footer().width(940.0).show(ctx, |ui| {
            if let Some(s) = status() {
                ui.label(RichText::new(s).color(theme::TEXT_DIM));
                return;
            }
            let cat = get();
            if self.makers.is_empty() {
                let mut m: std::collections::BTreeMap<&str, usize> = Default::default();
                for b in &cat.bits {
                    *m.entry(b.maker.as_str()).or_default() += 1;
                }
                self.makers = m.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
                self.sort_makers();
            }
            // search + maker + size + feeds
            ui.horizontal(|ui| {
                let r = ui.add(crate::widgets::field(&mut self.filter.query).hint_text(i18n::t("Search: part number, name, size (1/4 down)")).desired_width(300.0));
                crate::widgets::focus_first(ui, &r);
                let maker_label = if self.filter.maker.is_empty() { i18n::t("All makers").to_string() } else { self.filter.maker.clone() };
                egui::ComboBox::from_id_salt("bb-maker").selected_text(maker_label).width(170.0).height(400.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.filter.maker, String::new(), i18n::t("All makers"));
                    let mine = my_makers();
                    for (i, (m, n)) in self.makers.iter().enumerate() {
                        if i > 0 && mine.contains(&self.makers[i - 1].0) && !mine.contains(m) {
                            ui.separator(); // above: the makers you added bits from, newest first
                        }
                        ui.selectable_value(&mut self.filter.maker, m.clone(), format!("{m}  ({n})"));
                    }
                });
                egui::ComboBox::from_id_salt("bb-size").selected_text(SIZES[self.filter.size].0).width(110.0).show_ui(ui, |ui| {
                    for (i, s) in SIZES.iter().enumerate() {
                        ui.selectable_value(&mut self.filter.size, i, s.0);
                    }
                });
                crate::widgets::checkbox(ui, &mut self.filter.feeds_only, i18n::t("With maker feeds"));
            });
            ui.add_space(4.0);
            // type chips with icons
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for (k, (label, fam, _)) in TYPES.iter().enumerate() {
                    let on = self.filter.kind == Some(k);
                    let tw = ui.painter().layout_no_wrap(label.to_string(), theme::font(11.0), theme::TEXT).size().x;
                    let (r, resp) = ui.allocate_exact_size(vec2(tw + 30.0, 30.0), egui::Sense::click());
                    let fill = if on { theme::ACCENT } else if resp.hovered() { theme::ROW_HOVER } else { theme::FIELD };
                    ui.painter().rect_filled(r, 6.0, fill);
                    bit_icon(ui.painter(), Rect::from_min_size(r.left_top() + vec2(5.0, 3.0), vec2(16.0, 24.0)), fam, None);
                    ui.painter().text(pos2(r.left() + 25.0, r.center().y), egui::Align2::LEFT_CENTER, *label, theme::font(11.0), if on { Color32::WHITE } else { theme::TEXT });
                    if resp.clicked() {
                        self.filter.kind = if on { None } else { Some(k) };
                    }
                }
            });
            ui.add_space(6.0);
            let hits = self.hits().to_vec();
            ui.horizontal_top(|ui| {
                // results
                ui.vertical(|ui| {
                    ui.set_width(560.0);
                    ui.label(RichText::new(format!("{} bit(s)", hits.len())).small().color(theme::TEXT_DIM));
                    let list_h = crate::widgets::list_height(ui, 150.0); // search row, two rows of type chips, the count
                    egui::ScrollArea::vertical().id_salt("bb-list").max_height(list_h).min_scrolled_height(list_h).show_rows(ui, 44.0, hits.len(), |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for &i in &hits[range] {
                            let b = &cat.bits[i];
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());
                            let on = self.selected == Some(i);
                            let p = ui.painter();
                            if on || resp.hovered() {
                                p.rect_filled(r, 4.0, if on { theme::ACTIVE } else { theme::ROW_HOVER });
                            }
                            bit_icon(p, Rect::from_min_size(r.left_top() + vec2(8.0, 4.0), vec2(22.0, 36.0)), &b.family, b.angle_deg);
                            let pc = ui.painter_at(Rect::from_min_max(r.left_top(), pos2(r.right() - 120.0, r.bottom())));
                            pc.text(pos2(r.left() + 40.0, r.top() + 6.0), egui::Align2::LEFT_TOP, format!("{}  {}", b.maker, b.part_number), theme::font(12.0), Color32::WHITE);
                            pc.text(pos2(r.left() + 40.0, r.top() + 24.0), egui::Align2::LEFT_TOP, &b.name, theme::font(11.0), theme::TEXT_DIM);
                            let spec = format!("{} · {}F", size_text(b, b.diameter), b.flutes);
                            p.text(pos2(r.right() - 10.0, r.top() + 8.0), egui::Align2::RIGHT_TOP, spec, theme::font(12.0), theme::TEXT);
                            if self.added.contains(&i) {
                                p.text(pos2(r.right() - 10.0, r.top() + 25.0), egui::Align2::RIGHT_TOP, i18n::t("✓ in library"), theme::font(10.0), theme::MODE_SIMULATE);
                            } else if b.chip_load.as_ref().is_some_and(|c| !c.is_empty()) || b.chipload_ref.is_some() {
                                p.text(pos2(r.right() - 10.0, r.top() + 25.0), egui::Align2::RIGHT_TOP, i18n::t("maker feeds"), theme::font(10.0), theme::MODE_TOOLPATHS);
                            }
                            if resp.double_clicked() {
                                add = Some(i);
                            } else if resp.clicked() {
                                self.selected = Some(i);
                            }
                        }
                    });
                });
                ui.separator();
                // detail
                ui.vertical(|ui| {
                    ui.set_width(320.0);
                    let Some(b) = self.selected.and_then(|i| cat.bits.get(i)) else {
                        ui.add_space(60.0);
                        ui.label(RichText::new(i18n::t("Pick a bit to see its details.\nDouble-click adds it to your library.")).color(theme::TEXT_DIM));
                        return;
                    };
                    ui.horizontal_top(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(70.0, 120.0), egui::Sense::hover());
                        bit_icon(ui.painter(), r, &b.family, b.angle_deg);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(format!("{} {}", b.maker, b.part_number)).size(15.0).family(crate::widgets::bold_family()).color(Color32::WHITE));
                            ui.add(egui::Label::new(RichText::new(&b.name).color(theme::TEXT)).wrap());
                            ui.label(RichText::new(TYPES.iter().find(|t| t.2.contains(&b.family.as_str())).map_or(b.family.as_str(), |t| t.0)).small().color(theme::TEXT_DIM));
                        });
                    });
                    ui.add_space(6.0);
                    egui::Grid::new("bb-spec").num_columns(2).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
                        let mut row = |k: &str, v: String| {
                            ui.label(RichText::new(k).color(theme::TEXT_DIM));
                            ui.label(v);
                            ui.end_row();
                        };
                        row(i18n::t("Cutting diameter"), size_text(b, b.diameter));
                        if let Some(s) = b.shank {
                            row(i18n::t("Shank"), size_text(b, s));
                        }
                        if let Some(c) = b.cut_length {
                            row(i18n::t("Cutting length"), size_text(b, c));
                        }
                        row(i18n::t("Flutes"), b.flutes.to_string());
                        if let Some(a) = b.angle_deg {
                            row("Angle", format!("{}°", trim(a)));
                        }
                        if let Some(r) = b.rpm.as_ref().filter(|r| !r.is_empty()) {
                            row("RPM", r.iter().map(|v| format!("{v:.0}")).collect::<Vec<_>>().join(" – "));
                        } else if let Some(m) = b.max_rpm {
                            row("Max RPM", format!("{m:.0}"));
                        }
                    });
                    let loads = cat.chip_loads(b);
                    if !loads.is_empty() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(i18n::t("Maker chip load (in/tooth)")).small().color(theme::MODE_TOOLPATHS));
                        egui::Grid::new("bb-loads").num_columns(2).spacing(vec2(16.0, 2.0)).show(ui, |ui| {
                            for (m, [lo, hi]) in &loads {
                                ui.label(RichText::new(m).small().color(theme::TEXT_DIM));
                                ui.label(RichText::new(format!("{lo:.4} – {hi:.4}")).small());
                                ui.end_row();
                            }
                        });
                    }
                    ui.add_space(10.0);
                    let i = self.selected.unwrap_or_default();
                    let done = self.added.contains(&i);
                    if ui.add_enabled(!done, egui::Button::new(if done { i18n::t("✓ In your library") } else { i18n::t("Add to library") }).fill(theme::ACCENT)).clicked() {
                        add = Some(i);
                    }
                    // the shop (and any affiliate tag) is set on the server: Admin › Bit shop link
                    let q: String = format!("{} {}", b.maker, b.part_number).bytes().map(|c| if c.is_ascii_alphanumeric() { (c as char).to_string() } else { format!("%{c:02X}") }).collect();
                    if ui.button(i18n::t("Buy this bit…")).on_hover_text(i18n::t("Find it at a shop (opens your browser). GCarve may earn a small commission, at no cost to you.")).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(format!("https://gcarve.app/api/buy?q={q}")));
                    }
                });
            });
        });
        if !open {
            self.open = false;
        }
        add.map(|i| {
            let mut mine = my_makers();
            let maker = get().bits[i].maker.clone();
            mine.retain(|m| *m != maker);
            mine.insert(0, maker);
            mine.truncate(6);
            crate::store::save(MAKERS_KEY, &mine.join("\n"));
            self.sort_makers();
            self.added.push(i);
            self.selected = Some(i);
            get().bit_tool(&get().bits[i])
        })
    }
}

/// A side view of a router bit of `family` in `r` (shank on top, cutter below), for lists and pickers.
pub fn bit_icon(p: &Painter, r: Rect, family: &str, angle: Option<f64>) {
    let steel = Color32::from_rgb(0xb8, 0xc4, 0xd6);
    let shank_c = Color32::from_rgb(0x7d, 0x84, 0x8f);
    let line = Stroke::new((r.width() / 20.0).max(1.0), Color32::from_rgb(0x55, 0x60, 0x70));
    let (cx, w, h) = (r.center().x, r.width(), r.height());
    let y0 = r.top() + h * 0.42; // shank / cutter split
    let y1 = r.bottom();
    let x = |f: f32| cx + w * f;
    let shank = |p: &Painter, half: f32, to: f32| p.rect_filled(Rect::from_min_max(pos2(cx - w * half, r.top()), pos2(cx + w * half, to)), 1.0, shank_c);
    let poly = |p: &Painter, pts: Vec<Pos2>| p.add(Shape::convex_polygon(pts, steel, Stroke::NONE));
    let cw = 0.22;
    let body = |p: &Painter, top: f32, bot: f32| p.rect_filled(Rect::from_min_max(pos2(x(-cw), top), pos2(x(cw), bot)), 1.0, steel);
    // diagonal flute lines across the body between `top` and `bot`; slope > 0 = up-cut look
    let flutes = |p: &Painter, top: f32, bot: f32, slope: f32| {
        let n = 3;
        let span = (bot - top) / n as f32;
        for k in 0..n {
            let yb = top + span * (k as f32 + 0.8);
            p.line_segment([pos2(x(-cw), yb), pos2(x(cw), yb - span * 0.6 * slope)], line);
        }
    };
    match family {
        "ball-nose" => {
            shank(p, 0.17, y0);
            let rr = w * cw;
            body(p, y0, y1 - rr);
            p.circle_filled(pos2(cx, y1 - rr), rr, steel);
            flutes(p, y0, y1 - rr, 1.0);
        }
        "tapered-ball" => {
            shank(p, 0.17, y0);
            let tip = w * 0.07;
            poly(p, vec![pos2(x(-cw), y0), pos2(x(cw), y0), pos2(cx + tip, y1 - tip), pos2(cx - tip, y1 - tip)]);
            p.circle_filled(pos2(cx, y1 - tip), tip, steel);
        }
        "v-bit" | "chamfer" | "engraving" => {
            shank(p, 0.17, y0);
            // cone from the shank to the tip; its width follows the angle
            let a = angle.unwrap_or(if family == "engraving" { 30.0 } else { 90.0 }).clamp(10.0, 160.0);
            let half = ((y1 - y0) * (a.to_radians() as f32 / 2.0).tan()).clamp(w * 0.2, w * 0.48);
            let flat = if family == "engraving" { w * 0.05 } else { 0.0 };
            poly(p, vec![pos2(cx - half, y0), pos2(cx + half, y0), pos2(cx + flat, y1), pos2(cx - flat, y1)]);
            p.line_segment([pos2(cx, y0), pos2(cx, y1 - 1.0)], line);
        }
        "surfacing" => {
            shank(p, 0.15, y1 - h * 0.2);
            p.rect_filled(Rect::from_min_max(pos2(x(-0.48), y1 - h * 0.2), pos2(x(0.48), y1)), 2.0, steel);
            p.line_segment([pos2(x(-0.3), y1 - h * 0.1), pos2(x(0.3), y1 - h * 0.1)], line);
        }
        "drill" => {
            shank(p, 0.17, y0);
            let tip = w * cw * 0.6;
            body(p, y0, y1 - tip);
            poly(p, vec![pos2(x(-cw), y1 - tip), pos2(x(cw), y1 - tip), pos2(cx, y1)]);
            flutes(p, y0, y1 - tip, 1.4);
        }
        "dovetail" => {
            shank(p, 0.17, y0);
            poly(p, vec![pos2(x(-0.12), y0), pos2(x(0.12), y0), pos2(x(0.45), y1), pos2(x(-0.45), y1)]);
        }
        "t-slot" | "keyhole" => {
            shank(p, 0.17, y0);
            p.rect_filled(Rect::from_min_max(pos2(x(-0.09), y0), pos2(x(0.09), y1 - h * 0.18)), 0.0, steel);
            p.rect_filled(Rect::from_min_max(pos2(x(-0.45), y1 - h * 0.18), pos2(x(0.45), y1)), 1.0, steel);
        }
        "roundover" | "cove" | "ogee" | "bowl" => {
            // profile bit: wide cutter with the profile cut into its lower corners + bearing
            shank(p, 0.17, y0);
            let (top, bot) = (y0, y1 - h * 0.1);
            let half = w * 0.45;
            let rad = (bot - top) * 0.7;
            match family {
                "bowl" => {
                    p.rect_filled(Rect::from_min_max(pos2(cx - half, top), pos2(cx + half, bot)), egui::CornerRadius { nw: 0, ne: 0, sw: rad as u8, se: rad as u8 }, steel);
                }
                _ => {
                    p.rect_filled(Rect::from_min_max(pos2(cx - half, top), pos2(cx + half, bot)), 1.0, steel);
                    // the cutting profile, drawn as an inset line on each side
                    for s in [-1.0f32, 1.0] {
                        let pts: Vec<Pos2> = (0..=8)
                            .map(|k| {
                                let t = k as f32 / 8.0 * std::f32::consts::FRAC_PI_2;
                                let (dx, dy) = if family == "cove" { (t.sin(), 1.0 - t.cos()) } else { (1.0 - t.cos(), t.sin()) };
                                pos2(cx + s * (half - rad * 0.6 * dx), bot - rad * dy)
                            })
                            .collect();
                        p.add(Shape::line(pts, line));
                    }
                }
            }
            p.circle_filled(pos2(cx, y1 - h * 0.05), w * 0.11, shank_c);
        }
        _ => {
            // straight-sided end mills, with flutes that show the type
            shank(p, 0.17, y0);
            body(p, y0, y1);
            match family {
                "end-mill-down" => flutes(p, y0, y1, -1.0),
                "compression" | "mortise-compression" => {
                    let mid = y1 - (y1 - y0) * if family == "compression" { 0.35 } else { 0.15 };
                    flutes(p, y0, mid, -1.0);
                    flutes(p, mid, y1, 1.0);
                    p.line_segment([pos2(x(-cw), mid), pos2(x(cw), mid)], line);
                }
                "straight" => {
                    for f in [-0.08, 0.08] {
                        p.line_segment([pos2(x(f), y0 + 2.0), pos2(x(f), y1 - 2.0)], line);
                    }
                }
                "o-flute" => {
                    p.line_segment([pos2(x(-cw), y1 - 2.0), pos2(x(cw), y0 + 2.0)], Stroke::new(line.width * 2.2, line.color));
                }
                "chipbreaker" => {
                    flutes(p, y0, y1, 1.0);
                    for k in 1..4 {
                        let yy = y0 + (y1 - y0) * k as f32 / 4.0;
                        p.line_segment([pos2(x(-cw), yy), pos2(x(-cw * 0.4), yy)], line);
                    }
                }
                "other" | "thread-mill" => {
                    for k in 1..5 {
                        let yy = y0 + (y1 - y0) * k as f32 / 5.0;
                        p.line_segment([pos2(x(-cw), yy), pos2(x(cw), yy)], line);
                    }
                }
                _ => flutes(p, y0, y1, 1.0),
            }
        }
    }
}
