//! LightBurn projects (.lbrn2, XML): the shapes come in as vectors, one layer per LightBurn cut layer (its name, or
//! "C00" style numbers). Laser shops send these.
//!
//! Format: `<Shape Type="Path|Group|Rect|Ellipse|Text…">` with an `<XForm>a b c d e f</XForm>` (mm, applied to the
//! shape and its children), groups hold `<Children>`. Paths: `<VertList>` = `V x y` per vertex, optional `c0x/c0y`
//! (control leaving it) and `c1x/c1y` (control arriving), "c0x1" = none; `<PrimList>` = `L a b` lines and `B a b`
//! Béziers between vertex numbers, or `LineClosed` = all vertices joined in a loop. Text keeps a `BackupPath` that is
//! read like a path.
//!
//! ponytail: old .lbrn (v1, per-vertex XML elements) and Polygon shapes are counted as skipped, images skipped.

use crate::geom::{bounds, pt, transformed, Affine, Builder, Contour, Pt, Rect};
use crate::svg::ImportResult;

/// the value of `key="…"` in a start tag's text
fn attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let i = tag.find(&format!(" {key}=\""))? + key.len() + 3;
    tag[i..].find('"').map(|e| &tag[i..i + e])
}
fn num(tag: &str, key: &str) -> f64 {
    attr(tag, key).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

struct Vert {
    p: Pt,
    c0: Option<Pt>,
    c1: Option<Pt>,
}

/// `V151.3 928.1c0x1c1x1V…` → vertices
fn verts(s: &str) -> Vec<Vert> {
    s.split('V').filter(|v| !v.trim().is_empty()).map(|v| {
        // numbers and the c0x / c0y / c1x / c1y markers in front of them
        let mut xy = [0.0f64; 2];
        let mut ctl = [[None::<f64>; 2]; 2];
        let mut rest = v.trim_start();
        let mut k = 0;
        let mut slot: Option<(usize, usize)> = None; // (control, axis) the next number belongs to
        while !rest.is_empty() {
            if let Some(r) = rest.strip_prefix('c') {
                let b = r.as_bytes();
                if b.len() >= 2 {
                    slot = Some(((b[0] == b'1') as usize, (b[1] == b'y') as usize));
                    rest = &r[2..];
                    continue;
                }
                break;
            }
            let end = rest.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(rest.len());
            let n: f64 = rest[..end].parse().unwrap_or(0.0);
            match slot.take() {
                // "c0x1" alone (no c0y after it) = no control point
                Some((c, axis)) => ctl[c][axis] = Some(n),
                None if k < 2 => {
                    xy[k] = n;
                    k += 1;
                }
                None => {}
            }
            rest = rest[end.max(1).min(rest.len())..].trim_start();
        }
        let ctl_pt = |c: [Option<f64>; 2]| Some(pt(c[0]?, c[1]?));
        Vert { p: pt(xy[0], xy[1]), c0: ctl_pt(ctl[0]), c1: ctl_pt(ctl[1]) }
    }).collect()
}

/// a path's contours from its vertices and primitive list
fn path(vs: &[Vert], prims: &str) -> Vec<Contour> {
    if vs.len() < 2 {
        return Vec::new();
    }
    let prims = prims.trim();
    let mut list: Vec<(bool, usize, usize)> = Vec::new(); // (bezier, from, to)
    if prims.starts_with("Line") {
        list = (0..vs.len() - 1).map(|i| (false, i, i + 1)).collect();
        if prims.contains("Closed") {
            list.push((false, vs.len() - 1, 0));
        }
    } else {
        let mut it = prims.split(|c: char| c == 'L' || c == 'B' || c.is_ascii_whitespace()).filter(|t| !t.is_empty());
        let kinds = prims.chars().filter(|c| matches!(c, 'L' | 'B'));
        for kind in kinds {
            let (Some(a), Some(b)) = (it.next().and_then(|t| t.parse::<usize>().ok()), it.next().and_then(|t| t.parse::<usize>().ok())) else { break };
            if a < vs.len() && b < vs.len() {
                list.push((kind == 'B', a, b));
            }
        }
    }
    let mut out = Vec::new();
    let mut b = Builder::default();
    let (mut first, mut last) = (usize::MAX, usize::MAX);
    let finish = |b: &mut Builder, out: &mut Vec<Contour>, closed: bool| {
        if closed {
            b.close();
        }
        if b.c.nodes.len() > 1 {
            out.push(std::mem::take(&mut b.c));
        }
        b.c = Contour::default();
    };
    for (bez, a, z) in list {
        if a != last {
            finish(&mut b, &mut out, false);
            b.start(vs[a].p);
            first = a;
        }
        if bez {
            b.cubic(vs[a].c0.unwrap_or(vs[a].p), vs[z].c1.unwrap_or(vs[z].p), vs[z].p);
        } else {
            b.line(vs[z].p);
        }
        last = z;
        if z == first {
            finish(&mut b, &mut out, true);
            last = usize::MAX;
        }
    }
    finish(&mut b, &mut out, false);
    out
}

/// a w × h rectangle centred on 0, 0 with corner radius r (quarter circles as cubics)
fn rect(w: f64, h: f64, r: f64) -> Contour {
    let (x, y, r) = (w / 2.0, h / 2.0, r.clamp(0.0, w.min(h) / 2.0));
    let mut b = Builder::default();
    let k = r * 0.552_284_75;
    b.start(pt(-x + r, -y));
    for (p0, p1, c) in [((x - r, -y), (x, -y + r), (1.0, 1.0)), ((x, y - r), (x - r, y), (-1.0, 1.0)), ((-x + r, y), (-x, y - r), (-1.0, -1.0)), ((-x, -y + r), (-x + r, -y), (1.0, -1.0))] {
        b.line(pt(p0.0, p0.1));
        if r > 0.0 {
            // tangent along the edge just drawn, then into the next one
            let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
            let (c1, c2) = if c.0 * c.1 > 0.0 { (pt(p0.0 + k * dx.signum(), p0.1), pt(p1.0, p1.1 - k * dy.signum())) } else { (pt(p0.0, p0.1 + k * dy.signum()), pt(p1.0 - k * dx.signum(), p1.1)) };
            b.cubic(c1, c2, pt(p1.0, p1.1));
        }
    }
    b.close();
    b.c
}

/// A LightBurn .lbrn2 project's shapes, inches, artwork's corner at 0, 0.
pub fn import_lbrn(text: &str) -> ImportResult {
    let mut r = ImportResult::default();
    if !text.contains("<LightBurnProject") {
        r.note = "This isn't a LightBurn project.".into();
        return r;
    }
    // cut layer names by index
    let mut names = std::collections::HashMap::new();
    for (i, _) in text.match_indices("<CutSetting") {
        let body = &text[i..text[i..].find("</CutSetting>").map_or(text.len(), |e| i + e)];
        let v = |k: &str| body.find(&format!("<{k} Value=\"")).map(|j| &body[j + k.len() + 9..]).and_then(|s| s.find('"').map(|e| s[..e].to_string()));
        if let Some(ix) = v("index") {
            names.insert(ix, v("name").unwrap_or_default());
        }
    }
    struct Frame {
        tag: String,
        xf: Affine,
        verts: String,
        prims: String,
    }
    let identity = Affine::default();
    let mut stack: Vec<Frame> = Vec::new();
    let (mut paths, mut layers, mut skipped) = (Vec::<Contour>::new(), Vec::<String>::new(), 0usize);
    // pictures: base64, corners (mm, before the mirror / move below), layer
    let mut images: Vec<(String, [Pt; 4], String)> = Vec::new();
    let mut emit = |f: &Frame, paths: &mut Vec<Contour>, layers: &mut Vec<String>, skipped: &mut usize| {
        let kind = attr(&f.tag, "Type").unwrap_or("");
        let ix = attr(&f.tag, "CutIndex").unwrap_or("0");
        let layer = names.get(ix).cloned().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("C{:0>2}", ix));
        if kind == "Bitmap" {
            // W × H centred on its own origin, then the XForm (row 0 of the picture at the top)
            let (w, h) = (num(&f.tag, "W") / 2.0, num(&f.tag, "H") / 2.0);
            if let Some(data) = attr(&f.tag, "Data").filter(|d| !d.is_empty() && w > 0.0 && h > 0.0) {
                images.push((data.to_string(), [pt(-w, -h), pt(w, -h), pt(w, h), pt(-w, h)].map(|p| f.xf.apply(p)), layer));
            } else {
                *skipped += 1;
            }
            return;
        }
        let local = match kind {
            "Path" | "Text" => path(&verts(&f.verts), &f.prims),
            "Rect" => vec![rect(num(&f.tag, "W"), num(&f.tag, "H"), num(&f.tag, "Cr"))],
            "Ellipse" => vec![crate::geom::make_ellipse(0.0, 0.0, num(&f.tag, "Rx"), num(&f.tag, "Ry"))],
            "Group" => Vec::new(),
            _ => {
                *skipped += 1;
                Vec::new()
            }
        };
        for c in local {
            paths.push(transformed(&c, &f.xf));
            layers.push(layer.clone());
        }
    };
    // shapes share vertex and primitive lists by id (copies, and paths with the same segment layout): the first
    // shape with an id carries the list
    let (mut vert_ids, mut prim_ids) = (std::collections::HashMap::new(), std::collections::HashMap::new());
    let mut shared = |f: &mut Frame| {
        for (key, list, ids) in [("VertID", &mut f.verts, &mut vert_ids), ("PrimID", &mut f.prims, &mut prim_ids)] {
            if let Some(id) = attr(&f.tag, key) {
                if list.is_empty() {
                    *list = ids.get(id).cloned().unwrap_or_default();
                } else {
                    ids.entry(id.to_string()).or_insert_with(|| list.clone());
                }
            }
        }
    };
    let mut i = 0;
    while let Some(k) = text[i..].find('<') {
        let at = i + k;
        let end = text[at..].find('>').map_or(text.len(), |e| at + e + 1);
        let tag = &text[at..end];
        let inner = |name: &str| text[end..].find(&format!("</{name}>")).map(|e| text[end..end + e].to_string()).unwrap_or_default();
        if tag.starts_with("<Shape ") {
            let parent = stack.last().map_or(identity, |f| f.xf);
            let mut f = Frame { tag: tag.to_string(), xf: parent, verts: String::new(), prims: String::new() };
            if tag.ends_with("/>") {
                shared(&mut f);
                emit(&f, &mut paths, &mut layers, &mut skipped);
            } else {
                stack.push(f);
            }
        } else if tag.starts_with("</Shape") {
            if let Some(mut f) = stack.pop() {
                shared(&mut f);
                emit(&f, &mut paths, &mut layers, &mut skipped);
            }
        } else if let Some(f) = stack.last_mut() {
            if tag == "<XForm>" {
                let m: Vec<f64> = inner("XForm").split_ascii_whitespace().filter_map(|n| n.parse().ok()).collect();
                if m.len() == 6 {
                    let parent = f.xf;
                    f.xf = parent.then(&Affine { a: m[0], b: m[1], c: m[2], d: m[3], e: m[4], f: m[5] });
                }
            } else if tag == "<VertList>" && f.verts.is_empty() {
                f.verts = inner("VertList");
            } else if tag == "<PrimList>" && f.prims.is_empty() {
                f.prims = inner("PrimList");
            }
        }
        i = end;
    }
    if paths.is_empty() && images.is_empty() {
        r.note = "No shapes found in the LightBurn project.".into();
        return r;
    }
    // the project's Mirror X / Y (how LightBurn shows the bed for this machine's origin): what the designer saw
    let head = &text[..text.find("<LightBurnProject").map_or(0, |i| i + text[i..].find('>').unwrap_or(0))];
    let flip = |k: &str| if attr(head, k) == Some("True") { -1.0 } else { 1.0 };
    let mirror = Affine { a: flip("MirrorX"), d: flip("MirrorY"), ..Default::default() };
    let paths: Vec<Contour> = paths.iter().map(|c| transformed(c, &mirror)).collect();
    let mut bb = Rect::default();
    for c in &paths {
        bb.add_rect(&bounds(c));
    }
    for (_, corners, _) in &images {
        for p in corners {
            bb.add(mirror.apply(*p));
        }
    }
    let m = Affine { a: 1.0 / 25.4, d: 1.0 / 25.4, e: -bb.x0 / 25.4, f: -bb.y0 / 25.4, ..Default::default() };
    r.contours = paths.iter().map(|c| transformed(c, &m)).collect();
    r.layers = layers;
    r.images = images.into_iter().map(|(data, corners, layer)| crate::svg::ImportImage { data, corners: corners.map(|p| m.apply(mirror.apply(p))), layer }).collect();
    r.note = format!("{:.2} × {:.2} in from LightBurn{}", bb.w() / 25.4, bb.h() / 25.4, if skipped > 0 { format!("; skipped {skipped} shapes it doesn't know") } else { String::new() });
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_come_with_their_corners() {
        // LightBurn's Bitmap: W × H centred on its origin, then the XForm (here a quarter size at 55, 55)
        let lb = r#"<LightBurnProject AppVersion="1.7.08" FormatVersion="1"><CutSetting type="Image"><index Value="2"/><name Value="Photo"/></CutSetting>
<Shape Type="Bitmap" CutIndex="2" W="40" H="20" Gamma="1" File="" Data="iVBORw0KGgo="><XForm>0.25 0 0 0.25 55 55</XForm></Shape></LightBurnProject>"#;
        let r = import_lbrn(lb);
        assert!(r.contours.is_empty());
        assert_eq!(r.images.len(), 1, "{}", r.note);
        let im = &r.images[0];
        assert_eq!((im.data.as_str(), im.layer.as_str()), ("iVBORw0KGgo=", "Photo"));
        let mm: Vec<(f64, f64)> = im.corners.iter().map(|p| ((p.x * 25.4 * 1e6).round() / 1e6, (p.y * 25.4 * 1e6).round() / 1e6)).collect();
        assert_eq!(mm, [(0.0, 0.0), (10.0, 0.0), (10.0, 5.0), (0.0, 5.0)]);
    }

    #[test]
    fn lightburn_paths_groups_rects_and_layers() {
        let lb = r#"<?xml version="1.0"?><LightBurnProject AppVersion="1.7.08" FormatVersion="1" MirrorX="False" MirrorY="False">
<CutSetting type="Cut"><index Value="0"/><name Value="Outline"/></CutSetting>
<Shape Type="Group" CutIndex="0"><XForm>1 0 0 1 101.6 0</XForm><Children>
  <Shape Type="Path" CutIndex="0" VertID="1" PrimID="1"><XForm>1 0 0 1 0 0</XForm>
    <VertList>V0 0c0x1c1x1V25.4 0c0x1c1x1V25.4 25.4c0x1c1x1V0 25.4c0x1c1x1</VertList><PrimList>LineClosed</PrimList></Shape>
  <Shape Type="Path" CutIndex="0" VertID="2" PrimID="1"><XForm>1 0 0 1 0 0</XForm>
    <VertList>V0 0c0x1c1x1V12.7 0c0x1c1x1V12.7 12.7c0x1c1x1V0 12.7c0x1c1x1</VertList></Shape>
  <Shape Type="Path" CutIndex="3"><XForm>1 0 0 1 0 0</XForm>
    <VertList>V0 50.8c0x10c0y60c1x1V50.8 50.8c0x1c1x40c1y60</VertList><PrimList>B0 1</PrimList></Shape>
</Children></Shape>
<Shape Type="Rect" CutIndex="0" W="50.8" H="25.4" Cr="5"><XForm>1 0 0 1 25.4 -12.7</XForm></Shape>
<Shape Type="Bitmap" CutIndex="0"><XForm>1 0 0 1 0 0</XForm></Shape>
</LightBurnProject>"#;
        let r = import_lbrn(lb);
        assert_eq!(r.contours.len(), 4, "{}", r.note);
        assert_eq!(r.layers, ["Outline", "Outline", "C03", "Outline"]);
        assert!(r.contours[1].closed && (bounds(&r.contours[1]).w() - 0.5).abs() < 1e-9, "the half-inch square shares the first one's loop");
        // the square: 1 in, closed, moved right of the rectangle by the group's transform
        let sq = bounds(&r.contours[0]);
        assert!(r.contours[0].closed && (sq.w() - 1.0).abs() < 1e-9 && (sq.x0 - 4.0).abs() < 1e-9, "{sq:?}");
        // the curve is open and bulges up to its controls
        assert!(!r.contours[2].closed && bounds(&r.contours[2]).y1 > 2.0 + 0.2);
        // the rounded rectangle: 2 × 1 in, from the artwork corner
        let rc = bounds(&r.contours[3]);
        assert!((rc.w() - 2.0).abs() < 1e-6 && (rc.h() - 1.0).abs() < 1e-6 && rc.x0.abs() < 1e-9 && rc.y0.abs() < 1e-9, "{rc:?}");
        assert!(r.note.contains("skipped 1"));
        // Mirror Y on (the bed as LightBurn showed it): the curve, at the top before, is now at the bottom
        let m = import_lbrn(&lb.replace(r#"MirrorY="False""#, r#"MirrorY="True""#));
        assert!(bounds(&m.contours[2]).y0.abs() < 1e-3 && bounds(&m.contours[3]).y0 > 2.0, "{:?}", m.contours.iter().map(bounds).collect::<Vec<_>>());
    }

    /// real files: `GCARVE_LBRN=file.lbrn2 cargo test -p gcarve-core --lib lbrn_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lbrn_file() {
        let path = std::env::var("GCARVE_LBRN").expect("GCARVE_LBRN");
        let r = import_lbrn(&std::fs::read_to_string(&path).unwrap());
        println!("{path}: {} paths; {}", r.contours.len(), r.note);
        if let Ok(out) = std::env::var("GCARVE_LBRN_SVG") {
            let mut s = String::from("<svg xmlns='http://www.w3.org/2000/svg' viewBox='-1 -60 100 62' width='1000'><g transform='scale(1,-1)' fill='none' stroke='black' stroke-width='0.05'>");
            for c in &r.contours {
                let pts = crate::geom::flatten(c, 0.01);
                s += &format!("<path d='M{}{}'/>", pts.iter().map(|p| format!("{:.3},{:.3}", p.x, p.y)).collect::<Vec<_>>().join("L"), if c.closed { "Z" } else { "" });
            }
            std::fs::write(out, s + "</g></svg>").unwrap();
        }
    }
}
