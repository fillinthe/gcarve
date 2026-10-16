//! Export audit: the showcase project (every toolpath type, 2 sheets) plus a texture and a photo carve, posted
//! through every built-in post, then read back (gimport + the G-code interpreter) and checked:
//! spindle off at the end and before every tool change, every cut has a feed, no sideways rapid inside the
//! material, nothing deeper than the board + 1/16 in or outside it, arcs consistent, and the depths read back
//! equal the generated ones (units right in mm posts).
//! `cargo test -p gcarve-core --test export_audit -- --nocapture` prints the per-post table.

use gcarve_core::document::{Document, Op, OpType};
use gcarve_core::gcode::parse_program;
use gcarve_core::gimport::read_program;
use gcarve_core::pipeline;
use gcarve_core::postdef::post_library;
use gcarve_core::project::load_project;
use gcarve_core::MoveType;
use std::path::PathBuf;

fn showcase() -> Document {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut doc = load_project(&std::fs::read_to_string(root.join("samples/showcase.tlp")).unwrap()).unwrap();
    // a closed shape on the active sheet for the two newest toolpath kinds
    let id = doc.shapes.iter().find(|s| s.c.closed && s.sheet == doc.active_sheet).map(|s| s.id).unwrap();
    let ball = doc.tools.iter().position(|t| t.kind == gcarve_core::document::ToolType::BallNose).unwrap_or(0) as i32;
    let vbit = doc.tools.iter().position(|t| t.kind == gcarve_core::document::ToolType::VBit).unwrap_or(0) as i32;
    doc.ops.push(Op { kind: OpType::Texture, name: "Texture".into(), shape_ids: vec![id], tool: ball, depth: 0.05, ramp_length: 0.5, stepover: 0.8, vary: 0.5, ..Default::default() });
    let px: Vec<u8> = (0..64 * 48).map(|i| ((i % 64) * 4) as u8).collect(); // left dark, right light
    doc.ops.push(Op { kind: OpType::Photo, name: "Photo".into(), shape_ids: vec![id], tool: vbit, depth: 0.06, photo: gcarve_core::photo::b64_encode(&px), photo_w: 64, photo_h: 48, ..Default::default() });
    doc
}

/// commands that start / stop the spindle in any built-in dialect
fn spindle_on(line: &str) -> bool {
    let u = line.to_ascii_uppercase();
    let code = u.split(['(', '[', ';']).next().unwrap_or("");
    code.split_whitespace().any(|w| matches!(w, "M3" | "M03" | "M4" | "M04")) || code.contains("M3S") || code.contains("M03S") || code.starts_with("SO,1,1")
}
fn spindle_off(line: &str) -> bool {
    let u = line.to_ascii_uppercase();
    let code = u.split(['(', '[', ';']).next().unwrap_or("");
    code.split_whitespace().any(|w| matches!(w, "M5" | "M05")) || code.starts_with("SO,1,0") || code.trim() == "C7" || code.contains("!MC0")
}
fn tool_change(line: &str) -> bool {
    let u = line.to_ascii_uppercase();
    let code = u.split(['(', '[', ';']).next().unwrap_or("");
    code.split_whitespace().any(|w| matches!(w, "M6" | "M06"))
}
fn cuts(line: &str) -> bool {
    let u = line.to_ascii_uppercase();
    let code = u.split(['(', '[', ';']).next().unwrap_or("").replace(' ', "");
    ["G1", "G01", "G2", "G02", "G3", "G03"].iter().any(|g| code.starts_with(g) && !code[g.len()..].starts_with(|c: char| c.is_ascii_digit())) || code.starts_with("M3,") || code.starts_with("MX,") || code.starts_with("MZ,")
}

