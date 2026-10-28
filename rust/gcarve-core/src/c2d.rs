//! Carbide Create projects (.c2d, the Shapeoko / Nomad software) as a new document: board, vectors, bits and
//! toolpaths. Two formats, both in millimetres with Y up:
//!
//! - up to version 6: one JSON object. DOCUMENT_VALUES (WIDTH, HEIGHT, THICKNESS, RETRACT, …); RECT_OBJECTS
//!   (centre `position`, width, height, rotation°, corner_type 1 = rounded by `radius`), CIRCLE_OBJECTS (centre,
//!   radius), CURVE_OBJECTS (`points` relative to `position`, `point_type` per point: 1 line to it, 2 / 3 cubic to it
//!   through control_point_1 / control_point_2 at the same index, 4 back to the first point = closed), TEXT_OBJECTS
//!   (text, font, bottom-left `position`, `width`); TOOLPATH_OBJECTS with no type field: `vcarve` true or
//!   ofset_dir 3 = V-carve, 2 = pocket, -1 / 0 / 1 = contour inside / on / outside (read off 22 sample projects:
//!   V-bits only ever on 3, cut-outs on 1, holes on -1). Their shapes are `elements[].uuid` (with tabs: `tab_u`, a
//!   fraction of the way round) or `contours`, plus `toolpath_links` {uuid: object, links: [toolpath uuids]}.
//! - version 7 on: an SQLite database. Table `params` (key, value: width, height, thickness, retract, display_mm…),
//!   table `items` (uuid, name, type, version, sz, data = zlib JSON): type "element" with geometryType "path"
//!   (`points`, `cp1`, `cp2`, `point_type` 0 start / 3 cubic / 4 end of a closed path, `position`, `tabs`
//!   [{pos}], `layer` {name}), "toolpath" with a `type` ("contour" + ofset_dir -1 / 0 / 1, "pocket_toolpath",
//!   "drilling_toolpath", …vcarve…) and `elements`. Depths are strings: numbers in mm, a fraction ("1/4") in the
//!   project's display unit. Read by our own small SQLite page reader (`Sqlite`): no SQLite library runs in the web
//!   build, and only two tables are needed.
//!
//! Texts come back apart (`C2dText`): the app sets them in its own font, sized to the width Carbide Create gives,
//! and a toolpath on a text holds the placeholder id -(k + 1) for text k until the app swaps in the letters.

use crate::document::{Document, Op, OpType, Side, Tool, ToolType};
use crate::geom::{make_ellipse, make_rect, pt, transformed, Affine, Builder, Contour, Pt};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const MM: f64 = 1.0 / 25.4;

/// A text object: the app builds its letters
#[derive(Clone, Debug, PartialEq)]
pub struct C2dText {
    pub text: String,
    /// bottom-left, inches
    pub at: Pt,
    /// the width Carbide Create laid it out to, inches (0 = unknown)
    pub width: f64,
    pub angle: f64,
    pub font: String,
}

pub fn import_c2d(bytes: &[u8]) -> Result<(Document, String, Vec<C2dText>), String> {
    if bytes.starts_with(b"SQLite format 3\0") {
        from_sqlite(bytes)
    } else {
        let v: Value = serde_json::from_slice(bytes).map_err(|_| "This isn't a Carbide Create project (.c2d) GCarve can read.".to_string())?;
        from_json(&v)
    }
}

fn num(v: &Value) -> f64 {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).unwrap_or(0.0)
}
fn pt_of(v: &Value) -> Pt {
    pt(num(&v[0]) * MM, num(&v[1]) * MM)
}

/// What the importer builds up: shapes by Carbide Create id, tools, ops, what was skipped
#[derive(Default)]
struct Out {
    doc: Document,
    ids: HashMap<String, Vec<i32>>,
    texts: Vec<C2dText>,
    skipped: HashMap<&'static str, usize>,
    off: usize,
}

impl Out {
    fn add(&mut self, id: &str, c: Contour, layer: Option<i32>) {
        let sid = self.doc.add(c, layer);
        self.ids.entry(id.to_string()).or_default().push(sid);
    }

    fn board(&mut self, w: f64, h: f64, t: f64, retract: f64) {
        let m = &mut self.doc.material;
        (m.w, m.h, m.t) = (w * MM, h * MM, t * MM);
        if retract > 0.0 {
            m.safe_z = retract * MM;
        }
        self.doc.sheets = vec![self.doc.material.clone()];
    }

