//! Document -> toolpaths -> G-code files, the way the Qt app's Canvas did it (regenerate / buildAllJobs / exportGcode),
//! shared by the desktop and web apps. Pure: no file system, no UI.

use crate::cam::{self, effective_tool, preset_for};
use crate::document::{Document, OpType, Tool};
use crate::feeds::{assess_feeds_range, chip_groups, chip_load_range, classify_material, suggest_feeds, ChipRange, Cutter, FeedAssessment, FeedSuggestion};
use crate::job::{Job, Toolpath};
use crate::postdef::{find_post, post_jobs, PostDef, PostOutput};
use crate::relief::generate_relief;

fn cutter(t: &Tool) -> Cutter {
    Cutter { vbit: t.kind == crate::document::ToolType::VBit, diameter: t.diameter, flutes: t.flutes }
}

/// The bit maker's chip load for the board material, when the tool came from the bit database.
pub fn maker_chip(t: &Tool, material: &str) -> Option<ChipRange> {
    chip_groups(classify_material(material)).iter().find_map(|g| t.chip_loads.get(*g)).map(|r| ChipRange { lo: r[0], hi: r[1] })
}

/// Maker charts and GCarve's own table assume a rigid industrial router. Lighter machines flex and chatter at those
/// chip loads, so suggestions scale by the machine's class: (chip load factor, pass depth factor).
/// ponytail: two numbers per class from Carbide 3D / Sienci starting feeds vs the Amana / Onsrud charts; per-machine
/// rigidity data would be better if makers published it.
pub fn class_factor(class: &str) -> (f64, f64) {
    match class {
        "prosumer" => (0.6, 0.75),
        "hobby" => (0.35, 0.5),
        "desktop" | "handheld" => (0.3, 0.4),
        _ => (1.0, 1.0), // industrial, or no machine picked
    }
}

/// How the job's machine moves, for time estimates: its published rapid speed (else 400 in/min) and a typical
/// acceleration for its class (makers rarely publish it).
pub fn motion(doc: &Document) -> crate::order::Motion {
    let m = doc.material.machine.as_ref();
    let accel = match m.map(|m| m.class.as_str()) {
        Some("industrial") => 100.0,
        Some("prosumer") => 40.0,
        Some("hobby") => 20.0,
        Some("desktop" | "handheld") => 12.0,
        _ => 30.0,
    };
    crate::order::Motion { rapid_ipm: m.map(|m| m.rapid_xy).filter(|&r| r > 0.0).unwrap_or(400.0), accel }
}

/// Feed suggestion within the machine's limits (spindle RPM range, max feeds) and, when known, the maker's chip load.
pub fn suggest(doc: &Document, t: &Tool) -> FeedSuggestion {
    let m = doc.material.machine.as_ref();
    let mut max_rpm = m.map(|m| m.rpm_max).filter(|&r| r > 0.0).unwrap_or(18000.0);
    if !t.catalog.is_empty() {
        max_rpm = max_rpm.min(f64::from(t.rpm.max(1000))); // the maker's recommended / max RPM for this bit
    }
    let max_feed = m.map(|m| m.feed_xy).filter(|&f| f > 0.0).unwrap_or(600.0);
    let mut s = suggest_feeds(&cutter(t), &doc.material.kind, max_rpm as i32, max_feed);
    let rpm_min = m.map(|m| m.rpm_min).unwrap_or(0.0);
    if f64::from(s.rpm) < rpm_min {
        s.rpm = rpm_min as i32; // the spindle can't go slower: keep the chip load by feeding faster
    }
    let (kc, kd) = class_factor(m.map_or("", |m| m.class.as_str()));
    let chip = maker_chip(t, &doc.material.kind).map(|c| (c.lo + c.hi) / 2.0).unwrap_or(s.chip_load) * kc;
    s.stepdown = (s.stepdown * kd * 1000.0).round() / 1000.0;
    let flutes = f64::from(t.flutes.max(1));
    let feed = (f64::from(s.rpm) * flutes * chip).min(max_feed);
    if (feed - s.feed_xy).abs() > 0.5 {
        s.feed_z = (s.feed_z * feed / s.feed_xy.max(1.0)).round();
        s.feed_xy = feed.round();
    }
    s.chip_load = s.feed_xy / (f64::from(s.rpm) * flutes);
    if let Some(fz) = m.map(|m| m.feed_z).filter(|&f| f > 0.0) {
        s.feed_z = s.feed_z.min(fz);
    }
    s
}

