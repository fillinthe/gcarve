//! AI assistants use GCarve through MCP (Model Context Protocol, an open standard: ChatGPT, Gemini CLI,
//! Cursor, VS Code, LM Studio … all speak it). Help › AI Assistants turns it on (off by default, store `mcp-on`):
//! the desktop app then answers MCP's Streamable HTTP transport on http://127.0.0.1:47110/mcp (plain JSON answers,
//! no event stream), and `gcarve --mcp` bridges stdio clients to it. The assistant works on the drawing that is open
//! in the window, through the same calls the menus use; every change is one undo step. Desktop only (no sockets in
//! the browser).

use crate::canvas::Canvas;
use gcarve_core::document::{OpType, Side, TextItem, ToolType};
use gcarve_core::geom::{bounds, make_ellipse, make_polyline, make_rect, pt, transformed, Affine};
use gcarve_core::pipeline;
use gcarve_core::postdef::PostDef;
use serde_json::{json, Value};

#[cfg(not(target_arch = "wasm32"))]
pub const PORT: u16 = 47110;
pub const URL: &str = "http://127.0.0.1:47110/mcp";
const PROTOCOL: &str = "2025-06-18";

/// What the tools need from the app besides the drawing
pub trait McpApp {
    fn canvas(&mut self) -> &mut Canvas;
    /// the board's post, user posts included
    fn post(&self) -> PostDef;
    /// project name for file names
    fn project(&self) -> String;
    /// (id, description) of every menu command
    fn commands(&self) -> Vec<(String, String)>;
    /// runs a menu command the way the menu does
    fn run(&mut self, id: &str);
    /// opens (or with `import`, adds to the drawing) a file from this computer, like File › Open; says what came in
    fn open(&mut self, path: &str, import: bool) -> Result<String, String>;
    /// saves the project (.tlp) to `path`, else where it was last saved
    fn save(&mut self, path: Option<&str>) -> Result<String, String>;
}

/// One JSON-RPC message in, its answer out (None for notifications).
pub fn handle(app: &mut dyn McpApp, req: &Value) -> Option<Value> {
    let id = req.get("id")?.clone();
    let method = req["method"].as_str().unwrap_or("");
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": req["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "gcarve", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "GCarve is CNC CAM software. Units are inches, Y up, (0,0) = the board's bottom-left corner. Start with get_drawing. Draw vectors, add toolpaths to them, run check, show a preview, and export G-code only when the user asks. Every change is one undo step in the user's window (undo tool or ⌘Z). Never call a cut safe yourself: say what check reports and tell the user to run an air cut first."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("");
            let args = &req["params"]["arguments"];
            Ok(match call(app, name, args) {
                Ok(content) => json!({"content": content}),
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            })
        }
        _ => Err(json!({"code": -32601, "message": format!("unknown method {method}")})),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    })
}

fn tool(name: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    json!({"name": name, "description": desc, "inputSchema": {"type": "object", "properties": props, "required": required}})
}

fn tools() -> Vec<Value> {
    let num = |d: &str| json!({"type": "number", "description": d});
    let ids = json!({"type": "array", "items": {"type": "integer"}, "description": "shape ids from get_drawing"});
    vec![
        tool("get_drawing", "The open drawing: board (size, thickness, material, machine post), sheets, layers with vector counts, vectors (id, layer, closed, bounding box; the first `limit`, selected ones first, optionally only one layer), texts, bits and toolpaths.",
            json!({"limit": {"type": "integer", "description": "most vectors listed, default 200"}, "layer": {"type": "string", "description": "only this layer's vectors"}}), &[]),
        tool("add_shapes", "Draw vectors on the active sheet. Each item is {type: rect, x, y, w, h} (bottom-left corner), {type: ellipse, cx, cy, w, h}, {type: polyline, points: [[x,y],…], closed} or {type: svg, svg: \"<svg …>\"} (an SVG file's text, placed with its bottom-left at x, y). Returns the new shape ids.",
            json!({"shapes": {"type": "array", "items": {"type": "object"}}, "layer": {"type": "string", "description": "layer name (made when missing)"}}), &["shapes"]),
        tool("add_text", "Text as vector letters, centered at x, y, letters `height` tall.", json!({"text": {"type": "string"}, "x": num("center x"), "y": num("center y"), "height": num("letter height"), "font": {"type": "string", "description": "font family, e.g. Liberation Sans, Single Line Sans"}, "bold": {"type": "boolean"}, "layer": {"type": "string"}}), &["text", "x", "y", "height"]),
        tool("edit_text", "Change a text placed earlier (id from get_drawing's texts): new words, letter height, font or bold. It stays centered where it was, on its layer, and its toolpaths follow the new letters.", json!({"id": {"type": "integer"}, "text": {"type": "string"}, "height": num("letter height"), "font": {"type": "string"}, "bold": {"type": "boolean"}}), &["id"]),
        tool("delete_shapes", "Delete vectors (and the toolpaths that only used them).", json!({"ids": ids}), &["ids"]),
        tool("set_board", "Set the board (the active sheet): width, height, thickness in inches, material name (Plywood, MDF, Oak …).", json!({"width": num("in"), "height": num("in"), "thickness": num("in"), "material": {"type": "string"}}), &[]),
        tool("add_toolpath", "Add a toolpath; settings left out get GCarve's defaults for that kind. kind: profile (cut along / out), pocket (clear the inside), drill (holes at circle centers), vcarve (V-bit carving of letters / artwork), bore (helix a hole bigger than the bit), chamfer (V-bit bevel along edges, width), flute (grooves along open vectors), texture (scooped surface inside shapes), prism (raised V-carved letters). side for profile: outside | inside | on. bit: {type: end_mill | v_bit | ball_nose, diameter} — the project's closest bit is used, added when missing. Applies to `shape_ids`, else to `layer`, else to the selection.",
            json!({"kind": {"type": "string", "enum": ["profile", "pocket", "drill", "vcarve", "bore", "chamfer", "flute", "texture", "prism"]}, "width": num("chamfer: bevel width"), "shape_ids": ids, "layer": {"type": "string"}, "depth": num("cut depth in (for vcarve: the maximum)"), "side": {"type": "string", "enum": ["outside", "inside", "on"]},
                "bit": {"type": "object", "properties": {"type": {"type": "string", "enum": ["end_mill", "v_bit", "ball_nose"]}, "diameter": {"type": "number"}}}, "tabs": {"type": "integer", "description": "tabs per part on a through cut (0 = none)"}, "flat_depth": num("vcarve: flat floor depth (0 = pointed)"), "name": {"type": "string"}}), &["kind"]),
        tool("edit_toolpath", "Change a toolpath (index from get_drawing): any of name, depth, side, tabs, flat_depth, bit {type, diameter}.",
            json!({"index": {"type": "integer"}, "name": {"type": "string"}, "depth": num("in"), "side": {"type": "string", "enum": ["outside", "inside", "on"]}, "tabs": {"type": "integer"}, "flat_depth": num("in"), "bit": {"type": "object"}}), &["index"]),
        tool("transform_shapes", "Move, scale or turn vectors (one undo step): dx, dy in inches, scale (1 = same size), rotate in degrees counter-clockwise, about the center of the shapes together.",
            json!({"ids": ids, "dx": num("in"), "dy": num("in"), "scale": num("factor"), "rotate": num("degrees")}), &["ids"]),
        tool("open_file", "Open a file from this computer by its full path (GCarve .tlp, VCarve .crv, Carbide Create .c2d, SVG, DXF, DWG, PDF, AI, EPS, LightBurn) in a new tab, or with import = true add its vectors to the open drawing.",
            json!({"path": {"type": "string"}, "import": {"type": "boolean"}}), &["path"]),
        tool("save_project", "Save the drawing as a GCarve project (.tlp): to `path` (full path), else where it was last saved.", json!({"path": {"type": "string"}}), &[]),
        tool("delete_toolpath", "Delete a toolpath by its index in get_drawing.", json!({"index": {"type": "integer"}}), &["index"]),
        tool("check", "Work out every toolpath on the active sheet and report: cutting time, warnings (bit too big, too deep …), cutting-order problems, machine limits and the G-code safety findings.", json!({}), &[]),
        tool("show_drawing", "A picture of the active sheet's vectors on the board (selected ones in blue), to see what was drawn before there are toolpaths.", json!({}), &[]),
        tool("preview", "A picture of the board as the toolpaths would carve it (simulated).", json!({}), &[]),
        tool("export_gcode", "The G-code files for the project with the board's post (text returned, nothing written to disk) and the safety findings.", json!({"max_chars": {"type": "integer", "description": "per file, default 20000"}}), &[]),
        tool("undo", "Undo the last change.", json!({}), &[]),
        tool("list_commands", "Every menu command (id and what it does), for run_command.", json!({"filter": {"type": "string"}}), &[]),
        tool("set_machine", "Choose the CNC machine for the job from GCarve's database by name words (e.g. \"Shapeoko 4 XXL\", \"Onefinity Woodworker\"): its work area, feed limits, spindle and G-code post go on every sheet. Several matches = their names come back to pick from.", json!({"name": {"type": "string"}}), &["name"]),
        tool("set_bit_feeds", "Set a bit's feeds and speeds for the board's material (bit index from get_drawing, or `bit_index` in check's answer): feed_xy and feed_z in inches / min, rpm, stepdown in inches. Leave a value out to keep what it cuts at now; `suggested: true` takes check's suggestion.", json!({"bit": {"type": "integer"}, "feed_xy": num("in / min"), "feed_z": num("in / min"), "rpm": {"type": "integer"}, "stepdown": num("in"), "suggested": {"type": "boolean"}}), &["bit"]),
        tool("sheet", "Sheets (boards) of the job: `go_to` a sheet number (0 = first; drawing, board and toolpath tools then work there), `add` a new sheet with this board (it becomes the active one), and/or `name` the active sheet.", json!({"go_to": {"type": "integer"}, "add": {"type": "boolean"}, "name": {"type": "string"}}), &[]),
        tool("select_shapes", "Select vectors, so run_command (weld, offset, align, nest …) and toolpaths without shape_ids work on them: `ids`, or every vector on `layer`, or `all`; nothing given = clear the selection.", json!({"ids": ids, "layer": {"type": "string"}, "all": {"type": "boolean"}}), &[]),
        tool("layer", "A layer by name (made when missing): move vectors onto it (`ids`), show or hide it (`visible`), or mark it `keep_out` (clamps, screws: check flags any cut near its closed shapes).", json!({"name": {"type": "string"}, "ids": ids, "visible": {"type": "boolean"}, "keep_out": {"type": "boolean"}}), &["name"]),
        tool("run_command", "Run a menu command by id, as if the user picked it (some open a dialog for the user). Vector commands work on the selection (select_shapes).", json!({"id": {"type": "string"}}), &["id"]),
    ]
}

