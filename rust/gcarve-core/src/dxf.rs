//! ASCII DXF importer (C++ core/dxf.cpp): LINE, ARC, CIRCLE, LWPOLYLINE, POLYLINE/VERTEX (with bulges); Rust only on
//! top of the C++ list: ELLIPSE (also arcs of one), SPLINE (B-spline / NURBS from knots + control points, else the fit
//! points) and INSERT of BLOCKS (offset, scale, rotation, nested). Loose segments are joined. Skipped (reported in the
//! note): TEXT, HATCH, DIMENSION, ...

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::PI;

use crate::geom::{dist, join_contours_tagged, lerp, make_ellipse, make_polyline, pt, transformed, Affine, Builder, Contour, Pt};
use crate::svg::{atof, ImportResult, ImportText};

type Entity = Vec<(i32, String)>; // group-code pairs after the code-0 line

/// C `atoi`: integer prefix, 0 when none.
fn atoi(s: &str) -> i32 {
    let t = s.trim_start();
    let end = t.char_indices().find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')))).map_or(t.len(), |(i, _)| i);
    t[..end].parse().unwrap_or(0)
}

fn get(e: &Entity, code: i32, def: f64) -> f64 {
    // numbers from a damaged file: inf / NaN count as missing and anything absurd is clamped (the loops and the
    // flattening below must stay bounded)
    e.iter().find(|p| p.0 == code).map_or(def, |p| {
        let v = atof(&p.1);
        if v.is_finite() { v.clamp(-1e9, 1e9) } else { def }
    })
}

struct Vtx {
    p: Pt,
    bulge: f64,
}

pub(crate) fn bulge_arc(b: &mut Builder, p1: Pt, p2: Pt, bulge: f64) {
    let th = 4.0 * bulge.atan();
    let c = dist(p1, p2);
    let m = lerp(p1, p2, 0.5);
    let n = pt(-(p2.y - p1.y) / c, (p2.x - p1.x) / c);
    let r = c / (2.0 * (th.abs() / 2.0).sin());
    let ctr = m + n * (c / 2.0 / (th / 2.0).tan());
    b.arc(ctr.x, ctr.y, r, r, 0.0, (p1.y - ctr.y).atan2(p1.x - ctr.x), th);
    if let Some(last) = b.c.nodes.last_mut() {
        last.p = p2;
    }
}

fn polyline(v: &[Vtx], closed: bool) -> Contour {
    let mut b = Builder::default();
    b.start(v[0].p);
    let segs = if closed { v.len() } else { v.len() - 1 };
    for i in 0..segs {
        let (p1, p2) = (v[i].p, v[(i + 1) % v.len()].p);
        if v[i].bulge.abs() < 1e-9 || dist(p1, p2) < 1e-12 {
            b.line(p2);
        } else {
            bulge_arc(&mut b, p1, p2, v[i].bulge);
        }
    }
    if closed {
        b.close();
    } else {
        b.c.closed = false;
    }
    b.c
}

pub fn import_dxf(text: &str) -> ImportResult {
    import(text, false)
}

/// As `import_dxf`, with TEXT / MTEXT returned in `texts` (position, height, justification; fonts and rotation
/// are not read) instead of counted as skipped.
pub fn import_dxf_with_text(text: &str) -> ImportResult {
    import(text, true)
}