/// Risk meter for typed feeds, against the maker's chip load when the tool has one.
pub fn assess(doc: &Document, t: &Tool, feed_xy: f64, feed_z: f64, rpm: i32, stepdown: f64) -> FeedAssessment {
    let mat = &doc.material.kind;
    let mut cr = maker_chip(t, mat).unwrap_or_else(|| chip_load_range(classify_material(mat), if t.kind == crate::document::ToolType::VBit { t.diameter.min(0.125) } else { t.diameter }));
    // the meter's sweet spot follows the machine: a hobby router at an industrial chip load chatters or stalls
    let (kc, _) = class_factor(doc.material.machine.as_ref().map_or("", |m| m.class.as_str()));
    cr.lo *= kc;
    cr.hi *= kc;
    assess_feeds_range(&cutter(t), cr, mat, feed_xy, feed_z, rpm, stepdown)
}

/// The tool as it will cut: material preset, or the suggestion (machine limits, maker chip load) when auto feeds is on.
pub fn tool_for_cut(doc: &Document, i: usize) -> Tool {
    let t = &doc.tools[i];
    if preset_for(t, &doc.material.kind).is_some() || !doc.material.auto_feeds {
        return effective_tool(t, &doc.material.kind);
    }
    let s = suggest(doc, t);
    let mut e = t.clone();
    e.feed_xy = s.feed_xy;
    e.feed_z = s.feed_z;
    e.rpm = s.rpm;
    e.stepdown = s.stepdown;
    e
}

/// Cutting order: a profile that cuts a part right through with no tabs, listed before a toolpath that still has
/// work inside that part, would free the part before it is finished (export check, active sheet). Also: small parts
/// (under 16 sq in) cut free with no tabs, which shift into the bit or get thrown.
pub fn order_issues(doc: &Document) -> Vec<String> {
    let rect_of = |op: &crate::document::Op| {
        let mut r = crate::geom::Rect::default();
        for c in doc.contours_of(&doc.op_shape_ids(op)) {
            r.add_rect(&crate::geom::bounds(&c));
        }
        r
    };
    let mut out = Vec::new();
    for (i, cut) in doc.ops.iter().enumerate() {
        let through = cut.kind == OpType::Profile && cut.side != crate::document::Side::Inside && !cut.tabs && cut.depth >= doc.material.t - 1e-3;
        if !through {
            continue;
        }
        let part = rect_of(cut);
        if part.is_empty() {
            continue;
        }
        let outlines: Vec<crate::geom::Rect> = doc.contours_of(&doc.op_shape_ids(cut)).iter().filter(|c| c.closed).map(crate::geom::bounds).collect();
        let small = outlines.iter().enumerate().filter(|(k, r)| r.w() * r.h() < 16.0 && !outlines.iter().enumerate().any(|(j, o)| j != *k && o.contains(r) && o.w() * o.h() > r.w() * r.h())).count();
        if small > 0 && cut.skin <= 0.0 {
            out.push(format!("\"{}\" cuts {small} small part{} free with no tabs: {} can shift into the bit or be thrown. Add tabs or an onion skin, or hold {} down.", cut.name, if small == 1 { "" } else { "s" }, if small == 1 { "it" } else { "they" }, if small == 1 { "it" } else { "them" }));
        }
        // later toolpaths that work inside the part (another cut-out of the same shapes doesn't count)
        let later: Vec<&str> = doc.ops[i + 1..].iter().filter(|o| doc.op_shape_ids(o) != doc.op_shape_ids(cut)).filter(|o| { let r = rect_of(o); !r.is_empty() && part.contains(&r) }).map(|o| o.name.as_str()).collect();
        if !later.is_empty() {
            out.push(format!("\"{}\" cuts the part free before {} {} cut: move it below {} in the toolpath list (right-click › Cut Later), or give it tabs.", cut.name, later.join(", "), if later.len() == 1 { "is" } else { "are" }, if later.len() == 1 { "it" } else { "them" }));
        }
    }
    out
}

