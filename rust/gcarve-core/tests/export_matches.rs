//! "What is exported is what is expected": the toolpaths the app generated (what the preview and the simulation show)
//! against the G-code files read back through the interpreter, for real drawings. Per post: no read errors, no NaN,
//! the spindle off at the end, and the XY length of all cutting moves, their bounding box and the deepest Z equal the
//! generated ones (arc fitting may shorten corners a hair: 0.5 %). Needs drawings:
//! `GCARVE_EXPORT_DIR=/Volumes/home/_Projects cargo test --release -p gcarve-core --test export_matches -- --ignored --nocapture`
//! (every .dxf below it; without the variable only the built-in showcase runs).

use gcarve_core::document::{default_tools, Op, OpType, Side, ToolType, Document};
use gcarve_core::gcode::parse_program;
use gcarve_core::geom::bounds;
use gcarve_core::gimport::read_program;
use gcarve_core::pipeline;
use gcarve_core::postdef::post_library;
use gcarve_core::project::load_project;
use gcarve_core::MoveType;
use std::path::{Path, PathBuf};

fn dxfs(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            dxfs(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dxf")) {
            out.push(p);
        }
    }
}

/// a drawing with a profile, a pocket and a V-carve on every closed shape (board grown to hold it)
fn job_for(text: &str) -> Option<Document> {
    let r = gcarve_core::dxf::import_dxf(text);
    if r.contours.is_empty() {
        return None;
    }
    let mut doc = Document::default();
    doc.tools = default_tools();
    let (mut x1, mut y1, mut x0, mut y0) = (f64::MIN, f64::MIN, f64::MAX, f64::MAX);
    for c in &r.contours {
        let b = bounds(c);
        (x0, y0, x1, y1) = (x0.min(b.x0), y0.min(b.y0), x1.max(b.x1), y1.max(b.y1));
    }
    // shift to the board's corner, board = drawing + 2 in each side
    let (dx, dy) = (2.0 - x0, 2.0 - y0);
    for (c, layer) in r.contours.into_iter().zip(r.layers) {
        let l = doc.layer_named(&layer);
        doc.add(gcarve_core::geom::transformed(&c, &gcarve_core::geom::Affine::translate(dx, dy)), Some(l));
    }
    doc.material.w = (x1 - x0 + 4.0).max(12.0);
    doc.material.h = (y1 - y0 + 4.0).max(12.0);
    let closed: Vec<i32> = doc.shapes.iter().filter(|s| s.c.closed).map(|s| s.id).collect();
    if closed.is_empty() {
        return None;
    }
    let find = |k: ToolType| doc.tools.iter().position(|t| t.kind == k).unwrap_or(0) as i32;
    let (em, vb) = (find(ToolType::EndMill), find(ToolType::VBit));
    // pocket and V-carve the three largest shapes (a real job doesn't carve all of 145)
    let mut by_size: Vec<(f64, i32)> = doc.shapes.iter().filter(|s| s.c.closed).map(|s| { let b = bounds(&s.c); ((b.x1 - b.x0) * (b.y1 - b.y0), s.id) }).collect();
    by_size.sort_by(|a, b| b.0.total_cmp(&a.0));
    let big: Vec<i32> = by_size.iter().take(3).map(|x| x.1).collect();
    doc.ops.push(Op { kind: OpType::Profile, name: "Cut out".into(), tool: em, side: Side::Outside, depth: doc.material.t, tabs: true, shape_ids: closed, ..Default::default() });
    doc.ops.push(Op { kind: OpType::Pocket, name: "Pocket".into(), tool: em, depth: 0.2, shape_ids: big.clone(), ..Default::default() });
    doc.ops.push(Op { kind: OpType::VCarve, name: "V-carve".into(), tool: vb, depth: 0.2, shape_ids: big, ..Default::default() });
    Some(doc)
}

/// WinCNC comments are [ ... ]: the interpreter reads ( ) and ; comments only
fn no_brackets(t: &str) -> String {
    let mut depth = 0;
    t.chars().filter(|&c| { if c == '[' { depth += 1; } let keep = depth == 0; if c == ']' && depth > 0 { depth -= 1; } keep }).collect()
}

