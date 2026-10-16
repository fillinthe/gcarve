//! Golden cross-check against the C++ engine: rust/testdata/cam/*.json were written by rust/testdata/cam/gen/dump.cpp
//! (C++ core). Every generator runs on the same inputs here and must give the same moves (type and count equal,
//! coordinates within 1e-4 in), the same warnings, safety lines and WinCNC files.

use gcarve_core::cam::{self, generate, CamResult, Material, Op, Tool};
use gcarve_core::geom::{pt, Contour, Node};
use gcarve_core::relief::{generate_relief, load_stl, rasterize_relief, tip_height, Relief};
use gcarve_core::sim::{shade, simulate};
use gcarve_core::{post_wincnc, Move, MoveType, Toolpath};
use serde_json::Value;
use std::path::PathBuf;

const EPS: f64 = 1e-4;

fn data(name: &str) -> Value {
    // GCARVE_CAM_GOLDEN_DIR: compare against another dump (e.g. the FMA-on app library, see gen/build.sh)
    let dir = std::env::var("GCARVE_CAM_GOLDEN_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testdata/cam"));
    let p = dir.join(name);
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}
fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}
fn of<T: serde::de::DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap()
}
fn contour(v: &Value) -> Contour {
    let nodes = v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| {
            let a: Vec<f64> = n.as_array().unwrap().iter().map(f).collect();
            Node { p: pt(a[0], a[1]), inh: pt(a[2], a[3]), out: pt(a[4], a[5]), has_in: a[6] != 0.0, has_out: a[7] != 0.0 }
        })
        .collect();
    Contour { nodes, closed: v["closed"].as_bool().unwrap() }
}
fn contours(v: &Value) -> Vec<Contour> {
    v.as_array().unwrap().iter().map(contour).collect()
}
fn path(v: &Value) -> Toolpath {
    let kind = |k: u64| [MoveType::Rapid, MoveType::Linear, MoveType::ArcCw, MoveType::ArcCcw][k as usize];
    let moves = v["moves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let a = m.as_array().unwrap();
            Move { kind: kind(a[0].as_u64().unwrap()), x: a[1].as_f64(), y: a[2].as_f64(), z: a[3].as_f64(), i: f(&a[4]), j: f(&a[5]) }
        })
        .collect();
    Toolpath { name: v["name"].as_str().unwrap().into(), tool: v["tool"].as_i64().unwrap() as i32, feed_xy: f(&v["feed_xy"]), feed_z: f(&v["feed_z"]), moves }
}
fn paths(v: &Value) -> Vec<Toolpath> {
    v.as_array().unwrap().iter().map(path).collect()
}
fn strs(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

/// Max coordinate difference over all moves, or why the paths don't match move-for-move.
fn compare(cpp: &[Toolpath], rust: &[Toolpath]) -> Result<f64, String> {
    if cpp.len() != rust.len() {
        return Err(format!("{} paths (C++) vs {} (Rust)", cpp.len(), rust.len()));
    }
    let mut worst: f64 = 0.0;
    for (k, (a, b)) in cpp.iter().zip(rust).enumerate() {
        if a.name != b.name || a.tool != b.tool || a.feed_xy != b.feed_xy || a.feed_z != b.feed_z {
            return Err(format!("path {k}: header {:?}/{}/{}/{} vs {:?}/{}/{}/{}", a.name, a.tool, a.feed_xy, a.feed_z, b.name, b.tool, b.feed_xy, b.feed_z));
        }
        if a.moves.len() != b.moves.len() {
            return Err(format!("path {k} ({}): {} moves (C++) vs {} (Rust)", a.name, a.moves.len(), b.moves.len()));
        }
        for (i, (p, q)) in a.moves.iter().zip(&b.moves).enumerate() {
            let d = |u: Option<f64>, v: Option<f64>| match (u, v) {
                (Some(u), Some(v)) => Ok((u - v).abs()),
                (None, None) => Ok(0.0),
                _ => Err(()),
            };
            let diff = [d(p.x, q.x), d(p.y, q.y), d(p.z, q.z), Ok((p.i - q.i).abs()), Ok((p.j - q.j).abs())];
            if p.kind != q.kind || diff.iter().any(|r| r.is_err()) {
                return Err(format!("path {k} move {i}: {p:?} vs {q:?}"));
            }
            let m = diff.iter().map(|r| *r.as_ref().unwrap()).fold(0.0, f64::max);
            if m > EPS {
                return Err(format!("path {k} move {i}: off by {m:e}: {p:?} vs {q:?}"));
            }
            worst = worst.max(m);
        }
    }
    Ok(worst)
}

fn check_result(cpp_paths: &Value, cpp_warn: &Value, r: &CamResult) -> Result<f64, String> {
    if strs(cpp_warn) != r.warnings {
        return Err(format!("warnings {:?} vs {:?}", strs(cpp_warn), r.warnings));
    }
    compare(&paths(cpp_paths), &r.paths)
}

#[test]
fn generators_match_cpp() {
    let cases = data("cases.json");
    let (mut report, mut failed) = (Vec::new(), Vec::new());
    for c in cases.as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let op: Op = of(&c["op"]);
        let tool: Tool = of(&c["tool"]);
        let clear: Option<Tool> = if c["clear_tool"].is_null() { None } else { Some(of(&c["clear_tool"])) };
        let m: Material = of(&c["material"]);
        let cs = contours(&c["contours"]);
        let raw = match op.kind {
            cam::OpType::Pocket => cam::generate_pocket(&op, &tool, &cs, &m),
            cam::OpType::Drill => cam::generate_drill(&op, &tool, &cs, &m),
            cam::OpType::VCarve => cam::generate_vcarve(&op, &tool, &cs, &m, clear.as_ref()),
            cam::OpType::Bore => cam::generate_bore(&op, &tool, &cs, &m),
            cam::OpType::Inlay => cam::generate_inlay(&op, &tool, &cs, &m, clear.as_ref()),
            cam::OpType::Flute => cam::generate_flute(&op, &tool, &cs, &m),
            cam::OpType::Chamfer => cam::generate_chamfer(&op, &tool, &cs, &m),
            cam::OpType::Prism => cam::generate_prism(&op, &tool, &cs, &m),
            _ => cam::generate_profile(&op, &tool, &cs, &m),
        };
        let r = generate(&op, &tool, &cs, &m, clear.as_ref());
        // export pipeline on the same tools
        let tools: Vec<Tool> = of(&c["tools"]);
        let ops = [op.clone()];
        let gen = [r.paths.clone()];
        let taps: Vec<String> = cam::build_jobs(&ops, &tools, &gen, &m, "t").iter().map(post_wincnc).collect();
        let checks = (|| {
            let d_raw = check_result(&c["raw"], &c["raw_warnings"], &raw).map_err(|e| format!("raw: {e}"))?;
            let d = check_result(&c["paths"], &c["warnings"], &r).map_err(|e| format!("generate: {e}"))?;
            if strs(&c["safety"]) != cam::safety_issues(&ops, &tools, &gen, &m) {
                return Err("safety lines differ".to_string());
            }
            if strs(&c["taps"]) != taps {
                return Err("WinCNC output differs".to_string());
            }
            Ok((d_raw, d))
        })();
        let moves: usize = r.paths.iter().map(|p| p.moves.len()).sum();
        match checks {
            Ok((d_raw, d)) => report.push(format!("{name:40} {moves:6} moves  max diff raw {d_raw:.1e} generate {d:.1e}")),
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    println!("{}", report.join("\n"));
    assert!(failed.is_empty(), "{} of {} cases differ from C++:\n{}", failed.len(), cases.as_array().unwrap().len(), failed.join("\n"));
}

#[test]
fn compare_sample_pipeline_matches_cpp() {
    let c = data("compare.json");
    let m: Material = of(&c["material"]);
    let tools: Vec<Tool> = of(&c["tools"]);
    let ops: Vec<Op> = of(&c["ops"]);
    let mut gen = Vec::new();
    for (k, op) in ops.iter().enumerate() {
        let cs = contours(&c["layers"][k]);
        let r = generate(op, &tools[op.tool as usize], &cs, &m, None);
        let d = compare(&paths(&c["generated"][k]), &r.paths).unwrap_or_else(|e| panic!("compare {}: {e}", op.name));
        println!("compare {:8} max diff {d:.1e}", op.name);
        gen.push(r.paths);
    }
    let jobs = cam::build_jobs(&ops, &tools, &gen, &m, "09:09 PM, Tuesday December 17 2024");
    let taps: Vec<String> = jobs.iter().map(post_wincnc).collect();
    let want = strs(&c["taps"]);
    assert_eq!(want.len(), taps.len());
    for (a, b) in want.iter().zip(&taps) {
        assert!(a == b, "WinCNC output differs from C++ (first diff at line {:?})", a.lines().zip(b.lines()).position(|(x, y)| x != y));
    }
    let mut air = jobs.clone();
    cam::make_air_cut(&mut air, cam::air_cut_raise(&m));
    assert_eq!(strs(&c["aircut_taps"]), air.iter().map(post_wincnc).collect::<Vec<_>>());
    let tbl = Material { z_bottom: true, origin: 4, ..m.clone() };
    assert_eq!(strs(&c["table_centre_taps"]), cam::build_jobs(&ops, &tools, &gen, &tbl, "t").iter().map(post_wincnc).collect::<Vec<_>>());
    assert_eq!(strs(&c["safety"]), cam::safety_issues(&ops, &tools, &gen, &m));
}

#[test]
fn relief_and_sim_match_cpp() {
    let c = data("relief_sim.json");
    let stl = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/pyramid.stl")).unwrap();
    let tris = load_stl(&stl).unwrap();
    assert_eq!(tris.len() as u64, c["triangles"].as_u64().unwrap());
    let mut r = Relief { x: 1.0, y: 1.0, w: 2.0, h: 2.0, depth: 0.3, ..Default::default() };
    rasterize_relief(&tris, &mut r, 40000);
    assert_eq!((r.nx as i64, r.ny as i64), (c["nx"].as_i64().unwrap(), c["ny"].as_i64().unwrap()));
    let h: Vec<f32> = c["height"].as_array().unwrap().iter().map(|v| f(v) as f32).collect();
    assert_eq!(h, r.height, "rasterized heights");
    let mut rm = Relief { mirror: true, w: 3.0, ..r.clone() };
    rasterize_relief(&tris, &mut rm, 5000);
    let hm: Vec<f32> = c["mirror_height"].as_array().unwrap().iter().map(|v| f(v) as f32).collect();
    assert_eq!(hm, rm.height, "mirrored heights");

    let (ball, flat, vee): (Tool, Tool, Tool) = (of(&c["ball"]), of(&c["flat"]), of(&c["vee"]));
    let mut tips = Vec::new();
    let mut x = 0.8;
    while x <= 3.2 {
        let mut y = 0.9;
        while y <= 3.1 {
            for t in [&ball, &flat, &vee] {
                tips.push(tip_height(&r, t, x, y, 0.01));
            }
            y += 0.35;
        }
        x += 0.3;
    }
    let want: Vec<f64> = c["tips"].as_array().unwrap().iter().map(f).collect();
    assert_eq!(want.len(), tips.len());
    let dt = want.iter().zip(&tips).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(dt < 1e-12, "tip heights off by {dt:e}"); // last-ulp differences in libm hypot/sqrt only

    let (op, op_y): (Op, Op) = (of(&c["op"]), of(&c["op_y"]));
    let m: Material = of(&c["material"]);
    let bnd = contours(&c["boundary"]);
    let a = generate_relief(&op, &ball, Some(&flat), &r, &m, None);
    assert_eq!(strs(&c["warnings"]), a.warnings);
    let d1 = compare(&paths(&c["rough_finish"]), &a.paths).unwrap_or_else(|e| panic!("relief rough+finish: {e}"));
    let b = generate_relief(&op, &ball, None, &r, &m, Some(&bnd));
    let d2 = compare(&paths(&c["bounded"]), &b.paths).unwrap_or_else(|e| panic!("relief bounded: {e}"));
    let cc = generate_relief(&op_y, &vee, None, &rm, &m, None);
    let d3 = compare(&paths(&c["vee_y_mirror"]), &cc.paths).unwrap_or_else(|e| panic!("relief vee/y/mirror: {e}"));
    println!("relief max diff {d1:.1e} {d2:.1e} {d3:.1e}");

    let sm: Material = of(&c["sim_material"]);
    let sp = paths(&c["sim_paths"]);
    let st: Vec<Tool> = of(&c["sim_tools"]);
    let pairs: Vec<(&Toolpath, &Tool)> = sp.iter().zip(&st).collect();
    let hf = simulate(&pairs, &sm, 20000);
    let s = &c["sim"];
    assert_eq!((hf.nx as i64, hf.ny as i64), (s["nx"].as_i64().unwrap(), s["ny"].as_i64().unwrap()));
    assert_eq!((hf.res, hf.x0, hf.y0), (f(&s["res"]), f(&s["x0"]), f(&s["y0"])));
    let depth: Vec<f32> = s["depth"].as_array().unwrap().iter().map(|v| f(v) as f32).collect();
    assert_eq!(depth, hf.depth, "sim depths");
    let img: Vec<u8> = s["img"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u8).collect();
    assert_eq!(img, shade(&hf, &sm), "shaded image");
}
