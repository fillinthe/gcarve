//! VCarve project (.crv) import, Rust only. Worked out from the files themselves (VCarve Pro 12.0–12.5): a .crv is
//! an OLE compound file whose streams are MFC CArchive data.
//! - `VectorData/MaterialSize`: u32 version, 2 bytes (the second 1 = inches), then the board box min x, y, z and
//!   max x, y, z as f64 (top Z zero = max z 0).
//! - `VectorData/2dDataV2`: layers (vcCadLayer: u32 4, GUID, CString name, ...) each followed by its objects
//!   (vcCadContour / vcCadPolyline / groups: u32 version, own GUID, layer GUID, ...), whose outlines are vdContour
//!   objects: u32 7, i32 (-1, 0 or 1), f64 tolerance, 10 bytes, u32 span count, spans = u8, u32 2, u32 kind (1 line,
//!   0 arc with a bulge after, 6 cubic with two control points after), start x y z, end x y z, u32 1.
//!   The layer and contour records are found by those signatures rather than by walking every MFC class (each
//!   object's Serialize differs and is undocumented); every span is checked, so a wrong match is dropped.
//! - Toolpaths (`Toolpaths/ToolpathData`) are not read yet.
//!
//! Checked against the preview picture inside each file (`PreviewData/Preview2D_GIF`) and all 305 projects on the
//! owner's NAS (`crv_survey` test).

use std::collections::HashMap;

use crate::document::{Document, Material};
use crate::geom::{dist, pt, Builder, Contour, Pt};

