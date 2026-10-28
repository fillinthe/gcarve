//! 3MF (3D printing's model format: Bambu Studio, PrusaSlicer, Cura, Fusion, Windows 3D Builder) read as triangles
//! for a 3D model. A 3MF is a zip; the model is XML: objects hold a mesh (vertices + triangles) or components (other
//! objects, possibly in other .model files of the zip, each with a transform); the build lists what is placed.
//!
//! ponytail: geometry only (colours, materials, slicer settings, beam lattices ignored), zip entries stored or
//! deflated (all 3MF writers use those), no zip64 (models over 4 GB).

use std::collections::HashMap;

use crate::relief::{Pt3, Triangle};

const MAX_ENTRY: usize = 512 << 20; // inflated bytes per zip entry
const MAX_UNPACKED: usize = 1 << 30; // inflated bytes per zip (entries may share one local header: each is read once)
const MAX_TRIANGLES: usize = 5_000_000; // placed triangles (components listing themselves fan out)
const MAX_DEPTH: usize = 8; // components inside components

fn u16_at(b: &[u8], k: usize) -> usize {
    b.get(k..k + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]) as usize)
}
fn u32_at(b: &[u8], k: usize) -> usize {
    b.get(k..k + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// The zip's entries (name → bytes) whose names end in `.model`.
fn model_entries(zip: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    zip_entries(zip, &[".model"]).map_err(|e| if e == NO_ZIP { "not a 3MF file (no zip directory)".into() } else { e })
}

const NO_ZIP: &str = "not a zip file";

/// A zip's entries (name → bytes, names starting with "/") whose names end in one of `ends` (lower case); also used
/// for fonts that come zipped.
pub fn zip_entries(zip: &[u8], ends: &[&str]) -> Result<HashMap<String, Vec<u8>>, String> {
    let eocd = (0..zip.len().saturating_sub(21)).rev().take(70_000).find(|&k| zip[k..].starts_with(b"PK\x05\x06")).ok_or(NO_ZIP)?;
    let (count, mut at) = (u16_at(zip, eocd + 10), u32_at(zip, eocd + 16));
    let mut out = HashMap::new();
    let (mut seen, mut total) = (std::collections::HashSet::new(), 0usize);
    for _ in 0..count {
        if !zip.get(at..).is_some_and(|s| s.starts_with(b"PK\x01\x02")) {
            break;
        }
        let (method, size, nlen) = (u16_at(zip, at + 10), u32_at(zip, at + 20), u16_at(zip, at + 28));
        let (extra, comment, local) = (u16_at(zip, at + 30), u16_at(zip, at + 32), u32_at(zip, at + 42));
        let name = String::from_utf8_lossy(zip.get(at + 46..at + 46 + nlen).unwrap_or_default()).into_owned();
        at += 46 + nlen + extra + comment;
        let lower = name.to_ascii_lowercase();
        if !ends.iter().any(|e| lower.ends_with(e)) || !seen.insert(local) {
            continue;
        }
        let data_at = local + 30 + u16_at(zip, local + 26) + u16_at(zip, local + 28);
        let raw = zip.get(data_at..data_at.saturating_add(size)).ok_or("the zip file is cut short")?;
        let data = match method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, MAX_ENTRY).map_err(|_| format!("{name}: damaged"))?,
            m => return Err(format!("{name}: zip method {m} not supported")),
        };
        total += data.len();
        if total > MAX_UNPACKED {
            return Err("the zip file unpacks to more than 1 GB".into());
        }
        out.insert(format!("/{}", name.trim_start_matches('/')), data);
    }
    Ok(out)
}

/// attribute `key` of a tag's text (`<vertex x="1" .../>`), by exact name (`p:path`, `objectid`)
fn attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(k) = tag[from..].find(key) {
        let i = from + k;
        let before = tag[..i].chars().last().is_some_and(|c| c.is_ascii_whitespace());
        let rest = tag[i + key.len()..].trim_start();
        if before && rest.starts_with('=') {
            let v = rest[1..].trim_start();
            let q = v.chars().next()?;
            return v[1..].find(q).map(|e| &v[1..1 + e]);
        }
        from = i + key.len();
    }
    None
}

