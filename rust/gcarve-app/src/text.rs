//! Editable text (Qt app: canvas_vectors.cpp buildText/placeText + TextDialog.qml). Glyph outlines come from the
//! bundled Liberation Sans (assets/, SIL OFL: see LiberationSans-LICENSE.txt) through ttf-parser, so the result is
//! identical on every platform and in the browser. Italic is a synthetic 12 degree shear.

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use crate::canvas::{Canvas, Tool};
use crate::units;
use eframe::egui;
use gcarve_core::document::TextItem;
use gcarve_core::geom::{bounds, dist, flatten, lerp, pt, transformed, Affine, Contour, Pt, Rect};
use ttf_parser::Face;

use gcarve_ui::glyphs::{bundled, line_glyphs, Glyph};

/// Font families offered in the dialog (one file each; the family name is kept in the project for the Qt app).
pub const FAMILIES: [&str; 1] = ["Liberation Sans"];

/// Fonts the user added (family name, file bytes kept for the life of the program: a Face borrows them). Saved as
/// `fonts.json` {family: base64} in the store, so they are back at the next start; a project only keeps the family
/// name, an unknown family falls back to Liberation Sans.
type FontList = std::sync::Mutex<Vec<(String, &'static [u8])>>;

fn custom() -> &'static FontList {
    static FONTS: std::sync::OnceLock<FontList> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let saved: std::collections::BTreeMap<String, String> = crate::store::load("fonts.json").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let list = saved.into_iter().map(|(n, b)| (n, &*Box::leak(gcarve_core::photo::b64_decode(&b).into_boxed_slice()))).filter(|(_, b)| Face::parse(b, 0).is_ok()).collect();
        std::sync::Mutex::new(list)
    })
}

/// The user's added font families
pub fn custom_families() -> Vec<String> {
    custom().lock().map(|l| l.iter().map(|(n, _)| n.clone()).collect()).unwrap_or_default()
}

/// Family just added (the open text dialog picks it up)
static ADDED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// A zipped font download (Google Fonts and most font sites) gives its regular style, else its first .ttf / .otf.
fn unzip_font(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if !bytes.starts_with(b"PK") {
        return Ok(bytes);
    }
    let files = gcarve_core::threemf::zip_entries(&bytes, &[".ttf", ".otf"])?;
    let mut names: Vec<&String> = files.keys().filter(|n| !n.contains("__MACOSX")).collect();
    names.sort_by_key(|n| (!n.to_ascii_lowercase().contains("regular"), n.len(), n.to_string()));
    names.first().map(|n| files[*n].clone()).ok_or_else(|| i18n::t("the zip holds no .ttf or .otf font").into())
}

/// Register a .ttf / .otf and keep it for next time; returns its family name.
pub fn add_font(bytes: Vec<u8>) -> Result<String, String> {
    let bytes = unzip_font(bytes)?;
    if bytes.len() > 3 << 20 {
        return Err(i18n::t("font file over 3 MB: pick a smaller one").into());
    }
    if bytes.starts_with(b"wOFF") || bytes.starts_with(b"wOF2") {
        return Err(i18n::t("a web font (.woff): download the .ttf or .otf version of it").into());
    }
    Face::parse(&bytes, 0).map_err(|_| i18n::t("not a font file (.ttf or .otf)").to_string())?;
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let face = Face::parse(leaked, 0).map_err(|_| i18n::t("not a font file (.ttf or .otf)").to_string())?;
    let name = |id: u16| face.names().into_iter().find(|n| n.name_id == id && n.is_unicode()).and_then(|n| n.to_string());
    let family = name(16).or_else(|| name(1)).or_else(|| name(4)).ok_or(i18n::t("the font has no name"))?;
    if face.glyph_index('A').is_none() {
        return Err(i18n::t("the font has no Latin letters").into());
    }
    let mut list = custom().lock().map_err(|_| i18n::t("fonts are busy").to_string())?;
    list.retain(|(n, _)| *n != family);
    list.push((family.clone(), leaked));
    #[cfg(not(test))] // tests must not write the user's font store
    {
        let saved: std::collections::BTreeMap<&str, String> = list.iter().map(|(n, b)| (n.as_str(), gcarve_core::photo::b64_encode(b))).collect();
        crate::store::save("fonts.json", &serde_json::to_string(&saved).unwrap_or_default());
    }
    *ADDED.lock().unwrap() = Some(family.clone());
    Ok(family)
}

fn face(family: &str, bold: bool) -> Face<'static> {
    let own = custom().lock().ok().and_then(|l| l.iter().find(|(n, _)| n == family).map(|(_, b)| *b));
    // a custom font has one weight: Bold means the family's own bold file, which isn't there
    own.and_then(|b| Face::parse(b, 0).ok()).unwrap_or_else(|| bundled(bold))
}

