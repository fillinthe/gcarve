//! SVG geometry importer (C++ core/svg.cpp + core/import.h): path/rect/circle/ellipse/line/polyline/polygon, nested
//! transforms, units. Not a full XML parser: it scans tags, which is enough for geometry from Illustrator/Inkscape/
//! Affinity/Fusion.

use std::collections::HashMap;
use std::collections::BTreeMap;
use std::f64::consts::PI;

use crate::geom::{join_contours_tagged, pt, transformed, Affine, Builder, Contour, Pt};

/// Importer output: inches, Y up, joined contours.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportResult {
    pub contours: Vec<Contour>,
    /// parallel to `contours`; "" = default layer
    pub layers: Vec<String>,
    /// human-readable: units assumed, entities skipped
    pub note: String,
    /// live text, only from the `_with_text` importers (the plain ones count it as skipped in the note)
    pub texts: Vec<ImportText>,
    /// embedded pictures (LightBurn projects)
    pub images: Vec<ImportImage>,
    /// parallel to `contours` (or empty): shapes with the same non-zero number were one group in the file (LightBurn's Groups)
    pub groups: Vec<u32>,
}

/// A picture embedded in a drawing: the file's bytes as base64 (PNG, JPEG…) and where its corners land.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportImage {
    pub data: String,
    /// inches, Y up, in the same frame as the contours: the picture's bottom-left, bottom-right, top-right, top-left
    pub corners: [Pt; 4],
    pub layer: String,
}

/// A line (or block) of text found in a drawing, for the app to set in a font: the core has none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportText {
    pub text: String,
    /// anchor point, inches, Y up
    pub at: Pt,
    /// capital height, inches
    pub height: f64,
    pub layer: String,
    /// where `at` sits on the text: 0 left / 1 centre / 2 right
    pub anchor: i32,
    /// 0 on the first line's baseline / 1 top / 2 middle
    pub valign: i32,
    /// degrees, counter-clockwise, about `at`
    pub angle: f64,
}

/// C `strtod` on a prefix: (value, bytes consumed); consumed = 0 when there is no number. Leading whitespace skipped.
pub(crate) fn strtod(s: &[u8]) -> (f64, usize) {
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    let start = i;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }
    let d0 = i;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - d0;
    if i < s.len() && s[i] == b'.' {
        i += 1;
        let f0 = i;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        digits += i - f0;
    }
    if digits == 0 {
        return (0.0, 0);
    }
    if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
        let mut j = i + 1;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        if j < s.len() && s[j].is_ascii_digit() {
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    // ponytail: no hex floats / inf / nan (strtod accepts them; no CAD exporter writes them)
    let v = std::str::from_utf8(&s[start..i]).ok().and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0);
    (v, i)
}

/// C `atof`: number prefix, 0 when none.
pub(crate) fn atof(s: &str) -> f64 {
    strtod(s.as_bytes()).0
}

type Attrs = BTreeMap<String, String>;

#[derive(Default)]
struct Tag {
    name: String,
    attr: Attrs,
    close: bool,
    self_close: bool,
}

fn find(s: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    if from > s.len() {
        return None;
    }
    s[from..].windows(pat.len()).position(|w| w == pat).map(|p| p + from)
}

/// Returns None at end of input. Skips comments, PI, DOCTYPE and CDATA.
fn next_tag(s: &[u8], i: &mut usize) -> Option<Tag> {
    loop {
        *i = find(s, *i, b"<")?;
        let rest = &s[*i..];
        let end: &[u8] = if rest.starts_with(b"<!--") {
            b"-->"
        } else if rest.starts_with(b"<![CDATA[") {
            b"]]>"
        } else if rest.starts_with(b"<?") {
            b"?>"
        } else if rest.starts_with(b"<!") {
            b">"
        } else {
            break;
        };
        *i = find(s, *i, end).map_or(s.len(), |e| e + end.len());
    }
    *i += 1;
    let mut t = Tag::default();
    let txt = |a: usize, b: usize| String::from_utf8_lossy(&s[a..b]).into_owned();
    if *i < s.len() && s[*i] == b'/' {
        t.close = true;
        *i += 1;
    }
    let n0 = *i;
    while *i < s.len() && !s[*i].is_ascii_whitespace() && s[*i] != b'>' && s[*i] != b'/' {
        *i += 1;
    }
    t.name = txt(n0, *i);
    loop {
        while *i < s.len() && s[*i].is_ascii_whitespace() {
            *i += 1;
        }
        if *i >= s.len() {
            return None;
        }
        if s[*i] == b'>' {
            *i += 1;
            return Some(t);
        }
        if s[*i] == b'/' {
            t.self_close = true;
            *i += 1;
            continue;
        }
        let k0 = *i;
        while *i < s.len() && s[*i] != b'=' && s[*i] != b'>' && s[*i] != b'/' && !s[*i].is_ascii_whitespace() {
            *i += 1;
        }
        let k = txt(k0, *i);
        let mut v = String::new();
        while *i < s.len() && s[*i].is_ascii_whitespace() {
            *i += 1;
        }
        if *i < s.len() && s[*i] == b'=' {
            *i += 1;
            while *i < s.len() && s[*i].is_ascii_whitespace() {
                *i += 1;
            }
            if *i < s.len() && (s[*i] == b'"' || s[*i] == b'\'') {
                let q = s[*i];
                *i += 1;
                let v0 = *i;
                while *i < s.len() && s[*i] != q {
                    *i += 1;
                }
                v = txt(v0, *i);
                *i += 1;
            }
        }
        if !k.is_empty() {
            t.attr.insert(k, v);
        }
    }
}

