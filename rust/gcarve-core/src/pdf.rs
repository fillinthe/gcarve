//! PDF / Illustrator (.ai, which is PDF inside) vector import: the paths drawn on the file's pages, in inches, Y up.
//! What sign shops get from customers.
//!
//! How it reads a file: every `N G obj` in the file (and inside compressed object streams) is collected; each
//! `/Type /Page` gives its content streams and its XObjects; the content is interpreted (paths, transforms, and
//! `Do` into form XObjects with their matrix, which is how transparency / blend groups and Illustrator layers are
//! stored). Pages sit side by side. Files with no page objects to be found fall back to reading every text-like
//! stream as content (`loose_streams`).
//!
//! ponytail: no encryption, no cross-reference parsing (objects are found by scanning, the newest copy of a number
//! wins), no fonts (live text is skipped), no images, clip paths ignored (clipped artwork comes in whole), patterns
//! and shadings skipped. A real xref / incremental-update reader when a customer file needs it.

use std::collections::HashMap;

use crate::geom::{bounds, pt, Affine, Builder, Contour, Rect};
use crate::svg::ImportResult;

const MAX_STREAM: usize = 64 << 20; // decoded bytes per stream
const MAX_DEPTH: usize = 12; // forms inside forms

fn find(s: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    s.get(from..)?.windows(pat.len()).position(|w| w == pat).map(|i| i + from)
}

struct Obj {
    dict: String,
    /// decoded stream data (None: no stream, or a filter this reader doesn't decode)
    stream: Option<Vec<u8>>,
}

fn inflate(dict: &str, raw: &[u8]) -> Option<Vec<u8>> {
    if dict.contains("/FlateDecode") {
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(raw, MAX_STREAM).ok()
    } else if dict.contains("/Filter") {
        None
    } else {
        Some(raw.to_vec())
    }
}

/// Every object in the file by number, including the ones packed into object streams.
fn objects(pdf: &[u8]) -> HashMap<u32, Obj> {
    let mut out: HashMap<u32, Obj> = HashMap::new();
    let mut i = 0;
    while let Some(at) = find(pdf, i, b" obj") {
        i = at + 4;
        // "<num> <gen> obj": walk back over the generation and the number
        let head = &pdf[at.saturating_sub(24)..at];
        let text = String::from_utf8_lossy(head);
        let mut parts = text.split_ascii_whitespace().rev();
        let (Some(gen), Some(num)) = (parts.next(), parts.next()) else { continue };
        let (Ok(_), Ok(num)) = (gen.parse::<u32>(), num.trim_start_matches(|c: char| !c.is_ascii_digit()).parse::<u32>()) else { continue };
        let end = find(pdf, i, b"endobj").unwrap_or(pdf.len());
        let body = &pdf[i..end];
        let (dict_end, stream) = match find(body, 0, b"stream") {
            Some(s) if body.get(s + 6) == Some(&b'\n') || body.get(s + 6..s + 8) == Some(b"\r\n") => {
                let d = s + 6 + if body[s + 6] == b'\r' { 2 } else { 1 };
                // the stream may itself contain "endobj": its end is "endstream", searched in the whole file
                let abs = i + d;
                let e = find(pdf, abs, b"endstream").unwrap_or(pdf.len());
                i = e;
                (s, Some(&pdf[abs..e]))
            }
            _ => (body.len(), None),
        };
        let dict = String::from_utf8_lossy(&body[..dict_end]).into_owned();
        let stream = stream.and_then(|raw| inflate(&dict, raw));
        out.insert(num, Obj { dict, stream });
    }
    // object streams: "<num> <offset>" pairs, then the objects themselves from /First on
    let packed: Vec<(usize, usize, Vec<u8>)> = out
        .values()
        .filter(|o| o.dict.contains("/ObjStm"))
        .filter_map(|o| Some((number_after(&o.dict, "/N")? as usize, number_after(&o.dict, "/First")? as usize, o.stream.clone()?)))
        .collect();
    for (n, first, data) in packed {
        let head = String::from_utf8_lossy(&data[..first.min(data.len())]).into_owned();
        let nums: Vec<usize> = head.split_ascii_whitespace().filter_map(|t| t.parse().ok()).collect();
        for k in 0..n.min(nums.len() / 2) {
            let (num, from) = (nums[2 * k] as u32, first + nums[2 * k + 1]);
            let to = if k + 1 < nums.len() / 2 { first + nums[2 * k + 3] } else { data.len() };
            if from < to && to <= data.len() {
                out.entry(num).or_insert(Obj { dict: String::from_utf8_lossy(&data[from..to]).into_owned(), stream: None });
            }
        }
    }
    out
}

