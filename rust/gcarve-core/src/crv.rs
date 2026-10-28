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
//! - `Toolpaths/ToolpathData`: a tree of nodes (CString name followed by a "Vectric__Version" id block); a toolpath
//!   node holds its bit (`tool_at`), the objects it cuts (veEntityGroup: u32 2, u8 0, u32 n, n object GUIDs, the
//!   GUIDs written after each object's version word in 2dDataV2), then its settings as named utParameters (`params`:
//!   "EditingDialog" = which VCarve form made it, "_ppdCutDepth", ...), then its per-vector sub-toolpaths, each with a
//!   copy of the bit. Each node's child count is the u32 just before the next node, so the tree is known. Merged
//!   toolpaths are skipped (their parts are read); an array copy sets the Array grid of the toolpaths under it. A
//!   toolpath's group in the list is the folder / merge / array node it sits in.
//!
//! Checked against the preview picture inside each file (`PreviewData/Preview2D_GIF`) and all 305 projects on the
//! owner's NAS (`crv_survey` test).

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::document::{Document, Entry, Material, Op, OpType, Side, Tool, ToolType};
use crate::geom::{dist, flatten, pt, transformed, Affine, Builder, Contour, Pt};

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
    fat_secs.sort_unstable();
    fat_secs.dedup(); // DIFAT sectors may list the same FAT sector again and again: read each once
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

/// start, end, bulge (arcs), control points (curves)
type Span = (Pt, Pt, Option<f64>, Option<(Pt, Pt)>);