fn text(s: impl Into<String>) -> Vec<Value> {
    vec![json!({"type": "text", "text": s.into()})]
}

fn f(a: &Value, k: &str) -> Option<f64> {
    a.get(k).and_then(Value::as_f64)
}

fn need(a: &Value, k: &str) -> Result<f64, String> {
    f(a, k).ok_or(format!("{k} (a number) is missing"))
}

fn call(app: &mut dyn McpApp, name: &str, a: &Value) -> Result<Vec<Value>, String> {
    match name {
        "list_commands" => {
            let q = a["filter"].as_str().unwrap_or("").to_lowercase();
            let lines: Vec<String> = app.commands().into_iter().filter(|(id, d)| q.is_empty() || id.contains(&q) || d.to_lowercase().contains(&q)).map(|(id, d)| format!("{id}\t{d}")).collect();
            return Ok(text(lines.join("\n")));
        }
        "run_command" => {
            let id = a["id"].as_str().ok_or("id is missing")?;
            if !app.commands().iter().any(|(c, _)| c == id) {
                return Err(format!("no command {id}: see list_commands"));
            }
            app.run(id);
            return Ok(text(format!("ran {id}")));
        }
        "open_file" => return app.open(a["path"].as_str().ok_or("path is missing")?, a["import"].as_bool().unwrap_or(false)).map(text),
        "save_project" => return app.save(a["path"].as_str()).map(text),
        "export_gcode" => {
            let post = app.post();
            let project = app.project();
            let doc = &app.canvas().doc;
            let gen = pipeline::regenerate(doc);
            let r = pipeline::export(doc, &gen, &project, "", 0.0, &post);
            let max = a["max_chars"].as_u64().unwrap_or(20_000) as usize;
            let mut out = format!("post: {}\nsafety: {}\n", post.name, if r.safety.is_empty() { "nothing found".into() } else { r.safety.join(" | ") });
            for file in &r.files {
                let cut: String = file.text.chars().take(max).collect();
                out += &format!("\n--- {} ({} lines){}\n{cut}", file.filename, file.text.lines().count(), if cut.len() < file.text.len() { ", shortened" } else { "" });
            }
            return Ok(text(out));
        }
        _ => {}
    }
    let c = app.canvas();
    match name {
        "get_drawing" => Ok(text(serde_json::to_string_pretty(&summary(c, a["limit"].as_u64().unwrap_or(200) as usize, a["layer"].as_str())).unwrap_or_default())),
        "add_shapes" => {
            let items = a["shapes"].as_array().ok_or("shapes (an array) is missing")?;
            let layer = a["layer"].as_str().map(|n| c.doc.layer_named(n));
            let mut contours = Vec::new();
            for s in items {
                match s["type"].as_str().unwrap_or("") {
                    "rect" => contours.push(make_rect(need(s, "x")?, need(s, "y")?, need(s, "x")? + need(s, "w")?, need(s, "y")? + need(s, "h")?)),
                    "ellipse" | "circle" => {
                        let w = f(s, "w").or(f(s, "diameter")).ok_or("ellipse: w (or diameter) is missing")?;
                        contours.push(make_ellipse(need(s, "cx")?, need(s, "cy")?, w / 2.0, f(s, "h").unwrap_or(w) / 2.0));
                    }
                    "polyline" => {
                        let p: Vec<_> = s["points"].as_array().ok_or("polyline: points is missing")?.iter().filter_map(|q| Some(pt(q[0].as_f64()?, q[1].as_f64()?))).collect();
                        if p.len() < 2 {
                            return Err("polyline: needs 2 points or more".into());
                        }
                        contours.push(make_polyline(&p, s["closed"].as_bool().unwrap_or(false) && p.len() >= 3));
                    }
                    "svg" => {
                        let r = gcarve_core::svg::import_svg(s["svg"].as_str().ok_or("svg: the svg text is missing")?);
                        let mut b = gcarve_core::geom::Rect::default();
                        r.contours.iter().for_each(|k| b.add_rect(&bounds(k)));
                        let m = Affine::translate(f(s, "x").unwrap_or(0.0) - b.x0, f(s, "y").unwrap_or(0.0) - b.y0);
                        contours.extend(r.contours.iter().map(|k| transformed(k, &m)));
                    }
                    t => return Err(format!("unknown shape type {t:?}: rect, ellipse, polyline or svg")),
                }
            }
            c.doc.checkpoint();
            let ids: Vec<i32> = contours.into_iter().map(|k| c.doc.add(k, layer)).collect();
            c.sel = ids.iter().copied().collect();
            c.sel_changed();
            c.mutated();
            Ok(text(format!("added shape ids {ids:?}")))
        }
        "add_text" => {
            let t = TextItem { text: a["text"].as_str().ok_or("text is missing")?.into(), family: a["font"].as_str().unwrap_or("Liberation Sans").into(), height: need(a, "height")?, bold: a["bold"].as_bool().unwrap_or(false), align: 1, ..Default::default() };
            let layer = a["layer"].as_str().map(|n| c.doc.layer_named(n));
            c.doc.checkpoint();
            c.place_text(t, pt(need(a, "x")?, need(a, "y")?), layer);
            let tid = c.doc.texts.last().map_or(0, |t| t.id);
            let ids: Vec<i32> = c.doc.shapes.iter().filter(|g| g.text == tid).map(|g| g.id).collect();
            c.mutated();
            Ok(text(format!("added text: letter shape ids {ids:?}")))
        }
        "edit_text" => {
            let id = a["id"].as_i64().ok_or("id is missing")? as i32;
            let mut t = c.doc.texts.iter().find(|t| t.id == id).cloned().ok_or(format!("no text {id}"))?;
            let (at, layer) = c.text_centre(id).ok_or(format!("text {id} has no letters"))?;
            if let Some(v) = a["text"].as_str() { t.text = v.into() }
            if let Some(v) = a["height"].as_f64() { t.height = v }
            if let Some(v) = a["font"].as_str() { t.family = v.into() }
            if let Some(v) = a["bold"].as_bool() { t.bold = v }
            if c.place_text(t, at, Some(layer)) == 0 {
                return Err("nothing to place: empty text".into());
            }
            c.mutated();
            Ok(text(format!("text {id} changed")))
        }
        "delete_shapes" => {
            let ids: Vec<i32> = a["ids"].as_array().ok_or("ids is missing")?.iter().filter_map(|v| v.as_i64().map(|v| v as i32)).collect();
            c.sel = ids.iter().copied().collect();
            c.delete_selected();
            Ok(text(format!("deleted {} shapes", ids.len())))
        }
        "set_board" => {
            c.doc.checkpoint();
            let m = &mut c.doc.material;
            if let Some(v) = f(a, "width").filter(|v| *v > 0.0) {
                m.w = v;
            }
            if let Some(v) = f(a, "height").filter(|v| *v > 0.0) {
                m.h = v;
            }
            if let Some(v) = f(a, "thickness").filter(|v| *v > 0.0) {
                m.t = v;
            }
            if let Some(k) = a["material"].as_str() {
                m.kind = k.into();
            }
            let m = m.clone();
            let j = c.doc.active_sheet.max(0) as usize;
            if let Some(s) = c.doc.sheets.get_mut(j) {
                *s = m.clone();
            }
            c.mutated();
            Ok(text(format!("board {} × {} × {} in {}", m.w, m.h, m.t, m.kind)))
        }
        "add_toolpath" => {
            let k = a["kind"].as_str().unwrap_or("");
            let kind = match k {
                "profile" => OpType::Profile,
                "pocket" => OpType::Pocket,
                "drill" => OpType::Drill,
                "vcarve" => OpType::VCarve,
                "bore" => OpType::Bore,
                "flute" => OpType::Flute,
                "chamfer" => OpType::Chamfer,
                "prism" => OpType::Prism,
                "texture" => OpType::Texture,
                _ => return Err(format!("unknown kind {k:?}: profile, pocket, drill, vcarve, bore, chamfer, flute, texture or prism")),
            };
            let shape_ids: Vec<i32> = match a["shape_ids"].as_array() {
                Some(v) => v.iter().filter_map(|v| v.as_i64().map(|v| v as i32)).collect(),
                None if a["layer"].is_string() => vec![],
                None => c.sel.iter().copied().collect(),
            };
            let layer_scope = a["layer"].as_str().filter(|_| shape_ids.is_empty()).map_or(-1, |n| c.doc.layer_named(n));
            if shape_ids.is_empty() && layer_scope < 0 {
                return Err("nothing to cut: give shape_ids, a layer, or select vectors first".into());
            }
            if let Some(bad) = shape_ids.iter().find(|id| c.doc.find(**id).is_none()) {
                return Err(format!("no shape {bad}"));
            }
            c.doc.checkpoint();
            // the toolpath dialog's defaults for this kind (bit, depth, stepover …), then what the assistant gave
            let mut op = crate::dialogs::op::new_op(&c.doc, kind, &shape_ids);
            op.layer_scope = layer_scope;
            let want_v = matches!(kind, OpType::VCarve | OpType::Chamfer | OpType::Prism);
            let given = a["bit"]["type"].as_str().map(|t| match t {
                "v_bit" => ToolType::VBit,
                "ball_nose" => ToolType::BallNose,
                _ => ToolType::EndMill,
            });
            let have = c.doc.tools.get(op.tool.max(0) as usize).map(|t| t.kind).filter(|_| op.tool >= 0);
            let bk = given.or(have.filter(|t| (*t == ToolType::VBit) == want_v)).unwrap_or(if want_v { ToolType::VBit } else { ToolType::EndMill });
            if given.is_some() || have != Some(bk) || f(&a["bit"], "diameter").is_some() {
                let dia = f(&a["bit"], "diameter").unwrap_or(if bk == ToolType::VBit { 0.5 } else { 0.25 });
                let label = match bk {
                    ToolType::VBit => format!("90° V-Bit ({dia} inch)"),
                    ToolType::BallNose => format!("Ball Nose ({dia} inch)"),
                    _ => format!("End Mill ({dia} inch)"),
                };
                op.tool = crate::dialogs::sign::bit(c, bk, dia, &label);
            }
            if let Some(d) = f(a, "depth") {
                if d <= 0.0 {
                    return Err("depth must be more than 0".into());
                }
                op.depth = d;
            }
            match a["side"].as_str() {
                Some("inside") => op.side = Side::Inside,
                Some("on") => op.side = Side::On,
                Some("outside") => op.side = Side::Outside,
                _ => {}
            }
            if let Some(t) = a["tabs"].as_i64() {
                (op.tabs, op.tab_count, op.tab_corners) = (t > 0, (t as i32).max(1), true);
            }
            if let Some(v) = f(a, "flat_depth") {
                op.flat_depth = v.max(0.0);
            }
            if let Some(v) = f(a, "width") {
                op.chamfer_width = v.max(0.0);
            }
            op.name = a["name"].as_str().map(String::from).unwrap_or_else(|| format!("{} {}", crate::dialogs::op::KIND_NAMES[kind as usize], c.doc.ops.len() + 1));
            let tool = op.tool;
            c.doc.ops.push(op);
            c.mutated();
            Ok(text(format!("added toolpath {} with bit #{tool}; run check to see what it does", c.doc.ops.len() - 1)))
        }
        "edit_toolpath" => {
            let i = a["index"].as_u64().ok_or("index is missing")? as usize;
            if i >= c.doc.ops.len() {
                return Err(format!("no toolpath {i}"));
            }
            c.doc.checkpoint();
            if let Some(b) = a["bit"].as_object() {
                let kind = match b.get("type").and_then(Value::as_str) {
                    Some("v_bit") => ToolType::VBit,
                    Some("ball_nose") => ToolType::BallNose,
                    _ => ToolType::EndMill,
                };
                let dia = b.get("diameter").and_then(Value::as_f64).unwrap_or(0.25);
                c.doc.ops[i].tool = crate::dialogs::sign::bit(c, kind, dia, &format!("{kind:?} ({dia} inch)"));
            }
            let o = &mut c.doc.ops[i];
            if let Some(n) = a["name"].as_str() {
                o.name = n.into();
            }
            if let Some(d) = f(a, "depth").filter(|d| *d > 0.0) {
                o.depth = d;
            }
            if let Some(d) = f(a, "flat_depth").filter(|d| *d >= 0.0) {
                o.flat_depth = d;
            }
            match a["side"].as_str() {
                Some("inside") => o.side = Side::Inside,
                Some("on") => o.side = Side::On,
                Some("outside") => o.side = Side::Outside,
                _ => {}
            }
            if let Some(t) = a["tabs"].as_i64() {
                (o.tabs, o.tab_count) = (t > 0, (t as i32).max(1));
            }
            c.mutated();
            Ok(text(format!("changed toolpath {i}")))
        }
        "set_machine" => {
            let words: Vec<String> = a["name"].as_str().ok_or("name is missing")?.to_lowercase().split_whitespace().map(str::to_string).collect();
            let all = &crate::catalog::get().machines;
            let hits: Vec<_> = all.iter().filter(|m| { let t = m.title().to_lowercase(); words.iter().all(|w| t.contains(w.as_str())) }).collect();
            let m = match hits.as_slice() {
                [] => return Err(format!("no machine matches; {} in the database, try fewer words", all.len())),
                [m] => *m,
                _ => match hits.iter().find(|m| m.title().to_lowercase() == words.join(" ")) {
                    Some(m) => *m,
                    None => return Err(format!("{} machines match, pick one: {}", hits.len(), hits.iter().take(15).map(|m| m.title()).collect::<Vec<_>>().join("; "))),
                },
            };
            let (limits, post) = (m.limits(m.default_spindle()), m.post().map(str::to_string));
            let doc = &mut c.doc;
            doc.checkpoint();
            doc.sync_sheet();
            for s in doc.sheets.iter_mut().chain(std::iter::once(&mut doc.material)) {
                s.machine = Some(limits.clone());
                if let Some(p) = &post {
                    s.post = p.clone();
                }
            }
            c.apply_bindings();
            c.mutated();
            Ok(text(format!("machine: {} ({}), post {}", limits.name, crate::catalog::limits_line(&limits), post.as_deref().unwrap_or("unchanged: the database names none"))))
        }
        "set_bit_feeds" => {
            let i = a["bit"].as_u64().ok_or("bit is missing")? as usize;
            if i >= c.doc.tools.len() {
                return Err(format!("no bit {i}"));
            }
            // what it cuts at now (preset or automatic suggestion), so values left out stay as they are
            let now = pipeline::tool_for_cut(&c.doc, i);
            let (mut xy, mut z, mut rpm, mut sd) = (now.feed_xy, now.feed_z, now.rpm, now.stepdown);
            if a["suggested"].as_bool() == Some(true) {
                let s = pipeline::suggest(&c.doc, &c.doc.tools[i]);
                (xy, z, rpm, sd) = (s.feed_xy.round(), s.feed_z.round(), s.rpm, s.stepdown);
            }
            xy = f(a, "feed_xy").filter(|v| *v > 0.0).unwrap_or(xy);
            z = f(a, "feed_z").filter(|v| *v > 0.0).unwrap_or(z);
            rpm = a["rpm"].as_i64().filter(|v| *v >= 0).map_or(rpm, |v| v as i32);
            sd = f(a, "stepdown").filter(|v| *v > 0.0).unwrap_or(sd);
            let material = c.doc.material.kind.clone();
            c.doc.checkpoint();
            let t = &mut c.doc.tools[i];
            let p = gcarve_core::document::FeedPreset { material: material.clone(), feed_xy: xy, feed_z: z, rpm, stepdown: sd };
            match t.presets.iter_mut().find(|q| q.material.eq_ignore_ascii_case(&material)) {
                Some(q) => *q = p,
                None => t.presets.push(p),
            }
            let name = t.name.clone();
            c.mutated();
            Ok(text(format!("{name} in {material}: {xy} in/min, plunge {z} in/min, {rpm} rpm, {sd} in a pass")))
        }
        "sheet" => {
            if let Some(j) = a["go_to"].as_u64() {
                if j as usize >= c.doc.sheets.len() {
                    return Err(format!("no sheet {j}: there are {}", c.doc.sheets.len()));
                }
                c.go_to_sheet(j as usize);
            }
            if a["add"].as_bool() == Some(true) {
                c.add_sheet();
            }
            if let Some(n) = a["name"].as_str() {
                c.doc.checkpoint();
                let j = c.doc.active_sheet as usize;
                c.doc.set_sheet_name(j, n);
                c.touch();
            }
            Ok(text(format!("active sheet {} \"{}\" of {}", c.doc.active_sheet, c.doc.sheet_name(c.doc.active_sheet as usize), c.doc.sheets.len())))
        }
        "select_shapes" => {
            let d = &c.doc;
            let layer = a["layer"].as_str();
            c.sel = if let Some(ids) = a["ids"].as_array() {
                ids.iter().filter_map(|v| v.as_i64().map(|v| v as i32)).filter(|id| d.find(*id).is_some()).collect()
            } else if layer.is_some() || a["all"].as_bool() == Some(true) {
                d.shapes.iter().filter(|s| s.sheet == d.active_sheet && layer.is_none_or(|l| d.layers.get(s.layer as usize).is_some_and(|x| x.name == l))).map(|s| s.id).collect()
            } else {
                Default::default()
            };
            c.sel_changed();
            Ok(text(format!("{} vectors selected", c.sel.len())))
        }
        "layer" => {
            let name = a["name"].as_str().ok_or("name is missing")?;
            c.doc.checkpoint();
            let i = c.doc.layer_named(name);
            let mut moved = 0;
            for id in a["ids"].as_array().into_iter().flatten().filter_map(|v| v.as_i64()) {
                if let Some(s) = c.doc.find_mut(id as i32) {
                    s.layer = i;
                    moved += 1;
                }
            }
            let l = &mut c.doc.layers[i as usize];
            if let Some(v) = a["visible"].as_bool() {
                l.visible = v;
            }
            if let Some(k) = a["keep_out"].as_bool() {
                l.keep_out = k;
            }
            let (v, k) = (l.visible, l.keep_out);
            c.mutated();
            Ok(text(format!("layer {name:?}: {moved} vectors moved onto it, {}, keep-out {}", if v { "shown" } else { "hidden" }, if k { "on" } else { "off" })))
        }
        "transform_shapes" => {
            let ids: std::collections::BTreeSet<i32> = a["ids"].as_array().ok_or("ids is missing")?.iter().filter_map(|v| v.as_i64().map(|v| v as i32)).collect();
            let mut b = gcarve_core::geom::Rect::default();
            for id in &ids {
                b.add_rect(&bounds(&c.doc.find(*id).ok_or(format!("no shape {id}"))?.c));
            }
            let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
            let k = f(a, "scale").filter(|k| *k > 0.0).unwrap_or(1.0);
            let r = f(a, "rotate").unwrap_or(0.0).to_radians();
            let m = Affine::translate(-cx, -cy).then(&Affine::scale(k, k)).then(&Affine::rotate(r)).then(&Affine::translate(cx + f(a, "dx").unwrap_or(0.0), cy + f(a, "dy").unwrap_or(0.0)));
            c.sel = ids;
            c.transform_selection(&m);
            Ok(text("done"))
        }
        "delete_toolpath" => {
            let i = a["index"].as_u64().ok_or("index is missing")? as usize;
            if i >= c.doc.ops.len() {
                return Err(format!("no toolpath {i}"));
            }
            c.doc.checkpoint();
            let op = c.doc.ops.remove(i);
            c.mutated();
            Ok(text(format!("deleted toolpath {i} ({})", op.name)))
        }
        "check" => Ok(text(serde_json::to_string_pretty(&check(&c.doc)).unwrap_or_default())),
        "preview" => {
            let gen = pipeline::regenerate(&c.doc);
            let (rgba, w, h, _) = crate::setupsheet::carved(&c.doc, &gen, 400_000).ok_or("nothing to show: no toolpaths cut on this sheet")?;
            let mut png = Vec::new();
            image::RgbaImage::from_raw(w, h, rgba).ok_or("picture failed")?.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
            Ok(vec![json!({"type": "image", "data": gcarve_core::photo::b64_encode(&png), "mimeType": "image/png"}), json!({"type": "text", "text": "the board as the toolpaths would carve it (simulated)"})])
        }
        "show_drawing" => Ok(vec![json!({"type": "image", "data": gcarve_core::photo::b64_encode(&drawing_png(c)), "mimeType": "image/png"}), json!({"type": "text", "text": "the board (tan) and its vectors, up = +Y"})]),
        "undo" => {
            c.undo();
            Ok(text("undone"))
        }
        _ => Err(format!("unknown tool {name}")),
    }
}

