//! Vectric tool database (.vtdb = SQLite) import (Qt Canvas::importVectricTools): geometry rows become tools, the
//! material-less cutting data their defaults, per-material rows `FeedPreset`s. Desktop reads the file with rusqlite;
//! the browser loads sql.js from cdnjs on first use (web below). Both hand the same rows to `tools_from`.

use gcarve_core::document::{FeedPreset, Tool, ToolType};
use std::collections::HashMap;

/// The three queries (column order matters: the web side returns the same arrays)
const Q_MATERIALS: &str = "select id, name from material";
const Q_GEOMETRY: &str = "select g.id, g.name_format, g.units, g.diameter, g.included_angle, t.name, g.num_flutes from tool_geometry g left join tool_tree_entry t on t.tool_geometry_id = g.id";
const Q_CUTTING: &str = "select e.tool_geometry_id, e.material_id, d.rate_units, d.feed_rate, d.plunge_rate, d.spindle_speed, d.stepdown, d.length_units from tool_entity e join tool_cutting_data d on d.id = e.tool_cutting_data_id";

/// One cell as read: ids are TEXT (uuids) in current files, INTEGER in older ones; numbers may be TEXT
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(untagged)]
pub enum Cell {
    Num(f64),
    Text(String),
    Null(()),
}

impl Cell {
    fn id(&self) -> String {
        match self {
            Cell::Text(s) => s.clone(),
            Cell::Num(v) => format!("{}", *v as i64),
            Cell::Null(_) => String::new(),
        }
    }
    fn num(&self) -> f64 {
        match self {
            Cell::Num(v) => *v,
            Cell::Text(s) => s.trim().parse().unwrap_or(0.0),
            Cell::Null(_) => 0.0,
        }
    }
    fn text(&self) -> String {
        match self {
            Cell::Text(s) => s.clone(),
            _ => String::new(),
        }
    }
}

#[derive(Default, serde::Deserialize)]
pub struct Rows {
    pub materials: Vec<Vec<Cell>>,
    pub geometry: Vec<Vec<Cell>>,
    pub cutting: Vec<Vec<Cell>>,
}

/// Tools from the query rows, plus how many material presets they carry.
pub fn tools_from(rows: &Rows) -> (Vec<Tool>, usize) {
    let at = |r: &[Cell], i: usize| r.get(i).cloned().unwrap_or(Cell::Null(()));
    let materials: HashMap<String, String> = rows.materials.iter().map(|r| (at(r, 0).id(), at(r, 1).text())).collect();
    let mut cutting: HashMap<String, Vec<&Vec<Cell>>> = HashMap::new();
    for r in &rows.cutting {
        cutting.entry(at(r, 0).id()).or_default().push(r);
    }
    let (mut tools, mut presets) = (Vec::new(), 0);
    for g in &rows.geometry {
        let (gid, fmt, units, dia, angle, tree, flutes) = (at(g, 0).id(), at(g, 1).text(), at(g, 2).num(), at(g, 3).num(), at(g, 4).num(), at(g, 5).text(), at(g, 6).num());
        let k = if units == 1.0 { 1.0 } else { 1.0 / 25.4 }; // 1 = inches, else mm
        let dia = dia * k;
        let mut name = if tree.is_empty() { fmt.clone() } else { tree };
        if name.contains('{') {
            // "{Diameter|F} Down {Tool Type} ({Diameter})" -> readable name
            let kind = if angle > 0.0 { "V-Bit" } else { "End Mill" };
            let d = format!("{}", (dia * 1e4).round() / 1e4); // Qt QString::number(dia, 'g', 4)
            name = format!("{d} {} ({d} inch)", if fmt.to_lowercase().contains("down") { format!("Down {kind}") } else { kind.to_string() });
        }
        let mut t = Tool { name, kind: if angle > 0.0 { ToolType::VBit } else { ToolType::EndMill }, diameter: if dia > 0.0 { dia } else { 0.25 }, ..Default::default() };
        if angle > 0.0 {
            t.angle = angle;
        }
        if flutes >= 1.0 {
            t.flutes = flutes as i32;
        }
        // cutting data: material-less row = defaults, material rows = presets
        let mut have_base = false;
        for r in cutting.get(&gid).map(Vec::as_slice).unwrap_or_default() {
            let (mid, rate_units, feed, plunge, rpm, step, len_units) = (at(r, 1).id(), at(r, 2).num() as i64, at(r, 3).num(), at(r, 4).num(), at(r, 5).num() as i64, at(r, 6).num(), at(r, 7).num() as i64);
            let fk = if rate_units == 1 { 1.0 / 25.4 } else { 1.0 }; // 4 = in/min (the reference files), 1 = mm/min
            let (feed, plunge) = (feed * fk, plunge * fk);
            let step = step * if len_units == 1 { 1.0 } else { 1.0 / 25.4 };
            if feed <= 0.0 {
                continue;
            }
            let plunge = if plunge > 0.0 { plunge } else { feed / 2.0 };
            if mid.is_empty() || !have_base {
                t.feed_xy = feed;
                t.feed_z = plunge;
                if rpm > 0 {
                    t.rpm = rpm as i32;
                }
                if step > 0.0 {
                    t.stepdown = step;
                }
                have_base = true;
            }
            if let Some(m) = materials.get(&mid).filter(|_| !mid.is_empty()) {
                t.presets.push(FeedPreset { material: m.clone(), feed_xy: feed, feed_z: plunge, stepdown: if step > 0.0 { step } else { t.stepdown }, rpm: if rpm > 0 { rpm as i32 } else { t.rpm } });
                presets += 1;
            }
        }
        tools.push(t);
    }
    (tools, presets)
}