fn attr<'a>(a: &'a Attrs, k: &str) -> &'a str {
    a.get(k).map_or("", |v| v.as_str())
}

fn num_attr(a: &Attrs, k: &str, def: f64) -> f64 {
    let v = attr(a, k);
    if v.is_empty() {
        def
    } else {
        atof(v)
    }
}

/// "12mm" -> px (1/96 in). Unitless, em and % are treated as px.
fn length_px(v: &str) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let (n, used) = strtod(v.as_bytes());
    if used == 0 {
        return None;
    }
    let k = match &v[used..] {
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        _ => 1.0,
    };
    Some(n * k)
}

fn parse_transform(s: &str) -> Affine {
    let b = s.as_bytes();
    let mut m = Affine::default();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b',') {
            i += 1;
        }
        let Some(p) = find(b, i, b"(") else { break };
        let name = &s[i..p];
        let Some(e) = find(b, p, b")") else { break };
        let mut v = Vec::new();
        for tok in s[p + 1..e].replace(',', " ").split_whitespace() {
            let mut t = tok.as_bytes();
            loop {
                let (d, used) = strtod(t);
                if used == 0 {
                    break;
                }
                v.push(d);
                t = &t[used..];
            }
        }
        let at = |k: usize, def: f64| v.get(k).copied().unwrap_or(def);
        let deg = PI / 180.0;
        let t = match name {
            "matrix" if v.len() >= 6 => Affine { a: v[0], b: v[1], c: v[2], d: v[3], e: v[4], f: v[5] },
            "translate" => Affine::translate(at(0, 0.0), at(1, 0.0)),
            "scale" => Affine::scale(at(0, 1.0), at(1, at(0, 1.0))),
            "rotate" => {
                Affine::translate(at(1, 0.0), at(2, 0.0))
                    * Affine::rotate(at(0, 0.0) * deg)
                    * Affine::translate(-at(1, 0.0), -at(2, 0.0))
            }
            "skewX" => Affine { c: (at(0, 0.0) * deg).tan(), ..Default::default() },
            "skewY" => Affine { b: (at(0, 0.0) * deg).tan(), ..Default::default() },
            _ => Affine::default(),
        };
        m = m * t;
        i = e + 1;
    }
    m
}

// ---- path data ----------------------------------------------------------------------------------------------------

struct PathParser<'a> {
    d: &'a [u8],
    i: usize,
}

