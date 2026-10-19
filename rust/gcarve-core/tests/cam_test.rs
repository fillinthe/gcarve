//! Port of tests/cam_test.cpp (CAM, order, sim, relief, arc fitting parts; the post library, .pp import and feeds
//! advisor belong to other modules) plus safety properties over every generator.

use gcarve_core::arcfit::{fit_arcs, DEFAULT_MAX_SWEEP, DEFAULT_TOL};
use gcarve_core::cam::*;
use gcarve_core::geom::{dist, make_ellipse, make_polyline, make_rect, nearest, pt, Contour, Pt};
use gcarve_core::order::{estimate_minutes, optimize_order, split_segments};
use gcarve_core::relief::{generate_relief, load_stl, rasterize_relief, tip_height, Relief};
use gcarve_core::sim::{shade, simulate};
use gcarve_core::{post_wincnc, Move, MoveType, Toolpath};
use std::f64::consts::PI;

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 2e-3
}
fn near_e(a: f64, b: f64, e: f64) -> bool {
    (a - b).abs() < e
}
fn tool(name: &str, kind: ToolType, diameter: f64, stepdown: f64, feed_xy: f64, feed_z: f64) -> Tool {
    Tool { name: name.into(), kind, diameter, angle: 90.0, stepdown, feed_xy, feed_z, rpm: 18000, ..Default::default() }
}
/// core/document.h defaultTools()
fn default_tools() -> Vec<Tool> {
    vec![
        tool("1/4 Down End Mill (0.25 inch)", ToolType::EndMill, 0.25, 0.25, 432.0, 216.0),
        tool("1/8 End Mill (0.125 inch)", ToolType::EndMill, 0.125, 0.125, 150.0, 50.0),
        tool("V-Bit (90 deg 0.5\")", ToolType::VBit, 0.5, 0.25, 100.0, 50.0),
        tool("1/4 Ball Nose (0.25 inch)", ToolType::BallNose, 0.25, 0.125, 200.0, 80.0),
    ]
}
fn mat() -> Material {
    Material { kind: "Plywood".into(), w: 48.0, h: 48.0, t: 0.484, ..Default::default() }
}
fn circ(x: f64, y: f64, r: f64) -> Contour {
    make_ellipse(x, y, r, r)
}
fn xyz(m: &Move) -> (f64, f64, f64) {
    (m.x.unwrap(), m.y.unwrap(), m.z.unwrap())
}
fn linear(tp: &Toolpath) -> impl Iterator<Item = &Move> {
    tp.moves.iter().filter(|m| m.kind == MoveType::Linear)
}

#[derive(Debug)]
struct Stats {
    zmin: f64,
    rmin: f64,
    rmax: f64,
    area2: f64,
}
/// Stats of the final flat lap (moves at zmin) around centre c.
fn stats(tp: &Toolpath, c: Pt) -> Stats {
    let mut s = Stats { zmin: 1e9, rmin: 1e9, rmax: 0.0, area2: 0.0 };
    for m in linear(tp) {
        s.zmin = s.zmin.min(m.z.unwrap());
    }
    let mut prev: Option<&Move> = None;
    for m in linear(tp) {
        let (x, y, z) = xyz(m);
        if (z - s.zmin).abs() < 1e-9 {
            let r = dist(pt(x, y), c);
            s.rmin = s.rmin.min(r);
            s.rmax = s.rmax.max(r);
            if let Some(p) = prev {
                if (p.z.unwrap() - s.zmin).abs() < 1e-9 {
                    s.area2 += p.x.unwrap() * y - x * p.y.unwrap();
                }
            }
        }
        prev = Some(m);
    }
    s
}

const C: Pt = pt(1.675, 1.675);
fn op_in() -> Op {
    Op { side: Side::Inside, depth: 0.484, climb: false, ..Default::default() }
}
fn op_out() -> Op {
    Op { depth: 0.484, climb: false, ..Default::default() }
}

#[test]
fn profile() {
    let (m, em) = (mat(), default_tools()[0].clone());
    // reference/02_EMD14.tap was cut conventional: inside circle CW, outside circle CCW
    let ri = generate_profile(&op_in(), &em, &[circ(C.x, C.y, 0.458333)], &m);
    assert!(ri.paths.len() == 1 && ri.warnings.is_empty());
    let si = stats(&ri.paths[0], C);
    assert!(near_e(si.zmin, -0.484, 1e-9) && near(si.rmin, 0.333333) && near(si.rmax, 0.333333) && si.area2 < 0.0, "{si:?}");
    // spiral: starts at the surface at the bottom of the circle, like the reference
    let (x, y, z) = xyz(&ri.paths[0].moves[1]);
    assert!(z == 0.0 && near(x, 1.675) && near(y, 1.675 - 0.333333));

    let ro = generate_profile(&op_out(), &em, &[circ(C.x, C.y, 0.925)], &m);
    let so = stats(&ro.paths[0], C);
    assert!(near(so.rmin, 1.05) && near(so.rmax, 1.05) && so.area2 > 0.0);

    // climb (the default) reverses both: outside CW, inside CCW
    let mut cl = Op { depth: 0.484, ..Default::default() };
    assert!(stats(&generate_profile(&cl, &em, &[circ(C.x, C.y, 0.925)], &m).paths[0], C).area2 < 0.0);
    cl.side = Side::Inside;
    assert!(stats(&generate_profile(&cl, &em, &[circ(C.x, C.y, 0.458333)], &m).paths[0], C).area2 > 0.0);

    // a disc with a hole, both in one op: outside the disc, and the hole cut from inside it
    let both = generate_profile(&op_out(), &em, &[circ(C.x, C.y, 0.925), circ(C.x, C.y, 0.458333)], &m);
    assert!(both.paths.len() == 2 && near(stats(&both.paths[1], C).rmax, 0.333333) && stats(&both.paths[1], C).area2 < 0.0);
    assert_eq!(both.paths[1].name, "Profile [1]");

    // tabs: on the last pass the cutter rises to depth - tabHeight, never lower inside a tab
    let tabbed = Op { tabs: true, tab_count: 4, tab_width: 0.25, tab_height: 0.125, entry: Entry::Plunge, ..op_out() };
    let tt = &generate_profile(&tabbed, &em, &[make_rect(0.0, 0.0, 10.0, 5.0)], &m).paths[0];
    assert!(linear(tt).filter(|m| near_e(m.z.unwrap(), -0.359, 1e-9)).count() >= 8); // 4 tabs x (rise + travel)

    // tabs placed by hand: one on the bottom edge, one on the right edge; nothing near the others
    let hand = Op { tab_points: vec![[5.0, 0.0], [10.0, 2.5]], ..tabbed.clone() };
    let th = &generate_profile(&hand, &em, &[make_rect(0.0, 0.0, 10.0, 5.0)], &m).paths[0];
    let high: Vec<_> = linear(th).filter(|m| near_e(m.z.unwrap(), -0.359, 1e-9)).collect();
    assert!(high.len() >= 4 && high.len() < linear(tt).filter(|m| near_e(m.z.unwrap(), -0.359, 1e-9)).count());
    let (xs, ys): (Vec<f64>, Vec<f64>) = high.iter().map(|m| (m.x.unwrap(), m.y.unwrap())).unzip();
    // all raised points are on the bottom edge (y ~ -r) or the right edge (x ~ 10 + r)
    assert!(xs.iter().zip(&ys).all(|(&x, &y)| y < 0.0 || x > 10.0), "{xs:?} {ys:?}");

    // inside cut on a shape smaller than the tool warns and makes nothing
    let tiny = generate_profile(&op_in(), &em, &[circ(0.0, 0.0, 0.1)], &m);
    assert!(tiny.paths.is_empty() && !tiny.warnings.is_empty());

    // full job -> WinCNC text: header, safe-Z approach, retract at end of each path
    let jobs = build_jobs(&[op_in(), op_out()], &default_tools(), &[ri.paths.clone(), ro.paths.clone()], &m, "09:09 PM, Tuesday December 17 2024");
    assert!(jobs.len() == 1 && jobs[0].filename == "01_1_4_Down_End_Mill_0_25_inch");
    let g = post_wincnc(&jobs[0]);
    assert!(g.contains("[MATERIAL size: 48.000\" x 48.000\" x 0.484\"]"));
    // offset polygons are within the 0.0005" arc tolerance of the true circle, so Y is 1.3417 not 1.341667
    assert!(g.contains(" G0 X1.675000 Y1.3417") && g.contains(" G0 Z0.800\r\n G1 X1.675000 Y1.3417"));
    assert!(g.contains("Z0.000000\r\n") && !g.contains("Z-0.000000"));
    assert!(g.contains(" F432 XY") && g.rfind(" G53 Z0\r\n M5").is_some());

    // feeds per material
    let mut t = em.clone();
    t.presets.push(FeedPreset { material: "MDF".into(), feed_xy: 300.0, feed_z: 100.0, stepdown: 0.3, rpm: 16000 });
    assert!(preset_for(&t, "mdf").is_some() && preset_for(&t, "Oak").is_none());
    let e = effective_tool(&t, "MDF");
    assert!(near(e.feed_xy, 300.0) && near(e.feed_z, 100.0) && e.rpm == 16000 && near(e.stepdown, 0.3));
    let d = effective_tool(&t, "Oak");
    assert!(near(d.feed_xy, em.feed_xy) && d.rpm == em.rpm);
    assert!(near(generate_profile(&op_out(), &e, &[make_rect(0.0, 0.0, 2.0, 1.0)], &m).paths[0].feed_xy, 300.0));
}

