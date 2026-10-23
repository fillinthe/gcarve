//! PCB files: Gerber RS-274X (copper, mask, silkscreen, outline layers) and Excellon drill files. Inches, Y up.
//!
//! Gerber: FS / MO, apertures C R O P and macros (AM: circle, vector / centre line, outline, polygon, thermal,
//! variables), D01 draws (G01 lines, G02 / G03 arcs, G75),
//! D02 moves, D03 flashes, G36 / G37 regions, LPD / LPC polarity. The copper comes out as filled outlines (traces
//! stroked round at the aperture's width and merged); zero-width draws (board outlines) stay as lines.
//! Excellon: METRIC / INCH (LZ / TZ), tool sizes `T1C0.8`, hits `X…Y…` → a circle per hole.

use crate::geom::{make_ellipse, make_polyline, pt, Contour, Pt};
use crate::svg::ImportResult;
use crate::vecops::{make_polygon, offset_contours, paint_layers};
use std::collections::HashMap;
use std::f64::consts::PI;

#[derive(Clone, Debug)]
enum Aperture {
    Circle(f64, f64),
    Rect(f64, f64, f64),
    Obround(f64, f64, f64),
    Poly(f64, usize, f64, f64),
    /// a macro's primitives and the parameters it was given (file units)
    Macro(Vec<String>, Vec<f64>),
}

/// `n` points round `c` at radius `r`, the first at `a0` radians.
fn ring(c: Pt, r: f64, n: usize, a0: f64) -> Vec<Pt> {
    (0..n).map(|i| { let a = a0 + 2.0 * PI * i as f64 / n as f64; pt(c.x + r * a.cos(), c.y + r * a.sin()) }).collect()
}

/// An aperture macro's pad at `c`: each primitive's outline in macro units, turned by its rotation about the macro's
/// origin, scaled to inches; exposure 0 scrapes away what came before it.
fn macro_pad(prims: &[String], params: &[f64], c: Pt, to_in: f64) -> Vec<Contour> {
    let mut vars: HashMap<String, f64> = params.iter().enumerate().map(|(i, v)| (format!("p{}", i + 1), *v)).collect();
    let ev = |s: &str, vars: &HashMap<String, f64>| -> f64 {
        let e = s.replace('$', "p").replace(['x', 'X'], "*");
        crate::expr::eval_expr_dim(&e, 1.0, &|n| vars.get(n).map(|v| (*v, false))).map_or(0.0, |r| r.0)
    };
    let mut steps: Vec<(bool, Vec<Contour>)> = Vec::new();
    for prim in prims {
        if let Some((name, e)) = prim.strip_prefix('$').and_then(|p| p.split_once('=')) {
            let v = ev(e, &vars);
            vars.insert(format!("p{name}"), v);
            continue;
        }
        let f: Vec<f64> = prim.split(',').map(|x| ev(x, &vars)).collect();
        let g = |i: usize| f.get(i).copied().unwrap_or(0.0);
        let code = g(0) as i32;
        // (dark, outlines in macro units, rotation °)
        let (dark, shapes, rot): (bool, Vec<Vec<Pt>>, f64) = match code {
            1 => (g(1) != 0.0, vec![ring(pt(g(3), g(4)), g(2) / 2.0, 48, 0.0)], g(5)),
            20 => {
                let (a, b, w) = (pt(g(3), g(4)), pt(g(5), g(6)), g(2) / 2.0);
                let l = (b.x - a.x).hypot(b.y - a.y).max(1e-12);
                let (nx, ny) = (-(b.y - a.y) / l * w, (b.x - a.x) / l * w);
                (g(1) != 0.0, vec![vec![pt(a.x + nx, a.y + ny), pt(b.x + nx, b.y + ny), pt(b.x - nx, b.y - ny), pt(a.x - nx, a.y - ny)]], g(7))
            }
            21 => {
                let (w, h, x, y) = (g(2) / 2.0, g(3) / 2.0, g(4), g(5));
                (g(1) != 0.0, vec![vec![pt(x - w, y - h), pt(x + w, y - h), pt(x + w, y + h), pt(x - w, y + h)]], g(6))
            }
            4 => {
                let n = g(2).max(0.0) as usize;
                let pts: Vec<Pt> = (0..n).map(|i| pt(g(3 + 2 * i), g(4 + 2 * i))).collect();
                (g(1) != 0.0, vec![pts], g(3 + 2 * (n + 1)))
            }
            5 => (g(1) != 0.0, vec![ring(pt(g(3), g(4)), g(5) / 2.0, (g(2) as usize).max(3), 0.0)], g(6)),
            7 => {
                // thermal: a ring with two crossing gaps
                let (c0, od, id, gap) = (pt(g(1), g(2)), g(3), g(4), g(5) / 2.0);
                let bar = |horiz: bool| if horiz { vec![pt(c0.x - od, c0.y - gap), pt(c0.x + od, c0.y - gap), pt(c0.x + od, c0.y + gap), pt(c0.x - od, c0.y + gap)] } else { vec![pt(c0.x - gap, c0.y - od), pt(c0.x + gap, c0.y - od), pt(c0.x + gap, c0.y + od), pt(c0.x - gap, c0.y + od)] };
                let rot = g(6);
                let place = |pts: Vec<Pt>| -> Contour { make_place(&pts, rot, c, to_in) };
                steps.push((true, vec![place(ring(c0, od / 2.0, 48, 0.0))]));
                steps.push((false, vec![place(ring(c0, id / 2.0, 48, 0.0)), place(bar(true)), place(bar(false))]));
                continue;
            }
            _ => continue,
        };
        let placed: Vec<Contour> = shapes.into_iter().filter(|p| p.len() >= 3).map(|p| make_place(&p, rot, c, to_in)).collect();
        match steps.last_mut() {
            Some(last) if last.0 == dark => last.1.extend(placed),
            _ => steps.push((dark, placed)),
        }
    }
    paint_layers(&steps)
}

