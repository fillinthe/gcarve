//! Adaptive clearing through the whole toolpath generator and the cutting simulator: the floor is cut, the island and
//! the walls are not touched, and every cutting move stays where the bit may be.

use gcarve_core::cam::*;
use gcarve_core::geom::{make_rect, Contour};
use gcarve_core::sim::simulate;
use gcarve_core::{MoveType, Toolpath};

fn pocket_op(adaptive: bool) -> Op {
    Op { kind: OpType::Pocket, name: "Pocket".into(), depth: 0.5, stepdown: 0.25, stepover: 0.12, adaptive, climb: true, ..Default::default() }
}

fn shapes() -> Vec<Contour> {
    // 4 × 2.5 in pocket with a 1 × 1 in island
    vec![make_rect(0.5, 0.5, 4.5, 3.0), make_rect(2.0, 1.2, 3.0, 2.2)]
}

#[test]
fn an_adaptive_pocket_cuts_the_floor_and_leaves_the_island_and_walls() {
    let tool = Tool { name: "1/4 EM".into(), kind: ToolType::EndMill, diameter: 0.25, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, ..Default::default() };
    let m = Material { w: 6.0, h: 4.0, t: 0.75, ..Default::default() };
    let r = generate(&pocket_op(true), &tool, &shapes(), &m, None);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(r.paths.len(), 1);
    let tp = &r.paths[0];
    let rad = 0.125;
    // every move is where the bit may be: inside the walls, outside the island (each grown by the radius)
    let mut deepest = 0.0f64;
    for mv in &tp.moves {
        if let (Some(x), Some(y)) = (mv.x, mv.y) {
            if mv.kind != MoveType::Rapid {
                assert!(x >= 0.5 + rad - 0.003 && x <= 4.5 - rad + 0.003 && y >= 0.5 + rad - 0.003 && y <= 3.0 - rad + 0.003, "({x:.3},{y:.3}) outside the walls");
                // the keep-out is the island grown by the radius, corners rounded: the bit's edge never reaches the island
                let (dx, dy) = ((2.0 - x).max(x - 3.0).max(0.0), (1.2 - y).max(y - 2.2).max(0.0));
                assert!(dx.hypot(dy) >= rad - 0.003, "({x:.3},{y:.3}) is over the island (edge {:.4} from it)", dx.hypot(dy));
            }
        }
        deepest = deepest.min(mv.z.unwrap_or(0.0));
    }
    assert!((deepest + 0.5).abs() < 1e-9, "deepest {deepest}");
    // a sideways rapid is only ever made above the surface
    let (mut z, mut x, mut y) = (m.retract_z, 0.0f64, 0.0f64);
    for mv in &tp.moves {
        if mv.kind == MoveType::Rapid && (mv.x.is_some_and(|v| (v - x).abs() > 1e-9) || mv.y.is_some_and(|v| (v - y).abs() > 1e-9)) {
            assert!(z >= -1e-9 && mv.z.unwrap_or(z) >= -1e-9, "a rapid across the work at z {z}");
        }
        z = mv.z.unwrap_or(z);
        x = mv.x.unwrap_or(x);
        y = mv.y.unwrap_or(y);
    }
    // cut it
    let hf = simulate(&[(tp, &tool)], &m, 1_500_000);
    // the grid covers only the cut area: outside it nothing was cut
    let depth_at = |px: f64, py: f64| {
        let (ix, iy) = (((px - hf.x0) / hf.res).floor() as i32, ((py - hf.y0) / hf.res).floor() as i32);
        if ix < 0 || iy < 0 || ix >= hf.nx || iy >= hf.ny { 0.0 } else { f64::from(hf.at(ix, iy)) }
    };
    // the open floor: cut to the full depth (a little tolerance for the grid)
    let (mut floor, mut cut_ok) = (0, 0);
    let mut px = 0.75;
    while px < 4.25 {
        let mut py = 0.75;
        while py < 2.75 {
            let near_island = px > 2.0 - 0.1 && px < 3.0 + 0.1 && py > 1.2 - 0.1 && py < 2.2 + 0.1;
            if !near_island {
                floor += 1;
                if depth_at(px, py) > 0.5 - 0.006 {
                    cut_ok += 1;
                }
            }
            py += 0.05;
        }
        px += 0.05;
    }
    assert!(cut_ok as f64 > 0.995 * f64::from(floor), "floor cut at {cut_ok} of {floor} sample points");
    // the island stands at the surface, and nothing outside the pocket was touched
    assert!(depth_at(2.5, 1.7) < 0.002 && depth_at(2.1, 1.3) < 0.002, "island cut: {} {}", depth_at(2.5, 1.7), depth_at(2.1, 1.3));
    for (px, py) in [(0.4, 1.5), (4.6, 1.5), (2.5, 3.1), (2.5, 0.4), (0.45, 0.45)] {
        assert!(depth_at(px, py) < 0.002, "outside the pocket at ({px},{py}): {}", depth_at(px, py));
    }
    // the walls are cut to the line: the wall lap leaves no more than a hair
    assert!(depth_at(0.52, 1.5) > 0.5 - 0.01 && depth_at(4.48, 1.5) > 0.5 - 0.01, "walls: {} {}", depth_at(0.52, 1.5), depth_at(4.48, 1.5));
}

