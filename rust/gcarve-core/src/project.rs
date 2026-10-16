//! GCarve project files (.tlp JSON, C++ app/project.cpp) and the clipboard format. Everything in Document except
//! undo history. Output matches Qt's QJsonDocument byte for byte (sorted keys, 4-space indent, Qt number format), so
//! files round-trip between the C++ and Rust apps without diffs. No file I/O: strings in, strings out.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::document::*;
use crate::geom::{transformed, Affine, Node, Pt};

const VERSION: i64 = 1;

// ---- Qt QJsonValue accessors (same defaults and conversions) ------------------------------------------------------

pub(crate) fn get<'a>(o: &'a Value, k: &str) -> &'a Value {
    o.get(k).unwrap_or(&Value::Null)
}
fn to_int(v: &Value, def: i32) -> i32 {
    if let Some(i) = v.as_i64() {
        return i32::try_from(i).unwrap_or(def);
    }
    match v.as_f64() {
        Some(d) if d.fract() == 0.0 && d >= i32::MIN as f64 && d <= i32::MAX as f64 => d as i32,
        _ => def,
    }
}
fn to_f64(v: &Value, def: f64) -> f64 {
    v.as_f64().unwrap_or(def)
}
fn to_bool(v: &Value, def: bool) -> bool {
    v.as_bool().unwrap_or(def)
}
fn to_str(v: &Value, def: &str) -> String {
    v.as_str().unwrap_or(def).to_string()
}
pub(crate) fn arr(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], |a| a.as_slice())
}
fn int(o: &Value, k: &str, def: i32) -> i32 {
    to_int(get(o, k), def)
}
fn num(o: &Value, k: &str, def: f64) -> f64 {
    to_f64(get(o, k), def)
}
fn flag(o: &Value, k: &str, def: bool) -> bool {
    to_bool(get(o, k), def)
}
pub(crate) fn s(o: &Value, k: &str, def: &str) -> String {
    to_str(get(o, k), def)
}

fn pt_json(p: Pt) -> Value {
    json!([p.x, p.y])
}
fn to_pt(v: &Value) -> Pt {
    let a = arr(v);
    let at = |i: usize| a.get(i).map_or(0.0, |x| to_f64(x, 0.0));
    Pt { x: at(0), y: at(1) }
}
fn bind_json(b: &Bind) -> Value {
    Value::Object(b.iter().map(|(k, e)| (k.clone(), Value::from(e.as_str()))).collect())
}
fn bind_from(v: &Value) -> Bind {
    v.as_object().map_or_else(Bind::new, |o| o.iter().map(|(k, e)| (k.clone(), to_str(e, ""))).collect())
}

fn node_json(n: &Node) -> Value {
    let mut o = Map::new();
    o.insert("p".into(), pt_json(n.p));
    if n.has_in {
        o.insert("in".into(), pt_json(n.inh));
    }
    if n.has_out {
        o.insert("out".into(), pt_json(n.out));
    }
    Value::Object(o)
}
fn node_from(o: &Value) -> Node {
    let mut n = Node { p: to_pt(get(o, "p")), ..Default::default() };
    if let Some(v) = o.get("in") {
        n.inh = to_pt(v);
        n.has_in = true;
    }
    if let Some(v) = o.get("out") {
        n.out = to_pt(v);
        n.has_out = true;
    }
    n
}

const TYPES: [&str; 13] = ["profile", "pocket", "drill", "vcarve", "bore", "inlay", "flute", "relief", "chamfer", "prism", "imported", "texture", "photo"];
const OP_TYPES: [OpType; 13] = [
    OpType::Profile,
    OpType::Pocket,
    OpType::Drill,
    OpType::VCarve,
    OpType::Bore,
    OpType::Inlay,
    OpType::Flute,
    OpType::Relief,
    OpType::Chamfer,
    OpType::Prism,
    OpType::Imported,
    OpType::Texture,
    OpType::Photo,
];
const SIDES: [&str; 3] = ["outside", "inside", "on"];