fn arc_len(from: (f64, f64), to: (f64, f64), i: f64, j: f64, cw: bool) -> f64 {
    let (cx, cy) = (from.0 + i, from.1 + j);
    let r = (from.0 - cx).hypot(from.1 - cy);
    let (a0, a1) = ((from.1 - cy).atan2(from.0 - cx), (to.1 - cy).atan2(to.0 - cx));
    let mut d = if cw { a0 - a1 } else { a1 - a0 };
    d = d.rem_euclid(std::f64::consts::TAU);
    if d < 1e-9 && ((from.0 - to.0).abs() + (from.1 - to.1).abs()) < 1e-6 {
        d = std::f64::consts::TAU;
    }
    r * d
}

/// (cut length XY, bbox x0 y0 x1 y1, deepest z) of generated toolpaths
fn generated(gens: &[&pipeline::Generated]) -> (f64, [f64; 4], f64) {
    let (mut len, mut bb, mut zmin) = (0.0, [f64::MAX, f64::MAX, f64::MIN, f64::MIN], 0.0_f64);
    for tp in gens.iter().flat_map(|g| g.paths.iter().flatten()) {
        let (mut x, mut y) = (0.0, 0.0);
        for m in &tp.moves {
            let (nx, ny) = (m.x.unwrap_or(x), m.y.unwrap_or(y));
            if m.kind != MoveType::Rapid {
                len += (nx - x).hypot(ny - y);
                bb = [bb[0].min(nx), bb[1].min(ny), bb[2].max(nx), bb[3].max(ny)];
            }
            if let Some(z) = m.z {
                zmin = zmin.min(z);
            }
            (x, y) = (nx, ny);
        }
    }
    (len, bb, zmin)
}

