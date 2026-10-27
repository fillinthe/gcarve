//! Glyph outlines of the bundled Liberation Sans (assets/, SIL OFL: see LiberationSans-LICENSE.txt) through
//! ttf-parser, shared by GCarve's Text tool and GSketch's 3D Text: the same letters on every platform and in the
//! browser. Font units, y up, baseline at 0.

use gcarve_core::geom::{pt, transformed, Affine, Builder, Contour};
use ttf_parser::{Face, GlyphId, OutlineBuilder};

pub const REGULAR: &[u8] = include_bytes!("../assets/LiberationSans-Regular.ttf");
pub const BOLD: &[u8] = include_bytes!("../assets/LiberationSans-Bold.ttf");

/// The bundled face, regular or bold.
pub fn bundled(bold: bool) -> Face<'static> {
    Face::parse(if bold { BOLD } else { REGULAR }, 0).expect("bundled font")
}

/// Collects one glyph's contours (font units, y up) as closed Contours.
pub struct Outline {
    cur: Builder,
    out: Vec<Contour>,
}

impl Outline {
    fn flush(&mut self) {
        if self.cur.c.nodes.len() >= 2 {
            self.cur.close();
            self.out.push(std::mem::take(&mut self.cur.c));
        }
        self.cur.c = Contour::default();
    }
}

impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        self.cur.start(pt(x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.cur.line(pt(x as f64, y as f64));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.cur.cur();
        let q = pt(x1 as f64, y1 as f64);
        let p = pt(x as f64, y as f64);
        self.cur.cubic(p0 + (q - p0) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.cur.cubic(pt(x1 as f64, y1 as f64), pt(x2 as f64, y2 as f64), pt(x as f64, y as f64));
    }
    fn close(&mut self) {
        self.flush();
    }
}

/// A font on this computer: family and style (from its name table), the file, and which face in a .ttc / .otc.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFile {
    pub family: String,
    pub style: String,
    pub path: std::path::PathBuf,
    pub index: u32,
}

fn font_name(f: &Face, ids: [u16; 2]) -> Option<String> {
    ids.iter().find_map(|id| f.names().into_iter().filter(|n| n.name_id == *id && n.is_unicode()).find_map(|n| n.to_string()).filter(|s| !s.trim().is_empty()))
}

/// Family and style of every face in a font file's bytes (one per face of a collection).
pub fn faces_in(data: &[u8]) -> Vec<(String, String, u32)> {
    (0..ttf_parser::fonts_in_collection(data).unwrap_or(1))
        .filter_map(|i| {
            let f = Face::parse(data, i).ok()?;
            Some((font_name(&f, [16, 1])?, font_name(&f, [17, 2]).unwrap_or_else(|| crate::i18n::t("Regular").into()), i))
        })
        .collect()
}

/// Every TrueType / OpenType font in the usual font folders (macOS, Windows, Linux, the user's own), sorted by family
/// then style; hidden system faces (names starting with '.') left out. Reads each file once: call it off the UI's
/// first frame and keep the list.
#[cfg(not(target_arch = "wasm32"))]
pub fn system_fonts() -> Vec<FontFile> {
    use std::path::{Path, PathBuf};
    let mut dirs: Vec<PathBuf> = ["/System/Library/Fonts", "/Library/Fonts", "/usr/share/fonts", "/usr/local/share/fonts", "C:\\Windows\\Fonts"].iter().map(PathBuf::from).collect();
    if let Some(h) = std::env::var_os("HOME") {
        for sub in ["Library/Fonts", ".fonts", ".local/share/fonts"] {
            dirs.push(Path::new(&h).join(sub));
        }
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(Path::new(&l).join("Microsoft\\Windows\\Fonts"));
    }
    fn walk(dir: &Path, depth: u32, out: &mut Vec<FontFile>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 4 {
                    walk(&p, depth + 1, out);
                }
                continue;
            }
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
                continue;
            }
            let Ok(data) = std::fs::read(&p) else { continue };
            for (family, style, index) in faces_in(&data) {
                if !family.starts_with('.') {
                    out.push(FontFile { family, style, path: p.clone(), index });
                }
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        walk(&d, 0, &mut out);
    }
    out.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()).then(a.style.cmp(&b.style)));
    out.dedup_by(|a, b| a.family == b.family && a.style == b.style);
    out
}

