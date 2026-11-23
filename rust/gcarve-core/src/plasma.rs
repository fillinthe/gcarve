//! Plasma cutting (plan: docs/PLASMA-PLAN-2026-10.md). A plasma torch is a tool (`ToolType::Plasma`): its diameter
//! is the kerf, its feed the cut speed, plus pierce height, pierce delay and cut height. A Profile with it is one
//! pass at cut height with a lead-in in the waste, and its G-code is written here, not by the router templates.
//!
//! SAFETY (the torch rule, the plasma twin of the spindle rule): the torch fires only at a pierce point, after the
//! move there; it is switched off at the end of every cut before anything else moves; no rapid is ever written
//! with the torch on; every file ends torch off. `post` is built so it cannot do otherwise (one place switches the
//! torch on, and every rapid, toolpath end and file end goes through `off`), and the test checks every style.
//!
//! The styles follow the makers' published formats (Langmuir FireControl, LinuxCNC QtPlasmaC, Mach3 / Mach4 and
//! grblHAL with torch on M3). None has been run on a table yet: try a file in the air first.

use crate::cam::{generate_profile, CamResult, Entry, Material, Op, Tool};
use crate::geom::Contour;
use crate::wincnc::num;
use crate::job::{Job, MoveType};
use crate::postdef::{PostDef, PostOutput};

/// The cut: the Profile's outline offset by half the kerf (outside / inside; on the line for marking), one pass at
/// cut height, lead-in / lead-out arcs in the waste (`Op::ramp_in`, 0 = 2 kerfs, at least 0.12 in) so the pierce
/// mark stays off the part. Depth, passes, tabs and ramps mean nothing here and are ignored.
/// The toolpath itself is drawn as a cut right through the sheet (Z = −thickness), which is what the kerf is: the
/// simulation, the time estimate and the board checks then work as for any through cut. The torch's real heights
/// are written by `post` from the tool, never from these Z values.
pub fn generate_cut(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    let one_pass = Op { depth: m.t, stepdown: 0.0, entry: Entry::Plunge, tabs: false, tab_count: 0, tab_points: Vec::new(), skin: 0.0, allowance: 0.0, ramp_in: if op.ramp_in > 0.0 { op.ramp_in } else { (2.0 * tool.diameter).max(0.12) }, cutter_comp: false, keyhole: false, ..op.clone() };
    let torch = Tool { stepdown: m.t + 1.0, feed_z: tool.feed_xy.max(1.0), ..tool.clone() };
    let mut res = generate_profile(&one_pass, &torch, contours, m);
    if tool.cut_height <= 0.0 || tool.pierce_height < tool.cut_height {
        res.warnings.push(format!("{}: the torch needs a cut height above the sheet and a pierce height at least as high (Tool Library)", op.name));
    }
    res
}

/// A plasma toolpath with a router post, or a router bit with a plasma post (export safety list)
pub fn post_issues(jobs: &[Job], def: &PostDef) -> Vec<String> {
    let mut out = Vec::new();
    if def.plasma.is_empty() && jobs.iter().any(|j| j.plasma.is_some()) {
        out.push(format!("Plasma cuts need a plasma G-code style (Job Setup): “{}” is for routers. The torch's files are written in the plain plasma style (M3 / M5) instead.", def.name));
    }
    if !def.plasma.is_empty() && jobs.iter().any(|j| j.plasma.is_none() && !j.toolpaths.is_empty()) {
        out.push(format!("“{}” is a plasma G-code style: toolpaths with router bits are left out of its files.", def.name));
    }
    out
}

/// One file per job in the post's plasma style (`def.plasma`: firecontrol | qtplasmac | grblhal | anything else =
/// plain M3 / M5 as Mach3). Jobs without a torch get a file that only says so.
pub fn post(jobs: &[Job], def: &PostDef) -> Vec<PostOutput> {
    jobs.iter().map(|j| PostOutput { filename: format!("{}.{}", j.filename, def.extension), text: file(j, def) }).collect()
}

