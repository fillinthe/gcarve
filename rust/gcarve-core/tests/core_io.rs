//! Ported from tests/core_test.cpp: importers, vector ops, dogbones, polygons, nesting.
#![allow(clippy::cloned_ref_to_slice_refs, clippy::field_reassign_with_default)]

use gcarve_core::dxf::import_dxf;
use gcarve_core::geom::{bounds, dist, make_polyline, make_rect, nearest, pt, Affine, Pt};
use gcarve_core::nest::{nest_shapes, NestPart, NestPlacement};
use gcarve_core::svg::import_svg;
use gcarve_core::vecops::*;

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn svg_units_viewbox_flip_transforms() {
    let svg = import_svg(
        r#"<?xml version="1.0"?>
      <svg xmlns="http://www.w3.org/2000/svg" width="25.4mm" height="25.4mm" viewBox="0 0 100 100">
        <g transform="translate(10,10)"><rect x="0" y="0" width="20" height="10"/></g>
        <circle cx="50" cy="50" r="10"/>
        <path d="M 60 60 h 10 v 10 h -10 z"/>
        <defs><rect width="999" height="999"/></defs>
        <rect x="0" y="0" width="30" height="30" rx="5" display="none"/>
      </svg>"#,
    );
    assert_eq!(svg.contours.len(), 3);
    let b0 = bounds(&svg.contours[0]);
    assert!(near(b0.x0, 0.1) && near(b0.x1, 0.3) && near(b0.y0, 0.8) && near(b0.y1, 0.9));
    let b1 = bounds(&svg.contours[1]);
    assert!(near(b1.w(), 0.2) && near(b1.h(), 0.2) && near((b1.x0 + b1.x1) / 2.0, 0.5));

    let arc = import_svg(r#"<svg width="96" height="96"><path d="M0 48 A48 48 0 0 1 96 48"/></svg>"#);
    assert_eq!(arc.contours.len(), 1);
    let ba = bounds(&arc.contours[0]);
    assert!(near(ba.w(), 1.0) && near(ba.h(), 0.5));
}

#[test]
fn dxf_units_bulges_joins() {
    let dxf = import_dxf(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
         0\nLWPOLYLINE\n90\n2\n70\n1\n10\n0\n20\n0\n42\n1\n10\n25.4\n20\n0\n\
         0\nLINE\n10\n0\n20\n50.8\n11\n25.4\n21\n50.8\n\
         0\nLINE\n10\n25.4\n20\n50.8\n11\n25.4\n21\n76.2\n\
         0\nCIRCLE\n10\n0\n20\n0\n40\n12.7\n\
         0\nSPLINE\n0\nENDSEC\n0\nEOF\n",
    );
    assert_eq!(dxf.contours.len(), 3); // bulge shape, joined 2-line chain, circle
    assert!(dxf.note.contains("mm") && dxf.note.contains("SPLINE"));
    let bd = bounds(&dxf.contours[0]);
    assert!(near(bd.w(), 1.0) && near(bd.h(), 0.5) && near(bd.y1, 0.0));
    assert!(dxf.contours[2].nodes.len() == 3 && !dxf.contours[2].closed);
}

#[test]
fn booleans_offsets_polygons() {
    let r = |x0, y0, x1, y1| make_rect(x0, y0, x1, y1);
    let weld = boolean_op(BoolOp::Weld, &[r(0.0, 0.0, 2.0, 2.0)], &[r(1.0, 1.0, 3.0, 3.0)]);
    assert!(weld.len() == 1 && near(bounds(&weld[0]).w(), 3.0) && weld[0].nodes.len() == 8);
    assert_eq!(boolean_op(BoolOp::Subtract, &[r(0.0, 0.0, 3.0, 3.0)], &[r(1.0, 1.0, 2.0, 2.0)]).len(), 2); // outer + hole
    let inter = boolean_op(BoolOp::Intersect, &[r(0.0, 0.0, 2.0, 2.0)], &[r(1.0, 1.0, 3.0, 3.0)]);
    assert!(inter.len() == 1 && near(bounds(&inter[0]).w(), 1.0) && near(bounds(&inter[0]).x0, 1.0));
    let off = offset_contours(&[r(0.0, 0.0, 2.0, 2.0)], 0.5, true);
    assert!(off.len() == 1 && near(bounds(&off[0]).w(), 3.0) && near(bounds(&off[0]).x0, -0.5));
    let inn = offset_contours(&[r(0.0, 0.0, 2.0, 2.0)], -0.5, true);
    assert!(inn.len() == 1 && near(bounds(&inn[0]).w(), 1.0));
    assert!(offset_contours(&[r(0.0, 0.0, 2.0, 2.0)], -1.5, true).is_empty());
    let stroke = offset_contours(&[make_polyline(&[pt(0.0, 0.0), pt(2.0, 0.0)], false)], 0.25, true);
    assert!(stroke.len() == 1 && stroke[0].closed && near(bounds(&stroke[0]).h(), 0.5) && near(bounds(&stroke[0]).w(), 2.5));

    let hex = make_polygon(pt(0.0, 0.0), 1.0, 6, 0.0);
    assert!(hex.closed && hex.nodes.len() == 6 && near(bounds(&hex).h(), 2.0));
    let star = make_star(pt(0.0, 0.0), 1.0, 0.4, 5, 0.0);
    assert!(star.nodes.len() == 10 && near(dist(star.nodes[0].p, pt(0.0, 0.0)), 1.0) && near(dist(star.nodes[1].p, pt(0.0, 0.0)), 0.4));
    let cl = close_contour(make_polyline(&[pt(0.0, 0.0), pt(1.0, 0.0), pt(1.0, 1.0)], false));
    assert!(cl.closed && cl.nodes.len() == 3);
}

#[test]
fn dogbones_reach_corners() {
    // a bit of radius r reaches every hole corner (inset by r, then out by r, covers the corners)
    let r = 0.125;
    for tb in [false, true] {
        let db = dogbone(&make_rect(0.0, 0.0, 4.0, 2.0), r + 0.0005, true, tb); // + clearance, as Canvas::dogboneSelection
        let reach = offset_contours(&offset_contours(&[db.clone()], -r, true), r, true);
        assert_eq!(reach.len(), 1);
        for corner in [pt(0.0, 0.0), pt(4.0, 0.0), pt(4.0, 2.0), pt(0.0, 2.0)] {
            assert!(nearest(&reach[0], corner).d < 0.005); // plain rect: 0.052
        }
        assert!(bounds(&db).x0 < -0.03); // lobes go into the material
    }
    assert_eq!(dogbone(&make_rect(0.0, 0.0, 4.0, 2.0), r, false, false).nodes.len(), 4); // a plain part has no inside corners
    let l = make_polyline(&[pt(0.0, 0.0), pt(4.0, 0.0), pt(4.0, 1.0), pt(1.0, 1.0), pt(1.0, 3.0), pt(0.0, 3.0)], true);
    assert!(dogbone(&l, r, false, false).nodes.len() > 6); // the L's inside corner gets a lobe
}

#[test]
fn box_nesting() {
    // four boxes on an 18 x 4 sheet (usable 17 x 3: one shelf)
    let it = |w, h| NestItem { w, h };
    let pl = nest_boxes(&[it(3.0, 2.0), it(3.0, 2.0), it(3.0, 2.0), it(2.0, 6.0)], 18.0, 4.0, 0.25, 0.5, true);
    assert_eq!(pl.iter().filter(|p| p.placed).count(), 4);
    assert!(pl[3].rotated); // the 2 x 6 only fits lying down
    for i in 0..pl.len() {
        let (w, h) = (if pl[i].rotated { 6.0 } else { 3.0 }, 2.0);
        assert!(pl[i].x >= 0.5 - 1e-9 && pl[i].y >= 0.5 - 1e-9 && pl[i].x + w <= 17.5 + 1e-9 && pl[i].y + h <= 3.5 + 1e-9);
        for j in i + 1..pl.len() {
            let wj = if pl[j].rotated { 6.0 } else { 3.0 };
            let sep = pl[i].x + w + 0.25 <= pl[j].x + 1e-9
                || pl[j].x + wj + 0.25 <= pl[i].x + 1e-9
                || pl[i].y + h + 0.25 <= pl[j].y + 1e-9
                || pl[j].y + h + 0.25 <= pl[i].y + 1e-9;
            assert!(sep);
        }
    }
    assert!(!nest_boxes(&[it(20.0, 1.0)], 10.0, 4.0, 0.25, 0.5, true)[0].placed);
    let two = nest_boxes(&[it(3.0, 2.0), it(3.0, 2.0), it(3.0, 2.0)], 10.0, 4.0, 0.25, 0.5, false);
    assert_eq!(two.iter().filter(|p| p.placed).count(), 2); // usable 9 x 3: only two fit on the single shelf
}

#[test]
fn true_shape_nesting() {
    // two 4x4 right triangles fit a 5 x 5 usable area only by interlocking
    let tri = NestPart { outline: vec![pt(0.0, 0.0), pt(4.0, 0.0), pt(0.0, 4.0)] };
    let tris = [tri.clone(), tri];
    let pl = nest_shapes(&tris, 6.0, 6.0, 0.2, 0.5, 90);
    assert!(pl[0].placed && pl[1].placed);
    assert!((pl[0].rotation - pl[1].rotation).abs() > 1.0); // one is turned around
    let place = |p: &NestPart, q: &NestPlacement| -> Vec<Pt> {
        let m = Affine::translate(q.offset.x, q.offset.y) * Affine::rotate(q.rotation);
        p.outline.iter().map(|&v| m.apply(v)).collect()
    };
    let (o0, o1) = (place(&tris[0], &pl[0]), place(&tris[1], &pl[1]));
    for v in o0.iter().chain(&o1) {
        assert!(v.x > 0.49 && v.y > 0.49 && v.x < 5.51 && v.y < 5.51);
    }
    let inter = boolean_op(BoolOp::Intersect, &[make_polyline(&o0, true)], &[make_polyline(&o1, true)]);
    let ia: f64 = inter.iter().map(|c| bounds(c).w() * bounds(c).h()).sum();
    assert!(ia < 1e-3);
    let none = nest_shapes(&tris, 6.0, 6.0, 0.2, 0.5, 0); // without rotation they can't both fit
    assert!(!(none[0].placed && none[1].placed));
    let big = [NestPart { outline: vec![pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 10.0), pt(0.0, 10.0)] }];
    assert!(!nest_shapes(&big, 6.0, 6.0, 0.2, 0.5, 90)[0].placed);
}