/// What would push the chosen machine past its published limits (export check; the user can still export anyway).
pub fn machine_issues(doc: &Document, eff: &[Tool]) -> Vec<String> {
    let Some(m) = &doc.material.machine else { return Vec::new() };
    let mut out = Vec::new();
    let b = &doc.material;
    if m.travel_x > 0.0 && m.travel_y > 0.0 {
        let fits = |w: f64, h: f64| w <= m.travel_x + 1e-6 && h <= m.travel_y + 1e-6;
        if !fits(b.w, b.h) && !fits(b.h, b.w) {
            out.push(format!("Board {:.2} × {:.2} in is bigger than the {} travel ({:.1} × {:.1} in).", b.w, b.h, m.name, m.travel_x, m.travel_y));
        }
    }
    if m.travel_z > 0.0 && b.t + b.safe_z > m.travel_z {
        out.push(format!("Board thickness + safe Z ({:.2} in) is more than the {} Z travel ({:.1} in).", b.t + b.safe_z, m.name, m.travel_z));
    }
    let used: std::collections::BTreeSet<usize> = doc.ops.iter().filter(|o| o.tool >= 0).map(|o| o.tool as usize).collect();
    if m.tool_slots > 0 {
        // tool changer: every bit the job loads needs its own pocket in the rack
        let loaded: std::collections::BTreeSet<usize> = doc.ops.iter().flat_map(|o| [o.tool, o.clear_tool]).filter(|&t| t >= 0 && (t as usize) < eff.len()).map(|t| t as usize).collect();
        if loaded.len() > m.tool_slots as usize {
            out.push(format!("This job uses {} bits but the {} tool changer has {} slots.", loaded.len(), m.name, m.tool_slots));
        }
        let mut seen: std::collections::BTreeMap<i32, &str> = Default::default();
        for &i in &loaded {
            let t = &eff[i];
            if t.slot > m.tool_slots {
                out.push(format!("{}: slot {} is beyond the {}-slot tool changer.", t.name, t.slot, m.tool_slots));
            }
            if t.slot > 0 {
                if let Some(other) = seen.insert(t.slot, &t.name) {
                    out.push(format!("{} and {} are both in slot {}.", other, t.name, t.slot));
                }
            }
        }
    }
    for i in used {
        let Some(t) = eff.get(i) else { continue };
        if m.rpm_max > 0.0 && f64::from(t.rpm) > m.rpm_max {
            out.push(format!("{}: {} RPM is above the spindle's {} max.", t.name, t.rpm, m.rpm_max));
        }
        if m.rpm_min > 0.0 && f64::from(t.rpm) < m.rpm_min {
            out.push(format!("{}: {} RPM is below the spindle's {} minimum.", t.name, t.rpm, m.rpm_min));
        }
        if m.feed_xy > 0.0 && t.feed_xy > m.feed_xy {
            out.push(format!("{}: feed {} in/min is above the machine's {} in/min.", t.name, t.feed_xy, m.feed_xy));
        }
        if m.feed_z > 0.0 && t.feed_z > m.feed_z {
            out.push(format!("{}: plunge {} in/min is above the machine's {} in/min.", t.name, t.feed_z, m.feed_z));
        }
    }
    out
}

pub struct Generated {
    /// per op (same index as doc.ops), empty for ops that can't run
    pub paths: Vec<Vec<Toolpath>>,
    pub warnings: Vec<String>,
    /// per op: rough cutting time in minutes at the programmed feeds (rapids at 400 in/min)
    pub minutes: Vec<f64>,
    /// app live preview: tool of an extra, not yet added toolpath at the end of `paths`
    pub draft_tool: i32,
}

/// Toolpaths for every op on the active sheet.
pub fn regenerate(doc: &Document) -> Generated {
    regenerate_some(doc, &|_| true)
}

