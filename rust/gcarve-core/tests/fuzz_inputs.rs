//! Importers and the project loader must never panic on damaged files: in the browser a panic ends the whole app.
//! Deterministic mutations (cut, byte flips, junk, repeats) of the sample files; run with
//! `cargo test -p gcarve-core --release --test fuzz_inputs -- --ignored` (a few seconds; GCARVE_FUZZ_CRV=x.crv adds a
//! VCarve project).

use gcarve_core::document::Material;
use std::panic::catch_unwind;

fn rng(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn mutate(src: &[u8], seed: &mut u64) -> Vec<u8> {
    let mut v = src.to_vec();
    if v.is_empty() {
        return v;
    }
    match rng(seed) % 5 {
        0 => v.truncate((rng(seed) as usize) % v.len()),
        1 => {
            for _ in 0..1 + rng(seed) % 12 {
                let i = (rng(seed) as usize) % v.len();
                v[i] = rng(seed) as u8;
            }
        }
        2 => {
            let i = (rng(seed) as usize) % v.len();
            let junk: &[&[u8]] = &[b"-1e999", b"NaN", b"0 0 0 0 0 0 0 0", b"99999999999999999999", b"<<>>", b"\"\"", b"-0", b"1e-320"];
            let j = junk[(rng(seed) as usize) % junk.len()];
            v.splice(i..i, j.iter().copied());
        }
        3 => {
            let (a, b) = ((rng(seed) as usize) % v.len(), (rng(seed) as usize) % v.len());
            let (a, b) = (a.min(b), a.max(b));
            let chunk = v[a..b].to_vec();
            v.splice(b..b, chunk);
        }
        _ => {
            let i = (rng(seed) as usize) % v.len();
            v.drain(i..(i + 1 + (rng(seed) as usize) % 40).min(v.len()));
        }
    }
    v
}

#[test]
#[ignore]
fn importers_survive_damaged_files() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |p: &str| std::fs::read(root.join(p)).unwrap_or_default();
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("svg", read("samples/demo.svg")),
        ("dxf", read("samples/demo.dxf")),
        ("tlp", read("samples/showcase.tlp")),
        ("gcode", read("reference/01_V90.tap")),
        ("gcode", read("reference/02_EMD14.tap")),
    ];
    // live text (kept for the app by the _with_text importers)
    cases.push(("svg", br#"<svg width="2in" height="1in" viewBox="0 0 192 96"><g id="a"><text x="9" y="48" style="font-size:24px" text-anchor="middle">T &amp; <tspan x="0" dy="30">Co</tspan></text><rect width="9" height="9"/></g></svg>"#.to_vec()));
    cases.push(("dxf", b"0\nSECTION\n2\nENTITIES\n0\nTEXT\n10\n2\n20\n3\n40\n0.5\n72\n1\n11\n4\n21\n3\n1\nShop\n0\nMTEXT\n10\n1\n20\n9\n40\n0.25\n71\n5\n1\n{\\fArial|b0;One}\\PTwo\n0\nENDSEC\n0\nEOF\n".to_vec()));
    cases.retain(|(_, b)| !b.is_empty());
    assert!(cases.len() >= 3, "sample files not found");
    let rounds: usize = std::env::var("FUZZ_N").ok().and_then(|v| v.parse().ok()).unwrap_or(300); // per file
    let mut seed = 0x9E3779B97F4A7C15u64;
    let m = Material::default();
    let mut bad = Vec::new();
    for (kind, src) in &cases {
        for n in 0..rounds {
            let bytes = mutate(src, &mut seed);
            let text = String::from_utf8_lossy(&bytes).into_owned();
            // the input being run is on disk: if a case never returns, this file is the one that hangs
            let _ = std::fs::write(std::env::temp_dir().join("fuzz-current.bin"), format!("{kind} #{n}\n").into_bytes().into_iter().chain(bytes.iter().copied()).collect::<Vec<u8>>());
            let t0 = std::time::Instant::now();
            let r = catch_unwind(|| match *kind {
                "svg" => drop(gcarve_core::svg::import_svg_with_text(&text)),
                "dxf" => drop(gcarve_core::dxf::import_dxf_with_text(&text)),
                "tlp" => drop(gcarve_core::project::load_project(&text)),
                _ => drop(gcarve_core::gimport::read_program(&text, "x", &m)),
            });
            if t0.elapsed().as_secs_f64() > 2.0 {
                eprintln!("SLOW {kind} #{n}: {:.1}s", t0.elapsed().as_secs_f64());
                let _ = std::fs::write(std::env::temp_dir().join(format!("slow-{kind}-{n}.bin")), &bytes);
                bad.push(format!("{kind} #{n} slow"));
            }
            if r.is_err() {
                bad.push(format!("{kind} #{n}"));
                let _ = std::fs::write(std::env::temp_dir().join(format!("fuzz-{kind}-{n}.bin")), &bytes);
            }
        }
    }
    // PDF: the compressed streams are mutated too, so build one with a real stream first
    let pdf = {
        let content = b"q 72 72 72 72 re f Q 0 0 m 36 0 l 18 36 l h B 10 10 m 20 30 40 50 60 10 c S";
        let z = miniz_oxide::deflate::compress_to_vec_zlib(content, 6);
        let mut f = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n".to_vec();
        f.extend(format!("4 0 obj\n<< /Length {} /Filter /FlateDecode >>\nstream\n", z.len()).bytes());
        f.extend(&z);
        f.extend(b"\nendstream\nendobj\n%%EOF\n");
        f
    };
    for n in 0..rounds * 2 {
        let bytes = mutate(&pdf, &mut seed);
        let _ = std::fs::write(std::env::temp_dir().join("fuzz-current.bin"), format!("pdf #{n}\n").into_bytes().into_iter().chain(bytes.iter().copied()).collect::<Vec<u8>>());
        let t0 = std::time::Instant::now();
        let r = catch_unwind(|| drop(gcarve_core::pdf::import_pdf(&bytes)));
        if t0.elapsed().as_secs_f64() > 2.0 {
            eprintln!("SLOW pdf #{n}: {:.1}s", t0.elapsed().as_secs_f64());
            let _ = std::fs::write(std::env::temp_dir().join(format!("slow-pdf-{n}.bin")), &bytes);
            bad.push(format!("pdf #{n} slow"));
        }
        if r.is_err() {
            bad.push(format!("pdf #{n}"));
            let _ = std::fs::write(std::env::temp_dir().join(format!("fuzz-pdf-{n}.bin")), &bytes);
        }
    }
    // VCarve .crv: no sample is committed (customers' projects), so only when GCARVE_FUZZ_CRV names a real one
    if let Some(crv) = std::env::var("GCARVE_FUZZ_CRV").ok().and_then(|p| std::fs::read(p).ok()) {
        for n in 0..rounds * 2 {
            let bytes = mutate(&crv, &mut seed);
            let t0 = std::time::Instant::now();
            let r = catch_unwind(|| drop(gcarve_core::crv::import_crv(&bytes)));
            if t0.elapsed().as_secs_f64() > 2.0 || r.is_err() {
                bad.push(format!("crv #{n}{}", if r.is_err() { "" } else { " slow" }));
                let _ = std::fs::write(std::env::temp_dir().join(format!("fuzz-crv-{n}.bin")), &bytes);
            }
        }
    }
    assert!(bad.is_empty(), "panics on damaged input: {bad:?} (inputs saved in the temp dir)");
}