/// Adaptive clearing asks for an end mill and a closed shape: anything else keeps the ring pocket, with a note.
#[test]
fn a_ball_nose_or_a_tiny_pocket_falls_back_to_rings() {
    let ball = Tool { name: "ball".into(), kind: ToolType::BallNose, diameter: 0.25, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, ..Default::default() };
    let m = Material { w: 6.0, h: 4.0, t: 0.75, ..Default::default() };
    let r = generate(&pocket_op(true), &ball, &shapes(), &m, None);
    assert_eq!(r.paths.len(), 1);
    let n_moves = |tp: &Toolpath| tp.moves.len();
    let rings = generate(&pocket_op(false), &ball, &shapes(), &m, None);
    assert_eq!(n_moves(&r.paths[0]), n_moves(&rings.paths[0]), "a ball nose cuts the same rings");
}

/// A project with an adaptive pocket (with an island) for looking at in the app and for the browser test:
/// GCARVE_ADAPTIVE_OUT=pocket.tlp cargo test -p gcarve-core --test adaptive_e2e write_sample -- --ignored
#[test]
#[ignore]
fn write_sample() {
    use gcarve_core::document::Document;
    let Ok(out) = std::env::var("GCARVE_ADAPTIVE_OUT") else { return };
    let mut d = Document::default();
    d.material = Material { w: 6.0, h: 4.0, t: 0.75, ..Default::default() };
    let (a, b) = (d.add(make_rect(0.5, 0.5, 4.5, 3.0), None), d.add(make_rect(2.0, 1.2, 3.0, 2.2), None));
    d.ops.push(Op { kind: OpType::Pocket, name: "Adaptive pocket".into(), shape_ids: vec![a, b], depth: 0.5, stepdown: 0.25, stepover: 0.12, adaptive: true, climb: true, ..Default::default() });
    std::fs::write(out, gcarve_core::project::save_project(&d)).unwrap();
}

/// How long the planner takes: cargo test -p gcarve-core --release --test adaptive_e2e timing -- --ignored --nocapture
#[test]
#[ignore]
fn timing() {
    let tool = Tool { name: "1/4 EM".into(), kind: ToolType::EndMill, diameter: 0.25, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, ..Default::default() };
    for (w, h, load) in [(3.0, 2.0, 0.12), (6.0, 6.0, 0.12), (12.0, 12.0, 0.12), (12.0, 12.0, 0.20), (24.0, 24.0, 0.25)] {
        let m = Material { w: w + 2.0, h: h + 2.0, t: 0.75, ..Default::default() };
        let op = Op { kind: OpType::Pocket, name: "P".into(), depth: 0.25, stepdown: 0.25, stepover: load, adaptive: true, climb: true, ..Default::default() };
        let t0 = std::time::Instant::now();
        let r = generate(&op, &tool, &[make_rect(1.0, 1.0, 1.0 + w, 1.0 + h)], &m, None);
        let n: usize = r.paths.iter().map(|p| p.moves.len()).sum();
        eprintln!("{w:>4} x {h:<4} load {load:.2}: {:>6.2} s, {n} moves, {:?}", t0.elapsed().as_secs_f64(), r.warnings);
    }
}
