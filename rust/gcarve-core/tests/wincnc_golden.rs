//! Golden test (port of tests/post_golden.cpp): parse each reference/*.tap (VCarve output from the shop's Panther)
//! into a Job, write it again with post_wincnc, require byte equality (modulo the trailing newline).

use gcarve_core::{post_wincnc, Job, Move, MoveType, Toolpath};
use std::path::PathBuf;

fn ref_dir() -> PathBuf {
    std::env::var("GCARVE_REFERENCE_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn between<'a>(l: &'a str, pre: &str) -> &'a str {
    match (l.find(pre), l.rfind(']')) {
        (Some(a), Some(b)) if a + pre.len() <= b => &l[a + pre.len()..b],
        _ => "",
    }
}

fn parse(text: &str) -> Job {
    let mut j = Job::default();
    let mut bracket_line = 0;
    for raw in text.lines() {
        let l = raw.trim_end_matches('\r').trim_start_matches(' ');
        if l.is_empty() {
            continue;
        }
        if l.starts_with("[FILENAME: ") {
            j.filename = between(l, "[FILENAME: ").into();
            bracket_line = 1;
        } else if bracket_line == 1 {
            j.timestamp = l[1..l.len() - 1].into();
            bracket_line = 2;
        } else if l.starts_with("[MATERIAL size: ") {
            j.material = between(l, "[MATERIAL size: ").into();
        } else if l.starts_with('[') && j.tool.is_empty() && bracket_line == 2 && !l.contains("MATERIAL") && !l.contains("Z Zero") {
            j.tool = l[1..l.len() - 1].into();
        } else if l.starts_with("[TOOLPATH NAME: ") {
            j.toolpaths.push(Toolpath { name: between(l, "[TOOLPATH NAME: ").into(), ..Default::default() });
        } else if j.toolpaths.is_empty() {
            continue;
        } else if let Some(rest) = l.strip_prefix('F') {
            let v: f64 = rest.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0.0);
            let tp = j.toolpaths.last_mut().unwrap();
            if l.contains(" XY") { tp.feed_xy = v } else { tp.feed_z = v }
        } else if l.starts_with('G') && !l.starts_with("G5") && !l.starts_with("G9") {
            let kind = match &l[1..2] { "0" => MoveType::Rapid, "1" => MoveType::Linear, "2" => MoveType::ArcCw, _ => MoveType::ArcCcw };
            let mut m = Move { kind, ..Default::default() };
            for w in l[2..].split_whitespace() {
                let v: f64 = w[1..].parse().unwrap_or(0.0);
                match &w[..1] { "X" => m.x = Some(v), "Y" => m.y = Some(v), "Z" => m.z = Some(v), "I" => m.i = v, "J" => m.j = v, _ => {} }
            }
            j.toolpaths.last_mut().unwrap().moves.push(m);
        }
    }
    j
}

#[test]
fn reference_files_round_trip_byte_for_byte() {
    for f in ["01_V90.tap", "02_EMD14.tap"] {
        let want = std::fs::read(ref_dir().join(f)).expect("reference file");
        let want = String::from_utf8_lossy(&want).trim_end_matches(['\r', '\n']).to_string();
        let got = post_wincnc(&parse(&want));
        let got = got.trim_end_matches(['\r', '\n']);
        if want != got {
            let i = want.bytes().zip(got.bytes()).take_while(|(a, b)| a == b).count();
            panic!("{f}: first difference at byte {i}\n want: {:?}\n got:  {:?}", &want[i..(i + 60).min(want.len())], &got[i..(i + 60).min(got.len())]);
        }
    }
}