/// Tools of a .vtdb file's bytes (desktop), plus how many material presets they carry.
#[cfg(not(target_arch = "wasm32"))]
pub fn import(bytes: &[u8]) -> Result<(Vec<Tool>, usize), String> {
    // ponytail: SQLite needs a file; a copy in the temp folder (Qt copies too: read-only shares). A live -wal sibling
    // is not copied (bytes only): Vectric checkpoints it on close, so close VCarve before importing.
    let tmp = std::env::temp_dir().join(format!("gcarve-import-{}.vtdb", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("Cannot write {}: {e}", tmp.display()))?;
    let r = read(&tmp);
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", tmp.display()));
    }
    r.map(|rows| tools_from(&rows)).map_err(|e| format!("Cannot read the Vectric database ({e})"))
}

#[cfg(not(target_arch = "wasm32"))]
fn read(path: &std::path::Path) -> rusqlite::Result<Rows> {
    use rusqlite::types::Value;
    let db = rusqlite::Connection::open(path)?;
    let query = |sql: &str| -> rusqlite::Result<Vec<Vec<Cell>>> {
        let mut q = db.prepare(sql)?;
        let n = q.column_count();
        let rows = q.query_map([], |r| {
            (0..n)
                .map(|i| {
                    Ok(match r.get::<_, Value>(i)? {
                        Value::Integer(v) => Cell::Num(v as f64),
                        Value::Real(v) => Cell::Num(v),
                        Value::Text(s) => Cell::Text(s),
                        _ => Cell::Null(()),
                    })
                })
                .collect::<rusqlite::Result<Vec<Cell>>>()
        })?;
        rows.collect()
    };
    Ok(Rows { materials: query(Q_MATERIALS)?, geometry: query(Q_GEOMETRY)?, cutting: query(Q_CUTTING)? })
}

/// Browser: sql.js (SQLite compiled to wasm) from cdnjs, loaded once on the first .vtdb import
#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = r#"
let sqlReady = null;
function loadSql() {
  if (!sqlReady) sqlReady = new Promise((ok, bad) => {
    const s = document.createElement("script");
    s.src = "https://cdnjs.cloudflare.com/ajax/libs/sql.js/1.12.0/sql-wasm.js";
    s.onload = () => initSqlJs({ locateFile: f => "https://cdnjs.cloudflare.com/ajax/libs/sql.js/1.12.0/" + f }).then(ok, bad);
    s.onerror = () => { sqlReady = null; bad(new Error("could not load the SQLite reader (are you online?)")); };
    document.head.appendChild(s);
  });
  return sqlReady;
}
export async function gc_vtdb_rows(bytes, q1, q2, q3) {
  const SQL = await loadSql();
  const db = new SQL.Database(bytes);
  try {
    const all = q => { const r = db.exec(q); return r.length ? r[0].values.map(row => row.map(v => v instanceof Uint8Array ? null : v)) : []; };
    return JSON.stringify({ materials: all(q1), geometry: all(q2), cutting: all(q3) });
  } finally { db.close(); }
}
"#)]
    extern "C" {
        #[wasm_bindgen(catch)]
        async fn gc_vtdb_rows(bytes: &[u8], q1: &str, q2: &str, q3: &str) -> Result<JsValue, JsValue>;
    }

    pub type Done = Result<(Vec<gcarve_core::document::Tool>, usize), String>;
    static RESULT: std::sync::Mutex<Option<Done>> = std::sync::Mutex::new(None);

    /// Starts reading; `poll` hands the result over once it is there
    pub fn start(bytes: Vec<u8>, ctx: eframe::egui::Context) {
        wasm_bindgen_futures::spawn_local(async move {
            let r = match gc_vtdb_rows(&bytes, super::Q_MATERIALS, super::Q_GEOMETRY, super::Q_CUTTING).await {
                Ok(v) => serde_json::from_str::<super::Rows>(&v.as_string().unwrap_or_default()).map(|rows| super::tools_from(&rows)).map_err(|e| e.to_string()),
                Err(e) => Err(e.as_string().or_else(|| js_sys::Error::from(e).message().as_string()).unwrap_or_else(|| "not a Vectric tool database".into())),
            };
            *RESULT.lock().unwrap() = Some(r.map_err(|e| format!("Cannot read the Vectric database ({e})")));
            ctx.request_repaint();
        });
    }

    pub fn poll() -> Option<Done> {
        RESULT.lock().unwrap().take()
    }
}
#[cfg(target_arch = "wasm32")]
pub use web::{poll, start};

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn sample_database_imports_like_qt() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../samples/vectric-tools.vtdb")).unwrap();
        let (tools, presets) = super::import(&bytes).unwrap();
        assert!(tools.len() >= 20 && presets > 0);
        let q = tools.iter().find(|t| t.name == "1/4 Down End Mill (0.25 inch)").expect("1/4 down end mill");
        assert!((q.diameter - 0.25).abs() < 1e-9 && (q.feed_xy - 432.0).abs() < 1e-6 && (q.feed_z - 216.0).abs() < 1e-6 && q.rpm == 18000);
        assert!(tools.iter().any(|t| t.kind == gcarve_core::document::ToolType::VBit && t.angle > 0.0));
        assert!(tools.iter().any(|t| t.presets.iter().any(|p| p.material == "Plywood")));
        assert!(super::import(b"not a database").is_err());
    }
}
