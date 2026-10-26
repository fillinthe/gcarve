//! End to end: samples/compare/compare.tlp -> regenerate -> WinCNC files, compared with what the C++ app exported
//! (samples/compare/gcarve/*.tap), ignoring the timestamp line.

use gcarve_core::pipeline::{self, active_post};
use gcarve_core::project::load_project;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn strip_timestamp(s: &str) -> String {
    s.lines().filter(|l| !(l.starts_with(" [") && l.contains(", ") && l.ends_with(" 2026]"))).collect::<Vec<_>>().join("\n")
}

#[test]
fn compare_project_exports_like_the_cpp_app() {
    let doc = load_project(&std::fs::read_to_string(root().join("samples/compare/compare.tlp")).unwrap()).unwrap();
    let gen = pipeline::regenerate(&doc);
    // the same single warning the C++ app shows for this project
    assert!(gen.warnings.iter().all(|w| w.contains("bit's V is only")), "{:?}", gen.warnings);
    let post = active_post(&doc, &|_| None);
    assert_eq!(post.id, "wincnc-single");
    let r = pipeline::export(&doc, &gen, "compare", "07:31 AM, Thursday October 8 2026", 0.0, &post);
    assert!(r.safety.is_empty(), "{:?}", r.safety);
    assert_eq!(r.files.len(), 2);
    for f in &r.files {
        let want = std::fs::read_to_string(root().join("samples/compare/gcarve").join(&f.filename)).unwrap_or_else(|_| panic!("missing C++ export {}", f.filename));
        assert_eq!(strip_timestamp(&f.text), strip_timestamp(&want), "{} differs from the C++ app's export", f.filename);
    }
    // air cut: every Z raised, names marked
    let air = pipeline::export(&doc, &gen, "compare", "t", 1.0625, &post);
    assert!(air.files.iter().all(|f| f.filename.contains("_AIRCUT")));
    assert!(air.files[0].text.contains("AIR CUT TEST"));
}

#[test]
fn machine_limits_clamp_feeds_and_flag_export() {
    use gcarve_core::document::MachineLimits;
    let mut doc = load_project(&std::fs::read_to_string(root().join("samples/compare/compare.tlp")).unwrap()).unwrap();
    let before = pipeline::tool_for_cut(&doc, 0);
    // a slow hobby machine: 10k RPM max, 100 in/min
    doc.material.machine = Some(MachineLimits { name: "Tiny".into(), rpm_min: 5000.0, rpm_max: 10000.0, feed_xy: 100.0, feed_z: 30.0, travel_x: 10.0, travel_y: 10.0, travel_z: 3.0, ..Default::default() });
    doc.material.auto_feeds = true;
    doc.tools[0].presets.clear();
    let t = pipeline::tool_for_cut(&doc, 0);
    assert!(t.rpm <= 10000 && t.feed_xy <= 100.0 && t.feed_z <= 30.0, "{} {} {}", t.rpm, t.feed_xy, t.feed_z);
    assert!(before.feed_xy > t.feed_xy);
    let gen = pipeline::regenerate(&doc);
    let issues = pipeline::machine_issues(&doc, &(0..doc.tools.len()).map(|i| pipeline::tool_for_cut(&doc, i)).collect::<Vec<_>>());
    assert!(issues.iter().any(|i| i.contains("bigger than the Tiny travel")), "{issues:?}");
    let r = pipeline::export(&doc, &gen, "x", "t", 0.0, &active_post(&doc, &|_| None));
    assert!(r.safety.iter().any(|i| i.contains("Tiny")));
    // maker chip loads drive the feed when the tool has them
    doc.material.machine = None;
    doc.tools[0].chip_loads.insert("plywood".into(), [0.002, 0.002]);
    doc.material.kind = "Plywood".into();
    let m = pipeline::tool_for_cut(&doc, 0);
    assert_eq!(m.feed_xy, (f64::from(m.rpm) * f64::from(m.flutes) * 0.002).round());
}

