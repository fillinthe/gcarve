//! Importers, vector ops and nesting vs the C++ engine: rust/testdata/io/golden.json is written by
//! rust/testdata/io/gen/dump.cpp (same inputs). Imports/polygons/dogbones match to 1e-6, Clipper results to 1e-4.
#![allow(clippy::cloned_ref_to_slice_refs, clippy::field_reassign_with_default)]

use gcarve_core::dxf::import_dxf;
use gcarve_core::geom::{make_ellipse, make_polyline, make_rect, pt, Contour, Node, Pt};
use gcarve_core::nest::{nest_shapes, NestPart};
use gcarve_core::svg::{import_svg, ImportResult};
use gcarve_core::vecops::*;
use serde_json::Value;

fn repo(p: &str) -> String {
    format!("{}/../../{p}", env!("CARGO_MANIFEST_DIR"))
}

fn golden() -> Value {
    serde_json::from_str(&std::fs::read_to_string(repo("rust/testdata/io/golden.json")).unwrap()).unwrap()
}

fn p(v: &Value) -> Pt {
    pt(v[0].as_f64().unwrap(), v[1].as_f64().unwrap())
}

fn contour(v: &Value) -> Contour {
    let nodes = v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| Node {
            p: p(&n["p"]),
            inh: n.get("in").map_or(Pt::default(), p),
            out: n.get("out").map_or(Pt::default(), p),
            has_in: n.get("in").is_some(),
            has_out: n.get("out").is_some(),
        })
        .collect();
    Contour { nodes, closed: v["closed"].as_bool().unwrap() }
}

/// max coordinate difference; panics on structural mismatch
fn diff(what: &str, got: &Contour, want: &Contour) -> f64 {
    assert_eq!(got.closed, want.closed, "{what}: closed");
    assert_eq!(got.nodes.len(), want.nodes.len(), "{what}: node count");
    let mut m = 0.0f64;
    for (i, (a, b)) in got.nodes.iter().zip(&want.nodes).enumerate() {
        assert_eq!((a.has_in, a.has_out), (b.has_in, b.has_out), "{what}: handles of node {i}");
        let d = |x: Pt, y: Pt| (x.x - y.x).abs().max((x.y - y.y).abs());
        m = m.max(d(a.p, b.p));
        if a.has_in {
            m = m.max(d(a.inh, b.inh));
        }
        if a.has_out {
            m = m.max(d(a.out, b.out));
        }
    }
    m
}

fn diff_all(what: &str, got: &[Contour], want: &Value) -> f64 {
    let want: Vec<Contour> = want.as_array().unwrap().iter().map(contour).collect();
    assert_eq!(got.len(), want.len(), "{what}: contour count");
    got.iter().zip(&want).enumerate().map(|(i, (a, b))| diff(&format!("{what}[{i}]"), a, b)).fold(0.0, f64::max)
}

