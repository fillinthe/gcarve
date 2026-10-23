//! A rotary rounding job goes through the real export with no safety complaints and A only turning forward.
use gcarve_core::document::{Document, Op, OpType, Tool};
use gcarve_core::geom::make_rect;
use gcarve_core::pipeline;

#[test]
fn a_rounding_job_exports_cleanly() {
    let mut doc = Document::default();
    let dia = 2.0;
    doc.material.rotary = Some(dia);
    doc.material.w = 10.0;
    doc.material.h = std::f64::consts::PI * dia;
    doc.material.t = dia;
    doc.material.safe_z = 0.8;
    doc.material.retract_z = 0.6;
    doc.material.post = "mach3-in".into();
    doc.tools = vec![Tool { name: "1/2 end mill".into(), diameter: 0.5, stepdown: 0.25, feed_xy: 100.0, feed_z: 30.0, rpm: 18000, ..Default::default() }];
    let id = doc.add(make_rect(1.0, 0.0, 6.0, doc.material.h), None);
    doc.ops.push(Op { kind: OpType::Pocket, name: "Round".into(), depth: 0.1, rounding: 2.0, stepover: 0.5, shape_ids: vec![id], ..Default::default() });
    doc.sync_sheet();
    let gen = pipeline::regenerate(&doc);
    assert!(gen.warnings.is_empty(), "{:?}", gen.warnings);
    let post = gcarve_core::postdef::find_post("mach3-in").unwrap();
    let out = pipeline::export(&doc, &gen, "r", "", 0.0, post);
    assert!(out.safety.is_empty(), "{:?}", out.safety);
    let text = &out.files[0].text;
    let a: Vec<f64> = text.lines().filter_map(|l| l.split_whitespace().find(|w| w.starts_with('A')).and_then(|w| w[1..].parse().ok())).collect();
    assert!(a.len() > 40 && a.windows(2).all(|w| w[1] >= w[0] - 1e-6), "{a:?}");
    eprintln!("minutes {:?}", gen.minutes);
}