/// The raw text of the value after `/Key` in a dictionary: a number, a name, "N G R", `[...]` or `<<...>>`.
fn value_after<'a>(dict: &'a str, key: &str) -> Option<&'a str> {
    let b = dict.as_bytes();
    let mut from = 0;
    loop {
        let at = dict[from..].find(key)? + from;
        from = at + key.len();
        // the key must end here ("/Type" must not match "/TypeX")
        if b.get(from).is_some_and(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let rest = dict[from..].trim_start();
        let start = dict.len() - rest.len();
        let r = rest.as_bytes();
        let len = match r.first()? {
            b'[' => r.iter().position(|&c| c == b']')? + 1,
            b'<' if r.get(1) == Some(&b'<') => {
                let (mut depth, mut k) = (0, 0);
                while k + 1 < r.len() {
                    if &r[k..k + 2] == b"<<" {
                        depth += 1;
                        k += 2;
                    } else if &r[k..k + 2] == b">>" {
                        depth -= 1;
                        k += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        k += 1;
                    }
                }
                k
            }
            // up to the next key or the end of the dictionary: "12 0 R", "/Name", "3.5"
            _ => r.iter().skip(1).position(|&c| c == b'/' || c == b'>').map_or(r.len(), |p| p + 1),
        };
        return Some(dict[start..start + len].trim());
    }
}

fn number_after(dict: &str, key: &str) -> Option<f64> {
    value_after(dict, key)?.split_ascii_whitespace().next()?.parse().ok()
}

/// The numbers in "[a b c d]".
fn numbers(value: &str) -> Vec<f64> {
    value.trim_matches(|c| c == '[' || c == ']').split_ascii_whitespace().filter_map(|t| t.parse().ok()).collect()
}

/// Object numbers referenced in a value: "12 0 R" or "[3 0 R 4 0 R]".
fn refs(value: &str) -> Vec<u32> {
    let t: Vec<&str> = value.trim_matches(|c| c == '[' || c == ']').split_ascii_whitespace().collect();
    (2..t.len()).filter(|&k| t[k].starts_with('R')).filter_map(|k| t[k - 2].parse().ok()).collect()
}

/// A dictionary value that may be inline or a reference: the dictionary text itself.
fn resolve<'a>(objs: &'a HashMap<u32, Obj>, value: &'a str) -> &'a str {
    if value.starts_with("<<") {
        return value;
    }
    refs(value).first().and_then(|n| objs.get(n)).map_or("", |o| o.dict.as_str())
}

/// Name → object number of the XObjects a page or form may draw.
fn xobjects(objs: &HashMap<u32, Obj>, dict: &str) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    let Some(res) = value_after(dict, "/Resources") else { return out };
    let res = resolve(objs, res);
    let Some(x) = value_after(res, "/XObject") else { return out };
    let x = resolve(objs, x);
    // "/Name 12 0 R" entries (names may touch the next token: "/G1 5 0 R/G2 6 0 R")
    let spaced = x.replace('/', " /").replace("<<", " ").replace(">>", " ");
    let t: Vec<&str> = spaced.split_ascii_whitespace().collect();
    for k in 0..t.len() {
        if t[k].starts_with('/') && t.get(k + 3).is_some_and(|r| r.starts_with('R')) {
            if let Ok(n) = t[k + 1].parse() {
                out.insert(t[k][1..].to_string(), n);
            }
        }
    }
    out
}

struct Reader<'a> {
    objs: &'a HashMap<u32, Obj>,
    /// every XObject name in the file: used when a page's own resources can't be followed (inherited ones)
    any: HashMap<String, u32>,
    out: Vec<Contour>,
}