pub fn tool_to_json(t: &Tool) -> Value {
    let presets: Vec<Value> = t
        .presets
        .iter()
        .map(|p| json!({"material": p.material, "feedXY": p.feed_xy, "feedZ": p.feed_z, "stepdown": p.stepdown, "rpm": p.rpm}))
        .collect();
    let kind = match t.kind {
        ToolType::VBit => "vbit",
        ToolType::BallNose => "ball",
        ToolType::RoundOver => "roundover",
        ToolType::EndMill => "endmill",
    };
    let mut o = json!({"name": t.name, "type": kind, "diameter": t.diameter, "angle": t.angle, "stepdown": t.stepdown, "feedXY": t.feed_xy,
           "feedZ": t.feed_z, "rpm": t.rpm, "flutes": t.flutes, "presets": presets});
    // only when set, so older files re-save byte for byte (the C++ app ignores these keys)
    if !t.catalog.is_empty() {
        o.as_object_mut().unwrap().insert("catalog".into(), json!(t.catalog));
    }
    if !t.chip_loads.is_empty() {
        o.as_object_mut().unwrap().insert("chipLoads".into(), json!(t.chip_loads));
    }
    if t.radius > 0.0 {
        o.as_object_mut().unwrap().insert("radius".into(), json!(t.radius));
    }
    if t.slot > 0 {
        o.as_object_mut().unwrap().insert("slot".into(), json!(t.slot));
    }
    if t.cut_length > 0.0 {
        o.as_object_mut().unwrap().insert("cutLength".into(), json!(t.cut_length));
    }
    o
}

pub fn tool_from_json(o: &Value) -> Tool {
    let kind = match get(o, "type").as_str() {
        Some("vbit") => ToolType::VBit,
        Some("ball") => ToolType::BallNose,
        Some("roundover") => ToolType::RoundOver,
        _ => ToolType::EndMill,
    };
    let mut t = Tool {
        name: s(o, "name", "Tool"),
        kind,
        diameter: num(o, "diameter", 0.25),
        angle: num(o, "angle", 90.0),
        stepdown: num(o, "stepdown", 0.25),
        feed_xy: num(o, "feedXY", 100.0),
        feed_z: num(o, "feedZ", 50.0),
        rpm: int(o, "rpm", 18000),
        flutes: int(o, "flutes", 2),
        ..Default::default()
    };
    t.catalog = s(o, "catalog", "");
    t.slot = int(o, "slot", 0).max(0);
    t.radius = num(o, "radius", 0.0).max(0.0);
    t.cut_length = num(o, "cutLength", 0.0).max(0.0);
    if let Some(m) = get(o, "chipLoads").as_object() {
        for (k, v) in m {
            if let (Some(lo), Some(hi)) = (v.get(0).and_then(Value::as_f64), v.get(1).and_then(Value::as_f64)) {
                t.chip_loads.insert(k.clone(), [lo, hi]);
            }
        }
    }
    for p in arr(get(o, "presets")) {
        if s(p, "material", "").is_empty() {
            continue;
        }
        t.presets.push(FeedPreset {
            material: s(p, "material", ""),
            feed_xy: num(p, "feedXY", t.feed_xy),
            feed_z: num(p, "feedZ", t.feed_z),
            stepdown: num(p, "stepdown", t.stepdown),
            rpm: int(p, "rpm", t.rpm),
        });
    }
    t
}

fn shape_json(sh: &Shape) -> Value {
    let nodes: Vec<Value> = sh.c.nodes.iter().map(node_json).collect();
    let mut o = json!({"id": sh.id, "layer": sh.layer, "closed": sh.c.closed, "nodes": nodes});
    let m = o.as_object_mut().unwrap();
    if sh.group != 0 {
        m.insert("group".into(), sh.group.into());
    }
    if sh.sheet != 0 {
        m.insert("sheet".into(), sh.sheet.into());
    }
    if sh.text != 0 {
        m.insert("text".into(), sh.text.into());
    }
    if !sh.bind.is_empty() {
        m.insert("bind".into(), bind_json(&sh.bind));
    }
    o
}
fn shape_from(o: &Value) -> Shape {
    let mut sh = Shape {
        id: int(o, "id", 0),
        layer: int(o, "layer", 0),
        group: int(o, "group", 0),
        sheet: int(o, "sheet", 0),
        text: int(o, "text", 0),
        bind: bind_from(get(o, "bind")),
        ..Default::default()
    };
    sh.c.closed = flag(o, "closed", false);
    sh.c.nodes = arr(get(o, "nodes")).iter().map(node_from).collect();
    sh
}