/// MTEXT content without its inline formatting codes: \\P = new line, \\~ = space, {\\fArial|b0;…} and friends dropped.
fn mtext_plain(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' | '}' => {}
            '\\' => match it.next() {
                Some('P') => out.push('\n'),
                Some('~') => out.push(' '),
                Some(c @ ('\\' | '{' | '}')) => out.push(c),
                Some('f' | 'F' | 'H' | 'C' | 'c' | 'T' | 'Q' | 'W' | 'A' | 'p' | 'S') => {
                    for d in it.by_ref() {
                        if d == ';' {
                            break;
                        }
                    }
                }
                _ => {} // \L \l \O \o \K \k: underline and the like
            },
            c => out.push(c),
        }
    }
    out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn import(text: &str, keep_text: bool) -> ImportResult {
    // 1. pairs (old Mac files end lines with a lone CR: lines() only splits on LF)
    let text = if text.contains('\r') { std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n")) } else { std::borrow::Cow::Borrowed(text) };
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let pairs: Vec<(i32, String)> = lines.as_chunks::<2>().0.iter().map(|l| (atoi(l[0]), l[1].to_string())).collect();

    // 2. header + entity list
    let (mut ins_units, mut measurement) = (0, 0);
    let mut ents: Vec<(String, Entity)> = Vec::new();
    let mut blocks: HashMap<String, Block> = HashMap::new();
    let mut cur_block: Option<(String, Block)> = None;
    let mut section = String::new();
    let mut i = 0;
    while i < pairs.len() {
        let p = &pairs[i];
        if p.0 == 0 && p.1 == "SECTION" && i + 1 < pairs.len() {
            i += 1;
            section = pairs[i].1.clone();
        } else if p.0 == 0 && p.1 == "ENDSEC" {
            section.clear();
        } else if section == "HEADER" && p.0 == 9 && i + 1 < pairs.len() {
            if p.1 == "$INSUNITS" {
                ins_units = atoi(&pairs[i + 1].1);
            }
            if p.1 == "$MEASUREMENT" {
                measurement = atoi(&pairs[i + 1].1);
            }
        } else if section == "ENTITIES" && p.0 == 0 {
            ents.push((p.1.clone(), Vec::new()));
        } else if section == "ENTITIES" && !ents.is_empty() {
            ents.last_mut().unwrap().1.push(p.clone());
        } else if section == "BLOCKS" && p.0 == 0 {
            // BLOCK ... entities ... ENDBLK: the block's own entities collect under its name
            match p.1.as_str() {
                "BLOCK" => cur_block = Some((String::new(), Block { base: pt(0.0, 0.0), ents: Vec::new() })),
                "ENDBLK" => {
                    if let Some((name, b)) = cur_block.take() {
                        blocks.entry(name).or_insert(b);
                    }
                }
                other => {
                    if let Some((_, b)) = cur_block.as_mut() {
                        b.ents.push((other.to_string(), Vec::new()));
                    }
                }
            }
        } else if section == "BLOCKS" {
            if let Some((name, b)) = cur_block.as_mut() {
                match b.ents.last_mut() {
                    Some(ent) => ent.1.push(p.clone()),
                    None => match p.0 {
                        2 => *name = p.1.clone(), // the BLOCK record itself: name and base point
                        10 => b.base.x = atof(&p.1),
                        20 => b.base.y = atof(&p.1),
                        _ => {}
                    },
                }
            }
        }
        i += 1;
    }

    // 3. units -> inches
    let mut assumed = false;
    let (k, unit_name) = match ins_units {
        1 => (1.0, "inches"),
        2 => (12.0, "feet"),
        4 => (1.0 / 25.4, "mm"),
        5 => (1.0 / 2.54, "cm"),
        6 => (39.3700787, "m"),
        _ => {
            assumed = true;
            if measurement == 1 {
                (1.0 / 25.4, "mm")
            } else {
                (1.0, "inches")
            }
        }
    };

    // 4. entities (INSERTs expand the BLOCKS they name)
    let run = |k: f64| {
        let mut out: Vec<(Contour, String)> = Vec::new();
        let mut rest = Rest { skipped: BTreeMap::new(), hatches: Vec::new(), used_hatch: false, texts: keep_text.then(Vec::new) };
        convert(&ents, &blocks, Affine::scale(k, k), "", 0, &mut out, &mut rest);
        if out.is_empty() && !rest.hatches.is_empty() {
            out = std::mem::take(&mut rest.hatches); // a file of fills only: their boundaries are the shapes
            rest.skipped.remove("HATCH");
            rest.used_hatch = true;
        }
        (out, rest)
    };
    let (mut out, mut rest) = run(k);
    let (mut unit_name, mut why) = (unit_name, "");
    // no unit in the file and the drawing is wider than any router table in inches (a 600 mm part reads as 600 in):
    // it is millimetres
    if assumed && k == 1.0 && out.iter().map(|(c, _)| crate::geom::bounds(c)).any(|b| (b.x1 - b.x0).max(b.y1 - b.y0) > 150.0) {
        (out, rest) = run(1.0 / 25.4);
        (unit_name, why) = ("mm", ": it is too big for inches");
    }
    let Rest { skipped, texts, used_hatch, .. } = rest;
    let (contours, layers): (Vec<Contour>, Vec<String>) = out.into_iter().unzip();
    let (contours, layers) = join_contours_tagged(contours, layers, 5e-4).into_iter().unzip();

    let mut note = format!("DXF in {unit_name}{}", if assumed { format!(" (assumed - no units in file{why})") } else { String::new() });
    if used_hatch {
        note += "; the file has only filled areas (HATCH): their outlines were used";
    }
    if !skipped.is_empty() {
        note += "; skipped";
        for (t, n) in &skipped {
            note += &format!(" {n} {t}");
        }
    }
    ImportResult { contours, layers, note, texts: texts.unwrap_or_default(), images: Vec::new() }
}

/// Boundary loops of a HATCH (closed polylines, and chains of line / arc edges), in the entity's own coordinates.
fn hatch_loops(e: &Entity) -> Vec<Contour> {
    let mut loops = Vec::new();
    let mut i = 0;
    while i < e.len() {
        if e[i].0 != 92 {
            i += 1;
            continue;
        }
        let flags = atoi(&e[i].1);
        i += 1;
        if flags & 2 != 0 {
            // polyline path: 72 has bulge, 73 closed, 93 vertex count, then 10 / 20 (/ 42) per vertex
            let mut n = 0;
            while i < e.len() && e[i].0 != 10 && e[i].0 != 92 {
                if e[i].0 == 93 {
                    n = atoi(&e[i].1);
                }
                i += 1;
            }
            let mut v: Vec<Vtx> = Vec::new();
            while i < e.len() {
                let full = v.len() as i32 >= n;
                match (e[i].0, v.last_mut()) {
                    (10, _) if !full => v.push(Vtx { p: pt(atof(&e[i].1), 0.0), bulge: 0.0 }),
                    (20, Some(last)) => last.p.y = atof(&e[i].1),
                    (42, Some(last)) => last.bulge = atof(&e[i].1),
                    _ => break,
                }
                i += 1;
            }
            if v.len() >= 3 {
                loops.push(polyline(&v, true));
            }
        } else {
            // edge path: 93 edge count, each edge 72 = 1 line (10 20 11 21) / 2 arc (10 20 40 50 51 73)
            let mut n = 0;
            while i < e.len() && e[i].0 != 72 && e[i].0 != 92 {
                if e[i].0 == 93 {
                    n = atoi(&e[i].1);
                }
                i += 1;
            }
            let mut b = Builder::default();
            let (mut have, mut ok) = (false, true);
            for _ in 0..n {
                if i >= e.len() || e[i].0 != 72 {
                    break;
                }
                let kind = atoi(&e[i].1);
                i += 1;
                let mut f = std::collections::HashMap::new();
                while i < e.len() && e[i].0 != 72 && e[i].0 != 92 && e[i].0 != 97 {
                    f.insert(e[i].0, atof(&e[i].1));
                    i += 1;
                }
                let g = |c: i32| f.get(&c).copied().unwrap_or(0.0);
                match kind {
                    1 => {
                        if !have {
                            b.start(pt(g(10), g(20)));
                            have = true;
                        }
                        b.line(pt(g(11), g(21)));
                    }
                    2 => {
                        let (r, a1, a2) = (g(40), g(50).to_radians(), g(51).to_radians());
                        let sweep = if g(73) != 0.0 { (a2 - a1).rem_euclid(2.0 * PI) } else { -(a1 - a2).rem_euclid(2.0 * PI) };
                        let sweep = if sweep.abs() < 1e-9 { 2.0 * PI } else { sweep };
                        if !have {
                            b.start(pt(g(10) + r * a1.cos(), g(20) + r * a1.sin()));
                            have = true;
                        }
                        b.arc(g(10), g(20), r, r, 0.0, a1, sweep);
                    }
                    _ => ok = false, // ellipse / spline edges: not read
                }
            }
            if ok && have && b.c.nodes.len() >= 3 {
                b.close();
                loops.push(b.c);
            }
        }
    }
    loops
}

struct Block {
    base: Pt,
    ents: Vec<(String, Entity)>,
}

/// Entities -> contours (document inches after `xf`), each with its layer name ("" = the default layer; "0" inside
/// a block takes the INSERT's layer).
/// What `convert` gathers besides contours: counts of entity kinds it can't draw, and text when it is wanted.
struct Rest {
    skipped: BTreeMap<String, i32>,
    /// HATCH boundaries: only used when the file has no other geometry (they usually repeat the outlines)
    hatches: Vec<(Contour, String)>,
    used_hatch: bool,
    texts: Option<Vec<ImportText>>,
}

fn convert(ents: &[(String, Entity)], blocks: &HashMap<String, Block>, xf: Affine, inherit: &str, depth: u32, out: &mut Vec<(Contour, String)>, rest: &mut Rest) {
    let push = |out: &mut Vec<(Contour, String)>, mut c: Contour, e: &Entity| {
        if get(e, 230, 1.0) < 0.0 {
            c = transformed(&c, &Affine::scale(-1.0, 1.0)); // extrusion (0,0,-1): OCS x is mirrored
        }
        let own = e.iter().rev().find(|p| p.0 == 8).map_or("", |p| p.1.as_str());
        let layer = if own == "0" || own.is_empty() { inherit.to_string() } else { own.to_string() };
        out.push((transformed(&c, &xf), layer));
    };
    let mut i = 0;
    while i < ents.len() {
        let (kind, e) = (&ents[i].0, &ents[i].1);
        match kind.as_str() {
            "LINE" => push(out, make_polyline(&[pt(get(e, 10, 0.0), get(e, 20, 0.0)), pt(get(e, 11, 0.0), get(e, 21, 0.0))], false), e),
            "CIRCLE" => push(out, make_ellipse(get(e, 10, 0.0), get(e, 20, 0.0), get(e, 40, 0.0), get(e, 40, 0.0)), e),
            "ARC" => {
                let (cx, cy, r) = (get(e, 10, 0.0), get(e, 20, 0.0), get(e, 40, 0.0));
                let (a1, a2) = (get(e, 50, 0.0) * PI / 180.0, get(e, 51, 0.0) * PI / 180.0);
                let mut sweep = a2 - a1;
                if sweep <= 0.0 {
                    sweep = match sweep.rem_euclid(2.0 * PI) {
                        0.0 => 2.0 * PI,
                        r => r,
                    };
                }
                let mut b = Builder::default();
                b.start(pt(cx + r * a1.cos(), cy + r * a1.sin()));
                b.arc(cx, cy, r, r, 0.0, a1, sweep);
                push(out, b.c, e);
            }
            "ELLIPSE" => {
                let (cx, cy, mx, my) = (get(e, 10, 0.0), get(e, 20, 0.0), get(e, 11, 0.0), get(e, 21, 0.0));
                let rx = mx.hypot(my);
                if rx > 0.0 {
                    let ry = rx * get(e, 40, 1.0).clamp(1e-6, 1.0e6);
                    let (t0, t1) = (get(e, 41, 0.0), get(e, 42, 2.0 * PI));
                    let mut sweep = (t1 - t0).rem_euclid(2.0 * PI);
                    if sweep < 1e-9 {
                        sweep = 2.0 * PI;
                    }
                    let phi = my.atan2(mx);
                    let (sp, cp) = phi.sin_cos();
                    let (x, y) = (rx * t0.cos(), ry * t0.sin());
                    let mut b = Builder::default();
                    b.start(pt(cx + cp * x - sp * y, cy + sp * x + cp * y));
                    b.arc(cx, cy, rx, ry, phi, t0, sweep);
                    if sweep >= 2.0 * PI - 1e-6 {
                        b.close();
                    }
                    push(out, b.c, e);
                }
            }
            "SPLINE" => match spline_points(e) {
                Some((pts, closed)) => push(out, make_polyline(&pts, closed), e),
                None => *rest.skipped.entry(kind.clone()).or_default() += 1,
            },
            "INSERT" => {
                let name = e.iter().find(|p| p.0 == 2).map(|p| p.1.as_str()).unwrap_or("");
                match blocks.get(name) {
                    Some(b) if depth < 6 => {
                        let (sx, sy) = (get(e, 41, 1.0), get(e, 42, 1.0));
                        let m = xf * Affine::translate(get(e, 10, 0.0), get(e, 20, 0.0)) * Affine::rotate(get(e, 50, 0.0).to_radians()) * Affine::scale(sx, sy) * Affine::translate(-b.base.x, -b.base.y);
                        let own = e.iter().rev().find(|p| p.0 == 8).map_or("", |p| p.1.as_str());
                        let layer = if own == "0" || own.is_empty() { inherit.to_string() } else { own.to_string() };
                        convert(&b.ents, blocks, m, &layer, depth + 1, out, rest);
                    }
                    _ => *rest.skipped.entry(kind.clone()).or_default() += 1,
                }
            }
            "LWPOLYLINE" => {
                let mut v: Vec<Vtx> = Vec::new();
                for p in e {
                    let d = atof(&p.1);
                    match (p.0, v.last_mut()) {
                        (10, _) => v.push(Vtx { p: pt(d, 0.0), bulge: 0.0 }),
                        (20, Some(last)) => last.p.y = d,
                        (42, Some(last)) => last.bulge = d,
                        _ => {}
                    }
                }
                if v.len() >= 2 {
                    push(out, polyline(&v, get(e, 70, 0.0) as i32 & 1 != 0), e);
                }
            }
            "POLYLINE" => {
                let mut v = Vec::new();
                let mut j = i + 1;
                while j < ents.len() && ents[j].0 != "SEQEND" {
                    if ents[j].0 == "VERTEX" {
                        let ve = &ents[j].1;
                        v.push(Vtx { p: pt(get(ve, 10, 0.0), get(ve, 20, 0.0)), bulge: get(ve, 42, 0.0) });
                    }
                    j += 1;
                }
                if v.len() >= 2 {
                    push(out, polyline(&v, get(e, 70, 0.0) as i32 & 1 != 0), e);
                }
                i = j;
            }
            "HATCH" => {
                for c in hatch_loops(e) {
                    push(&mut rest.hatches, c, e);
                }
                *rest.skipped.entry(kind.clone()).or_default() += 1; // dropped from the note again when its boundaries are used
            }
            "VERTEX" | "SEQEND" => {}
            "TEXT" | "MTEXT" if rest.texts.is_some() => {
                let mtext = kind == "MTEXT";
                let raw: String = e.iter().filter(|p| p.0 == 1 || (mtext && p.0 == 3)).map(|p| p.1.as_str()).collect();
                let content = if mtext { mtext_plain(&raw) } else { raw.trim().to_string() };
                let height = get(e, 40, 0.0) * (xf.a * xf.d - xf.b * xf.c).abs().sqrt();
                if content.is_empty() || height <= 0.0 {
                    *rest.skipped.entry(kind.clone()).or_default() += 1;
                } else {
                    // TEXT: justified text sits at its alignment point (11, 21); MTEXT: 71 = 1..9, top-left first
                    let (anchor, valign, code) = if mtext {
                        let a = (get(e, 71, 1.0) as i32 - 1).clamp(0, 8);
                        (a % 3, [1, 2, 0][(a / 3) as usize], 10)
                    } else {
                        let h = get(e, 72, 0.0) as i32;
                        let v = get(e, 73, 0.0) as i32;
                        (if (1..=2).contains(&h) { h } else { 0 }, [0, 0, 2, 1][v.clamp(0, 3) as usize], if h != 0 || v != 0 { 11 } else { 10 })
                    };
                    let own = e.iter().rev().find(|p| p.0 == 8).map_or("", |p| p.1.as_str());
                    let layer = if own == "0" || own.is_empty() { inherit.to_string() } else { own.to_string() };
                    let at = xf.apply(pt(get(e, code, 0.0), get(e, code + 10, 0.0)));
                    // TEXT turns by degrees (50); MTEXT by its direction vector (11, 21) or radians (50)
                    let own_angle = if !mtext {
                        get(e, 50, 0.0).to_radians()
                    } else if e.iter().any(|p| p.0 == 11) {
                        get(e, 21, 0.0).atan2(get(e, 11, 1.0))
                    } else {
                        get(e, 50, 0.0)
                    };
                    let angle = (own_angle + xf.b.atan2(xf.a)).to_degrees();
                    rest.texts.as_mut().unwrap().push(ImportText { text: content, at, height, layer, anchor, valign, angle });
                }
            }
            _ => *rest.skipped.entry(kind.clone()).or_default() += 1,
        }
        i += 1;
    }
}

/// A SPLINE as a polyline: the B-spline / NURBS evaluated from its knots and control points, else the fit points.
fn spline_points(e: &Entity) -> Option<(Vec<Pt>, bool)> {
    let all = |code: i32| -> Vec<f64> { e.iter().filter(|p| p.0 == code).map(|p| atof(&p.1)).filter(|v| v.is_finite()).collect() };
    let pairs = |cx: i32, cy: i32| -> Vec<Pt> {
        let mut v: Vec<Pt> = Vec::new();
        for p in e {
            let d = atof(&p.1);
            if p.0 == cx {
                v.push(pt(d, 0.0));
            } else if p.0 == cy {
                if let Some(l) = v.last_mut() {
                    l.y = d;
                }
            }
        }
        v
    };
    let closed = get(e, 70, 0.0) as i32 & 1 != 0;
    let degree = get(e, 71, 3.0) as usize;
    let (knots, ctrl, w) = (all(40), pairs(10, 20), all(41));
    if (1..=10).contains(&degree) && ctrl.len() > degree && knots.len() == ctrl.len() + degree + 1 && knots.windows(2).all(|k| k[0] <= k[1]) {
        let n = ctrl.len();
        let (u0, u1) = (knots[degree], knots[n]);
        if u1 > u0 {
            let weight = |i: usize| if w.len() == n && w[i] > 0.0 { w[i] } else { 1.0 };
            let samples = (n * 12).clamp(24, 4000);
            let mut out = Vec::with_capacity(samples + 1);
            for s in 0..=samples {
                let u = u0 + (u1 - u0) * s as f64 / samples as f64;
                // span: knots[k] <= u < knots[k + 1], the last one for u = u1
                let k = (degree..n).rev().find(|&k| knots[k] <= u && knots[k] < knots[k + 1]).unwrap_or(n - 1);
                // de Boor on homogeneous points (x w, y w, w)
                let mut d: Vec<[f64; 3]> = (0..=degree).map(|j| { let i = k + j - degree; let wi = weight(i); [ctrl[i].x * wi, ctrl[i].y * wi, wi] }).collect();
                for r in 1..=degree {
                    for j in (r..=degree).rev() {
                        let i = k + j - degree;
                        let den = knots[i + degree + 1 - r] - knots[i];
                        let a = if den.abs() < 1e-12 { 0.0 } else { (u - knots[i]) / den };
                        let (p, q) = (d[j - 1], d[j]);
                        d[j] = std::array::from_fn(|c| (1.0 - a) * p[c] + a * q[c]);
                    }
                }
                let h = d[degree];
                if h[2].abs() > 1e-12 {
                    out.push(pt(h[0] / h[2], h[1] / h[2]));
                }
            }
            if out.len() >= 2 {
                return Some((out, closed));
            }
        }
    }
    let fit = pairs(11, 21);
    if fit.len() >= 2 {
        return Some((fit, closed));
    }
    (ctrl.len() >= 2).then_some((ctrl, closed)) // the control polygon: rough, but the right place and size
}

/// Vectors as an R12 DXF in inches (POLYLINE / VERTEX: read by every CAD and CAM program), curves flattened to
/// 0.001 in, the layer name on each entity.
pub fn export_dxf(items: &[(&Contour, &str)]) -> String {
    let mut out = String::from("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n1\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n");
    for (c, layer) in items {
        let mut p = crate::geom::flatten(c, 0.001);
        if c.closed && p.len() > 2 && dist(p[0], p[p.len() - 1]) < 1e-9 {
            p.pop(); // the closed flag repeats the start
        }
        if p.len() < 2 {
            continue;
        }
        let layer: String = if layer.is_empty() { "0".into() } else { layer.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect() };
        out += &format!("0\nPOLYLINE\n8\n{layer}\n66\n1\n70\n{}\n", i32::from(c.closed));
        for q in p {
            out += &format!("0\nVERTEX\n8\n{layer}\n10\n{:.5}\n20\n{:.5}\n", q.x, q.y);
        }
        out += &format!("0\nSEQEND\n8\n{layer}\n");
    }
    out + "0\nENDSEC\n0\nEOF\n"
}

#[cfg(test)]
mod curve_tests {
    use super::*;
    use crate::geom::bounds;

    #[test]
    fn text_and_mtext_are_kept_when_asked() {
        let src = dxf("0\nTEXT\n8\nLabels\n10\n2\n20\n3\n40\n0.5\n50\n90\n1\nShop\n0\nMTEXT\n10\n1\n20\n9\n40\n0.25\n71\n5\n1\n{\\fArial|b0;Line one}\\PLine two\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n", "");
        let plain = import_dxf(&src);
        assert!(plain.texts.is_empty() && plain.note.contains("1 MTEXT 1 TEXT"), "{}", plain.note);
        let r = import_dxf_with_text(&src);
        assert_eq!(r.contours.len(), 1);
        assert!(!r.note.contains("TEXT"), "{}", r.note);
        assert_eq!(r.texts[0], ImportText { text: "Shop".into(), at: pt(2.0, 3.0), height: 0.5, layer: "Labels".into(), anchor: 0, valign: 0, angle: 90.0 });
        assert_eq!(r.texts[1], ImportText { text: "Line one\nLine two".into(), at: pt(1.0, 9.0), height: 0.25, layer: String::new(), anchor: 1, valign: 2, angle: 0.0 });
    }

    fn dxf(entities: &str, blocks: &str) -> String {
        format!("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n1\n0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n{blocks}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n")
    }

    #[test]
    fn a_unitless_drawing_too_big_for_inches_is_millimetres() {
        let dxf = |x: f64| format!("0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nA\n10\n0\n20\n0\n11\n{x}\n21\n0\n0\nENDSEC\n0\nEOF\n");
        let w = |t: String| { let r = import_dxf(&t); let b = crate::geom::bounds(&r.contours[0]); (b.x1 - b.x0, r.note) };
        assert!((w(dxf(600.0)).0 - 600.0 / 25.4).abs() < 1e-9);
        assert!(w(dxf(600.0)).1.contains("too big for inches"));
        assert_eq!(w(dxf(40.0)).0, 40.0); // a plausible size stays inches
    }

    #[test]
    fn a_file_of_hatches_only_uses_their_outlines() {
        // one polyline loop (0..2 square) and one edge loop (line, arc, line: a half disc)
        let hatch = "0\nHATCH\n8\nFill\n91\n2\n92\n7\n72\n0\n73\n1\n93\n4\n10\n0\n20\n0\n10\n2\n20\n0\n10\n2\n20\n2\n10\n0\n20\n2\n97\n0\n\
92\n1\n93\n3\n72\n1\n10\n10\n20\n0\n11\n14\n21\n0\n72\n2\n10\n12\n20\n0\n40\n2\n50\n0\n51\n180\n73\n1\n72\n1\n10\n10\n20\n0\n11\n10\n21\n0\n97\n0\n";
        let r = import_dxf(&format!("0\nSECTION\n2\nENTITIES\n{hatch}0\nENDSEC\n0\nEOF\n"));
        assert_eq!(r.contours.len(), 2, "{}", r.note);
        assert!(r.layers.iter().all(|l| l == "Fill") && r.note.contains("only filled areas") && !r.note.contains("skipped"), "{}", r.note);
        // with outlines present the hatch is still just skipped (it would double every cut)
        let r = import_dxf(&format!("0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nA\n10\n0\n20\n0\n11\n1\n21\n1\n{hatch}0\nENDSEC\n0\nEOF\n"));
        assert_eq!(r.contours.len(), 1);
        assert!(r.note.contains("skipped 1 HATCH"));
    }

    #[test]
    fn lone_cr_line_endings_read_like_lf() {
        let lf = "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nA\n10\n0\n20\n0\n11\n3\n21\n4\n0\nENDSEC\n0\nEOF\n";
        let (cr, crlf) = (lf.replace('\n', "\r"), lf.replace('\n', "\r\n"));
        for t in [&cr, &crlf] {
            let r = import_dxf(t);
            assert_eq!((r.contours.len(), r.layers[0].as_str()), (1, "A"), "{r:?}");
        }
        assert_eq!(import_dxf(&cr).contours, import_dxf(lf).contours);
    }

    #[test]
    fn ellipse_spline_and_inserted_blocks() {
        // inches. Ellipse: centre (5, 5), major axis (2, 0) along x, ratio 0.5: 4 x 2 across.
        // Quadratic spline through (0,0) (1,2) (2,0): the curve peaks at y = 1 in the middle.
        // Block "B": a square 0..1 with base point (0.5, 0.5), inserted at (10, 10) turned 90 degrees, scaled 2, on layer "L".
        let blocks = "0\nBLOCK\n2\nB\n10\n0.5\n20\n0.5\n0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n10\n0\n20\n0\n10\n1\n20\n0\n10\n1\n20\n1\n10\n0\n20\n1\n0\nENDBLK\n";
        let ents = "0\nELLIPSE\n10\n5\n20\n5\n11\n2\n21\n0\n40\n0.5\n41\n0\n42\n6.283185307\n\
                    0\nSPLINE\n70\n8\n71\n2\n72\n6\n73\n3\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n2\n10\n2\n20\n0\n\
                    0\nINSERT\n2\nB\n8\nL\n10\n10\n20\n10\n41\n2\n42\n2\n50\n90\n0\nINSERT\n2\nNOPE\n10\n0\n20\n0\n";
        let r = import_dxf(&dxf(ents, blocks));
        assert_eq!(r.contours.len(), 3, "{}", r.note);
        assert!(r.note.contains("skipped 1 INSERT"), "{}", r.note);
        let by = |layer: &str| r.layers.iter().position(|l| l == layer);
        let e = bounds(&r.contours[0]);
        assert!((e.w() - 4.0).abs() < 1e-3 && (e.h() - 2.0).abs() < 1e-3 && r.contours[0].closed, "{e:?}");
        let sp = bounds(&r.contours[1]);
        assert!(sp.x0.abs() < 1e-9 && (sp.x1 - 2.0).abs() < 1e-9 && (sp.y1 - 1.0).abs() < 1e-3, "{sp:?}");
        // the square's centre (the base point) lands on the insertion point; 2x scale, 90 degrees: still 2 x 2
        let k = by("L").expect("the block's layer 0 entity takes the INSERT's layer");
        let b = bounds(&r.contours[k]);
        assert!((b.x0 - 9.0).abs() < 1e-6 && (b.x1 - 11.0).abs() < 1e-6 && (b.y0 - 9.0).abs() < 1e-6 && (b.y1 - 11.0).abs() < 1e-6, "{b:?}");
    }
}