/// A TrueType face, or one of the single-line engraving fonts (open strokes, gcarve_ui::strokes)
enum Font {
    Ttf(Box<Face<'static>>),
    Stroke(String),
}

impl Font {
    fn of(family: &str, bold: bool) -> Font {
        if gcarve_ui::strokes::is_stroke(family) { Font::Stroke(family.into()) } else { Font::Ttf(Box::new(face(family, bold))) }
    }
    /// font units from the baseline to the top of the capitals
    fn cap(&self) -> f64 {
        match self {
            Font::Ttf(f) => f.capital_height().filter(|&c| c > 0).unwrap_or(f.ascender()) as f64,
            Font::Stroke(_) => gcarve_ui::strokes::CAP,
        }
    }
    fn line_h(&self) -> f64 {
        match self {
            Font::Ttf(f) => f.height() as f64 + f.line_gap() as f64,
            Font::Stroke(_) => gcarve_ui::strokes::LINE,
        }
    }
    /// one line's glyphs at the origin with their x offsets (font units)
    fn line(&self, text: &str, spacing_units: f64) -> Vec<(Glyph, f64)> {
        match self {
            Font::Ttf(f) => line_glyphs(f, text, spacing_units),
            Font::Stroke(fam) => {
                let mut x = 0.0;
                text.chars().map(|ch| { let g = gcarve_ui::strokes::glyph(fam, ch); let at = x; x += g.advance + spacing_units; (g, at) }).collect()
            }
        }
    }
    fn stroke(&self) -> bool {
        matches!(self, Font::Stroke(_))
    }
}

fn place(contours: &[Contour], m: &Affine, out: &mut Vec<Contour>) {
    out.extend(contours.iter().map(|c| transformed(c, m)));
}

/// Glyph outlines for a text item (inches). Block text is centred on `centre`; curved text follows `path`.
/// Returns the contours and a note (e.g. characters dropped past the end of an open curve).
pub fn build_text(t: &TextItem, centre: Pt, path: Option<&Contour>) -> (Vec<Contour>, String) {
    if path.is_none() && t.arch.abs() > 1e-6 && !t.text.trim().is_empty() && t.height > 0.0 {
        return arched(t, centre);
    }
    build(t, centre, path, None)
}

/// Arched block text: the first line laid along a circle of radius |arch| through `centre` (on its top when arch > 0,
/// its bottom when < 0), centred on that point, by the curved-text layout.
fn arched(t: &TextItem, centre: Pt) -> (Vec<Contour>, String) {
    let font = Font::of(&t.family, t.bold);
    let k = t.height / font.cap();
    let line = t.text.lines().next().unwrap_or("");
    let n = line.chars().count();
    let width = font.line(line, t.spacing / k).last().map_or(0.0, |(g, x)| (x + g.advance) * k) + (0..n.saturating_sub(1)).map(|i| t.kern(i)).sum::<f64>();
    let r = t.arch.abs().max(t.height);
    let up = t.arch > 0.0;
    // the arc runs left to right over the top (clockwise) or under the bottom (anticlockwise), half a turn each way
    let (c, mid) = if up { (pt(centre.x, centre.y - r), std::f64::consts::FRAC_PI_2) } else { (pt(centre.x, centre.y + r), -std::f64::consts::FRAC_PI_2) };
    let half = std::f64::consts::PI * 0.999;
    let n = 360;
    let pts: Vec<Pt> = (0..=n).map(|i| {
        let f = i as f64 / n as f64;
        let a = if up { mid + half - 2.0 * half * f } else { mid - half + 2.0 * half * f };
        pt(c.x + r * a.cos(), c.y + r * a.sin())
    }).collect();
    let arc = gcarve_core::geom::make_polyline(&pts, false);
    let along = TextItem { start_offset: r * half - width / 2.0, above: true, ..t.clone() };
    let (contours, mut note) = build(&along, centre, Some(&arc), None);
    if t.text.lines().count() > 1 {
        note = i18n::t("Arched text uses the first line only").into();
    }
    (contours, note)
}

/// Text found in an imported drawing (SVG `<text>`, DXF TEXT / MTEXT) becomes editable text in the default font,
/// standing where the file put it. Returns how many were placed.
pub fn import_texts(doc: &mut gcarve_core::document::Document, texts: &[gcarve_core::svg::ImportText]) -> usize {
    let mut n = 0;
    for it in texts {
        let t = TextItem { id: doc.next_text_id, text: it.text.clone(), height: it.height, align: it.anchor.clamp(0, 2), angle: if it.angle.is_finite() { it.angle } else { 0.0 }, ..Default::default() };
        let (contours, _) = build(&t, it.at, None, Some((it.anchor, it.valign)));
        if contours.is_empty() {
            continue;
        }
        doc.next_text_id += 1;
        let (layer, group) = (doc.layer_named(&it.layer), doc.new_group_id());
        for c in contours {
            let id = doc.add(c, Some(layer));
            let s = doc.find_mut(id).unwrap();
            s.text = t.id;
            s.group = group;
        }
        doc.texts.push(t);
        n += 1;
    }
    n
}

fn build(t: &TextItem, centre: Pt, path: Option<&Contour>, anchor: Option<(i32, i32)>) -> (Vec<Contour>, String) {
    let mut contours = Vec::new();
    if t.text.trim().is_empty() || t.height <= 0.0 {
        return (contours, String::new());
    }
    let font = Font::of(&t.family, t.bold);
    let cap = font.cap();
    let k = t.height / cap; // font units -> inches
    let italic = if t.italic { Affine { c: 0.2, ..Default::default() } } else { Affine::default() };
    let spacing_units = t.spacing / k;

    let Some(path) = path else {
        let line_h = font.line_h() * t.line_spacing;
        let mut raw = Vec::new();
        // kerning by hand: each character's own extra gap pushes everything after it on its line
        let mut first = 0; // char index of the line's first character in the whole text
        let lines: Vec<Vec<(Glyph, f64)>> = t
            .text
            .split('\n')
            .map(|l| {
                let l = l.strip_suffix('\r').unwrap_or(l);
                let mut extra = 0.0;
                let g = font.line(l, spacing_units).into_iter().enumerate().map(|(i, (g, x))| { let at = x + extra; extra += t.kern(first + i) / k; (g, at) }).collect();
                first += l.chars().count() + 1;
                g
            })
            .collect();
        let width = |l: &[(Glyph, f64)]| l.last().map_or(0.0, |(g, x)| x + g.advance);
        let widest = lines.iter().map(|l| width(l)).fold(0.0, f64::max);
        for (i, line) in lines.iter().enumerate() {
            let shift = (widest - width(line)) * f64::from(t.align.clamp(0, 2)) / 2.0; // 0 left, 1 centre, 2 right
            for (g, x) in line {
                place(&g.contours, &(Affine::translate(x + shift, -(i as f64) * line_h) * italic), &mut raw);
            }
        }
        let mut br = Rect::default();
        for c in &raw {
            br.add_rect(&bounds(c));
        }
        if br.is_empty() {
            return (contours, String::new());
        }
        // `anchor`: `centre` is not the middle but a point on the text (imported drawings): x 0 left / 1 centre /
        // 2 right, y 0 first baseline / 1 top of the capitals / 2 middle
        let (ox, oy) = match anchor {
            None => ((br.x0 + br.x1) / 2.0, (br.y0 + br.y1) / 2.0),
            Some((h, v)) => ([br.x0, (br.x0 + br.x1) / 2.0, br.x1][h.clamp(0, 2) as usize], [0.0, cap, (br.y0 + br.y1) / 2.0][v.clamp(0, 2) as usize]),
        };
        let m = Affine::translate(centre.x, centre.y) * Affine::rotate(t.angle.to_radians()) * Affine::translate(-ox * k, -oy * k) * Affine::scale(k, k);
        place(&raw, &m, &mut contours);
        if t.weld && !font.stroke() {
            contours = gcarve_core::vecops::weld_keep_holes(&contours);
        }
        return (contours, String::new());
    };

    // arc-length table of the path
    let mut pts = flatten(path, 0.0005);
    if pts.len() < 2 {
        return (contours, String::new());
    }
    if path.closed {
        // walk closed curves clockwise so the left normal points outward (letters stand on the outside)
        let mut area = 0.0;
        for i in 0..pts.len() {
            let j = (i + pts.len() - 1) % pts.len();
            area += pts[j].x * pts[i].y - pts[i].x * pts[j].y;
        }
        if area > 0.0 {
            pts.reverse();
        }
    }
    let mut cum = vec![0.0];
    for i in 1..pts.len() {
        cum.push(cum[i - 1] + dist(pts[i - 1], pts[i]));
    }
    let total = *cum.last().unwrap();
    let closed = path.closed;
    let at = |mut s: f64| -> (Pt, Pt) {
        if closed {
            s = s.rem_euclid(total);
        }
        let s = s.clamp(0.0, total);
        let i = cum.partition_point(|&c| c <= s).clamp(1, pts.len() - 1);
        let seg = cum[i] - cum[i - 1];
        let tt = if seg > 1e-12 { (s - cum[i - 1]) / seg } else { 0.0 };
        let d = pts[i] - pts[i - 1];
        let l = d.x.hypot(d.y);
        (lerp(pts[i - 1], pts[i], tt), if l > 1e-12 { d * (1.0 / l) } else { pt(1.0, 0.0) })
    };

    let line = t.text.lines().next().unwrap_or("");
    let glyphs = font.line(line, 0.0);
    let mut s = t.start_offset;
    let mut note = String::new();
    for (ci, (g, _)) in glyphs.iter().enumerate() {
        let adv = g.advance * k;
        let mid = s + adv / 2.0;
        if !closed && mid > total {
            note = format!("Text is longer than the curve: {} character(s) dropped", glyphs.len() - ci);
            break;
        }
        let (mut p, tan) = at(mid);
        let nrm = pt(-tan.y, tan.x); // left normal = "up" for the letters
        if !t.above {
            p = p - nrm * t.height; // hanging: cap tops touch the curve
        }
        let ang = tan.y.atan2(tan.x);
        let m = Affine::translate(p.x, p.y) * Affine::rotate(ang) * Affine::translate(-adv / 2.0, 0.0) * Affine::scale(k, k) * italic;
        place(&g.contours, &m, &mut contours);
        s += adv + t.spacing + t.kern(ci);
    }
    if t.weld && !font.stroke() {
        contours = gcarve_core::vecops::weld_keep_holes(&contours);
    }
    (contours, note)
}

impl Canvas {
    /// The selection was turned or scaled evenly by `m`: block texts wholly inside it take the turn and the size,
    /// so editing one later keeps it (a stretch or a mirror can't be kept: those stay as drawn until edited).
    pub fn texts_follow(&mut self, m: &Affine) {
        let det = m.a * m.d - m.b * m.c;
        let k = det.abs().sqrt();
        if det <= 0.0 || (m.a - m.d).abs() > 1e-6 * k.max(1.0) || (m.b + m.c).abs() > 1e-6 * k.max(1.0) {
            return;
        }
        let turn = m.b.atan2(m.a).to_degrees();
        if turn.abs() < 1e-9 && (k - 1.0).abs() < 1e-9 {
            return;
        }
        let (doc, sel) = (&mut self.doc, &self.sel);
        for t in doc.texts.iter_mut().filter(|t| t.path_shape == 0) {
            let mut glyphs = doc.shapes.iter().filter(|s| s.text == t.id).peekable();
            if glyphs.peek().is_some() && glyphs.all(|s| sel.contains(&s.id)) {
                t.angle = (t.angle + turn + 180.0).rem_euclid(360.0) - 180.0;
                t.height *= k;
                t.spacing *= k;
            }
        }
    }