impl PathParser<'_> {
    fn sep(&mut self) {
        while self.i < self.d.len() && (self.d[self.i].is_ascii_whitespace() || self.d[self.i] == b',') {
            self.i += 1;
        }
    }
    fn num(&mut self) -> Option<f64> {
        self.sep();
        let (v, used) = strtod(&self.d[self.i..]);
        if used == 0 {
            return None;
        }
        self.i += used;
        Some(v)
    }
    fn flag(&mut self) -> Option<bool> {
        self.sep();
        match self.d.get(self.i) {
            Some(&c) if c == b'0' || c == b'1' => {
                self.i += 1;
                Some(c == b'1')
            }
            _ => None,
        }
    }
    fn need<const N: usize>(&mut self) -> Option<[f64; N]> {
        let mut v = [0.0; N];
        for x in &mut v {
            *x = self.num()?;
        }
        Some(v)
    }

    fn parse(mut self) -> Vec<Contour> {
        let mut out = Vec::new();
        let mut b = Builder::default();
        let mut have = false;
        let (mut cur, mut start, mut last_c) = (Pt::default(), Pt::default(), Pt::default()); // last_c: S/T reflection
        let (mut cmd, mut prev) = (0u8, 0u8);
        fn flush(b: &mut Builder, have: &mut bool, out: &mut Vec<Contour>) {
            if *have && b.c.nodes.len() >= 2 {
                out.push(std::mem::take(&mut b.c));
            }
            *have = false;
        }
        fn ensure(b: &mut Builder, have: &mut bool, cur: Pt) {
            if !*have {
                b.start(cur);
                *have = true;
            }
        }
        loop {
            self.sep();
            if self.i >= self.d.len() {
                break;
            }
            let ch = self.d[self.i];
            if ch.is_ascii_alphabetic() {
                cmd = ch;
                self.i += 1;
            } else if cmd == 0 || cmd == b'Z' || cmd == b'z' {
                break;
            } else if cmd == b'M' {
                cmd = b'L';
            } else if cmd == b'm' {
                cmd = b'l';
            }
            let rel = cmd.is_ascii_lowercase();
            let c = cmd.to_ascii_uppercase();
            let o = if rel { cur } else { Pt::default() };
            let ok = match c {
                b'M' => self.need::<2>().map(|v| {
                    flush(&mut b, &mut have, &mut out);
                    cur = o + pt(v[0], v[1]);
                    start = cur;
                    b.start(cur);
                    have = true;
                }),
                b'L' => self.need::<2>().map(|v| {
                    ensure(&mut b, &mut have, cur);
                    cur = o + pt(v[0], v[1]);
                    b.line(cur);
                }),
                b'H' => self.need::<1>().map(|v| {
                    ensure(&mut b, &mut have, cur);
                    cur = pt(if rel { cur.x } else { 0.0 } + v[0], cur.y);
                    b.line(cur);
                }),
                b'V' => self.need::<1>().map(|v| {
                    ensure(&mut b, &mut have, cur);
                    cur = pt(cur.x, if rel { cur.y } else { 0.0 } + v[0]);
                    b.line(cur);
                }),
                b'C' | b'S' => {
                    let r = if c == b'C' {
                        self.need::<6>().map(|v| (o + pt(v[0], v[1]), [v[2], v[3], v[4], v[5]]))
                    } else {
                        self.need::<4>().map(|v| (if prev == b'C' || prev == b'S' { cur * 2.0 - last_c } else { cur }, v))
                    };
                    r.map(|(c1, v)| {
                        let (c2, p) = (o + pt(v[0], v[1]), o + pt(v[2], v[3]));
                        ensure(&mut b, &mut have, cur);
                        b.cubic(c1, c2, p);
                        last_c = c2;
                        cur = p;
                    })
                }
                b'Q' | b'T' => {
                    let r = if c == b'Q' {
                        self.need::<4>().map(|v| (o + pt(v[0], v[1]), [v[2], v[3]]))
                    } else {
                        self.need::<2>().map(|v| (if prev == b'Q' || prev == b'T' { cur * 2.0 - last_c } else { cur }, v))
                    };
                    r.map(|(q, v)| {
                        let p = o + pt(v[0], v[1]);
                        ensure(&mut b, &mut have, cur);
                        b.cubic(cur + (q - cur) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p);
                        last_c = q;
                        cur = p;
                    })
                }
                b'A' => self.arc_args().map(|(rx, ry, rot, large, sweep, v)| {
                    let p = o + v;
                    ensure(&mut b, &mut have, cur);
                    arc_to(&mut b, cur, rx, ry, rot, large, sweep, p);
                    cur = p;
                }),
                b'Z' => {
                    if have {
                        b.close();
                        out.push(std::mem::take(&mut b.c));
                        have = false;
                    }
                    cur = start;
                    Some(())
                }
                _ => None,
            };
            if ok.is_none() {
                break;
            }
            prev = c;
        }
        flush(&mut b, &mut have, &mut out);
        out
    }

    fn arc_args(&mut self) -> Option<(f64, f64, f64, bool, bool, Pt)> {
        let [rx, ry, rot] = self.need::<3>()?;
        let large = self.flag()?;
        let sweep = self.flag()?;
        let [x, y] = self.need::<2>()?;
        Some((rx, ry, rot, large, sweep, pt(x, y)))
    }
}