#[test]
fn pocket() {
    let (m, em) = (mat(), default_tools()[0].clone());
    let pk = Op { kind: OpType::Pocket, depth: 0.25, entry: Entry::Plunge, ..Default::default() };
    let rp = generate_pocket(&pk, &em, &[make_rect(0.0, 0.0, 2.0, 1.0)], &m);
    assert!(rp.paths.len() == 1 && rp.warnings.is_empty());
    let (mut zmin, mut cuts, mut near_centre) = (0.0_f64, 0, false);
    for mv in linear(&rp.paths[0]) {
        let (x, y, z) = xyz(mv);
        cuts += 1;
        zmin = zmin.min(z);
        // every cut point stays inside the rect inset by the cutter radius
        assert!(x > 0.125 - 1e-3 && x < 2.0 - 0.125 + 1e-3 && y > 0.125 - 1e-3 && y < 1.0 - 0.125 + 1e-3);
        if (y - 0.5).abs() <= 0.125 {
            near_centre = true;
        }
    }
    assert!(near_e(zmin, -0.25, 1e-9) && cuts > 20 && near_centre); // rings reach the middle
    assert!((rp.paths[0].moves[1].y.unwrap() - 0.5).abs() < 0.2); // first cut = innermost ring (inside-out)

    // island: no cut point within island + r
    let ri2 = generate_pocket(&pk, &em, &[make_rect(0.0, 0.0, 3.0, 3.0), make_rect(1.0, 1.0, 2.0, 2.0)], &m);
    assert_eq!(ri2.paths.len(), 1);
    for mv in linear(&ri2.paths[0]) {
        let (x, y, _) = xyz(mv);
        let (dx, dy) = ((1.0 - x).max(0.0).max(x - 2.0), (1.0 - y).max(0.0).max(y - 2.0));
        assert!(dx.hypot(dy) >= 0.125 - 1e-3);
    }
    // two separate pockets in one op: one retract between them (start, lift+move, end)
    let two = generate_pocket(&pk, &em, &[make_rect(0.0, 0.0, 1.0, 1.0), make_rect(3.0, 0.0, 4.0, 1.0)], &m);
    assert_eq!(two.paths[0].moves.iter().filter(|m| m.kind == MoveType::Rapid).count(), 4);
    // allowance leaves material on the walls
    let pa = Op { allowance: 0.1, ..pk.clone() };
    for mv in linear(&generate_pocket(&pa, &em, &[make_rect(0.0, 0.0, 2.0, 1.0)], &m).paths[0]) {
        let y = mv.y.unwrap();
        assert!(y > 0.225 - 1e-3 && y < 1.0 - 0.225 + 1e-3);
    }
    assert!(generate_pocket(&pk, &em, &[make_rect(0.0, 0.0, 0.2, 0.2)], &m).paths.is_empty()); // too small

    // pocket ramp entry: first ring of a pass descends gradually
    let pr = Op { entry: Entry::Ramp, ramp_length: 0.5, depth: 0.25, ..pk.clone() };
    let pp = &generate_pocket(&pr, &em, &[make_rect(0.0, 0.0, 2.0, 1.0)], &m).paths[0];
    assert!(near_e(pp.moves[1].z.unwrap(), 0.0, 1e-9) && pp.moves[3].z.unwrap() > -0.25 + 1e-9);
}

#[test]
fn drill_and_dispatch() {
    let (m, em) = (mat(), default_tools()[0].clone());
    let dr = Op { kind: OpType::Drill, name: "Drill".into(), depth: 0.4, peck: 0.15, ..Default::default() };
    let rd = generate_drill(&dr, &em, &[circ(1.0, 1.0, 0.125), circ(3.0, 1.0, 0.125)], &m);
    assert!(rd.paths.len() == 2 && rd.warnings.is_empty());
    let mut zs = Vec::new();
    for mv in &rd.paths[0].moves {
        assert!(near(mv.x.unwrap(), 1.0) && near(mv.y.unwrap(), 1.0));
        if mv.kind == MoveType::Linear {
            zs.push(mv.z.unwrap());
        }
    }
    assert!(zs.len() == 3 && near(zs[0], -0.15) && near(zs[1], -0.30) && near(zs[2], -0.4));
    assert_eq!(rd.paths[1].name, "Drill [1]");
    // export: an op's per-shape paths form one toolpath block per bit, named after the op
    let dj = build_jobs(std::slice::from_ref(&dr), std::slice::from_ref(&em), std::slice::from_ref(&rd.paths), &m, "");
    let moves = rd.paths[0].moves.len() + rd.paths[1].moves.len();
    assert!(dj.len() == 1 && dj[0].toolpaths.len() == 1 && dj[0].toolpaths[0].name == "Drill" && dj[0].toolpaths[0].moves.len() >= moves);
    assert!(!generate_drill(&dr, &em, &[circ(1.0, 1.0, 0.5)], &m).warnings.is_empty()); // hole bigger than drill
    // dispatch
    assert_eq!(generate(&dr, &em, &[circ(1.0, 1.0, 0.1)], &m, None).paths[0].moves.len(), rd.paths[0].moves.len());
}