/// The streams of an OLE compound file by path ("VectorData/MaterialSize"). Damaged files give an error, never a
/// panic or an endless loop (every chain is bounded by the number of sectors).
pub fn cfb_streams(b: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    const MAGIC: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    let bad = || "This .crv file is damaged (or isn't a VCarve project).".to_string();
    if b.len() < 512 || b[..8] != MAGIC {
        return Err(bad());
    }
    let u16_at = |p: usize| b.get(p..p + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize);
    let u32_at = |p: usize| b.get(p..p + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize);
    let shift = u16_at(0x1e).ok_or_else(bad)?;
    if !(9..=16).contains(&shift) {
        return Err(bad());
    }
    let ss = 1usize << shift;
    let nsec = b.len() / ss; // sectors that can exist
    let sector = |n: usize| -> Option<&[u8]> { b.get((n + 1) * ss..(n + 2) * ss) };
    // FAT sector numbers: 109 in the header, then the DIFAT chain
    let mut fat_secs: Vec<usize> = (0..109).filter_map(|i| u32_at(0x4c + 4 * i)).filter(|&s| s < nsec).collect();
    let mut d = u32_at(0x44).ok_or_else(bad)?;
    let mut guard = 0;
    while d < nsec && guard <= nsec {
        let s = sector(d).ok_or_else(bad)?;
        let per = ss / 4 - 1;
        fat_secs.extend((0..per).map(|i| u32::from_le_bytes(s[4 * i..4 * i + 4].try_into().unwrap()) as usize).filter(|&x| x < nsec));
        d = u32::from_le_bytes(s[ss - 4..].try_into().unwrap()) as usize;
        guard += 1;
    }
    let mut fat: Vec<usize> = Vec::new();
    for &f in &fat_secs {
        let s = sector(f).ok_or_else(bad)?;
        fat.extend(s.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c) as usize));
    }
    let chain = |start: usize, next: &dyn Fn(usize) -> Option<usize>, limit: usize| -> Vec<usize> {
        let mut out = Vec::new();
        let mut s = start;
        while s < limit && out.len() <= limit {
            out.push(s);
            match next(s) {
                Some(n) => s = n,
                None => break,
            }
        }
        out
    };
    let fat_next = |s: usize| fat.get(s).copied();
    let read_chain = |start: usize| -> Vec<u8> { chain(start, &fat_next, nsec).into_iter().filter_map(sector).flatten().copied().collect() };
    let dir = read_chain(u32_at(0x30).ok_or_else(bad)?);
    let entry = |i: usize| dir.get(i * 128..(i + 1) * 128);
    let root = entry(0).ok_or_else(bad)?;
    let le32 = |e: &[u8], p: usize| u32::from_le_bytes(e[p..p + 4].try_into().unwrap()) as usize;
    let ministream = read_chain(le32(root, 0x74));
    let minifat: Vec<usize> = read_chain(u32_at(0x3c).ok_or_else(bad)?).as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c) as usize).collect();
    let cutoff = u32_at(0x38).ok_or_else(bad)?;
    let nmini = ministream.len() / 64;
    let mini_next = |s: usize| minifat.get(s).copied();
    let read = |e: &[u8]| -> Vec<u8> {
        let size = (u64::from_le_bytes(e[0x78..0x80].try_into().unwrap()) as usize).min(b.len());
        let start = le32(e, 0x74);
        let mut v: Vec<u8> = if size < cutoff {
            chain(start, &mini_next, nmini).into_iter().filter_map(|s| ministream.get(s * 64..s * 64 + 64)).flatten().copied().collect()
        } else {
            read_chain(start)
        };
        v.truncate(size);
        v
    };
    // walk the directory tree: children of a storage are a binary tree through left / right
    let mut out = HashMap::new();
    let mut stack: Vec<(usize, String)> = vec![(le32(root, 0x4c), String::new())];
    let mut seen = vec![false; dir.len() / 128];
    while let Some((i, prefix)) = stack.pop() {
        let Some(e) = entry(i) else { continue };
        if std::mem::replace(&mut seen[i], true) {
            continue; // a loop in a damaged file
        }
        let n = (u16::from_le_bytes([e[0x40], e[0x41]]) as usize).clamp(2, 64) / 2 - 1;
        let name = String::from_utf16_lossy(&e[..n * 2].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect::<Vec<_>>());
        stack.push((le32(e, 0x44), prefix.clone()));
        stack.push((le32(e, 0x48), prefix.clone()));
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        match e[0x42] {
            1 => stack.push((le32(e, 0x4c), path)),
            2 => {
                out.insert(path, read(e));
            }
            _ => {}
        }
    }
    Ok(out)
}

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl Rd<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.d.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
    }
    fn f64(&mut self) -> Option<f64> {
        self.take(8).map(|s| f64::from_le_bytes(s.try_into().unwrap())).filter(|v| v.is_finite() && v.abs() < 1e7)
    }
    fn xy(&mut self) -> Option<Pt> {
        Some(pt(self.f64()?, self.f64()?))
    }
    /// MFC CString, Unicode: ff fe ff, then the length in u8 (0xff = u16 follows), then UTF-16
    fn cstring(&mut self) -> Option<String> {
        if self.take(3)? != [0xff, 0xfe, 0xff] {
            return None;
        }
        let mut n = self.u8()? as usize;
        if n == 0xff {
            n = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
        }
        let s = self.take(2 * n)?;
        Some(String::from_utf16_lossy(&s.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect::<Vec<_>>()))
    }
}

fn find_all<'a>(d: &'a [u8], pat: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    d.windows(pat.len()).enumerate().filter(move |(_, w)| *w == pat).map(|(i, _)| i)
}

/// One vdContour starting at its version word (`at`); None when the bytes there aren't one.
fn vd_contour(d: &[u8], at: usize) -> Option<Contour> {
    let mut r = Rd { d, p: at + 8 };
    let tol = r.f64()?;
    if !(1e-7..0.1).contains(&tol) {
        return None;
    }
    r.take(10)?;
    let n = r.u32()? as usize;
    if n == 0 || n > (d.len() - r.p) / 40 {
        return None;
    }
    let mut b = Builder::default();
    let mut first = None;
    for _ in 0..n {
        r.u8()?;
        if r.u32()? != 2 {
            return None;
        }
        let kind = r.u32()?;
        let p0 = r.xy()?;
        r.take(8)?;
        let p1 = r.xy()?;
        r.take(8)?;
        r.u32()?;
        if first.is_none() {
            first = Some(p0);
            b.start(p0);
        }
        match kind {
            1 => b.line(p1),
            0 => {
                let bulge = r.f64()?;
                if bulge.abs() < 1e-9 || dist(p0, p1) < 1e-12 {
                    b.line(p1);
                } else {
                    crate::dxf::bulge_arc(&mut b, p0, p1, bulge);
                }
            }
            6 => {
                let (c1, c2) = (r.xy()?, r.xy()?);
                b.cubic(c1, c2, p1);
            }
            _ => return None,
        }
    }
    let end = b.cur();
    if first.is_some_and(|f| dist(f, end) < 1e-6) {
        b.close();
    } else {
        b.c.closed = false;
    }
    Some(b.c)
}