/// The active sheet's vectors over the board, at most 900 px on the long side
fn drawing_png(c: &Canvas) -> Vec<u8> {
    let d = &c.doc;
    let mut ext = gcarve_core::geom::Rect { x0: 0.0, y0: 0.0, x1: d.material.w, y1: d.material.h };
    let shapes: Vec<_> = d.shapes.iter().filter(|s| s.sheet == d.active_sheet).collect();
    shapes.iter().for_each(|s| ext.add_rect(&bounds(&s.c)));
    let (pad, long) = (20.0, 900.0);
    let k = long / (ext.x1 - ext.x0).max(ext.y1 - ext.y0).max(1e-6);
    let (w, h) = (((ext.x1 - ext.x0) * k + 2.0 * pad) as u32, ((ext.y1 - ext.y0) * k + 2.0 * pad) as u32);
    let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([250, 250, 250, 255]));
    let px = |x: f64, y: f64| ((x - ext.x0) * k + pad, h as f64 - ((y - ext.y0) * k + pad));
    let (bx0, by1) = px(0.0, 0.0);
    let (bx1, by0) = px(d.material.w, d.material.h);
    for y in by0.max(0.0) as u32..(by1 as u32).min(h) {
        for x in bx0.max(0.0) as u32..(bx1 as u32).min(w) {
            img.put_pixel(x, y, image::Rgba([228, 205, 168, 255]));
        }
    }
    for s in &shapes {
        let col = if c.sel.contains(&s.id) { image::Rgba([30, 90, 220, 255]) } else { image::Rgba([40, 40, 40, 255]) };
        let mut p = gcarve_core::geom::flatten(&s.c, 0.5 / k);
        if s.c.closed {
            if let Some(f) = p.first().copied() {
                p.push(f);
            }
        }
        for seg in p.windows(2) {
            let (a, b) = (px(seg[0].x, seg[0].y), px(seg[1].x, seg[1].y));
            let n = (b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil().max(1.0) as usize;
            for i in 0..=n {
                let t = i as f64 / n as f64;
                let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (x, y) = (x as i64 + dx, y as i64 + dy);
                    if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
                        img.put_pixel(x as u32, y as u32, col);
                    }
                }
            }
        }
    }
    let mut png = Vec::new();
    let _ = img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png);
    png
}

