//! Assertions ported from the C++ tests: tests/cam_test.cpp (posts, spindle safety, Vectric .pp, arc fitting, feeds),
//! tests/core_test.cpp (expressions) and tests/control_test.cpp (G-code reader). Jobs that C++ builds with
//! buildJobs/generateProfile (cam.rs, not ported yet) are replaced by the golden fixture jobs.

use gcarve_core::arcfit::{fit_arcs, DEFAULT_MAX_SWEEP, DEFAULT_TOL};
use gcarve_core::expr::{eval_expr, identifiers, rename_ident};
use gcarve_core::feeds::{assess_feeds, classify_material, suggest_feeds, Cutter, MaterialClass};
use gcarve_core::gcode::parse_program;
use gcarve_core::geom::{dist, pt};
use gcarve_core::postdef::*;
use gcarve_core::{post_wincnc, Job, Move, MoveType, Toolpath};
use std::collections::HashMap;

mod common;
use common::{fixture_jobs, golden, l, r, N};

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 2e-3
}

// ---------------------------------------------------------------- posts

/// motion regex of cam_test: ^\s*(N\d+\s*)?(G0?[1-3](\D|$)|G0?0\s.*[XY]|J3,|M3,|PU|Z-?\d)  (case-insensitive)
fn is_motion(line: &str) -> bool {
    let u = line.trim_start().to_ascii_uppercase();
    let mut s = u.as_str();
    if let Some(rest) = s.strip_prefix('N').filter(|r| r.starts_with(|c: char| c.is_ascii_digit())) {
        s = rest.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start();
    }
    let g = |s: &str| -> bool {
        let Some(t) = s.strip_prefix('G') else { return false };
        for t in [t.strip_prefix('0'), Some(t)].into_iter().flatten() {
            if t.starts_with(['1', '2', '3']) && !t[1..].starts_with(|c: char| c.is_ascii_digit()) {
                return true;
            }
            if let Some(rest) = t.strip_prefix('0') {
                if rest.starts_with(char::is_whitespace) && rest.contains(['X', 'Y']) {
                    return true;
                }
            }
        }
        false
    };
    g(s) || s.starts_with("J3,") || s.starts_with("M3,") || s.starts_with("PU") || s.strip_prefix('Z').is_some_and(|t| t.trim_start_matches('-').starts_with(|c: char| c.is_ascii_digit()))
}

/// off regex of cam_test: (^|[\s;])(M0?5|C7|SO,\s*\d+,\s*0|!MC0)([\s;]|$)
fn is_off(line: &str) -> bool {
    let u = line.to_ascii_uppercase();
    u.split(|c: char| c.is_whitespace() || c == ';').any(|w| {
        matches!(w, "M5" | "M05" | "C7" | "!MC0")
            || w.strip_prefix("SO,").and_then(|t| t.split_once(',')).is_some_and(|(n, z)| {
                let n = n.trim_start();
                !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) && z.trim_start() == "0"
            })
    })
}

fn spindle_off_at_end(text: &str) -> bool {
    let (mut last_move, mut last_off) = (None, None);
    for (k, l) in text.split('\n').enumerate() {
        if is_motion(l) {
            last_move = Some(k);
        }
        if is_off(l) {
            last_off = Some(k);
        }
    }
    last_move.is_some() && last_off > last_move
}

#[test]
fn air_cut_note_in_header() {
    let jobs = fixture_jobs();
    let at = &post_jobs(&jobs, find_post("wincnc-single").unwrap())[0].text;
    let ag = &post_jobs(&jobs, find_post("grbl-in").unwrap())[0].text;
    assert!(at.contains("[AIR CUT TEST - NOT FOR CUTTING]") && ag.contains("(AIR CUT TEST - NOT FOR CUTTING)"));
}

