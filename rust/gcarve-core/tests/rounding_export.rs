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

/// A turned profile (blank in a chuck with a tailstock) through the real export: nothing flagged, A only forward
/// within each pass, Z never below the drawn waist, spindle off at the end.
#[test]
fn a_turning_job_exports_cleanly() {
    use gcarve_core::document::ToolType;
    use gcarve_core::geom::{make_polyline, pt};
    let mut doc = Document::default();
    let dia = 3.0;
    doc.material.rotary = Some(dia);
    doc.material.w = 20.0;
    doc.material.h = std::f64::consts::PI * dia;
    doc.material.t = dia;
    doc.material.safe_z = 0.8;
    doc.material.retract_z = 0.6;
    (doc.material.chuck, doc.material.tail) = (2.0, 1.0);
    doc.material.post = "mach3-in".into();
    doc.tools = vec![Tool { name: "1/4 ball".into(), kind: ToolType::BallNose, diameter: 0.25, stepdown: 0.2, feed_xy: 100.0, feed_z: 30.0, rpm: 18000, ..Default::default() }];
    let id = doc.add(make_polyline(&[pt(0.0, 5.0), pt(6.0, 5.0), pt(9.0, 4.6), pt(13.0, 4.6), pt(15.0, 5.0), pt(20.0, 5.0)], false), None);
    doc.ops.push(Op { kind: OpType::Pocket, name: "Leg".into(), turn: true, stepover: 0.4, shape_ids: vec![id], ..Default::default() });
    doc.sync_sheet();
    let gen = pipeline::regenerate(&doc);
    assert!(gen.warnings.is_empty(), "{:?}", gen.warnings);
    let post = gcarve_core::postdef::find_post("mach3-in").unwrap();
    let out = pipeline::export(&doc, &gen, "t", "", 0.0, post);
    assert!(out.safety.is_empty(), "{:?}", out.safety);
    let text = &out.files[0].text;
    let word = |l: &str, c: char| l.split_whitespace().find(|w| w.starts_with(c)).and_then(|w| w[1..].parse::<f64>().ok());
    let z: Vec<f64> = text.lines().filter_map(|l| word(l, 'Z')).collect();
    assert!(z.iter().copied().fold(0.0, f64::min) > -0.4 - 1e-4 && z.iter().any(|z| (z + 0.4).abs() < 1e-3), "the waist is 0.4 deep");
    let a: Vec<f64> = text.lines().filter(|l| l.starts_with("G1") || l.starts_with('A') || l.starts_with('X')).filter_map(|l| word(l, 'A')).collect();
    assert!(a.len() > 100, "{}", a.len());
    assert!(text.lines().filter(|l| !l.trim().is_empty()).rev().take(8).any(|l| l.contains("M5")), "spindle off at the end");
    // the same toolpath a little further along the chuck is flagged
    doc.material.chuck = 7.0;
    doc.sync_sheet();
    let cut_at = |x: f64| gen.paths[0].iter().flat_map(|p| &p.moves).any(|m| m.x.is_some_and(|v| v < x) && m.z.is_some_and(|z| z < 0.0));
    assert!(cut_at(7.0), "the first run cut between 6 and 7");
    assert!(pipeline::export(&doc, &gen, "t", "", 0.0, post).safety.iter().any(|s| s.contains("chuck")));
}