/// SVG endpoint arc -> centre parameterisation, appended as cubics.
#[allow(clippy::too_many_arguments)]
fn arc_to(b: &mut Builder, cur: Pt, rx: f64, ry: f64, rot: f64, large: bool, sweep: bool, p: Pt) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if p.x == cur.x && p.y == cur.y {
        return;
    }
    if rx == 0.0 || ry == 0.0 {
        b.line(p);
        return;
    }
    let phi = rot * PI / 180.0;
    let (sp, cp) = phi.sin_cos();
    let (dx, dy) = ((cur.x - p.x) / 2.0, (cur.y - p.y) / 2.0);
    let (x1, y1) = (cp * dx + sp * dy, -sp * dx + cp * dy);
    let lam = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if lam > 1.0 {
        rx *= lam.sqrt();
        ry *= lam.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let co = if large == sweep { -1.0 } else { 1.0 } * (num / den).max(0.0).sqrt();
    let (cx1, cy1) = (co * rx * y1 / ry, -co * ry * x1 / rx);
    let cx = cp * cx1 - sp * cy1 + (cur.x + p.x) / 2.0;
    let cy = sp * cx1 + cp * cy1 + (cur.y + p.y) / 2.0;
    let t1 = ((y1 - cy1) / ry).atan2((x1 - cx1) / rx);
    let t2 = ((-y1 - cy1) / ry).atan2((-x1 - cx1) / rx);
    let mut dt = t2 - t1;
    if !sweep && dt > 0.0 {
        dt -= 2.0 * PI;
    }
    if sweep && dt < 0.0 {
        dt += 2.0 * PI;
    }
    b.arc(cx, cy, rx, ry, phi, t1, dt);
    if let Some(last) = b.c.nodes.last_mut() {
        last.p = p;
    }
}

/// printf("%.9g"): the string is only re-parsed, so the 9-significant-digit rounding is what matters.
fn fmt(v: f64) -> String {
    format!("{v:.8e}")
}

/// Basic shapes become path data so there is one geometry code path.
fn shape_to_path(t: &Tag) -> String {
    let a = &t.attr;
    let n = |k| num_attr(a, k, 0.0);
    match t.name.as_str() {
        "path" => attr(a, "d").to_string(),
        "rect" => {
            let (x, y, w, h) = (n("x"), n("y"), n("width"), n("height"));
            let (mut rx, mut ry) = (num_attr(a, "rx", -1.0), num_attr(a, "ry", -1.0));
            if rx < 0.0 {
                rx = if ry < 0.0 { 0.0 } else { ry };
            }
            if ry < 0.0 {
                ry = rx;
            }
            rx = rx.min(w / 2.0);
            ry = ry.min(h / 2.0);
            if w <= 0.0 || h <= 0.0 {
                return String::new();
            }
            if rx <= 0.0 || ry <= 0.0 {
                return format!("M{} {}h{}v{}h{}Z", fmt(x), fmt(y), fmt(w), fmt(h), fmt(-w));
            }
            let r = format!("{} {} 0 0 1 ", fmt(rx), fmt(ry));
            format!(
                "M{} {}H{}A{r}{} {}V{}A{r}{} {}H{}A{r}{} {}V{}A{r}{} {}Z",
                fmt(x + rx),
                fmt(y),
                fmt(x + w - rx),
                fmt(x + w),
                fmt(y + ry),
                fmt(y + h - ry),
                fmt(x + w - rx),
                fmt(y + h),
                fmt(x + rx),
                fmt(x),
                fmt(y + h - ry),
                fmt(y + ry),
                fmt(x + rx),
                fmt(y)
            )
        }
        name @ ("circle" | "ellipse") => {
            let (cx, cy) = (n("cx"), n("cy"));
            let rx = if name == "circle" { n("r") } else { n("rx") };
            let ry = if name == "circle" { rx } else { n("ry") };
            if rx <= 0.0 || ry <= 0.0 {
                return String::new();
            }
            let r = format!("{} {} 0 1 0 ", fmt(rx), fmt(ry));
            format!("M{} {}A{r}{} {}A{r}{} {}Z", fmt(cx + rx), fmt(cy), fmt(cx - rx), fmt(cy), fmt(cx + rx), fmt(cy))
        }
        "line" => format!("M{} {}L{} {}", fmt(n("x1")), fmt(n("y1")), fmt(n("x2")), fmt(n("y2"))),
        name @ ("polyline" | "polygon") => {
            let p = attr(a, "points");
            if p.is_empty() {
                String::new()
            } else {
                format!("M{p}{}", if name == "polygon" { "Z" } else { "" })
            }
        }
        _ => String::new(),
    }
}