fn text_json(t: &TextItem) -> Value {
    let mut o = json!({"id": t.id, "text": t.text, "family": t.family, "height": t.height, "bold": t.bold, "italic": t.italic,
           "path": t.path_shape, "start": t.start_offset, "spacing": t.spacing, "above": t.above});
    // only when set, so older files re-save byte for byte
    if t.align != 0 {
        o["align"] = json!(t.align);
    }
    if t.weld {
        o["weld"] = json!(true);
    }
    if t.angle != 0.0 {
        o["angle"] = json!(t.angle);
    }
    o
}
fn text_from(o: &Value) -> TextItem {
    TextItem {
        id: int(o, "id", 0),
        text: s(o, "text", ""),
        family: s(o, "family", ""),
        height: num(o, "height", 1.0),
        bold: flag(o, "bold", false),
        italic: flag(o, "italic", false),
        path_shape: int(o, "path", 0),
        start_offset: num(o, "start", 0.0),
        spacing: num(o, "spacing", 0.0),
        above: flag(o, "above", true),
        align: int(o, "align", 0).clamp(0, 2),
        weld: flag(o, "weld", false),
        angle: num(o, "angle", 0.0),
    }
}

fn constraint_json(c: &Constraint) -> Value {
    json!({"anchor": c.anchor, "target": c.target, "type": c.kind, "expr": c.expr})
}