/// Only the toolpaths `want` picks (by op index); the others come back empty with no time (the app works out just
/// what is on screen while drawing).
pub fn regenerate_some(doc: &Document, want: &dyn Fn(usize) -> bool) -> Generated {
    let mut g = Generated { paths: Vec::new(), warnings: Vec::new(), minutes: Vec::new(), draft_tool: -1 };
    let mo = motion(doc);
    for (i, op) in doc.ops.iter().enumerate() {
        if !want(i) {
            g.paths.push(Vec::new());
            g.minutes.push(0.0);
            continue;
        }
        let tool_ok = op.tool >= 0 && (op.tool as usize) < doc.tools.len();
        if !tool_ok {
            g.paths.push(Vec::new());
            g.minutes.push(0.0);
            g.warnings.push(format!("{}: no tool", op.name));
            continue;
        }
        let tool = tool_for_cut(doc, op.tool as usize);
        let clear = (op.clear_tool >= 0 && (op.clear_tool as usize) < doc.tools.len()).then(|| tool_for_cut(doc, op.clear_tool as usize));
        let mut r = if op.kind == OpType::Imported {
            let mut paths = crate::gimport::section_toolpaths(&op.gcode, &op.name);
            for tp in &mut paths {
                cam::enforce_safe_rapids(tp, doc.material.retract_z);
            }
            cam::CamResult { paths, warnings: Vec::new() }
        } else if op.kind == OpType::Relief {
            match doc.find_relief(op.relief) {
                Some(rl) if !rl.height.is_empty() => {
                    let boundary = doc.contours_of(&doc.op_shape_ids(op));
                    generate_relief(op, &tool, if op.rough { clear.as_ref() } else { None }, rl, &doc.material, (!boundary.is_empty()).then_some(boundary.as_slice()))
                }
                _ => cam::CamResult { paths: Vec::new(), warnings: vec![format!("{}: 3D model not loaded", op.name)] },
            }
        } else {
            let mut cs = doc.contours_of(&doc.op_shape_ids(op).into_iter().filter(|id| *id != op.section).collect::<Vec<_>>());
            // moulding: the cross-section rides along as the last contour (cam::generate_moulding)
            let moulding = op.kind == OpType::Flute && op.section > 0;
            match doc.find(op.section).filter(|_| moulding) {
                Some(s) if !cs.is_empty() => cs.push(s.c.clone()),
                None if moulding => cs.clear(),
                _ => {}
            }
            if cs.is_empty() {
                cam::CamResult { paths: Vec::new(), warnings: Vec::new() }
            } else {
                cam::generate(op, &tool, &cs, &doc.material, clear.as_ref())
            }
        };
        cam::array_copies(op, &mut r.paths);
        if let Some(rl) = (op.project_on > 0 && op.kind != OpType::Relief).then(|| doc.find_relief(op.project_on)).flatten().filter(|rl| !rl.height.is_empty()) {
            crate::relief::project_paths(&mut r.paths, rl);
        }
        g.minutes.push(r.paths.iter().map(|tp| crate::order::estimate_minutes_with(tp, tp.feed_xy, tp.feed_z, mo)).sum());
        g.paths.push(r.paths);
        g.warnings.extend(r.warnings);
    }
    g
}

/// Every sheet's jobs, named "<project>_[SheetN_]NN_<tool>". `gen_active` is the active sheet's generated paths
/// (reused); other sheets are regenerated here.
pub fn all_jobs(doc: &Document, gen_active: &Generated, project: &str, timestamp: &str) -> Vec<Job> {
    all_jobs_each(doc, gen_active, project, timestamp, &mut |_, _, _| {})
}

/// `all_jobs`, calling `per_sheet(sheet document, its toolpaths, its jobs)` for every sheet as it goes.
fn all_jobs_each(doc: &Document, gen_active: &Generated, project: &str, timestamp: &str, per_sheet: &mut dyn FnMut(&Document, &Generated, &[Job])) -> Vec<Job> {
    let eff: Vec<Tool> = (0..doc.tools.len()).map(|i| tool_for_cut(doc, i)).collect();
    let mut jobs = Vec::new();
    let sheets = doc.sheets.len().max(1);
    for si in 0..sheets {
        let other;
        let (d, g) = if si as i32 == doc.active_sheet {
            (doc, gen_active)
        } else {
            let mut d = doc.clone();
            d.set_active_sheet(si as i32);
            d.apply_bindings(); // thickness / board-w / board-h are this sheet's (Qt switchSheet): bound depths follow
            let g = regenerate(&d);
            other = (d, g);
            (&other.0, &other.1)
        };
        let mut js = cam::build_jobs(&d.ops, &eff, &g.paths, &d.material, timestamp);
        for j in &mut js {
            if sheets > 1 {
                j.filename = format!("Sheet{}_{}", si + 1, j.filename);
            }
            if !project.is_empty() {
                j.filename = format!("{project}_{}", j.filename);
            }
        }
        per_sheet(d, g, &js);
        jobs.extend(js);
    }
    jobs
}