/// 3MF transform "m00 m01 m02 m10 m11 m12 m20 m21 m22 m30 m31 m32" (row vector × matrix); identity when absent
type Xf = [f64; 12];
const IDENTITY: Xf = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
fn xf(tag: &str) -> Xf {
    let v: Vec<f64> = attr(tag, "transform").map(|t| t.split_ascii_whitespace().filter_map(|n| n.parse().ok()).collect()).unwrap_or_default();
    v.try_into().unwrap_or(IDENTITY)
}
fn apply(m: &Xf, p: Pt3) -> Pt3 {
    Pt3 { x: p.x * m[0] + p.y * m[3] + p.z * m[6] + m[9], y: p.x * m[1] + p.y * m[4] + p.z * m[7] + m[10], z: p.x * m[2] + p.y * m[5] + p.z * m[8] + m[11] }
}
/// `a` then `b`
fn then(a: &Xf, b: &Xf) -> Xf {
    let mut r = [0.0; 12];
    for row in 0..4 {
        for col in 0..3 {
            r[row * 3 + col] = (0..3).map(|k| a[row * 3 + k] * b[k * 3 + col]).sum::<f64>() + if row == 3 { b[9 + col] } else { 0.0 };
        }
    }
    r
}

enum Object {
    Mesh(Vec<Pt3>, Vec<[usize; 3]>),
    /// (file, object id, transform)
    Parts(Vec<(String, String, Xf)>),
}

/// every `<tag ...>` start tag in `s` (attributes text)
fn tags<'a>(s: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    s.match_indices(open.as_str())
        .filter_map(|(i, _)| {
            let rest = &s[i + open.len()..];
            rest.starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/').then(|| &rest[..rest.find('>').unwrap_or(rest.len())])
        })
        .collect()
}

