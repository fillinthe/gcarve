//! The user's bit library (Qt: ~/Library/Application Support/GCarve/GCarve/tools.json): loaded into new projects,
//! saved whenever the Tools dialog edits it. Imports: GCarve tools.json and Fusion 360 tools.json; Vectric .vtdb on the
//! desktop (vtdb.rs).

use gcarve_core::document::{Tool, ToolType};
use gcarve_core::project::{tool_library_from_json, tool_library_to_json};
use serde_json::Value;

const KEY: &str = "tools.json";

pub fn load() -> Vec<Tool> {
    tool_library_from_json(&crate::store::load(KEY).unwrap_or_default())
}

pub fn save(tools: &[Tool]) {
    crate::store::save(KEY, &tool_library_to_json(tools));
    crate::library::touched(); // goes up to the account (App::sync_library)
}

/// Merge `incoming` into `tools` by name (same name = replaced, keeping its tool-changer slot when the import has
/// none). Returns (added, updated); the added ones are the last `added` tools.
pub fn merge(tools: &mut Vec<Tool>, incoming: Vec<Tool>) -> (usize, usize) {
    let (mut added, mut updated) = (0, 0);
    for mut t in incoming {
        match tools.iter_mut().find(|x| x.name == t.name) {
            Some(x) => {
                if t.slot == 0 {
                    t.slot = x.slot;
                }
                *x = t;
                updated += 1;
            }
            None => {
                tools.push(t);
                added += 1;
            }
        }
    }
    (added, updated)
}

/// Tool-changer machines (`slots` > 0): bits from index `from` on without a slot get the lowest free one (rack full = 0).
pub fn assign_slots(tools: &mut [Tool], from: usize, slots: i32) {
    if slots <= 0 {
        return;
    }
    for i in from..tools.len() {
        if tools[i].slot == 0 {
            let used: Vec<i32> = tools.iter().map(|t| t.slot).collect();
            tools[i].slot = (1..=slots).find(|s| !used.contains(s)).unwrap_or(0);
        }
    }
}

/// Slots of the document's machine (0 = manual tool changes).
pub fn machine_slots(doc: &gcarve_core::document::Document) -> i32 {
    doc.material.machine.as_ref().map_or(0, |m| m.tool_slots)
}

/// Fusion 360 tool library (`tools.json` inside the .tools zip): `data[]` with geometry DC/TA/NOF, unit, start-values
/// presets (v_f, v_f_plunge, n). Same mapping as the Qt importer.
pub fn fusion_tools_from_json(text: &str) -> Result<Vec<Tool>, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let data = root.get("data").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or("No tools found (for a .tools file, unzip it and pick tools.json)")?;
    let mut out = Vec::new();
    for o in data {
        let kind = o.get("type").and_then(Value::as_str).unwrap_or("");
        if kind.contains("holder") || kind.contains("probe") {
            continue;
        }
        let geo = o.get("geometry").cloned().unwrap_or(Value::Null);
        let k = if o.get("unit").and_then(Value::as_str) == Some("millimeters") { 1.0 / 25.4 } else { 1.0 };
        let ta = geo.get("TA").and_then(Value::as_f64).unwrap_or(0.0);
        let vbit = kind.contains("chamfer") || kind.contains("engrav") || ta > 0.0;
        let mut t = Tool { name: o.get("description").and_then(Value::as_str).unwrap_or("Fusion tool").to_string(), kind: if vbit { ToolType::VBit } else { ToolType::EndMill }, ..Default::default() };
        t.diameter = geo.get("DC").and_then(Value::as_f64).unwrap_or(0.25) * k;
        if vbit {
            t.angle = if ta > 0.0 { 2.0 * ta } else { 90.0 };
        }
        if let Some(n) = geo.get("NOF").and_then(Value::as_i64).filter(|&n| n > 0) {
            t.flutes = n as i32;
        }
        if let Some(p) = o.get("start-values").and_then(|s| s.get("presets")).and_then(Value::as_array).and_then(|a| a.first()) {
            t.feed_xy = p.get("v_f").and_then(Value::as_f64).unwrap_or(100.0) * k;
            t.feed_z = p.get("v_f_plunge").and_then(Value::as_f64).map(|v| v * k).unwrap_or(t.feed_xy / 2.0);
            t.rpm = p.get("n").and_then(Value::as_f64).unwrap_or(18000.0) as i32;
        }
        t.stepdown = t.diameter;
        out.push(t);
    }
    Ok(out)
}

/// A picked file: GCarve library (`tools` array) or Fusion (`data` array).
pub fn import(text: &str) -> Result<Vec<Tool>, String> {
    if text.contains("\"data\"") && !text.contains("\"tools\"") {
        return fusion_tools_from_json(text);
    }
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if v.get("tools").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
        Ok(tool_library_from_json(text))
    } else {
        Err("Not a GCarve bit library or Fusion tools.json".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fusion_import_maps_units_and_vbits() {
        let json = r#"{"data":[{"type":"flat end mill","unit":"millimeters","description":"6mm flat","geometry":{"DC":6.35,"NOF":3},
            "start-values":{"presets":[{"v_f":2540,"v_f_plunge":508,"n":16000}]}},
            {"type":"chamfer mill","unit":"inches","description":"V 60","geometry":{"DC":0.5,"TA":30}},
            {"type":"holder","description":"skip"}]}"#;
        let t = fusion_tools_from_json(json).unwrap();
        assert_eq!(t.len(), 2);
        assert!((t[0].diameter - 0.25).abs() < 1e-9 && t[0].flutes == 3 && (t[0].feed_xy - 100.0).abs() < 1e-9 && t[0].rpm == 16000);
        assert_eq!(t[1].kind, ToolType::VBit);
        assert_eq!(t[1].angle, 60.0);
        let mut lib = vec![Tool { name: "V 60".into(), ..Default::default() }];
        assert_eq!(merge(&mut lib, t), (1, 1));
    }

    #[test]
    fn slots_fill_the_lowest_free_pocket() {
        let mut lib: Vec<Tool> = (0..4).map(|i| Tool { name: format!("t{i}"), slot: [2, 0, 0, 0][i], ..Default::default() }).collect();
        assign_slots(&mut lib, 0, 0);
        assert_eq!(lib[1].slot, 0, "manual machines keep slot 0");
        assign_slots(&mut lib, 2, 3);
        assert_eq!(lib.iter().map(|t| t.slot).collect::<Vec<_>>(), [2, 0, 1, 3]);
        let n = lib.len();
        lib.push(Tool { name: "full".into(), ..Default::default() });
        assign_slots(&mut lib, n, 3);
        assert_eq!(lib[n].slot, 0, "rack full");
        merge(&mut lib, vec![Tool { name: "t0".into(), ..Default::default() }]);
        assert_eq!(lib[0].slot, 2, "re-import keeps the slot");
    }
}
