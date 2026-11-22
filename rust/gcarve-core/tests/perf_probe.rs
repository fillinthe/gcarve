//! Timing probe: a big welded circle + square pocketed with a 1/4 in bit (the size a new user draws on a 60 x 96 board).
use gcarve_core::cam::{generate_pocket, Material, Op, OpType, Tool};
use gcarve_core::geom::{make_ellipse, make_rect};
use gcarve_core::vecops::{boolean_op, refit_arcs, BoolOp};

#[test]
#[ignore]
fn big_pocket_time() {
    let welded = boolean_op(BoolOp::Weld, &[make_ellipse(20.0, 60.0, 12.0, 12.0), make_rect(24.0, 34.0, 51.0, 58.0)], &[]);
    let tool = Tool { diameter: 0.25, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, ..Default::default() };
    let m = Material { w: 60.0, h: 96.0, ..Default::default() };
    let op = Op { kind: OpType::Pocket, name: "P".into(), depth: 0.375, stepover: 0.4, climb: true, ..Default::default() };
    for (what, shapes) in [("polyline", welded.clone()), ("refit", welded.iter().map(|c| refit_arcs(c, 0.001)).collect::<Vec<_>>())] {
        let t = std::time::Instant::now();
        let r = generate_pocket(&op, &tool, &shapes, &m);
        println!("{what}: {} nodes, {} moves, {:?}", shapes[0].nodes.len(), r.paths.first().map_or(0, |p| p.moves.len()), t.elapsed());
    }
}

/// Every toolpath kind on a large job (a 30 in welded shape on a 4 x 4 ft board): none should take seconds.
#[test]
#[ignore]
fn big_job_times() {
    use gcarve_core::cam::{generate, Entry, Side, ToolType};
    let shape = boolean_op(BoolOp::Weld, &[make_ellipse(16.0, 28.0, 12.0, 12.0), make_rect(20.0, 6.0, 44.0, 26.0)], &[]);
    let m = Material { w: 48.0, h: 48.0, ..Default::default() };
    let end = Tool { diameter: 0.25, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, ..Default::default() };
    let vbit = Tool { kind: ToolType::VBit, diameter: 0.5, angle: 90.0, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, ..Default::default() };
    let base = Op { name: "P".into(), depth: 0.25, stepover: 0.4, climb: true, ..Default::default() };
    let cases: Vec<(&str, Op, &Tool)> = vec![
        ("profile+tabs+leads", Op { kind: OpType::Profile, side: Side::Outside, depth: 0.75, tabs: true, tab_count: 6, tab_width: 0.5, tab_height: 0.125, ramp_in: 0.5, allowance: 0.02, entry: Entry::Ramp, ramp_length: 2.0, ..base.clone() }, &end),
        ("pocket raster", Op { kind: OpType::Pocket, raster: true, raster_angle: 30, ..base.clone() }, &end),
        ("v-carve", Op { kind: OpType::VCarve, ..base.clone() }, &vbit),
        ("v-carve flat", Op { kind: OpType::VCarve, flat_depth: 0.1, ..base.clone() }, &vbit),
        ("texture", Op { kind: OpType::Texture, depth: 0.05, ramp_length: 0.5, stepover: 0.8, vary: 0.5, ..base.clone() }, &end),
        ("chamfer", Op { kind: OpType::Chamfer, depth: 0.1, ..base.clone() }, &vbit),
        ("prism", Op { kind: OpType::Prism, depth: 0.2, ..base.clone() }, &vbit),
        ("bore", Op { kind: OpType::Bore, depth: 0.5, ..base.clone() }, &end),
    ];
    for (what, op, tool) in cases {
        let t = std::time::Instant::now();
        let r = generate(&op, tool, &shape, &m, Some(&end));
        println!("{what:20} {:>8} moves  {:?}  {}", r.paths.iter().map(|p| p.moves.len()).sum::<usize>(), t.elapsed(), r.warnings.first().cloned().unwrap_or_default());
    }
}

