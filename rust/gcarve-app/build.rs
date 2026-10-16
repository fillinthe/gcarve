//! Embeds the machine + bit databases (repo data/) into the desktop app: one include_str! per file, so it works
//! offline. The web build embeds nothing and fetches site/public/data/catalog.json (data/publish.py) instead.
//! Re-runs when data/ changes.
use std::{env, fs, path::Path};

fn list(dir: &Path, skip: &[&str]) -> String {
    let mut files: Vec<_> = fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e == "json") && !skip.iter().any(|s| p.ends_with(s)));
    files.sort();
    files.iter().map(|p| format!("({:?}, include_str!({:?})),\n", p.file_name().unwrap().to_string_lossy(), p.canonicalize().unwrap())).collect()
}

fn main() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    println!("cargo:rerun-if-changed={}", data.display());
    println!("cargo:rerun-if-changed={}", data.join("machines").display());
    println!("cargo:rerun-if-changed={}", data.join("bits").display());
    if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        let out = "pub const MACHINES: &[(&str, &str)] = &[];\npub const BITS: &[(&str, &str)] = &[];\npub const CHIPLOAD: &str = \"{}\";\n";
        fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("catalog_files.rs"), out).unwrap();
        return;
    }
    let out = format!(
        "pub const MACHINES: &[(&str, &str)] = &[\n{}];\npub const BITS: &[(&str, &str)] = &[\n{}];\npub const CHIPLOAD: &str = include_str!({:?});\n",
        list(&data.join("machines"), &["spindles.json"]),
        list(&data.join("bits"), &["chipload.json"]),
        data.join("bits/chipload.json").canonicalize().unwrap()
    );
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("catalog_files.rs"), out).unwrap();
}