fn summary(c: &Canvas, limit: usize, layer: Option<&str>) -> Value {
    let d = &c.doc;
    let m = &d.material;
    let on: Vec<_> = d.shapes.iter().filter(|s| s.sheet == d.active_sheet).collect();
    let mut pick: Vec<_> = on.iter().filter(|s| layer.is_none_or(|l| d.layers.get(s.layer as usize).is_some_and(|x| x.name == l))).collect();
    pick.sort_by_key(|s| !c.sel.contains(&s.id)); // selected first, then drawing order
    let more = pick.len().saturating_sub(limit);
    let shapes: Vec<Value> = pick.into_iter().take(limit).map(|s| {
        let b = bounds(&s.c);
        json!({"id": s.id, "layer": d.layers.get(s.layer as usize).map(|l| l.name.as_str()), "closed": s.c.closed, "box": [b.x0, b.y0, b.x1, b.y1], "text": s.text != 0})
    }).collect();
    let kinds = ["profile", "pocket", "drill", "vcarve", "bore", "inlay", "flute", "relief", "chamfer", "prism", "imported", "texture", "photo"];
    json!({
        "units": "inches, Y up, 0,0 = board bottom-left",
        "board": {"width": m.w, "height": m.h, "thickness": m.t, "material": m.kind, "post": m.post, "machine": m.machine.as_ref().map(|x| x.name.as_str())},
        "sheets": (0..d.sheets.len()).map(|j| d.sheet_name(j)).collect::<Vec<_>>(),
        "active_sheet": d.active_sheet,
        "layers": d.layers.iter().enumerate().map(|(i, l)| json!({"name": l.name, "vectors": on.iter().filter(|s| s.layer == i as i32).count(), "visible": l.visible, "locked": l.locked, "keep_out": l.keep_out})).collect::<Vec<_>>(),
        "vectors_not_listed": more,
        "selected": c.sel.iter().collect::<Vec<_>>(),
        "shapes": shapes,
        "texts": d.texts.iter().filter(|t| on.iter().any(|s| s.text == t.id)).map(|t| json!({"id": t.id, "text": t.text, "font": t.family, "height": t.height})).collect::<Vec<_>>(),
        "bits": d.tools.iter().enumerate().map(|(i, t)| json!({"index": i, "name": t.name, "type": format!("{:?}", t.kind), "diameter": t.diameter, "angle": t.angle})).collect::<Vec<_>>(),
        "toolpaths": d.ops.iter().enumerate().map(|(i, o)| json!({"index": i, "name": o.name, "kind": kinds.get(o.kind as usize), "bit": o.tool, "depth": o.depth, "side": format!("{:?}", o.side), "shapes": o.shape_ids.len(), "layer": d.layers.get(o.layer_scope.max(0) as usize).filter(|_| o.layer_scope >= 0).map(|l| l.name.as_str())})).collect::<Vec<_>>(),
    })
}