fn check_import(what: &str, r: &ImportResult, want: &Value) -> f64 {
    assert_eq!(r.note, want["note"].as_str().unwrap(), "{what}: note");
    let layers: Vec<&str> = want["layers"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
    assert_eq!(r.layers, layers, "{what}: layers");
    diff_all(what, &r.contours, &want["contours"])
}

const INLINE_SVG: &str = r##"<?xml version="1.0"?>
      <svg xmlns="http://www.w3.org/2000/svg" xmlns:inkscape="x" width="25.4mm" height="25.4mm" viewBox="0 0 100 100">
        <!-- comment <rect/> -->
        <g transform="translate(10,10)" inkscape:label="Cut"><rect x="0" y="0" width="20" height="10"/></g>
        <g id="g123" transform="rotate(30 50 50) scale(0.5)"><circle cx="50" cy="50" r="10"/><ellipse cx="20" cy="30" rx="8" ry="3"/></g>
        <path d="M 60 60 h 10 v 10 h -10 z m 5 5 l 2 0 0 2 z"/>
        <path d="M10 80 C 20 70 30 90 40 80 S 60 70 70 80 Q 80 90 90 80 T 95 95 L 97 97"/>
        <path d="M5 5 A 10 5 30 1 0 25 15 a 3 3 0 0 1 6 0"/>
        <rect x="70" y="5" width="20" height="15" rx="4"/>
        <polygon points="1,1 9,1 5,9"/><polyline points="20 1 25 4 30 1"/><line x1="0" y1="99" x2="99" y2="99"/>
        <g transform="matrix(1 0 0.2 1 0 0) skewX(5)"><rect x="40" y="40" width="5" height="5"/></g>
        <text>hi</text><use href="#a"/>
        <defs><rect width="999" height="999"/></defs>
        <rect x="0" y="0" width="30" height="30" rx="5" display="none"/>
      </svg>"##;

const INLINE_DXF: &str = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
0\nLWPOLYLINE\n8\nCUT\n90\n2\n70\n1\n10\n0\n20\n0\n42\n1\n10\n25.4\n20\n0\n\
0\nLINE\n10\n0\n20\n50.8\n11\n25.4\n21\n50.8\n\
0\nLINE\n10\n25.4\n20\n50.8\n11\n25.4\n21\n76.2\n\
0\nCIRCLE\n8\n0\n10\n0\n20\n0\n40\n12.7\n\
0\nARC\n10\n100\n20\n0\n40\n10\n50\n300\n51\n45\n230\n-1\n\
0\nPOLYLINE\n70\n1\n0\nVERTEX\n10\n200\n20\n0\n42\n-0.5\n0\nVERTEX\n10\n220\n20\n0\n0\nVERTEX\n10\n220\n20\n20\n0\nSEQEND\n\
0\nSPLINE\n0\nTEXT\n0\nTEXT\n0\nENDSEC\n0\nEOF\n";

#[test]
fn imports_match_cpp() {
    let g = golden();
    let mut max = 0.0f64;
    for f in ["samples/demo.svg", "samples/demo.dxf", "samples/compare/compare.dxf"] {
        let text = std::fs::read_to_string(repo(f)).unwrap();
        let r = if f.ends_with(".svg") { import_svg(&text) } else { import_dxf(&text) };
        max = max.max(check_import(f, &r, &g[format!("import:{f}")]));
    }
    max = max.max(check_import("inline.svg", &import_svg(INLINE_SVG), &g["import:inline.svg"]));
    max = max.max(check_import("inline.dxf", &import_dxf(INLINE_DXF), &g["import:inline.dxf"]));
    println!("imports: max |rust - c++| = {max:e}");
    assert!(max < 1e-6, "{max}");
}

#[test]
fn vecops_match_cpp() {
    let g = golden();
    let rect = make_rect(0.0, 0.0, 2.0, 2.0);
    let rect2 = make_rect(1.0, 1.0, 3.0, 3.0);
    let circ = make_ellipse(2.0, 1.0, 1.2, 0.8);
    let l = make_polyline(&[pt(0.0, 0.0), pt(4.0, 0.0), pt(4.0, 1.0), pt(1.0, 1.0), pt(1.0, 3.0), pt(0.0, 3.0)], true);
    let star = make_star(pt(1.0, 1.0), 1.0, 0.4, 5, 0.1);
    let open = make_polyline(&[pt(0.0, 0.0), pt(2.0, 0.0), pt(2.0, 1.0)], false);

    let clipper = [
        ("weld", boolean_op(BoolOp::Weld, &[rect.clone()], &[rect2.clone(), circ.clone()])),
        ("subtract", boolean_op(BoolOp::Subtract, &[make_rect(0.0, 0.0, 3.0, 3.0)], &[make_rect(1.0, 1.0, 2.0, 2.0), circ.clone()])),
        ("intersect", boolean_op(BoolOp::Intersect, &[rect.clone()], &[circ.clone()])),
        ("xor", boolean_op(BoolOp::Xor, &[rect.clone()], &[rect2])),
        ("offset+0.5", offset_contours(&[rect, circ], 0.5, true)),
        ("offset-0.3", offset_contours(&[l.clone()], -0.3, true)),
        ("offset-miter", offset_contours(&[star.clone()], 0.1, false)),
        ("stroke-round", offset_contours(&[open.clone()], 0.25, true)),
        ("stroke-square", offset_contours(&[open.clone()], -0.25, false)),
    ];
    let mut max = 0.0f64;
    for (k, got) in &clipper {
        max = max.max(diff_all(k, got, &g[*k]));
    }
    println!("booleans/offsets: max |rust - c++| = {max:e}");
    assert!(max < 1e-4, "{max}");

    let exact = [
        ("dogbone-in", dogbone(&make_rect(0.0, 0.0, 4.0, 2.0), 0.1255, true, false)),
        ("tbone-in", dogbone(&make_rect(0.0, 0.0, 4.0, 2.0), 0.1255, true, true)),
        ("dogbone-L-out", dogbone(&l, 0.125, false, false)),
        ("tbone-L-out", dogbone(&l, 0.125, false, true)),
        ("polygon", make_polygon(pt(1.0, 2.0), 1.5, 7, 0.3)),
        ("star", star),
        ("close", close_contour(open)),
    ];
    let mut max = 0.0f64;
    for (k, got) in &exact {
        max = max.max(diff(k, got, &contour(&g[*k])));
    }
    let joined = join_open(
        &[
            make_polyline(&[pt(0.0, 0.0), pt(1.0, 0.0)], false),
            make_polyline(&[pt(1.0, 1.0), pt(1.0, 0.0)], false),
            make_polyline(&[pt(1.0, 1.0), pt(0.0, 0.0)], false),
        ],
        1e-4,
    );
    max = max.max(diff_all("join", &joined, &g["join"]));
    println!("dogbones/polygons: max |rust - c++| = {max:e}");
    assert!(max < 1e-6, "{max}");
}

#[test]
fn nesting_matches_cpp() {
    let g = golden();
    let items: Vec<NestItem> = [(3.0, 2.0), (3.0, 2.0), (3.0, 2.0), (2.0, 6.0), (5.0, 1.0), (1.0, 1.0), (4.0, 3.0)]
        .iter()
        .map(|&(w, h)| NestItem { w, h })
        .collect();
    for (got, want) in nest_boxes(&items, 18.0, 8.0, 0.25, 0.5, true).iter().zip(g["nestBoxes"].as_array().unwrap()) {
        assert_eq!(got.placed, want["placed"].as_bool().unwrap());
        assert_eq!(got.rotated, want["rotated"].as_bool().unwrap());
        assert!((got.x - want["x"].as_f64().unwrap()).abs() < 1e-9 && (got.y - want["y"].as_f64().unwrap()).abs() < 1e-9);
    }
    let poly = |v: &[(f64, f64)]| NestPart { outline: v.iter().map(|&(x, y)| pt(x, y)).collect() };
    let parts = [
        poly(&[(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)]),
        poly(&[(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)]),
        poly(&[(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (0.0, 1.0)]),
        poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 3.0), (0.0, 3.0)]),
        poly(&[(0.0, 0.0), (9.0, 0.0), (9.0, 9.0)]),
    ];
    // True-shape nesting no longer follows the C++ engine: its collision region only covered the outline of a placed
    // part, so a small part could land inside a bigger one (the golden has the 2 x 1 rectangle on top of a triangle).
    // The Rust engine includes the interior (nest.rs); here: the golden really overlaps, the Rust result never does.
    let outline = |i: usize, rot: f64, off: gcarve_core::geom::Pt| -> Vec<gcarve_core::geom::Pt> {
        let (sn, cs) = rot.sin_cos();
        parts[i].outline.iter().map(|q| pt(cs * q.x - sn * q.y + off.x, sn * q.x + cs * q.y + off.y)).collect()
    };
    let overlaps = |shapes: &[Option<Vec<gcarve_core::geom::Pt>>]| -> f64 {
        let mut worst = 0.0f64;
        for i in 0..shapes.len() {
            for j in i + 1..shapes.len() {
                if let (Some(a), Some(b)) = (&shapes[i], &shapes[j]) {
                    let r = gcarve_core::vecops::boolean_op(gcarve_core::vecops::BoolOp::Intersect, &[gcarve_core::geom::make_polyline(a, true)], &[gcarve_core::geom::make_polyline(b, true)]);
                    let area: f64 = r.iter().map(|c| { let p = gcarve_core::geom::flatten(c, 0.001); (0..p.len()).map(|k| { let (u, v) = (p[k], p[(k + 1) % p.len()]); u.x * v.y - v.x * u.y }).sum::<f64>().abs() / 2.0 }).sum();
                    worst = worst.max(area);
                }
            }
        }
        worst
    };
    let cpp: Vec<_> = g["nestShapes"].as_array().unwrap().iter().enumerate().map(|(i, w)| w["placed"].as_bool().unwrap().then(|| outline(i, w["rotation"].as_f64().unwrap(), p(&w["offset"])))).collect();
    let got = nest_shapes(&parts, 8.0, 7.0, 0.2, 0.5, 90);
    let rust: Vec<_> = got.iter().enumerate().map(|(i, a)| a.placed.then(|| outline(i, a.rotation, a.offset))).collect();
    println!("nestShapes: worst overlap c++ {:.3} sq in, rust {:.6}", overlaps(&cpp), overlaps(&rust));
    assert!(overlaps(&cpp) > 0.5, "the C++ golden was expected to overlap");
    assert!(overlaps(&rust) < 1e-6);
    assert!(got.iter().filter(|a| a.placed).count() >= 3);
}

#[test]
fn a_toolpath_can_keep_to_one_sheet_or_take_every_vector() {
    use gcarve_core::document::{Document, Op};
    use gcarve_core::geom::make_rect;
    let mut d = Document::default();
    d.sheets = vec![d.material.clone(), d.material.clone()];
    let a = d.add(make_rect(0.0, 0.0, 1.0, 1.0), None);
    d.set_active_sheet(1);
    let b = d.add(make_rect(0.0, 0.0, 2.0, 2.0), None);
    let all = Op { all_vectors: true, ..Default::default() };
    assert_eq!((d.op_shape_ids_on(&all, 0), d.op_shape_ids_on(&all, 1)), (vec![a], vec![b]), "all vectors: each sheet's own");
    let one = Op { all_vectors: true, sheet_scope: 1, ..Default::default() };
    assert!(d.op_shape_ids_on(&one, 0).is_empty() && d.op_shape_ids_on(&one, 1) == vec![b], "kept to sheet 2");
    // saved and read back
    d.ops.push(one.clone());
    let back = gcarve_core::project::load_project(&gcarve_core::project::save_project(&d)).unwrap();
    assert_eq!((back.ops[0].sheet_scope, back.ops[0].all_vectors), (1, true));
}