    /// The bit, shared with earlier toolpaths that use the same one
    fn tool(&mut self, t: &Value, speeds: &Value, stepdown: f64) -> i32 {
        let dia = num(&t["diameter"]) * MM;
        let angle = num(&t["angle"]);
        let corner = num(&t["corner_radius"]) * MM;
        let kind = if angle > 0.0 { ToolType::VBit } else if corner > 0.0 { ToolType::BallNose } else { ToolType::EndMill };
        let name = t["name"].as_str().filter(|s| !s.trim().is_empty()).map(str::to_string).unwrap_or_else(|| match kind {
            ToolType::VBit => format!("{angle}° V-Bit ({:.3} inch)", dia),
            ToolType::BallNose => format!("Ball Nose ({:.3} inch)", dia),
            _ => format!("End Mill ({:.3} inch)", dia),
        });
        let tool = Tool {
            name,
            kind,
            diameter: dia,
            angle: if angle > 0.0 { angle } else { 90.0 },
            radius: if kind == ToolType::BallNose && corner < dia / 2.0 - 1e-6 { corner } else { 0.0 },
            flutes: (num(&t["flutes"]) as i32).max(1),
            cut_length: num(&t["length"]) * MM,
            stepdown: if stepdown > 0.0 { stepdown } else { dia / 2.0 },
            feed_xy: num(&speeds["feedrate"]) * MM,
            feed_z: num(&speeds["plungerate"]) * MM,
            rpm: num(&speeds["rpm"]) as i32,
            ..Default::default()
        };
        let same = |a: &Tool| a.kind == tool.kind && (a.diameter - tool.diameter).abs() < 1e-6 && (a.angle - tool.angle).abs() < 1e-6 && (a.radius - tool.radius).abs() < 1e-6 && a.name == tool.name;
        if let Some(i) = self.doc.tools.iter().position(same) {
            return i as i32;
        }
        self.doc.tools.push(tool);
        self.doc.tools.len() as i32 - 1
    }

    /// One toolpath; `kind` from the format's own rules, `shapes` = Carbide Create ids, `tabs` = points on outlines
    fn op(&mut self, tp: &Value, kind: OpType, side: Side, shapes: &[String], tabs: Vec<[f64; 2]>, depth: f64) {
        if tp["enabled"] == Value::Bool(false) {
            self.off += 1;
            return;
        }
        let stepdown = num(&tp["stepdown"]) * MM;
        let tool = self.tool(&tp["tool"], &tp["speeds"], stepdown);
        let shape_ids: Vec<i32> = shapes.iter().filter_map(|s| self.ids.get(s)).flatten().copied().collect();
        if shape_ids.is_empty() {
            *self.skipped.entry("toolpath with no vectors").or_default() += 1;
            return;
        }
        let dia = self.doc.tools[tool as usize].diameter;
        let mut op = Op { kind, name: tp["name"].as_str().unwrap_or("Toolpath").to_string(), tool, shape_ids, depth, stepdown, side, ..Default::default() };
        if let Some(c) = tp["climb"].as_bool() {
            op.climb = c;
        }
        match kind {
            OpType::Pocket => {
                let so = num(&tp["stepover"]) * MM;
                if so > 0.0 && dia > 0.0 {
                    op.stepover = (so / dia).clamp(0.05, 1.0);
                }
            }
            OpType::Drill => op.peck = num(&tp["peck_distance"]) * MM,
            OpType::VCarve => op.flat_depth = 0.0,
            _ => {}
        }
        if kind == OpType::Profile && !tabs.is_empty() && tp["ignore_tabs"] != Value::Bool(true) {
            op.tabs = true;
            op.tab_points = tabs;
            op.tab_width = num(&tp["tab_width"]) * MM;
            op.tab_height = num(&tp["tab_height"]) * MM;
        }
        self.doc.ops.push(op);
    }

    fn finish(mut self, version: &str) -> (Document, String, Vec<C2dText>) {
        self.doc.material = self.doc.sheets[0].clone();
        let mut note = format!("Carbide Create {version} project: {} vectors, {} toolpaths", self.doc.shapes.len() + self.texts.len(), self.doc.ops.len());
        if self.off > 0 {
            note += &format!(", {} switched off there left out", self.off);
        }
        let mut sk: Vec<_> = self.skipped.into_iter().collect();
        sk.sort();
        for (what, n) in sk {
            note += &format!("; skipped {n} {what}{}", if n == 1 { "" } else { "s" });
        }
        if !self.texts.is_empty() {
            note += "; text set in GCarve's font (Carbide Create's fonts aren't in the file)";
        }
        (self.doc, note, self.texts)
    }
}