pub struct Glyph {
    pub contours: Vec<Contour>, // font units
    pub advance: f64,
}

pub fn glyph(face: &Face, ch: char) -> (Glyph, Option<GlyphId>) {
    let Some(gid) = face.glyph_index(ch) else { return (Glyph { contours: Vec::new(), advance: face.units_per_em() as f64 / 3.0 }, None) };
    let mut o = Outline { cur: Builder::default(), out: Vec::new() };
    face.outline_glyph(gid, &mut o);
    o.flush();
    (Glyph { contours: o.out, advance: face.glyph_hor_advance(gid).unwrap_or(0) as f64 }, Some(gid))
}

/// ponytail: 'kern' table pairs only (Liberation has them); GPOS kerning would need the opentype-layout walk
pub fn kern(face: &Face, a: Option<GlyphId>, b: Option<GlyphId>) -> f64 {
    let (Some(a), Some(b), Some(k)) = (a, b, face.tables().kern) else { return 0.0 };
    k.subtables.into_iter().filter(|st| st.horizontal && !st.variable).find_map(|st| st.glyphs_kerning(a, b)).unwrap_or(0) as f64
}

/// Outlines of one line of text at the origin (font units, baseline y = 0): each glyph with its x offset.
pub fn line_glyphs(face: &Face, text: &str, spacing_units: f64) -> Vec<(Glyph, f64)> {
    let mut x = 0.0;
    let mut prev = None;
    let mut out = Vec::new();
    for ch in text.chars() {
        let (g, gid) = glyph(face, ch);
        x += kern(face, prev, gid);
        let adv = g.advance;
        out.push((g, x));
        x += adv + spacing_units;
        prev = gid;
    }
    out
}

/// Text as closed outlines in inches, capitals `cap_in` tall, lines `line_h` (cap heights) apart, left-aligned, the
/// first baseline at y = 0 (GSketch's 3D Text).
pub fn text_outlines(text: &str, cap_in: f64, bold: bool) -> Vec<Contour> {
    let face = bundled(bold);
    let cap = face.capital_height().filter(|&c| c > 0).unwrap_or(face.ascender()) as f64;
    let k = cap_in / cap;
    let line_h = face.height() as f64 + face.line_gap() as f64;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for (g, x) in line_glyphs(&face, line, 0.0) {
            let m = Affine::scale(k, k) * Affine::translate(x, -(i as f64) * line_h);
            out.extend(g.contours.iter().map(|c| transformed(c, &m)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// how long the scan takes on this computer: cargo test -p gcarve-ui system_fonts_scan -- --ignored --nocapture
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore]
    fn system_fonts_scan() {
        let t = std::time::Instant::now();
        let f = super::system_fonts();
        println!("{} faces in {:?}; first {:?}", f.len(), t.elapsed(), f.first());
    }

    #[test]
    fn the_bundled_fonts_name_themselves() {
        assert_eq!(super::faces_in(super::REGULAR), vec![("Liberation Sans".to_string(), "Regular".to_string(), 0)]);
        assert_eq!(super::faces_in(super::BOLD)[0].1, "Bold");
        assert!(super::faces_in(b"not a font").is_empty());
    }

    #[test]
    fn letters_come_out_at_the_asked_height() {
        let out = super::text_outlines("HO", 2.0, false);
        assert_eq!(out.len(), 3, "H has one outline, O two");
        let b = out.iter().map(gcarve_core::geom::bounds).fold(gcarve_core::geom::Rect::default(), |mut r, x| {
            r.add_rect(&x);
            r
        });
        assert!((b.y1 - 2.0).abs() < 0.05 && b.y0.abs() < 0.05, "{b:?}");
    }
}