#[test]
fn ramps_3d_tabs_origin() {
    let (m, em) = (mat(), default_tools()[0].clone());
    // ramp entry over a distance: depth reached after rampLength, then flat
    let rp = Op { depth: 0.4, stepdown: 0.4, entry: Entry::Ramp, ramp_length: 1.0, ..op_out() };
    let t = &generate_profile(&rp, &em, &[make_rect(0.0, 0.0, 4.0, 2.0)], &m).paths[0];
    let (mut run, mut prev, mut reached, mut z_at1) = (0.0, None, false, 1.0);
    for mv in linear(t) {
        let q = pt(mv.x.unwrap(), mv.y.unwrap());
        if let Some(p) = prev {
            run += dist(p, q);
        }
        prev = Some(q);
        if run > 1.0 + 1e-6 && !reached {
            reached = true;
            z_at1 = mv.z.unwrap();
        }
        if reached {
            assert!(mv.z.unwrap() <= -0.4 + 1e-6);
        }
    }
    assert!(near_e(z_at1, -0.4, 1e-6));
    // 3D tabs: on the final lap the climb out of a tab is gradual
    let tb = Op { depth: 0.5, entry: Entry::Plunge, tabs: true, tab_count: 2, tab_width: 0.3, tab_height: 0.2, tab_ramped: true, ..op_out() };
    let tt = &generate_profile(&tb, &em, &[make_rect(0.0, 0.0, 10.0, 1.0)], &m).paths[0];
    let (mut max_jump, mut pm): (f64, Option<&Move>) = (0.0, None);
    for mv in linear(tt) {
        if let Some(p) = pm {
            if near_e(p.z.unwrap(), mv.z.unwrap(), 0.5) {
                let dxy = dist(pt(p.x.unwrap(), p.y.unwrap()), pt(mv.x.unwrap(), mv.y.unwrap()));
                if dxy > 1e-9 {
                    max_jump = max_jump.max((mv.z.unwrap() - p.z.unwrap()).abs() / dxy);
                }
            }
        }
        pm = Some(mv);
    }
    assert!(max_jump < 1.5); // ramped: slope <= ~45 degrees
    assert!(linear(tt).filter(|m| near_e(m.z.unwrap(), -0.3, 1e-9)).count() >= 4); // tab tops at depth - height

    // origin corner and Z zero at the bed shift the G-code
    let mo = Material { w: 10.0, h: 6.0, t: 0.5, origin: 4, z_bottom: true, ..mat() };
    let po = Op { depth: 0.5, entry: Entry::Plunge, ..op_out() };
    let r = generate_profile(&po, &em, &[make_rect(4.0, 2.0, 6.0, 4.0)], &mo);
    let jobs = build_jobs(&[po], &default_tools(), &[r.paths], &mo, "t");
    assert_eq!(jobs[0].z_zero, "Table");
    let (mut minx, mut maxx, mut minz) = (1e9_f64, -1e9_f64, 1e9_f64);
    for mv in jobs[0].toolpaths.iter().flat_map(|t| &t.moves) {
        if let Some(x) = mv.x {
            minx = minx.min(x);
            maxx = maxx.max(x);
        }
        if let Some(z) = mv.z {
            minz = minz.min(z);
        }
    }
    assert!(near(minx, -1.125) && near(maxx, 1.125)); // centred on the 10" board
    assert!(near_e(minz, 0.0, 1e-9)); // full-depth cut lands on the bed
    assert!(post_wincnc(&jobs[0]).contains("[Z Zero Position: Table]"));
}

#[test]
fn arc_fitting() {
    let (m, em) = (mat(), default_tools()[0].clone());
    let ro = generate_profile(&op_out(), &em, &[circ(C.x, C.y, 0.925)], &m);
    let fit = fit_arcs(&ro.paths[0], DEFAULT_TOL, DEFAULT_MAX_SWEEP);
    let arcs = fit.moves.iter().filter(|m| matches!(m.kind, MoveType::ArcCw | MoveType::ArcCcw)).count();
    let lin = linear(&fit).count();
    let lin_at_depth = linear(&fit).filter(|m| near_e(m.z.unwrap(), -0.484, 1e-9)).count();
    assert!(arcs == 4 && lin_at_depth <= 1 && lin > 100); // ramp laps stay G1; final lap is arcs
    let mut prev = pt(0.0, 0.0);
    for mv in &fit.moves {
        if mv.kind == MoveType::ArcCcw {
            let ctr = prev + pt(mv.i, mv.j);
            assert!(near(ctr.x, C.x) && near(ctr.y, C.y) && near(dist(ctr, pt(mv.x.unwrap(), mv.y.unwrap())), 1.05));
            assert!(mv.z.is_none()); // arc lines carry no Z, like the reference
        }
        assert!(mv.kind != MoveType::ArcCw); // CCW cut -> G3 only
        if let (Some(x), Some(y)) = (mv.x, mv.y) {
            prev = pt(x, y);
        }
    }
    // a pure rectangle gains only the 4 rounded corners of the offset path
    let rt = fit_arcs(&generate_profile(&op_out(), &em, &[make_rect(0.0, 0.0, 4.0, 2.0)], &m).paths[0], DEFAULT_TOL, DEFAULT_MAX_SWEEP);
    assert_eq!(rt.moves.iter().filter(|m| matches!(m.kind, MoveType::ArcCw | MoveType::ArcCcw)).count(), 4);
    // every fitted arc stays within tolerance of the original polyline (16 samples per arc)
    let poly: Vec<Pt> = linear(&ro.paths[0]).filter(|m| near_e(m.z.unwrap(), -0.484, 1e-9)).map(|m| pt(m.x.unwrap(), m.y.unwrap())).collect();
    let pc = make_polyline(&poly, false);
    prev = pt(0.0, 0.0);
    for mv in &fit.moves {
        if mv.kind == MoveType::ArcCcw {
            let ctr = prev + pt(mv.i, mv.j);
            let a0 = (prev.y - ctr.y).atan2(prev.x - ctr.x);
            let mut a1 = (mv.y.unwrap() - ctr.y).atan2(mv.x.unwrap() - ctr.x);
            while a1 < a0 {
                a1 += 2.0 * PI;
            }
            for s in 1..16 {
                let a = a0 + (a1 - a0) * s as f64 / 16.0;
                assert!(nearest(&pc, pt(ctr.x + 1.05 * a.cos(), ctr.y + 1.05 * a.sin())).d < 0.002);
            }
        }
        if let (Some(x), Some(y)) = (mv.x, mv.y) {
            prev = pt(x, y);
        }
    }
    // through the post: G3 with I/J, no Z
    let gg = post_wincnc(&build_jobs(&[op_out()], &default_tools(), &[ro.paths], &m, "t")[0]);
    let g3 = gg.find(" G3 X").unwrap();
    assert!(gg[g3..g3 + 60].contains(" J1.0"));
    assert!(!gg[g3..g3 + gg[g3..].find("\r\n").unwrap()].contains(" Z"));
}