/// A Carbide Create path: per point its type and the two control points of the piece that ends there
fn path(points: &[Value], cp1: &[Value], cp2: &[Value], types: &[Value], at: Affine) -> Vec<Contour> {
    let mut out = Vec::new();
    let mut b: Option<Builder> = None;
    let p = |v: &Value| at.apply(pt_of(v));
    for i in 0..points.len().min(types.len()).min(200_000) {
        let ty = types[i].as_i64().unwrap_or(1);
        match (ty, b.as_mut()) {
            (0, _) | (_, None) => {
                if let Some(done) = b.take() {
                    if done.c.nodes.len() > 1 {
                        out.push(done.c);
                    }
                }
                let mut nb = Builder::default();
                nb.start(p(&points[i]));
                b = Some(nb);
            }
            (4, Some(bb)) => {
                bb.close();
                out.push(b.take().unwrap().c);
            }
            (2 | 3, Some(bb)) => match (cp1.get(i), cp2.get(i)) {
                (Some(c1), Some(c2)) => bb.cubic(p(c1), p(c2), p(&points[i])),
                _ => bb.line(p(&points[i])),
            },
            (_, Some(bb)) => bb.line(p(&points[i])),
        }
    }
    if let Some(done) = b {
        if done.c.nodes.len() > 1 {
            out.push(done.c);
        }
    }
    out
}

/// Points `u` (0..1 of the way round) on an outline, for tabs
fn along(c: &[Contour], u: &[f64]) -> Vec<[f64; 2]> {
    let Some(c) = c.first() else { return Vec::new() };
    let f = crate::geom::flatten(c, 0.002);
    let lens: Vec<f64> = std::iter::once(0.0).chain(f.windows(2).scan(0.0, |s, w| { *s += crate::geom::dist(w[0], w[1]); Some(*s) })).collect();
    let total = *lens.last().unwrap_or(&0.0);
    u.iter().filter_map(|&u| {
        let s = u.clamp(0.0, 1.0) * total;
        let i = lens.partition_point(|&l| l < s).clamp(1, f.len().max(2) - 1);
        let (a, b) = (f.get(i - 1)?, f.get(i)?);
        let t = if lens[i] > lens[i - 1] { (s - lens[i - 1]) / (lens[i] - lens[i - 1]) } else { 0.0 };
        Some([a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t])
    }).collect()
}

fn rotate_about(deg: f64, c: Pt) -> Affine {
    Affine::translate(c.x, c.y) * Affine::rotate(deg.to_radians()) * Affine::translate(-c.x, -c.y)
}