pub(crate) fn op_json(op: &Op, tools: &[Tool]) -> Value {
    let entry = match op.entry {
        Entry::Spiral => "spiral",
        Entry::Ramp => "ramp",
        Entry::Plunge => "plunge",
    };
    let mut o = json!({
        "type": TYPES[op.kind as usize], "name": op.name, "shapes": op.shape_ids, "tool": op.tool, "layerScope": op.layer_scope,
        "groupScope": op.group_scope, "side": SIDES[op.side as usize], "depth": op.depth, "stepdown": op.stepdown, "entry": entry,
        "rampLength": op.ramp_length, "tabRamped": op.tab_ramped, "climb": op.climb, "visible": op.visible, "tabs": op.tabs,
        "tabCount": op.tab_count, "tabWidth": op.tab_width, "tabHeight": op.tab_height, "stepover": op.stepover,
        "allowance": op.allowance, "peck": op.peck, "flatDepth": op.flat_depth, "clearTool": op.clear_tool,
        "startDepth": op.start_depth, "glueGap": op.glue_gap, "inlayMargin": op.inlay_margin, "rampIn": op.ramp_in,
        "rampOut": op.ramp_out, "relief": op.relief, "rasterAngle": op.raster_angle, "rough": op.rough,
        "roughStepdown": op.rough_stepdown, "roughStepover": op.rough_stepover});
    let m = o.as_object_mut().unwrap();
    if op.chamfer_width > 0.0 {
        m.insert("chamferWidth".into(), json!(op.chamfer_width));
    }
    if op.raster {
        m.insert("raster".into(), json!(true));
    }
    if op.start_first {
        m.insert("startFirst".into(), json!(true));
    }
    if !op.tab_points.is_empty() {
        m.insert("tabPoints".into(), json!(op.tab_points));
    }
    if op.thread_pitch > 0.0 {
        m.insert("threadPitch".into(), json!(op.thread_pitch));
    }
    if !op.photo.is_empty() {
        m.insert("photo".into(), json!({"w": op.photo_w, "h": op.photo_h, "gray": op.photo, "invert": op.photo_invert}));
    }
    if op.vary > 0.0 {
        m.insert("vary".into(), json!(op.vary));
    }
    if !op.op_group.is_empty() {
        m.insert("opGroup".into(), Value::from(op.op_group.as_str()));
    }
    if !op.gcode.is_empty() {
        m.insert("gcode".into(), Value::from(op.gcode.as_str()));
    }
    if !op.bind.is_empty() {
        m.insert("bind".into(), bind_json(&op.bind));
    }
    if let Some(t) = usize::try_from(op.tool).ok().and_then(|i| tools.get(i)) {
        m.insert("toolName".into(), Value::from(t.name.as_str())); // for matching across projects
    }
    o
}
pub(crate) fn op_from(o: &Value, tool_count: i32) -> Op {
    let kind = get(o, "type").as_str().and_then(|t| TYPES.iter().position(|&n| n == t)).map_or(OpType::Profile, |i| OP_TYPES[i]);
    let side = match get(o, "side").as_str() {
        Some("inside") => Side::Inside,
        Some("on") => Side::On,
        _ => Side::Outside,
    };
    let entry = match get(o, "entry").as_str() {
        Some("plunge") => Entry::Plunge,
        Some("ramp") => Entry::Ramp,
        _ => Entry::Spiral,
    };
    Op {
        kind,
        name: s(o, "name", "Toolpath"),
        shape_ids: arr(get(o, "shapes")).iter().map(|v| to_int(v, 0)).collect(),
        layer_scope: int(o, "layerScope", -1),
        group_scope: int(o, "groupScope", 0),
        tool: clamp(int(o, "tool", 0), 0, (tool_count - 1).max(0)),
        side,
        depth: num(o, "depth", 0.75),
        stepdown: num(o, "stepdown", 0.0),
        entry,
        ramp_length: num(o, "rampLength", 1.0),
        tab_ramped: flag(o, "tabRamped", false),
        climb: flag(o, "climb", true),
        visible: flag(o, "visible", true),
        tabs: flag(o, "tabs", false),
        tab_count: int(o, "tabCount", 4),
        tab_width: num(o, "tabWidth", 0.5),
        tab_height: num(o, "tabHeight", 0.125),
        stepover: num(o, "stepover", 0.4),
        allowance: num(o, "allowance", 0.0),
        peck: num(o, "peck", 0.0),
        flat_depth: num(o, "flatDepth", 0.0),
        clear_tool: clamp(int(o, "clearTool", -1), -1, tool_count - 1),
        start_depth: num(o, "startDepth", 0.0),
        glue_gap: num(o, "glueGap", 0.1),
        inlay_margin: num(o, "inlayMargin", 0.5),
        ramp_in: num(o, "rampIn", 0.0),
        ramp_out: num(o, "rampOut", 0.0),
        chamfer_width: num(o, "chamferWidth", 0.0),
        gcode: s(o, "gcode", ""),
        op_group: s(o, "opGroup", ""),
        vary: num(o, "vary", 0.0),
        photo: s(get(o, "photo"), "gray", ""),
        photo_w: int(get(o, "photo"), "w", 0).max(0) as u32,
        photo_h: int(get(o, "photo"), "h", 0).max(0) as u32,
        photo_invert: flag(get(o, "photo"), "invert", false),
        raster: flag(o, "raster", false),
        thread_pitch: num(o, "threadPitch", 0.0).max(0.0),
        start_first: flag(o, "startFirst", false),
        tab_points: arr(get(o, "tabPoints")).iter().filter_map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?])).collect(),
        relief: int(o, "relief", -1),
        raster_angle: int(o, "rasterAngle", 0),
        rough: flag(o, "rough", false),
        rough_stepdown: num(o, "roughStepdown", 0.2),
        rough_stepover: num(o, "roughStepover", 0.5),
        bind: bind_from(get(o, "bind")),
    }
}

fn material_json(m: &Material) -> Value {
    let mut o = json!({"type": m.kind, "w": m.w, "h": m.h, "t": m.t, "safeZ": m.safe_z, "retractZ": m.retract_z, "origin": m.origin,
                       "zBottom": m.z_bottom, "post": m.post, "autoFeeds": m.auto_feeds});
    if !m.bind.is_empty() {
        o.as_object_mut().unwrap().insert("bind".into(), bind_json(&m.bind));
    }
    if let Some(ml) = &m.machine {
        o.as_object_mut().unwrap().insert("machine".into(), serde_json::to_value(ml).unwrap_or(Value::Null));
    }
    o
}
fn material_from(o: &Value) -> Material {
    Material {
        kind: s(o, "type", "Plywood"),
        w: num(o, "w", 48.0),
        h: num(o, "h", 24.0),
        t: num(o, "t", 0.75),
        safe_z: num(o, "safeZ", 0.8),
        retract_z: num(o, "retractZ", 0.2),
        origin: clamp(int(o, "origin", 0), 0, 4),
        z_bottom: flag(o, "zBottom", false),
        post: s(o, "post", "wincnc-single"),
        auto_feeds: flag(o, "autoFeeds", true),
        machine: get(o, "machine").as_object().and_then(|_| serde_json::from_value(get(o, "machine").clone()).ok()),
        bind: bind_from(get(o, "bind")),
    }
}