struct Level {
    m: Affine,
    hidden: bool,
    layer: String,
}

#[derive(Default)]
struct Out {
    contours: Vec<Contour>,
    layers: Vec<String>,
    texts: usize,
    /// Some = keep text for the app instead of counting it
    keep: Option<Vec<ImportText>>,
    /// images, and use elements that point nowhere
    skipped: usize,
    /// use elements followed so far (a symbol holding many uses of itself fans out within the depth limit)
    uses: usize,
}

const MAX_USE_DEPTH: u32 = 8;

/// Every element with an id as its source text, for `<use>` (also what sits inside <defs> / <symbol>). A symbol
/// is turned into a plain group so the scan below draws it.
fn collect_defs(text: &str) -> HashMap<String, String> {
    let s = text.as_bytes();
    let mut defs = HashMap::new();
    let mut open: Vec<Option<(String, usize)>> = Vec::new();
    let mut pos = 0;
    while let Some(t) = next_tag(s, &mut pos) {
        if t.close {
            if let Some(Some((id, start))) = open.pop() {
                defs.entry(id).or_insert_with(|| text[start..pos].to_string());
            }
            continue;
        }
        let id = attr(&t.attr, "id");
        // the tag started at the last '<' before `pos`
        let start = text[..pos].rfind('<').unwrap_or(0);
        if t.self_close {
            if !id.is_empty() {
                defs.entry(id.to_string()).or_insert_with(|| text[start..pos].to_string());
            }
        } else {
            open.push((!id.is_empty()).then(|| (id.to_string(), start)));
        }
    }
    for v in defs.values_mut() {
        if v.starts_with("<symbol") {
            *v = format!("<g{}", &v["<symbol".len()..]);
            if let Some(k) = v.rfind("</symbol") {
                v.replace_range(k..k + "</symbol".len(), "</g");
            }
        }
    }
    defs
}

pub fn import_svg(text: &str) -> ImportResult {
    import(text, false)
}

/// As `import_svg`, with `<text>` returned in `texts` (position, size, anchor; fonts and rotation are not read).
pub fn import_svg_with_text(text: &str) -> ImportResult {
    import(text, true)
}

fn import(text: &str, keep_text: bool) -> ImportResult {
    let defs = collect_defs(text);
    let mut out = Out { keep: keep_text.then(Vec::new), ..Default::default() };
    scan(text, None, &defs, 0, &mut out);
    let (contours, layers) = join_contours_tagged(out.contours, out.layers, 1e-5).into_iter().unzip();
    let mut note = String::from("SVG");
    if out.texts > 0 {
        note += &format!("; skipped {} text (convert text to outlines)", out.texts);
    }
    if out.skipped > 0 {
        note += &format!("; skipped {} use/image", out.skipped); // C++ wording (golden); uses that resolve are drawn
    }
    ImportResult { contours, layers, note, texts: out.keep.unwrap_or_default(), images: Vec::new(), groups: Vec::new() }
}