fn check(doc: &gcarve_core::document::Document) -> Value {
    let gen = pipeline::regenerate(doc);
    let eff: Vec<_> = doc.ops.iter().map(|o| if o.tool >= 0 && (o.tool as usize) < doc.tools.len() { pipeline::tool_for_cut(doc, o.tool as usize) } else { Default::default() }).collect();
    json!({
        "minutes": (gen.minutes.iter().sum::<f64>() * 10.0).round() / 10.0,
        "per_toolpath": doc.ops.iter().zip(&gen.paths).zip(&gen.minutes).zip(&eff).map(|(((o, p), m), t)| {
            let a = pipeline::assess(doc, t, t.feed_xy, t.feed_z, t.rpm, t.stepdown);
            let s = pipeline::suggest(doc, t);
            json!({"name": o.name, "cuts": !p.is_empty(), "minutes": (m * 10.0).round() / 10.0, "bit": t.name, "bit_index": o.tool,
                "feeds": {"feed_xy": t.feed_xy, "feed_z": t.feed_z, "rpm": t.rpm, "stepdown": t.stepdown, "advice": a.message, "level": a.level},
                "suggested": {"feed_xy": s.feed_xy.round(), "feed_z": s.feed_z.round(), "rpm": s.rpm, "stepdown": s.stepdown}})
        }).collect::<Vec<_>>(),
        "feed_units": "in/min, stepdown in; level 0 fine, 1 check, 2 problem",
        "warnings": gen.warnings,
        "order_problems": pipeline::order_issues(doc),
        "machine_problems": pipeline::machine_issues(doc, &eff),
        "safety": pipeline::keep_out_issues(doc, &eff, &gen.paths),
        "note": "no tool can promise a safe cut: tell the user to run File › Export Air-Cut Test first",
    })
}