#[test]
fn tool_changer_slots_number_the_tool_changes_and_are_checked() {
    use gcarve_core::document::MachineLimits;
    use gcarve_core::project::{save_project, tool_from_json, tool_to_json};
    let mut doc = load_project(&std::fs::read_to_string(root().join("samples/compare/compare.tlp")).unwrap()).unwrap();
    // slots are saved only when set: old projects re-save as before
    assert!(!tool_to_json(&doc.tools[0]).to_string().contains("slot"));
    let mut used: Vec<usize> = doc.ops.iter().map(|o| o.tool as usize).collect();
    used.sort();
    used.dedup();
    assert!(used.len() >= 2, "the sample uses two bits");
    doc.tools[used[0]].slot = 7;
    doc.tools[used[1]].slot = 3;
    assert_eq!(tool_from_json(&tool_to_json(&doc.tools[used[0]])).slot, 7);
    assert!(save_project(&doc).contains("\"slot\""));
    doc.material.post = "wincnc-atc".into(); // a tool-change post: one combined file
    doc.material.machine = Some(MachineLimits { name: "ATC".into(), tool_slots: 8, ..Default::default() });
    let gen = pipeline::regenerate(&doc);
    let post = active_post(&doc, &|_| None);
    assert!(!post.tool_change.is_empty());
    let r = pipeline::export(&doc, &gen, "x", "t", 0.0, &post);
    assert_eq!(r.files.len(), 1);
    let t = &r.files[0].text;
    assert!(t.contains("T7 M6") && t.contains("T3 M6") && !t.contains("T1 M6") && !t.contains("T2 M6"), "{t}");
    assert!(r.safety.iter().all(|s| !s.contains("slot")), "{:?}", r.safety);
    // duplicate slot, then a rack too small
    doc.tools[used[1]].slot = 7;
    let r = pipeline::export(&doc, &gen, "x", "t", 0.0, &post);
    assert!(r.safety.iter().any(|s| s.contains("both in slot 7")), "{:?}", r.safety);
    doc.material.machine.as_mut().unwrap().tool_slots = 1;
    let r = pipeline::export(&doc, &gen, "x", "t", 0.0, &post);
    assert!(r.safety.iter().any(|s| s.contains("tool changer has 1 slots")) && r.safety.iter().any(|s| s.contains("beyond the 1-slot")), "{:?}", r.safety);
    assert_eq!(r.plan.len(), r.files.len()); // the export dialog's per-file tool / toolpaths line
}

#[test]
fn export_file_names_are_cleaned() {
    use gcarve_core::pipeline::clean_file_name;
    assert_eq!(clean_file_name("  ", "Job_1.tap"), "Job_1.tap"); // empty keeps the post's name
    assert_eq!(clean_file_name("sign", "Job_1.tap"), "sign.tap"); // extension kept
    assert_eq!(clean_file_name("sign.nc", "Job_1.tap"), "sign.nc");
    assert_eq!(clean_file_name("a/b\\c:d", "x.tap"), "a_b_c_d.tap"); // no folders
}

#[test]
fn feeds_scale_with_the_machine_class() {
    use gcarve_core::document::{Document, MachineLimits};
    let mut doc = Document::default();
    let t = doc.tools.iter().find(|t| t.kind == gcarve_core::document::ToolType::EndMill).cloned().unwrap();
    let none = pipeline::suggest(&doc, &t);
    let machine = |class: &str| MachineLimits { rpm_max: 24000.0, feed_xy: 1000.0, feed_z: 300.0, class: class.into(), ..Default::default() };
    doc.material.machine = Some(machine("industrial"));
    let ind = pipeline::suggest(&doc, &t);
    doc.material.machine = Some(machine("hobby"));
    let hob = pipeline::suggest(&doc, &t);
    doc.material.machine = Some(machine(""));
    let unknown = pipeline::suggest(&doc, &t);
    // same spindle and limits: the hobby router gets ~0.35x the chip load and half the pass depth
    assert_eq!(ind.rpm, hob.rpm);
    assert!((hob.chip_load / ind.chip_load - 0.35).abs() < 0.02, "{} vs {}", hob.chip_load, ind.chip_load);
    assert!((hob.stepdown / ind.stepdown - 0.5).abs() < 0.02, "{} vs {}", hob.stepdown, ind.stepdown);
    assert_eq!((unknown.feed_xy, unknown.stepdown), (ind.feed_xy, ind.stepdown));
    // the meter agrees: the hobby suggestion sits in the sweet spot for a hobby machine
    doc.material.machine = Some(machine("hobby"));
    let b = pipeline::assess(&doc, &t, hob.feed_xy, hob.feed_z, hob.rpm, hob.stepdown);
    assert_eq!(b.level, 0, "{b:?}");
    let _ = none;
}

