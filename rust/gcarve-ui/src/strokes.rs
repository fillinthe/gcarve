//! Single-line (stroke) fonts for engraving: the Hershey fonts (assets/hershey, credits in README.txt there), read
//! from their .jhf text: one glyph per line for ASCII 32..126, five digits of id, three of vertex count, then pairs of
//! letters (coordinate = letter − 'R'), the first pair the left and right edges, " R" = pen up. Glyphs come out as
//! open lines in font units, y up, baseline at 0, capitals CAP tall: the same shape as `glyphs::Glyph`.

use crate::glyphs::Glyph;
use gcarve_core::geom::{make_polyline, pt, Pt};

/// font units from the baseline to the top of the capitals (Hershey roman capitals run y −12 .. 9)
pub const CAP: f64 = 21.0;
/// baseline to baseline
pub const LINE: f64 = 32.0;

/// (family name in the Text dialog, .jhf data)
pub const FAMILIES: [(&str, &str); 3] = [
    ("Single Line Sans", include_str!("../assets/hershey/futural.jhf")),
    ("Single Line Script", include_str!("../assets/hershey/scripts.jhf")),
    ("Single Line Cursive", include_str!("../assets/hershey/cursive.jhf")),
];

pub fn is_stroke(family: &str) -> bool {
    FAMILIES.iter().any(|(f, _)| *f == family)
}

/// The glyph for `ch` in `family` (an unknown character: a blank the width of a space).
pub fn glyph(family: &str, ch: char) -> Glyph {
    let data = FAMILIES.iter().find(|(f, _)| *f == family).map_or(FAMILIES[0].1, |(_, d)| d);
    let i = (ch as u32).wrapping_sub(32) as usize;
    let line = if (ch as u32) < 127 { data.lines().nth(i) } else { None };
    parse(line.unwrap_or("")).unwrap_or(Glyph { contours: Vec::new(), advance: 16.0 })
}

fn parse(line: &str) -> Option<Glyph> {
    let b = line.as_bytes();
    let n: usize = line.get(5..8)?.trim().parse().ok()?;
    let pairs = b.get(8..8 + 2 * n)?;
    let v = |c: u8| f64::from(c) - f64::from(b'R');
    let (left, right) = (v(pairs[0]), v(pairs[1]));
    let mut contours = Vec::new();
    let mut cur: Vec<Pt> = Vec::new();
    for p in pairs[2..].chunks(2) {
        if p == b" R" {
            if cur.len() > 1 {
                contours.push(make_polyline(&cur, false));
            }
            cur.clear();
        } else {
            cur.push(pt(v(p[0]) - left, 9.0 - v(p[1]))); // y down from the top → y up from the baseline
        }
    }
    if cur.len() > 1 {
        contours.push(make_polyline(&cur, false));
    }
    Some(Glyph { contours, advance: right - left })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_core::geom::bounds;

    #[test]
    fn every_font_has_every_printable_letter_as_open_lines_on_the_baseline() {
        for (family, data) in FAMILIES {
            assert!(data.lines().count() >= 95, "{family}");
            for ch in (33u8..127).map(char::from) {
                let g = glyph(family, ch);
                assert!(!g.contours.is_empty() && g.advance > 0.0, "{family} {ch:?}");
                assert!(g.contours.iter().all(|c| !c.closed), "single lines");
            }
            // capital H stands on the baseline and is CAP tall; a space is blank but moves on
            let mut r = gcarve_core::geom::Rect::default();
            glyph(family, 'H').contours.iter().for_each(|c| r.add_rect(&bounds(c)));
            assert!(r.y0.abs() < 1.5 && (r.y1 - CAP).abs() < 1.5, "{family}: {r:?}");
            let sp = glyph(family, ' ');
            assert!(sp.contours.is_empty() && sp.advance > 0.0);
        }
    }
}