impl Reader<'_> {
    /// Paths painted by one content stream; `ctm` = the transform in force, `names` = its XObjects.
    fn content(&mut self, s: &[u8], mut ctm: Affine, names: &HashMap<String, u32>, depth: usize) {
        let mut stack: Vec<Affine> = Vec::new();
        let mut nums: Vec<f64> = Vec::new();
        let mut name = String::new(); // the last /Name operand (for Do)
        let mut path: Vec<Contour> = Vec::new(); // finished subpaths of the current path
        let mut b = Builder::default();
        let mut open = false; // b holds a subpath
        let mut in_text = false;
        let mut i = 0;
        let delim = |c: u8| c.is_ascii_whitespace() || matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%');
        while i < s.len() {
            let c = s[i];
            match c {
                _ if c.is_ascii_whitespace() => i += 1,
                b'%' => i = find(s, i, b"\n").unwrap_or(s.len()),
                b'(' => {
                    // string: balanced parentheses, backslash escapes
                    let mut level = 0;
                    while i < s.len() {
                        match s[i] {
                            b'\\' => i += 1,
                            b'(' => level += 1,
                            b')' => level -= 1,
                            _ => {}
                        }
                        i += 1;
                        if level == 0 {
                            break;
                        }
                    }
                    nums.clear();
                }
                // (a stray ')' too: it is a delimiter, so the token branch below would never move past it)
                b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' => {
                    i += 1;
                    nums.clear();
                }
                b'/' => {
                    let start = i + 1;
                    i += 1;
                    while i < s.len() && !delim(s[i]) {
                        i += 1;
                    }
                    name = String::from_utf8_lossy(&s[start..i]).into_owned();
                }
                _ => {
                    let start = i;
                    while i < s.len() && !delim(s[i]) {
                        i += 1;
                    }
                    let tok = &s[start..i];
                    if let Some(v) = std::str::from_utf8(tok).ok().and_then(|t| t.parse::<f64>().ok()) {
                        if nums.len() < 64 {
                            nums.push(v);
                        }
                        continue;
                    }
                    let n = nums.len();
                    let p = |k: usize| ctm.apply(pt(nums[k], nums[k + 1]));
                    let end_sub = |b: &mut Builder, open: &mut bool, path: &mut Vec<Contour>, close: bool| {
                        if std::mem::take(open) {
                            if close {
                                b.close();
                            }
                            if b.c.nodes.len() > 1 {
                                path.push(std::mem::take(&mut b.c));
                            }
                        }
                    };
                    match tok {
                        b"BT" => in_text = true,
                        b"ET" => in_text = false,
                        _ if in_text => {}
                        b"q" => stack.push(ctm),
                        b"Q" => ctm = stack.pop().unwrap_or(ctm),
                        b"cm" if n >= 6 => {
                            let m = Affine { a: nums[n - 6], b: nums[n - 5], c: nums[n - 4], d: nums[n - 3], e: nums[n - 2], f: nums[n - 1] };
                            ctm = ctm.then(&m); // (ctm × m: the new matrix applies first, then what was in force)
                        }
                        b"m" if n >= 2 => {
                            end_sub(&mut b, &mut open, &mut path, false);
                            b.start(p(n - 2));
                            open = true;
                        }
                        b"l" if n >= 2 && open => b.line(p(n - 2)),
                        b"c" if n >= 6 && open => b.cubic(p(n - 6), p(n - 4), p(n - 2)),
                        b"v" if n >= 4 && open => {
                            let cur = b.cur();
                            b.cubic(cur, p(n - 4), p(n - 2));
                        }
                        b"y" if n >= 4 && open => b.cubic(p(n - 4), p(n - 2), p(n - 2)),
                        b"h" => end_sub(&mut b, &mut open, &mut path, true),
                        b"re" if n >= 4 => {
                            end_sub(&mut b, &mut open, &mut path, false);
                            let (x, y, w, h) = (nums[n - 4], nums[n - 3], nums[n - 2], nums[n - 1]);
                            b.start(ctm.apply(pt(x, y)));
                            for q in [pt(x + w, y), pt(x + w, y + h), pt(x, y + h)] {
                                b.line(ctm.apply(q));
                            }
                            b.close();
                            path.push(std::mem::take(&mut b.c));
                        }
                        // painted: stroked paths keep open ends, filled ones are closed shapes
                        b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" => {
                            let fills = !matches!(tok, b"S");
                            end_sub(&mut b, &mut open, &mut path, fills);
                            for mut c in path.drain(..) {
                                c.closed |= fills && c.nodes.len() > 2;
                                self.out.push(c);
                            }
                        }
                        b"n" => {
                            // clip path only: not artwork
                            open = false;
                            path.clear();
                        }
                        // a form XObject: a group / layer / symbol drawn through its own matrix
                        b"Do" if depth < MAX_DEPTH => {
                            let objs = self.objs;
                            if let Some(form) = names.get(&name).or_else(|| self.any.get(&name)).and_then(|n| objs.get(n)).filter(|f| f.dict.contains("/Form")) {
                                if let Some(data) = &form.stream {
                                    let m = value_after(&form.dict, "/Matrix").map(numbers).unwrap_or_default();
                                    let fm = if m.len() == 6 { Affine { a: m[0], b: m[1], c: m[2], d: m[3], e: m[4], f: m[5] } } else { Affine { a: 1.0, d: 1.0, ..Default::default() } };
                                    let own = xobjects(objs, &form.dict);
                                    self.content(data, ctm.then(&fm), &own, depth + 1); // the form's matrix first, then the page's
                                }
                            }
                        }
                        b"BI" => i = find(s, i, b"EI").map_or(s.len(), |e| e + 2), // inline image data
                        _ => {}
                    }
                    nums.clear();
                }
            }
        }
    }
}

