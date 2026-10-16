//! Cross-check against the C++ engine: rust/testdata/posts/* were written by rust/testdata/posts/gen/gen_golden.cpp
//! (rebuild with gen/build.sh). Posts must match byte for byte; arcfit and the G-code reader to 1e-9.

use gcarve_core::arcfit::{fit_arcs, DEFAULT_MAX_SWEEP, DEFAULT_TOL};
use gcarve_core::gcode::{motion_points, parse_program};
use gcarve_core::postdef::{post_from_json, post_from_vectric_pp, post_jobs, post_library, PostOutput};
use gcarve_core::{Move, MoveType, Toolpath};
use std::f64::consts::PI;

mod common;
use common::{fixture_jobs, golden, repo};

fn check_outputs(id: &str, outs: &[PostOutput], index: &str, failures: &mut Vec<String>) {
    let want: Vec<(&str, &str)> = index.lines().filter_map(|ln| ln.split_once(' ').filter(|(i, _)| *i == id).map(|(_, rest)| rest.split_once(' ').unwrap())).collect();
    if want.len() != outs.len() {
        failures.push(format!("{id}: {} files, C++ wrote {}", outs.len(), want.len()));
        return;
    }
    for ((gold, fname), o) in want.iter().zip(outs) {
        let g = golden(gold);
        if o.filename != *fname {
            failures.push(format!("{id}: filename {:?} vs C++ {fname:?}", o.filename));
        }
        if o.text != g {
            let i = o.text.bytes().zip(g.bytes()).take_while(|(a, b)| a == b).count();
            let ctx = |s: &str| s.get(i.saturating_sub(40)..(i + 40).min(s.len())).unwrap_or("").to_string();
            failures.push(format!("{gold}: first difference at byte {i}\n  C++:  {:?}\n  Rust: {:?}", ctx(&g), ctx(&o.text)));
        }
    }
}

#[test]
fn every_builtin_post_matches_cpp_byte_for_byte() {
    let index = golden("index.txt");
    let jobs = fixture_jobs();
    // the 47 posts the C++ app has; later Rust-only posts are covered by ported.rs (round trip + spindle safety)
    let cpp: Vec<_> = post_library().iter().filter(|d| index.lines().any(|l| l.split(' ').next() == Some(d.id.as_str()))).collect();
    assert_eq!(cpp.len(), 47);
    let mut failures = Vec::new();
    for d in cpp {
        check_outputs(&d.id, &post_jobs(&jobs, d), &index, &mut failures);
    }
    for pp in ["test-router", "mach3-like"] {
        let d = post_from_vectric_pp(&golden(&format!("{pp}.pp"))).unwrap();
        // the C++ translation, read back through our JSON reader, is the same PostDef
        assert_eq!(post_from_json(&golden(&format!("{pp}.json"))).unwrap(), d, "{pp}.pp translation differs from C++");
        check_outputs(&format!("pp-{pp}"), &post_jobs(&jobs, &d), &index, &mut failures);
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}

/// token-wise comparison: numbers within 1e-9 (relative above 1), everything else exact
fn same_dump(name: &str, cpp: &str, rust: &str) {
    let (a, b): (Vec<_>, Vec<_>) = (cpp.lines().collect(), rust.lines().collect());
    assert_eq!(a.len(), b.len(), "{name}: line count");
    for (k, (la, lb)) in a.iter().zip(&b).enumerate() {
        let (ta, tb): (Vec<_>, Vec<_>) = (la.split_whitespace().collect(), lb.split_whitespace().collect());
        let ok = ta.len() == tb.len()
            && ta.iter().zip(&tb).all(|(x, y)| match (x.parse::<f64>(), y.parse::<f64>()) {
                (Ok(p), Ok(q)) => (p.is_nan() && q.is_nan()) || p == q || (p - q).abs() <= 1e-9 * p.abs().max(1.0),
                _ => x.eq_ignore_ascii_case(y),
            });
        assert!(ok, "{name} line {}:\n  C++:  {la}\n  Rust: {lb}", k + 1);
    }
}

fn opt(v: Option<f64>) -> String {
    v.map_or("-".into(), |v| v.to_string())
}
fn dump_tp(tp: &Toolpath) -> String {
    let mut o = format!("toolpath {}\n", tp.moves.len());
    for m in &tp.moves {
        o += &format!("{} {} {} {} {} {}\n", m.kind as i32, opt(m.x), opt(m.y), opt(m.z), m.i, m.j);
    }
    o
}

#[test]
fn arcfit_matches_cpp() {
    let input = golden("arcfit_in.txt");
    let mut tps: Vec<Toolpath> = Vec::new();
    for ln in input.lines() {
        if ln.starts_with("toolpath") {
            tps.push(Toolpath { name: String::new(), tool: -1, ..Default::default() });
            continue;
        }
        let t: Vec<&str> = ln.split_whitespace().collect();
        let f = |s: &str| (s != "-").then(|| s.parse::<f64>().unwrap());
        let kind = [MoveType::Rapid, MoveType::Linear, MoveType::ArcCw, MoveType::ArcCcw][t[0].parse::<usize>().unwrap()];
        tps.last_mut().unwrap().moves.push(Move { kind, x: f(t[1]), y: f(t[2]), z: f(t[3]), i: f(t[4]).unwrap(), j: f(t[5]).unwrap() });
    }
    assert_eq!(tps.len(), 6);
    let out: String = tps.iter().map(|tp| dump_tp(&fit_arcs(tp, DEFAULT_TOL, DEFAULT_MAX_SWEEP)) + &dump_tp(&fit_arcs(tp, 0.0001, PI))).collect();
    same_dump("arcfit_out.txt", &golden("arcfit_out.txt"), &out);
    assert!(out.lines().any(|l| l.starts_with("3 ")) && out.lines().any(|l| l.starts_with("2 ")), "fixture should produce both arc directions");
}

#[test]
fn gcode_reader_matches_cpp() {
    let files = [
        ("compare_01_1_4_End_Mill", repo().join("samples/compare/grbl/compare_01_1_4_End_Mill.nc")),
        ("compare_02_V_Bit_90_deg_0_5in", repo().join("samples/compare/grbl/compare_02_V_Bit_90_deg_0_5in.nc")),
        ("01_V90", repo().join("reference/01_V90.tap")),
        ("02_EMD14", repo().join("reference/02_EMD14.tap")),
        ("misc", repo().join("rust/testdata/posts/misc.nc")),
    ];
    let b = |v: bool| i32::from(v);
    for (name, path) in files {
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
        let p = parse_program(&text, 1000.0);
        let mut o = format!("lines {}\nerrors {}\n", p.lines.len(), p.errors.len());
        for e in &p.errors {
            o += &format!("{e}\n");
        }
        o += &format!("motions {}\n", p.motions.len());
        for m in &p.motions {
            o += &format!(
                "{} {} {} {} {} {} {} {} {} {} {} {} {} {}\n",
                m.kind as i32,
                m.from.x,
                m.from.y,
                m.from.z,
                m.to.x,
                m.to.y,
                m.to.z,
                m.i,
                m.j,
                m.feed,
                b(m.machine_coords),
                b(m.spindle_on),
                m.line,
                motion_points(m, 0.002).len()
            );
        }
        o += &format!("min {} {} {}\nmax {} {} {}\n", p.min.x, p.min.y, p.min.z, p.max.x, p.max.y, p.max.z);
        o += &format!("spindleOffAtEnd {}\nminutes {}\n", b(p.spindle_off_at_end), p.minutes);
        same_dump(name, &golden(&format!("gcode_{name}.txt")), &o);
    }
}