    /// Replaces the glyph shapes of text `t` (none yet when t.id == 0). Toolpaths that used the old glyphs get
    /// the new ones. Returns the number of outlines placed.
    pub fn place_text(&mut self, mut t: TextItem, centre: Pt, layer: Option<i32>) -> usize {
        let path = (t.path_shape > 0).then(|| self.doc.find(t.path_shape).map(|s| s.c.clone())).flatten();
        if t.path_shape > 0 && path.is_none() {
            t.path_shape = 0; // the curve is gone: becomes block text where the glyphs are
        }
        let (contours, note) = build_text(&t, centre, path.as_ref());
        if contours.is_empty() {
            self.status = i18n::t("Nothing to place: empty text.").into();
            return 0;
        }
        self.doc.checkpoint();
        let mut group = 0;
        let old: Vec<i32> = if t.id != 0 {
            self.doc.shapes.iter().filter(|s| s.text == t.id).inspect(|s| if s.group != 0 { group = s.group }).map(|s| s.id).collect()
        } else {
            Vec::new()
        };
        if t.id == 0 {
            t.id = self.doc.next_text_id;
            self.doc.next_text_id += 1;
            self.doc.texts.push(t.clone());
        } else if let Some(cur) = self.doc.find_text(t.id) {
            *cur = t.clone();
        }
        if group == 0 {
            group = self.doc.new_group_id();
        }
        self.doc.shapes.retain(|s| !old.contains(&s.id));
        self.sel.clear();
        let n = contours.len();
        for c in contours {
            let id = self.doc.add(c, layer);
            let s = self.doc.find_mut(id).unwrap();
            s.text = t.id;
            s.group = group;
            self.sel.insert(id);
        }
        let new_ids: Vec<i32> = self.sel.iter().copied().collect();
        for op in &mut self.doc.ops {
            let before = op.shape_ids.len();
            op.shape_ids.retain(|id| !old.contains(id));
            if op.shape_ids.len() != before {
                op.shape_ids.extend(&new_ids);
            }
        }
        self.status = if !note.is_empty() {
            note
        } else if old.is_empty() {
            format!("Added \"{}\" ({n} outlines). Double-click to edit the text.", t.text.chars().take(20).collect::<String>())
        } else {
            format!("Text updated ({n} outlines)")
        };
        self.tool = Tool::Select;
        self.mutated();
        n
    }