fn is_gcarve(root: &Value) -> bool {
    matches!(get(root, "app").as_str(), Some("GCarve" | "ToolpathLab"))
}

/// Indented project JSON (what the C++ app writes for File > Save).
pub fn save_project(d: &Document) -> String {
    let units = if d.bare_unit_in > 10.0 {
        "ft"
    } else if d.bare_unit_in < 0.5 {
        "mm"
    } else {
        "in"
    };
    let vars: Vec<Value> = d.variables.iter().map(|v| json!({"name": v.name, "expr": v.expr})).collect();
    let dims: Vec<Value> = d
        .dimensions
        .iter()
        .map(|m| {
            json!({"a": pt_json(m.a), "b": pt_json(m.b), "offset": m.offset, "sa": m.shape_a, "na": m.node_a, "sb": m.shape_b,
                   "nb": m.node_b, "label": m.label, "drive": m.drive})
        })
        .collect();
    let sheets: Vec<Value> = d
        .sheets
        .iter()
        .enumerate()
        .map(|(i, m)| material_json(if i as i32 == d.active_sheet { &d.material } else { m }))
        .collect();
    let reliefs: Vec<Value> = d
        .reliefs
        .iter()
        .map(|r| {
            json!({"id": r.id, "name": r.name, "source": r.source, "x": r.x, "y": r.y, "w": r.w, "h": r.h, "depth": r.depth,
                   "mirror": r.mirror})
        })
        .collect();
    let layers: Vec<Value> =
        d.layers.iter().map(|l| {
            let mut o = json!({"name": l.name, "color": l.color, "visible": l.visible, "locked": l.locked});
            if l.keep_out {
                o.as_object_mut().unwrap().insert("keepOut".into(), json!(true)); // only when set (Rust only)
            }
            o
        }).collect();
    let mut root = json!({
        "app": "GCarve", "version": VERSION, "units": units,
        "variables": vars,
        "constraints": d.constraints.iter().map(constraint_json).collect::<Vec<_>>(),
        "dimensions": dims,
        "texts": d.texts.iter().map(text_json).collect::<Vec<_>>(),
        "nextText": d.next_text_id,
        "material": material_json(&d.material),
        "sheets": sheets,
        "reliefs": reliefs,
        "nextReliefId": d.next_relief_id,
        "activeSheet": d.active_sheet,
        "layers": layers,
        "activeLayer": d.active_layer,
        "tools": d.tools.iter().map(tool_to_json).collect::<Vec<_>>(),
        "shapes": d.shapes.iter().map(shape_json).collect::<Vec<_>>(),
        "ops": d.ops.iter().map(|o| op_json(o, &d.tools)).collect::<Vec<_>>(),
    });
    if !d.guides.is_empty() {
        // only when there are any: files without guides stay byte-identical to the Qt app's
        root["guides"] = d.guides.iter().map(|&(v, at)| json!({"v": v, "at": at})).collect();
    }
    qt_json(&root, true)
}