/// Tiled export of the showcase (12 in board in 5 in tiles, 1 in overlap) through a few posts, read back: every file
/// stays inside its tile, never rapids sideways in the material, reaches the job's depth somewhere and stops the spindle.
#[test]
fn tiled_export_stays_in_its_tile_and_is_safe() {
    use gcarve_core::gcode::parse_program;
    use gcarve_core::gimport::read_program;
    use gcarve_core::MoveType;
    let doc = load_project(&std::fs::read_to_string(root().join("samples/showcase.tlp")).unwrap()).unwrap();
    let gen = pipeline::regenerate(&doc);
    let tiling = Some(pipeline::Tiling { along_x: true, len: 5.0, overlap: 1.0 });
    for id in ["wincnc-single", "grbl", "mach3"] {
        let post = gcarve_core::postdef::post_library().iter().find(|p| p.id.starts_with(id)).unwrap_or_else(|| panic!("post {id}"));
        let plain = pipeline::export(&doc, &gen, "t", "d", 0.0, post);
        let r = pipeline::export_tiled(&doc, &gen, "t", "d", 0.0, post, tiling);
        assert!(r.files.len() > plain.files.len(), "{id}: {} tiled files vs {}", r.files.len(), plain.files.len());
        let mut deepest = 0.0_f64;
        for f in &r.files {
            assert!(f.filename.contains("_Tile"), "{}", f.filename);
            let prog = read_program(&f.text, "t", &doc.material);
            assert!(!prog.sections.is_empty(), "{}: nothing read back", f.filename);
            for s in &prog.sections {
                let p = parse_program(&s.gcode, 300.0);
                assert!(p.errors.is_empty(), "{}: {:?}", f.filename, p.errors);
                for mv in p.motions.iter().filter(|m| !m.machine_coords) {
                    let (a, b) = (mv.from, mv.to);
                    if mv.kind != MoveType::Rapid {
                        assert!(b.x >= -0.01 && b.x <= 5.01, "{}: line {} cuts at x {:.3}, outside the 5 in tile", f.filename, mv.line, b.x);
                        deepest = deepest.min(b.z);
                    } else if (a.x - b.x).abs() > 1e-6 || (a.y - b.y).abs() > 1e-6 {
                        assert!(a.z >= -1e-6 && b.z >= -1e-6, "{}: line {}: sideways rapid in the material", f.filename, mv.line);
                    }
                }
            }
            let lines: Vec<&str> = f.text.lines().collect();
            let last_cut = lines.iter().rposition(|l| l.trim_start().starts_with("G1") || l.trim_start().starts_with("G01")).unwrap_or(0);
            assert!(lines[last_cut..].iter().any(|l| l.split_whitespace().any(|w| w == "M5" || w == "M05")), "{}: spindle not stopped at the end", f.filename);
        }
        assert!((deepest + 0.75).abs() < 0.01, "{id}: deepest cut {deepest}");
    }
    // XY zero anywhere else: refused with a note, files unchanged
    let mut centred = doc.clone();
    centred.material.origin = 4;
    centred.sync_sheet();
    let gen = pipeline::regenerate(&centred);
    let post = &gcarve_core::postdef::post_library()[0];
    let r = pipeline::export_tiled(&centred, &gen, "t", "d", 0.0, post, tiling);
    assert!(r.safety.iter().any(|s| s.contains("NOT tiled")) && r.files.iter().all(|f| !f.filename.contains("_Tile")));
}