    /// The text item every selected shape belongs to, else 0.
    pub fn selected_text(&self) -> i32 {
        let mut id = 0;
        for sid in &self.sel {
            match self.doc.find(*sid) {
                Some(s) if s.text != 0 && (id == 0 || s.text == id) => id = s.text,
                _ => return 0,
            }
        }
        id
    }

    pub fn convert_text_to_curves(&mut self) {
        if !self.sel.iter().any(|id| self.doc.find(*id).is_some_and(|s| s.text != 0)) {
            return;
        }
        self.doc.checkpoint();
        for id in self.sel.clone() {
            if let Some(s) = self.doc.find_mut(id) {
                s.text = 0;
            }
        }
        self.status = i18n::t("Text converted to curves").into();
        self.mutated();
    }

    /// Centre of a text item's glyphs (where an edit regenerates it).
    /// Vectors › Plate Production… (VCarve's): the selection is the master plate; every row of `rows` makes a copy
    /// to its right, then row by row downwards, with {1} {2}… (or {heading}) in its texts replaced by that row's
    /// fields. Stops at the board's bottom edge. Returns the plates made (one undo step).
    pub fn plate_production(&mut self, rows: &[Vec<String>], headings: &[String], gap: f64) -> usize {
        let r = self.sel_rect;
        let master: Vec<gcarve_core::document::Shape> = self.sel.iter().filter_map(|id| self.doc.find(*id).cloned()).collect();
        let texts: Vec<TextItem> = self.doc.texts.iter().filter(|t| t.text.contains('{') && t.path_shape == 0 && master.iter().any(|s| s.text == t.id)).cloned().collect();
        if r.is_empty() || texts.is_empty() {
            self.status = "Plate Production: select the master plate, with {1} (first field), {2}… or {heading} in its text.".into();
            return 0;
        }
        let (w, h) = (r.w() + gap, r.h() + gap);
        let per_line = (((self.doc.material.w - r.x0 + gap) / w).floor() as usize).max(1);
        self.doc.checkpoint();
        let mut made = 0;
        for (k, row) in rows.iter().enumerate() {
            let slot = k + 1; // the master keeps the first spot
            let (dx, dy) = ((slot % per_line) as f64 * w, -((slot / per_line) as f64) * h);
            if r.y0 + dy < -1e-9 {
                self.status = format!("Plate Production: {made} plates made; the other {} don't fit on the board below the master.", rows.len() - made);
                self.mutated();
                return made;
            }
            let group = self.doc.new_group_id();
            let shift = Affine::translate(dx, dy);
            for s in master.iter().filter(|s| s.text == 0 || !texts.iter().any(|t| t.id == s.text)) {
                let id = self.doc.add(transformed(&s.c, &shift), Some(s.layer));
                self.doc.find_mut(id).unwrap().group = group;
            }
            for src in &texts {
                let Some((c, layer)) = self.text_centre(src.id) else { continue };
                let mut text = src.text.clone();
                for (i, field) in row.iter().enumerate() {
                    text = text.replace(&format!("{{{}}}", i + 1), field.trim());
                    if let Some(hd) = headings.get(i).filter(|h| !h.trim().is_empty()) {
                        text = text.replace(&format!("{{{}}}", hd.trim()), field.trim());
                    }
                }
                let t = TextItem { id: self.doc.next_text_id, text, ..src.clone() };
                self.doc.next_text_id += 1;
                let (contours, _) = build_text(&t, pt(c.x + dx, c.y + dy), None);
                for g in contours {
                    let id = self.doc.add(g, Some(layer));
                    let s = self.doc.find_mut(id).unwrap();
                    s.text = t.id;
                    s.group = group;
                }
                self.doc.texts.push(t);
            }
            made += 1;
        }
        self.status = format!("Plate Production: {made} plates made next to the master.");
        self.mutated();
        made
    }

    fn text_centre(&self, id: i32) -> Option<(Pt, i32)> {
        let mut b = Rect::default();
        let mut layer = -1;
        for s in self.doc.shapes.iter().filter(|s| s.text == id) {
            b.add_rect(&bounds(&s.c));
            layer = s.layer;
        }
        (!b.is_empty()).then(|| (pt((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0), layer))
    }
}

/// Text dialog (new text, text on the selected curve, or editing an existing item).
#[derive(Default)]
pub struct TextDialog {
    pub open: bool,
    item: TextItem,
    height: String,
    spacing: String,
    arch: String,
    start: String,
    on_path: bool,
    path_choice: i32,
    /// just opened: put the cursor in the wording (a new text's placeholder selected, so typing replaces it)
    typed: bool,
    /// (select everything, frames since the button was released)
    focus: Option<(bool, u8)>,
    /// "Add a font file…" clicked: the app opens the file picker (tag "font")
    pub want_font: bool,
}

impl TextDialog {
    /// New block text (or on the one selected shape when `on_path`), centred where the canvas shows.
    pub fn open_new(&mut self, canvas: &Canvas, on_path: bool) {
        let doc = &canvas.doc;
        self.item = TextItem { text: i18n::t("Text").into(), family: FAMILIES[0].into(), ..Default::default() };
        // readable on the board at hand: about a tenth of its short side, in half inches (1 in on a 12 × 8), at most
        // 3 in: taller letters have strokes too wide for the standard 1/2 in V-bit, so a first V-carve would warn
        self.item.height = ((doc.material.w.min(doc.material.h) / 10.0 * 2.0).round() / 2.0).clamp(0.5, 3.0);
        self.height = units::text(doc, self.item.height);
        self.spacing = units::text(doc, 0.0);
        self.arch = units::text(doc, 0.0);
        self.start = units::text(doc, 0.0);
        self.path_choice = canvas.sel.iter().next().copied().unwrap_or(0);
        self.on_path = on_path && self.path_choice > 0;
        self.focus = Some((true, 0));
        self.typed = false;
        self.open = true;
    }
    pub fn open_for(&mut self, canvas: &Canvas, id: i32) {
        let Some(t) = canvas.doc.texts.iter().find(|t| t.id == id) else { return };
        self.item = t.clone();
        let doc = &canvas.doc;
        self.height = units::text(doc, t.height);
        self.spacing = units::text(doc, t.spacing);
        self.arch = units::text(doc, t.arch);
        self.start = units::text(doc, t.start_offset);
        self.on_path = t.path_shape > 0;
        self.path_choice = t.path_shape;
        self.focus = Some((false, 0));
        self.typed = false;
        self.open = true;
    }