#[test]
fn spindle_safety_every_post_every_file() {
    let jobs = fixture_jobs();
    for pd in post_library() {
        for po in post_jobs(&jobs, pd) {
            assert!(spindle_off_at_end(&po.text), "post {}: no spindle off at the end", pd.id);
        }
    }
    let mut bare = find_post("grbl-in").unwrap().clone();
    bare.id = "user".into();
    bare.footer = String::new();
    assert!(spindle_off_at_end(&post_jobs(&jobs, &bare)[0].text));
    bare.footer = "G0 Z1\nM30\n".into(); // program end without M5: M5 goes in before the M30
    let bt = &post_jobs(&jobs, &bare)[0].text;
    assert!(spindle_off_at_end(bt) && bt.contains("M5\nM30"));
    assert_eq!(spindle_safe_footer("M05\nM30\n"), "M05\nM30\n"); // already safe: unchanged
    assert_eq!(spindle_safe_tool_change("T{TOOLNUM} M6\n"), "M5\nT{TOOLNUM} M6\n");
    assert_eq!(spindle_safe_tool_change("M5\nT1 M6\n"), "M5\nT1 M6\n");
    // a user ATC post whose tool change forgets M5 still stops the spindle before each change
    let mut atc = find_post("mach3-atc-in").unwrap().clone();
    atc.tool_change = "T{TOOLNUM} M6\nS{RPM} M3\n".into();
    let t = &post_jobs(&jobs, &atc)[0].text;
    assert!(t.contains("M5\nT1 M6") && t.contains("M5\nT2 M6"));
}

#[test]
fn industrial_iso_posts_round_trip() {
    // every Rust-only ISO post: what it writes reads back (gimport) as the same cut in its units
    // (spindle safety: spindle_safety_every_post_every_file covers every post)
    let jobs = fixture_jobs();
    let ids = ["fanuc-in", "osai-mm", "osai-in", "fagor-mm", "fagor-in", "siemens-mm", "smartcore-in", "multicam-in", "multicam-mm", "axyz-in", "axyz-mm", "thermwood-in", "techno-in", "techno-mm", "laguna-touch-in", "starcnc-in", "delta-in", "din-mm", "format4-mm"];
    let reference = gcarve_core::gimport::read_program(&post_jobs(&jobs, find_post("linuxcnc-in").unwrap()).iter().map(|o| o.text.clone()).collect::<Vec<_>>().join("\n"), "ref", &Default::default());
    let total = |p: &gcarve_core::gimport::ImportedProgram| p.sections.iter().map(|s| s.cut_length).sum::<f64>();
    for id in ids {
        let d = find_post(id).unwrap_or_else(|| panic!("{id} missing"));
        let text = post_jobs(&jobs, d).iter().map(|o| o.text.clone()).collect::<Vec<_>>().join("\n");
        let p = gcarve_core::gimport::read_program(&text, id, &Default::default());
        assert!(p.errors.is_empty(), "{id}: {:?}", &p.errors[..p.errors.len().min(3)]);
        assert!((total(&p) - total(&reference)).abs() < 1e-3 * total(&reference), "{id}: cut {} vs {}", total(&p), total(&reference));
        let min_z = p.sections.iter().map(|s| s.min_z).fold(f64::MAX, f64::min);
        let ref_z = reference.sections.iter().map(|s| s.min_z).fold(f64::MAX, f64::min);
        assert!((min_z - ref_z).abs() < 1e-3, "{id}: depth {min_z} vs {ref_z}");
    }
}