/// Macro units → turned about the macro origin, inches, at the pad's centre.
fn make_place(pts: &[Pt], rot_deg: f64, c: Pt, to_in: f64) -> Contour {
    let (s, co) = rot_deg.to_radians().sin_cos();
    make_polyline(&pts.iter().map(|p| pt(c.x + (p.x * co - p.y * s) * to_in, c.y + (p.x * s + p.y * co) * to_in)).collect::<Vec<_>>(), true)
}

impl Aperture {
    fn width(&self) -> f64 {
        match *self {
            Aperture::Circle(d, _) | Aperture::Poly(d, ..) => d,
            Aperture::Rect(w, h, _) | Aperture::Obround(w, h, _) => w.min(h),
            Aperture::Macro(..) => 0.0,
        }
    }

    /// The pad's outline at `c`.
    fn flash(&self, c: Pt, to_in: f64) -> Vec<Contour> {
        let (shape, hole) = match self.clone() {
            Aperture::Circle(d, h) => (vec![make_polygon(c, d / 2.0, 48, 0.0)], h),
            Aperture::Rect(w, h, hole) => (vec![make_polyline(&[pt(c.x - w / 2.0, c.y - h / 2.0), pt(c.x + w / 2.0, c.y - h / 2.0), pt(c.x + w / 2.0, c.y + h / 2.0), pt(c.x - w / 2.0, c.y + h / 2.0)], true)], hole),
            Aperture::Obround(w, h, hole) => {
                // a stadium: the short side's width swept along the long axis
                let r = w.min(h) / 2.0;
                let (dx, dy) = if w > h { (w / 2.0 - r, 0.0) } else { (0.0, h / 2.0 - r) };
                let line = if dx == 0.0 && dy == 0.0 { make_polygon(c, r, 48, 0.0) } else { make_polyline(&[pt(c.x - dx, c.y - dy), pt(c.x + dx, c.y + dy)], false) };
                (if line.closed { vec![line] } else { offset_contours(&[line], r, true) }, hole)
            }
            // Gerber's first corner sits on +X; make_polygon starts at the top
            Aperture::Poly(d, n, rot, hole) => (vec![make_polygon(c, d / 2.0, n as i32, rot.to_radians() - PI / 2.0)], hole),
            Aperture::Macro(prims, params) => return macro_pad(&prims, &params, c, to_in),
        };
        if hole > 0.0 { paint_layers(&[(true, shape), (false, vec![make_polygon(c, hole / 2.0, 32, 0.0)])]) } else { shape }
    }
}