    pub fn ui(&mut self, ctx: &egui::Context, canvas: &mut Canvas) {
        if !self.open {
            return;
        }
        let mut open = true;
        let mut ok = false;
        let editing = self.item.id != 0;
        crate::widgets::Dialog::new(if editing { i18n::t("Edit Text") } else { i18n::t("Add Text") }).id("text").open(&mut open).show(ctx, |ui| {
            let u = units::name(&canvas.doc);
            let dim = |t: &str| egui::RichText::new(t).color(crate::theme::TEXT.gamma_multiply(0.6));
            egui::Grid::new("text").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                ui.label(i18n::t("Text"));
                let mut out = egui::TextEdit::multiline(&mut self.item.text).desired_width(320.0).min_size(egui::vec2(320.0, 70.0)).margin(egui::Margin::symmetric(9, 6)).font(crate::theme::font(crate::theme::FONT_SIZE)).show(ui);
                // The cursor goes to the wording for the first half second (like the command palette, which asks
                // every frame): on the web the click that opened the dialog takes the keyboard back for a moment.
                // The selection is kept until the first keystroke, so typing during that time is not undone.
                if let Some((select_all, frames)) = self.focus {
                    out.response.request_focus();
                    if out.response.changed() {
                        self.typed = true; // from here on the cursor is the user's
                    }
                    if !self.typed {
                        let end = egui::text::CCursor::new(self.item.text.chars().count());
                        let from = if select_all { egui::text::CCursor::new(0) } else { end };
                        out.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(from, end)));
                        out.state.store(ui.ctx(), out.response.id);
                    }
                    let released = !ui.input(|i| i.pointer.any_down());
                    self.focus = (frames < 30 && !(frames > 3 && ui.input(|i| i.pointer.any_pressed()))).then_some((select_all, frames + u8::from(released)));
                    ui.ctx().request_repaint();
                }
                ui.end_row();
                ui.label(i18n::t("Font"));
                ui.horizontal(|ui| {
                    crate::widgets::combo_box("family").selected_text(&self.item.family).width(230.0).show_ui(ui, |ui| {
                        for f in FAMILIES.iter().map(|f| f.to_string()).chain(gcarve_ui::strokes::FAMILIES.iter().map(|(f, _)| f.to_string())).chain(custom_families()) {
                            ui.selectable_value(&mut self.item.family, f.clone(), f);
                        }
                    });
                    if ui.button(i18n::t("Add font…")).on_hover_text(i18n::t("Use your own .ttf or .otf file; it is kept on this device")).clicked() {
                        self.want_font = true;
                    }
                });
                if let Some(f) = ADDED.lock().unwrap().take() {
                    self.item.family = f; // the font just added
                }
                ui.end_row();
                ui.label(i18n::t("Lines"));
                ui.horizontal(|ui| {
                    for (v, label) in [(0, i18n::t("Left")), (1, i18n::t("Centre")), (2, i18n::t("Right"))] {
                        ui.selectable_value(&mut self.item.align, v, label);
                    }
                });
                ui.end_row();
                ui.label(format!("Letter height ({u})"));
                ui.horizontal(|ui| {
                    units::num_field(ui, "h", &mut self.height, &canvas.doc, true, 110.0);
                    ui.label(dim(i18n::t("cap height")));
                });
                ui.end_row();
                ui.label(i18n::t("Angle"));
                ui.add(egui::DragValue::new(&mut self.item.angle).speed(1.0).range(-360.0..=360.0).suffix("°")).on_hover_text(i18n::t("Turn the text counter-clockwise (not used along a curve)"));
                ui.end_row();
                ui.label(i18n::t("Style"));
                ui.horizontal(|ui| {
                    crate::widgets::checkbox(ui, &mut self.item.bold, i18n::t("Bold"));
                    crate::widgets::checkbox(ui, &mut self.item.italic, i18n::t("Italic"));
                    crate::widgets::checkbox(ui, &mut self.item.weld, i18n::t("Weld letters")).on_hover_text(i18n::t("Merge letters that overlap (script fonts) into one outline, so a V-carve doesn't cut through the joins"));
                });
                ui.end_row();
                let path_ok = self.path_choice > 0 && canvas.doc.find(self.path_choice).is_some();
                ui.label(i18n::t("Along curve"));
                ui.add_enabled(path_ok, crate::widgets::CheckBox::new(&mut self.on_path, if path_ok { i18n::t("Place letters along the selected curve") } else { i18n::t("Select one curve first to enable") }));
                ui.end_row();
                let on = self.on_path && path_ok;
                ui.add_enabled(on, egui::Label::new(format!("Start offset ({u})")));
                ui.add_enabled_ui(on, |ui| {
                    ui.horizontal(|ui| {
                        units::num_field(ui, "st", &mut self.start, &canvas.doc, true, 110.0);
                        ui.label(dim(i18n::t("distance along the curve")));
                    })
                });
                ui.end_row();
                ui.label(format!("Letter spacing ({u})"));
                units::num_field(ui, "sp", &mut self.spacing, &canvas.doc, true, 110.0);
                ui.end_row();
                ui.add_enabled(!self.on_path, egui::Label::new(format!("Arch radius ({u})")));
                ui.add_enabled_ui(!self.on_path, |ui| {
                    ui.horizontal(|ui| {
                        units::num_field(ui, "arch", &mut self.arch, &canvas.doc, true, 110.0);
                        ui.label(dim(i18n::t("0 = straight; negative bows it down")));
                    })
                });
                ui.end_row();
                // kerning by hand: the gap after each letter, nudged one by one (inches, + = wider)
                ui.label(i18n::t("Letter gaps")).on_hover_text(i18n::t("Drag a number to open or close the gap after that letter (in inches). Most fonts space well already: this is for the few pairs that look off at sign size, like AV or To."));
                let chars: Vec<char> = self.item.text.chars().collect();
                self.item.kerns.resize(chars.len(), 0.0);
                ui.vertical(|ui| {
                    egui::ScrollArea::horizontal().id_salt("kerns").max_width(330.0).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (i, ch) in chars.iter().enumerate().take(80) {
                                if *ch == '\n' || *ch == '\r' || i + 1 == chars.len() || chars[i + 1] == '\n' {
                                    continue; // no gap after the last letter of a line
                                }
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(format!("{ch}{}", chars[i + 1])).monospace());
                                    ui.add(egui::DragValue::new(&mut self.item.kerns[i]).speed(0.002).range(-self.item.height..=self.item.height).fixed_decimals(3));
                                });
                            }
                        });
                    });
                    if self.item.kerns.iter().any(|k| *k != 0.0) && ui.small_button(i18n::t("Reset gaps")).clicked() {
                        self.item.kerns.iter_mut().for_each(|k| *k = 0.0);
                    }
                });
                ui.end_row();
                ui.add_enabled(!self.on_path, egui::Label::new(i18n::t("Line spacing")));
                ui.add_enabled(!self.on_path, egui::DragValue::new(&mut self.item.line_spacing).range(0.3..=10.0).speed(0.02).fixed_decimals(2).suffix(i18n::t(" × normal")));
                ui.end_row();
                ui.add_enabled(on, egui::Label::new(i18n::t("Side")));
                ui.add_enabled(on, crate::widgets::CheckBox::new(&mut self.item.above, i18n::t("Letters stand on the curve (else hang below)")));
                ui.end_row();
                ui.label("");
                ui.allocate_ui(egui::vec2(320.0, 20.0), |ui| ui.add(egui::Label::new(dim(i18n::t("Text stays editable: double-click it later to change the wording or font. Toolpaths follow."))).wrap()));
                ui.end_row();
            });
            let (o, c) = crate::widgets::ok_cancel(ui);
            ok |= o;
            if c {
                self.open = false;
            }
        });
        if ok {
            let doc = &canvas.doc;
            let vals = [units::get(doc, &self.height, true), units::get(doc, &self.spacing, true), units::get(doc, &self.start, true), units::get(doc, &self.arch, true)];
            if let Some(Err(e)) = vals.iter().find(|v| v.is_err()) {
                canvas.status = e.clone();
                return;
            }
            let mut t = self.item.clone();
            t.height = vals[0].clone().unwrap();
            t.spacing = vals[1].clone().unwrap();
            t.start_offset = vals[2].clone().unwrap();
            t.arch = vals[3].clone().unwrap();
            t.path_shape = if self.on_path { self.path_choice } else { 0 };
            let (centre, layer) = if t.id != 0 {
                canvas.text_centre(t.id).map(|(c, l)| (c, Some(l))).unwrap_or((canvas.view_centre(), None))
            } else if t.path_shape > 0 {
                (Pt::default(), canvas.doc.find(t.path_shape).map(|s| s.layer))
            } else {
                (canvas.view_centre(), None)
            };
            if canvas.place_text(t, centre, layer) > 0 {
                self.open = false;
            }
        }
        if !open {
            self.open = false;
        }
    }
}