#[test]
fn post_library_renders_other_controllers() {
    let jobs = fixture_jobs();
    assert_eq!(jobs.len(), 2);
    assert!(find_post("grbl-in").is_some() && find_post("shopbot-in").is_some() && find_post("wincnc-single").is_some() && find_post("linuxcnc-mm").is_some());
    assert!(post_library().len() >= 40 && find_post("nope").is_none());
    let g = post_jobs(&jobs, find_post("grbl-in").unwrap());
    assert!(g.len() == 2 && g[0].filename.contains(".nc")); // one file per tool
    let t = &g[0].text;
    assert!(t.contains("G90 G20 G17 G94") && t.contains("M3 S18000") && t.contains("M30"));
    assert!(t.contains("G1 X") && t.contains("F432") && (t.contains("G2 X") || t.contains("G3 X")) && t.contains(" I"));
    assert!(!t.contains('[')); // no WinCNC-style comments leaked
    assert!(t.contains("G0 Z0.8000")); // modal axes: a pure Z move has no X/Y words
    let mm = post_jobs(&jobs, find_post("grbl-mm").unwrap());
    assert!(mm[0].text.contains("G21") && mm[0].text.contains("F10973")); // 432 in/min -> 10973 mm/min
    // combined ATC file: one output with two tool changes
    let atc = post_jobs(&jobs, find_post("wincnc-atc").unwrap());
    assert_eq!(atc.len(), 1);
    let (t1, t2) = (atc[0].text.find("T1 M6").unwrap(), atc[0].text.find("T2 M6").unwrap());
    assert!(t2 > t1 && atc[0].text.contains("\r\n"));
    assert_eq!(atc[0].filename, "Profile_(outside).tap"); // "01_" prefix dropped for the combined file
    // ShopBot: bare numbers, no arcs, its own verbs
    let sb = &post_jobs(&jobs, find_post("shopbot-in").unwrap())[0].text;
    assert!(sb.contains("M3,") && sb.contains("J3,") && !sb.contains("G1") && !sb.contains("G3"));
    // the exact WinCNC writer is used for its id
    let wc = post_jobs(&jobs, find_post("wincnc-single").unwrap());
    assert!(wc.len() == 2 && wc[0].text == post_wincnc(&jobs[0]) && wc[0].filename.contains(".tap"));
    // JSON round trip of a post definition
    let mut custom = find_post("grbl-in").unwrap().clone();
    custom.id = "mine".into();
    custom.header = "G90 G20\n(hello \"there\")\n".into();
    let back = post_from_json(&post_to_json(&custom)).unwrap();
    assert_eq!(back, custom);
    assert!(post_jobs(&[], &custom).is_empty());
}