/// A VCarve project as a new document: board size and thickness, layers and vectors. Toolpaths aren't read yet
/// (the note says so).
pub fn import_crv(bytes: &[u8]) -> Result<(Document, String), String> {
    from_streams(&cfb_streams(bytes)?)
}

/// board, offset added to drawing points, scale to inches
type Board = (Material, Pt, f64);

/// A board box (`VectorData/MaterialSize`, and each vcCadSheet after its name): u32 version, 2 bytes (the second
/// 0 = millimetres), min x y z, max x y z. Returns the board and what to add to drawing points, then the scale.
fn board(r: &mut Rd) -> Option<Board> {
    r.u32()?;
    r.u8()?;
    let scale = if r.u8()? == 0 { 1.0 / 25.4 } else { 1.0 }; // ponytail: no metric .crv seen yet; mm is VCarve's other unit
    let (x0, y0, z0, x1, y1, z1) = (r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?);
    if !(x1 > x0 && y1 > y0 && z1 > z0) {
        return None;
    }
    let mut m = Material::default();
    (m.w, m.h, m.t) = ((x1 - x0) * scale, (y1 - y0) * scale, (z1 - z0) * scale);
    m.z_bottom = z0.abs() < 1e-9;
    // VCarve keeps the drawing in job coordinates; GCarve draws from the board's corner and moves the zero instead
    m.origin = if (x0 + x1).abs() < 1e-6 && (y0 + y1).abs() < 1e-6 { 4 } else { 0 };
    Some((m, pt(-x0, -y0), scale))
}