/// Parses a project. Reliefs come back with their placement and `source` path but no height field: the caller
/// re-rasterizes them from the STL (the C++ loader does that inline; this crate does no file I/O).
pub fn load_project(text: &str) -> Result<Document, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("Not a GCarve project: {e}"))?;
    if !root.is_object() {
        return Err("Not a GCarve project: not a JSON object".into());
    }
    if !is_gcarve(&root) {
        return Err("Not a GCarve project".into());
    }
    if int(&root, "version", 0) as i64 > VERSION {
        return Err("Project was saved by a newer GCarve".into());
    }

    let mut out = Document::default();
    out.bare_unit_in = match get(&root, "units").as_str().unwrap_or("in") {
        "ft" => 12.0,
        "mm" => 1.0 / 25.4,
        _ => 1.0,
    };
    for o in arr(get(&root, "variables")) {
        if !s(o, "name", "").is_empty() {
            out.variables.push(Variable { name: s(o, "name", ""), expr: s(o, "expr", "0") });
        }
    }
    out.constraints = arr(get(&root, "constraints")).iter().map(constraint_from).collect();
    out.texts = arr(get(&root, "texts")).iter().map(text_from).collect();
    out.next_text_id = out.texts.iter().map(|t| t.id + 1).fold(int(&root, "nextText", 1), i32::max);
    out.dimensions = arr(get(&root, "dimensions"))
        .iter()
        .map(|o| Dimension {
            a: to_pt(get(o, "a")),
            b: to_pt(get(o, "b")),
            offset: num(o, "offset", 0.5),
            shape_a: int(o, "sa", -1),
            node_a: int(o, "na", -1),
            shape_b: int(o, "sb", -1),
            node_b: int(o, "nb", -1),
            label: s(o, "label", ""),
            drive: s(o, "drive", ""),
        })
        .collect();
    out.material = material_from(get(&root, "material"));
    out.reliefs = arr(get(&root, "reliefs"))
        .iter()
        .map(|o| Relief {
            id: int(o, "id", 0),
            name: s(o, "name", ""),
            source: s(o, "source", ""),
            x: num(o, "x", 0.0),
            y: num(o, "y", 0.0),
            w: num(o, "w", 4.0),
            h: num(o, "h", 4.0),
            depth: num(o, "depth", 0.25),
            mirror: flag(o, "mirror", false),
            ..Default::default()
        })
        .collect();
    out.guides = arr(get(&root, "guides")).iter().map(|o| (flag(o, "v", false), num(o, "at", 0.0))).collect();
    out.next_relief_id = int(&root, "nextReliefId", 1).max(out.reliefs.len() as i32 + 1);
    out.sheets = arr(get(&root, "sheets")).iter().map(material_from).collect();
    if out.sheets.is_empty() {
        out.sheets.push(out.material.clone());
    }
    out.active_sheet = clamp(int(&root, "activeSheet", 0), 0, out.sheets.len() as i32 - 1);
    out.material = out.sheets[out.active_sheet as usize].clone();

    out.layers = arr(get(&root, "layers"))
        .iter()
        .map(|o| Layer {
            name: s(o, "name", "Layer"),
            color: s(o, "color", "#d7dae0"),
            visible: flag(o, "visible", true),
            locked: flag(o, "locked", false),
            keep_out: flag(o, "keepOut", false),
        })
        .collect();
    if out.layers.is_empty() {
        out.layers.push(Layer::default());
    }
    out.active_layer = clamp(int(&root, "activeLayer", 0), 0, out.layers.len() as i32 - 1);

    out.tools = arr(get(&root, "tools")).iter().map(tool_from_json).collect();
    if out.tools.is_empty() {
        out.tools = default_tools();
    }

    let mut max_id = 0;
    for v in arr(get(&root, "shapes")) {
        let mut sh = shape_from(v);
        sh.layer = clamp(sh.layer, 0, out.layers.len() as i32 - 1);
        if sh.c.nodes.is_empty() || sh.id <= 0 {
            continue;
        }
        max_id = max_id.max(sh.id);
        out.shapes.push(sh);
    }
    out.set_next_id(max_id + 1);

    let n = out.tools.len() as i32;
    out.ops = arr(get(&root, "ops")).iter().map(|o| op_from(o, n)).collect();
    Ok(out)
}

fn constraint_from(o: &Value) -> Constraint {
    Constraint { anchor: int(o, "anchor", 0), target: int(o, "target", 0), kind: s(o, "type", "right-of"), expr: s(o, "expr", "") }
}