// ---- transport ----

/// a request from a client connection, answered on `reply`
pub struct Incoming {
    pub body: String,
    pub reply: std::sync::mpsc::Sender<Option<String>>,
}

/// The local HTTP endpoint: one thread accepting on 127.0.0.1, each request handed to the app's frame loop.
/// Dropping the receiver ends it at the next connection.
#[cfg(not(target_arch = "wasm32"))]
pub fn serve(ctx: eframe::egui::Context) -> Result<std::sync::mpsc::Receiver<Incoming>, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", PORT)).map_err(|e| format!("port {PORT} is busy ({e}): is another GCarve open?"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            // ponytail: one thread per connection, fine for a local assistant or two
            std::thread::spawn(move || {
                let _ = connection(stream, &tx, &ctx);
            });
        }
    });
    Ok(rx)
}

#[cfg(not(target_arch = "wasm32"))]
fn connection(mut s: std::net::TcpStream, tx: &std::sync::mpsc::Sender<Incoming>, ctx: &eframe::egui::Context) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader, Read, Write};
    s.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
    let mut r = BufReader::new(s.try_clone()?);
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let mut parts = line.split_whitespace();
        let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
        let (mut len, mut host, mut origin, mut close) = (0usize, String::new(), String::new(), false);
        loop {
            let mut h = String::new();
            r.read_line(&mut h)?;
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            let (k, v) = h.split_once(':').unwrap_or((h, ""));
            match k.to_ascii_lowercase().as_str() {
                "content-length" => len = v.trim().parse().unwrap_or(0),
                "host" => host = v.trim().to_string(),
                "origin" => origin = v.trim().to_string(),
                "connection" => close = v.trim().eq_ignore_ascii_case("close"),
                _ => {}
            }
        }
        if len > 16 << 20 {
            return Ok(());
        }
        let mut body = vec![0; len];
        r.read_exact(&mut body)?;
        let (status, out) = route(&method, &path, &host, &origin, String::from_utf8_lossy(&body).into_owned(), tx, ctx);
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", out.len());
        s.write_all(head.as_bytes())?;
        s.write_all(out.as_bytes())?;
        if close {
            return Ok(());
        }
    }
}

/// Only this computer: the Host must be the loopback address (a web page can't reach it through DNS rebinding) and
/// a browser Origin, when there is one, must be local too.
#[cfg(not(target_arch = "wasm32"))]
fn local(host: &str, origin: &str) -> bool {
    let ok = |h: &str| h == "127.0.0.1" || h == "localhost" || h == "[::1]";
    let hostname = |h: &str| h.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map_or(h.to_string(), |(a, _)| a.to_string());
    ok(&hostname(host)) && (origin.is_empty() || ok(&hostname(origin.trim_start_matches("http://").trim_start_matches("https://"))))
}

#[cfg(not(target_arch = "wasm32"))]
fn route(method: &str, path: &str, host: &str, origin: &str, body: String, tx: &std::sync::mpsc::Sender<Incoming>, ctx: &eframe::egui::Context) -> (&'static str, String) {
    if !local(host, origin) {
        return ("403 Forbidden", r#"{"error":"only programs on this computer may use GCarve"}"#.into());
    }
    if path != "/mcp" {
        return ("404 Not Found", "{}".into());
    }
    if method != "POST" {
        return ("405 Method Not Allowed", "{}".into());
    }
    let (rtx, rrx) = std::sync::mpsc::channel();
    if tx.send(Incoming { body, reply: rtx }).is_err() {
        return ("503 Service Unavailable", "{}".into());
    }
    ctx.request_repaint();
    match rrx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(Some(out)) => ("200 OK", out),
        Ok(None) => ("202 Accepted", String::new()),
        Err(_) => ("504 Gateway Timeout", "{}".into()),
    }
}

/// A body (one message or a batch) → the answer text (None = only notifications)
pub fn answer(app: &mut dyn McpApp, body: &str) -> Option<String> {
    let parse_err = || Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "not JSON"}}).to_string());
    let Ok(v) = serde_json::from_str::<Value>(body) else { return parse_err() };
    match v {
        Value::Array(list) => {
            let out: Vec<Value> = list.iter().filter_map(|m| handle(app, m)).collect();
            (!out.is_empty()).then(|| Value::Array(out).to_string())
        }
        m => handle(app, &m).map(|o| o.to_string()),
    }
}

/// `gcarve --mcp`: stdio clients (one JSON message per line) passed on to the open app's local endpoint.
#[cfg(not(target_arch = "wasm32"))]
pub fn stdio_bridge() {
    use std::io::{BufRead, Read, Write};
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = (|| -> std::io::Result<String> {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", PORT))?;
            write!(s, "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{line}", line.len())?;
            let mut out = String::new();
            s.read_to_string(&mut out)?;
            Ok(out.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default())
        })();
        let out = match reply {
            Ok(b) => b,
            Err(_) => {
                let id = serde_json::from_str::<Value>(&line).ok().and_then(|v| v.get("id").cloned());
                match id {
                    Some(id) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": "GCarve isn't open, or AI access is off: open GCarve, then Help › AI Assistants… › Let AI assistants use GCarve"}}).to_string(),
                    None => String::new(),
                }
            }
        };
        if !out.trim().is_empty() {
            let mut o = std::io::stdout().lock();
            let _ = writeln!(o, "{}", out.trim());
            let _ = o.flush();
        }
    }
}