fn objects(file: &str, xml: &str) -> HashMap<String, Object> {
    let mut out = HashMap::new();
    let starts: Vec<usize> = xml.match_indices("<object").map(|(i, _)| i).collect();
    for (k, &i) in starts.iter().enumerate() {
        let lim = starts.get(k + 1).copied().unwrap_or(xml.len()); // an unclosed object ends at the next one
        let body = &xml[i..xml[i..lim].find("</object>").map_or(lim, |e| i + e)];
        let Some(id) = attr(body, "id") else { continue };
        let obj = if body.contains("<mesh") {
            let num = |t: &str, k: &str| attr(t, k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            let verts: Vec<Pt3> = tags(body, "vertex").into_iter().map(|t| Pt3 { x: num(t, "x"), y: num(t, "y"), z: num(t, "z") }).collect();
            let ix = |t: &str, k: &str| attr(t, k).and_then(|v| v.parse::<usize>().ok()).filter(|&v| v < verts.len());
            let tris = tags(body, "triangle").into_iter().filter_map(|t| Some([ix(t, "v1")?, ix(t, "v2")?, ix(t, "v3")?])).collect();
            Object::Mesh(verts, tris)
        } else {
            Object::Parts(tags(body, "component").into_iter().filter_map(|t| Some((attr(t, "p:path").map_or(file.to_string(), str::to_string), attr(t, "objectid")?.to_string(), xf(t)))).collect())
        };
        out.insert(id.to_string(), obj);
    }
    out
}

fn place(all: &HashMap<String, HashMap<String, Object>>, file: &str, id: &str, m: &Xf, depth: usize, out: &mut Vec<Triangle>) {
    if out.len() > MAX_TRIANGLES {
        return;
    }
    match all.get(file).and_then(|o| o.get(id)) {
        Some(Object::Mesh(v, t)) => out.extend(t.iter().map(|&[a, b, c]| Triangle { a: apply(m, v[a]), b: apply(m, v[b]), c: apply(m, v[c]) })),
        Some(Object::Parts(parts)) if depth < MAX_DEPTH => {
            for (f, id, x) in parts {
                place(all, f, id, &then(x, m), depth + 1, out);
            }
        }
        _ => {}
    }
}

/// The triangles of every build item of a 3MF file, placed as the file places them.
pub fn load_3mf(zip: &[u8]) -> Result<Vec<Triangle>, String> {
    let files = model_entries(zip)?;
    let main = files.keys().find(|k| k.eq_ignore_ascii_case("/3D/3dmodel.model")).or_else(|| files.keys().next()).ok_or("no model in the 3MF file")?.clone();
    let all: HashMap<String, HashMap<String, Object>> = files.iter().map(|(name, data)| (name.clone(), objects(name, &String::from_utf8_lossy(data)))).collect();
    let xml = String::from_utf8_lossy(&files[&main]);
    let build = &xml[xml.find("<build").unwrap_or(xml.len())..];
    let mut out = Vec::new();
    for item in tags(build, "item") {
        if let Some(id) = attr(item, "objectid") {
            place(&all, attr(item, "p:path").unwrap_or(&main), id, &xf(item), 0, &mut out);
        }
    }
    if out.is_empty() {
        return Err("no triangles in the 3MF file".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a stored zip of (name, data) entries
    fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let (mut out, mut dir) = (Vec::new(), Vec::new());
        for (name, data) in entries {
            let local = out.len() as u32;
            let head = |sig: &[u8], central: bool| {
                let mut h = sig.to_vec();
                h.extend(if central { &[20u8, 0, 20, 0][..] } else { &[20u8, 0][..] });
                h.extend([0u8; 8]); // flags, method 0, time, date
                h.extend([0u8; 4]); // crc (not checked)
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes());
                h.extend([0u8; 2]);
                if central {
                    h.extend([0u8; 10]); // comment length, disk, attributes
                    h.extend(local.to_le_bytes());
                }
                h.extend(name.bytes());
                h
            };
            out.extend(head(b"PK\x03\x04", false));
            out.extend(data.bytes());
            dir.extend(head(b"PK\x01\x02", true));
        }
        let at = out.len() as u32;
        out.extend(&dir);
        out.extend(b"PK\x05\x06\0\0\0\0");
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((dir.len() as u32).to_le_bytes());
        out.extend(at.to_le_bytes());
        out.extend([0u8; 2]);
        out
    }

    #[test]
    fn components_in_other_files_are_placed_through_every_transform() {
        // like Bambu Studio: the main model has a component pointing into 3D/Objects, moved by the build item
        let part = r#"<model><resources><object id="1" type="model"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="0" y="10" z="5"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources></model>"#;
        let main = r#"<model unit="millimeter"><resources><object id="2"><components><component p:path="/3D/Objects/object_1.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 0 0 1"/></components></object></resources><build><item objectid="2" transform="0 1 0 -1 0 0 0 0 1 100 0 0"/></build></model>"#;
        let t = load_3mf(&zip(&[("[Content_Types].xml", "<Types/>"), ("3D/3dmodel.model", main), ("3D/Objects/object_1.model", part)])).unwrap();
        assert_eq!(t.len(), 1);
        // (10, 0, 0): component lifts z by 1, the item turns 90° (x → y) and moves x by 100
        let b = t[0].b;
        assert!((b.x - 100.0).abs() < 1e-9 && (b.y - 10.0).abs() < 1e-9 && (b.z - 1.0).abs() < 1e-9, "{b:?}");
        assert!(load_3mf(b"PK\x03\x04nonsense").is_err());
    }
}

/// real files: `GCARVE_3MF=file.3mf cargo test -p gcarve-core --lib threemf_file -- --ignored --nocapture`
#[test]
#[ignore]
fn threemf_file() {
    let path = std::env::var("GCARVE_3MF").expect("GCARVE_3MF");
    let t = load_3mf(&std::fs::read(&path).unwrap()).unwrap();
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in t.iter().flat_map(|t| [t.a, t.b, t.c]) {
        for (k, v) in [p.x, p.y, p.z].into_iter().enumerate() {
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    println!("{path}: {} triangles, {:.1} × {:.1} × {:.1}", t.len(), hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
}