fn from_json(v: &Value) -> Result<(Document, String, Vec<C2dText>), String> {
    let d = &v["DOCUMENT_VALUES"];
    if !d.is_object() {
        return Err("This isn't a Carbide Create project (.c2d) GCarve can read.".into());
    }
    let mut o = Out::default();
    o.board(num(&d["WIDTH"]), num(&d["HEIGHT"]), num(&d["THICKNESS"]), num(&d["RETRACT"]));
    let empty = Vec::new();
    let list = |k: &str| v[k].as_array().unwrap_or(&empty);
    for r in list("RECT_OBJECTS") {
        let (c, w, h) = (pt_of(&r["position"]), num(&r["width"]) * MM, num(&r["height"]) * MM);
        let mut rect = make_rect(c.x - w / 2.0, c.y - h / 2.0, c.x + w / 2.0, c.y + h / 2.0);
        if r["corner_type"].as_i64() == Some(1) && num(&r["radius"]) > 0.0 {
            rect = crate::vecops::fillet_contours(&[rect.clone()], (num(&r["radius"]) * MM).min(w.min(h) / 2.0 - 1e-4), true, false).into_iter().next().unwrap_or(rect);
        }
        o.add(r["id"].as_str().unwrap_or(""), transformed(&rect, &rotate_about(num(&r["rotation"]), c)), None);
    }
    for r in list("CIRCLE_OBJECTS") {
        let c = pt_of(&r["position"]);
        o.add(r["id"].as_str().unwrap_or(""), make_ellipse(c.x, c.y, num(&r["radius"]) * MM, num(&r["radius"]) * MM), None);
    }
    for r in list("CURVE_OBJECTS") {
        let at = pt_of(&r["position"]);
        let m = rotate_about(num(&r["rotation"]), at) * Affine::translate(at.x, at.y);
        let arr = |k: &str| r[k].as_array().cloned().unwrap_or_default();
        for c in path(&arr("points"), &arr("control_point_1"), &arr("control_point_2"), &arr("point_type"), m) {
            o.add(r["id"].as_str().unwrap_or(""), c, None);
        }
    }
    let n = list("REGULAR_POLYGON_OBJECTS").len();
    if n > 0 {
        o.skipped.insert("polygon", n);
    }
    for t in list("TEXT_OBJECTS") {
        let k = o.texts.len() as i32;
        o.texts.push(C2dText { text: t["text"].as_str().unwrap_or("").to_string(), at: pt_of(&t["position"]), width: num(&t["width"]) * MM, angle: num(&t["rotation"]), font: t["font"].as_str().unwrap_or("").to_string() });
        o.ids.insert(t["id"].as_str().unwrap_or("").to_string(), vec![-(k + 1)]);
    }
    // toolpath uuid -> object ids, from the links table (newer v6 files)
    let mut linked: HashMap<String, Vec<String>> = HashMap::new();
    for l in list("toolpath_links") {
        let obj = l["uuid"].as_str().unwrap_or("").to_string();
        for t in l["links"].as_array().unwrap_or(&empty) {
            linked.entry(t.as_str().unwrap_or("").to_string()).or_default().push(obj.clone());
        }
    }
    for tp in list("TOOLPATH_OBJECTS") {
        let dir = tp["ofset_dir"].as_i64().unwrap_or(0);
        let kind = if tp["vcarve"] == Value::Bool(true) || dir == 3 { OpType::VCarve } else if dir == 2 { OpType::Pocket } else { OpType::Profile };
        let side = match dir { -1 => Side::Inside, 1 => Side::Outside, _ => Side::On };
        let mut shapes: Vec<String> = linked.get(tp["uuid"].as_str().unwrap_or("")).cloned().unwrap_or_default();
        let mut tabs = Vec::new();
        for e in tp["elements"].as_array().unwrap_or(&empty).iter().chain(tp["contours"].as_array().unwrap_or(&empty)) {
            let id = e["uuid"].as_str().or(e.as_str()).unwrap_or("").to_string();
            if let Some(us) = e["tab_u"].as_array() {
                let outline: Vec<Contour> = o.ids.get(&id).map(|ids| ids.iter().filter_map(|i| o.doc.find(*i)).map(|s| s.c.clone()).collect()).unwrap_or_default();
                tabs.extend(along(&outline, &us.iter().map(num).collect::<Vec<_>>()));
            }
            if !shapes.contains(&id) {
                shapes.push(id);
            }
        }
        let depth = (num(&tp["end_depth"]).abs() * MM).max(0.0);
        o.op(tp, kind, side, &shapes, tabs, depth);
    }
    Ok(o.finish("(up to version 6)"))
}

/// A depth string of a version 7 file: a plain number is mm, a fraction ("1/4", "1 1/2") the display unit
fn depth7(v: &Value, display_mm: bool) -> f64 {
    let Some(s) = v.as_str() else { return num(v).abs() * MM };
    let s = s.trim().trim_end_matches('"').trim();
    if !s.contains('/') {
        return s.parse::<f64>().map(|x| x.abs() * MM).unwrap_or(0.0);
    }
    let frac = |t: &str| t.split_once('/').and_then(|(a, b)| Some(a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok().filter(|b| *b != 0.0)?));
    let val = match s.split_once(' ') {
        Some((whole, f)) => whole.trim().parse::<f64>().unwrap_or(0.0) + frac(f).unwrap_or(0.0),
        None => frac(s).unwrap_or(0.0),
    };
    if display_mm { val.abs() * MM } else { val.abs() }
}