#[test]
fn json_reader_defaults_and_errors() {
    let d = post_from_json(r#"{"header": "G90\n", "inches": false, "unknown": 3}"#).unwrap();
    assert!(d.id == "custom" && d.name == "custom" && !d.inches && d.header == "G90\n" && d.decimals == 4 && d.rapid == "G0 {X} {Y} {Z}");
    // the C++ writer prints integral doubles without ".0"
    assert_eq!(post_from_json(r#"{"id":"a","maxArcSweepDeg": 90}"#).unwrap().max_arc_sweep_deg, 90.0);
    assert!(post_from_json("[1]").is_err() && post_from_json("not json").is_err());
}

#[test]
fn vectric_pp_import() {
    let jobs = fixture_jobs();
    let d = post_from_vectric_pp(&golden("test-router.pp")).unwrap();
    assert!(d.name == "Test Router (inch)(*.tap)" && d.extension == "tap" && d.inches && d.decimals == 4 && d.arcs);
    assert!(d.header.contains("G90 G20") && d.header.contains("T{TOOLNUM} M6") && d.header.contains("S{RPM} M3"));
    assert!(d.rapid == "G0 X{x} Y{y} Z{z}" && d.arc_ccw == "G3 X{x} Y{y} {I} {J} F{f}" && d.footer.contains("M30")); // mode A = every move carries every axis
    let outp = post_jobs(&jobs, &d);
    assert!(!outp.is_empty() && outp[0].text.contains("T1 M6") && outp[0].text.contains("G90 G20") && outp[0].text.contains("G1 X"));
    assert!(outp[0].text.contains(&format!("( {} )", jobs[0].filename)));
    assert!(post_from_vectric_pp("just text").is_err_and(|e| !e.is_empty()));

    // a Mach3-style post: A/C var modes, line numbers, feed-rate-change block, plunge, first feed move, substitution
    let m3 = post_from_vectric_pp(&golden("mach3-like.pp")).unwrap();
    assert!(!m3.inches && m3.decimals == 3 && m3.feed_decimals == 1 && m3.substitute == "({)}" && m3.line_number_start == 100 && m3.line_number_step == 5);
    assert!(m3.rapid == "N{N}G0{X}{Y}{Z}" && m3.plunge == "N{N}G1{Z}F{f}" && m3.first_linear == "N{N}G1{X}{Y}{Z}F{f}" && m3.feed_change == "N{N}F{f}" && !m3.arcs);
    let tpp = Toolpath {
        name: "p".into(),
        tool: -1,
        feed_xy: 100.0,
        feed_z: 50.0,
        moves: vec![
            Move::rapid(Some(0.0), Some(0.0), Some(0.5)),
            Move::linear(Some(0.0), Some(0.0), Some(-0.1)),
            Move::linear(Some(1.0), Some(0.0), Some(-0.1)),
            Move::linear(Some(1.0), Some(1.0), Some(-0.1)),
            Move::rapid(Some(1.0), Some(1.0), Some(0.5)),
        ],
    };
    let jt = Job { filename: "t".into(), tool: "1/4 (6mm) end mill".into(), rpm: 16000, toolpaths: vec![tpp], ..Default::default() };
    let o = &post_jobs(std::slice::from_ref(&jt), &m3)[0].text;
    assert!(o.contains("N100(1/4 {6mm} end mill)")); // substitute ( ) -> { } inside names, numbering from 100
    assert!(o.contains("N105G21 G90") && o.contains("M3S16000"));
    assert!(o.contains("G0X0.000Y0.000Z12.700")); // mm output, first rapid carries every axis
    assert!(o.contains("N120F1270.0\nN125G1Z-2.540F1270.0\n")); // plunge: feed-rate-change block + PLUNGE_MOVE at the plunge feed
    assert!(o.contains("F2540.0\nN135G1X25.400\n")); // back to the cutting feed, then modal XY move without F
    assert!(o.contains("G1Y25.400\n"));
    assert!(o.contains("G0Z12.700\n") && o.contains("G0X0\n") && o.contains("M30")); // retract is Z-only, [XH] home
    // round-trips through our JSON
    let back = post_from_json(&post_to_json(&m3)).unwrap();
    assert!(back.plunge == m3.plunge && back.feed_decimals == 1 && back.substitute == m3.substitute && back.line_number_step == 5);
    assert_eq!(&post_jobs(&[jt], &back)[0].text, o);
}

// ---------------------------------------------------------------- arc fitting

fn circle(c: (f64, f64), rad: f64, n: usize, z: f64, ccw: bool) -> Toolpath {
    let s = if ccw { 1.0 } else { -1.0 };
    let mut moves = vec![r(c.0 + rad, c.1, 0.5), l(N, N, z)];
    moves.extend((1..=n).map(|k| {
        let a = s * 2.0 * std::f64::consts::PI * k as f64 / n as f64;
        l(c.0 + rad * a.cos(), c.1 + rad * a.sin(), N)
    }));
    moves.push(r(N, N, 0.5));
    Toolpath { name: "c".into(), tool: -1, feed_xy: 100.0, feed_z: 50.0, moves }
}

#[test]
fn arcfit_circle_becomes_quadrant_arcs() {
    let c = (3.0, 3.0);
    let fit = fit_arcs(&circle(c, 1.05, 72, -0.484, true), DEFAULT_TOL, DEFAULT_MAX_SWEEP);
    let arcs = fit.moves.iter().filter(|m| matches!(m.kind, MoveType::ArcCw | MoveType::ArcCcw)).count();
    assert_eq!(arcs, 4);
    let mut prev = pt(0.0, 0.0);
    for m in &fit.moves {
        assert_ne!(m.kind, MoveType::ArcCw); // CCW cut -> G3 only
        if m.kind == MoveType::ArcCcw {
            let ctr = prev + pt(m.i, m.j);
            assert!(near(ctr.x, c.0) && near(ctr.y, c.1) && near(dist(ctr, pt(m.x.unwrap(), m.y.unwrap())), 1.05));
            assert!(m.z.is_none()); // arc lines carry no Z, like the reference
        }
        if let (Some(x), Some(y)) = (m.x, m.y) {
            prev = pt(x, y);
        }
    }
    // clockwise gives G2, a square gains no arcs, a helix (Z changes) gains none
    let cw = fit_arcs(&circle(c, 1.05, 72, -0.484, false), DEFAULT_TOL, DEFAULT_MAX_SWEEP);
    assert_eq!(cw.moves.iter().filter(|m| m.kind == MoveType::ArcCw).count(), 4);
    let sq = Toolpath { moves: vec![r(0.0, 0.0, 0.5), l(N, N, -0.1), l(4.0, 0.0, N), l(4.0, 2.0, N), l(0.0, 2.0, N), l(0.0, 0.0, N)], ..Default::default() };
    assert_eq!(fit_arcs(&sq, DEFAULT_TOL, DEFAULT_MAX_SWEEP), sq);
    let helix = Toolpath {
        moves: (0..=72).map(|k| {
            let a = 2.0 * std::f64::consts::PI * k as f64 / 36.0;
            l(2.0 + 0.375 * a.cos(), 2.0 + 0.375 * a.sin(), -0.4 * k as f64 / 72.0)
        }).collect(),
        ..Default::default()
    };
    assert_eq!(fit_arcs(&helix, DEFAULT_TOL, DEFAULT_MAX_SWEEP).moves.len(), helix.moves.len());
}

// ---------------------------------------------------------------- feeds

#[test]
fn feeds_advisor() {
    use MaterialClass::*;
    assert!(classify_material("Baltic birch") == Plywood && classify_material("Red Oak") == Hardwood && classify_material("MDF") == Mdf);
    assert!(classify_material("pine") == Softwood && classify_material("cast acrylic") == Plastic && classify_material("6061 aluminum") == Aluminum);
    let q = Cutter { vbit: false, diameter: 0.25, flutes: 2 };
    let s = suggest_feeds(&q, "Plywood", 18000, 600.0);
    assert!(s.rpm == 18000 && s.feed_xy >= 380.0 && s.feed_xy <= 480.0 && s.feed_z < s.feed_xy && s.stepdown > 0.1 && s.stepdown < 0.25); // 18000*2*0.012 = 432
    let good = assess_feeds(&q, "Plywood", 432.0, 216.0, 18000, 0.1875);
    assert!(good.level == 0 && good.meter > 0.35 && good.meter < 0.65);
    let rub = assess_feeds(&q, "Hardwood", 60.0, 30.0, 18000, 0.125);
    assert!(rub.level == 2 && rub.meter < 0.25 && rub.message.contains("Rubbing"));
    let hot = assess_feeds(&q, "Hardwood", 1200.0, 300.0, 18000, 0.125);
    assert!(hot.level == 2 && hot.meter > 0.75 && hot.message.contains("Overloaded"));
    let deep = assess_feeds(&q, "Hardwood", 300.0, 100.0, 18000, 0.6);
    assert!(deep.level == 2 && deep.message.contains("too deep"));
    let v = Cutter { vbit: true, diameter: 0.5, ..q };
    let vs = suggest_feeds(&v, "MDF", 18000, 600.0);
    assert!(vs.feed_xy > 100.0 && vs.feed_xy < 300.0 && vs.stepdown <= 0.25);
    // a pass twice the bit's width wants 80% of the chip: the feed that was good at 1×D is now aggressive
    let d2 = assess_feeds(&q, "Plywood", 432.0, 216.0, 18000, 0.5);
    assert!(d2.level >= 1 && d2.message.contains("80%"), "{}", d2.message);
    assert!(assess_feeds(&q, "Plywood", 432.0 * 0.8, 216.0, 18000, 0.5).message.starts_with("Good"));
    assert_eq!(gcarve_core::feeds::depth_factor(0.25, 1.0), 0.6);
    // exact text, same printf formatting as C++
    assert_eq!(good.message, "Good: 0.0120 in/tooth is in the 0.011–0.013 range for plywood.");
}

// ---------------------------------------------------------------- expressions

#[test]
fn expressions() {
    let none = |_: &str| None;
    let ev = |s: &str, bare: f64| eval_expr(s, bare, &none);
    let ok = |s: &str, bare: f64, want: f64| ev(s, bare).is_ok_and(|v| near(v, want));
    assert!(ok("4+2", 1.0, 6.0));
    assert!(ok("1 in + 5mm", 1.0, 1.0 + 5.0 / 25.4)); // mixed units add in inches
    assert!(ok("2ft - 3in + 10 mm", 1.0, 24.0 - 3.0 + 10.0 / 25.4));
    assert!(ok("1 in + 5mm", 1.0 / 25.4, 1.0 + 5.0 / 25.4)); // explicit units ignore the bare unit
    assert!(ok("5 + 1in", 1.0 / 25.4, 5.0 / 25.4 + 1.0)); // bare 5 means 5 mm in a mm project
    assert!(ok("4 + 2 * 3", 1.0, 10.0));
    assert!(ok("(4 + 2) * 3", 1.0, 18.0));
    assert!(ok("4ft", 1.0, 48.0));
    assert!(ok("1200mm", 1.0, 1200.0 / 25.4));
    assert!(ok("3' + 6\"", 1.0, 42.0));
    assert!(ev("3' 6\"", 1.0).is_err()); // no implicit addition
    assert!(ok("48", 12.0, 48.0 * 12.0)); // bare numbers in feet
    assert!(ok("48in", 12.0, 48.0)); // suffix wins
    assert!(ok("2 x 4", 1.0, 8.0));
    assert!(ok("-2 + 10 / 4", 1.0, 0.5));
    assert!(ev("4 +", 1.0).is_err_and(|e| !e.is_empty()));
    assert!(ev("abc", 1.0).is_err_and(|e| e.contains("abc")));
    assert!(ev("1/0", 1.0).is_err());
    assert_eq!(ev("", 1.0), Err("unexpected end".into()));
    assert_eq!(ev("1e400", 1.0), Err("not a number".into()));

    // Document::eval stand-in: variables are expressions themselves; runaway cycles stop
    fn eval_doc(vars: &HashMap<&str, &str>, s: &str, bare: f64, depth: u32) -> Result<f64, String> {
        if depth > 40 {
            return Err("variables reference each other in a loop".into());
        }
        eval_expr(s, bare, &|n| vars.get(n).and_then(|e| eval_doc(vars, e, bare, depth + 1).ok()))
    }
    let mut vars = HashMap::from([("box-w", "4in"), ("box-h", "box-w / 2"), ("gap", "0.25")]);
    let d = |vars: &HashMap<&str, &str>, s: &str, bare: f64| eval_doc(vars, s, bare, 0).unwrap();
    assert!(near(d(&vars, "box-w", 1.0), 4.0));
    assert!(near(d(&vars, "box-h + gap", 1.0), 2.25));
    assert!(near(d(&vars, "box-w - gap", 1.0), 3.75)); // spaces: subtraction
    assert!(near(d(&vars, "2*box-w", 1.0), 8.0));
    assert!(near(d(&vars, "gap", 1.0 / 25.4), 0.25 / 25.4) && near(d(&vars, "box-w", 1.0 / 25.4), 4.0));
    vars.insert("loop", "loop + 1");
    assert!(eval_doc(&vars, "loop", 1.0, 0).is_err_and(|e| e.contains("loop")));

    assert_eq!(identifiers("box-w + 2in * gap - 4ft"), ["box-w", "gap"]);
    assert_eq!(rename_ident("box-w + box-w2 * gap", "box-w", "W"), "W + box-w2 * gap");
}

// ---------------------------------------------------------------- G-code reader

#[test]
fn gcode_reader() {
    let p = parse_program("G90 G20\nM3 S18000\nG0 X1 Y1 Z0.2\nG1 Z-0.25 F50\nG1 X3 F100\nG2 X3 Y1 I1 J0\nG0 Z0.8\nM5\nM30\n", 1000.0);
    assert!(p.errors.is_empty() && p.motions.len() == 5 && p.spindle_off_at_end);
    assert!(near(p.min.z, -0.25) && near(p.max.x, 5.0) && near(p.min.x, 1.0)); // full circle reaches X5
    assert!(!parse_program("G0 X1\nG1 X2\n", 1000.0).errors.is_empty()); // feed move without F
    assert!(!parse_program("M3 S1000\nG1 X1 F10\n", 1000.0).spindle_off_at_end);
    let nx = parse_program("G0X5Y1\n", 1000.0); // no spaces, no hex
    assert!(nx.motions.len() == 1 && near(nx.motions[0].to.x, 5.0));
    let mm = parse_program("G21\nG1 X25.4 F254\n", 1000.0);
    assert!(mm.errors.is_empty() && near(mm.motions[0].to.x, 1.0) && near(mm.motions[0].feed, 10.0));
}
