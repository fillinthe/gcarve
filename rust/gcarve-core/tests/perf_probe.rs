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