#[test]
fn vcarve() {
    let (m, em) = (mat(), default_tools()[0].clone());
    let mut vb = default_tools()[2].clone();
    vb.diameter = 1.0; // V 0.5 deep: these shapes fit the bit
    let vc = Op { kind: OpType::VCarve, ..Default::default() };
    let rv = generate_vcarve(&vc, &vb, &[make_rect(0.0, 0.0, 0.8, 0.8)], &m, None);
    assert!(rv.paths.len() == 1 && rv.warnings.is_empty(), "{:?}", rv.warnings);
    // 0.4 deep with a 0.5" 90 deg bit (V only 0.25 deep): warns, still carves
    let narrow = Tool { diameter: 0.5, ..vb.clone() };
    let rw = generate_vcarve(&vc, &narrow, &[make_rect(0.0, 0.0, 0.8, 0.8)], &m, None);
    assert!(rw.paths.len() == 1 && rw.warnings.len() == 1 && rw.warnings[0].contains("V is only 0.250"));
    // medial axis of a square = the two diagonals; every point on the V floor (tan 45 = 1), centre at -0.4
    let (mut zmin, mut n, mut on_floor) = (0.0_f64, 0, 0);
    for mv in linear(&rv.paths[0]) {
        let (x, y, z) = xyz(mv);
        n += 1;
        let d = x.min(0.8 - x).min(y).min(0.8 - y);
        assert!(d > -1e-6 && -z <= d + 1e-3); // never below the V floor
        if (-z - d).abs() < 1e-3 {
            on_floor += 1;
        }
        zmin = zmin.min(z);
    }
    assert!(n >= 4 && on_floor >= 4 && near_e(zmin, -0.4, 1e-3));
    // 0.4 deep with a 0.25 pass depth: one layer lap at -0.25 before the skeleton, like VCarve
    assert!(linear(&rv.paths[0]).filter(|m| near_e(m.z.unwrap(), -0.25, 1e-9)).count() >= 5 && near_e(rv.paths[0].moves[2].z.unwrap(), -0.25, 1e-9));
    // a rounded shape: skeleton stays inside, never cuts below the floor
    let rc = generate_vcarve(&vc, &vb, &[make_ellipse(1.0, 1.0, 0.4, 0.25)], &m, None);
    assert_eq!(rc.paths.len(), 1);
    for mv in linear(&rc.paths[0]) {
        let (x, y, z) = xyz(mv);
        let (dx, dy) = ((x - 1.0) / 0.4, (y - 1.0) / 0.25);
        assert!(dx * dx + dy * dy < 1.0 + 1e-3 && z >= -0.25 - 1e-3);
    }
    // flat depth: nothing deeper than 0.2, and a floor at exactly -0.2 inside the inset region
    let vf = Op { flat_depth: 0.2, ..vc.clone() };
    let rf = generate_vcarve(&vf, &vb, &[make_rect(0.0, 0.0, 0.8, 0.8)], &m, None);
    let mut floor = false;
    for mv in linear(&rf.paths[0]) {
        let (x, _, z) = xyz(mv);
        assert!(z >= -0.2 - 1e-9);
        if near_e(z, -0.2, 1e-9) && x > 0.3 && x < 0.5 {
            floor = true;
        }
    }
    assert!(floor);
    // too wide for the material thickness -> clamped with a warning
    assert!(!generate_vcarve(&vc, &vb, &[make_rect(0.0, 0.0, 4.0, 4.0)], &m, None).warnings.is_empty());
    // letter-like shape with a counter: the hole stays uncut
    let ro2 = generate_vcarve(&vc, &vb, &[make_rect(0.0, 0.0, 1.0, 1.0), make_rect(0.4, 0.4, 0.6, 0.6)], &m, None);
    for mv in linear(&ro2.paths[0]) {
        let (x, y, _) = xyz(mv);
        assert!(!(x > 0.405 && x < 0.595 && y > 0.405 && y < 0.595));
    }
    // clearance tool: end mill clears the flat floor as a second path on its own tool; the V-bit keeps the walls
    let vcl = Op { tool: 2, flat_depth: 0.1, clear_tool: 0, ..vc.clone() };
    let rc = generate_vcarve(&vcl, &vb, &[make_rect(0.0, 0.0, 2.0, 2.0)], &m, Some(&em));
    assert!(rc.paths.len() == 2 && rc.paths[1].tool == 0 && rc.paths[1].name.contains("clearance"));
    for mv in linear(&rc.paths[1]) {
        let (x, y, z) = xyz(mv);
        let d = x.min(2.0 - x).min(y).min(2.0 - y);
        assert!(near_e(z, -0.1, 1e-9) && d >= 0.1 + 0.125 - 2e-3);
    }
    for mv in linear(&rc.paths[0]) {
        assert!(mv.z.unwrap() >= -0.1 - 1e-9);
    }
    let jobs2 = build_jobs(&[vcl], &default_tools(), &[rc.paths], &m, "t");
    assert!(jobs2.len() == 2 && jobs2[0].tool == vb.name && jobs2[1].tool == em.name); // one file per tool
    // end mill is refused
    assert!(generate_vcarve(&vc, &em, &[make_rect(0.0, 0.0, 0.8, 0.8)], &m, None).paths.is_empty());

    // start depth: rings begin below the surface; a clearance end mill pockets the material above
    let vs = Op { tool: 2, start_depth: 0.1, flat_depth: 0.3, clear_tool: 0, ..vc.clone() };
    let rs = generate_vcarve(&vs, &vb, &[make_rect(0.0, 0.0, 0.8, 0.8)], &m, Some(&em));
    assert!(rs.paths.len() >= 2 && rs.paths[0].name.contains("clear to start") && rs.paths[0].tool == 0);
    for mv in linear(&rs.paths[0]) {
        assert!(near_e(mv.z.unwrap(), -0.1, 1e-9));
    }
    assert!(linear(&rs.paths[1]).map(|m| m.z.unwrap()).fold(-1.0, f64::max) <= -0.1 + 1e-9);
    assert!(!generate_vcarve(&vs, &vb, &[make_rect(0.0, 0.0, 0.8, 0.8)], &m, None).warnings.is_empty());

    // inlay: female + male (mirrored, start at glue gap), two tools
    let il = Op { kind: OpType::Inlay, tool: 2, flat_depth: 0.2, glue_gap: 0.1, inlay_margin: 0.5, clear_tool: 0, ..Default::default() };
    let ri = generate_inlay(&il, &vb, &[make_rect(1.0, 1.0, 2.0, 2.0)], &m, Some(&em));
    assert!(ri.paths.iter().any(|p| p.name.contains("[female]")) && ri.paths.iter().any(|p| p.name.contains("[male]")));
    for p in ri.paths.iter().filter(|p| p.name.contains("[male]") && p.tool < 0) {
        for mv in linear(p) {
            let (x, y, z) = xyz(mv);
            assert!(x > 0.49 && x < 2.51 && y > 0.49 && y < 2.51 && z <= -0.1 + 1e-9);
            assert!(!(x > 1.001 && x < 1.999 && y > 1.001 && y < 1.999));
        }
    }
    assert_eq!(build_jobs(&[il], &default_tools(), &[ri.paths], &m, "t").len(), 2); // V-bit + clearance end mill
}