/// A sheet's picture for the setup sheet: (data URI, board rect x0 y0 w h), None = no picture.
pub type SheetPicture = Option<(String, [f64; 4])>;

/// The setup sheet (printable job sheet) for an export with these settings: same jobs and file names (`names` =
/// the export dialog's, empty = the post's), one page per sheet. `picture(sheet document, its toolpaths)` gives the
/// top-down image as (data URI, board rect x0 y0 w h); the core has no image encoder.
#[allow(clippy::too_many_arguments)]
pub fn setup_sheet(
    doc: &Document, gen: &Generated, project: &str, date: &str, air_cut_raise: f64, post: &PostDef, names: &[String], picture: &mut dyn FnMut(&Document, &Generated) -> SheetPicture,
) -> String {
    use crate::setupsheet::{setup_sheet_html, SetupPage, SetupSheet};
    let tools: Vec<Tool> = (0..doc.tools.len()).map(|i| tool_for_cut(doc, i)).collect();
    let mut pages: Vec<SetupPage> = Vec::new();
    let mut counts = Vec::new();
    let mut jobs = all_jobs_each(doc, gen, project, "", &mut |d, g, js| {
        let mut p = SetupPage { board: d.material.clone(), ..Default::default() };
        for w in &g.warnings {
            if !p.warnings.contains(w) {
                p.warnings.push(w.clone()); // Qt camWarnings: duplicates removed
            }
        }
        p.warnings.extend(cam::safety_issues(&d.ops, &tools, &g.paths, &d.material));
        p.warnings.extend(keep_out_issues(d, &tools, &g.paths));
        if let Some((uri, rect)) = picture(d, g) {
            p.image = uri;
            p.image_rect = rect;
        }
        pages.push(p);
        counts.push(js.len());
    });
    if air_cut_raise > 0.0 {
        cam::make_air_cut(&mut jobs, air_cut_raise);
    }
    let files = post_jobs(&jobs, post);
    let combined = files.len() != jobs.len(); // one tool-change file for all jobs
    let mut jobs = jobs.into_iter().enumerate();
    for (p, n) in pages.iter_mut().zip(counts) {
        for (k, j) in jobs.by_ref().take(n) {
            let f = files.get(if combined { 0 } else { k }).map(|f| clean_file_name(names.get(if combined { 0 } else { k }).map(String::as_str).unwrap_or(""), &f.filename));
            p.files.push(f.unwrap_or_else(|| j.filename.clone()));
            if !post.tool_change.is_empty() {
                p.tool_slots.push(if j.slot > 0 { j.slot } else { k as i32 + 1 }); // TOOLNUM in the combined file
            }
            p.jobs.push(j);
        }
    }
    setup_sheet_html(&SetupSheet { project: project.to_string(), date: date.to_string(), post: post.name.clone(), mm: doc.bare_unit_in < 0.5, tools, pages, motion: motion(doc) })
}

/// The project's post: library id, or `user_posts` (id -> PostDef JSON) for uploaded ones; WinCNC when unknown.
pub fn active_post(doc: &Document, user_posts: &dyn Fn(&str) -> Option<PostDef>) -> PostDef {
    if let Some(p) = find_post(&doc.material.post) {
        return p.clone();
    }
    user_posts(&doc.material.post).unwrap_or_else(|| find_post("wincnc-single").cloned().expect("wincnc-single"))
}

pub struct ExportResult {
    pub files: Vec<PostOutput>,
    pub safety: Vec<String>,
    /// per file: (tools, toolpath names) inside it, for the export dialog (Qt Canvas::exportPlan)
    pub plan: Vec<(String, String)>,
}

/// G-code files for the whole project (all sheets), plus the safety findings for the active sheet.
pub fn export(doc: &Document, gen: &Generated, project: &str, timestamp: &str, air_cut_raise: f64, post: &PostDef) -> ExportResult {
    export_tiled(doc, gen, project, timestamp, air_cut_raise, post, None)
}