fn from_sqlite(bytes: &[u8]) -> Result<(Document, String, Vec<C2dText>), String> {
    let bad = |e: &str| format!("This Carbide Create file can't be read ({e}).");
    let db = Sqlite::new(bytes).ok_or_else(|| bad("not a database GCarve understands"))?;
    let params: HashMap<String, String> = db.rows("params").ok_or_else(|| bad("no project settings"))?.into_iter().filter_map(|r| Some((r.first()?.text()?, r.get(1)?.text()?))).collect();
    let p = |k: &str| params.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    let mut o = Out::default();
    o.board(p("width"), p("height"), p("thickness"), p("retract"));
    let display_mm = params.get("display_mm").is_some_and(|v| v == "1");
    let mut toolpaths = Vec::new();
    let mut budget = 256usize << 20; // inflated bytes per file
    for r in db.rows("items").ok_or_else(|| bad("no drawing"))? {
        // items(id, uuid, name, type, version, sz, data)
        let (Some(ty), Some(Cell::Blob(data))) = (r.get(3).and_then(Cell::text), r.get(6)) else { continue };
        if ty != "element" && ty != "toolpath" {
            continue;
        }
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, 64 << 20).ok().filter(|r| r.len() <= budget);
        budget -= raw.as_ref().map_or(0, Vec::len);
        let Some(v) = raw.and_then(|raw| serde_json::from_slice::<Value>(&raw).ok()) else {
            *o.skipped.entry("unreadable item").or_default() += 1;
            continue;
        };
        if ty == "toolpath" {
            toolpaths.push(v);
            continue;
        }
        if v["geometryType"].as_str() != Some("path") {
            *o.skipped.entry("non-path shape").or_default() += 1;
            continue;
        }
        let layer = v["layer"]["name"].as_str().filter(|n| !n.is_empty() && *n != "DEFAULT").map(|n| o.doc.layer_named(n));
        let at = pt_of(&v["position"]);
        let arr = |k: &str| v[k].as_array().cloned().unwrap_or_default();
        for c in path(&arr("points"), &arr("cp1"), &arr("cp2"), &arr("point_type"), Affine::translate(at.x, at.y)) {
            o.add(v["id"].as_str().unwrap_or(""), c, layer);
        }
    }
    // tabs live on the elements in version 7
    for tp in toolpaths {
        let ty = tp["type"].as_str().unwrap_or("");
        let kind = if ty.contains("vcarve") || ty.contains("v_carve") { OpType::VCarve } else if ty.contains("pocket") { OpType::Pocket } else if ty.contains("drill") { OpType::Drill } else if ty == "contour" { OpType::Profile } else {
            *o.skipped.entry("toolpath of another kind").or_default() += 1;
            continue;
        };
        let side = match tp["ofset_dir"].as_i64() { Some(-1) => Side::Inside, Some(1) => Side::Outside, _ => Side::On };
        let shapes: Vec<String> = tp["elements"].as_array().map(|a| a.iter().filter_map(|e| e["uuid"].as_str().map(str::to_string)).collect()).unwrap_or_default();
        let depth = depth7(&tp["end_depth"], display_mm);
        o.op(&tp, kind, side, &shapes, Vec::new(), depth);
    }
    Ok(o.finish(format!("{} (version 7+)", params.get("build_num").map(|b| format!("build {b}")).unwrap_or_default()).trim()))
}

// ---- a small read-only SQLite reader: table b-trees, records, overflow pages (https://sqlite.org/fileformat.html)

#[derive(Clone, Debug, PartialEq)]
enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Cell {
    fn text(&self) -> Option<String> {
        match self {
            Cell::Text(s) => Some(s.clone()),
            Cell::Int(i) => Some(i.to_string()),
            Cell::Real(f) => Some(f.to_string()),
            _ => None,
        }
    }
}

struct Sqlite<'a> {
    b: &'a [u8],
    page: usize,
    usable: usize,
}

fn varint(b: &[u8], mut p: usize) -> Option<(u64, usize)> {
    let mut v: u64 = 0;
    for i in 0..9 {
        let c = *b.get(p)?;
        p += 1;
        if i == 8 {
            return Some(((v << 8) | c as u64, p));
        }
        v = (v << 7) | (c & 0x7f) as u64;
        if c & 0x80 == 0 {
            return Some((v, p));
        }
    }
    None
}

impl<'a> Sqlite<'a> {
    fn new(b: &'a [u8]) -> Option<Self> {
        let raw = u16::from_be_bytes([*b.get(16)?, *b.get(17)?]) as usize;
        let page = if raw == 1 { 65536 } else { raw };
        if !(512..=65536).contains(&page) || !page.is_power_of_two() {
            return None;
        }
        Some(Sqlite { b, page, usable: page - *b.get(20)? as usize })
    }