/// Points along an arc from `a` to `b` round `c` (clockwise when `cw`), `a` left out, `b` included.
fn arc_points(a: Pt, b: Pt, c: Pt, cw: bool) -> Vec<Pt> {
    let r = (a.x - c.x).hypot(a.y - c.y);
    let a0 = (a.y - c.y).atan2(a.x - c.x);
    let mut sweep = (b.y - c.y).atan2(b.x - c.x) - a0;
    if cw {
        while sweep >= -1e-9 {
            sweep -= 2.0 * PI;
        }
    } else {
        while sweep <= 1e-9 {
            sweep += 2.0 * PI;
        }
    }
    // the end on the start = a full circle; otherwise a sweep of a whole turn is rounding
    if (a.x - b.x).hypot(a.y - b.y) > 1e-9 && sweep.abs() > 2.0 * PI - 1e-6 {
        sweep = 0.0;
    }
    let n = ((sweep.abs() * r / 0.002).ceil() as usize).clamp(1, 360);
    (1..=n).map(|i| if i == n { b } else { let t = a0 + sweep * i as f64 / n as f64; pt(c.x + r * t.cos(), c.y + r * t.sin()) }).collect()
}

/// A Gerber file's copper (or mask / silkscreen) as filled outlines; zero-width draws as lines.
pub fn import_gerber(text: &str) -> ImportResult {
    let mut r = ImportResult::default();
    let (mut int_d, mut dec_d, mut trailing) = (2usize, 4usize, false);
    let mut to_in = 1.0;
    let mut aps: HashMap<u32, Aperture> = HashMap::new();
    let mut macros: HashMap<String, Vec<String>> = HashMap::new();
    let mut ap: Option<Aperture> = None;
    let (mut at, mut mode, mut region) = (pt(0.0, 0.0), 1u8, false);
    let mut steps: Vec<(bool, Vec<Contour>)> = vec![(true, Vec::new())];
    let mut lines: Vec<Contour> = Vec::new();
    let mut trace: Vec<Pt> = Vec::new();
    let mut trace_w = 0.0;
    let mut poly: Vec<Pt> = Vec::new();
    let (mut skipped, mut seen) = (0usize, false);

    // a finished run of draws: stroked into the current polarity's shapes, or kept as a line at zero width
    let flush_trace = |trace: &mut Vec<Pt>, w: f64, steps: &mut Vec<(bool, Vec<Contour>)>, lines: &mut Vec<Contour>| {
        if trace.len() >= 2 {
            let c = make_polyline(trace, false);
            if w > 1e-9 {
                steps.last_mut().unwrap().1.extend(offset_contours(&[c], w / 2.0, true));
            } else {
                lines.push(c);
            }
        }
        trace.clear();
    };
    let flush_poly = |poly: &mut Vec<Pt>, steps: &mut Vec<(bool, Vec<Contour>)>| {
        if poly.len() >= 3 {
            steps.last_mut().unwrap().1.push(make_polyline(poly, true));
        }
        poly.clear();
    };

    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i] == b'%' {
            // extended commands: up to the closing %
            let end = text[i + 1..].find('%').map(|k| i + 1 + k).unwrap_or(b.len());
            let block: String = text[i + 1..end].chars().filter(|c| !c.is_whitespace()).collect();
            i = end + 1;
            if let Some(m) = block.strip_prefix("AM") {
                // a macro: its name, then one primitive per *
                let mut parts = m.split('*').filter(|c| !c.is_empty());
                if let Some(name) = parts.next() {
                    macros.insert(name.to_string(), parts.map(str::to_string).collect());
                }
                continue;
            }
            for cmd in block.split('*').filter(|c| !c.is_empty()) {
                if let Some(f) = cmd.strip_prefix("FS") {
                    seen = true;
                    trailing = f.starts_with('T');
                    if let Some(x) = f.find('X') {
                        let d: Vec<usize> = f[x + 1..].chars().take(2).filter_map(|c| c.to_digit(10)).map(|d| d as usize).collect();
                        if d.len() == 2 {
                            (int_d, dec_d) = (d[0], d[1]);
                        }
                    }
                } else if cmd.starts_with("MOMM") {
                    to_in = 1.0 / 25.4;
                } else if cmd.starts_with("MOIN") {
                    to_in = 1.0;
                } else if cmd.starts_with("LPD") || cmd.starts_with("LPC") {
                    flush_trace(&mut trace, trace_w, &mut steps, &mut lines);
                    let dark = cmd.starts_with("LPD");
                    if steps.last().unwrap().0 != dark {
                        steps.push((dark, Vec::new()));
                    }
                } else if let Some(d) = cmd.strip_prefix("ADD") {
                    seen = true;
                    let k = d.find(|c: char| !c.is_ascii_digit()).unwrap_or(d.len());
                    let Ok(code) = d[..k].parse::<u32>() else { continue };
                    let (name, params) = d[k..].split_once(',').unwrap_or((&d[k..], ""));
                    let raw: Vec<f64> = params.split('X').filter_map(|v| v.parse::<f64>().ok()).collect();
                    let p: Vec<f64> = raw.iter().map(|v| v * to_in).collect();
                    let g = |n: usize| p.get(n).copied().unwrap_or(0.0);
                    let a = match name {
                        "C" => Aperture::Circle(g(0), g(1)),
                        "R" => Aperture::Rect(g(0), g(1), g(2)),
                        "O" => Aperture::Obround(g(0), g(1), g(2)),
                        // vertices and rotation are counts / degrees, not lengths
                        "P" => Aperture::Poly(g(0), (g(1) / to_in).round().max(3.0) as usize, g(2) / to_in, g(3)),
                        m => match macros.get(m) {
                            Some(prims) => Aperture::Macro(prims.clone(), raw),
                            None => {
                                skipped += 1;
                                continue;
                            }
                        },
                    };
                    aps.insert(code, a);
                } else if cmd.starts_with("SR") && cmd.len() > 2 {
                    skipped += 1;
                }
                // AM macro bodies, attributes (TF TA TO TD), IP, OF, LN: nothing to draw
            }
            continue;
        }
        // a word command, up to *
        let end = text[i..].find('*').map(|k| i + k).unwrap_or(b.len());
        let cmd: String = text[i..end].chars().filter(|c| !c.is_whitespace()).collect();
        i = end + 1;
        if cmd.is_empty() || cmd.starts_with("G04") {
            continue;
        }
        // the letters and their numbers
        let (mut x, mut y, mut ci, mut cj, mut d) = (None, None, 0.0, 0.0, None);
        let cb = cmd.as_bytes();
        let mut k = 0;
        while k < cb.len() {
            let letter = cb[k];
            if !letter.is_ascii_alphabetic() {
                break;
            }
            let s = k + 1;
            k = s;
            while k < cb.len() && (cb[k].is_ascii_digit() || cb[k] == b'-' || cb[k] == b'+' || cb[k] == b'.') {
                k += 1;
            }
            let num = &cmd[s..k];
            let coord = || -> f64 {
                if num.contains('.') {
                    return num.parse::<f64>().unwrap_or(0.0) * to_in;
                }
                let neg = num.starts_with('-');
                let digits = num.trim_start_matches(['-', '+']);
                let padded = if trailing { format!("{digits:0<width$}", width = int_d + dec_d) } else { digits.to_string() };
                let v = padded.parse::<f64>().unwrap_or(0.0) / 10f64.powi(dec_d as i32);
                if neg { -v * to_in } else { v * to_in }
            };
            match letter {
                b'G' => match num.parse::<u32>().unwrap_or(99) {
                    1 => mode = 1,
                    2 => mode = 2,
                    3 => mode = 3,
                    36 => {
                        flush_trace(&mut trace, trace_w, &mut steps, &mut lines);
                        region = true;
                    }
                    37 => {
                        flush_poly(&mut poly, &mut steps);
                        region = false;
                    }
                    70 => to_in = 1.0,
                    71 => to_in = 1.0 / 25.4,
                    _ => {}
                },
                b'X' => x = Some(coord()),
                b'Y' => y = Some(coord()),
                b'I' => ci = coord(),
                b'J' => cj = coord(),
                b'D' => d = num.parse::<u32>().ok(),
                b'M' => {}
                _ => break,
            }
        }
        let to = pt(x.unwrap_or(at.x), y.unwrap_or(at.y));
        // an old-style file repeats the last operation when only coordinates are given
        let op = match d {
            Some(n) if n >= 10 => {
                flush_trace(&mut trace, trace_w, &mut steps, &mut lines);
                ap = aps.get(&n).cloned();
                at = to;
                continue;
            }
            Some(n) => n,
            None if x.is_some() || y.is_some() => 1,
            None => continue,
        };
        match op {
            1 => {
                let path = if mode == 1 { vec![to] } else { arc_points(at, to, pt(at.x + ci, at.y + cj), mode == 2) };
                if region {
                    if poly.is_empty() {
                        poly.push(at);
                    }
                    poly.extend(path);
                } else {
                    let w = ap.as_ref().map_or(0.0, |a| a.width());
                    if trace.is_empty() || (w - trace_w).abs() > 1e-9 {
                        flush_trace(&mut trace, trace_w, &mut steps, &mut lines);
                        trace_w = w;
                        trace.push(at);
                    }
                    trace.extend(path);
                }
            }
            2 => {
                if region {
                    flush_poly(&mut poly, &mut steps);
                } else {
                    flush_trace(&mut trace, trace_w, &mut steps, &mut lines);
                }
            }
            3 => {
                flush_trace(&mut trace, trace_w, &mut steps, &mut lines);
                match &ap {
                    Some(a) => steps.last_mut().unwrap().1.extend(a.flash(to, to_in)),
                    None => skipped += 1,
                }
            }
            _ => {}
        }
        at = to;
    }
    flush_trace(&mut trace, trace_w, &mut steps, &mut lines);
    flush_poly(&mut poly, &mut steps);
    for c in paint_layers(&steps).into_iter().chain(lines) {
        r.contours.push(c);
        r.layers.push("Gerber".into());
    }
    r.note = if !seen && r.contours.is_empty() {
        "This isn't a Gerber file.".into()
    } else {
        format!("{} outlines from the Gerber file{}", r.contours.len(), if skipped > 0 { format!(", {skipped} unknown pads / step-repeats skipped") } else { String::new() })
    };
    r
}