#[test]
fn bore_and_flute() {
    let (m, em) = (mat(), default_tools()[0].clone());
    let bo = Op { kind: OpType::Bore, depth: 0.4, stepdown: 0.1, ..Default::default() };
    let rb = generate_bore(&bo, &em, &[circ(2.0, 2.0, 0.5)], &m);
    assert!(rb.paths.len() == 1 && rb.warnings.is_empty());
    let (mut zmin, mut zprev, mut monotone, mut lin) = (0.0_f64, 0.0, true, 0);
    for mv in linear(&rb.paths[0]) {
        let (x, y, z) = xyz(mv);
        lin += 1;
        assert!(near(dist(pt(x, y), pt(2.0, 2.0)), 0.375)); // on the helix radius
        if z > zprev + 1e-9 {
            monotone = false;
        }
        zprev = z;
        zmin = zmin.min(z);
    }
    assert!(monotone && near_e(zmin, -0.4, 1e-9) && lin >= 4 * 36 + 36); // 4 turns + finishing lap
    assert!(fit_arcs(&rb.paths[0], DEFAULT_TOL, DEFAULT_MAX_SWEEP).moves.iter().filter(|m| m.kind == MoveType::ArcCcw).count() >= 4);
    let small = generate_bore(&bo, &em, &[circ(2.0, 2.0, 0.1)], &m);
    assert!(small.paths.len() == 1 && !small.warnings.is_empty());

    // flute: 4" line, depth 0.2, ramp 1" each end, 2 passes alternating direction
    let fl = Op { kind: OpType::Flute, depth: 0.2, stepdown: 0.1, ramp_in: 1.0, ramp_out: 1.0, ..Default::default() };
    let line = [make_polyline(&[pt(0.0, 0.0), pt(4.0, 0.0)], false)];
    let rf = generate_flute(&fl, &em, &line, &m);
    assert!(rf.paths.len() == 1 && rf.warnings.is_empty());
    let (mut zmin, mut lin) = (0.0_f64, 0);
    for mv in linear(&rf.paths[0]) {
        let (x, _, z) = xyz(mv);
        lin += 1;
        if (1.0 - 1e-6..=3.0 + 1e-6).contains(&x) {
            assert!(near_e(z, -0.1, 1e-9) || near_e(z, -0.2, 1e-9));
        }
        if near(x, 0.0) || near(x, 4.0) {
            assert!(near_e(z, 0.0, 1e-9));
        }
        if near(x, 0.5) {
            assert!(near_e(z, -0.05, 1e-9) || near_e(z, -0.1, 1e-9));
        }
        zmin = zmin.min(z);
    }
    assert!(near_e(zmin, -0.2, 1e-9) && lin > 40);
    assert!(near(linear(&rf.paths[0]).last().unwrap().x.unwrap(), 0.0)); // second pass runs back
    // ramps longer than the vector are scaled to meet in the middle (lens)
    let lens = Op { ramp_in: 10.0, ramp_out: 10.0, stepdown: 0.2, ..fl };
    let zm = linear(&generate_flute(&lens, &em, &line, &m).paths[0]).map(|m| m.z.unwrap()).fold(0.0, f64::min);
    assert!(near(zm, -0.2));
}

#[test]
fn simulation() {
    let tools = default_tools();
    let em = tools[0].clone();
    let mut vb = tools[2].clone();
    vb.diameter = 1.0;
    let sm = Material { kind: "P".into(), w: 4.0, h: 3.0, t: 0.5, ..Default::default() };
    let pk2 = Op { kind: OpType::Pocket, depth: 0.2, ..Default::default() };
    let pocket = generate_pocket(&pk2, &em, &[make_rect(1.0, 1.0, 3.0, 2.0)], &sm);
    let pr2 = Op { depth: 0.5, entry: Entry::Plunge, ..Default::default() };
    let prof = generate_profile(&pr2, &em, &[circ(0.6, 0.6, 0.25)], &sm);
    let hf = simulate(&[(&pocket.paths[0], &em), (&prof.paths[0], &em)], &sm, 200000);
    assert!(hf.nx > 100 && hf.ny > 50 && hf.x0 < 0.4 && hf.y0 < 0.4); // grid covers the cut region
    assert!(near_e(hf.sample(2.0, 1.5) as f64, 0.2, 1e-4)); // pocket floor
    assert_eq!(hf.sample(0.3, 2.7), 0.0); // untouched corner
    assert!(near_e(hf.sample(0.6 + 0.375, 0.6) as f64, 0.5, 1e-4)); // profile goes through
    assert_eq!(hf.sample(0.6, 0.6), 0.0); // inside the disc: untouched
    // v-carve cone: 0.4 wide slot -> 0.2 deep centre line
    let v2 = Op { kind: OpType::VCarve, tool: 2, ..Default::default() };
    let vc2 = generate_vcarve(&v2, &vb, &[make_rect(0.5, 0.5, 1.5, 0.9)], &sm, None);
    let hv = simulate(&[(&vc2.paths[0], &vb)], &sm, 400000);
    assert!(near_e(hv.sample(1.0, 0.7) as f64, 0.2, 0.02));
    assert!(near_e(hv.sample(1.0, 0.6) as f64, 0.1, 0.03));
    assert_eq!(hv.sample(1.0, 0.45), 0.0);
    let img = shade(&hf, &sm);
    assert_eq!(img.len(), hf.nx as usize * hf.ny as usize * 4);
    let px = |x: f64, y: f64| {
        let (ix, iy) = ((x / hf.res) as usize, (y / hf.res) as usize);
        img[((hf.ny as usize - 1 - iy) * hf.nx as usize + ix) * 4 + 3]
    };
    assert!(px(0.6 + 0.375, 0.6) == 0 && px(2.0, 1.5) == 255); // through-cut transparent, floor opaque
}