fn from_streams(s: &HashMap<String, Vec<u8>>) -> Result<(Document, String), String> {
    let vec = s.get("VectorData/2dDataV2").ok_or("This VCarve file has no vector data (is it a .crv from VCarve or Aspire?).")?;
    let job = s.get("VectorData/MaterialSize").and_then(|m| board(&mut Rd { d: m, p: 0 }));
    // sheets (vcCadSheet): u32 3, "Vectric__Version" block whose id string is the sheet's, CString name, board box
    let mut sheets: Vec<(String, &[u8], Board)> = Vec::new();
    const SHEET: &[u8] = b"\x03\0\0\0\x10\0\0\0Vectric__Version\x01\0\0\0$\0\0\0";
    for i in find_all(vec, SHEET) {
        let id = &vec[(i + SHEET.len()).min(vec.len())..(i + SHEET.len() + 36).min(vec.len())];
        let mut r = Rd { d: vec, p: i + SHEET.len() + 36 };
        if let (Some(name), Some(b)) = (r.cstring(), job.clone()) {
            let b = board(&mut r).unwrap_or(b);
            if !sheets.iter().any(|q| q.1 == id) {
                sheets.push((name, id, b));
            }
        }
    }
    // sheet names as people number them: "2" before "10"
    let key = |n: &str| (n.trim_start_matches(|c: char| !c.is_ascii_digit()).split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse::<u64>().ok()).unwrap_or(u64::MAX), n.to_string());
    sheets.sort_by_key(|q| key(&q.0));
    let mut doc = Document::default();
    let fallback = job.unwrap_or((doc.material.clone(), pt(0.0, 0.0), 1.0));
    doc.sheets = if sheets.is_empty() { vec![fallback.0.clone()] } else { sheets.iter().map(|q| q.2 .0.clone()).collect() };
    doc.material = doc.sheets[0].clone();
    // layers: tag, u32 4, GUID, CString name
    let mut layers: HashMap<&[u8], String> = HashMap::new();
    for i in find_all(vec, &[4, 0, 0, 0]) {
        let mut r = Rd { d: vec, p: i + 20 };
        if let (Some(g), Some(name)) = (vec.get(i + 4..i + 20), r.cstring()) {
            layers.entry(g).or_insert(name);
        }
    }
    // objects: u32 version, own GUID, layer GUID; sorted, so each outline belongs to the nearest object before it
    let mut owners: Vec<(usize, &str)> = Vec::new();
    for (g, name) in &layers {
        for p in find_all(vec, g) {
            if p >= 20 && (1..64).contains(&u32::from_le_bytes(vec[p - 20..p - 16].try_into().unwrap())) {
                owners.push((p - 20, name));
            }
        }
    }
    owners.sort();
    let mut n = 0;
    for at in find_all(vec, &[7, 0, 0, 0]) {
        let Some(mut c) = vd_contour(vec, at) else { continue };
        let owner = owners.partition_point(|o| o.0 < at);
        let layer = if owner > 0 { owners[owner - 1].1 } else { "" };
        if layer == "Toolpath Previews" {
            continue; // VCarve's own drawing of its toolpaths
        }
        // its sheet: the id string just before the outline ("$" 0 0 0, 36 characters, u32 2, class tag)
        let before = &vec[at.saturating_sub(80)..at];
        let sheet = sheets.iter().position(|q| find_all(before, q.1).next().is_some()).unwrap_or(0);
        let (_, shift, scale) = sheets.get(sheet).map_or(&fallback, |q| &q.2);
        for node in &mut c.nodes {
            for q in [&mut node.p, &mut node.inh, &mut node.out] {
                *q = (*q + *shift) * *scale;
            }
        }
        let l = doc.layer_named(layer);
        let id = doc.add(c, Some(l));
        if let Some(sh) = doc.shapes.iter_mut().find(|s| s.id == id) {
            sh.sheet = sheet as i32;
        }
        n += 1;
    }
    if n == 0 {
        return Err("No vectors found in this VCarve file.".into());
    }
    // VCarve's first layer is usually "Layer 1": keep GCarve's default layer only when something is on it
    if doc.layers.len() > 1 && !doc.shapes.iter().any(|s| s.layer == 0) {
        doc.layers.remove(0);
        for s in &mut doc.shapes {
            s.layer -= 1;
        }
        doc.active_layer = 0;
    }
    let m = &doc.material;
    let mut note = format!("{n} vectors from VCarve, board {} × {} × {} in", num(m.w), num(m.h), num(m.t));
    if sheets.len() > 1 {
        note += &format!(", {} sheets ({})", sheets.len(), sheets.iter().map(|q| q.0.as_str()).collect::<Vec<_>>().join(", "));
    }
    note += ". Toolpaths aren't read from .crv files yet: add them in GCarve.";
    Ok((doc, note))
}

