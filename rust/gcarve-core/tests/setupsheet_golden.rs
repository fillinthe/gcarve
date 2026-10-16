//! Setup sheet for samples/showcase.tlp (2 sheets) is the same HTML as the C++ app's (tests/canvas_test.cpp setupSheet
//! writes samples/setupsheet/showcase_setup.html), once the date and the picture are normalised.

use gcarve_core::pipeline::{self, active_post};
use gcarve_core::project::load_project;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// "Setup sheet &middot; <date></p>" -> DATE, "<image .../>" -> "<image/>" (the C++ test's normalisation)
fn normalise(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find("<image ") {
        out += &rest[..i];
        out += "<image/>";
        rest = &rest[i + rest[i..].find("/>").unwrap() + 2..];
    }
    out += rest;
    let key = "Setup sheet &middot; ";
    let mut res = String::new();
    let mut rest = out.as_str();
    while let Some(i) = rest.find(key) {
        res += &rest[..i + key.len()];
        res += "DATE";
        rest = &rest[i + key.len() + rest[i + key.len()..].find("</p>").unwrap()..];
    }
    res + rest
}

/// "2.3 min" -> "N min": Rust times count the machine's acceleration (`pipeline::motion`), the C++ ones don't
fn no_times(html: &str) -> String {
    html.split(" min<").map(|p| p.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.')).collect::<Vec<_>>().join("N min<")
}

#[test]
fn showcase_setup_sheet_matches_the_cpp_engine() {
    let doc = load_project(&std::fs::read_to_string(root().join("samples/showcase.tlp")).unwrap()).unwrap();
    let gen = pipeline::regenerate(&doc);
    let post = active_post(&doc, &|_| None);
    let mut pics = 0;
    let html = pipeline::setup_sheet(&doc, &gen, "showcase", "8 October 2026, 06:02 PM", 0.0, &post, &[], &mut |_, g| {
        pics += 1;
        (!g.paths.is_empty()).then(|| ("data:image/png;base64,AAAA".to_string(), [0.0, 0.0, 1.0, 1.0]))
    });
    assert_eq!(pics, 2, "one picture per sheet");
    assert!(html.contains("data:image/png;base64,AAAA"));
    let want = std::fs::read_to_string(root().join("samples/setupsheet/showcase_setup.html")).unwrap();
    let (got, want) = (no_times(&normalise(&html)), no_times(&want));
    if got != want {
        let _ = std::fs::write(std::env::temp_dir().join("showcase_setup_rust.html"), &got);
    }
    assert!(got == want, "differs from the C++ setup sheet (Rust output in {})", std::env::temp_dir().join("showcase_setup_rust.html").display());
    // export dialog names win
    let named = pipeline::setup_sheet(&doc, &gen, "showcase", "d", 0.0, &post, &["mine.tap".to_string()], &mut |_, _| None);
    assert!(named.contains("<td class=\"file\">mine.tap</td>"));
}