#[test]
fn toolpaths_near_a_keep_out_shape_are_flagged() {
    use gcarve_core::geom::make_rect;
    let mut doc = load_project(&std::fs::read_to_string(root().join("samples/compare/compare.tlp")).unwrap()).unwrap();
    let gen = pipeline::regenerate(&doc);
    let tools: Vec<_> = doc.tools.clone();
    assert!(pipeline::keep_out_issues(&doc, &tools, &gen.paths).is_empty());
    // a clamp over the whole board: every cutting toolpath comes down in it
    let l = doc.layer_named("Clamps");
    doc.layers[l as usize].keep_out = true;
    doc.add(make_rect(-1.0, -1.0, doc.material.w + 2.0, doc.material.h + 2.0), Some(l));
    let iss = pipeline::keep_out_issues(&doc, &tools, &gen.paths);
    assert!(!iss.is_empty() && iss[0].contains("keep-out area (layer \"Clamps\")"), "{iss:?}");
    // a screw far off the board: nothing; the flag survives save / load
    let far = load_project(&gcarve_core::project::save_project(&doc)).unwrap();
    assert!(far.layers[l as usize].keep_out);
    let mut doc2 = far.clone();
    for s in doc2.shapes.iter_mut().filter(|s| s.layer == l) {
        s.c = make_rect(500.0, 500.0, 501.0, 501.0); // two corners
    }
    let i2 = pipeline::keep_out_issues(&doc2, &tools, &gen.paths);
    assert!(i2.is_empty(), "{i2:?} {:?}", doc2.shapes.iter().filter(|s| s.layer == l).map(|s| gcarve_core::geom::bounds(&s.c)).collect::<Vec<_>>());
}

#[test]
fn small_parts_cut_free_without_tabs_are_flagged() {
    use gcarve_core::document::{Op, OpType, Side};
    use gcarve_core::geom::make_rect;
    let mut doc = load_project(&std::fs::read_to_string(root().join("samples/compare/compare.tlp")).unwrap()).unwrap();
    let a = doc.add(make_rect(30.0, 1.0, 32.0, 3.0), None); // 2 x 2 in
    let b = doc.add(make_rect(30.0, 5.0, 40.0, 15.0), None); // 10 x 10 in
    let op = |ids: Vec<i32>, tabs: bool| Op { name: "Cut".into(), kind: OpType::Profile, side: Side::Outside, depth: doc.material.t, tabs, shape_ids: ids, ..Default::default() };
    doc.ops.push(op(vec![a, b], false));
    let iss = pipeline::order_issues(&doc);
    assert!(iss.iter().any(|s| s.contains("cuts 1 small part free")), "{iss:?}");
    doc.ops.last_mut().unwrap().tabs = true;
    assert!(!pipeline::order_issues(&doc).iter().any(|s| s.contains("small part")));
}

#[test]
fn only_the_wanted_toolpaths_are_worked_out() {
    let doc = load_project(&std::fs::read_to_string(root().join("samples/showcase.tlp")).unwrap()).unwrap();
    let all = pipeline::regenerate(&doc);
    let some = pipeline::regenerate_some(&doc, &|i| i == 1);
    assert_eq!(some.paths.len(), doc.ops.len());
    assert!(!all.paths[1].is_empty());
    assert_eq!(format!("{:?}", some.paths[1]), format!("{:?}", all.paths[1]), "a wanted toolpath is the same as in a full run");
    assert_eq!(some.minutes[1], all.minutes[1]);
    assert!(some.paths.iter().enumerate().all(|(i, p)| i == 1 || p.is_empty()) && some.minutes.iter().enumerate().all(|(i, m)| i == 1 || *m == 0.0));
}