/// A node contour helper for tests
#[cfg(test)]
pub fn outline_bounds(t: &TextItem) -> Rect {
    let (c, _) = build_text(t, Pt::default(), None);
    let mut r = Rect::default();
    for k in &c {
        r.add_rect(&bounds(k));
    }
    r
}

#[cfg(test)]
mod tests {
    #[test]
    fn plates_fill_their_texts_from_the_rows() {
        let mut c = Canvas::default();
        c.doc.material.w = 30.0;
        c.doc.material.h = 20.0;
        let plate = c.doc.add(gcarve_core::geom::make_rect(1.0, 15.0, 7.0, 18.0), None);
        c.place_text(TextItem { text: "{name} {2}".into(), height: 1.0, ..Default::default() }, pt(4.0, 16.5), None);
        c.sel.insert(plate);
        c.sel_changed();
        let rows = vec![vec!["Ann".to_string(), "101".to_string()], vec!["Bo".to_string(), "102".to_string()], vec!["Cy".to_string(), "103".to_string()], vec!["Di".to_string(), "104".to_string()], vec!["Ed".to_string(), "105".to_string()]];
        let made = c.plate_production(&rows, &["name".to_string(), "room".to_string()], 0.5);
        assert_eq!(made, 5);
        let words: Vec<&str> = c.doc.texts.iter().map(|t| t.text.as_str()).collect();
        assert!(words.contains(&"Ann 101") && words.contains(&"Ed 105") && words.contains(&"{name} {2}"), "{words:?}");
        // 4 plates fit on a line of the 30 in board (master + 3), the rest start the next line lower down
        assert_eq!(c.doc.shapes.iter().filter(|s| s.text == 0).count(), 6);
        c.undo();
        assert_eq!(c.doc.texts.len(), 1);
    }

    use super::*;

    #[test]
    fn imported_text_stands_on_its_anchor_and_stays_editable() {
        use gcarve_core::svg::ImportText;
        let mut doc = gcarve_core::document::Document::default();
        let it = |anchor, valign| ImportText { text: "HE".into(), at: pt(4.0, 2.0), height: 1.0, layer: "Signs".into(), anchor, valign, angle: 0.0 };
        assert_eq!(import_texts(&mut doc, &[it(0, 0), it(2, 1), it(1, 2), ImportText { angle: 90.0, ..it(0, 0) }]), 4);
        let b = |id: i32| {
            let mut r = Rect::default();
            doc.shapes.iter().filter(|s| s.text == id).for_each(|s| r.add_rect(&bounds(&s.c)));
            r
        };
        let ids: Vec<i32> = doc.texts.iter().map(|t| t.id).collect();
        let (left, right, mid) = (b(ids[0]), b(ids[1]), b(ids[2]));
        assert!((left.x0 - 4.0).abs() < 0.2 && (left.y0 - 2.0).abs() < 0.02 && (left.y1 - 3.0).abs() < 0.02, "{left:?}"); // on the baseline
        assert!((right.x1 - 4.0).abs() < 0.2 && (right.y1 - 2.0).abs() < 0.02, "{right:?}"); // hangs from the top, ends at x
        assert!(((mid.x0 + mid.x1) / 2.0 - 4.0).abs() < 0.02 && ((mid.y0 + mid.y1) / 2.0 - 2.0).abs() < 0.02, "{mid:?}");
        // turned a quarter: the word climbs from its anchor instead of running right
        let up = b(ids[3]);
        assert!((up.y0 - 2.0).abs() < 0.2 && (up.x1 - 4.0).abs() < 0.02 && (up.x0 - 3.0).abs() < 0.02 && up.h() > up.w(), "{up:?}");
        assert!(doc.shapes.iter().all(|s| s.text != 0 && s.group != 0 && doc.layers[s.layer as usize].name == "Signs"));
    }

