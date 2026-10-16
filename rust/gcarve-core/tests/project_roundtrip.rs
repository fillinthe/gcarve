//! .tlp projects and the clipboard (C++ app/project.cpp; assertions from tests/canvas_test.cpp projectRoundTrip /
//! clipboard). The samples were written by the C++ app, so saving them again must reproduce the files byte for byte.
#![allow(clippy::cloned_ref_to_slice_refs, clippy::field_reassign_with_default)]

use std::collections::BTreeSet;

use gcarve_core::document::*;
use gcarve_core::geom::{dist, make_rect, pt};
use gcarve_core::project::*;

fn repo(p: &str) -> String {
    format!("{}/../../{p}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn samples_load_and_save_byte_identical() {
    for f in ["samples/showcase.tlp", "samples/compare/compare.tlp"] {
        let text = std::fs::read_to_string(repo(f)).unwrap();
        let d = load_project(&text).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert!(!d.shapes.is_empty() && !d.ops.is_empty(), "{f}");
        let saved = save_project(&d);
        std::fs::write(format!("{}/{}", env!("CARGO_TARGET_TMPDIR"), f.replace('/', "_")), &saved).unwrap(); // for diffing
        assert_eq!(saved, text, "{f}: Rust save differs from the C++ file");
        assert_eq!(load_project(&saved).unwrap(), d, "{f}: reload differs");
    }
}

#[test]
fn showcase_contents() {
    let d = load_project(&std::fs::read_to_string(repo("samples/showcase.tlp")).unwrap()).unwrap();
    assert_eq!(d.sheets.len(), 2);
    assert!(d.layers.len() >= 5 && d.layers[1].name == "Pockets");
    assert!(!d.variables.is_empty() && d.dimensions.len() == 1);
    let kinds: BTreeSet<_> = d.ops.iter().map(|o| o.kind as i32).collect();
    assert!(kinds.len() >= 6, "toolpath types: {kinds:?}");
    // new ids continue after the loaded ones
    let mut d = d;
    let id = d.add(make_rect(0.0, 0.0, 1.0, 1.0), None);
    assert!(d.shapes[..d.shapes.len() - 1].iter().all(|s| s.id < id));
}

#[test]
fn round_trip_and_rejects() {
    let mut d = Document::default();
    let a = d.add(make_rect(0.0, 0.0, 2.0, 1.0), None);
    let l2 = d.layer_named("Pockets");
    let b = d.add(gcarve_core::geom::make_ellipse(5.0, 5.0, 1.0, 1.0), Some(l2));
    d.shapes[1].group = 2;
    d.shapes[1].bind.insert("w".into(), "box-w".into());
    d.variables.push(Variable { name: "box-w".into(), expr: "4in".into() });
    d.material.kind = "MDF".into();
    d.material.w = 24.0;
    d.material.bind.insert("t".into(), "0.5in".into());
    d.sync_sheet();
    d.bare_unit_in = 1.0 / 25.4;
    d.tools[0].presets.push(FeedPreset { material: "MDF".into(), feed_xy: 250.0, feed_z: 90.0, stepdown: 0.2, rpm: 16000 });
    d.ops.push(Op { name: "Outer cut".into(), shape_ids: vec![a, b], tabs: true, entry: Entry::Ramp, ..Default::default() });
    d.ops.push(Op { kind: OpType::VCarve, side: Side::On, clear_tool: 1, layer_scope: l2, ..Default::default() });
    d.texts.push(TextItem { id: 3, text: "Hi \"there\"\n".into(), ..Default::default() });
    d.next_text_id = 4;
    d.constraints.push(Constraint { anchor: a, target: b, kind: "above".into(), expr: "1".into() });
    d.dimensions.push(Dimension { a: pt(0.0, 0.0), b: pt(2.0, 0.0), shape_a: a, node_a: 0, ..Default::default() });
    d.reliefs.push(Relief { id: 1, name: "pyr".into(), source: "/x/pyramid.stl".into(), mirror: true, ..Default::default() });
    d.next_relief_id = 2;

    let saved = save_project(&d);
    std::fs::write(format!("{}/rust_features.tlp", env!("CARGO_TARGET_TMPDIR")), &saved).unwrap(); // opened by the C++ app
    let back = load_project(&saved).unwrap();
    assert_eq!(back, d);
    assert_eq!(back.tools[0].presets[0].rpm, 16000);
    assert_eq!(back.layers[1].name, "Pockets");
    assert!(dist(back.shapes[1].c.nodes[0].p, d.shapes[1].c.nodes[0].p) < 1e-12);

    assert!(load_project("{\"nope\": 1}").is_err());
    assert!(load_project("not json").is_err());
    assert!(load_project(r#"{"app":"GCarve","version":2}"#).unwrap_err().contains("newer"));
    // legacy ToolpathLab files load; missing sections get defaults
    let mut old = load_project(r#"{"app":"ToolpathLab","version":1,"shapes":[{"id":7,"layer":9,"closed":true,"nodes":[{"p":[0,0]},{"p":[1,0],"in":[0.5,0.5]}]}],
        "ops":[{"type":"pocket","shapes":[7],"tool":99,"clearTool":99}]}"#)
    .unwrap();
    assert_eq!(old.tools, default_tools());
    assert_eq!(old.shapes[0].layer, 0); // clamped to the layer list
    assert!(old.shapes[0].c.nodes[1].has_in && !old.shapes[0].c.nodes[1].has_out);
    assert_eq!((old.ops[0].kind, old.ops[0].tool, old.ops[0].clear_tool), (OpType::Pocket, 3, 3));
    assert_eq!(old.add(make_rect(0.0, 0.0, 1.0, 1.0), None), 8);
}

#[test]
fn clipboard_carries_toolpaths_and_tools() {
    let mut d = Document::default();
    let a = d.add(make_rect(0.0, 0.0, 2.0, 1.0), None);
    let b = d.add(make_rect(3.0, 0.0, 4.0, 1.0), None);
    d.shapes[0].group = 5;
    d.shapes[0].text = 1;
    d.texts.push(TextItem { id: 1, text: "A".into(), path_shape: b, ..Default::default() });
    d.next_text_id = 2;
    d.tools[0].name = "My 1/4 bit".into();
    d.ops.push(Op { name: "Outer".into(), shape_ids: vec![a], tabs: true, ..Default::default() });
    d.ops.push(Op { name: "Both".into(), shape_ids: vec![a, b], tool: 1, ..Default::default() });
    d.constraints.push(Constraint { anchor: a, target: b, ..Default::default() });

    let only_a = save_clip(&d, &BTreeSet::from([a]));
    assert!(!only_a.contains('\n'));
    // paste back: tool matched by name, not duplicated; the group gets a fresh id
    let ids = load_clip(&only_a, &mut d, pt(1.0, 1.0));
    assert_eq!(ids.len(), 1);
    let n = *ids.iter().next().unwrap();
    assert!(n != a && d.ops.len() == 3 && d.ops[2].shape_ids == vec![n] && d.ops[2].tabs);
    assert_eq!(d.tools.len(), 4);
    assert_eq!(d.tools[d.ops[2].tool as usize].name, "My 1/4 bit");
    let ns = d.find(n).unwrap();
    assert_eq!((ns.group, ns.text), (6, 2));
    assert!((ns.c.nodes[0].p.x - 1.0).abs() < 1e-12);
    assert_eq!(d.texts.len(), 2);
    assert_eq!(d.texts[1].path_shape, 0); // its path shape was not copied
    assert_eq!(d.constraints.len(), 1); // the constraint needs both shapes

    // a fresh project whose library lacks that bit -> the bit is appended
    let mut fresh = Document::default();
    let ids = load_clip(&save_clip(&d, &BTreeSet::from([a, b])), &mut fresh, pt(0.0, 0.0));
    assert_eq!(ids.len(), 2);
    assert_eq!(fresh.tools.len(), 5);
    assert_eq!(fresh.tools[fresh.ops[0].tool as usize].name, "My 1/4 bit");
    assert_eq!(fresh.ops.len(), 2);
    assert_eq!(fresh.constraints.len(), 1);
    assert_eq!(fresh.texts[0].path_shape, *ids.iter().nth(1).unwrap());

    assert!(load_clip("{\"app\":\"GCarve\"}", &mut fresh, pt(0.0, 0.0)).is_empty()); // not a clip
}

#[test]
fn tool_library() {
    let mut tools = default_tools();
    tools[2].presets.push(FeedPreset { material: "Oak".into(), ..Default::default() });
    assert_eq!(tool_library_from_json(&tool_library_to_json(&tools)), tools);
    assert_eq!(tool_library_from_json(""), default_tools());
}