/// The characters between a `<text …>` tag ending at `from` and its `</text>`: inner tags dropped (a new `<tspan>`
/// with its own y = a new line), entities decoded.
fn text_content(src: &str, from: usize) -> String {
    let body = &src[from.min(src.len())..];
    let body = &body[..body.find("</text").unwrap_or(body.len())];
    let mut out = String::new();
    let mut rest = body;
    while let Some(i) = rest.find('<') {
        out += &rest[..i];
        let end = rest[i..].find('>').map_or(rest.len(), |j| i + j + 1);
        let tag = &rest[i..end];
        if tag.starts_with("<tspan") && (tag.contains(" y=") || tag.contains(" dy=")) && !out.trim().is_empty() {
            out.push('\n');
        }
        rest = &rest[end..];
    }
    out += rest;
    let mut s = out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#160;", " ").replace("&nbsp;", " ");
    s = s.replace("&amp;", "&");
    s.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// `font-size` of a text element in user units: the attribute, or the style property; 16 when neither.
fn font_size(a: &Attrs) -> f64 {
    let v = attr(a, "font-size");
    let v = if v.is_empty() { attr(a, "style").split(';').find_map(|p| p.trim().strip_prefix("font-size:")).unwrap_or("") } else { v };
    length_px(v.trim()).filter(|&x| x > 0.0).unwrap_or(16.0)
}

/// One pass over a document or a `<use>`d fragment. `init`: the transform / layer the fragment starts in (the root
/// `<svg>` is then not looked for).
fn scan(text: &str, init: Option<Level>, defs: &HashMap<String, String>, depth: u32, out: &mut Out) {
    const SKIP: [&str; 12] =
        ["defs", "clipPath", "mask", "symbol", "pattern", "marker", "metadata", "style", "script", "title", "desc", "foreignObject"];
    const SHAPES: [&str; 7] = ["path", "rect", "circle", "ellipse", "line", "polyline", "polygon"];
    let px_to_in = Affine::scale(1.0 / 96.0, 1.0 / 96.0);

    let mut stack: Vec<Level> = Vec::new();
    let mut skip_depth = 0i32;
    let mut saw_root = init.is_some();
    if let Some(l) = init {
        stack.push(l);
    }
    let mut pos = 0;
    let s = text.as_bytes();
    while let Some(t) = next_tag(s, &mut pos) {
        if SKIP.contains(&t.name.as_str()) {
            if !t.self_close {
                skip_depth += if t.close { -1 } else { 1 };
            }
            continue;
        }
        if skip_depth > 0 {
            continue;
        }
        let container = matches!(t.name.as_str(), "g" | "svg" | "a");
        if t.close {
            if container {
                stack.pop();
            }
            continue;
        }

        let hidden = stack.last().is_some_and(|l| l.hidden)
            || attr(&t.attr, "display") == "none"
            || attr(&t.attr, "style").contains("display:none");
        let mut parent = stack.last().map_or(Affine::default(), |l| l.m);
        let own = parse_transform(attr(&t.attr, "transform"));
        let mut layer = stack.last().map_or(String::new(), |l| l.layer.clone());
        if t.name == "g" && layer.is_empty() {
            layer = attr(&t.attr, "inkscape:label").to_string();
            if layer.is_empty() {
                layer = attr(&t.attr, "id").to_string();
            }
            if layer.starts_with('g') && layer[1..].bytes().all(|c| c.is_ascii_digit()) {
                layer.clear(); // auto ids like g123
            }
        }

        if t.name == "svg" && !saw_root {
            saw_root = true;
            let w = length_px(attr(&t.attr, "width"));
            let h = length_px(attr(&t.attr, "height"));
            let vb: Vec<f64> =
                attr(&t.attr, "viewBox").replace(',', " ").split_whitespace().map_while(|x| x.parse().ok()).take(4).collect();
            // user units -> px, Y flipped
            let px = if vb.len() == 4 && vb[2] > 0.0 && vb[3] > 0.0 {
                let s = match (w, h) {
                    (Some(w), Some(h)) => (w / vb[2]).min(h / vb[3]),
                    (Some(w), None) => w / vb[2],
                    (None, Some(h)) => h / vb[3],
                    _ => 1.0,
                };
                Affine { a: s, b: 0.0, c: 0.0, d: -s, e: -s * vb[0], f: s * (vb[1] + vb[3]) }
            } else {
                Affine { a: 1.0, b: 0.0, c: 0.0, d: -1.0, e: 0.0, f: h.unwrap_or(0.0) }
            };
            if !t.self_close {
                stack.push(Level { m: px_to_in * px, hidden, layer: String::new() });
            }
            continue;
        }
        if container {
            if !t.self_close {
                stack.push(Level { m: parent * own, hidden, layer });
            }
            continue;
        }
        if t.name == "text" {
            let m = stack.last().map(|l| l.m * own);
            match (out.keep.as_mut(), m) {
                (Some(keep), Some(m)) if !hidden && !t.self_close => {
                    let content = text_content(text, pos);
                    if !content.is_empty() {
                        let style = attr(&t.attr, "style");
                        let anchor = [attr(&t.attr, "text-anchor"), style].iter().find_map(|v| if v.contains("middle") { Some(1) } else if v.contains("end") { Some(2) } else { None }).unwrap_or(0);
                        // capitals are about 0.72 of the font size in the usual sans faces
                        let height = font_size(&t.attr) * 0.72 * (m.a * m.d - m.b * m.c).abs().sqrt();
                        keep.push(ImportText { text: content, at: m.apply(pt(num_attr(&t.attr, "x", 0.0), num_attr(&t.attr, "y", 0.0))), height, layer: layer.clone(), anchor, valign: 0, angle: m.b.atan2(m.a).to_degrees() });
                    }
                }
                _ => out.texts += 1,
            }
        } else if t.name == "image" {
            out.skipped += 1;
        } else if t.name == "use" {
            if hidden {
                continue;
            }
            // a clone of an element elsewhere in the file (symbols, repeated shapes, Illustrator / Inkscape clones)
            let href = [attr(&t.attr, "href"), attr(&t.attr, "xlink:href")].into_iter().find(|h| !h.is_empty()).unwrap_or("");
            match href.strip_prefix('#').and_then(|id| defs.get(id)) {
                Some(frag) if depth < MAX_USE_DEPTH && out.uses < 200_000 => {
                    out.uses += 1;
                    let m = parent * own * Affine::translate(num_attr(&t.attr, "x", 0.0), num_attr(&t.attr, "y", 0.0));
                    scan(frag, Some(Level { m, hidden: false, layer }), defs, depth + 1, out);
                }
                _ => out.skipped += 1,
            }
            continue;
        }
        if hidden || !SHAPES.contains(&t.name.as_str()) {
            continue;
        }
        if !saw_root {
            saw_root = true;
            let m = px_to_in * Affine::scale(1.0, -1.0);
            stack.push(Level { m, hidden: false, layer: String::new() });
            parent = m;
        }

        let m = parent * own;
        let d = shape_to_path(&t);
        for c in (PathParser { d: d.as_bytes(), i: 0 }).parse() {
            out.contours.push(transformed(&c, &m));
            out.layers.push(layer.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strtod_prefixes() {
        assert_eq!(strtod(b"10-5"), (10.0, 2));
        assert_eq!(strtod(b"0.5.5"), (0.5, 3));
        assert_eq!(strtod(b" 1e3x"), (1000.0, 4));
        assert_eq!(strtod(b"2e"), (2.0, 1));
        assert_eq!(strtod(b"abc").1, 0);
        assert_eq!(fmt(1.0 / 3.0).parse::<f64>().unwrap(), 0.333333333);
    }
}

/// Vectors as an SVG drawing: real size in inches (`w` x `h` = the board), Y flipped to SVG's top-down, curves kept
/// as cubic Béziers, one `<g id="layer">` per layer name (the importer reads those back as layers).
pub fn export_svg(items: &[(&Contour, &str)], w: f64, h: f64) -> String {
    let n = |v: f64| { let s = format!("{v:.4}"); s.trim_end_matches('0').trim_end_matches('.').to_string() };
    let p = |q: Pt| format!("{} {}", n(q.x), n(h - q.y));
    let mut layers: Vec<&str> = Vec::new();
    for (_, l) in items {
        if !layers.contains(l) {
            layers.push(l);
        }
    }
    let mut out = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}in\" height=\"{}in\" viewBox=\"0 0 {} {}\">\n", n(w), n(h), n(w), n(h));
    for layer in layers {
        let id: String = layer.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        out += &if id.is_empty() { "<g fill=\"none\" stroke=\"#000\" stroke-width=\"0.01\">\n".to_string() } else { format!("<g id=\"{id}\" fill=\"none\" stroke=\"#000\" stroke-width=\"0.01\">\n") };
        for (c, _) in items.iter().filter(|(_, l)| *l == layer) {
            if c.nodes.is_empty() {
                continue;
            }
            let mut d = format!("M{}", p(c.nodes[0].p));
            for i in 0..crate::geom::seg_count(c) {
                let s = crate::geom::segment(c, i);
                d += &if s.curve { format!(" C{} {} {}", p(s.c1), p(s.c2), p(s.p1)) } else { format!(" L{}", p(s.p1)) };
            }
            if c.closed {
                d += " Z";
            }
            out += &format!("<path d=\"{d}\"/>\n");
        }
        out += "</g>\n";
    }
    out + "</svg>\n"
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use crate::geom::{bounds, make_ellipse, make_polyline, make_rect};

    #[test]
    fn svg_and_dxf_exports_read_back_the_same() {
        let (rect, circle, line) = (make_rect(1.0, 1.0, 3.0, 2.0), make_ellipse(6.0, 4.0, 1.5, 1.5), make_polyline(&[pt(0.0, 0.0), pt(2.0, 5.0), pt(4.0, 0.5)], false));
        let items = [(&rect, "Cut"), (&circle, "Cut"), (&line, "Engrave")];
        for (what, back) in [("svg", import_svg(&export_svg(&items, 12.0, 8.0))), ("dxf", crate::dxf::import_dxf(&crate::dxf::export_dxf(&items)))] {
            assert_eq!(back.contours.len(), 3, "{what}: {}", back.note);
            assert_eq!(back.layers, ["Cut", "Cut", "Engrave"], "{what}");
            for (a, (b, _)) in back.contours.iter().zip(items) {
                let (ra, rb) = (bounds(a), bounds(b));
                assert!((ra.x0 - rb.x0).abs() < 0.002 && (ra.y0 - rb.y0).abs() < 0.002 && (ra.x1 - rb.x1).abs() < 0.002 && (ra.y1 - rb.y1).abs() < 0.002, "{what}: {ra:?} vs {rb:?}");
                assert_eq!(a.closed, b.closed, "{what}");
            }
        }
        // the SVG keeps the circle as curves (4 segments), not hundreds of points
        assert!(export_svg(&items, 12.0, 8.0).matches(" C").count() == 4);
    }

    #[test]
    fn text_is_kept_when_asked() {
        let src = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2in" height="1in" viewBox="0 0 192 96">
            <g id="Signs"><rect width="10" height="10"/>
            <text x="96" y="48" font-size="48" text-anchor="middle">Tom &amp; <tspan>Co</tspan></text>
            <text transform="translate(96 48) rotate(-90)" font-size="24">Up</text>
            <text x="0" y="96" style="font-size:24px"><tspan x="0" y="96">One</tspan><tspan x="0" dy="30">Two</tspan></text></g></svg>"#;
        assert!(import_svg(src).note.contains("skipped 3 text") && import_svg(src).texts.is_empty());
        let r = import_svg_with_text(src);
        assert!(!r.note.contains("text"), "{}", r.note);
        let t = &r.texts[0];
        assert_eq!((t.text.as_str(), t.anchor, t.layer.as_str()), ("Tom & Co", 1, "Signs"));
        assert!((t.at.x - 1.0).abs() < 1e-9 && (t.at.y - 0.5).abs() < 1e-9 && (t.height - 0.36).abs() < 1e-9, "{t:?}");
        assert!(t.angle.abs() < 1e-9);
        let up = &r.texts[1]; // reads bottom to top: a quarter turn counter-clockwise, about the point it was moved to
        assert!((up.angle - 90.0).abs() < 1e-9 && (up.at.x - 1.0).abs() < 1e-9 && (up.at.y - 0.5).abs() < 1e-9, "{up:?}");
        assert_eq!(r.texts[2].text, "One\nTwo");
        assert!(r.texts[2].at.y.abs() < 1e-9 && (r.texts[2].height - 0.18).abs() < 1e-9);
    }

    #[test]
    fn use_clones_symbols_and_nested_uses() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="96" height="96" viewBox="0 0 96 96">
          <defs><rect id="r" width="10" height="10"/><symbol id="s"><circle cx="5" cy="5" r="5"/></symbol><use id="again" xlink:href="#r" x="20"/></defs>
          <use xlink:href="#r" x="30" y="40"/><use href="#s" transform="translate(60,0)"/><use href="#again" y="50"/><use href="#missing"/>
          <use id="loop" href="#loop"/></svg>"##;
        let r = import_svg(svg);
        assert_eq!(r.contours.len(), 3, "{}", r.note);
        let mut b: Vec<_> = r.contours.iter().map(crate::geom::bounds).collect();
        b.sort_by(|p, q| p.x0.total_cmp(&q.x0)); // the importer may reorder
        // 96 px = 1 in; y is from the bottom
        assert!((b[0].x0 - 20.0 / 96.0).abs() < 1e-6 && (b[0].y0 - 36.0 / 96.0).abs() < 1e-6, "use of a use: {:?}", b[0]);
        assert!((b[1].x0 - 30.0 / 96.0).abs() < 1e-6 && (b[1].y0 - 46.0 / 96.0).abs() < 1e-6, "{:?}", b[1]);
        assert!((b[2].x0 - 60.0 / 96.0).abs() < 1e-6 && (b[2].y0 - 86.0 / 96.0).abs() < 1e-6, "symbol moved by the use's transform: {:?}", b[2]);
        assert!(r.note.contains("skipped 2 use/image"), "{}", r.note); // #missing and the self-loop
    }
}