    #[test]
    fn turning_or_scaling_text_by_hand_survives_an_edit() {
        let mut c = Canvas::default();
        c.place_text(TextItem { text: "HE".into(), height: 1.0, ..Default::default() }, pt(5.0, 5.0), None);
        c.rotate_selection(90.0);
        c.transform_selection(&(Affine::translate(5.0, 5.0) * Affine::scale(2.0, 2.0) * Affine::translate(-5.0, -5.0)));
        let t = c.doc.texts[0].clone();
        assert!((t.angle - 90.0).abs() < 1e-9 && (t.height - 2.0).abs() < 1e-9, "{t:?}");
        let before = c.sel_rect;
        c.place_text(t, pt(5.0, 5.0), None); // what the text dialog's OK does
        let after = c.sel_rect;
        assert!((before.w() - after.w()).abs() < 1e-6 && (before.h() - after.h()).abs() < 1e-6 && after.h() > after.w(), "{before:?} {after:?}");
        // a stretch can't be kept: the text is left alone
        c.transform_selection(&Affine::scale(2.0, 1.0));
        assert!((c.doc.texts[0].height - 2.0).abs() < 1e-9);
    }

    #[test]
    fn block_text_bounds() {
        let t = TextItem { text: "HE".into(), height: 1.0, ..Default::default() };
        let (c, note) = build_text(&t, pt(5.0, 5.0), None);
        assert!(note.is_empty() && c.len() == 2 && c.iter().all(|k| k.closed));
        let b = outline_bounds(&t);
        assert!((b.h() - 1.0).abs() < 1e-6, "cap height is the text height: {}", b.h());
        assert!(b.w() > 1.0 && b.w() < 2.0);
        let mut r = Rect::default();
        for k in &c {
            r.add_rect(&bounds(k));
        }
        assert!(((r.x0 + r.x1) / 2.0 - 5.0).abs() < 1e-6 && ((r.y0 + r.y1) / 2.0 - 5.0).abs() < 1e-6); // centred
        let o = TextItem { text: "O".into(), height: 1.0, bold: true, ..Default::default() };
        let (c, _) = build_text(&o, Pt::default(), None);
        assert_eq!(c.len(), 2); // outer + hole
        let wide = TextItem { text: "HE".into(), height: 1.0, spacing: 0.5, ..Default::default() };
        assert!(outline_bounds(&wide).w() > b.w() + 0.49);
        // kerning by hand: 0.3 in after the first of two letters makes the pair 0.3 wider
        let tight = outline_bounds(&TextItem { text: "HE".into(), height: 1.0, ..Default::default() }).w();
        let loose = outline_bounds(&TextItem { text: "HE".into(), height: 1.0, kerns: vec![0.3], ..Default::default() }).w();
        assert!((loose - tight - 0.3).abs() < 1e-6, "{tight} {loose}");
        // line spacing: two lines twice as far apart make the block taller by one more line pitch
        let two = |line_spacing| outline_bounds(&TextItem { text: "H\nH".into(), height: 1.0, line_spacing, ..Default::default() }).h();
        let (normal, double) = (two(1.0), two(2.0));
        assert!(double > normal + 0.9, "{normal} {double}");
        // arched up: the middle letters stand higher than the ends, and the word stays centred
        let arch = |arch| build_text(&TextItem { text: "ABCDEFG".into(), height: 1.0, arch, ..Default::default() }, pt(0.0, 0.0), None).0;
        let ys = |cs: &[gcarve_core::geom::Contour]| { let b: Vec<_> = cs.iter().map(bounds).collect(); (b.first().unwrap().y0, b.iter().map(|r| r.y1).fold(f64::MIN, f64::max)) };
        let (end_y, top) = ys(&arch(4.0));
        assert!(top > end_y + 0.9, "{end_y} {top}");
        let all = arch(4.0).iter().fold(gcarve_core::geom::Rect::default(), |mut r, c| { r.add_rect(&bounds(c)); r });
        assert!(((all.x0 + all.x1) / 2.0).abs() < 0.15, "{all:?}");
        if let Ok(out) = std::env::var("GCARVE_ARCH_SVG") {
            let mut cs = arch(4.0);
            cs.extend(arch(-4.0).iter().map(|c| gcarve_core::geom::transformed(c, &Affine::translate(0.0, -3.0))));
            let cs: Vec<_> = cs.iter().map(|c| gcarve_core::geom::transformed(c, &Affine::translate(5.0, 5.0))).collect();
            std::fs::write(out, gcarve_core::svg::export_svg(&cs.iter().map(|c| (c, "")).collect::<Vec<_>>(), 10.0, 7.0)).unwrap();
        }
    }

    #[test]
    fn place_and_edit_keeps_group_and_toolpaths() {
        use gcarve_core::document::Op;
        let mut c = Canvas::default();
        let t = TextItem { text: "Hl".into(), height: 1.0, ..Default::default() };
        assert_eq!(c.place_text(t, pt(2.0, 2.0), None), 2);
        let g = c.doc.shapes[0].group;
        assert!(g != 0 && c.doc.shapes.iter().all(|s| s.group == g && s.text == 1) && c.doc.texts.len() == 1);
        let ids: Vec<i32> = c.doc.shapes.iter().map(|s| s.id).collect();
        c.doc.ops.push(Op { shape_ids: ids.clone(), ..Default::default() });
        let mut t = c.doc.texts[0].clone();
        t.text = "Hey".into();
        assert_eq!(c.place_text(t, pt(2.0, 2.0), None), 4); // H, e (with hole), y
        assert!(c.doc.shapes.iter().all(|s| s.group == g && !ids.contains(&s.id)));
        assert_eq!(c.doc.ops[0].shape_ids.len(), 4, "toolpath follows the new glyphs");
        assert_eq!(c.selected_text(), 1);
        c.convert_text_to_curves();
        assert!(c.doc.texts.is_empty() && c.selected_text() == 0);
    }

