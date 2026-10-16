//! Toolpath templates (VCarve-style), port of C++ core/template.* + the .gctemplate JSON of app/project.cpp (same bytes).
//! A template keeps a document's toolpaths with their parameters, bindings, the tools they use and the variables their
//! expressions use; applying it re-points each toolpath at the layer of the same name.

use serde_json::{json, Value};

use crate::document::*;
use crate::expr::identifiers;
use crate::project::{arr, get, op_from, op_json, qt_json, s, tool_from_json, tool_to_json};

#[derive(Clone, Debug, PartialEq)]
pub struct TemplateOp {
    /// tool / clear_tool index `ToolpathTemplate::tools`; no shapes, no scope
    pub op: Op,
    /// shapes come from this layer (its scope layer, else the layer of its first shape)
    pub layer: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ToolpathTemplate {
    pub name: String,
    pub description: String,
    pub tools: Vec<Tool>,
    pub variables: Vec<Variable>,
    pub ops: Vec<TemplateOp>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TemplateApplyResult {
    pub applied: usize,
    /// "<op name>: <reason>"
    pub skipped: Vec<String>,
    pub tools_added: Vec<String>,
    pub variables_added: Vec<String>,
}

/// ponytail: 3D model (relief) toolpaths are left out: they point at a model, not at shapes.
pub fn make_template(d: &Document, name: &str, description: &str) -> ToolpathTemplate {
    let mut t = ToolpathTemplate { name: name.into(), description: description.into(), ..Default::default() };
    let layer_name = |l: i32| usize::try_from(l).ok().and_then(|i| d.layers.get(i)).map(|l| l.name.clone());
    let layer_of = |id: i32| d.find(id).and_then(|sh| layer_name(sh.layer)).unwrap_or_default();
    let mut pending: Vec<String> = Vec::new();
    for src in &d.ops {
        if src.kind == OpType::Relief {
            continue;
        }
        let layer = if let Some(n) = (src.layer_scope >= 0).then(|| layer_name(src.layer_scope)).flatten() {
            n
        } else if src.group_scope > 0 {
            d.shapes.iter().find(|sh| sh.group == src.group_scope).map(|sh| layer_of(sh.id)).unwrap_or_default()
        } else {
            src.shape_ids.iter().map(|&id| layer_of(id)).find(|n| !n.is_empty()).unwrap_or_default()
        };
        let mut op = src.clone();
        op.shape_ids.clear();
        op.layer_scope = -1;
        op.group_scope = 0;
        op.relief = -1;
        op.tool = template_tool(&mut t.tools, &d.tools, src.tool).max(0);
        op.clear_tool = template_tool(&mut t.tools, &d.tools, src.clear_tool);
        pending.extend(src.bind.values().flat_map(|e| identifiers(e)));
        t.ops.push(TemplateOp { op, layer });
    }
    let mut used: Vec<String> = Vec::new(); // plus the variables those variables use
    while let Some(n) = pending.pop() {
        let Some(v) = d.find_variable(&n) else { continue };
        if used.contains(&n) {
            continue;
        }
        used.push(n);
        pending.extend(identifiers(&v.expr));
    }
    t.variables = d.variables.iter().filter(|v| used.contains(&v.name)).cloned().collect();
    t
}

/// index of document tool `i` in the template's tool list (added once, by name); -1 = none
fn template_tool(out: &mut Vec<Tool>, tools: &[Tool], i: i32) -> i32 {
    let Some(tool) = usize::try_from(i).ok().and_then(|i| tools.get(i)) else { return -1 };
    if let Some(k) = out.iter().position(|t| t.name == tool.name) {
        return k as i32;
    }
    out.push(tool.clone());
    out.len() as i32 - 1
}

/// Appends the ops to `d`, layer-scoped to the layers of the same name. Ops whose layer is missing or has no shapes
/// (on any sheet) are skipped and listed. Tools are matched by name or added; missing variables are added.
pub fn apply_template(t: &ToolpathTemplate, d: &mut Document) -> TemplateApplyResult {
    let mut r = TemplateApplyResult::default();
    for v in &t.variables {
        if d.find_variable(&v.name).is_none() {
            d.variables.push(v.clone());
            r.variables_added.push(v.name.clone());
        }
    }
    let tool = |i: i32, d: &mut Document, r: &mut TemplateApplyResult| {
        let Some(tt) = usize::try_from(i).ok().and_then(|i| t.tools.get(i)) else { return -1 };
        let before = d.tools.len();
        let k = tool_index_for(&mut d.tools, &tt.name, &t.tools);
        if d.tools.len() > before {
            r.tools_added.push(tt.name.clone());
        }
        k
    };
    for to in &t.ops {
        let layer = d.layers.iter().position(|l| l.name == to.layer);
        let Some(layer) = layer.filter(|_| !to.layer.is_empty()) else {
            r.skipped.push(format!("{}: no layer \"{}\"", to.op.name, to.layer));
            continue;
        };
        if !d.shapes.iter().any(|sh| sh.layer == layer as i32) {
            r.skipped.push(format!("{}: layer \"{}\" has no shapes", to.op.name, to.layer));
            continue;
        }
        let mut op = to.op.clone();
        op.layer_scope = layer as i32;
        op.tool = tool(to.op.tool, d, &mut r).max(0);
        op.clear_tool = tool(to.op.clear_tool, d, &mut r);
        d.ops.push(op);
        r.applied += 1;
    }
    r
}

/// A few lines for the UI (same text as the C++ summaryText).
pub fn summary_text(r: &TemplateApplyResult) -> String {
    let mut s = format!("{} toolpath{} added", r.applied, if r.applied == 1 { "" } else { "s" });
    for (head, v) in [("Skipped (no shapes found):", &r.skipped), ("Tools added:", &r.tools_added), ("Variables added:", &r.variables_added)] {
        if !v.is_empty() {
            s += &format!("\n{head}");
            for x in v {
                s += &format!("\n  {x}");
            }
        }
    }
    s
}

/// .gctemplate JSON (indented, Qt format: byte-identical to the C++ saveTemplate)
pub fn save_template(t: &ToolpathTemplate) -> String {
    let ops: Vec<Value> = t
        .ops
        .iter()
        .map(|to| {
            let mut o = op_json(&to.op, &t.tools);
            o.as_object_mut().unwrap().insert("layer".into(), Value::from(to.layer.as_str()));
            o
        })
        .collect();
    let vars: Vec<Value> = t.variables.iter().map(|v| json!({"name": v.name, "expr": v.expr})).collect();
    let tools: Vec<Value> = t.tools.iter().map(tool_to_json).collect();
    qt_json(
        &json!({"app": "GCarve", "template": 1, "name": t.name, "description": t.description, "tools": tools, "variables": vars, "ops": ops}),
        true,
    )
}

pub fn load_template(text: &str) -> Result<ToolpathTemplate, String> {
    let root: Value = serde_json::from_str(text).map_err(|_| "Not a GCarve toolpath template".to_string())?;
    if get(&root, "app").as_str() != Some("GCarve") || root.get("template").is_none() {
        return Err("Not a GCarve toolpath template".into());
    }
    if get(&root, "template").as_i64().unwrap_or(0) > 1 {
        return Err("Template was saved by a newer GCarve".into());
    }
    let tools: Vec<Tool> = arr(get(&root, "tools")).iter().map(tool_from_json).collect();
    let n = tools.len() as i32;
    Ok(ToolpathTemplate {
        name: s(&root, "name", "Template"),
        description: s(&root, "description", ""),
        variables: arr(get(&root, "variables"))
            .iter()
            .filter(|o| !s(o, "name", "").is_empty())
            .map(|o| Variable { name: s(o, "name", ""), expr: s(o, "expr", "0") })
            .collect(),
        ops: arr(get(&root, "ops")).iter().map(|o| TemplateOp { op: op_from(o, n), layer: s(o, "layer", "") }).collect(),
        tools,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::make_rect;

    #[test]
    fn save_apply_roundtrip() {
        let mut d = Document::default();
        d.layers = vec![Layer { name: "Cut".into(), ..Default::default() }, Layer { name: "Pocket".into(), ..Default::default() }];
        let a = d.add(make_rect(0.0, 0.0, 2.0, 2.0), Some(0));
        d.add(make_rect(3.0, 0.0, 4.0, 1.0), Some(1));
        d.variables = vec![
            Variable { name: "t2".into(), expr: "thickness / 2".into() },
            Variable { name: "deep".into(), expr: "t2 + 0.1".into() },
            Variable { name: "unused".into(), expr: "1".into() },
        ];
        let mut cut = Op { name: "Cut".into(), shape_ids: vec![a], tool: 1, ..Default::default() };
        cut.bind.insert("depth".into(), "deep".into());
        let pocket = Op { kind: OpType::Pocket, name: "Pocket".into(), layer_scope: 1, tool: 2, clear_tool: 1, ..Default::default() };
        d.ops = vec![cut, pocket];

        let t = make_template(&d, "Sign", "two ops");
        assert_eq!(t.tools.len(), 2);
        assert_eq!(t.variables.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["t2", "deep"]);
        assert_eq!((t.ops[0].layer.as_str(), t.ops[1].layer.as_str()), ("Cut", "Pocket"));
        let t2 = load_template(&save_template(&t)).unwrap();
        assert_eq!(t2, t);

        // target: no "Pocket" shapes, one tool, no variables
        let mut e = Document::default();
        e.layers = vec![Layer { name: "Cut".into(), ..Default::default() }, Layer { name: "Pocket".into(), ..Default::default() }];
        e.add(make_rect(0.0, 0.0, 5.0, 5.0), Some(0));
        e.tools.truncate(1);
        let r = apply_template(&t2, &mut e);
        assert_eq!(r.applied, 1);
        assert_eq!(r.skipped, ["Pocket: layer \"Pocket\" has no shapes"]);
        assert_eq!(r.variables_added, ["t2", "deep"]);
        assert_eq!(r.tools_added, [d.tools[1].name.clone()]);
        assert_eq!((e.ops[0].layer_scope, e.ops[0].bind["depth"].as_str()), (0, "deep"));
        assert_eq!(e.tools[e.ops[0].tool as usize].name, d.tools[1].name);
        assert!(load_template("{\"app\":\"GCarve\"}").is_err());
    }
}