/// One vdContour starting at its version word (`at`) and the tabs VCarve placed on it; None when the bytes there
/// aren't one.
fn vd_contour(d: &[u8], at: usize) -> Option<(Contour, Vec<Pt>)> {
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
    let mut spans: Vec<Span> = Vec::new();
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
            1 => {
                b.line(p1);
                spans.push((p0, p1, None, None));
            }
            0 => {
                let bulge = r.f64()?;
                if bulge.abs() < 1e-9 || dist(p0, p1) < 1e-12 {
                    b.line(p1);
                    spans.push((p0, p1, None, None));
                } else {
                    crate::dxf::bulge_arc(&mut b, p0, p1, bulge);
                    spans.push((p0, p1, Some(bulge), None));
                }
            }
            6 => {
                let (c1, c2) = (r.xy()?, r.xy()?);
                b.cubic(c1, c2, p1);
                spans.push((p0, p1, None, Some((c1, c2))));
            }
            _ => return None,
        }
    }
    // tabs (vcToolpathTab): own GUID, i32 -1, 2 bytes, u32 count, then per tab: class tag, u32 1, u32 span, f64
    // position along it (0..1), u32 1
    let mut tabs = Vec::new();
    let mut t = Rd { d, p: r.p + 16 };
    if t.u32() == Some(u32::MAX) && t.take(2).is_some() {
        let count = t.u32().unwrap_or(0).min(256);
        for _ in 0..count {
            let Some(tag) = t.take(2).map(|g| [g[0], g[1]]) else { break };
            if tag == [0xff, 0xff] {
                if t.take(2).is_none() {
                    break;
                }
                let Some(len) = t.take(2).map(|l| u16::from_le_bytes([l[0], l[1]]) as usize) else { break };
                if t.take(len) != Some(b"vcToolpathTab".as_slice()) {
                    break;
                }
            } else if tag[1] & 0x80 == 0 {
                break;
            }
            let (Some(1), Some(k), Some(f), Some(_)) = (t.u32(), t.u32(), t.f64(), t.u32()) else { break };
            let Some(&(a, z, bulge, ctl)) = spans.get(k as usize) else { break };
            if !(0.0..=1.0).contains(&f) {
                break;
            }
            tabs.push(match (bulge, ctl) {
                (Some(bu), _) => {
                    // the same arc as dxf::bulge_arc
                    let th = 4.0 * bu.atan();
                    let c = dist(a, z);
                    let m = (a + z) * 0.5;
                    let nrm = pt(-(z.y - a.y) / c, (z.x - a.x) / c);
                    let ctr = m + nrm * (c / 2.0 / (th / 2.0).tan());
                    let ang = (a.y - ctr.y).atan2(a.x - ctr.x) + th * f;
                    ctr + pt(ang.cos(), ang.sin()) * dist(a, ctr)
                }
                (_, Some((c1, c2))) => {
                    let u = 1.0 - f;
                    a * (u * u * u) + c1 * (3.0 * u * u * f) + c2 * (3.0 * u * f * f) + z * (f * f * f)
                }
                _ => a + (z - a) * f,
            });
        }
    }
    let end = b.cur();
    if first.is_some_and(|f| dist(f, end) < 1e-6) {
        b.close();
    } else {
        b.c.closed = false;
    }
    Some((b.c, tabs))
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
    doc.sheets = if sheets.is_empty() { vec![fallback.0.clone()] } else { sheets.iter().map(|q| Material { name: q.0.clone(), ..q.2 .0.clone() }).collect() };
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
    let mut owners: Vec<(usize, &str, &[u8])> = Vec::new();
    for (g, name) in &layers {
        for p in find_all(vec, g) {
            if p >= 20 && (1..64).contains(&u32::from_le_bytes(vec[p - 20..p - 16].try_into().unwrap())) {
                owners.push((p - 20, name, &vec[p - 16..p]));
            }
        }
    }
    owners.sort();
    // object GUID -> its shapes (toolpaths name the objects they cut)
    let mut objs: HashMap<&[u8], Vec<i32>> = HashMap::new();
    let found: Vec<(usize, (Contour, Vec<Pt>))> = find_all(vec, &[7, 0, 0, 0]).filter_map(|at| vd_contour(vec, at).map(|c| (at, c))).collect();
    let outlines: Vec<(usize, Contour)> = found.iter().map(|(at, c)| (*at, c.0.clone())).collect();
    let tab_at: HashMap<usize, &Vec<Pt>> = found.iter().filter(|f| !f.1 .1.is_empty()).map(|(at, c)| (*at, &c.1)).collect();
    // shape id -> the tabs placed on it in VCarve (document inches)
    let mut tabs: HashMap<i32, Vec<Pt>> = HashMap::new();
    let text = text_layout(vec, &outlines);
    let mut groups: HashMap<usize, i32> = HashMap::new();
    let mut n = 0;
    for (at, mut c) in outlines.iter().map(|(a, c)| (*a, c.clone())) {
        let owner = owners.partition_point(|o| o.0 < at);
        let layer = if owner > 0 { owners[owner - 1].1 } else { "" };
        if layer == "Toolpath Previews" || text.skip.contains(&at) {
            continue; // VCarve's own drawing of its toolpaths, the curve a text follows, unread letters
        }
        let letter = text.placed.get(&at);
        if let Some((m, _)) = letter {
            c = transformed(&c, m);
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
        if let Some(ts) = tab_at.get(&at) {
            let m = letter.map_or(Affine::default(), |l| l.0);
            tabs.insert(id, ts.iter().map(|&q| (m.apply(q) + *shift) * *scale).collect());
        }
        let group = letter.map(|(_, block)| *groups.entry(*block).or_insert_with(|| doc.new_group_id()));
        if let Some(sh) = doc.shapes.iter_mut().find(|s| s.id == id) {
            sh.sheet = sheet as i32;
            sh.group = group.unwrap_or(sh.group); // each text block's letters move together
        }
        if owner > 0 {
            objs.entry(owners[owner - 1].2).or_default().push(id);
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
    if text.blocks > 0 {
        note += &format!(", {} text{} as letter outlines", text.blocks, if text.blocks == 1 { "" } else { "s" });
        if text.curved > 0 {
            note += " (text on a curve can sit up to about 1/32 in from where VCarve puts it)";
        }
    }
    if text.unread > 0 {
        note += &format!(". {} text{} not read: in VCarve, convert it to curves first", text.unread, if text.unread == 1 { "" } else { "s" });
    }
    let (made, copies, skipped) = s.get("Toolpaths/ToolpathData").map_or((0, 0, Vec::new()), |d| toolpaths(d, &objs, &tabs, fallback.2, &mut doc));
    note += &format!(". {made} toolpath{} read", if made == 1 { "" } else { "s" });
    if copies > 0 {
        note += &format!(" ({copies} as array copies)");
    }
    if !skipped.is_empty() {
        note += &format!("; not read: {}", skipped.join(", "));
    }
    note += ": check the bits and depths before cutting.";
    Ok((doc, note))
}

/// Where VCarve text letters go. A text object holds its settings (utParameters "_txtAL_Text", "_txtAL_height",
/// "_txtblkIsOnCurve", ...), then u32 1, its anchor (2 f64), a "Vectric__Version" id block, u32 1, the box the
/// laid-out letters fill (min x y, max x y) and a 3 × 3 matrix to the drawing (rows a b c / d e f / 0 0 1), then per
/// letter (txtChar: u32 4, u32 character, CString font spec) its outlines centred on x = 0 with the baseline at y = 0,
/// then ff ff ff ff 00 00 01 00 00 00, 5 bytes and f64s whose second is half the letter's advance; after the last
/// letter of text on a curve, the curve (a vdContour). VCarve lays the letters out itself when drawing: here lines
/// are 1.5 × the letter height apart and letters follow their advances (straight text matches VCarve's box to a
/// thousandth), along the curve raised by the offset distance for text on a curve, then the whole is centred on the
/// stored box.
struct TextLayout {
    /// outline position -> (letter placement, text object start)
    placed: HashMap<usize, (Affine, usize)>,
    /// outlines that are not drawing: the curves texts follow, letters of texts that couldn't be laid out
    skip: HashSet<usize>,
    blocks: usize,
    curved: usize,
    unread: usize,
}

fn text_layout(d: &[u8], outlines: &[(usize, Contour)]) -> TextLayout {
    let mut out = TextLayout { placed: HashMap::new(), skip: HashSet::new(), blocks: 0, curved: 0, unread: 0 };
    let ps = params(d);
    let starts: Vec<usize> = ps.iter().filter(|q| q.1 == "_txtAL_Version").map(|q| q.0).collect();
    // letters: u32 4, u32 character, CString font spec "n,0,family,,size,..."
    let chars: Vec<usize> = find_all(d, &[4, 0, 0, 0])
        .filter(|&i| d.get(i + 6..i + 11) == Some(&[0, 0, 0xff, 0xfe, 0xff]) && Rd { d, p: i + 8 }.cstring().is_some_and(|f| f.contains(",,")))
        .collect();
    const TRAIL: &[u8] = &[0xff, 0xff, 0xff, 0xff, 0, 0, 1, 0, 0, 0];
    for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(d.len());
        let mine: Vec<&(usize, String, Val, usize)> = ps.iter().filter(|q| (start..end).contains(&q.0)).collect();
        let num = |key: &str| mine.iter().find(|q| q.1 == key).and_then(|q| if let Val::N(v) = q.2 { Some(v) } else { None });
        let letters: Vec<usize> = chars.iter().copied().filter(|c| (start..end).contains(c)).collect();
        let in_letters = |at: usize| letters.first().is_some_and(|&f| at > f && at < end);
        // the box and matrix after the last setting
        let laid = (|| {
            let text = mine.iter().find_map(|q| if let (true, Val::S(t)) = (q.1 == "_txtAL_Text", &q.2) { Some(t.replace('\r', "")) } else { None })?;
            let h = num("_txtAL_height")?;
            let mut r = Rd { d, p: mine.iter().find(|q| q.1 == "_txtblkSpacingValue")?.3 };
            if r.u32()? != 1 {
                return None;
            }
            r.take(16)?;
            if r.take(20)? != b"\x10\0\0\0Vectric__Version" {
                return None;
            }
            r.u32()?;
            let n = r.u32()? as usize;
            r.take(n)?;
            r.u32()?;
            let (x0, y0, x1, y1) = (r.f64()?, r.f64()?, r.f64()?, r.f64()?);
            let m: Vec<f64> = (0..9).map(|_| r.f64()).collect::<Option<_>>()?;
            let to_drawing = Affine { a: m[0], c: m[1], e: m[2], b: m[3], d: m[4], f: m[5] };
            let lines: Vec<usize> = text.split('\n').map(|l| l.chars().count()).collect();
            if lines.iter().sum::<usize>() != letters.len() || letters.is_empty() {
                return None;
            }
            // each letter: its outlines and its advance (spaces have no outline: 0.3 × height)
            let mut cells = Vec::new();
            let mut last_trail = start;
            for (i, &c) in letters.iter().enumerate() {
                let next = letters.get(i + 1).copied().unwrap_or(end);
                let trail = find_all(&d[c..next], TRAIL).next().map(|t| c + t);
                let adv = trail.and_then(|t| Rd { d, p: t + 23 }.f64()).map_or(0.3 * h, |half| 2.0 * half);
                let stop = trail.unwrap_or(next);
                last_trail = trail.unwrap_or(last_trail).max(c);
                cells.push((adv, outlines.iter().filter(|o| o.0 > c && o.0 < stop).map(|o| o.0).collect::<Vec<_>>()));
            }
            let mut place: Vec<Affine> = Vec::new();
            let curve = num("_txtblkIsOnCurve") == Some(1.0);
            if curve {
                let base = outlines.iter().find(|o| o.0 > last_trail && o.0 < end)?;
                out.skip.insert(base.0);
                let off = num("_txtblkOffsetDistance").unwrap_or(0.0);
                let mut line = flatten(&base.1, 0.001);
                if num("_txtblkReverseCurve") == Some(1.0) {
                    line.reverse();
                }
                // the baseline: the curve raised by the offset distance, measured along its length
                let raised: Vec<Pt> = (0..line.len())
                    .map(|i| {
                        let (a, b) = (line[i.saturating_sub(1)], line[(i + 1).min(line.len() - 1)]);
                        let t = (b - a) * (1.0 / dist(a, b).max(1e-12));
                        line[i] + pt(-t.y, t.x) * off
                    })
                    .collect();
                let mut len = vec![0.0];
                for w in raised.windows(2) {
                    len.push(len[len.len() - 1] + dist(w[0], w[1]));
                }
                let total: f64 = cells.iter().map(|c| c.0).sum();
                let mut s = (len[len.len() - 1] - total) / 2.0;
                for (adv, _) in &cells {
                    let at = (s + adv / 2.0).clamp(0.0, len[len.len() - 1]);
                    let i = len.partition_point(|&l| l < at).clamp(1, len.len() - 1);
                    let (a, b) = (raised[i - 1], raised[i]);
                    let t = if len[i] > len[i - 1] { (at - len[i - 1]) / (len[i] - len[i - 1]) } else { 0.0 };
                    let p = a + (b - a) * t;
                    place.push(Affine::translate(p.x, p.y).then(&Affine::rotate((b.y - a.y).atan2(b.x - a.x))));
                    s += adv;
                }
                out.curved += 1;
            } else {
                let justify = num("_txtAL_text_justify").unwrap_or(2.0);
                let mut i = 0;
                for (li, &count) in lines.iter().enumerate() {
                    let row = &cells[i..i + count];
                    let total: f64 = row.iter().map(|c| c.0).sum();
                    // ponytail: 0 left, 1 right, 2 centre (only centred text seen); the box below fixes the position
                    let mut x = match justify as i32 {
                        0 => 0.0,
                        1 => -total,
                        _ => -total / 2.0,
                    };
                    for (adv, _) in row {
                        place.push(Affine::translate(x + adv / 2.0, -(li as f64) * 1.5 * h));
                        x += adv;
                    }
                    i += count;
                }
            }
            // centre the laid-out letters on VCarve's box
            let (mut lo, mut hi) = (pt(f64::MAX, f64::MAX), pt(f64::MIN, f64::MIN));
            for (m, (_, ids)) in place.iter().zip(&cells) {
                for o in outlines.iter().filter(|o| ids.contains(&o.0)) {
                    for q in flatten(&transformed(&o.1, m), 0.002) {
                        (lo, hi) = (pt(lo.x.min(q.x), lo.y.min(q.y)), pt(hi.x.max(q.x), hi.y.max(q.y)));
                    }
                }
            }
            if lo.x > hi.x {
                return None;
            }
            let fit = to_drawing.then(&Affine::translate((x0 + x1 - lo.x - hi.x) / 2.0, (y0 + y1 - lo.y - hi.y) / 2.0));
            Some(place.iter().zip(cells).map(|(m, (_, ids))| (fit.then(m), ids)).collect::<Vec<_>>())
        })();
        match laid {
            Some(letters) => {
                out.blocks += 1;
                for (m, ids) in letters {
                    for id in ids {
                        out.placed.insert(id, (m, start));
                    }
                }
            }
            None => {
                out.unread += 1;
                out.skip.extend(outlines.iter().map(|o| o.0).filter(|&at| in_letters(at)));
            }
        }
    }
    out
}

enum Val {
    N(f64),
    S(String),
}

/// Named settings (utParameter): u32 2, CString key, u32 type (0 f64, 1 i32, 2 bool byte, 3 CString), value.
fn params(d: &[u8]) -> Vec<(usize, String, Val, usize)> {
    let mut out = Vec::new();
    for at in find_all(d, b"\x02\0\0\0\xff\xfe\xff") {
        let mut r = Rd { d, p: at + 4 };
        let Some(k) = r.cstring().filter(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) else { continue };
        let v = match r.u32() {
            Some(0) => r.f64().map(Val::N),
            Some(1) => r.u32().map(|x| Val::N(x as i32 as f64)),
            Some(2) => r.u8().map(|x| Val::N(x as f64)),
            Some(3) => r.cstring().map(Val::S),
            _ => None,
        };
        if let Some(v) = v {
            out.push((at, k, v, r.p));
        }
    }
    out
}

/// A bit, found by its name record at `i`: u32 4, u32 rpm, u32 number, u32 length, ASCII name ending in 0. Before
/// it: u32 2, u32 kind (1 end mill, 3 V-bit, 8 form), f32 radius, f32 cone height, 5 bytes, f64 diameter, stepdown,
/// stepover, feed, plunge. Returns the bit (inches) and its stepover.
fn tool_at(d: &[u8], i: usize, scale: f64) -> Option<(Tool, f64)> {
    let u = |p: usize| d.get(p..p + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let n = u(i + 12)? as usize;
    let name = d.get(i + 16..i + 16 + n)?;
    if !(2..200).contains(&n) || name[n - 1] != 0 || !name[..n - 1].iter().all(|c| (32..127).contains(c)) {
        return None;
    }
    let (rpm, slot) = (u(i + 4)?, u(i + 8)?);
    if u(i)? > 16 || rpm > 200_000 || slot > 1000 || i < 61 || u(i - 61)? != 2 {
        return None;
    }
    let mut r = Rd { d, p: i - 40 };
    let (dia, stepdown, stepover, feed, plunge) = (r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?);
    if !(dia > 0.0 && dia * scale < 10.0) {
        return None;
    }
    let f32_at = |p: usize| d.get(p..p + 4).map(|b| f32::from_le_bytes(b.try_into().unwrap()) as f64);
    let (rad, height) = (f32_at(i - 53)?, f32_at(i - 49)?);
    let name = String::from_utf8_lossy(&name[..n - 1]).trim().to_string();
    let lower = name.to_lowercase();
    let mut t = Tool { diameter: dia * scale, stepdown: stepdown * scale, feed_xy: feed * scale, feed_z: plunge * scale, rpm: rpm as i32, ..Tool::default() };
    match u(i - 57)? {
        3 if height > 0.0 => (t.kind, t.angle) = (ToolType::VBit, (2.0 * (rad / height).atan().to_degrees()).round()),
        _ if lower.contains("ball") => t.kind = ToolType::BallNose,
        8 if lower.contains("round") => t.kind = ToolType::RoundOver,
        _ => {}
    }
    t.name = name;
    Some((t, stepover * scale))
}

/// The toolpaths, as GCarve toolpaths on the shapes they cut. Returns how many were made, how many array copies
/// were set and what was left out.
fn toolpaths(d: &[u8], objs: &HashMap<&[u8], Vec<i32>>, tabs: &HashMap<i32, Vec<Pt>>, scale: f64, doc: &mut Document) -> (usize, usize, Vec<String>) {
    let mut nodes = Vec::new();
    for at in find_all(d, &[0xff, 0xfe, 0xff]) {
        let mut r = Rd { d, p: at };
        if let Some(name) = r.cstring() {
            if d[r.p..].starts_with(b"\x10\0\0\0Vectric__Version") {
                nodes.push((at, name));
            }
        }
    }
    // the tree, depth first: a node is its record (class tag, u32 3, name, id block, its toolpath), u32 child count,
    // the children, then one byte as it closes. So the count sits just before the next node's record, less one byte
    // per node closed in between (the count says how many close)
    let record = |at: usize| {
        if at >= 6 && d[at - 5] & 0x80 != 0 {
            at - 6
        } else {
            let from = at.saturating_sub(80);
            d[from..at].windows(4).rposition(|w| w == [0xff, 0xff, 1, 0]).map_or(at, |i| from + i)
        }
    };
    let mut parent = vec![usize::MAX; nodes.len()];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for (k, up) in parent.iter_mut().enumerate() {
        if let Some(top) = stack.last_mut() {
            *up = top.0;
            top.1 = top.1.saturating_sub(1);
        }
        let Some(next) = nodes.get(k + 1) else { break };
        let r = record(next.0);
        let count = (0..stack.len() + 2).find_map(|c| {
            let n = r.checked_sub(4 + c).and_then(|p| d.get(p..p + 4)).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)?;
            let closing = if n > 0 { 0 } else { 1 + stack.iter().rev().take_while(|t| t.1 == 0).count() };
            (n <= nodes.len() && closing == c).then_some(n)
        });
        stack.push((k, count.unwrap_or(0)));
        while stack.last().is_some_and(|t| t.1 == 0) {
            stack.pop();
        }
    }
    let under = |mut k: usize, a: usize| {
        while k != usize::MAX {
            k = parent[k];
            if k == a {
                return true;
            }
        }
        false
    };
    let mut node_op: Vec<(usize, usize)> = Vec::new();
    let mut made_copies = 0;
    let mut arrays: Vec<(usize, usize, usize, f64, f64)> = Vec::new();
    let tools: Vec<(usize, Tool, f64)> = (0..d.len().saturating_sub(20)).filter_map(|i| tool_at(d, i, scale).map(|(t, s)| (i, t, s))).collect();
    let ps = params(d);
    let mut groups: Vec<(usize, Vec<i32>)> = Vec::new();
    for at in find_all(d, b"\x02\0\0\0\0") {
        let mut r = Rd { d, p: at + 5 };
        let Some(n) = r.u32().map(|n| n as usize).filter(|&n| n > 0 && n <= (d.len() - r.p) / 16) else { continue };
        // VCarve keeps the GUIDs of vectors deleted since: use the ones still there (a random match is impossible)
        let ids: Vec<i32> = (0..n).filter_map(|k| objs.get(&d[r.p + 16 * k..r.p + 16 * k + 16])).flatten().copied().collect();
        if !ids.is_empty() {
            groups.push((at, ids));
        }
    }
    let (mut made, mut skipped) = (0, BTreeMap::<String, usize>::new());
    for (k, (at, name)) in nodes.iter().enumerate() {
        let span = *at..nodes.get(k + 1).map_or(d.len(), |n| n.0);
        let mine: Vec<&(usize, Tool, f64)> = tools.iter().filter(|t| span.contains(&t.0)).collect();
        let Some(&&(_, ref bit, stepover)) = mine.first() else { continue }; // Root, Top, group headings
        let own = *at..mine.get(1).map_or(span.end, |t| t.0);
        let p: HashMap<&str, &Val> = ps.iter().filter(|q| own.contains(&q.0)).map(|q| (q.1.as_str(), &q.2)).collect();
        let num = |key: &str| match p.get(key) {
            Some(Val::N(v)) => *v,
            _ => 0.0,
        };
        let text = |key: &str| match p.get(key) {
            Some(Val::S(v)) => v.as_str(),
            _ => "",
        };
        let ttype = text("ToolpathType");
        let (kind, pre) = match text("EditingDialog") {
            "uiProfileMachineForm" => (OpType::Profile, "_ppd"),
            "uiQuickEngraveDialog" if ttype == "Pocket" => (OpType::Pocket, "_pkpd"),
            "uiQuickEngraveDialog" => (OpType::Profile, "_ppd"),
            "uiMultiToolPocketForm" => (OpType::Pocket, "_mtpkpd"),
            "uiDrillForm" => (OpType::Drill, "_dpd"),
            "uiVCarvingForm" => (OpType::VCarve, "_vcpd"),
            "uiChamferDialog" => (OpType::Chamfer, "_chpd"),
            "uiToolpathMergeForm" => continue, // its parts are their own nodes
            "uiToolpathsArrayCopyForm" => {
                // rows × columns of copies of the toolpaths under it, spaced by the given distance (or that gap
                // between copies of the bounding box)
                let (rows, cols) = (num("_actdRectNumRows").clamp(1.0, 500.0) as usize, num("_actdRectNumColumns").clamp(1.0, 500.0) as usize);
                let gaps = num("_actdRectSpacingIsGaps") == 1.0;
                let w = if gaps { num("_actdBoundsMaxX") - num("_actdBoundsMinX") } else { 0.0 };
                let h = if gaps { num("_actdBoundsMaxY") - num("_actdBoundsMinY") } else { 0.0 };
                arrays.push((k, rows, cols, (w + num("_actdRectXSpacing")) * scale, (h + num("_actdRectYSpacing")) * scale));
                continue;
            }
            other => {
                *skipped.entry(other.trim_start_matches("ui").trim_end_matches("Form").trim_end_matches("Dialog").to_string()).or_default() += 1;
                continue;
            }
        };
        let Some(shapes) = groups.iter().find(|g| span.contains(&g.0)).map(|g| g.1.clone()) else {
            *skipped.entry("toolpath whose vectors were deleted in VCarve".into()).or_default() += 1;
            continue;
        };
        let len = |key: &str| num(&format!("{pre}{key}")) * scale;
        let mut op = Op { kind, name: name.clone(), shape_ids: shapes, tool: add_tool(doc, bit), ..Op::default() };
        let mut distinct: Vec<&Tool> = Vec::new();
        for t in &mine {
            if !distinct.iter().any(|x| x.name == t.1.name) {
                distinct.push(&t.1);
            }
        }
        match kind {
            OpType::Profile => {
                op.side = [Side::Outside, Side::Inside, Side::On][(num("_ppdProfileType") as usize).min(2)];
                op.depth = len("CutDepth");
                op.climb = num("_ppdCutDirection") == 1.0;
                op.allowance = len("Allowance");
                op.tabs = num("_ppdUseTabs") == 1.0;
                (op.tab_width, op.tab_height, op.tab_ramped) = (len("TabLength"), len("TabThickness"), num("_ppd3dTabs") == 1.0);
                op.tab_count = (num("tabsNumTabs") as i32).max(1);
                op.tab_corners = num("tabsAvoidCorners") == 1.0;
                if num("tabsAutoType") == 1.0 && num("tabsTabDistance") > 0.0 {
                    // ponytail: 1 = VCarve's "constant distance" is a guess (all 305 NAS files use 0, constant number)
                    op.tab_spacing = num("tabsTabDistance") * scale;
                }
                // tabs placed in VCarve go where they were (GCarve's hand-placed tabs)
                if op.tabs {
                    op.tab_points = op.shape_ids.iter().filter_map(|id| tabs.get(id)).flatten().map(|q| [q.x, q.y]).collect();
                }
                op.entry = if num("_mcrdDoRamping") == 1.0 { Entry::Ramp } else { Entry::Plunge };
                op.ramp_length = num("_mcrdRampingDistance") * scale;
                if num("_mcldDoRamping") == 1.0 {
                    op.ramp_in = num("_mcldCircularRadius") * scale;
                }
            }
            OpType::Pocket => {
                op.depth = len("CutDepth");
                op.allowance = len("Allowance");
                op.raster = num(&format!("{pre}DoRaster")) == 1.0;
                op.raster_angle = num(&format!("{pre}RasterAngle")).round() as i32;
                op.climb = num("_mtpkptpdCutDirection") == 1.0 || num("_pkpdCutDirection") == 1.0;
                op.entry = if num("_mchrdHelicalRamps") == 1.0 {
                    Entry::Spiral
                } else if num("_mtpkptpdDoRamping") == 1.0 {
                    Entry::Ramp
                } else {
                    Entry::Plunge
                };
                op.ramp_length = num("_mtpkptpdRampDistance") * scale;
                // several bits: GCarve clears with the largest and finishes with the smallest
                distinct.sort_by(|a, b| a.diameter.total_cmp(&b.diameter));
                op.tool = add_tool(doc, distinct[0]);
                if let Some(big) = distinct.last().filter(|b| b.diameter > distinct[0].diameter) {
                    op.clear_tool = add_tool(doc, big);
                }
                let d0 = doc.tools[op.tool as usize].diameter;
                if stepover > 0.0 && d0 > 0.0 {
                    op.stepover = (stepover / d0).clamp(0.05, 1.0);
                }
            }
            OpType::Drill => {
                op.depth = len("CutDepth");
                if num("_dpdPeckDrill") == 1.0 {
                    op.peck = bit.stepdown;
                }
            }
            OpType::VCarve => {
                op.start_depth = len("StartDepth");
                if num("_vcpdDoFlatBottom") == 1.0 {
                    op.flat_depth = len("FlatDepth");
                    if let Some(clear) = distinct.iter().find(|t| t.kind != ToolType::VBit).filter(|_| num("_vcpdUseAreClearTool") == 1.0) {
                        op.clear_tool = add_tool(doc, clear);
                    }
                }
            }
            _ => {
                op.depth = len("ChamferDepth");
                op.start_depth = len("StartDepth");
                op.side = if num("_chpdInside") == 1.0 { Side::Inside } else { Side::Outside };
            }
        }
        // VCarve's folders, merged and array toolpaths: the nearest one above it names its group in the list
        let up = parent[k];
        let root_or_side = |n: usize| parent[n] == usize::MAX || parent[parent[n]] == usize::MAX; // Root, Top, Bottom
        if up != usize::MAX && !root_or_side(up) {
            op.op_group = nodes[up].1.clone();
        }
        node_op.push((k, doc.ops.len()));
        doc.ops.push(op);
        made += 1;
    }
    // array copies: the toolpaths under each array get its grid (GCarve's per-toolpath Array setting)
    for &(a, rows, cols, px, py) in &arrays {
        let ops: Vec<usize> = node_op.iter().filter(|(k, _)| under(*k, a)).map(|q| q.1).collect();
        if ops.is_empty() {
            *skipped.entry("array copy".into()).or_default() += 1;
            continue;
        }
        made_copies += 1;
        for o in ops {
            let op = &mut doc.ops[o];
            (op.array_rows, op.array_cols, op.array_dx, op.array_dy) = (rows as i32, cols as i32, px, py);
        }
    }
    let plural = |k: String, n: usize| if n == 1 { k } else { k.replacen("copy", "copies", 1).replacen("toolpath ", "toolpaths ", 1) };
    (made, made_copies, skipped.into_iter().map(|(k, n)| format!("{n} {}", plural(k, n))).collect())
}

/// The project's bit under its VCarve name: replaces a bit of that name (the built-in list uses VCarve's names), else
/// added.
fn add_tool(doc: &mut Document, t: &Tool) -> i32 {
    match doc.tools.iter().position(|x| x.name == t.name) {
        Some(i) => {
            doc.tools[i] = t.clone();
            i as i32
        }
        None => {
            doc.tools.push(t.clone());
            doc.tools.len() as i32 - 1
        }
    }
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

    /// a V-bit record and two settings as VCarve writes them
    #[test]
    fn bit_and_settings_are_read() {
        let mut d = b"mcVBitTool".to_vec();
        d.extend([2, 0, 0, 0, 3, 0, 0, 0]);
        d.extend(0.125f32.to_le_bytes());
        d.extend((0.125f32 / 30f32.to_radians().tan()).to_le_bytes()); // 60 degree cone
        d.extend([7, 0, 0, 0, 0]);
        d.extend(le(&[0.25, 0.2, 0.01, 100.0, 50.0]));
        let at = d.len();
        for v in [4u32, 18000, 1, 21] {
            d.extend(v.to_le_bytes());
        }
        d.extend(b"V-Bit (60 deg 0.25\")\0");
        let key = |d: &mut Vec<u8>, k: &str, ty: u32| {
            d.extend([9, 0x80, 2, 0, 0, 0, 0xff, 0xfe, 0xff, k.len() as u8]);
            d.extend(k.encode_utf16().flat_map(|c| c.to_le_bytes()));
            d.extend(ty.to_le_bytes());
        };
        key(&mut d, "_ppdCutDepth", 0);
        d.extend(0.3f64.to_le_bytes());
        key(&mut d, "ToolpathType", 3);
        d.extend([0xff, 0xfe, 0xff, 2, b'O', 0, b'n', 0]);
        let (t, stepover) = tool_at(&d, at, 1.0).unwrap();
        assert_eq!((t.kind, t.angle, t.diameter, t.feed_xy, t.rpm, t.name.as_str()), (ToolType::VBit, 60.0, 0.25, 100.0, 18000, "V-Bit (60 deg 0.25\")"));
        assert_eq!(stepover, 0.01);
        let p = params(&d);
        assert!(matches!(p[0], (_, ref k, Val::N(v), _) if k == "_ppdCutDepth" && v == 0.3));
        assert!(matches!(p[1], (_, ref k, Val::S(ref v), _) if k == "ToolpathType" && v == "On"));
    }

    /// One file (or one sheet: GCARVE_CRV_SHEET=index) as SVG to compare with VCarve's preview: `GCARVE_CRV=x.crv GCARVE_CRV_SVG=x.svg cargo test -p
    /// gcarve-core --lib crv_to_svg -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn crv_to_svg() {
        let (d, note) = import_crv(&std::fs::read(std::env::var("GCARVE_CRV").unwrap()).unwrap()).unwrap();
        println!("{note}");
        for op in &d.ops {
            println!("  {} {:?} {:?} depth {:.3} tabs {} ({} placed) array {}x{} group {:?}", op.name, op.kind, op.side, op.depth, op.tabs, op.tab_points.len(), op.array_rows, op.array_cols, op.op_group);
        }
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
        let (mut ok, mut bad, mut vectors, mut ops) = (0, 0, 0, 0);
        while let Some(p) = stack.pop() {
            for e in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                let q = e.path();
                if q.is_dir() {
                    stack.push(q);
                } else if q.extension().is_some_and(|x| x.eq_ignore_ascii_case("crv")) {
                    match import_crv(&std::fs::read(&q).unwrap()) {
                        Ok((d, note)) => {
                            ok += 1;
                            vectors += d.shapes.len();
                            ops += d.ops.len();
                            if std::env::var("GCARVE_CRV_NOTES").is_ok() {
                                println!("{}: {note}", q.display());
                            }
                        }
                        Err(e) => {
                            bad += 1;
                            println!("{}: {e}", q.display());
                        }
                    }
                }
            }
        }
        println!("{ok} opened, {bad} failed, {vectors} vectors, {ops} toolpaths");
        assert_eq!(bad, 0);
    }
}
