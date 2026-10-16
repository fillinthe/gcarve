//! Toolpath templates across engines: rust/testdata/template/* are written by the C++ canvas_test
//! (templatesCrossEngine, GCARVE_UPDATE_GOLDEN=1) and checked by both, so a template saved by either app applies
//! identically in the other: same .gctemplate bytes, same project after applying, same summary.

use gcarve_core::project::{load_project, save_project};
use gcarve_core::template::*;

fn read(p: &str) -> String {
    std::fs::read_to_string(format!("{}/../../{p}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn template_matches_cpp() {
    let src = load_project(&read("samples/showcase.tlp")).unwrap();
    let saved = save_template(&make_template(&src, "Showcase", "Every toolpath type of the showcase sign"));
    assert_eq!(saved, read("rust/testdata/template/showcase.gctemplate"), "Rust template differs from the C++ one");

    let t = load_template(&saved).unwrap();
    let mut dst = load_project(&read("rust/testdata/template/target.tlp")).unwrap();
    let r = apply_template(&t, &mut dst);
    assert_eq!(save_project(&dst), read("rust/testdata/template/applied.tlp"), "applied project differs from C++");
    assert_eq!(summary_text(&r) + "\n", read("rust/testdata/template/summary.txt"));
    assert_eq!(r.skipped.len(), 3);
}