use gcarve_core::document::{Document, ToolType};
use gcarve_core::relief::Relief;

/// Rest machining with a tapered ball nose on a 10 × 10 in model: `cargo test -p gcarve-core --release --test perf_probe rest_timing -- --ignored --nocapture`
#[test]
#[ignore]
fn rest_timing() {
    let (nx, ny) = (1000, 1000);
    let height: Vec<f32> = (0..nx * ny).map(|i| {
        let (x, y) = ((i % nx) as f64 / 100.0, (i / nx) as f64 / 100.0);
        (0.5 + 0.25 * (x * 2.0).sin() * (y * 1.7).cos() + if (x * 7.0).sin().abs() < 0.08 { -0.4 } else { 0.0 }).clamp(0.0, 1.0) as f32
    }).collect();
    let mut d = Document::default();
    d.material.w = 12.0;
    d.material.h = 12.0;
    d.reliefs.push(Relief { id: 1, x: 1.0, y: 1.0, w: 10.0, h: 10.0, depth: 0.3, nx, ny, height, ..Default::default() });
    d.next_relief_id = 2;
    d.tools.clear();
    d.tools.push(Tool { kind: ToolType::BallNose, diameter: 0.125, stepdown: 0.1, feed_xy: 100.0, feed_z: 30.0, ..Default::default() });
    d.tools.push(Tool { kind: ToolType::TaperedBall, diameter: 0.25, radius: 0.25 / 25.4, angle: 5.12, stepdown: 0.1, feed_xy: 60.0, feed_z: 20.0, ..Default::default() });
    d.ops.push(Op { kind: OpType::Relief, name: "Finish".into(), relief: 1, tool: 0, stepover: 0.1, ..Default::default() });
    d.ops.push(Op { kind: OpType::Relief, name: "Details".into(), relief: 1, tool: 1, stepover: 0.2, rest: true, ..Default::default() });
    let t0 = std::time::Instant::now();
    let g = gcarve_core::pipeline::regenerate_some(&d, &|i| i == 1);
    let cut = |paths: &[gcarve_core::job::Toolpath]| -> f64 {
        let mut n = 0.0;
        for p in paths {
            let (mut x, mut y) = (f64::NAN, f64::NAN);
            for m in &p.moves {
                let (nx, ny) = (m.x.unwrap_or(x), m.y.unwrap_or(y));
                if m.kind == gcarve_core::job::MoveType::Linear && x.is_finite() {
                    n += (nx - x).hypot(ny - y);
                }
                (x, y) = (nx, ny);
            }
        }
        n
    };
    println!("rest pass on 10 x 10 in: {:.2} s to work out, {} moves, {:.0} in of moves, {:.0} min to cut", t0.elapsed().as_secs_f64(), g.paths[1].iter().map(|p| p.moves.len()).sum::<usize>(), cut(&g.paths[1]), g.minutes[1]);
    let (mut rapids, mut zonly, mut zdist) = (0, 0, 0.0);
    let mut z = 0.0_f64;
    for m in g.paths[1].iter().flat_map(|p| p.moves.iter()) {
        if m.kind == gcarve_core::job::MoveType::Rapid { rapids += 1; }
        if m.x.is_none() && m.y.is_none() { zonly += 1; }
        if let Some(nz) = m.z { zdist += (nz - z).abs(); z = nz; }
    }
    println!("  rapids {rapids}, z-only {zonly}, z travel {zdist:.0} in, paths {}", g.paths[1].len());
    d.ops[1].rest = false;
    let t0 = std::time::Instant::now();
    let g = gcarve_core::pipeline::regenerate_some(&d, &|i| i == 1);
    println!("the same bit over everything: {:.2} s to work out, {} moves, {:.0} in of moves, {:.0} min to cut", t0.elapsed().as_secs_f64(), g.paths[1].iter().map(|p| p.moves.len()).sum::<usize>(), cut(&g.paths[1]), g.minutes[1]);
}