fn num(v: f64) -> String {
    format!("{v:.3}").trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(v: &[f64]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }
    fn span(kind: u32, p0: (f64, f64), p1: (f64, f64), extra: &[f64]) -> Vec<u8> {
        let mut b = vec![0u8];
        b.extend(2u32.to_le_bytes());
        b.extend(kind.to_le_bytes());
        b.extend(le(&[p0.0, p0.1, 0.0, p1.0, p1.1, 0.0]));
        b.extend(1u32.to_le_bytes());
        b.extend(le(extra));
        b
    }

    #[test]
    fn layers_board_and_line_arc_curve_spans() {
        let guid_l = [7u8; 16];
        let mut v = vec![2, 0, 0, 0, 5, 0, 0, 0, 1, 0, 0, 0, 0xff, 0xff, 1, 0, 10, 0];
        v.extend(b"vcCadLayer");
        v.extend([4, 0, 0, 0]);
        v.extend(guid_l);
        v.extend([0xff, 0xfe, 0xff, 4]);
        v.extend("Cuts".encode_utf16().flat_map(|c| c.to_le_bytes()));
        v.extend([0; 14]);
        // an object on that layer, then its outline: a square with one arc corner and one curved side
        v.extend([0x08, 0x80, 10, 0, 0, 0]);
        v.extend([9u8; 16]);
        v.extend(guid_l);
        v.extend([0; 20]);
        v.extend([7, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
        v.extend(le(&[0.0003937]));
        v.extend([0; 10]);
        v.extend(4u32.to_le_bytes());
        v.extend(span(1, (1.0, 1.0), (3.0, 1.0), &[]));
        v.extend(span(0, (3.0, 1.0), (4.0, 2.0), &[0.4142135623730951]));
        v.extend(span(6, (4.0, 2.0), (1.0, 3.0), &[4.0, 3.0, 2.0, 3.0]));
        v.extend(span(1, (1.0, 3.0), (1.0, 1.0), &[]));
        let mut m = vec![8, 0, 0, 0, 0, 1];
        m.extend(le(&[0.0, 0.0, -0.75, 48.0, 24.0, 0.0]));
        let s = HashMap::from([("VectorData/2dDataV2".to_string(), v), ("VectorData/MaterialSize".to_string(), m)]);
        let (doc, note) = from_streams(&s).unwrap();
        assert_eq!((doc.material.w, doc.material.h, doc.material.t, doc.material.z_bottom), (48.0, 24.0, 0.75, false));
        assert_eq!(doc.shapes.len(), 1, "{note}");
        assert_eq!(doc.layers[doc.shapes[0].layer as usize].name, "Cuts");
        let c = &doc.shapes[0].c;
        assert!(c.closed);
        let b = crate::geom::bounds(c);
        assert!((b.x1 - 4.0).abs() < 1e-3 && (b.y0 - 1.0).abs() < 1e-6 && (b.y1 - 3.0).abs() < 1e-6, "{b:?}");
        // the quarter arc (bulge tan 22.5°, counter-clockwise) bows out to the lower right of its chord, round (3, 2)
        let f = crate::geom::flatten(c, 0.001);
        let mid = f.iter().filter(|q| q.x > 3.01 && q.y < 1.99).fold(0.0f64, |m, q| m.max(crate::geom::dist(*q, pt(3.0, 2.0))));
        assert!((mid - 1.0).abs() < 1e-3, "arc radius {mid}");
    }

    /// One file (or one sheet: GCARVE_CRV_SHEET=index) as SVG to compare with VCarve's preview: `GCARVE_CRV=x.crv GCARVE_CRV_SVG=x.svg cargo test -p
    /// gcarve-core --lib crv_to_svg -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn crv_to_svg() {
        let (d, note) = import_crv(&std::fs::read(std::env::var("GCARVE_CRV").unwrap()).unwrap()).unwrap();
        println!("{note}");
        let sheet: Option<i32> = std::env::var("GCARVE_CRV_SHEET").ok().and_then(|v| v.parse().ok()); // one sheet only
        let items: Vec<_> = d.shapes.iter().filter(|s| sheet.is_none_or(|i| s.sheet == i)).map(|s| (&s.c, d.layers[s.layer as usize].name.as_str())).collect();
        std::fs::write(std::env::var("GCARVE_CRV_SVG").unwrap(), crate::svg::export_svg(&items, d.material.w, d.material.h)).unwrap();
    }

    /// Every .crv under GCARVE_CRV_DIR opens (`GCARVE_CRV_DIR=/Volumes/home/_Projects cargo test --release -p
    /// gcarve-core --lib crv_survey -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn crv_survey() {
        let dir = std::env::var("GCARVE_CRV_DIR").expect("GCARVE_CRV_DIR");
        let mut stack = vec![std::path::PathBuf::from(dir)];
        let (mut ok, mut bad, mut vectors) = (0, 0, 0);
        while let Some(p) = stack.pop() {
            for e in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                let q = e.path();
                if q.is_dir() {
                    stack.push(q);
                } else if q.extension().is_some_and(|x| x.eq_ignore_ascii_case("crv")) {
                    match import_crv(&std::fs::read(&q).unwrap()) {
                        Ok((d, _)) => {
                            ok += 1;
                            vectors += d.shapes.len();
                        }
                        Err(e) => {
                            bad += 1;
                            println!("{}: {e}", q.display());
                        }
                    }
                }
            }
        }
        println!("{ok} opened, {bad} failed, {vectors} vectors");
        assert_eq!(bad, 0);
    }
}