/// Clipboard: the given shapes plus any toolpaths that use only them, plus the tools those toolpaths need (compact JSON).
pub fn save_clip(d: &Document, ids: &BTreeSet<i32>) -> String {
    let shapes: Vec<Value> = d.shapes.iter().filter(|sh| ids.contains(&sh.id)).map(shape_json).collect();
    let mut ops = Vec::new();
    let mut used_tools: Vec<usize> = Vec::new();
    for op in &d.ops {
        if op.shape_ids.is_empty() || !op.shape_ids.iter().all(|id| ids.contains(id)) {
            continue;
        }
        ops.push(op_json(op, &d.tools));
        if let Ok(t) = usize::try_from(op.tool) {
            if t < d.tools.len() && !used_tools.contains(&t) {
                used_tools.push(t);
            }
        }
    }
    let tools: Vec<Value> = used_tools.iter().map(|&t| tool_to_json(&d.tools[t])).collect();
    let cons: Vec<Value> =
        d.constraints.iter().filter(|c| ids.contains(&c.anchor) && ids.contains(&c.target)).map(constraint_json).collect();
    let text_ids: BTreeSet<i32> = d.shapes.iter().filter(|sh| ids.contains(&sh.id) && sh.text != 0).map(|sh| sh.text).collect();
    let texts: Vec<Value> = d.texts.iter().filter(|t| text_ids.contains(&t.id)).map(text_json).collect();
    let root = json!({"app": "GCarve", "clip": 1, "shapes": shapes, "ops": ops, "tools": tools, "constraints": cons, "texts": texts});
    qt_json(&root, false)
}

/// Pastes into `d` (shapes get new ids on the active layer, tools are matched by name or appended). Returns the new ids.
pub fn load_clip(text: &str, d: &mut Document, offset: Pt) -> BTreeSet<i32> {
    let mut new_ids = BTreeSet::new();
    let Ok(root) = serde_json::from_str::<Value>(text) else { return new_ids };
    if !root.is_object() || !is_gcarve(&root) || root.get("clip").is_none() {
        return new_ids;
    }
    let (mut id_map, mut group_map, mut text_map) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let mut next_group = d.new_group_id();
    for v in arr(get(&root, "texts")) {
        let mut t = text_from(v);
        let old = t.id;
        t.id = d.next_text_id;
        d.next_text_id += 1;
        text_map.insert(old, t.id);
        d.texts.push(t);
    }
    for v in arr(get(&root, "shapes")) {
        let sh = shape_from(v);
        if sh.c.nodes.is_empty() {
            continue;
        }
        let new_id = d.add(transformed(&sh.c, &Affine::translate(offset.x, offset.y)), None); // active layer
        let group = (sh.group != 0).then(|| {
            *group_map.entry(sh.group).or_insert_with(|| {
                next_group += 1;
                next_group - 1
            })
        });
        let text = text_map.get(&sh.text).copied().filter(|_| sh.text != 0);
        let ns = d.find_mut(new_id).unwrap();
        if let Some(g) = group {
            ns.group = g;
        }
        if let Some(t) = text {
            ns.text = t;
        }
        id_map.insert(sh.id, new_id);
        new_ids.insert(new_id);
    }
    for new_text in text_map.values() {
        if let Some(t) = d.find_text(*new_text) {
            t.path_shape = id_map.get(&t.path_shape).copied().unwrap_or(0);
        }
    }
    // tools: match by name, else append
    let clip_tools: Vec<Tool> = arr(get(&root, "tools")).iter().map(tool_from_json).collect();
    for o in arr(get(&root, "ops")) {
        let mut op = op_from(o, d.tools.len() as i32);
        op.tool = tool_index_for(&mut d.tools, &s(o, "toolName", ""), &clip_tools);
        let ids: Vec<i32> = op.shape_ids.iter().filter_map(|id| id_map.get(id).copied()).collect();
        if ids.is_empty() {
            continue;
        }
        op.shape_ids = ids;
        d.ops.push(op);
    }
    for o in arr(get(&root, "constraints")) {
        if let (Some(&a), Some(&t)) = (id_map.get(&int(o, "anchor", 0)), id_map.get(&int(o, "target", 0))) {
            d.constraints.push(Constraint { anchor: a, target: t, kind: s(o, "type", "right-of"), expr: s(o, "expr", "") });
        }
    }
    new_ids
}

/// The user's bit library file (`{"app":"GCarve","tools":[...]}`); defaults when empty or unreadable.
pub fn tool_library_from_json(text: &str) -> Vec<Tool> {
    let root: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let tools: Vec<Tool> = arr(get(&root, "tools")).iter().map(tool_from_json).collect();
    if tools.is_empty() {
        default_tools()
    } else {
        tools
    }
}
pub fn tool_library_to_json(tools: &[Tool]) -> String {
    qt_json(&json!({"app": "GCarve", "tools": tools.iter().map(tool_to_json).collect::<Vec<_>>()}), true)
}