#[test]
fn safety_net_and_air_cut() {
    let (m, em) = (mat(), default_tools()[0].clone());
    // a sideways rapid never starts below the surface (generate() lifts first)
    let mut t = Toolpath {
        moves: vec![
            Move::rapid(Some(0.0), Some(0.0), Some(0.2)),
            Move::linear(Some(0.0), Some(0.0), Some(-0.15)),
            Move::linear(Some(1.0), Some(0.0), Some(-0.15)),
            Move::rapid(Some(2.0), Some(1.0), Some(0.2)),
            Move::rapid(Some(3.0), Some(1.0), Some(-0.1)),
        ],
        ..Default::default()
    };
    enforce_safe_rapids(&mut t, 0.2);
    assert_no_rapid_in_material(&t, "enforce_safe_rapids");
    assert!(t.moves.len() == 6 && near(t.moves[3].x.unwrap(), 1.0) && near(t.moves[3].z.unwrap(), 0.2)); // straight lift at X1

    // pre-export check and air-cut files
    let ro = generate_profile(&op_out(), &em, &[circ(C.x, C.y, 0.925)], &m);
    let deep = Op { depth: 1.0, ..op_out() }; // 0.516 into the spoilboard on 0.484 stock
    let gd = generate_profile(&deep, &em, &[circ(C.x, C.y, 0.925)], &m).paths;
    let iss = safety_issues(&[deep], &default_tools(), &[gd], &m);
    assert!(iss.len() == 1 && iss[0].contains("spoilboard"));
    assert!(safety_issues(&[op_out()], &default_tools(), std::slice::from_ref(&ro.paths), &m).is_empty());
    // flutes shorter than the cut: warned; long enough: not
    let mut short = default_tools();
    short[op_out().tool as usize].cut_length = 0.25;
    assert!(safety_issues(&[op_out()], &short, std::slice::from_ref(&ro.paths), &m).iter().any(|s| s.contains("flutes are only 0.250")));
    short[op_out().tool as usize].cut_length = 1.0;
    assert!(safety_issues(&[op_out()], &short, std::slice::from_ref(&ro.paths), &m).is_empty());
    let low_safe = Material { safe_z: 0.0, ..m.clone() };
    assert!(!safety_issues(&[op_out()], &default_tools(), std::slice::from_ref(&ro.paths), &low_safe).is_empty());
    let mut aj = build_jobs(&[op_out()], &default_tools(), &[ro.paths], &m, "t");
    let raise = air_cut_raise(&m);
    make_air_cut(&mut aj, raise);
    let zlow = aj[0].toolpaths.iter().flat_map(|t| &t.moves).filter_map(|m| m.z).fold(1e9, f64::min);
    assert!(raise >= 1.0 && zlow > 0.2 && aj[0].filename.contains("_AIRCUT"));
    assert!(post_wincnc(&aj[0]).contains("[AIR CUT TEST - NOT FOR CUTTING]"));
}

#[test]
fn travel_order() {
    let (m, em) = (mat(), default_tools()[0].clone());
    let po = Op { depth: 0.5, stepdown: 0.25, ..Default::default() }; // two passes per shape
    let sq = [make_rect(10.0, 10.0, 11.0, 11.0), make_rect(0.5, 0.5, 1.5, 1.5), make_rect(10.0, 0.5, 11.0, 1.5)];
    let rr = generate(&po, &em, &sq, &m, None);
    assert!(rr.paths.len() == 3 && rr.paths[0].name == po.name && rr.paths[1].name.contains("[1]"));
    let segs = split_segments(&rr.paths[0]);
    assert!(!segs.is_empty() && segs[0].start.x < 3.0 && segs[0].start.y < 3.0); // near square first
    // within the near square, the shallow pass comes before the deep one
    let (mut last_z, mut ordered) = (0.0, true);
    for mv in linear(&rr.paths[0]) {
        let (x, y, z) = xyz(mv);
        if x < 3.0 && y < 3.0 {
            if z < last_z - 1e-9 {
                last_z = z;
            } else if z > last_z + 0.1 {
                ordered = false;
            }
        }
    }
    assert!(ordered);
    let mut copy = rr.paths[0].clone();
    let before = estimate_minutes(&copy, 100.0, 50.0, 400.0);
    optimize_order(&mut copy, pt(0.0, 0.0));
    assert!(estimate_minutes(&copy, 100.0, 50.0, 400.0) <= before + 1e-9 && copy.moves.len() == rr.paths[0].moves.len());
    assert!(estimate_minutes(&rr.paths[0], 100.0, 50.0, 400.0) > 0.0);
    // with acceleration: one long straight move is barely slower; sharp little zig-zags (V-carve, photo) are much slower
    {
        use gcarve_core::job::Move;
        use gcarve_core::order::{estimate_minutes_with, Motion};
        let m = Motion { rapid_ipm: 400.0, accel: 30.0 };
        let line = Toolpath { moves: vec![Move::linear(Some(0.0), Some(0.0), Some(-0.1)), Move::linear(Some(40.0), Some(0.0), Some(-0.1))], ..Default::default() };
        let mut zig = Toolpath { moves: vec![Move::linear(Some(0.0), Some(0.0), Some(-0.1))], ..Default::default() };
        for k in 1..=400 {
            zig.moves.push(Move::linear(Some(k as f64 * 0.1), Some(if k % 2 == 0 { 0.0 } else { 0.5 }), Some(-0.1)));
        }
        let (plain_line, acc_line) = (estimate_minutes(&line, 200.0, 50.0, 400.0), estimate_minutes_with(&line, 200.0, 50.0, m));
        assert!(acc_line > plain_line && acc_line < plain_line * 1.05, "{plain_line} {acc_line}");
        let (plain_zig, acc_zig) = (estimate_minutes(&zig, 200.0, 50.0, 400.0), estimate_minutes_with(&zig, 200.0, 50.0, m));
        assert!(acc_zig > plain_zig * 1.5, "{plain_zig} {acc_zig}");
        assert_eq!(estimate_minutes_with(&zig, 200.0, 50.0, Motion::default()), plain_zig);
    }
}

fn pyramid_stl() -> String {
    let mut stl = String::from("solid p\n");
    for t in [[0, 0, 0, 10, 0, 0, 5, 5, 3], [10, 0, 0, 10, 10, 0, 5, 5, 3], [10, 10, 0, 0, 10, 0, 5, 5, 3], [0, 10, 0, 0, 0, 0, 5, 5, 3]] {
        stl += &format!("facet normal 0 0 1\nouter loop\nvertex {} {} {}\nvertex {} {} {}\nvertex {} {} {}\nendloop\nendfacet\n", t[0], t[1], t[2], t[3], t[4], t[5], t[6], t[7], t[8]);
    }
    stl + "endsolid p\n"
}

