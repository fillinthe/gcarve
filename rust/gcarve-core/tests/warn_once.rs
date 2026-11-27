//! A warning that every shape of a toolpath earns is listed once (ticket #83).
use gcarve_core::document::{Document, Op, OpType};
use gcarve_core::geom::{make_polyline, Pt};

#[test]
fn many_open_lines_give_one_warning() {
    let mut doc = Document::default();
    let ids: Vec<_> = (0..40).map(|i| doc.add(make_polyline(&[Pt { x: 1.0, y: i as f64 * 0.2 }, Pt { x: 5.0, y: i as f64 * 0.2 }], false), None)).collect();
    let tool = doc.tools.iter().position(|t| t.kind == gcarve_core::document::ToolType::VBit).map_or_else(|| { doc.tools.push(gcarve_core::document::Tool { kind: gcarve_core::document::ToolType::VBit, diameter: 0.5, angle: 90.0, feed_xy: 100.0, feed_z: 50.0, rpm: 18000, ..Default::default() }); doc.tools.len() - 1 }, |i| i) as i32;
    doc.ops.push(Op { kind: OpType::VCarve, name: "Grid Lines".into(), shape_ids: ids, tool, depth: 0.1, ..Default::default() });
    let g = gcarve_core::pipeline::regenerate(&doc);
    assert_eq!(g.warnings.iter().filter(|w| w.contains("open shapes")).count(), 1, "{:?}", g.warnings);
}