// ---- Qt-compatible JSON writer ------------------------------------------------------------------------------------

/// QJsonDocument::toJson: Indented = 4 spaces, "key": value, trailing newline; Compact = no whitespace.
pub(crate) fn qt_json(v: &Value, indented: bool) -> String {
    let mut out = String::new();
    write_value(v, &mut out, 0, indented);
    if indented {
        out.push('\n');
    }
    out
}

fn write_value(v: &Value, out: &mut String, indent: usize, ind: bool) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => match n.as_i64() {
            Some(i) => out.push_str(&i.to_string()),
            None => out.push_str(&qt_double(n.as_f64().unwrap_or(0.0))),
        },
        Value::String(s) => write_str(s, out),
        Value::Array(a) => {
            out.push_str(if ind { "[\n" } else { "[" });
            for (i, x) in a.iter().enumerate() {
                if ind {
                    out.push_str(&" ".repeat(4 * (indent + 1)));
                }
                write_value(x, out, indent + 1, ind);
                if i + 1 < a.len() {
                    out.push(',');
                }
                if ind {
                    out.push('\n');
                }
            }
            if ind {
                out.push_str(&" ".repeat(4 * indent));
            }
            out.push(']');
        }
        Value::Object(o) => {
            out.push_str(if ind { "{\n" } else { "{" });
            let mut keys: Vec<(&String, &Value)> = o.iter().collect();
            keys.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16())); // QJsonObject keeps keys sorted (UTF-16)
            for (i, (k, x)) in keys.into_iter().enumerate() {
                if ind {
                    out.push_str(&" ".repeat(4 * (indent + 1)));
                }
                write_str(k, out);
                out.push_str(if ind { ": " } else { ":" });
                write_value(x, out, indent + 1, ind);
                if i + 1 < o.len() {
                    out.push(',');
                }
                if ind {
                    out.push('\n');
                }
            }
            if ind {
                out.push_str(&" ".repeat(4 * indent));
            }
            out.push('}');
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// QByteArray::number(d, 'g', QLocale::FloatingPointShortest): shortest round-trip digits; plain decimal unless the
/// exponent form ("1e-05", "1.5e+20") is shorter.
fn qt_double(d: f64) -> String {
    if !d.is_finite() {
        return "null".into();
    }
    if d == 0.0 {
        return "0".into();
    }
    let e = format!("{:e}", d.abs()); // shortest digits: "1.2345e2"
    let (mant, exp) = e.split_once('e').unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let n = digits.len() as i32;
    let decpt = exp.parse::<i32>().unwrap() + 1; // value = 0.digits * 10^decpt
    let bias = 4; // "e+" plus two exponent digits
    let decimal = if decpt <= 0 { 1 - decpt <= bias } else { decpt <= n || decpt <= n + bias };
    let sign = if d < 0.0 { "-" } else { "" };
    if decimal {
        let body = if decpt <= 0 {
            format!("0.{}{digits}", "0".repeat((-decpt) as usize))
        } else if decpt >= n {
            format!("{digits}{}", "0".repeat((decpt - n) as usize))
        } else {
            format!("{}.{}", &digits[..decpt as usize], &digits[decpt as usize..])
        };
        format!("{sign}{body}")
    } else {
        let x = decpt - 1;
        let m = if n > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits };
        format!("{sign}{m}e{}{:02}", if x < 0 { '-' } else { '+' }, x.abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qt_number_format() {
        for (v, s) in [(8.0, "8"), (0.25, "0.25"), (-0.5, "-0.5"), (0.0005, "0.0005"), (0.00005, "5e-05"), (1e20, "1e+20"), (123456.0, "123456")] {
            assert_eq!(qt_double(v), s, "{v}");
        }
    }
}

#[cfg(test)]
mod guide_tests {
    use super::*;

    #[test]
    fn guides_are_saved_only_when_there_are_any() {
        let mut d = Document::default();
        assert!(!save_project(&d).contains("guides"));
        d.guides = vec![(true, 1.5), (false, 3.25)];
        let back = load_project(&save_project(&d)).unwrap();
        assert_eq!(back.guides, d.guides);
    }
}