/// An Excellon drill file: a circle per hole at its tool's size.
pub fn import_excellon(text: &str) -> ImportResult {
    let mut r = ImportResult::default();
    let (mut to_in, mut lz, mut dec_d) = (1.0, false, 4i32);
    let mut tools: HashMap<u32, f64> = HashMap::new();
    let (mut tool, mut at) = (0.0, pt(0.0, 0.0));
    let mut skipped = 0usize;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with("METRIC") || line.starts_with("INCH") {
            let metric = line.starts_with("METRIC");
            (to_in, dec_d) = if metric { (1.0 / 25.4, 3) } else { (1.0, 4) };
            lz = line.contains("LZ");
            // a format like 000.000 gives the decimals
            if let Some(f) = line.split(',').find(|p| p.contains('.') && p.chars().all(|c| c == '0' || c == '.')) {
                dec_d = (f.len() - f.find('.').unwrap() - 1) as i32;
            }
            continue;
        }
        if line == "M71" {
            to_in = 1.0 / 25.4;
        } else if line == "M72" {
            to_in = 1.0;
        }
        let num = |s: &str| -> f64 {
            if s.contains('.') {
                return s.parse::<f64>().unwrap_or(0.0) * to_in;
            }
            let neg = s.starts_with('-');
            let digits = s.trim_start_matches(['-', '+']);
            // leading zeros kept = trailing ones dropped: the digits are read from the left
            let int_d = if to_in < 1.0 { 3 } else { 2 };
            let padded = if lz { format!("{digits:0<width$}", width = int_d + dec_d as usize) } else { digits.to_string() };
            let v = padded.parse::<f64>().unwrap_or(0.0) / 10f64.powi(dec_d) * to_in;
            if neg { -v } else { v }
        };
        if let Some(t) = line.strip_prefix('T') {
            let k = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
            let Ok(n) = t[..k].parse::<u32>() else { continue };
            if let Some(c) = t[k..].find('C') {
                // a size: up to the next letter
                let s = &t[k + c + 1..];
                let e = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
                tools.insert(n, s[..e].parse::<f64>().unwrap_or(0.0) * to_in);
            }
            tool = tools.get(&n).copied().unwrap_or(0.0);
            continue;
        }
        if line.starts_with('X') || line.starts_with('Y') {
            if line.contains("G85") {
                skipped += 1;
                continue;
            }
            let (mut x, mut y) = (at.x, at.y);
            let mut rest = line;
            while let Some(c) = rest.chars().next() {
                let e = rest[1..].find(|c: char| c.is_ascii_alphabetic()).map(|k| k + 1).unwrap_or(rest.len());
                match c {
                    'X' => x = num(&rest[1..e]),
                    'Y' => y = num(&rest[1..e]),
                    _ => {}
                }
                rest = &rest[e..];
            }
            at = pt(x, y);
            let d = if tool > 0.0 { tool } else { 0.04 };
            r.contours.push(make_ellipse(at.x, at.y, d / 2.0, d / 2.0));
            r.layers.push("Drill".into());
        }
    }
    r.note = if r.contours.is_empty() { "No holes in the drill file.".into() } else { format!("{} holes from the drill file{}", r.contours.len(), if skipped > 0 { format!(", {skipped} routed slots skipped") } else { String::new() }) };
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{bounds, flatten};

    fn area(c: &Contour) -> f64 {
        let p = flatten(c, 0.0005);
        (0..p.len()).map(|i| { let (a, b) = (p[i], p[(i + 1) % p.len()]); a.x * b.y - b.x * a.y }).sum::<f64>() / 2.0
    }

    #[test]
    fn traces_pads_regions_and_clear_polarity() {
        // mm, 2.4 digits: a 1 mm trace 10 mm long between two 2 mm round pads, a 4 × 4 region with a 2 × 2 hole
        // scraped out by clear polarity, and a zero-width outline
        let g = "%FSLAX24Y24*%\n%MOMM*%\n%ADD10C,1.0*%\n%ADD11C,2.0*%\n%ADD12C,0*%\nD10*\nX0Y0D02*\nX100000Y0D01*\nD11*\nX0Y0D03*\nX100000Y0D03*\n\
                 G36*\nX200000Y0D02*\nG01X240000Y0D01*\nX240000Y40000D01*\nX200000Y40000D01*\nX200000Y0D01*\nG37*\n\
                 %LPC*%\nG36*\nX210000Y10000D02*\nX230000Y10000D01*\nX230000Y30000D01*\nX210000Y30000D01*\nX210000Y10000D01*\nG37*\n%LPD*%\n\
                 D12*\nX0Y-50000D02*\nX50000Y-50000D01*\nM02*\n";
        let r = import_gerber(g);
        let mm2 = |a: f64| a.abs() * 25.4 * 25.4;
        let filled: Vec<&Contour> = r.contours.iter().filter(|c| c.closed).collect();
        let lines: Vec<&Contour> = r.contours.iter().filter(|c| !c.closed).collect();
        assert_eq!(lines.len(), 1, "{}", r.note);
        // the trace and pads merged: 10 × 1 plus two 2 mm discs minus the overlap ≈ 10 + 2 × π − 2 × (half discs of the trace's caps)
        let trace = filled.iter().find(|c| bounds(c).x1 < 0.6).unwrap();
        let a = mm2(area(trace));
        assert!(a > 12.0 && a < 14.5, "trace + pads {a}");
        // region minus its hole: 16 − 4 = 12, as an outer and a hole
        let sq: f64 = filled.iter().filter(|c| bounds(c).x0 > 0.7).map(|c| area(c)).sum();
        assert!((mm2(sq) - 12.0).abs() < 0.05, "region {}", mm2(sq));
        assert!(import_gerber("hello").note.contains("isn't"));
        // garbage (cut-up files, other text) never panics
        for cut in (0..g.len()).step_by(7) {
            import_gerber(&g[..cut]);
            import_excellon(&g[cut..]);
        }
        import_gerber("%ADDé,1*%X1é2D01*G02X5Y5I9J9D01*%FS");
        import_excellon("T1Cé\nXéY1\nX-\nT");
    }

    #[test]
    fn macro_pads_octagons_and_thermals() {
        // Eagle's OC8 octagon (a polygon primitive with a $1 expression) and a thermal
        let g = "%FSLAX24Y24*%\n%MOIN*%\n%AMOC8*\n5,1,8,0,0,1.08239X$1,22.5*\n%\n%AMTH*7,0,0,0.1,0.06,0.02,0*%\n%ADD12OC8,0.1*%\n%ADD13TH*%\nD12*\nX10000Y10000D03*\nD13*\nX30000Y10000D03*\nM02*\n";
        let r = import_gerber(g);
        let oct = r.contours.iter().find(|c| bounds(c).x1 < 1.5).unwrap();
        assert_eq!(oct.nodes.len(), 8, "{}", r.note);
        // 1.08239 × 0.1 across the corners = 0.1 across the flats
        let b = bounds(oct);
        assert!(((b.x1 - b.x0) - 0.1).abs() < 1e-4 && ((b.x0 + b.x1) / 2.0 - 1.0).abs() < 1e-6, "{b:?}");
        // the thermal: four quarter-ring pieces
        let th: Vec<&Contour> = r.contours.iter().filter(|c| bounds(c).x0 > 2.5).collect();
        assert_eq!(th.len(), 4);
        let a: f64 = th.iter().map(|c| area(c).abs()).sum();
        let ring = PI * (0.05f64.powi(2) - 0.03f64.powi(2)) - 2.0 * 0.02 * 0.04;
        assert!((a - ring).abs() < ring * 0.03, "{a} vs {ring}");
    }

    #[test]
    fn arcs_and_trailing_zero_coordinates() {
        // inch, trailing zeros left out: X015 = 01.5000; a counter-clockwise full circle of radius 0.5 drawn at zero width
        let g = "%FSTAX24Y24*%\n%MOIN*%\n%ADD10C,0*%\nD10*\nG75*\nX015Y01D02*\nG03X015Y01I-005J0D01*\nM02*\n";
        let r = import_gerber(g);
        let b = bounds(&r.contours[0]);
        assert!((b.x0 - 0.5).abs() < 1e-3 && (b.x1 - 1.5).abs() < 1e-3 && (b.y1 - 1.5).abs() < 1e-3, "{b:?}");
    }

    #[test]
    fn drill_hits_become_circles_at_their_tool_size() {
        let d = "M48\nMETRIC,TZ\nT1C0.800\nT2C3.000\n%\nT1\nX10.0Y5.0\nX20.0\nT2\nX0Y0\nM30\n";
        let r = import_excellon(d);
        assert_eq!(r.contours.len(), 3);
        let b = bounds(&r.contours[1]);
        assert!(((b.x0 + b.x1) / 2.0 * 25.4 - 20.0).abs() < 1e-6 && ((b.x1 - b.x0) * 25.4 - 0.8).abs() < 1e-6);
        assert!(((bounds(&r.contours[2]).x1 - bounds(&r.contours[2]).x0) * 25.4 - 3.0).abs() < 1e-6);
        // no decimal points, inch, trailing zeros kept: 2.4 → X015000 = 1.5"
        let r = import_excellon("M48\nINCH,TZ\nT1C0.0320\n%\nT1\nX15000Y5000\nM30\n");
        let b = bounds(&r.contours[0]);
        assert!(((b.x0 + b.x1) / 2.0 - 1.5).abs() < 1e-6, "{b:?}");
    }
}