/// App-side state: the dialog, the switch, the running endpoint
#[derive(Default)]
pub struct State {
    pub open: bool,
    pub rx: Option<std::sync::mpsc::Receiver<Incoming>>,
    started: bool,
    err: String,
}

impl State {
    /// starts the endpoint once when the switch is on (store `mcp-on`); true while it runs
    pub fn start_if_on(&mut self, ctx: &eframe::egui::Context, shot: bool) -> bool {
        if !self.started {
            self.started = true;
            if !shot && !cfg!(test) && crate::store::load("mcp-on").as_deref() == Some("1") {
                self.set(ctx, true);
            }
        }
        self.rx.is_some()
    }

    fn set(&mut self, ctx: &eframe::egui::Context, on: bool) {
        self.err.clear();
        crate::store::save("mcp-on", if on { "1" } else { "0" });
        if !on {
            // ponytail: the accept thread lives until the next connection finds the channel gone; restart = same port
            self.rx = None;
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        match serve(ctx.clone()) {
            Ok(rx) => self.rx = Some(rx),
            Err(e) => self.err = e,
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }
}

/// Help › AI Assistants…
pub fn settings_ui(st: &mut State, ctx: &eframe::egui::Context) {
    use eframe::egui;
    use gcarve_ui::i18n;
    if !st.open {
        return;
    }
    let mut open = true;
    crate::widgets::Dialog::new(i18n::t("AI Assistants")).open(&mut open).close_footer().width(460.0).show(ctx, |ui| {
        ui.label(i18n::t("Let an AI assistant (ChatGPT, Gemini, Copilot, Cursor, LM Studio … any app that speaks MCP) look at the open drawing and work in it: draw, add toolpaths, check them, show a preview, make the G-code. You see every change and can undo it."));
        ui.add_space(6.0);
        let mut on = st.rx.is_some();
        if crate::widgets::checkbox(ui, &mut on, i18n::t("Let AI assistants use GCarve")).changed() {
            st.set(ctx, on);
        }
        if !st.err.is_empty() {
            ui.label(egui::RichText::new(&st.err).color(egui::Color32::from_rgb(220, 120, 60)));
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new(i18n::t("Apps that take an address:")).strong());
        ui.horizontal(|ui| {
            ui.monospace(URL);
            if ui.small_button(i18n::t("Copy")).clicked() {
                ui.ctx().copy_text(URL.into());
            }
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new(i18n::t("Apps that start a program, in their MCP settings:")).strong());
        let cfg = setup_json();
        egui::ScrollArea::horizontal().id_salt("mcp-cfg").show(ui, |ui| {
            ui.monospace(&cfg);
        });
        if ui.small_button(i18n::t("Copy")).clicked() {
            ui.ctx().copy_text(cfg);
        }
        crate::dialogs::hint(ui, i18n::t("Only programs on this computer can connect, and only while GCarve is open with this switched on. An assistant can make mistakes: check the toolpaths in Simulate and run an air cut before cutting."));
    });
    st.open = open;
}

/// Setup text shown in Help › AI Assistants: the two ways in, and a config block most clients take
pub fn setup_json() -> String {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "gcarve".into());
    serde_json::to_string_pretty(&json!({"mcpServers": {"gcarve": {"command": exe, "args": ["--mcp"]}}})).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct T(Canvas, Vec<String>);
    impl McpApp for T {
        fn canvas(&mut self) -> &mut Canvas {
            &mut self.0
        }
        fn post(&self) -> PostDef {
            gcarve_core::postdef::post_library().iter().find(|p| p.id.starts_with("grbl")).unwrap().clone()
        }
        fn project(&self) -> String {
            "mcp".into()
        }
        fn commands(&self) -> Vec<(String, String)> {
            vec![("fit".into(), "Zoom to fit".into())]
        }
        fn run(&mut self, id: &str) {
            self.1.push(id.into());
        }
        fn open(&mut self, path: &str, _import: bool) -> Result<String, String> {
            Err(format!("no {path}"))
        }
        fn save(&mut self, _path: Option<&str>) -> Result<String, String> {
            Ok("saved".into())
        }
    }

    fn call_tool(app: &mut T, name: &str, args: Value) -> Value {
        let r = handle(app, &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})).unwrap();
        r["result"].clone()
    }