#[test]
#[ignore] // minutes with a drawing folder: see the header
fn exported_gcode_matches_the_generated_toolpaths() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut docs: Vec<(String, Document)> = vec![("showcase".into(), load_project(&std::fs::read_to_string(root.join("samples/showcase.tlp")).unwrap()).unwrap())];
    if let Ok(dir) = std::env::var("GCARVE_EXPORT_DIR") {
        let mut files = Vec::new();
        dxfs(Path::new(&dir), &mut files);
        for f in files {
            let bytes = std::fs::read(&f).unwrap_or_default();
            if std::env::var("GCARVE_EXPORT_ONLY").is_ok_and(|o| !f.display().to_string().contains(&o)) {
                continue;
            }
            if let Some(d) = job_for(&String::from_utf8_lossy(&bytes)) {
                docs.push((f.display().to_string(), d));
            }
        }
    }
    let posts: Vec<_> = post_library().iter().filter(|p| ["wincnc-single", "grbl", "mach3", "linuxcnc", "fusion-mm", "grbl-mm"].iter().any(|id| p.id.contains(id))).take(5).collect();
    assert!(!posts.is_empty());
    let mut problems: Vec<String> = Vec::new();
    for (name, doc) in &docs {
        eprintln!("start {name}");
        let t0 = std::time::Instant::now();
        let gen = if std::env::var("GCARVE_EXPORT_OPS").is_ok() { // time each toolpath alone
            for op in &doc.ops {
                let mut one = doc.clone();
                one.ops = vec![op.clone()];
                let t = std::time::Instant::now();
                let _ = pipeline::regenerate(&one);
                eprintln!("   {:>6.1}s {}", t.elapsed().as_secs_f64(), op.name);
            }
            pipeline::regenerate(doc)
        } else {
            pipeline::regenerate(doc)
        };
        eprintln!("{:>6.1}s generate  {} shapes  {name}", t0.elapsed().as_secs_f64(), doc.shapes.len());
        // the files cover every sheet; `gen` is the active one
        let others: Vec<pipeline::Generated> = (0..doc.sheets.len() as i32)
            .filter(|&k| k != doc.active_sheet)
            .map(|k| {
                let mut d = doc.clone();
                d.set_active_sheet(k);
                d.apply_bindings(); // as the export does for the other sheets
                pipeline::regenerate(&d)
            })
            .collect();
        let sheets: Vec<&pipeline::Generated> = others.iter().chain(std::iter::once(&gen)).collect();
        let (glen, gbb, gz) = generated(&sheets);
        if glen <= 0.0 {
            problems.push(format!("{name}: no cutting moves generated ({:?})", gen.warnings.first()));
            continue;
        }
        for post in &posts {
            let res = pipeline::export(doc, &gen, "match", "2026-10-14", 0.0, post);
            if res.files.is_empty() {
                problems.push(format!("{name} / {}: no files", post.id));
            }
            let (mut rlen, mut rbb, mut rz, mut ends_off) = (0.0, [f64::MAX, f64::MAX, f64::MIN, f64::MIN], 0.0_f64, true);
            for f in &res.files {
                let last_section_off = parse_program(&no_brackets(&f.text), 300.0).spindle_off_at_end; // the whole file: sections drop the spindle lines
                if f.text.contains("NaN") || f.text.contains("inf") || f.text.trim().is_empty() {
                    problems.push(format!("{name} / {}: {} has NaN / inf / is empty", post.id, f.filename));
                }
                let prog = read_program(&f.text, "m", &doc.material);
                for s in &prog.sections {
                    let p = parse_program(&s.gcode, 300.0);
                    for e in p.errors.iter().take(2) {
                        problems.push(format!("{name} / {}: {}: {e}", post.id, f.filename));
                    }
                    for mv in p.motions.iter().filter(|m| !m.machine_coords) {
                        rz = rz.min(mv.to.z);
                        if mv.kind == MoveType::Rapid {
                            continue;
                        }
                        let cw = mv.kind == MoveType::ArcCw;
                        rlen += if matches!(mv.kind, MoveType::ArcCw | MoveType::ArcCcw) { arc_len((mv.from.x, mv.from.y), (mv.to.x, mv.to.y), mv.i, mv.j, cw) } else { (mv.to.x - mv.from.x).hypot(mv.to.y - mv.from.y) };
                        rbb = [rbb[0].min(mv.to.x), rbb[1].min(mv.to.y), rbb[2].max(mv.to.x), rbb[3].max(mv.to.y)];
                    }
                }
                ends_off &= last_section_off;
            }
            let who = format!("{name} / {}", post.id);
            if std::env::var("GCARVE_EXPORT_DEBUG").is_ok() && post.id == "wincnc-single" {
                for g in &sheets {
                    for tp in g.paths.iter().flatten() {
                        let l = generated(&[&pipeline::Generated { paths: vec![vec![tp.clone()]], warnings: vec![], minutes: vec![], draft_tool: -1 }]);
                        eprintln!("  gen  {:>9.2}  tool {:>2}  {}", l.0, tp.tool, tp.name);
                    }
                }
                for f in &res.files {
                    let prog = read_program(&f.text, "m", &doc.material);
                    for s in &prog.sections {
                        let p = parse_program(&s.gcode, 300.0);
                        let l: f64 = p.motions.iter().filter(|m| m.kind != MoveType::Rapid && !m.machine_coords).map(|m| (m.to.x - m.from.x).hypot(m.to.y - m.from.y)).sum();
                        eprintln!("  read {l:>9.2}  {}  off={}  {}", f.filename, p.spindle_off_at_end, s.name);
                    }
                }
            }
            if !ends_off {
                if let Some(f) = res.files.first() {
                    eprintln!("tail of {}: {:?}", f.filename, f.text.lines().rev().take(6).collect::<Vec<_>>());
                }
                problems.push(format!("{who}: a file does not end with the spindle off"));
            }
            if (rlen - glen).abs() > glen * 0.005 + 0.01 {
                problems.push(format!("{who}: cut length read {rlen:.3} vs generated {glen:.3}"));
            }
            if rbb.iter().zip(&gbb).any(|(a, b)| (a - b).abs() > 0.003) {
                problems.push(format!("{who}: cut extents read {rbb:.3?} vs generated {gbb:.3?}"));
            }
            if (rz - gz).abs() > 0.002 {
                problems.push(format!("{who}: deepest Z read {rz:.4} vs generated {gz:.4}"));
            }
        }
    }
    println!("{} drawings x {} posts", docs.len(), posts.len());
    problems.dedup();
    for p in problems.iter().take(60) {
        println!("PROBLEM {p}");
    }
    assert!(problems.is_empty(), "{} problem(s), first: {}", problems.len(), problems[0]);
}