/// Height (inches above the board top) clamps and screw heads are taken to reach when the layer gives none, for the holder check.
const CLAMP_TOP: f64 = 1.0;

/// Toolpaths that come down near a clamp, screw or other no-go area (closed shapes of the active sheet on a layer
/// marked Keep out): any move below safe Z that passes within the bit's radius of one. Sampled every 0.05 in.
pub fn keep_out_issues(doc: &Document, tools: &[Tool], paths: &[Vec<Toolpath>]) -> Vec<String> {
    use crate::geom::{dist_to_segment, flatten, point_in_poly, pt, Pt, Rect};
    let zones: Vec<(Vec<Pt>, Rect, &str, f64)> = doc
        .shapes
        .iter()
        .filter(|s| s.sheet == doc.active_sheet && s.c.closed)
        .filter_map(|s| doc.layers.get(s.layer as usize).filter(|l| l.keep_out).map(|l| (s, l.name.as_str(), if l.clamp_height > 0.0 { l.clamp_height } else { CLAMP_TOP })))
        .map(|(s, name, top)| {
            let p = flatten(&s.c, 0.01);
            let mut r = Rect::default();
            p.iter().for_each(|q| r.add(*q));
            (p, r, name, top)
        })
        .collect();
    let mut out: Vec<String> = Vec::new();
    if zones.is_empty() {
        return out;
    }
    let low = doc.material.safe_z - 1e-6;
    for (op, gen) in doc.ops.iter().zip(paths) {
        for tp in gen {
            let ti = if tp.tool >= 0 { tp.tool } else { op.tool };
            let Some(t) = usize::try_from(ti).ok().and_then(|i| tools.get(i)) else { continue };
            // the holder is wider than the bit: from where it comes down to clamp height its radius counts
            let (r_bit, r_holder) = (t.diameter / 2.0, t.holder_dia.max(t.diameter) / 2.0);
            // the holder counts inside a zone when its clamps reach above the tip + stick-out
            let hit = |q: Pt, tip: f64| {
                zones.iter().find(|(poly, b, _, top)| {
                    let r = if t.stick_out > 0.0 && tip + t.stick_out < *top { r_holder } else { r_bit };
                    q.x >= b.x0 - r && q.x <= b.x1 + r && q.y >= b.y0 - r && q.y <= b.y1 + r
                        && (point_in_poly(q, poly.len(), |i| (poly[i].x, poly[i].y)) || (0..poly.len()).any(|i| dist_to_segment(q, poly[i], poly[(i + 1) % poly.len()]) < r))
                })
            };
            let (mut x, mut y, mut z) = (0.0, 0.0, doc.material.safe_z);
            let mut found = None;
            for mv in &tp.moves {
                let (nx, ny, nz) = (mv.x.unwrap_or(x), mv.y.unwrap_or(y), mv.z.unwrap_or(z));
                if z.min(nz) < low {
                    let n = ((nx - x).hypot(ny - y) / 0.05).ceil().max(1.0) as usize;
                    found = (0..=n).find_map(|k| hit(pt(x + (nx - x) * k as f64 / n as f64, y + (ny - y) * k as f64 / n as f64), z.min(nz)));
                }
                (x, y, z) = (nx, ny, nz);
                if found.is_some() {
                    break;
                }
            }
            if let Some((_, _, name, _)) = found {
                let s = format!("{} ({}): comes down inside a keep-out area (layer \"{name}\"): clamp or screw in the way.", tp.name, t.name);
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// Export option: cut a board longer than the machine in sections (`cam::tile_jobs`), lengths in inches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tiling {
    pub along_x: bool,
    pub len: f64,
    pub overlap: f64,
}

/// `export`, optionally split into tiles (one set of files per tile).
pub fn export_tiled(doc: &Document, gen: &Generated, project: &str, timestamp: &str, air_cut_raise: f64, post: &PostDef, tiling: Option<Tiling>) -> ExportResult {
    export_tiled_with(doc, gen, project, timestamp, air_cut_raise, post, tiling, false)
}

/// `export_tiled`; `ghost` = the raised run with the spindle off (`cam::make_ghost`, `_GHOST` files)
#[allow(clippy::too_many_arguments)]
pub fn export_tiled_with(doc: &Document, gen: &Generated, project: &str, timestamp: &str, air_cut_raise: f64, post: &PostDef, tiling: Option<Tiling>, ghost: bool) -> ExportResult {
    let eff: Vec<Tool> = (0..doc.tools.len()).map(|i| tool_for_cut(doc, i)).collect();
    let mut safety = cam::safety_issues(&doc.ops, &eff, &gen.paths, &doc.material);
    safety.extend(machine_issues(doc, &eff));
    safety.extend(order_issues(doc));
    safety.extend(keep_out_issues(doc, &eff, &gen.paths));
    let mut jobs = all_jobs(doc, gen, project, timestamp);
    if doc.material.merge {
        cam::merge_jobs(&mut jobs, doc.material.t);
    }
    if let Some(dia) = doc.material.rotary {
        cam::wrap_jobs(&mut jobs, dia);
        if doc.material.rotary_y {
            for j in &mut jobs {
                j.a_along_y = true;
                j.notes.push("ROTARY ALONG Y: the length is the Y axis, A turns the blank".into());
            }
        }
        if !post.linear.contains("{Y}") {
            safety.push(format!("Rotary job: the \"{}\" post writes plain numbers and can't name an A axis; choose a G-code post for the rotary.", post.name));
        }
        if tiling.is_some() {
            safety.push("Tiling doesn't apply to a rotary job: the files are NOT tiled.".into());
        }
    }
    if ghost {
        cam::make_ghost(&mut jobs, air_cut_raise.max(cam::air_cut_raise(&doc.material)));
    } else if air_cut_raise > 0.0 {
        cam::make_air_cut(&mut jobs, air_cut_raise);
    }
    if let Some(t) = tiling.filter(|_| doc.material.rotary.is_none()) {
        if doc.material.origin != 0 {
            safety.push("Tiling needs XY zero at the board's bottom-left corner (Board settings): the files are NOT tiled.".into());
        } else if t.len <= t.overlap || t.len <= 0.0 {
            safety.push("Tiling: the tile length must be larger than the overlap: the files are NOT tiled.".into());
        } else {
            jobs = cam::tile_jobs(&jobs, t.along_x, t.len, t.overlap, if t.along_x { doc.material.w } else { doc.material.h });
        }
    }
    if let Some(h) = doc.material.home {
        cam::go_home(&mut jobs, h);
    }
    let mo = motion(doc);
    let files = post_jobs(&jobs, post);
    let plan = (0..files.len())
        .map(|i| {
            // one file per job, or one combined tool-change file for all of them
            let mine: Vec<&Job> = jobs.iter().enumerate().filter(|(k, _)| files.len() != jobs.len() || *k == i).map(|(_, j)| j).collect();
            let mut paths: Vec<&str> = Vec::new();
            for tp in mine.iter().flat_map(|j| &j.toolpaths) {
                if !paths.contains(&tp.name.as_str()) {
                    paths.push(&tp.name);
                }
            }
            // cutting time at the programmed feeds with the machine's rapids and acceleration (`motion`)
            let mins: f64 = mine.iter().flat_map(|j| &j.toolpaths).map(|tp| crate::order::estimate_minutes_with(tp, tp.feed_xy, tp.feed_z, mo)).sum();
            let time = if mins < 1.0 { "under 1 min".to_string() } else if mins < 90.0 { format!("about {:.0} min", mins) } else { format!("about {:.1} h", mins / 60.0) };
            (mine.iter().map(|j| j.tool.as_str()).collect::<Vec<_>>().join(" + "), format!("{}  ·  {time}", paths.join(", ")))
        })
        .collect();
    ExportResult { files, safety, plan }
}

/// A user-typed export file name (Qt cleanFileName): no folders, the post's extension kept when none is typed.
pub fn clean_file_name(typed: &str, original: &str) -> String {
    let n: String = typed.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':') { '_' } else { c }).collect();
    if n.is_empty() {
        return original.to_string();
    }
    match original.rsplit_once('.') {
        Some((_, ext)) if !n.contains('.') => format!("{n}.{ext}"),
        _ => n,
    }
}