    #[test]
    fn an_assistant_draws_a_sign_adds_toolpaths_checks_and_exports() {
        let mut app = T(Canvas::default(), vec![]);
        let init = handle(&mut app, &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle(&mut app, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        let list = handle(&mut app, &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
        assert!(list["result"]["tools"].as_array().unwrap().len() >= 12);

        call_tool(&mut app, "set_board", json!({"width": 12, "height": 8, "thickness": 0.75, "material": "Plywood"}));
        let r = call_tool(&mut app, "add_shapes", json!({"shapes": [{"type": "rect", "x": 1, "y": 1, "w": 10, "h": 6}], "layer": "Outline"}));
        assert!(r["isError"].is_null(), "{r}");
        call_tool(&mut app, "add_text", json!({"text": "HELLO", "x": 6, "y": 4, "height": 2}));
        let letters: Vec<i32> = app.0.doc.shapes.iter().filter(|s| s.text != 0).map(|s| s.id).collect();
        assert!(!letters.is_empty());
        call_tool(&mut app, "add_toolpath", json!({"kind": "vcarve", "shape_ids": letters, "depth": 0.2, "flat_depth": 0.1}));
        let r = call_tool(&mut app, "add_toolpath", json!({"kind": "profile", "layer": "Outline", "depth": 0.75, "side": "outside", "tabs": 4, "bit": {"type": "end_mill", "diameter": 0.25}}));
        assert!(r["isError"].is_null(), "{r}");
        assert_eq!(app.0.doc.ops.len(), 2);

        let d: Value = serde_json::from_str(call_tool(&mut app, "get_drawing", json!({}))["content"][0]["text"].as_str().unwrap()).unwrap();
        let one: Value = serde_json::from_str(call_tool(&mut app, "get_drawing", json!({"limit": 1}))["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(one["shapes"].as_array().unwrap().len(), 1);
        assert_eq!(one["vectors_not_listed"].as_u64().unwrap() + 1, d["shapes"].as_array().unwrap().len() as u64);
        assert_eq!(d["board"]["width"], 12.0);
        assert_eq!(d["toolpaths"].as_array().unwrap().len(), 2);

        let tid = d["texts"][0]["id"].as_i64().unwrap();
        let r = call_tool(&mut app, "edit_text", json!({"id": tid, "text": "HI"}));
        assert!(r["isError"].is_null(), "{r}");
        let hi: Vec<i32> = app.0.doc.shapes.iter().filter(|s| s.text == tid as i32).map(|s| s.id).collect();
        assert!(hi.len() < letters.len(), "fewer letters");
        let (centre, _) = app.0.text_centre(tid as i32).unwrap();
        assert!((centre.x - 6.0).abs() < 0.01 && (centre.y - 4.0).abs() < 0.01, "stays where it was: {centre:?}");
        let mut vc = app.0.doc.ops[0].shape_ids.clone();
        vc.sort();
        assert_eq!(vc, hi, "the V-carve follows the new letters");

        call_tool(&mut app, "select_shapes", json!({"layer": "Outline"}));
        let outline = app.0.doc.layers.iter().position(|l| l.name == "Outline").unwrap() as i32;
        assert!(!app.0.sel.is_empty() && app.0.sel.iter().all(|id| app.0.doc.find(*id).unwrap().layer == outline));
        call_tool(&mut app, "select_shapes", json!({}));
        assert!(app.0.sel.is_empty());
        let r = call_tool(&mut app, "layer", json!({"name": "Clamps", "ids": [hi[0]], "keep_out": true}));
        let r = r["content"][0]["text"].as_str().unwrap();
        assert!(r.contains("1 vectors moved") && r.contains("keep-out on"), "{r}");
        let d = &app.0.doc;
        assert!(d.layers.iter().any(|l| l.name == "Clamps" && l.keep_out) && d.layers[d.find(hi[0]).unwrap().layer as usize].name == "Clamps");
        call_tool(&mut app, "undo", json!({}));
        assert!(!app.0.doc.layers.iter().any(|l| l.keep_out));

        let r = call_tool(&mut app, "set_machine", json!({"name": "shapeoko"}));
        assert!(r["isError"] == true && r["content"][0]["text"].as_str().unwrap().contains("pick one"), "{r}");
        let r = call_tool(&mut app, "set_machine", json!({"name": "Shapeoko 4 XXL"}));
        assert!(r["isError"].is_null(), "{r}");
        assert!(app.0.doc.sheets.iter().all(|s| s.machine.as_ref().is_some_and(|m| m.name.contains("Shapeoko"))));
        assert!(app.0.doc.material.post.starts_with("shapeoko"), "{}", app.0.doc.material.post);

        let b = app.0.doc.ops[0].tool as usize;
        let r = call_tool(&mut app, "set_bit_feeds", json!({"bit": b, "feed_xy": 77, "rpm": 15000}));
        assert!(r["isError"].is_null(), "{r}");
        let t = pipeline::tool_for_cut(&app.0.doc, b);
        assert_eq!((t.feed_xy, t.rpm), (77.0, 15000), "the bit now cuts at what was set");
        let c: Value = serde_json::from_str(call_tool(&mut app, "check", json!({}))["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(c["per_toolpath"][0]["feeds"]["feed_xy"], 77.0, "check reports the toolpath's own bit: {c}");

        call_tool(&mut app, "sheet", json!({"add": true, "name": "Lid"}));
        assert_eq!((app.0.doc.sheets.len(), app.0.doc.active_sheet), (2, 1));
        assert_eq!(app.0.doc.sheet_name(1), "Lid");
        assert!(call_tool(&mut app, "sheet", json!({"go_to": 5}))["isError"] == true);
        call_tool(&mut app, "sheet", json!({"go_to": 0}));
        assert_eq!(app.0.doc.active_sheet, 0);

        let c: Value = serde_json::from_str(call_tool(&mut app, "check", json!({}))["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(c["per_toolpath"].as_array().unwrap().iter().all(|p| p["cuts"] == true), "{c}");
        assert!(c["minutes"].as_f64().unwrap() > 0.0);

        let g = call_tool(&mut app, "export_gcode", json!({}));
        let g = g["content"][0]["text"].as_str().unwrap();
        assert!(g.contains("M5") || g.contains("M05"), "spindle off in the files");

        let shot = call_tool(&mut app, "show_drawing", json!({}));
        assert_eq!(shot["content"][0]["mimeType"], "image/png");
        if let Ok(out) = std::env::var("GCARVE_MCP_DRAWING") {
            std::fs::write(out, gcarve_core::photo::b64_decode(shot["content"][0]["data"].as_str().unwrap())).unwrap();
        }
        let p = call_tool(&mut app, "preview", json!({}));
        assert_eq!(p["content"][0]["mimeType"], "image/png");

        // move and grow the outline, change the cut-out
        let out: Vec<i32> = app.0.doc.shapes.iter().filter(|s| s.text == 0).map(|s| s.id).collect();
        call_tool(&mut app, "transform_shapes", json!({"ids": out, "dx": 0.5, "scale": 1.0}));
        let b = bounds(&app.0.doc.find(out[0]).unwrap().c);
        assert!((b.x0 - 1.5).abs() < 1e-6, "{b:?}");
        call_tool(&mut app, "edit_toolpath", json!({"index": 1, "depth": 0.8, "tabs": 0}));
        assert!(app.0.doc.ops[1].depth == 0.8 && !app.0.doc.ops[1].tabs);
        assert!(c["per_toolpath"][0]["suggested"]["rpm"].as_i64().unwrap() > 0, "{c}");

        // errors come back as tool errors, not crashes; undo takes the last change back
        assert_eq!(call_tool(&mut app, "add_toolpath", json!({"kind": "pocket", "shape_ids": [9999], "depth": 0.1}))["isError"], true);
        // a chamfer with no settings takes the dialog's defaults and a V-bit
        let r = call_tool(&mut app, "add_toolpath", json!({"kind": "chamfer", "shape_ids": [out[0]]}));
        assert!(r["isError"].is_null(), "{r}");
        let ch = app.0.doc.ops.last().unwrap();
        assert!(ch.kind == OpType::Chamfer && ch.chamfer_width > 0.0 && app.0.doc.tools[ch.tool as usize].kind == ToolType::VBit);
        call_tool(&mut app, "undo", json!({}));
        call_tool(&mut app, "undo", json!({}));
        assert_eq!(app.0.doc.ops[1].depth, 0.75, "undo took the edit back");
        assert_eq!(call_tool(&mut app, "run_command", json!({"id": "nope"}))["isError"], true);
        call_tool(&mut app, "run_command", json!({"id": "fit"}));
        assert_eq!(app.1, vec!["fit"]);
    }

    #[test]
    fn only_this_computer_gets_in() {
        assert!(local("127.0.0.1:47110", ""));
        assert!(local("localhost:47110", "http://localhost:3000"));
        assert!(!local("evil.example:47110", ""));
        assert!(!local("127.0.0.1:47110", "https://evil.example"));
        assert!(!local("127.0.0.1:47110", "null"));
    }

    /// the real endpoint: a POST from this computer is answered by the frame loop, another Host is refused
    #[test]
    fn the_local_endpoint_answers_and_refuses_other_hosts() {
        use std::io::{Read, Write};
        let Ok(rx) = serve(eframe::egui::Context::default()) else { return }; // port taken (GCarve is open): skip
        let post = |host: &str| {
            let host = host.to_string();
            std::thread::spawn(move || {
                let body = r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#;
                let mut s = std::net::TcpStream::connect(("127.0.0.1", PORT)).unwrap();
                write!(s, "POST /mcp HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
                let mut out = String::new();
                let _ = s.read_to_string(&mut out);
                out
            })
        };
        let bad = post("rebind.example:47110").join().unwrap();
        assert!(bad.starts_with("HTTP/1.1 403"), "{bad}");
        let good = post("127.0.0.1:47110");
        let mut app = T(Canvas::default(), vec![]);
        let m = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        m.reply.send(answer(&mut app, &m.body)).unwrap();
        let out = good.join().unwrap();
        assert!(out.starts_with("HTTP/1.1 200") && out.contains("add_toolpath"), "{out}");
    }

    #[test]
    fn batches_and_bad_json() {
        let mut app = T(Canvas::default(), vec![]);
        let out = answer(&mut app, r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"}]"#).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap().as_array().unwrap().len(), 1);
        assert!(answer(&mut app, "{oops").unwrap().contains("-32700"));
    }
}
