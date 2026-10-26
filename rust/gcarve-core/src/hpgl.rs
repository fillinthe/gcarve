//! HPGL plot files (.plt / .hpgl): what CorelDRAW, sign cutters and many laser shops export. One layer per pen
//! ("Pen 1"…). Units: 40 plotter units per mm, Y up.
//!
//! Read: IN, SP, PU / PD (absolute after PA, relative after PR), PA, PR, CI (circle round the pen), AA / AR (arcs),
//! EA / ER (rectangle outlines). LB labels (text) and fills are counted as skipped.

use crate::geom::{make_ellipse, make_polyline, pt, Pt};
use crate::svg::ImportResult;

/// plotter units per inch (40 per mm)
const UNITS_PER_IN: f64 = 1016.0;

/// A .plt file's pen-down paths, inches, Y up.
pub fn import_hpgl(text: &str) -> ImportResult {
    let mut r = ImportResult::default();
    let b = text.as_bytes();
    let (mut at, mut down, mut rel, mut pen) = (pt(0.0, 0.0), false, false, 1u32);
    let mut run: Vec<Pt> = Vec::new();
    let (mut skipped, mut seen) = (0usize, 0usize);
    let layer = |pen: u32| format!("Pen {pen}");
    // a finished pen-down run becomes a contour (closed when it ends where it began)
    let flush = |run: &mut Vec<Pt>, r: &mut ImportResult, pen: u32| {
        if run.len() >= 2 {
            let closed = run.len() > 2 && (run[0].x - run[run.len() - 1].x).hypot(run[0].y - run[run.len() - 1].y) < 1e-6;
            if closed {
                run.pop();
            }
            r.contours.push(make_polyline(run, closed));
            r.layers.push(layer(pen));
        }
        run.clear();
    };
    let mut i = 0;
    while i + 1 < b.len() {
        if !(b[i].is_ascii_alphabetic() && b[i + 1].is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        let cmd = [b[i].to_ascii_uppercase(), b[i + 1].to_ascii_uppercase()];
        i += 2;
        if &cmd == b"LB" {
            // a label runs to the ETX character (0x03)
            while i < b.len() && b[i] != 3 {
                i += 1;
            }
            skipped += 1;
            continue;
        }
        // the parameters: numbers up to the next command or ';'
        let start = i;
        while i < b.len() && b[i] != b';' && !b[i].is_ascii_alphabetic() {
            i += 1;
        }
        let nums: Vec<f64> = text[start..i].split(|c: char| c == ',' || c.is_ascii_whitespace()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        let to_in = |v: f64| v / UNITS_PER_IN;
        match &cmd {
            b"IN" => {
                flush(&mut run, &mut r, pen);
                (at, down, rel) = (pt(0.0, 0.0), false, false);
            }
            b"SP" => {
                flush(&mut run, &mut r, pen);
                pen = nums.first().map_or(1, |p| *p as u32);
            }
            b"PA" | b"PR" | b"PU" | b"PD" => {
                match &cmd {
                    b"PA" => rel = false,
                    b"PR" => rel = true,
                    b"PU" => {
                        flush(&mut run, &mut r, pen);
                        down = false;
                    }
                    _ => down = true,
                }
                for p in nums.chunks(2).filter(|c| c.len() == 2) {
                    let q = if rel { pt(at.x + to_in(p[0]), at.y + to_in(p[1])) } else { pt(to_in(p[0]), to_in(p[1])) };
                    if down {
                        if run.is_empty() {
                            run.push(at);
                        }
                        run.push(q);
                    }
                    at = q;
                }
                seen += 1;
            }
            b"CI" => {
                if let Some(&rad) = nums.first() {
                    flush(&mut run, &mut r, pen);
                    let rad = to_in(rad).abs();
                    r.contours.push(make_ellipse(at.x, at.y, rad, rad));
                    r.layers.push(layer(pen));
                }
            }
            b"AA" | b"AR" => {
                // arc round a centre from the pen, `sweep` degrees (positive = anticlockwise); drawn when the pen is down
                if let [cx, cy, sweep, ..] = nums[..] {
                    let sweep = if sweep.is_finite() { sweep.clamp(-360.0, 360.0) } else { 0.0 }; // a damaged file's 1e14° hung the import
                    let c = if &cmd == b"AR" { pt(at.x + to_in(cx), at.y + to_in(cy)) } else { pt(to_in(cx), to_in(cy)) };
                    let (rad, a0) = ((at.x - c.x).hypot(at.y - c.y), (at.y - c.y).atan2(at.x - c.x));
                    let steps = ((sweep.abs() / 5.0).ceil() as usize).max(1);
                    if down && run.is_empty() {
                        run.push(at);
                    }
                    for k in 1..=steps {
                        let a = a0 + sweep.to_radians() * k as f64 / steps as f64;
                        at = pt(c.x + rad * a.cos(), c.y + rad * a.sin());
                        if down {
                            run.push(at);
                        }
                    }
                }
            }
            b"EA" | b"ER" => {
                if let [x, y, ..] = nums[..] {
                    let o = if &cmd == b"ER" { pt(at.x + to_in(x), at.y + to_in(y)) } else { pt(to_in(x), to_in(y)) };
                    let c = make_polyline(&[at, pt(o.x, at.y), o, pt(at.x, o.y)], true);
                    r.contours.push(c);
                    r.layers.push(layer(pen));
                }
            }
            b"RA" | b"RR" | b"WG" | b"PM" | b"FP" | b"EP" => skipped += 1,
            _ => {}
        }
        if i < b.len() && b[i] == b';' {
            i += 1;
        }
    }
    flush(&mut run, &mut r, pen);
    if r.contours.is_empty() {
        r.note = if seen == 0 { "This isn't an HPGL plot file.".into() } else { "No pen-down paths in the plot file.".into() };
        return r;
    }
    r.note = format!("{} paths from the plot file{}", r.contours.len(), if skipped > 0 { format!(", {skipped} labels / fills skipped") } else { String::new() });
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::bounds;

    #[test]
    fn pens_become_layers_and_units_are_40_per_mm() {
        // a 10 × 5 mm rectangle with pen 1 (absolute), a relative 4 mm line with pen 2, a circle of radius 2 mm
        let r = import_hpgl("IN;SP1;PU0,0;PD400,0,400,200,0,200,0,0;PU;SP2;PU1000 1000;PR;PD160,0;PA;PU2000,2000;CI80;LBhello\x03;");
        assert_eq!(r.layers, ["Pen 1", "Pen 2", "Pen 2"]);
        let mm = |v: f64| v * 25.4;
        let b = bounds(&r.contours[0]);
        assert!(r.contours[0].closed && (mm(b.w()) - 10.0).abs() < 1e-9 && (mm(b.h()) - 5.0).abs() < 1e-9);
        let l = bounds(&r.contours[1]);
        assert!(!r.contours[1].closed && (mm(l.w()) - 4.0).abs() < 1e-9 && (mm(l.x0) - 25.0).abs() < 1e-9);
        let c = bounds(&r.contours[2]);
        assert!((mm(c.w()) - 4.0).abs() < 1e-6 && (mm(c.x0) - 48.0).abs() < 1e-6);
        assert!(r.note.contains("1 labels"), "{}", r.note);
        // run together without separators, lower case
        assert_eq!(import_hpgl("in;sp1;pu0,0pd40,0,40,40pu").contours.len(), 1);
        // an arc: a half circle of radius 1 mm round (1 mm, 0) from the origin, pen down
        let a = import_hpgl("PU0,0;PD;AA40,0,180;PU;");
        let ab = bounds(&a.contours[0]);
        assert!((mm(ab.w()) - 2.0).abs() < 1e-6 && (mm(ab.h()) - 1.0).abs() < 0.01, "{ab:?}");
        assert!(import_hpgl("<svg/>").contours.is_empty());
        // junk never panics
        let mut x = 7u64;
        for _ in 0..2000 {
            let junk: String = (0..40)
                .map(|_| {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    ['P', 'D', 'U', 'A', 'C', 'I', 'L', 'B', ';', ',', '-', '1', '9', '.', ' ', 'é', '\x03', 'R', 'E'][(x >> 33) as usize % 19]
                })
                .collect();
            let _ = import_hpgl(&junk);
        }
    }
}