#[test]
fn relief() {
    let tris = load_stl(pyramid_stl().as_bytes()).unwrap();
    assert_eq!(tris.len(), 4);
    // the same triangles as binary STL load identically
    let mut bin = vec![0u8; 80];
    bin.extend_from_slice(&4u32.to_le_bytes());
    for t in &tris {
        bin.extend_from_slice(&[0u8; 12]);
        for p in [t.a, t.b, t.c] {
            for v in [p.x, p.y, p.z] {
                bin.extend_from_slice(&(v as f32).to_le_bytes());
            }
        }
        bin.extend_from_slice(&[0u8; 2]);
    }
    assert_eq!(load_stl(&bin).unwrap(), tris);
    assert!(load_stl(b"solid nothing\nendsolid\n").is_err());

    let mut r = Relief { x: 1.0, y: 1.0, w: 2.0, h: 2.0, depth: 0.3, ..Default::default() };
    rasterize_relief(&tris, &mut r, 40000);
    assert!(r.nx > 100 && r.ny > 100);
    assert!(near_e(r.surface_at(2.0, 2.0), 0.0, 0.02)); // apex at the surface
    assert!(near_e(r.surface_at(1.05, 1.05), -0.3, 0.03)); // corner near the base
    assert_eq!(r.surface_at(0.5, 0.5), 0.0); // outside: uncut
    let ball = Tool { kind: ToolType::BallNose, diameter: 0.125, feed_xy: 100.0, feed_z: 50.0, ..Default::default() };
    let flat = Tool { diameter: 0.25, ..Default::default() };
    assert!(near_e(tip_height(&r, &ball, 2.0, 2.0, 0.0), 0.0, 0.02) && near_e(tip_height(&r, &flat, 2.0, 2.0, 0.0), 0.0, 0.02));
    assert!(tip_height(&r, &ball, 1.5, 2.0, 0.0) < tip_height(&r, &flat, 1.5, 2.0, 0.0)); // ball sits lower on the slope
    let ro = Op { kind: OpType::Relief, name: "relief".into(), stepover: 0.2, rough: true, rough_stepdown: 0.1, rough_stepover: 0.5, allowance: 0.02, clear_tool: 0, ..Default::default() };
    let rm = Material { w: 4.0, h: 4.0, t: 0.75, ..Default::default() };
    let rr = generate_relief(&ro, &ball, Some(&flat), &r, &rm, None);
    assert!(rr.paths.len() == 2 && rr.paths[0].name.contains("rough") && rr.paths[0].tool == 0);
    let (mut n_fin, mut zmin) = (0, 0.0_f64);
    for mv in linear(&rr.paths[1]) {
        let (x, y, z) = xyz(mv);
        n_fin += 1;
        zmin = zmin.min(z);
        assert!(z >= r.surface_at(x, y) - 1e-6); // the tip never dips below the surface
        assert!(z <= 1e-9 && (-1e-9..=4.0 + 1e-9).contains(&x));
    }
    assert!(n_fin > 200 && zmin < -0.2);
    let mut floors: Vec<i64> = linear(&rr.paths[0]).filter(|m| m.z.unwrap() < 0.0).map(|m| (m.z.unwrap() * 1000.0).round() as i64).collect();
    floors.sort();
    floors.dedup();
    assert_eq!(floors, vec![-200, -100]); // roughing layers; the -0.3 layer is all inside the 0.02 allowance
    assert!(estimate_minutes(&rr.paths[1], 100.0, 50.0, 400.0) > 0.0);
    // a boundary vector limits the machined area
    let bnd = [make_ellipse(2.0, 2.0, 0.6, 0.6)];
    let rc = generate_relief(&ro, &ball, None, &r, &rm, Some(&bnd));
    let (mut outside, mut inside) = (0, 0);
    for mv in linear(&rc.paths[0]).filter(|m| m.z.unwrap() < -1e-6) {
        if (mv.x.unwrap() - 2.0).hypot(mv.y.unwrap() - 2.0) > 0.6 + 0.01 {
            outside += 1;
        } else {
            inside += 1;
        }
    }
    assert!(outside == 0 && inside > 50);
}

// ---- safety properties over every generator ------------------------------------------------------------------------

/// No rapid moves sideways while the bit is (or ends up) below the surface.
fn assert_no_rapid_in_material(tp: &Toolpath, what: &str) {
    let (mut x, mut y, mut z): (Option<f64>, Option<f64>, Option<f64>) = (None, None, None);
    for (i, mv) in tp.moves.iter().enumerate() {
        let moved = |a: Option<f64>, b: Option<f64>| matches!((a, b), (Some(a), Some(b)) if (a - b).abs() > 1e-9);
        if mv.kind == MoveType::Rapid && (moved(mv.x, x) || moved(mv.y, y)) {
            assert!(z.is_none_or(|z| z >= -1e-9) && mv.z.is_none_or(|z| z >= -1e-9), "{what}: sideways rapid below Z0 at move {i}: {mv:?} from z {z:?}");
        }
        x = mv.x.or(x);
        y = mv.y.or(y);
        z = mv.z.or(z);
    }
}

#[test]
fn safety_properties_all_generators() {
    let tools = default_tools();
    let (em, eighth) = (tools[0].clone(), tools[1].clone());
    let vb = tools[2].clone();
    let m = Material { w: 12.0, h: 8.0, t: 0.75, ..Default::default() };
    let star = make_polyline(
        &[pt(8.0, 3.35), pt(7.8237, 2.8427), pt(7.2867, 2.8318), pt(7.7147, 2.5073), pt(7.5592, 1.9932), pt(8.0, 2.3), pt(8.4408, 1.9932), pt(8.2853, 2.5073), pt(8.7133, 2.8318), pt(8.1763, 2.8427)],
        true,
    );
    let shapes = vec![make_rect(1.0, 1.0, 3.0, 2.5), circ(5.0, 4.0, 0.8), circ(5.0, 4.0, 0.3), star, make_ellipse(2.0, 5.5, 0.9, 0.4), make_polyline(&[pt(1.0, 7.0), pt(4.0, 7.2), pt(6.0, 6.5)], false)];
    let mut ops = Vec::new();
    ops.push(Op { kind: OpType::Inlay, flat_depth: 0.2, stepdown: 0.1, ..Default::default() });
    for (kind, depth) in [(OpType::Profile, 0.75), (OpType::Pocket, 0.3), (OpType::Drill, 0.5), (OpType::VCarve, 0.0), (OpType::Bore, 0.4), (OpType::Flute, 0.2)] {
        for side in [Side::Outside, Side::Inside, Side::On] {
            for (entry, tabs, climb) in [(Entry::Plunge, false, true), (Entry::Spiral, true, false), (Entry::Ramp, true, true)] {
                ops.push(Op { kind, depth, side, entry, tabs, climb, tab_ramped: climb, stepdown: 0.2, peck: 0.15, ..Default::default() });
            }
        }
        if kind == OpType::VCarve {
            for (flat, start, sd) in [(0.15, 0.0, 0.05), (0.1, 0.05, 0.0), (0.3, 0.1, 0.1)] {
                ops.push(Op { kind, flat_depth: flat, start_depth: start, stepdown: sd, clear_tool: 0, ..Default::default() });
            }
        }
    }
    for op in &ops {
        let t = match op.kind {
            OpType::VCarve | OpType::Inlay => &vb,
            OpType::Pocket => &eighth,
            _ => &em,
        };
        let clear = if op.clear_tool >= 0 || op.kind == OpType::Inlay { Some(&em) } else { None };
        // inlay: the male blank spans all shapes (~2.5 s per op in C++ and Rust alike), so it gets two small shapes
        let r = generate(op, t, if op.kind == OpType::Inlay { &shapes[2..4] } else { &shapes }, &m, clear);
        // the deepest cut asked for: depth (profile/pocket/drill/bore/flute), the flat depth or board for v-carves
        let floor = match op.kind {
            OpType::VCarve | OpType::Inlay => if op.flat_depth > 0.0 { op.flat_depth } else { m.t },
            _ => op.depth,
        };
        for tp in &r.paths {
            let what = format!("{:?}/{:?}/{:?} {}", op.kind, op.side, op.entry, tp.name);
            assert!(!tp.moves.is_empty(), "{what}: empty path");
            assert_no_rapid_in_material(tp, &what);
            for mv in &tp.moves {
                let z = mv.z.unwrap_or(0.0);
                assert!(z >= -floor - 1e-9, "{what}: cuts to {z}, deeper than {floor}");
                assert!(mv.kind != MoveType::Rapid || z >= m.retract_z - 1e-9 || mv.x.is_none(), "{what}: rapid at z {z}");
            }
            assert_eq!(tp.moves[0].kind, MoveType::Rapid, "{what}: first move must be a rapid");
            assert!(tp.moves.last().unwrap().z.unwrap() >= m.retract_z - 1e-9, "{what}: ends below retract");
        }
        // the export of every op starts with a rapid to XY then down to safe Z, and stays out of the material sideways
        let mut all_tools = tools.clone();
        all_tools[op.tool as usize] = t.clone();
        for j in build_jobs(std::slice::from_ref(op), &all_tools, std::slice::from_ref(&r.paths), &m, "t") {
            let first = &j.toolpaths[0].moves;
            assert!(first[0].kind == MoveType::Rapid && first[0].z.is_none() && first[1].z == Some(m.safe_z) && first[1].x.is_none());
            for tp in &j.toolpaths {
                assert_no_rapid_in_material(tp, &j.filename);
            }
            let text = post_wincnc(&j);
            assert!(text.trim_end().ends_with("M5"), "spindle must be off at the end of {}", j.filename);
        }
        assert!(safety_issues(std::slice::from_ref(op), &all_tools, std::slice::from_ref(&r.paths), &m).iter().all(|s| !s.contains("rapid sideways")));
    }
}