#[test]
#[ignore] // ~20 s in release: cargo test -p gcarve-core --release --test export_audit -- --ignored --nocapture
fn every_post_exports_safe_readable_gcode() {
    let doc = showcase();
    let gen = pipeline::regenerate(&doc);
    let m = &doc.material;
    let deepest_generated = gen.paths.iter().flatten().flat_map(|t| &t.moves).filter_map(|mv| mv.z).fold(0.0_f64, f64::min);
    let mut problems: Vec<String> = Vec::new();
    let mut table = String::new();
    for post in post_library() {
        let res = pipeline::export(&doc, &gen, "audit", "2026-10-10", 0.0, post);
        let mut unread = 0;
        let mut deepest_read = 0.0_f64;
        for f in &res.files {
            let lines: Vec<&str> = f.text.lines().collect();
            let who = format!("{} / {}", post.id, f.filename);
            // spindle: off after the last cut, and off before every tool change that follows cutting
            let last_cut = lines.iter().rposition(|l| cuts(l));
            if let Some(lc) = last_cut {
                if !lines[lc..].iter().any(|l| spindle_off(l)) {
                    problems.push(format!("{who}: spindle not turned off after the last cut"));
                }
            }
            let mut running = false;
            for (n, l) in lines.iter().enumerate() {
                if spindle_on(l) {
                    running = true;
                }
                if spindle_off(l) {
                    running = false;
                }
                if tool_change(l) && running && lines[..n].iter().any(|x| cuts(x)) {
                    problems.push(format!("{who}: line {}: tool change with the spindle running", n + 1));
                }
            }
            // read back, check every move
            let prog = read_program(&f.text, "audit", m);
            unread += prog.errors.len();
            for s in &prog.sections {
                deepest_read = deepest_read.min(s.min_z);
                let p = parse_program(&s.gcode, 300.0);
                for e in p.errors.iter().take(3) {
                    problems.push(format!("{who}: reread: {e}"));
                }
                let floor = -(m.t + 1.0 / 16.0) - 1e-6;
                let mut last = None;
                for mv in &p.motions {
                    if mv.machine_coords {
                        last = None;
                        continue;
                    }
                    let (a, b) = (mv.from, mv.to);
                    if mv.kind != MoveType::Rapid && mv.feed <= 0.0 {
                        problems.push(format!("{who}: line {}: cut without a feed", mv.line));
                    }
                    if b.z < floor {
                        problems.push(format!("{who}: line {}: Z {:.4} is deeper than the board + 1/16", mv.line, b.z));
                    }
                    let sideways = (a.x - b.x).abs() > 1e-6 || (a.y - b.y).abs() > 1e-6;
                    if mv.kind == MoveType::Rapid && sideways && last.is_some() && (a.z < -1e-6 || b.z < -1e-6) {
                        problems.push(format!("{who}: line {}: sideways rapid inside the material (Z {:.4})", mv.line, a.z.min(b.z)));
                    }
                    if matches!(mv.kind, MoveType::ArcCw | MoveType::ArcCcw) {
                        let (cx, cy) = (a.x + mv.i, a.y + mv.j);
                        let (r0, r1) = ((a.x - cx).hypot(a.y - cy), (b.x - cx).hypot(b.y - cy));
                        if (r0 - r1).abs() > 0.002 {
                            problems.push(format!("{who}: line {}: arc radius {:.4} vs {:.4}", mv.line, r0, r1));
                        }
                    }
                    let pad = 0.6; // largest bit radius in the project + rounding
                    if mv.kind != MoveType::Rapid && (b.x < -pad || b.y < -pad || b.x > m.w + pad || b.y > m.h + pad) {
                        problems.push(format!("{who}: line {}: cut outside the board ({:.3}, {:.3})", mv.line, b.x, b.y));
                    }
                    last = Some(b);
                }
            }
        }
        let readable = !matches!(post.id.as_str(), "shopbot-in" | "shopbot-mm" | "roland-mm"); // own languages, not G-code
        if readable && (deepest_read - deepest_generated).abs() > 0.002 {
            problems.push(format!("{}: deepest Z read back {:.4} ≠ generated {:.4} (units?)", post.id, deepest_read, deepest_generated));
        }
        table += &format!("{:<28} {:>2} file(s)  unread lines {:>3}  deepest {:.4}\n", post.id, res.files.len(), unread, deepest_read);
    }
    // every toolpath is posted with its own bit's feeds, in that bit's file (generators must not override the bit)
    for (op, paths) in doc.ops.iter().zip(&gen.paths) {
        for tp in paths {
            let t = if tp.tool >= 0 { tp.tool } else { op.tool };
            if tp.feed_xy <= 0.0 || tp.feed_z <= 0.0 {
                problems.push(format!("{}: toolpath {} has a zero feed", op.name, tp.name));
            }
            if tp.tool >= 0 && tp.tool != op.tool && op.clear_tool != tp.tool {
                problems.push(format!("{}: toolpath {} moved to bit {t} (not the op's or its clearance bit)", op.name, tp.name));
            }
        }
    }
    println!("{table}");
    problems.dedup();
    for p in problems.iter().take(80) {
        println!("PROBLEM {p}");
    }
    assert!(problems.is_empty(), "{} problem(s), first: {}", problems.len(), problems[0]);
}