fn file(j: &Job, def: &PostDef) -> String {
    let style = def.plasma.as_str();
    let (k, dec) = (if def.inches { 1.0 } else { 25.4 }, def.decimals.max(0) as usize);
    let n = |v: f64| num(v * k, dec);
    let mut out = String::new();
    let mut line = |s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    if style == "firecontrol" {
        line("(v1.6-af)".into()); // FireControl refuses a file without its version tag on the first line
    }
    line(format!("({})", j.filename));
    let Some([pierce_h, delay, cut_h]) = j.plasma else {
        line("(Nothing written: this G-code style is for plasma torches, and this toolpath uses a router bit)".into());
        line("M5".into());
        line("M30".into());
        return out;
    };
    // heights are above the sheet: with Z zero on the table the sheet's top is its thickness up
    let top = if j.z_zero == "Table" { j.board[2] } else { 0.0 };
    let (pierce_h, cut_h) = (pierce_h + top, cut_h + top);
    line(format!("(Torch: {})", j.tool.replace(['(', ')'], "")));
    line(format!("G90 {}", if def.inches { "G20" } else { "G21" }));
    let safe = j.safe_z.max(pierce_h);
    let qt = style == "qtplasmac";
    let feed = j.toolpaths.first().map_or(0.0, |t| t.feed_xy);
    if qt {
        // a temporary material: the controller runs the whole height sequence itself from these
        line(format!("(o=0, kw={}, ph={}, pd={}, ch={}, fr={})", n(j.kerf), n(pierce_h - top), num(delay, 2), n(cut_h - top), num(feed * k, 0)));
        line(format!("F{}", num(feed * k, 0)));
    }
    for note in &j.notes {
        line(format!("({})", note.replace(['(', ')'], "")));
    }
    // air-cut test / ghost run: the same moves at the travel height, and no line that fires the torch
    let dry = j.torch_off;
    if dry {
        line("(DRY RUN: the torch stays off)".into());
        if !qt {
            line(format!("G0 Z{}", n(safe)));
        }
    }
    let mut on = false;
    let (mut x, mut y) = (f64::NAN, f64::NAN);
    // the one way the torch goes off: called before every rapid, at every toolpath end and at the file end
    let off = |on: &mut bool, line: &mut dyn FnMut(String)| {
        if *on && dry {
            *on = false;
        }
        if *on {
            if style == "firecontrol" {
                line("H0".into());
            }
            line(if qt { "M5 $0".into() } else { "M5".into() });
            if !qt {
                line(format!("G0 Z{}", n(safe)));
            }
            *on = false;
        }
    };
    for tp in &j.toolpaths {
        line(format!("({})", tp.name.replace(['(', ')'], "")));
        for mv in &tp.moves {
            let (nx, ny) = (mv.x.unwrap_or(x), mv.y.unwrap_or(y));
            let sideways = (nx - x).abs() > 1e-9 || (ny - y).abs() > 1e-9 || x.is_nan();
            if mv.kind == MoveType::Rapid {
                off(&mut on, &mut line);
                if sideways && !nx.is_nan() && !ny.is_nan() {
                    line(format!("G0 X{} Y{}", n(nx), n(ny)));
                }
            } else if !nx.is_nan() && !ny.is_nan() {
                if !on {
                    // pierce here: the only place the torch is switched on
                    if x.is_nan() {
                        line(format!("G0 X{} Y{}", n(nx), n(ny)));
                    }
                    match style {
                        _ if dry => {}
                        "qtplasmac" => line("M3 $0 S1".into()),
                        _ => {
                            line(format!("G0 Z{}", n(pierce_h)));
                            line(if style == "grblhal" { "M3 S1000".into() } else { "M3".into() });
                            line(format!("G4 P{}", num(delay, 2)));
                            line(format!("G1 Z{} F{}", n(cut_h), num(tp.feed_z * k, 0)));
                            if style == "firecontrol" {
                                line("H1".into());
                            }
                        }
                    }
                    on = true;
                }
                // arcs as arcs (the export fits them to the toolpath): I / J from the arc's start, as every style here reads them
                if sideways {
                    let word = match mv.kind {
                        MoveType::ArcCw => "G2",
                        MoveType::ArcCcw => "G3",
                        _ => "G1",
                    };
                    let centre = if word == "G1" { String::new() } else { format!(" I{} J{}", n(mv.i), n(mv.j)) };
                    line(format!("{word} X{} Y{}{centre} F{}", n(nx), n(ny), num(tp.feed_xy * k, 0)));
                }
            }
            (x, y) = (nx, ny);
        }
        off(&mut on, &mut line);
    }
    off(&mut on, &mut line);
    line("M5".into());
    line("M30".into());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cam::{OpType, Side, ToolType};
    use crate::geom::make_rect;

    fn torch() -> Tool {
        Tool { name: "45 A, 14 ga mild steel".into(), kind: ToolType::Plasma, diameter: 0.06, feed_xy: 120.0, rpm: 0, pierce_height: 0.15, pierce_delay: 0.5, cut_height: 0.06, ..Default::default() }
    }

    /// a plate with a window: two cuts (window inside, plate outside), each offset by half the kerf, one pass at
    /// cut height, the pierce point off the line in the waste
    #[test]
    fn a_plasma_profile_is_one_pass_at_cut_height_offset_by_half_the_kerf_with_the_pierce_in_the_waste() {
        let m = Material { w: 24.0, h: 24.0, t: 0.075, ..Default::default() };
        let t = torch();
        let outside = Op { kind: OpType::Profile, name: "Plate".into(), side: Side::Outside, depth: 0.5, tabs: true, tab_count: 4, ..Default::default() };
        let r = generate_cut(&outside, &t, &[make_rect(4.0, 4.0, 14.0, 10.0)], &m);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let moves: Vec<_> = r.paths.iter().flat_map(|p| &p.moves).collect();
        let cut: Vec<_> = moves.iter().filter(|mv| mv.kind != MoveType::Rapid).collect();
        assert!(cut.iter().all(|mv| mv.z.is_none_or(|z| (z + 0.075).abs() < 1e-9)), "one pass, drawn right through the sheet");
        assert_eq!(moves.iter().filter(|mv| mv.kind != MoveType::Rapid && mv.x.is_none() && mv.z.is_some()).count().max(1), 1, "one pass: one pierce");
        // the cut runs half a kerf outside the 10 × 6 plate, and the pierce is further out still
        let xs: Vec<f64> = cut.iter().filter_map(|mv| mv.x).collect();
        let (lo, hi) = (xs.iter().copied().fold(f64::MAX, f64::min), xs.iter().copied().fold(f64::MIN, f64::max));
        assert!(lo < 4.0 - 0.03 + 1e-6 && hi > 14.0 + 0.03 - 1e-6, "{lo} .. {hi}");
        let first = cut.iter().find(|mv| mv.x.is_some() && mv.y.is_some()).unwrap();
        let (px, py) = (first.x.unwrap(), first.y.unwrap());
        let outside_by = (3.97 - px).max(px - 14.03).max(3.97 - py).max(py - 10.03);
        assert!(outside_by > 0.05, "the pierce at {px}, {py} is {outside_by} outside the cut line");
    }

    fn sample_job() -> Job {
        let m = Material { w: 24.0, h: 24.0, t: 0.075, ..Default::default() };
        let t = torch();
        let mut paths = Vec::new();
        for (side, c) in [(Side::Inside, make_rect(6.0, 6.0, 9.0, 8.0)), (Side::Outside, make_rect(4.0, 4.0, 14.0, 10.0))] {
            paths.extend(generate_cut(&Op { kind: OpType::Profile, name: "Cut".into(), side, ..Default::default() }, &t, &[c], &m).paths);
        }
        Job { filename: "01_plate".into(), tool: t.name.clone(), toolpaths: paths, plasma: Some([0.15, 0.5, 0.06]), kerf: 0.06, safe_z: 0.8, ..Default::default() }
    }

    /// THE TORCH RULE, for every plasma style: on only after arriving at a pierce point, a delay after it (the
    /// controller's own in QtPlasmaC), off before every rapid, nothing lower than the cut height, the file ends off
    #[test]
    fn no_plasma_style_ever_moves_at_rapid_with_the_torch_on_and_every_file_ends_torch_off() {
        let j = sample_job();
        let styles: Vec<&PostDef> = crate::postdef::post_library().iter().filter(|d| !d.plasma.is_empty()).collect();
        assert!(styles.len() >= 4, "{} plasma styles", styles.len());
        for def in styles {
            let text = &post(std::slice::from_ref(&j), def)[0].text;
            let k = if def.inches { 1.0 } else { 25.4 };
            let (mut on, mut fires, mut cuts) = (false, 0, 0);
            let lines: Vec<&str> = text.lines().collect();
            for (i, l) in lines.iter().enumerate() {
                let word = |c: char| l.split_whitespace().find(|w| w.starts_with(c)).and_then(|w| w[1..].parse::<f64>().ok());
                if *l == "M3" || l.starts_with("M3 ") {
                    assert!(!on, "{}: fired twice: line {i}", def.id);
                    (on, fires) = (true, fires + 1);
                    if def.plasma != "qtplasmac" {
                        assert!(lines[i + 1].starts_with("G4 P"), "{}: no pierce delay after M3", def.id);
                        assert!(lines[i - 1].starts_with("G0 Z"), "{}: not at pierce height before M3", def.id);
                    }
                } else if l.starts_with("M5") {
                    on = false;
                } else if l.starts_with("G0") {
                    assert!(!on, "{}: a rapid with the torch on: line {i} {l}", def.id);
                } else if l.starts_with("G1") {
                    assert!(on, "{}: a cutting move with the torch off: line {i} {l}", def.id);
                    cuts += usize::from(word('X').is_some());
                    assert!(word('Z').is_none_or(|z| (z - 0.06 * k).abs() < 1e-3), "{}: a cutting move not at the cut height: {l}", def.id);
                }
            }
            assert!(!on && fires == 2 && cuts > 8, "{}: on {on}, {fires} pierces, {cuts} cuts", def.id);
            let tail: Vec<&str> = lines.iter().rev().take(2).copied().collect();
            assert_eq!(tail, ["M30", "M5"], "{}", def.id);
            assert_eq!(text.starts_with("(v1.6-af)"), def.plasma == "firecontrol", "{}", def.id);
            if std::env::var("GCARVE_PLASMA_PRINT").is_ok_and(|v| v == def.id) {
                println!("{}", text.lines().take(24).collect::<Vec<_>>().join("\n"));
            }
        }
    }

    /// a round hole through the real export (which fits arcs): it comes out round, as G2 / G3 with the centre, not as
    /// the chords between the arcs' ends; and with Z zero on the table the torch heights sit above the sheet's top
    #[test]
    fn round_holes_stay_round_and_heights_follow_the_z_zero() {
        use crate::document::Document;
        let mut d = Document::default();
        (d.material.w, d.material.h, d.material.t) = (12.0, 12.0, 0.25);
        d.material.post = "plasma-mach".into();
        d.tools = vec![torch()];
        let ring: Vec<_> = (0..64).map(|k| (k as f64 / 64.0 * std::f64::consts::TAU).sin_cos()).map(|(s, c)| crate::geom::pt(6.0 + c, 6.0 + s)).collect();
        let hole = d.add(crate::geom::make_polyline(&ring, true), None);
        d.ops.push(Op { kind: OpType::Profile, name: "Hole".into(), side: Side::Inside, shape_ids: vec![hole], ..Default::default() });
        d.sync_sheet();
        let post = crate::postdef::find_post("plasma-mach").unwrap();
        let export = |d: &Document| crate::pipeline::export(d, &crate::pipeline::regenerate(d), "p", "", 0.0, post).files[0].text.clone();
        let text = export(&d);
        let word = |l: &str, c: char| l.split_whitespace().find(|w| w.starts_with(c)).and_then(|w| w[1..].parse::<f64>().ok());
        // every arc's centre (start + I, J) is the hole's centre and its end is on the cut circle (radius 1 − half kerf)
        let (mut x, mut y, mut arcs) = (0.0, 0.0, 0);
        for l in text.lines().filter(|l| l.starts_with('G')) {
            let (nx, ny) = (word(l, 'X').unwrap_or(x), word(l, 'Y').unwrap_or(y));
            if l.starts_with("G2") || l.starts_with("G3") {
                let (cx, cy) = (x + word(l, 'I').unwrap(), y + word(l, 'J').unwrap());
                if (cx - 6.0).hypot(cy - 6.0) < 1e-3 {
                    arcs += 1;
                    assert!(((nx - 6.0).hypot(ny - 6.0) - 0.97).abs() < 2e-3, "{l}");
                }
            }
            (x, y) = (nx, ny);
        }
        assert!(arcs >= 3, "the hole is cut as arcs round its centre: {arcs}\n{text}");
        assert!(text.contains("G0 Z0.1500") && text.contains("G1 Z0.0600"));
        d.material.z_bottom = true;
        d.sync_sheet();
        let text = export(&d);
        assert!(text.contains("G0 Z0.4000") && text.contains("G1 Z0.3100") && !text.contains("Z0.0600"), "{text}");
    }

    #[test]
    fn a_router_post_never_writes_a_plasma_job_and_a_plasma_post_never_a_router_job() {
        let j = sample_job();
        let router = crate::postdef::find_post("grbl").or_else(|| crate::postdef::post_library().iter().find(|d| d.plasma.is_empty() && d.id != "wincnc-single")).unwrap();
        // through the one export entry: the plasma job comes out in the plain torch style, never as spindle code
        let text = &crate::postdef::post_jobs(std::slice::from_ref(&j), router)[0].text;
        assert!(text.contains("G4 P0.50") && !text.contains("S18000") && text.trim_end().ends_with("M30"));
        assert!(post_issues(std::slice::from_ref(&j), router)[0].contains("plasma G-code style"));
        // a router bit's job in the same export keeps its own file, spindle and all
        let bit = Job { plasma: None, rpm: 18000, filename: "02_bit".into(), ..j.clone() };
        let both = crate::postdef::post_jobs(&[j.clone(), bit], router);
        assert_eq!(both.len(), 2);
        // in the jobs' own order: the torch's file first here, then the bit's
        assert!(both[0].text.contains("G4 P0.50") && !both[0].text.contains("S18000"), "{}", both[0].text);
        assert!(both[1].text.contains("S18000") && both[1].text.contains("G1") && !both[1].text.contains("G4 P0.50"), "{}", both[1].text);
        let plasma = crate::postdef::post_library().iter().find(|d| d.plasma == "firecontrol").unwrap();
        // an air-cut test or a ghost run of a plasma job never fires the torch, in any style
        for def in crate::postdef::post_library().iter().filter(|d| !d.plasma.is_empty()) {
            for ghost in [false, true] {
                let mut dry = vec![j.clone()];
                if ghost { crate::cam::make_ghost(&mut dry, 1.0) } else { crate::cam::make_air_cut(&mut dry, 1.0) }
                let text = &crate::postdef::post_jobs(&dry, def)[0].text;
                assert!(!text.lines().any(|l| l == "M3" || l.starts_with("M3 ")) && !text.contains("G4") && text.contains("DRY RUN") && text.lines().filter(|l| l.starts_with("G1 X")).count() > 8, "{}: {text}", def.id);
            }
        }
        let bit = Job { plasma: None, ..j.clone() };
        let text = &crate::postdef::post_jobs(std::slice::from_ref(&bit), plasma)[0].text;
        assert!(!text.contains("G1") && !text.lines().any(|l| l == "M3" || l.starts_with("M3 ")) && text.contains("M5"));
        assert!(post_issues(std::slice::from_ref(&bit), plasma)[0].contains("left out"));
    }

    /// a whole drawing through the real export: a plate with a window on a FireControl table
    #[test]
    fn a_plasma_drawing_exports_through_the_normal_export_and_survives_a_save() {
        use crate::document::Document;
        let mut d = Document::default();
        (d.material.w, d.material.h, d.material.t, d.material.kind) = (24.0, 24.0, 0.075, "Mild steel".into());
        d.material.post = "plasma-firecontrol".into();
        d.tools = vec![torch()];
        let plate = d.add(make_rect(4.0, 4.0, 14.0, 10.0), None);
        let window = d.add(make_rect(6.0, 6.0, 9.0, 8.0), None);
        d.ops.push(Op { kind: OpType::Profile, name: "Window".into(), side: Side::Inside, shape_ids: vec![window], ..Default::default() });
        d.ops.push(Op { kind: OpType::Pocket, name: "No".into(), depth: 0.05, shape_ids: vec![window], ..Default::default() });
        d.ops.push(Op { kind: OpType::Profile, name: "Plate".into(), side: Side::Outside, shape_ids: vec![plate], ..Default::default() });
        d.sync_sheet();
        let d = crate::project::load_project(&crate::project::save_project(&d)).unwrap();
        assert_eq!((d.tools[0].kind, d.tools[0].pierce_delay, d.tools[0].cut_height), (ToolType::Plasma, 0.5, 0.06));
        let gen = crate::pipeline::regenerate(&d);
        assert_eq!(gen.warnings, ["No: a plasma torch only cuts Profile toolpaths"]);
        assert!(gen.minutes[0] > 0.0 && gen.minutes[2] > gen.minutes[0]);
        let post = crate::postdef::find_post("plasma-firecontrol").unwrap();
        let out = crate::pipeline::export(&d, &gen, "plate", "", 0.0, post);
        assert!(out.safety.is_empty(), "{:?}", out.safety);
        let text = &out.files[0].text;
        assert_eq!(text.lines().filter(|l| *l == "M3").count(), 2, "{text}");
        assert!(text.starts_with("(v1.6-af)") && text.trim_end().ends_with("M5\nM30"));
        // the same drawing sent to a router's G-code style is flagged, and still comes out torch-safe
        let router = crate::postdef::find_post("grbl").unwrap_or(&crate::postdef::post_library()[1]);
        let out = crate::pipeline::export(&d, &gen, "plate", "", 0.0, router);
        assert!(out.safety.iter().any(|s| s.contains("plasma G-code style")), "{:?}", out.safety);
        assert!(out.files[0].text.contains("G4 P0.50"));
    }
}
