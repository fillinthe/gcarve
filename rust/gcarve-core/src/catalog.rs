//! Machine and bit databases (data/machines/*.json, data/bits/*.json, data/bits/chipload.json; researched, every
//! record sourced). Parsing + the two conversions the app needs: a machine record -> `MachineLimits` for the board,
//! a bit record -> a library `Tool` carrying the maker's chip loads. Strings in (the app embeds the files).

use crate::document::{MachineLimits, Tool, ToolType};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// The research files use null for "not published" anywhere, also for whole blocks and strings.
fn nullable<'de, D: serde::Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Xyz {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct XyZ {
    pub xy: Option<f64>,
    pub z: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Spindle {
    #[serde(rename = "type")]
    #[serde(deserialize_with = "nullable")]
    pub kind: String,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    pub power_kw: Option<f64>,
    pub power_hp: Option<f64>,
    pub rpm_min: Option<f64>,
    pub rpm_max: Option<f64>,
    pub collet: Option<String>,
    pub default: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Controller {
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    pub gcarve_post: Option<String>,
    pub default: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ToolChange {
    #[serde(deserialize_with = "nullable")]
    pub kind: String,
    pub slots: Option<i32>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Machine {
    #[serde(deserialize_with = "nullable")]
    pub id: String,
    #[serde(deserialize_with = "nullable")]
    pub maker: String,
    #[serde(deserialize_with = "nullable")]
    pub model: String,
    pub variant: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub class: String,
    #[serde(deserialize_with = "nullable")]
    pub work_area_in: Xyz,
    #[serde(deserialize_with = "nullable")]
    pub spindles: Vec<Spindle>,
    #[serde(deserialize_with = "nullable")]
    pub max_feed_ipm: XyZ,
    #[serde(deserialize_with = "nullable")]
    pub rapid_ipm: XyZ,
    #[serde(deserialize_with = "nullable")]
    pub controllers: Vec<Controller>,
    #[serde(deserialize_with = "nullable")]
    pub tool_change: ToolChange,
    #[serde(deserialize_with = "nullable")]
    pub sources: Vec<String>,
}

impl Machine {
    pub fn title(&self) -> String {
        match &self.variant {
            Some(v) if !v.is_empty() => format!("{} {} {}", self.maker, self.model, v),
            _ => format!("{} {}", self.maker, self.model),
        }
    }
    /// default spindle (flagged default, else the first)
    pub fn default_spindle(&self) -> usize {
        self.spindles.iter().position(|s| s.default == Some(true)).unwrap_or(0)
    }
    /// post of the default controller
    pub fn post(&self) -> Option<&str> {
        let c = self.controllers.iter().find(|c| c.default == Some(true)).or(self.controllers.first())?;
        c.gcarve_post.as_deref()
    }
    pub fn limits(&self, spindle: usize) -> MachineLimits {
        let sp = self.spindles.get(spindle);
        // ponytail: when a maker publishes no max feed, the rapid rate is the ceiling
        let feed_xy = self.max_feed_ipm.xy.or(self.rapid_ipm.xy).unwrap_or(0.0);
        let feed_z = self.max_feed_ipm.z.or(self.rapid_ipm.z).unwrap_or(0.0);
        MachineLimits {
            id: self.id.clone(),
            name: self.title(),
            spindle: sp.map(|s| s.name.clone()).unwrap_or_default(),
            rpm_min: sp.and_then(|s| s.rpm_min).unwrap_or(0.0),
            rpm_max: sp.and_then(|s| s.rpm_max).unwrap_or(0.0),
            feed_xy,
            feed_z,
            travel_x: self.work_area_in.x.unwrap_or(0.0),
            travel_y: self.work_area_in.y.unwrap_or(0.0),
            travel_z: self.work_area_in.z.unwrap_or(0.0),
            tool_slots: if self.tool_change.kind.starts_with("atc") { self.tool_change.slots.unwrap_or(0) } else { 0 },
            class: self.class.clone(),
            rapid_xy: self.rapid_ipm.xy.unwrap_or(0.0),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Bit {
    #[serde(deserialize_with = "nullable")]
    pub id: String,
    #[serde(deserialize_with = "nullable")]
    pub maker: String,
    #[serde(deserialize_with = "nullable")]
    pub part_number: String,
    #[serde(deserialize_with = "nullable")]
    pub name: String,
    #[serde(deserialize_with = "nullable")]
    pub family: String,
    #[serde(deserialize_with = "nullable")]
    pub units: String,
    #[serde(deserialize_with = "nullable")]
    pub diameter: f64,
    pub shank: Option<f64>,
    pub cut_length: Option<f64>,
    #[serde(deserialize_with = "nullable")]
    pub flutes: i32,
    pub angle_deg: Option<f64>,
    pub corner_radius: Option<f64>,
    pub max_rpm: Option<f64>,
    pub rpm: Option<Vec<f64>>,
    pub chip_load: Option<BTreeMap<String, [f64; 2]>>,
    pub chipload_ref: Option<String>,
    pub gcarve_tool: Value,
}

impl Bit {
    fn inch(&self, v: f64) -> f64 {
        if self.units == "mm" { v / 25.4 } else { v }
    }
}

/// table id -> group -> [(diameter in, [lo, hi])] sorted by diameter
type Tables = BTreeMap<String, BTreeMap<String, Vec<(f64, [f64; 2])>>>;

#[derive(Default)]
pub struct Catalog {
    pub machines: Vec<Machine>,
    pub bits: Vec<Bit>,
    tables: Tables,
}

impl Catalog {
    /// `machine_files`: data/machines/<maker>.json texts; `bit_files`: data/bits/<maker>.json; `chipload`: data/bits/chipload.json.
    /// Files that fail to parse are skipped (a broken maker file must not take the rest down); their names come back.
    pub fn load(machine_files: &[(&str, &str)], bit_files: &[(&str, &str)], chipload: &str) -> (Catalog, Vec<String>) {
        let mut c = Catalog::default();
        let mut bad = Vec::new();
        #[derive(Deserialize)]
        struct MF {
            #[serde(default)]
            machines: Vec<Machine>,
        }
        #[derive(Deserialize)]
        struct BF {
            #[serde(default)]
            bits: Vec<Bit>,
        }
        for (name, text) in machine_files {
            match serde_json::from_str::<MF>(text) {
                Ok(f) => c.machines.extend(f.machines),
                Err(e) => bad.push(format!("{name}: {e}")),
            }
        }
        for (name, text) in bit_files {
            match serde_json::from_str::<BF>(text) {
                Ok(f) => c.bits.extend(f.bits),
                Err(e) => bad.push(format!("{name}: {e}")),
            }
        }
        if let Ok(v) = serde_json::from_str::<Value>(chipload) {
            for (id, t) in v.get("tables").and_then(Value::as_object).into_iter().flatten() {
                let mut groups = BTreeMap::new();
                for (g, rows) in t.get("rows").and_then(Value::as_object).into_iter().flatten() {
                    let mut r: Vec<(f64, [f64; 2])> = rows
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter_map(|(d, v)| Some((d.parse().ok()?, [v.get(0)?.as_f64()?, v.get(1)?.as_f64()?])))
                        .collect();
                    r.sort_by(|a, b| a.0.total_cmp(&b.0));
                    groups.insert(g.clone(), r);
                }
                c.tables.insert(id.clone(), groups);
            }
        } else {
            bad.push("chipload.json".into());
        }
        c.machines.sort_by_key(|m| m.title().to_ascii_lowercase());
        (c, bad)
    }

    /// The web app's copy (data/publish.py → site/public/data/catalog.json): {"machines": {file: object},
    /// "bits": {file: object}, "chipload": object}. Same parsing as the embedded files.
    pub fn from_published(text: &str) -> Result<Catalog, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let files = |k: &str| -> Vec<(String, String)> { v.get(k).and_then(Value::as_object).into_iter().flatten().map(|(n, f)| (n.clone(), f.to_string())).collect() };
        let (m, b) = (files("machines"), files("bits"));
        let chip = v.get("chipload").map(Value::to_string).unwrap_or_default();
        let (c, bad) = Catalog::load(&pairs(&m), &pairs(&b), &chip);
        if c.machines.is_empty() && c.bits.is_empty() {
            return Err(format!("empty catalog ({})", bad.join("; ")));
        }
        Ok(c)
    }

    pub fn machine(&self, id: &str) -> Option<&Machine> {
        self.machines.iter().find(|m| m.id == id)
    }

    /// Case-insensitive match on every word (maker, part number, name, family, size); at most `limit`.
    pub fn search_bits(&self, query: &str, limit: usize) -> Vec<&Bit> {
        let words: Vec<String> = query.split_whitespace().map(|w| w.to_ascii_lowercase()).collect();
        self.bits
            .iter()
            .filter(|b| {
                let hay = format!("{} {} {} {} {}", b.maker, b.part_number, b.name, b.family, b.id).to_ascii_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .take(limit)
            .collect()
    }

    pub fn search_machines(&self, query: &str, limit: usize) -> Vec<&Machine> {
        let words: Vec<String> = query.split_whitespace().map(|w| w.to_ascii_lowercase()).collect();
        self.machines
            .iter()
            .filter(|m| {
                let hay = format!("{} {}", m.title(), m.id).to_ascii_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .take(limit)
            .collect()
    }

    /// Maker chip loads for a bit, in/tooth by group: the bit's own, else its family table at the nearest diameter.
    pub fn chip_loads(&self, b: &Bit) -> BTreeMap<String, [f64; 2]> {
        if let Some(own) = &b.chip_load {
            if !own.is_empty() {
                return own.clone();
            }
        }
        let d = b.inch(b.diameter);
        let mut out = BTreeMap::new();
        if let Some(t) = b.chipload_ref.as_ref().and_then(|r| self.tables.get(r)) {
            for (g, rows) in t {
                if let Some(r) = rows.iter().min_by(|a, c| (a.0 - d).abs().total_cmp(&(c.0 - d).abs())) {
                    out.insert(g.clone(), r.1);
                }
            }
        }
        out
    }

    /// A library tool for this bit (inches), carrying the maker chip loads.
    pub fn bit_tool(&self, b: &Bit) -> Tool {
        let kind = match b.gcarve_tool.get("kind").and_then(Value::as_str) {
            Some("VBit") => ToolType::VBit,
            Some("BallNose") => ToolType::BallNose,
            // point-cutting round-overs only (radius = half the diameter); bearing-guided ones have another shape
            _ if b.family == "roundover" && b.corner_radius.is_some_and(|r| (-0.002..0.04).contains(&(b.inch(b.diameter) - 2.0 * b.inch(r)))) => ToolType::RoundOver,
            _ => ToolType::EndMill,
        };
        let diameter = b.gcarve_tool.get("diameter").and_then(Value::as_f64).unwrap_or_else(|| b.inch(b.diameter));
        let rpm = b.rpm.as_ref().and_then(|r| r.last().copied()).or(b.max_rpm).unwrap_or(18000.0).min(b.max_rpm.unwrap_or(f64::MAX));
        let name = b.gcarve_tool.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("{} {} {}", b.maker, b.part_number, b.name));
        Tool {
            name,
            kind,
            diameter,
            angle: b.gcarve_tool.get("angle").and_then(Value::as_f64).or(b.angle_deg).unwrap_or(90.0),
            stepdown: if kind == ToolType::VBit { 0.125 } else { (diameter * 0.5).max(0.01) },
            flutes: b.flutes.max(1),
            rpm: rpm as i32,
            chip_loads: self.chip_loads(b),
            catalog: b.id.clone(),
            radius: if kind == ToolType::RoundOver { b.corner_radius.map_or(0.0, |r| b.inch(r)) } else { 0.0 },
            cut_length: b.cut_length.map_or(0.0, |l| b.inch(l)),
            ..Default::default()
        }
    }
}

/// (name, text) pairs borrowed from owned strings, for `Catalog::load`.
fn pairs(f: &[(String, String)]) -> Vec<(&str, &str)> {
    f.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published web copy parses to the same records as data/ (stale = run `python3 -I data/publish.py`).
    #[test]
    fn published_matches_data() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let Ok(text) = std::fs::read_to_string(format!("{root}/site/public/data/catalog.json")) else { return };
        let read = |sub: &str, skip: &str| -> Vec<(String, String)> {
            let mut v: Vec<_> = std::fs::read_dir(format!("{root}/data/{sub}")).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json") && !p.ends_with(skip)).map(|p| (p.display().to_string(), std::fs::read_to_string(&p).unwrap())).collect();
            v.sort();
            v
        };
        let (m, b) = (read("machines", "spindles.json"), read("bits", "chipload.json"));
        let chip = std::fs::read_to_string(format!("{root}/data/bits/chipload.json")).unwrap();
        let (local, _) = Catalog::load(&pairs(&m), &pairs(&b), &chip);
        let web = Catalog::from_published(&text).unwrap();
        assert_eq!((web.machines.len(), web.bits.len()), (local.machines.len(), local.bits.len()), "site/public/data/catalog.json is stale: run python3 -I data/publish.py");
        assert_eq!(web.tables.len(), local.tables.len());
    }

    #[test]
    fn machine_limits_and_bit_tool() {
        let machines = r#"{"machines":[{"id":"m1","maker":"Acme","model":"Router","variant":"4x8","work_area_in":{"x":49,"y":97,"z":6},
            "spindles":[{"type":"router","name":"R","rpm_min":10000,"rpm_max":30000},{"type":"spindle","name":"S","rpm_min":6000,"rpm_max":24000,"default":true}],
            "max_feed_ipm":{"xy":600,"z":200},"controllers":[{"name":"GRBL","gcarve_post":"grbl","default":true}],"tool_change":{"kind":"atc-linear","slots":8}}]}"#;
        let bits = r#"{"bits":[{"id":"x-1","maker":"X","part_number":"P1","name":"Up 1/4","family":"end-mill-up","units":"in","diameter":0.25,"flutes":2,
            "chipload_ref":"t/up","gcarve_tool":{"kind":"EndMill","diameter":0.25}},
            {"id":"x-2","maker":"X","part_number":"P2","name":"V 6mm","family":"v-bit","units":"mm","diameter":6,"flutes":2,"angle_deg":60,"gcarve_tool":{"kind":"VBit"}}]}"#;
        let cl = r#"{"tables":{"t/up":{"rows":{"mdf":{"0.125":[0.003,0.005],"0.25":[0.008,0.010]}}}}}"#;
        let (c, bad) = Catalog::load(&[("m", machines)], &[("b", bits)], cl);
        assert!(bad.is_empty(), "{bad:?}");
        let m = c.machine("m1").unwrap();
        assert_eq!(m.default_spindle(), 1);
        assert_eq!(m.post(), Some("grbl"));
        let l = m.limits(m.default_spindle());
        assert_eq!((l.rpm_min, l.rpm_max, l.feed_xy, l.feed_z, l.travel_x, l.tool_slots), (6000.0, 24000.0, 600.0, 200.0, 49.0, 8));
        assert_eq!(l.name, "Acme Router 4x8");
        let t = c.bit_tool(c.search_bits("x p1", 5)[0]);
        assert_eq!(t.chip_loads["mdf"], [0.008, 0.010]);
        assert_eq!(t.catalog, "x-1");
        let v = c.bit_tool(c.search_bits("v 6mm", 5)[0]);
        assert_eq!(v.kind, ToolType::VBit);
        let kind = |cr: f64| c.bit_tool(&serde_json::from_value(serde_json::json!({"family": "roundover", "diameter": 0.5, "corner_radius": cr})).unwrap()).kind;
        assert_eq!((kind(0.25), kind(0.24), kind(0.1875)), (ToolType::RoundOver, ToolType::RoundOver, ToolType::EndMill), "point-cutting (also with a small flat tip) vs bearing round-over");
        assert!((v.diameter - 6.0 / 25.4).abs() < 1e-12 && v.angle == 60.0);
    }

    /// Every file in data/ parses (the app embeds them).
    #[test]
    fn real_databases_load() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let read = |dir: &str| -> Vec<(String, String)> {
            let mut v: Vec<_> = std::fs::read_dir(root.join(dir)).unwrap().flatten().map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json") && !p.ends_with("chipload.json") && !p.ends_with("spindles.json"))
                .map(|p| (p.display().to_string(), std::fs::read_to_string(&p).unwrap())).collect();
            v.sort();
            v
        };
        let (m, b) = (read("machines"), read("bits"));
        fn refs(v: &[(String, String)]) -> Vec<(&str, &str)> {
            v.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
        }
        let cl = std::fs::read_to_string(root.join("bits/chipload.json")).unwrap();
        let (c, bad) = Catalog::load(&refs(&m), &refs(&b), &cl);
        assert!(bad.is_empty(), "{bad:?}");
        assert!(c.machines.len() > 300 && c.bits.len() > 4000, "{} {}", c.machines.len(), c.bits.len());
        let p = c.machine("camaster-panther-pt508").expect("the user's Panther");
        assert_eq!(p.post(), Some("wincnc-single"));
        // most bits that name a family table resolve to maker chip loads
        let with = c.bits.iter().filter(|b| b.chipload_ref.is_some()).filter(|b| !c.chip_loads(b).is_empty()).count();
        assert!(with * 10 > c.bits.iter().filter(|b| b.chipload_ref.is_some()).count() * 9, "{with}");
    }
}