/// Fallback for files whose pages can't be found: every text-like stream read as page content.
fn loose_streams(objs: &HashMap<u32, Obj>) -> Vec<&[u8]> {
    let mut nums: Vec<&u32> = objs.keys().collect();
    nums.sort();
    nums.into_iter()
        .filter_map(|n| {
            let o = &objs[n];
            let has = |k: &str| o.dict.contains(k);
            if has("/Image") || has("/Length1") || has("/FontFile") || has("/ObjStm") || has("/XRef") || has("/Metadata") {
                return None;
            }
            let data = o.stream.as_deref()?;
            // content streams are text; fonts, colour profiles and the like are binary
            let head = &data[..data.len().min(400)];
            // Illustrator's private copy of the file (PostScript, same path operators) and XML metadata are not page content
            if head.is_empty() || head.starts_with(b"%!PS") || find(head, 0, b"%%Creator").is_some() || find(head, 0, b"%AI").is_some() || head.starts_with(b"<?x") || head.starts_with(b"<x:") {
                return None;
            }
            (head.iter().filter(|b| b.is_ascii_graphic() || b.is_ascii_whitespace()).count() * 10 >= head.len() * 9).then_some(data)
        })
        .collect()
}

/// Vector paths of a PDF or .ai file. The artwork's bottom-left lands on (0, 0); pages sit side by side.
pub fn import_pdf(bytes: &[u8]) -> ImportResult {
    let mut r = ImportResult::default();
    if find(&bytes[..bytes.len().min(1024)], 0, b"%PDF-").is_none() {
        r.note = "This isn't a PDF (old Illustrator files without PDF content can't be read: save as PDF or SVG).".into();
        return r;
    }
    let objs = objects(bytes);
    let mut rd = Reader { objs: &objs, any: HashMap::new(), out: Vec::new() };
    for o in objs.values() {
        rd.any.extend(xobjects(&objs, &o.dict));
    }
    // pages in object order (the order they were written in), each shifted right of the one before
    let is_page = |o: &Obj| value_after(&o.dict, "/Type").is_some_and(|t| t.trim_start_matches('/').split_ascii_whitespace().next() == Some("Page"));
    let mut pages: Vec<&u32> = objs.iter().filter(|(_, o)| is_page(o)).map(|(n, _)| n).collect();
    pages.sort();
    let identity = Affine { a: 1.0, d: 1.0, ..Default::default() };
    let mut x = 0.0;
    for n in &pages {
        let page = &objs[*n];
        let names = xobjects(&objs, &page.dict);
        let before = rd.out.len();
        for c in value_after(&page.dict, "/Contents").map(refs).unwrap_or_default() {
            if let Some(data) = objs.get(&c).and_then(|o| o.stream.as_deref()) {
                rd.content(data, Affine { e: x, ..identity }, &names, 0);
            }
        }
        if rd.out.len() > before {
            let w = value_after(&page.dict, "/MediaBox").map(numbers).filter(|m| m.len() == 4).map_or(612.0, |m| (m[2] - m[0]).abs());
            x += w + 72.0;
        }
    }
    if rd.out.is_empty() {
        for s in loose_streams(&objs) {
            rd.content(s, identity, &HashMap::new(), 0);
        }
    }
    let mut paths = rd.out;
    // identical paths drawn twice (fill, then stroke) count once
    let mut seen = std::collections::HashSet::new();
    paths.retain(|c| seen.insert(c.nodes.iter().flat_map(|n| [(n.p.x * 100.0).round() as i64, (n.p.y * 100.0).round() as i64]).chain([i64::from(c.closed)]).collect::<Vec<_>>()));
    if paths.is_empty() {
        r.note = if find(bytes, 0, b"/Encrypt").is_some() { "The PDF is password-protected.".into() } else { "No vector paths found (the file may hold only pictures or live text: convert text to outlines and save again).".into() };
        return r;
    }
    let mut bb = Rect::default();
    for c in &paths {
        bb.add_rect(&bounds(c));
    }
    // points to inches, artwork's corner to the origin
    let m = Affine { a: 1.0 / 72.0, d: 1.0 / 72.0, e: -bb.x0 / 72.0, f: -bb.y0 / 72.0, ..Default::default() };
    r.contours = paths.iter().map(|c| crate::geom::transformed(c, &m)).collect();
    r.layers = vec![String::new(); r.contours.len()];
    r.note = format!("{:.2} × {:.2} in of artwork; live text and pictures are not imported", bb.w() / 72.0, bb.h() / 72.0);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf_with(content: &[u8], flate: bool) -> Vec<u8> {
        let body = if flate { miniz_oxide::deflate::compress_to_vec_zlib(content, 6) } else { content.to_vec() };
        let mut f = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_vec();
        f.extend(format!("4 0 obj\n<< /Length {}{} >>\nstream\n", body.len(), if flate { " /Filter /FlateDecode" } else { "" }).bytes());
        f.extend(&body);
        f.extend(b"\nendstream\nendobj\n5 0 obj\n<< /Subtype /Image /Length 4 >>\nstream\n0 0 m 9 9 l S\nendstream\nendobj\n%%EOF\n");
        f
    }

    #[test]
    fn reads_paths_with_transforms_from_plain_and_compressed_streams() {
        // a 1 in square at (72, 72) via `re`, a triangle inside a scaled + moved group, a stroked open curve, a clip path
        // and text that must be ignored
        let content = b"q 72 72 72 72 re f Q\nq 2 0 0 2 144 0 cm 0 0 m 36 0 l 18 36 l h B Q\n0 0 m 10 20 30 40 72 0 c S\n500 500 10 10 re W n\nBT /F1 12 Tf (m 1 2 l) Tj 5 5 m ET\n";
        for flate in [false, true] {
            let r = import_pdf(&pdf_with(content, flate));
            assert_eq!(r.contours.len(), 3, "flate {flate}: {}", r.note);
            let b: Vec<Rect> = r.contours.iter().map(bounds).collect();
            let close = |a: f64, v: f64| (a - v).abs() < 1e-9;
            // artwork spans x 0..216 pt, y 0..144 pt: nothing shifts
            assert!(close(b[0].x0, 1.0) && close(b[0].y0, 1.0) && close(b[0].x1, 2.0) && close(b[0].y1, 2.0) && r.contours[0].closed);
            assert!(close(b[1].x0, 2.0) && close(b[1].x1, 3.0) && close(b[1].y1, 1.0) && r.contours[1].closed && r.contours[1].nodes.len() == 3);
            assert!(!r.contours[2].closed && r.contours[2].nodes.len() == 2 && r.contours[2].nodes[0].has_out && r.contours[2].nodes[1].has_in);
        }
        // an Illustrator private-data stream (PostScript with the same operators) is not imported a second time
        let mut ai = pdf_with(content, true);
        let at = ai.len() - 6;
        ai.splice(at..at, b"6 0 obj\n<< /Length 60 >>\nstream\n%!PS-Adobe-3.0\n%%Creator: Adobe Illustrator\n0 0 m 500 500 l S\nendstream\nendobj\n".iter().copied());
        assert_eq!(import_pdf(&ai).contours.len(), 3);
        // nested transforms apply innermost first: move 1 in right, then (inside) scale 2x: the 36 pt square is 1 in at x = 1
        let nested = import_pdf(&pdf_with(b"0 0 1 1 re f q 1 0 0 1 72 0 cm q 2 0 0 2 0 0 cm 0 0 36 36 re f Q Q", false));
        let nb = bounds(&nested.contours[1]);
        assert!((nb.x0 - 1.0).abs() < 1e-9 && (nb.x1 - 2.0).abs() < 1e-9, "{nb:?}");
        assert!(import_pdf(b"not a pdf").contours.is_empty());
        assert!(import_pdf(&pdf_with(b"BT (hello) Tj ET", false)).note.contains("No vector paths"));
    }

    #[test]
    fn stray_delimiters_in_a_damaged_stream_do_not_hang() {
        // a ")" outside a string, "}" and "]" runs, binary junk: must return, not loop
        let r = import_pdf(&pdf_with(b")))) }}} ]] 0 0 m 10 10 l S ) 5 5 m 6 6 l S", false));
        assert_eq!(r.contours.len(), 2, "{}", r.note);
    }

    #[test]
    fn pages_forms_and_object_streams() {
        // Two pages. Page 1 draws a 1 in square and a form (a group) twice through different transforms; the form's own
        // /Matrix moves it 1 in up. Page 2's dictionary lives in a compressed object stream. A form is never read on
        // its own, and the second page lands to the right of the first.
        let form = b"0 0 72 72 re f";
        let p1 = b"0 0 72 72 re f\nq 1 0 0 1 144 0 cm /G1 Do Q\nq 2 0 0 2 288 0 cm /G1 Do Q\n";
        let p2 = b"0 0 36 36 re f\n";
        let packed_page = b"<< /Type /Page /MediaBox [0 0 612 792] /Contents 9 0 R >>";
        let mut packed = b"8 0 ".to_vec();
        let first = packed.len();
        packed.extend(packed_page);
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&packed, 6);
        let mut f = b"%PDF-1.5\n".to_vec();
        let add = |f: &mut Vec<u8>, n: u32, dict: &str, stream: Option<&[u8]>| {
            f.extend(format!("{n} 0 obj\n{dict}\n").bytes());
            if let Some(s) = stream {
                f.extend(b"stream\n");
                f.extend(s);
                f.extend(b"\nendstream\n");
            }
            f.extend(b"endobj\n");
        };
        add(&mut f, 1, "<< /Type /Catalog /Pages 2 0 R >>", None);
        add(&mut f, 2, "<< /Type /Pages /Kids [3 0 R 8 0 R] /Count 2 >>", None);
        add(&mut f, 3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents [4 0 R] /Resources << /XObject << /G1 5 0 R >> >> >>", None);
        add(&mut f, 4, &format!("<< /Length {} >>", p1.len()), Some(p1));
        add(&mut f, 5, &format!("<< /Type /XObject /Subtype /Form /BBox [0 0 72 72] /Matrix [1 0 0 1 0 72] /Length {} >>", form.len()), Some(form));
        add(&mut f, 7, &format!("<< /Type /ObjStm /N 1 /First {first} /Filter /FlateDecode /Length {} >>", z.len()), Some(&z));
        add(&mut f, 9, &format!("<< /Length {} >>", p2.len()), Some(p2));
        f.extend(b"%%EOF\n");
        let r = import_pdf(&f);
        assert_eq!(r.contours.len(), 4, "{}", r.note);
        let b: Vec<Rect> = r.contours.iter().map(bounds).collect();
        let at = |r: &Rect, x0: f64, y0: f64, x1: f64, y1: f64| (r.x0 - x0).abs() < 1e-9 && (r.y0 - y0).abs() < 1e-9 && (r.x1 - x1).abs() < 1e-9 && (r.y1 - y1).abs() < 1e-9;
        assert!(at(&b[0], 0.0, 0.0, 1.0, 1.0), "{:?}", b[0]);
        assert!(at(&b[1], 2.0, 1.0, 3.0, 2.0), "form moved by its matrix then the page's: {:?}", b[1]);
        assert!(at(&b[2], 4.0, 2.0, 6.0, 4.0), "form scaled 2x: {:?}", b[2]);
        // page 2: 0.5 in square, one page width (8.5 in) plus an inch to the right
        assert!(at(&b[3], 9.5, 0.0, 10.0, 0.5), "{:?}", b[3]);
    }
}

#[cfg(test)]
mod survey {
    /// `GCARVE_PDF=file.pdf cargo test -p gcarve-core --lib pdf_file -- --ignored --nocapture`: what a real file gives
    #[test]
    #[ignore]
    fn pdf_file() {
        let path = std::env::var("GCARVE_PDF").expect("GCARVE_PDF");
        let r = super::import_pdf(&std::fs::read(&path).unwrap());
        println!("{}: {} paths; {}", path, r.contours.len(), r.note);
        for c in r.contours.iter().take(12) {
            let b = crate::geom::bounds(c);
            println!("  {} nodes {} x {:.3}..{:.3} y {:.3}..{:.3}", c.nodes.len(), if c.closed { "closed" } else { "open" }, b.x0, b.x1, b.y0, b.y1);
        }
    }
}