    #[test]
    fn text_on_path() {
        let path = gcarve_core::geom::make_polyline(&[pt(0.0, 0.0), pt(10.0, 0.0)], false);
        let t = TextItem { text: "AB".into(), height: 0.5, path_shape: 1, start_offset: 1.0, ..Default::default() };
        let (c, note) = build_text(&t, Pt::default(), Some(&path));
        assert!(note.is_empty() && c.len() == 5, "{note} {}", c.len()); // A has a hole, B two
        let mut r = Rect::default();
        for k in &c {
            r.add_rect(&bounds(k));
        }
        assert!(r.x0 > 0.9 && r.y0 > -1e-6 && (r.y1 - 0.5).abs() < 1e-6);
        let long = TextItem { text: "ABCDEFGHIJKLMNOPQRSTUVWXYZ".into(), height: 1.0, path_shape: 1, ..Default::default() };
        let (_, note) = build_text(&long, Pt::default(), Some(&path));
        assert!(note.contains("dropped"));
    }

    #[test]
    fn a_zipped_font_download_gives_its_regular_style_and_web_fonts_are_named() {
        let zip = crate::files::zip(&[("__MACOSX/._Font-Regular.ttf".into(), b"junk".to_vec()), ("Font-Bold.ttf".into(), b"bold".to_vec()), ("static/Font-Regular.ttf".into(), gcarve_ui::glyphs::BOLD.to_vec())]);
        assert_eq!(unzip_font(zip).unwrap(), gcarve_ui::glyphs::BOLD);
        assert!(add_font(crate::files::zip(&[("readme.txt".into(), b"x".to_vec())])).unwrap_err().contains("no .ttf"));
        assert!(add_font(b"wOF2 rest".to_vec()).unwrap_err().contains(".woff"));
    }

    #[test]
    fn an_added_font_is_used_for_its_family_and_unknown_families_fall_back() {
        let family = add_font(gcarve_ui::glyphs::BOLD.to_vec()).unwrap();
        assert!(custom_families().contains(&family), "{family}");
        let t = |f: &str| gcarve_core::document::TextItem { text: "HE".into(), height: 1.0, family: f.into(), ..Default::default() };
        let w = |f: &str| {
            let c = build_text(&t(f), pt(0.0, 0.0), None).0;
            c.iter().map(bounds).fold(Rect::default(), |mut a, b| {
                a.add_rect(&b);
                a
            })
            .w()
        };
        // the family added is the bold file: wider than the regular face, and an unknown name gives the regular face
        assert!(w(&family) > w("No Such Font") + 1e-6, "{} vs {}", w(&family), w("No Such Font"));
        assert!(add_font(b"not a font".to_vec()).is_err());
    }

    #[test]
    fn single_line_fonts_give_open_strokes_the_right_size_that_cut_on_the_line() {
        let t = TextItem { text: "HELLO\nworld".into(), family: "Single Line Sans".into(), height: 1.0, align: 1, weld: true, ..Default::default() };
        let (cs, _) = build_text(&t, pt(10.0, 5.0), None);
        assert!(!cs.is_empty() && cs.iter().all(|c| !c.closed), "open strokes, weld ignored");
        let mut r = Rect::default();
        cs.iter().for_each(|c| r.add_rect(&bounds(c)));
        assert!(((r.x0 + r.x1) / 2.0 - 10.0).abs() < 0.05, "centred: {r:?}");
        let mut h = Rect::default();
        build_text(&TextItem { text: "H".into(), ..t.clone() }, pt(0.0, 0.0), None).0.iter().for_each(|c| h.add_rect(&bounds(c)));
        assert!((h.y1 - h.y0 - 1.0).abs() < 0.08, "capitals 1 in tall: {h:?}");
        // a V-bit on the line engraves them
        let mut doc = gcarve_core::document::Document::default();
        let ids: Vec<i32> = cs.into_iter().map(|c| doc.add(c, None)).collect();
        doc.tools = vec![gcarve_core::document::Tool { name: "V".into(), kind: gcarve_core::document::ToolType::VBit, diameter: 0.5, angle: 60.0, stepdown: 0.1, feed_xy: 60.0, feed_z: 20.0, rpm: 18000, ..Default::default() }];
        doc.ops = vec![gcarve_core::document::Op { kind: gcarve_core::document::OpType::Profile, side: gcarve_core::document::Side::On, depth: 0.05, shape_ids: ids, ..Default::default() }];
        let g = gcarve_core::pipeline::regenerate(&doc);
        assert!(g.warnings.is_empty() && !g.paths[0].is_empty(), "{:?}", g.warnings);
    }

    #[test]
    fn lines_align_and_welded_letters_keep_their_holes() {
        let t = |align: i32, weld: bool, text: &str, spacing: f64| gcarve_core::document::TextItem { text: text.into(), height: 1.0, align, weld, spacing, ..Default::default() };
        let right_edge = |item: &gcarve_core::document::TextItem, line_top: bool| {
            let c = build_text(item, pt(0.0, 0.0), None).0;
            let mid = c.iter().map(bounds).fold(Rect::default(), |mut a, b| {
                a.add_rect(&b);
                a
            });
            // the right edge of the glyphs in the top (or bottom) half
            let cut = (mid.y0 + mid.y1) / 2.0;
            c.iter().map(bounds).filter(|b| ((b.y0 + b.y1) / 2.0 > cut) == line_top).map(|b| b.x1).fold(f64::MIN, f64::max)
        };
        // a long line over a short one: left = ragged right, right = both end at the same x
        let (l, r) = (t(0, false, "WWWW\nW", 0.0), t(2, false, "WWWW\nW", 0.0));
        assert!((right_edge(&l, true) - right_edge(&l, false)).abs() > 0.5);
        assert!((right_edge(&r, true) - right_edge(&r, false)).abs() < 1e-6);
        // "oo" pulled together so the two letters overlap: welded = one outer + the two counters; loose = 4 outlines
        let loose = build_text(&t(0, false, "oo", -0.25), pt(0.0, 0.0), None).0;
        let welded = build_text(&t(0, true, "oo", -0.25), pt(0.0, 0.0), None).0;
        assert_eq!(loose.len(), 4);
        assert_eq!(welded.len(), 3, "{}", welded.len());
    }
}
