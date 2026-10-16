//! The engine runs in the browser, where the system clock and threads don't exist: `Instant::now()` /
//! `SystemTime::now()` compile fine and then abort the whole app at run time (it took the live Pocket toolpath down
//! for 25 minutes on 2026-10-11). This test fails the build if any of them shows up in the engine's code.

#[test]
fn engine_never_touches_the_clock_or_threads() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut bad = Vec::new();
    for e in std::fs::read_dir(&src).unwrap() {
        let path = e.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if ["Instant::now", "SystemTime::now", "thread::spawn", "thread::sleep"].iter().any(|w| code.contains(w)) {
                bad.push(format!("{}:{}: {}", path.file_name().unwrap().to_string_lossy(), n + 1, line.trim()));
            }
        }
    }
    assert!(bad.is_empty(), "not available in the browser:\n{}", bad.join("\n"));
}