#[test]
fn a_ghost_run_never_spins_and_stays_above_the_board() {
    use gcarve_core::gimport::read_program;
    let doc = load_project(&std::fs::read_to_string(root().join("samples/showcase.tlp")).unwrap()).unwrap();
    let gen = pipeline::regenerate(&doc);
    for id in ["wincnc-single", "grbl", "mach3", "shopbot"] {
        let post = gcarve_core::postdef::post_library().iter().find(|p| p.id.starts_with(id)).unwrap_or_else(|| panic!("post {id}"));
        let real = pipeline::export_tiled_with(&doc, &gen, "t", "d", 0.0, post, None, false);
        let ghost = pipeline::export_tiled_with(&doc, &gen, "t", "d", 0.0, post, None, true);
        assert_eq!(real.files.len(), ghost.files.len(), "{id}: one ghost per real file");
        for (r, g) in real.files.iter().zip(&ghost.files) {
            assert!(g.filename.contains("_GHOST"), "{}", g.filename);
            let spins = |t: &str| t.lines().any(|l| { let w = l.split_whitespace().collect::<Vec<_>>(); w.iter().any(|x| matches!(x.to_ascii_uppercase().as_str(), "M3" | "M03" | "M4" | "M04" | "C6")) });
            assert!(spins(&r.text) || id == "shopbot", "{id}: the real file turns the spindle on");
            assert!(!spins(&g.text), "{id}: the ghost turns the spindle on:\n{}", g.text.lines().take(12).collect::<Vec<_>>().join("\n"));
            // no Z below the board top anywhere in the ghost (a Z word with a minus), while the real file cuts below it
            let below = |t: &str| t.lines().filter(|l| !l.starts_with('(') && !l.starts_with('[') && !l.starts_with(';')).any(|l| l.split_whitespace().any(|w| w.starts_with("Z-") || w.starts_with("z-")));
            assert!(below(&r.text) || id == "shopbot", "{id}: the real file never goes below the board?");
            assert!(!below(&g.text), "{id}: the ghost goes below the board top");
            if id != "shopbot" {
                assert!(!read_program(&g.text, "t", &doc.material).sections.is_empty(), "{id}: the ghost reads back"); // ShopBot files aren't G-code
            }
        }
    }
}

#[test]
fn a_wide_holder_near_a_clamp_is_flagged_where_the_bare_bit_clears() {
    use gcarve_core::geom::make_rect;
    let mut doc = load_project(&std::fs::read_to_string(root().join("samples/compare/compare.tlp")).unwrap()).unwrap();
    let gen = pipeline::regenerate(&doc);
    let mut tools: Vec<_> = doc.tools.clone();
    // find the closest approach: put a small clamp 0.3 in outside the board's left edge, below every cut
    let l = doc.layer_named("Clamps");
    doc.layers[l as usize].keep_out = true;
    doc.add(make_rect(-1.3, 0.0, -0.3, doc.material.h), Some(l));
    for t in tools.iter_mut() {
        t.stick_out = 0.5;
        t.holder_dia = t.diameter;
    }
    let before = pipeline::keep_out_issues(&doc, &tools, &gen.paths);
    for t in tools.iter_mut() {
        t.holder_dia = 4.0; // a router holder: reaches over the clamp from the board's edge
    }
    let after = pipeline::keep_out_issues(&doc, &tools, &gen.paths);
    assert!(after.len() > before.len(), "{before:?} / {after:?}");
    // bits that stick out 3 in keep the holder above clamps only 0.5 in tall: only the bare bit counts again
    doc.layers[l as usize].clamp_height = 0.5;
    tools.iter_mut().for_each(|t| t.stick_out = 3.0);
    let low = pipeline::keep_out_issues(&doc, &tools, &gen.paths);
    assert_eq!(low.len(), before.len(), "{before:?} / {low:?}");
}
