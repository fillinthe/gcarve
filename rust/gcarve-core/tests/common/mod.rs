//! Shared by the integration tests: repo paths and the fixture jobs (same as gen_golden.cpp).
#![allow(dead_code)]

use gcarve_core::{Job, Move, MoveType, Toolpath};
use std::path::PathBuf;

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
pub fn golden(f: &str) -> String {
    let p = repo().join("rust/testdata/posts").join(f);
    String::from_utf8(std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}

pub const N: f64 = f64::NAN;
pub fn mv(kind: MoveType, x: f64, y: f64, z: f64, i: f64, j: f64) -> Move {
    let o = |v: f64| (!v.is_nan()).then_some(v);
    Move { kind, x: o(x), y: o(y), z: o(z), i, j }
}
pub fn r(x: f64, y: f64, z: f64) -> Move {
    mv(MoveType::Rapid, x, y, z, 0.0, 0.0)
}
pub fn l(x: f64, y: f64, z: f64) -> Move {
    mv(MoveType::Linear, x, y, z, 0.0, 0.0)
}

/// Keep in sync with fixtureJobs() in gen_golden.cpp
pub fn fixture_jobs() -> Vec<Job> {
    use MoveType::{ArcCcw as CCW, ArcCw as CW};
    let notes = vec!["AIR CUT TEST - NOT FOR CUTTING".to_string()];
    let ts = "09:09 PM, Tuesday December 17 2024".to_string();
    let material = "48.000\" x 24.000\" x 0.750\"".to_string();
    let p1 = Toolpath {
        name: "Profile 1 (outside)".into(),
        tool: -1,
        feed_xy: 432.0,
        feed_z: 216.0,
        moves: vec![
            r(N, N, 0.8),
            r(1.0, 1.0, N),
            l(N, N, -0.25),
            l(3.0, N, N),
            l(3.0, 2.0, -0.25),
            mv(CCW, 1.0, 2.0, N, -1.0, 0.0),
            mv(CW, 2.0, 3.0, N, 1.0, 0.0),
            mv(CCW, 2.0, 3.0, N, 0.0, -1.0),
            l(N, N, -0.5),
            l(2.5, 3.2, -0.5),
            l(2.123456789, 3.987654321, -0.5),
            r(N, N, 0.8),
        ],
    };
    let p2 = Toolpath {
        name: "Pocket inner".into(),
        tool: -1,
        feed_xy: 300.0,
        feed_z: 100.0,
        moves: vec![r(5.0, 5.0, N), l(N, N, -0.1), l(6.0, 5.0, N), l(N, 6.0, N), mv(CW, 5.0, 6.0, -0.2, -0.5, 0.0), l(5.0, 5.0, -0.2), r(N, N, 0.8)],
    };
    let a = Job {
        filename: "01_Profile_(outside)".into(),
        timestamp: ts.clone(),
        material: material.clone(),
        tool: "1/4 End Mill (0.25 inch)".into(),
        rpm: 18000,
        toolpaths: vec![p1, p2],
        notes: notes.clone(),
        ..Default::default()
    };
    let v = Toolpath {
        name: "V-Carve".into(),
        tool: -1,
        feed_xy: 120.5,
        feed_z: 60.25,
        moves: vec![r(0.0, 0.0, 0.8), l(N, N, -0.1), l(0.333333, 0.1, -0.15), l(1.0, 0.0, N), l(1.0, 0.0, -0.1), r(N, N, 0.5), r(10.0, 10.0, 0.5), l(10.0, 10.0, -0.05)],
    };
    let e = Toolpath { name: "Empty".into(), tool: -1, feed_xy: 50.0, feed_z: 25.0, moves: vec![] };
    let b = Job {
        filename: "02_V-Bit".into(),
        timestamp: ts,
        material,
        tool: "V-Bit (90 deg 0.5\")".into(),
        rpm: 16000,
        z_zero: "Table".into(),
        notes,
        toolpaths: vec![v, e],
        slot: 0,
    };
    vec![a, b]
}