    fn pg(&self, n: u32) -> Option<&'a [u8]> {
        let s = (n as usize).checked_sub(1)?.checked_mul(self.page)?;
        self.b.get(s..s + self.page)
    }

    /// Every row of a table b-tree, as payload bytes
    fn walk(&self, root: u32, out: &mut Vec<Vec<u8>>, depth: u32, seen: &mut HashSet<u32>) -> Option<()> {
        if depth > 20 || out.len() > 1_000_000 || !seen.insert(root) {
            return None; // a b-tree never visits a page twice: a page pointing at itself would fan out
        }
        let d = self.pg(root)?;
        let h = if root == 1 { 100 } else { 0 };
        let kind = *d.get(h)?;
        let cells = u16::from_be_bytes([*d.get(h + 3)?, *d.get(h + 4)?]) as usize;
        let ptrs = h + if kind == 5 { 12 } else { 8 };
        for i in 0..cells {
            let at = u16::from_be_bytes([*d.get(ptrs + 2 * i)?, *d.get(ptrs + 2 * i + 1)?]) as usize;
            if kind == 5 {
                let child = u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?);
                self.walk(child, out, depth + 1, seen)?;
            } else if kind == 13 {
                let (size, p) = varint(d, at)?;
                let (_, p) = varint(d, p)?;
                out.push(self.payload(d, p, size as usize)?);
            } else {
                return None;
            }
        }
        if kind == 5 {
            self.walk(u32::from_be_bytes(d.get(h + 8..h + 12)?.try_into().ok()?), out, depth + 1, seen)?;
        }
        Some(())
    }

    /// A cell's payload: the local part, then the overflow chain
    fn payload(&self, d: &[u8], p: usize, size: usize) -> Option<Vec<u8>> {
        if size > self.b.len() {
            return None; // no cell holds more than the file
        }
        let u = self.usable;
        let x = u - 35;
        if size <= x {
            return Some(d.get(p..p + size)?.to_vec());
        }
        let m = (u - 12) * 32 / 255 - 23;
        let k = m + (size - m) % (u - 4);
        let local = if k <= x { k } else { m };
        let mut out = d.get(p..p + local)?.to_vec();
        let mut next = u32::from_be_bytes(d.get(p + local..p + local + 4)?.try_into().ok()?);
        let mut hops = 0;
        while out.len() < size && next != 0 && hops < self.b.len() / self.page {
            let o = self.pg(next)?;
            let take = (size - out.len()).min(u - 4);
            out.extend_from_slice(o.get(4..4 + take)?);
            next = u32::from_be_bytes(o.get(0..4)?.try_into().ok()?);
            hops += 1;
        }
        (out.len() == size).then_some(out)
    }

    fn record(r: &[u8]) -> Option<Vec<Cell>> {
        let (hl, mut p) = varint(r, 0)?;
        let mut types = Vec::new();
        while p < hl as usize {
            let (t, q) = varint(r, p)?;
            types.push(t);
            p = q;
        }
        let mut at = hl as usize;
        let mut out = Vec::new();
        for t in types {
            let int = |n: usize, at: usize| -> Option<i64> {
                let s = r.get(at..at + n)?;
                let mut v: i64 = if s.first()? & 0x80 != 0 { -1 } else { 0 };
                for &c in s {
                    v = (v << 8) | c as i64;
                }
                Some(v)
            };
            let (cell, n) = match t {
                0 => (Cell::Null, 0),
                1..=4 => (Cell::Int(int(t as usize, at)?), t as usize),
                5 => (Cell::Int(int(6, at)?), 6),
                6 => (Cell::Int(int(8, at)?), 8),
                7 => (Cell::Real(f64::from_be_bytes(r.get(at..at + 8)?.try_into().ok()?)), 8),
                8 => (Cell::Int(0), 0),
                9 => (Cell::Int(1), 0),
                t if t >= 12 && t % 2 == 0 => {
                    let n = (t as usize - 12) / 2;
                    (Cell::Blob(r.get(at..at + n)?.to_vec()), n)
                }
                t if t >= 13 => {
                    let n = (t as usize - 13) / 2;
                    (Cell::Text(String::from_utf8_lossy(r.get(at..at + n)?).into_owned()), n)
                }
                _ => return None,
            };
            out.push(cell);
            at += n;
        }
        Some(out)
    }

    /// The rows of a table, by name (from sqlite_master: type, name, tbl_name, rootpage, sql)
    fn rows(&self, table: &str) -> Option<Vec<Vec<Cell>>> {
        let mut master = Vec::new();
        self.walk(1, &mut master, 0, &mut HashSet::new())?;
        let root = master.iter().filter_map(|r| Self::record(r)).find(|r| r.first() == Some(&Cell::Text("table".into())) && r.get(1) == Some(&Cell::Text(table.into())))?;
        let Some(Cell::Int(root)) = root.get(3) else { return None };
        let mut rows = Vec::new();
        self.walk(u32::try_from(*root).ok()?, &mut rows, 0, &mut HashSet::new())?;
        Some(rows.iter().filter_map(|r| Self::record(r)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::bounds;

    #[test]
    fn a_version_6_project_brings_its_board_shapes_and_toolpaths() {
        let json = r##"{"DOCUMENT_VALUES": {"WIDTH": 254, "HEIGHT": 127, "THICKNESS": 19.05, "RETRACT": 6.35, "DISPLAYMM": false},
          "RECT_OBJECTS": [{"id": "r1", "position": [127, 63.5], "width": 203.2, "height": 101.6, "rotation": 0, "corner_type": 1, "radius": 12.7}],
          "CIRCLE_OBJECTS": [{"id": "c1", "position": [25.4, 25.4], "radius": 6.35}],
          "CURVE_OBJECTS": [{"id": "k1", "position": [50.8, 50.8], "points": [[0,0],[25.4,0],[25.4,25.4],[0,0]], "control_point_1": [[0,0],[0,0],[25.4,12.7],[0,0]], "control_point_2": [[0,0],[0,0],[25.4,12.7],[0,0]], "point_type": [1,1,3,4]}],
          "TEXT_OBJECTS": [{"id": "t1", "text": "HELLO", "font": "Arial", "position": [76.2, 50.8], "width": 101.6, "rotation": 0}],
          "REGULAR_POLYGON_OBJECTS": [],
          "TOOLPATH_OBJECTS": [
            {"name": "Holes", "ofset_dir": -1, "enabled": true, "end_depth": -19.05, "stepdown": 3.175, "stepover": 1, "speeds": {"feedrate": 1270, "plungerate": 254, "rpm": 18000}, "tool": {"diameter": 3.175, "angle": 0, "corner_radius": 0, "flutes": 2, "name": "#102"}, "contours": ["c1"], "uuid": "p1", "vcarve": false},
            {"name": "Words", "ofset_dir": 3, "enabled": true, "end_depth": -6.35, "stepdown": 1, "stepover": 1, "speeds": {"feedrate": 1270, "plungerate": 254, "rpm": 18000}, "tool": {"diameter": 12.7, "angle": 90, "corner_radius": 0, "flutes": 2, "name": ""}, "elements": [{"uuid": "t1"}], "uuid": "p2"},
            {"name": "Cut out", "ofset_dir": 1, "enabled": true, "end_depth": -19.05, "stepdown": 3.175, "stepover": 1, "tab_width": 6.35, "tab_height": 3.175, "speeds": {"feedrate": 1270, "plungerate": 254, "rpm": 18000}, "tool": {"diameter": 3.175, "angle": 0, "corner_radius": 0, "flutes": 2, "name": "#102"}, "elements": [{"uuid": "r1", "tab_u": [0.25, 0.75], "tab_index": [0, 0]}], "uuid": "p3"},
            {"name": "Off", "ofset_dir": 2, "enabled": false, "end_depth": -3, "stepdown": 1, "stepover": 1, "speeds": {}, "tool": {"diameter": 3.175}, "elements": [{"uuid": "k1"}], "uuid": "p4"}]}"##;
        let (doc, note, texts) = import_c2d(json.as_bytes()).unwrap();
        assert!((doc.material.w - 10.0).abs() < 1e-9 && (doc.material.h - 5.0).abs() < 1e-9 && (doc.material.t - 0.75).abs() < 1e-9);
        assert_eq!(doc.shapes.len(), 3);
        let r = bounds(&doc.shapes[0].c);
        assert!((r.x0 - 1.0).abs() < 1e-6 && (r.y1 - 4.5).abs() < 1e-6, "centred rect: {r:?}");
        let k = &doc.shapes[2].c;
        assert!(k.closed && (bounds(k).x1 - 3.0).abs() < 0.01, "curve offset by its position");
        assert_eq!((texts.len(), texts[0].text.as_str(), texts[0].font.as_str()), (1, "HELLO", "Arial"));
        assert!(crate::geom::dist(texts[0].at, pt(3.0, 2.0)) < 1e-9 && (texts[0].width - 4.0).abs() < 1e-9);
        assert_eq!(doc.ops.len(), 3, "{note}");
        assert!(note.contains("1 switched off"), "{note}");
        let (holes, words, cut) = (&doc.ops[0], &doc.ops[1], &doc.ops[2]);
        assert_eq!((holes.kind, holes.side, holes.shape_ids.clone()), (OpType::Profile, Side::Inside, vec![doc.shapes[1].id]));
        assert!((holes.depth - 0.75).abs() < 1e-9 && (doc.tools[holes.tool as usize].feed_xy - 50.0).abs() < 1e-9);
        assert_eq!((words.kind, words.shape_ids.clone()), (OpType::VCarve, vec![-1]), "text placeholder");
        assert_eq!(doc.tools[words.tool as usize].kind, ToolType::VBit);
        assert_eq!(cut.side, Side::Outside);
        assert!(cut.tabs && cut.tab_points.len() == 2 && (cut.tab_height - 0.125).abs() < 1e-9);
        assert_eq!(cut.tool, holes.tool, "the same bit once");
        // a tab point lies on the outline
        let on = |p: [f64; 2]| crate::geom::nearest(&doc.shapes[0].c, pt(p[0], p[1])).d < 0.01;
        assert!(cut.tab_points.iter().all(|p| on(*p)));
    }

    #[test]
    fn version_7_depths_read_mm_numbers_and_inch_fractions() {
        assert!((depth7(&Value::String("20.955".into()), false) - 0.825).abs() < 1e-9);
        assert!((depth7(&Value::String("1/4".into()), false) - 0.25).abs() < 1e-9);
        assert!((depth7(&Value::String("1 1/2".into()), false) - 1.5).abs() < 1e-9);
        assert!((depth7(&Value::String("1/4".into()), true) - 0.25 / 25.4).abs() < 1e-9);
    }

    /// testdata/c2d/small.c2d: a version 7 database made with the sqlite3 tool (testdata/c2d/make.py), one element
    /// big enough to need overflow pages
    #[test]
    fn a_version_7_database_is_read_by_the_page_reader() {
        let bytes = include_bytes!("../../testdata/c2d/small.c2d");
        let (doc, note, _) = import_c2d(bytes).unwrap();
        assert!((doc.material.w - 12.0).abs() < 1e-6 && (doc.material.t - 0.75).abs() < 1e-6, "{note}");
        assert_eq!(doc.shapes.len(), 2, "{note}");
        let big = doc.shapes.iter().map(|s| s.c.nodes.len()).max().unwrap();
        assert!(big > 400, "the long path came through its overflow pages: {big}");
        assert_eq!(doc.ops.len(), 2, "{note}");
        assert_eq!((doc.ops[0].kind, doc.ops[0].side), (OpType::Profile, Side::Outside));
        assert!((doc.ops[0].depth - 0.825).abs() < 1e-9);
        assert_eq!(doc.ops[1].kind, OpType::Pocket);
        assert!((doc.ops[1].depth - 0.25).abs() < 1e-9, "1/4 = inches when the project shows inches");
        assert_eq!(doc.layers.iter().filter(|l| l.name == "Parts").count(), 1);
    }

    /// GCARVE_C2D_DIR=folder cargo test -p gcarve-core --lib c2d_survey -- --ignored --nocapture
    #[test]
    #[ignore]
    fn c2d_survey() {
        let Ok(dir) = std::env::var("GCARVE_C2D_DIR") else { return };
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let b = std::fs::read(e.path()).unwrap();
            match import_c2d(&b) {
                Ok((d, note, t)) => println!("{:60} {:.1}×{:.1}×{:.2}  {} texts  {note}", e.file_name().to_string_lossy(), d.material.w, d.material.h, d.material.t, t.len()),
                Err(err) => println!("{:60} ERROR {err}", e.file_name().to_string_lossy()),
            }
        }
    }
}