#[test]
fn texture_strokes_stay_inside_and_shallow() {
    let t = &default_tools()[3]; // ball nose
    let op = Op { kind: OpType::Texture, name: "Tex".into(), depth: 0.06, ramp_length: 0.5, stepover: 0.8, vary: 0.5, ..Default::default() };
    let r = generate(&op, t, &[make_rect(0.0, 0.0, 4.0, 2.0)], &mat(), None);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let moves: Vec<&Move> = r.paths.iter().flat_map(|p| &p.moves).collect();
    let cuts: Vec<&&Move> = moves.iter().filter(|m| m.kind == MoveType::Linear).collect();
    assert!(cuts.len() > 200, "a 4 x 2 panel gets many strokes: {}", cuts.len());
    for m in &cuts {
        let (x, y, z) = (m.x.unwrap(), m.y.unwrap(), m.z.unwrap());
        assert!((-1e-9..=4.0 + 1e-9).contains(&x) && (-1e-9..=2.0 + 1e-9).contains(&y), "inside the shape: {x} {y}");
        assert!((-0.06 - 1e-9..=1e-9).contains(&z), "between surface and depth: {z}");
    }
    assert!(cuts.iter().any(|m| m.z.unwrap() < -0.05), "reaches near full depth");
    // same settings, same G-code (fixed seed)
    let again = generate(&op, t, &[make_rect(0.0, 0.0, 4.0, 2.0)], &mat(), None);
    assert_eq!(again.paths[0].moves.len(), r.paths[0].moves.len());
}

#[test]
fn photo_grooves_follow_darkness() {
    // 2 x 1 picture: left half black, right half white, in a 4 x 2 rectangle
    let px = [0u8, 0, 255, 255, 0, 0, 255, 255];
    let op = Op { kind: OpType::Photo, name: "Photo".into(), depth: 0.1, photo: gcarve_core::photo::b64_encode(&px), photo_w: 4, photo_h: 2, ..Default::default() };
    let vbit = &default_tools()[2];
    let r = generate(&op, vbit, &[make_rect(0.0, 0.0, 4.0, 2.0)], &mat(), None);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let cuts: Vec<&Move> = r.paths.iter().flat_map(|p| &p.moves).filter(|m| m.kind == MoveType::Linear).collect();
    assert!(!cuts.is_empty());
    assert!(cuts.iter().all(|m| m.x.unwrap() < 2.7), "white right part stays uncut");
    assert!(cuts.iter().any(|m| m.z.unwrap() < -0.095), "black part goes to full depth");
    let inv = Op { photo_invert: true, ..op.clone() };
    let r = generate(&inv, vbit, &[make_rect(0.0, 0.0, 4.0, 2.0)], &mat(), None);
    assert!(r.paths.iter().flat_map(|p| &p.moves).filter(|m| m.kind == MoveType::Linear).all(|m| m.x.unwrap() > 1.3), "inverted: the black part stays");
    let none = Op { photo: String::new(), ..op };
    assert!(!generate(&none, vbit, &[make_rect(0.0, 0.0, 4.0, 2.0)], &mat(), None).warnings.is_empty());
}

#[test]
fn array_copies_repeat_the_cut_across_the_grid() {
    let t = &default_tools()[0];
    let op = Op { kind: OpType::Profile, name: "Part".into(), depth: 0.2, array_rows: 2, array_cols: 3, array_dx: 5.0, array_dy: 4.0, ..Default::default() };
    let mut paths = generate(&op, t, &[make_rect(0.0, 0.0, 2.0, 1.0)], &mat(), None).paths;
    let one = paths.len();
    gcarve_core::cam::array_copies(&op, &mut paths);
    assert_eq!(paths.len(), one * 6);
    // every copy is complete (explicit X / Y) and the grid reaches the far cell
    let xs: Vec<f64> = paths.iter().flat_map(|p| &p.moves).filter_map(|m| m.x).collect();
    let ys: Vec<f64> = paths.iter().flat_map(|p| &p.moves).filter_map(|m| m.y).collect();
    let max = |v: &[f64]| v.iter().cloned().fold(f64::MIN, f64::max);
    assert!((max(&xs) - (10.0 + 2.0 + t.diameter / 2.0)).abs() < 1e-6, "{}", max(&xs));
    assert!((max(&ys) - (4.0 + 1.0 + t.diameter / 2.0)).abs() < 1e-6, "{}", max(&ys));
    for p in &paths[one..] {
        let first_xy = p.moves.iter().find(|m| m.x.is_some() || m.y.is_some()).unwrap();
        assert!(first_xy.x.is_some() && first_xy.y.is_some());
    }
}
