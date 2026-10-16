//! G-code import: reads a finished program (VCarve WinCNC, GRBL, Fusion, Mach3, LinuxCNC ...) back into the document.
//! The file is split at every tool change; each section becomes one `OpType::Imported` toolpath that replays the
//! file's own moves (simulate, safety-check, re-post for another machine) and its cut centrelines come in as vectors.
//!
//! The bit is never guessed silently. In order of trust: the file's tool comment ("[V-Bit (90 deg 0.5")]",
//! "(T1 D=6.35 ... flat end mill)"), the toolpath / file name ("01_V90", "EMD14"), then the moves themselves
//! (Z changing while cutting = V-bit or 3D). The app shows the guess per section and the user confirms it.

use crate::cam::origin_shift;
use crate::document::{Document, FeedPreset, Material, Op, OpType, Tool, ToolType};
use crate::gcode::{interpret_line, motion_points, GMotion, GState};
use crate::geom::{pt, Contour, Node};
use crate::job::{Move, MoveType, Toolpath};

/// What a section says (or shows) about its bit. `None` = not known.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ToolGuess {
    pub kind: Option<ToolType>,
    /// inches
    pub diameter: Option<f64>,
    /// V-bit included angle, degrees
    pub angle: Option<f64>,
    /// a shaped bit (roundover, ogee, cove ...) that GCarve can only approximate
    pub form: bool,
    /// where the guess came from: "file comment", "name", "moves"
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ImportSection {
    /// first toolpath name in the section ("01_EMD14"), else "<file> T<n>"
    pub name: String,
    /// the file's tool comment, verbatim ("" = none)
    pub tool_text: String,
    /// T number (0 = none)
    pub tnum: i32,
    pub guess: ToolGuess,
    /// notes for the user ("Z changes while cutting: V-bit or 3D toolpath")
    pub hints: Vec<String>,
    pub rpm: f64,
    /// inches/min
    pub feed_xy: f64,
    pub feed_z: f64,
    /// work coordinates of the file, inches
    pub min_z: f64,
    pub max_z: f64,
    pub cut_length: f64,
    pub minutes: f64,
    /// normalized program of this section in document coordinates (see `section_toolpaths`)
    pub gcode: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ImportedProgram {
    pub sections: Vec<ImportSection>,
    /// board from a VCarve header ("MATERIAL size: 48.000" x 48.000" x 0.484""), inches
    pub board: Option<(f64, f64, f64)>,
    /// header said Z zero is the table ("Z Zero Position: Table"); None = not stated
    pub z_zero_table: Option<bool>,
    /// "line N: ..." lines that were skipped
    pub errors: Vec<String>,
}

/// Comments of a line (`( )`, `[ ]` WinCNC, after `;`) and the code left over. Nested brackets stay in the comment.
fn split_comments(s: &str) -> (String, Vec<String>) {
    let (mut code, mut comments, mut cur, mut depth) = (String::new(), Vec::new(), String::new(), 0);
    for (k, c) in s.char_indices() {
        match c {
            ';' if depth == 0 => {
                comments.push(s[k + 1..].trim().to_string());
                break;
            }
            '(' | '[' => {
                if depth > 0 {
                    cur.push(c);
                }
                depth += 1;
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    comments.push(std::mem::take(&mut cur).trim().to_string());
                } else {
                    cur.push(c);
                }
            }
            _ if depth > 0 => cur.push(c),
            _ => code.push(c),
        }
    }
    (code, comments)
}

fn num_at(s: &str) -> Option<(f64, usize)> {
    let b = s.as_bytes();
    let mut k = 0;
    while k < b.len() && (b[k].is_ascii_digit() || b[k] == b'.') {
        k += 1;
    }
    s[..k].parse::<f64>().ok().map(|v| (v, k))
}

/// "1/4" or "0.25" at the start of `s` (fractions like "3/8").
fn frac_at(s: &str) -> Option<(f64, usize)> {
    let (a, k) = num_at(s)?;
    if s[k..].starts_with('/') {
        if let Some((b, k2)) = num_at(&s[k + 1..]) {
            if b > 0.0 {
                return Some((a / b, k + 1 + k2));
            }
        }
    }
    Some((a, k))
}

/// Reads a tool comment: Vectric ("1/4 Down End Mill (0.25 inch)", "V-Bit (60 deg 0.25")", "End Mill (1/4")",
/// "3/8 Downcut (0.375)"), Fusion ("T1 D=6.35 CR=0 - ZMIN=-3 - flat end mill"), and plain names ("6mm ball nose").
pub fn guess_from_text(text: &str) -> ToolGuess {
    let low = text.to_lowercase();
    let mut g = ToolGuess { source: "file comment".into(), ..Default::default() };
    let round = low.contains("roundover") || low.contains("round over");
    if ["roundover", "round over", "ogee", "cove", "bead", "chamfer", "form "].iter().any(|w| low.contains(w)) {
        g.form = !low.contains("chamfer") && !round; // a chamfer bit is a V-bit, a round-over has its own shape
        if low.contains("chamfer") {
            g.kind = Some(ToolType::VBit);
        }
    }
    if round {
        g.kind = Some(ToolType::RoundOver);
    } else if ["ball", "core box", "corebox", "round nose", "roundnose"].iter().any(|w| low.contains(w)) {
        g.kind = Some(ToolType::BallNose);
    } else if low.contains("v-bit") || low.contains("vbit") || low.contains("v bit") || low.contains("engrav") || low.contains(" deg") || low.contains("°") {
        g.kind = Some(ToolType::VBit);
    } else if ["end mill", "endmill", "flat", "downcut", "down cut", "upcut", "up cut", "compression", "straight", "drill", "roughing"].iter().any(|w| low.contains(w)) {
        g.kind = Some(ToolType::EndMill);
    }
    // angle: "90 deg", "60°"
    for (k, _) in low.match_indices(|c: char| c.is_ascii_digit()) {
        if k > 0 && (low.as_bytes()[k - 1].is_ascii_digit() || low.as_bytes()[k - 1] == b'.') {
            continue;
        }
        if let Some((v, n)) = num_at(&low[k..]) {
            let rest = low[k + n..].trim_start();
            if rest.starts_with("deg") || rest.starts_with('°') {
                g.angle = Some(v);
                g.kind = Some(ToolType::VBit);
            }
        }
    }
    // Fusion: D=6.35 (mm unless the program is inches; Fusion writes the unit of the program, assume mm when > 1.5)
    if let Some(k) = low.find("d=") {
        if let Some((v, _)) = num_at(&low[k + 2..]) {
            g.diameter = Some(if v > 1.5 { v / 25.4 } else { v });
        }
    }
    // inside parentheses / after the angle: (0.25 inch) (1/4") (0.375) (0.5") — not "CL 1 inch" (cut length)
    if g.diameter.is_none() {
        let mut best = None;
        for (k, _) in low.match_indices(|c: char| c.is_ascii_digit()) {
            if k > 0 && (low.as_bytes()[k - 1].is_ascii_digit() || low.as_bytes()[k - 1] == b'.' || low.as_bytes()[k - 1] == b'/') {
                continue;
            }
            let Some((v, n)) = frac_at(&low[k..]) else { continue };
            let before = low[..k].trim_end();
            let after = low[k + n..].trim_start();
            if before.ends_with("cl") || after.starts_with("deg") || after.starts_with('°') || after.starts_with("flute") {
                continue;
            }
            let mm = after.starts_with("mm");
            let marked = after.starts_with('"') || after.starts_with("inch") || after.starts_with("in)") || mm;
            let in_paren = before.ends_with('(');
            let leading = before.is_empty() && low[k + n..].starts_with(' ');
            // a fraction ending the name is the size too: "Point Round Over 1/2"
            let tail = low[k..k + n].contains('/') && after.is_empty();
            if marked || in_paren || leading || tail {
                let d = if mm { v / 25.4 } else { v };
                // the explicit "(... inch)" / '"' value wins over a leading "1/4"
                if best.is_none() || marked || in_paren || tail {
                    best = Some(d);
                }
            }
        }
        g.diameter = best.filter(|d| *d > 0.0 && *d < 4.0);
    }
    g
}

/// Shop naming codes in toolpath / file names: V60 / V90 (V-bit angle), EMD14 / EMU18 / CMP14 / RDC38 (end mill,
/// digits = fraction 1/4, 1/8, 3/8), PRO (point roundover). Unknown names give an empty guess.
pub fn guess_from_name(name: &str) -> ToolGuess {
    let mut g = ToolGuess { source: "name".into(), ..Default::default() };
    for tok in name.split(|c: char| !c.is_ascii_alphanumeric()) {
        let t = tok.to_ascii_uppercase();
        let letters: String = t.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let digits = &t[letters.len()..];
        if !digits.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        match (letters.as_str(), digits.len()) {
            ("V" | "VB", 2 | 3) => {
                g.kind = Some(ToolType::VBit);
                g.angle = digits.parse().ok();
            }
            ("EM" | "EMD" | "EMU" | "CMP" | "RDC" | "DC" | "UC" | "DN" | "UP", 2) => {
                let (a, b) = (f64::from(digits.as_bytes()[0] - b'0'), f64::from(digits.as_bytes()[1] - b'0'));
                if b > 0.0 && a < b {
                    g.kind = Some(ToolType::EndMill);
                    g.diameter = Some(a / b);
                }
            }
            ("PRO", 0) => g.form = true,
            _ => continue,
        }
        return g;
    }
    g
}

/// Fills what `a` doesn't know from `b`.
fn merge(mut a: ToolGuess, b: &ToolGuess) -> ToolGuess {
    let had = a.kind.is_some() || a.diameter.is_some() || a.angle.is_some() || a.form;
    if a.kind.is_none() {
        a.kind = b.kind;
    }
    if a.diameter.is_none() {
        a.diameter = b.diameter;
    }
    if a.angle.is_none() {
        a.angle = b.angle;
    }
    a.form |= b.form;
    if !had {
        a.source = b.source.clone();
    } else if b.kind.is_some() || b.diameter.is_some() || b.angle.is_some() {
        a.source = format!("{} + {}", a.source, b.source);
    }
    a
}

fn looks_like_tool(c: &str) -> bool {
    let low = c.to_lowercase();
    if low.starts_with("toolpath name") || low.starts_with("filename") || low.starts_with("material") || low.starts_with("z zero") || low.contains("feedrate") || low.contains("spindle") {
        return false;
    }
    let g = guess_from_text(c);
    g.kind.is_some() || g.form || (g.diameter.is_some() && (low.contains("mill") || low.contains("bit") || low.contains("cut")))
}

#[derive(Default)]
struct Builder {
    sec: ImportSection,
    out: String,
    xy_len: f64,
    dz_len: f64,
    rising: f64,
    started: bool,
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.5}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// Industrial dialects → the words the interpreter knows: G70/G71 (OSAI, Fagor, Siemens inch/mm) → G20/G21;
/// dropped: G43 (length offset), O (program number), D (tool offset), N handled by the interpreter.
fn normalize_dialect(code: &str) -> String {
    let b = code.as_bytes();
    let (mut out, mut k) = (String::new(), 0);
    while k < b.len() {
        if !b[k].is_ascii_alphabetic() {
            out.push(b[k] as char);
            k += 1;
            continue;
        }
        // letter, optional spaces, [+-]digits.digits (the interpreter's word shape)
        let start = k;
        k += 1;
        while k < b.len() && b[k] == b' ' {
            k += 1;
        }
        let num = k;
        if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
            k += 1;
        }
        while k < b.len() && (b[k].is_ascii_digit() || b[k] == b'.') {
            k += 1;
        }
        let v = code[num..k].parse::<f64>().ok();
        match (b[start], v) {
            (b'G', Some(70.0)) => out.push_str("G20"),
            (b'G', Some(71.0)) => out.push_str("G21"),
            (b'G', Some(43.0)) => {}
            (b'O' | b'D', _) => {}
            _ => out.push_str(&code[start..k]),
        }
    }
    out
}

/// Reads a whole program. `stem` names sections without a toolpath name; `m` = the document's board (the file's
/// coordinates are relative to its origin corner and Z zero, the document's are not).
pub fn read_program(text: &str, stem: &str, m: &Material) -> ImportedProgram {
    let mut p = ImportedProgram::default();
    let (dx, dy, dz) = origin_shift(m);
    let mut st = GState::default();
    let (mut fxy, mut fz) = (0.0, 0.0);
    let mut tool_table: Vec<(i32, String)> = Vec::new();
    let (mut pending_tool, mut pending_name, mut tnum) = (String::new(), String::new(), 0);
    let mut b = Builder::default();
    let mut sections: Vec<Builder> = Vec::new();
    let mut lift_z: f64 = 0.0;
    for (ln, raw) in text.lines().enumerate() {
        let line_no = ln as i32 + 1;
        let (code, comments) = split_comments(raw);
        for c in &comments {
            let up = c.to_uppercase();
            if let Some(rest) = up.strip_prefix("MATERIAL SIZE:") {
                let v: Vec<f64> = rest.split('X').filter_map(|s| s.trim().trim_end_matches('"').trim().parse().ok()).collect();
                if v.len() == 3 {
                    p.board = Some((v[0], v[1], v[2]));
                }
            } else if let Some(rest) = up.strip_prefix("Z ZERO POSITION:") {
                p.z_zero_table = Some(rest.contains("TABLE"));
            } else if up.starts_with("TOOLPATH NAME:") {
                pending_name = c["TOOLPATH NAME:".len()..].trim().to_string();
            } else if up.starts_with('T') && up[1..].trim_start().starts_with(|ch: char| ch.is_ascii_digit()) && up.contains("D=") {
                let t = num_at(up[1..].trim_start()).map_or(0, |(v, _)| v as i32);
                tool_table.push((t, c.clone()));
            } else if looks_like_tool(c) {
                pending_tool = c.clone();
            }
        }
        let codeu = code.trim().to_uppercase();
        // WinCNC axis feeds: "F100 XY", "F50 Z"
        if let Some(rest) = codeu.strip_prefix('F') {
            if let Some((v, n)) = num_at(rest.trim_start()) {
                let axes = rest.trim_start()[n..].trim();
                if !axes.is_empty() && axes.chars().all(|c| matches!(c, 'X' | 'Y' | 'Z' | ' ')) {
                    let v = if st.inches { v } else { v / 25.4 };
                    if axes.contains('X') || axes.contains('Y') {
                        fxy = v;
                    }
                    if axes.contains('Z') {
                        fz = v;
                    }
                    st.feed = if axes.contains('Z') && !axes.contains('X') { st.feed.max(v) } else { v };
                    continue;
                }
            }
        }
        // tool change: M6, or a new T number
        let words: Vec<&str> = codeu.split_whitespace().collect();
        let t_here = words.iter().find_map(|w| w.strip_prefix('T').and_then(|r| r.parse::<i32>().ok()));
        let m6 = codeu.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "M6" || w == "M06");
        if (m6 || t_here.is_some_and(|t| t != tnum)) && b.started {
            sections.push(std::mem::take(&mut b));
        }
        if let Some(t) = t_here {
            tnum = t;
        }
        // G28 (Fusion's "G28 G91 Z0") = go home: treat like a lift to the top
        if codeu.contains("G28") {
            st.absolute = true;
            continue;
        }
        // Siemens "SUPA G0 Z0" / OSAI "G79" = machine-coordinate lift, like G53
        if codeu.contains("SUPA") || codeu.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "G79") {
            if b.started {
                b.out.push_str("LIFT\n");
            }
            continue;
        }
        let code = normalize_dialect(&codeu);
        let before_feed = st.feed;
        let mut mv: Vec<GMotion> = Vec::new();
        if let Err(e) = interpret_line(&code, &mut st, &mut mv, line_no) {
            p.errors.push(format!("line {line_no}: {e}"));
            continue;
        }
        let plain_feed = st.feed != before_feed;
        for g in mv {
            if g.machine_coords {
                // G53 Z0 and the like: lift to the highest Z the program uses (decided per section at the end)
                if b.started {
                    b.out.push_str("LIFT\n");
                }
                continue;
            }
            if !b.started {
                if g.kind == MoveType::Rapid && g.from == g.to {
                    continue;
                }
                b.started = true;
                b.sec.tnum = tnum;
                b.sec.rpm = st.rpm;
                let table = tool_table.iter().find(|(t, _)| *t == tnum && tnum > 0).map(|(_, s)| s.clone());
                b.sec.tool_text = table.unwrap_or_else(|| pending_tool.clone());
                b.sec.name = pending_name.clone();
                b.sec.min_z = 1e300;
                b.sec.max_z = -1e300;
            }
            if !pending_name.is_empty() {
                b.out.push_str(&format!("(TP {})\n", pending_name));
                pending_name.clear();
            }
            let (x, y, z) = (g.to.x - dx, g.to.y - dy, g.to.z - dz);
            b.sec.min_z = b.sec.min_z.min(g.to.z);
            b.sec.max_z = b.sec.max_z.max(g.to.z);
            lift_z = lift_z.max(z);
            let sideways = (g.to.x - g.from.x).hypot(g.to.y - g.from.y);
            let dzm = g.to.z - g.from.z;
            let feed = if g.kind == MoveType::Rapid {
                0.0
            } else if plain_feed || (fxy == 0.0 && fz == 0.0) {
                g.feed
            } else if sideways < 1e-9 {
                if fz > 0.0 { fz } else { g.feed }
            } else if fxy > 0.0 {
                fxy
            } else {
                g.feed
            };
            if g.kind != MoveType::Rapid {
                let pts = motion_points(&g, 0.002);
                let len: f64 = pts.windows(2).map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2) + (w[1].z - w[0].z).powi(2)).sqrt()).sum();
                b.sec.cut_length += len;
                b.sec.minutes += len / feed.max(1e-6);
                if sideways > 1e-9 && g.to.z.min(g.from.z) < -1e-4 {
                    b.xy_len += sideways;
                    if dzm.abs() > 1e-4 {
                        b.dz_len += sideways;
                    }
                    if dzm > 1e-4 {
                        b.rising += sideways;
                    }
                }
                if sideways > 1e-9 {
                    b.sec.feed_xy = b.sec.feed_xy.max(feed);
                } else if dzm < 0.0 {
                    b.sec.feed_z = b.sec.feed_z.max(feed);
                }
            } else {
                b.sec.minutes += (sideways.powi(2) + dzm.powi(2)).sqrt() / 1000.0;
            }
            let mut l = match g.kind {
                MoveType::Rapid => format!("G0 X{} Y{} Z{}", fmt(x), fmt(y), fmt(z)),
                MoveType::Linear => format!("G1 X{} Y{} Z{} F{}", fmt(x), fmt(y), fmt(z), fmt(feed)),
                MoveType::ArcCw | MoveType::ArcCcw => format!(
                    "G{} X{} Y{} Z{} I{} J{} F{}",
                    if g.kind == MoveType::ArcCw { 2 } else { 3 },
                    fmt(x),
                    fmt(y),
                    fmt(z),
                    fmt(g.i),
                    fmt(g.j),
                    fmt(feed)
                ),
            };
            l.push('\n');
            b.out.push_str(&l);
        }
    }
    if b.started {
        sections.push(b);
    }
    let lift = lift_z.max(0.2);
    for (k, mut b) in sections.into_iter().enumerate() {
        b.sec.gcode = b.out.replace("LIFT\n", &format!("G0 Z{}\n", fmt(lift)));
        if b.sec.name.is_empty() {
            b.sec.name = if b.sec.tnum > 0 { format!("{stem} T{}", b.sec.tnum) } else if k == 0 { stem.to_string() } else { format!("{stem} {}", k + 1) };
        }
        let mut g = guess_from_text(&b.sec.tool_text);
        if b.sec.tool_text.is_empty() {
            g = ToolGuess::default();
        }
        let from_name = guess_from_name(&b.sec.name);
        g = merge(g, &from_name);
        if g.kind.is_none() && g.diameter.is_none() {
            g = merge(g, &guess_from_name(stem));
        }
        let share = if b.xy_len > 0.0 { b.dz_len / b.xy_len } else { 0.0 };
        let rises = if b.xy_len > 0.0 { b.rising / b.xy_len } else { 0.0 };
        if rises > 0.1 && share > 0.3 {
            b.sec.hints.push("Z rises and falls while cutting: a V-bit or ball nose (V-carve or 3D).".into());
            if g.kind.is_none() {
                g.kind = Some(ToolType::VBit);
                g.source = if g.source.is_empty() { "moves".into() } else { format!("{} + moves", g.source) };
            } else if g.kind == Some(ToolType::EndMill) && !g.form {
                b.sec.hints.push("The file names an end mill but cuts like a V-bit: check the bit.".into());
            }
        }
        if g.form {
            b.sec.hints.push("Shaped bit (ogee, cove ...): simulated with a flat bit of the same size.".into());
        }
        if g.kind == Some(ToolType::VBit) && g.angle.is_none() {
            b.sec.hints.push("V-bit angle unknown.".into());
        }
        if g.diameter.is_none() && g.kind != Some(ToolType::VBit) {
            b.sec.hints.push("Bit diameter unknown.".into());
        }
        b.sec.guess = g;
        p.sections.push(b.sec);
    }
    p
}

/// The stored program of an imported toolpath → toolpaths (one per "(TP name)" block, and a new one whenever the feed
/// changes). Moves are in document coordinates; `name` names blocks without one.
pub fn section_toolpaths(gcode: &str, name: &str) -> Vec<Toolpath> {
    let mut out: Vec<Toolpath> = Vec::new();
    let mut cur = Toolpath { name: name.into(), tool: -1, ..Default::default() };
    let (mut x, mut y) = (f64::NAN, f64::NAN);
    let flush = |cur: &mut Toolpath, out: &mut Vec<Toolpath>, next: String| {
        let next = Toolpath { name: next, tool: -1, ..Default::default() };
        let done = std::mem::replace(cur, next);
        if done.moves.iter().any(|m| m.kind != MoveType::Rapid) {
            out.push(done);
        }
    };
    for l in gcode.lines() {
        if let Some(n) = l.strip_prefix("(TP ").and_then(|r| r.strip_suffix(')')) {
            flush(&mut cur, &mut out, n.to_string());
            continue;
        }
        let mut w = [None::<f64>; 6]; // X Y Z I J F
        let mut g = -1;
        for t in l.split_whitespace() {
            let (c, v) = t.split_at(1);
            let v: f64 = v.parse().unwrap_or(0.0);
            match c {
                "G" => g = v as i32,
                "X" => w[0] = Some(v),
                "Y" => w[1] = Some(v),
                "Z" => w[2] = Some(v),
                "I" => w[3] = Some(v),
                "J" => w[4] = Some(v),
                "F" => w[5] = Some(v),
                _ => {}
            }
        }
        let kind = match g {
            0 => MoveType::Rapid,
            1 => MoveType::Linear,
            2 => MoveType::ArcCw,
            3 => MoveType::ArcCcw,
            _ => continue,
        };
        let sideways = w[0].is_some_and(|v| (v - x).abs() > 1e-9 || x.is_nan()) || w[1].is_some_and(|v| (v - y).abs() > 1e-9 || y.is_nan());
        if let Some(f) = w[5] {
            let slot = if sideways || kind != MoveType::Linear { &mut cur.feed_xy } else { &mut cur.feed_z };
            if *slot == 0.0 {
                *slot = f;
            } else if (*slot - f).abs() > 1e-6 {
                let nm = cur.name.clone();
                flush(&mut cur, &mut out, nm);
                if let Some(&last) = out.last().and_then(|t| t.moves.last()) {
                    cur.moves.push(Move { kind: MoveType::Rapid, i: 0.0, j: 0.0, ..last }); // start where the last block ended
                }
                if sideways || kind != MoveType::Linear { cur.feed_xy = f } else { cur.feed_z = f }
            }
        }
        cur.moves.push(Move { kind, x: w[0], y: w[1], z: w[2], i: w[3].unwrap_or(0.0), j: w[4].unwrap_or(0.0) });
        x = w[0].unwrap_or(x);
        y = w[1].unwrap_or(y);
    }
    flush(&mut cur, &mut out, String::new());
    for tp in &mut out {
        if tp.feed_z == 0.0 {
            tp.feed_z = tp.feed_xy;
        }
        if tp.feed_xy == 0.0 {
            tp.feed_xy = tp.feed_z;
        }
    }
    out
}

/// Cut centrelines of a section as open/closed vectors (XY only, arcs flattened); a path cut again at the next
/// depth pass comes in once.
pub fn section_vectors(gcode: &str) -> Vec<Contour> {
    let mut out: Vec<Contour> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut chain: Vec<(f64, f64)> = Vec::new();
    let mut finish = |chain: &mut Vec<(f64, f64)>| {
        let mut c = std::mem::take(chain);
        c.dedup_by(|a, b| (a.0 - b.0).hypot(a.1 - b.1) < 1e-4);
        if c.len() < 2 {
            return;
        }
        let key = |v: &[(f64, f64)]| v.iter().map(|p| format!("{:.3},{:.3};", p.0, p.1)).collect::<String>();
        let k = key(&c);
        let mut r = c.clone();
        r.reverse();
        if !seen.insert(k) || !seen.insert(key(&r)) {
            return;
        }
        let closed = c.len() > 2 && (c[0].0 - c[c.len() - 1].0).hypot(c[0].1 - c[c.len() - 1].1) < 1e-3;
        if closed {
            c.pop();
        }
        out.push(Contour { nodes: c.iter().map(|&(x, y)| Node::at(pt(x, y))).collect(), closed });
    };
    for tp in section_toolpaths(gcode, "") {
        let mut pos = crate::gcode::Vec3::default();
        for m in &tp.moves {
            let to = crate::gcode::Vec3 { x: m.x.unwrap_or(pos.x), y: m.y.unwrap_or(pos.y), z: m.z.unwrap_or(pos.z) };
            if m.kind == MoveType::Rapid || to.z > -1e-4 && pos.z > -1e-4 {
                finish(&mut chain);
            } else {
                if chain.is_empty() {
                    chain.push((pos.x, pos.y));
                }
                let g = GMotion { kind: m.kind, from: pos, to, i: m.i, j: m.j, ..Default::default() };
                chain.extend(motion_points(&g, 0.002).iter().skip(1).map(|p| (p.x, p.y)));
            }
            pos = to;
        }
        finish(&mut chain);
    }
    out
}

/// Adds a read program to the document: one Imported toolpath per section on the chosen bit (`bits[i]` = index into
/// `doc.tools`, None = a new bit from the file), optionally the board from the file's header and the cut centrelines
/// as vectors (layer "G-code <section>"). Returns a one-line summary. The caller checkpoints for undo.
pub fn apply_import(doc: &mut Document, text: &str, stem: &str, bits: &[Option<usize>], set_board: bool, vectors: bool) -> String {
    if set_board {
        let head = read_program(text, stem, &doc.material);
        if let Some((w, h, t)) = head.board {
            (doc.material.w, doc.material.h, doc.material.t) = (w, h, t);
        }
        if let Some(table) = head.z_zero_table {
            doc.material.z_bottom = table;
        }
        doc.sync_sheet();
    }
    let p = read_program(text, stem, &doc.material); // in this board's coordinates
    let (mut paths, mut shapes) = (0, 0);
    for (i, s) in p.sections.iter().enumerate() {
        let tool = match bits.get(i).copied().flatten().filter(|&t| t < doc.tools.len()) {
            Some(t) => t,
            None => {
                doc.tools.push(tool_from_section(s, &doc.material.kind));
                doc.tools.len() - 1
            }
        };
        doc.ops.push(Op { kind: OpType::Imported, name: s.name.clone(), tool: tool as i32, gcode: s.gcode.clone(), ..Default::default() });
        paths += 1;
        if vectors {
            let layer = doc.layer_named(&format!("G-code {}", s.name));
            for c in section_vectors(&s.gcode) {
                doc.add(c, Some(layer));
                shapes += 1;
            }
        }
    }
    let mut msg = format!("{stem}: {paths} imported toolpath(s)");
    if vectors {
        msg += &format!(", {shapes} vector(s)");
    }
    if !p.errors.is_empty() {
        msg += &format!("; {} line(s) skipped ({})", p.errors.len(), p.errors[0]);
    }
    msg
}

/// The bit in `tools` this section most likely used: the same name as the file's tool comment (a Vectric tool
/// database imported into GCarve keeps VCarve's names), else the same kind and size (V-bits: angle too).
pub fn match_tool(s: &ImportSection, tools: &[Tool]) -> Option<usize> {
    let name = s.tool_text.trim();
    if !name.is_empty() {
        if let Some(i) = tools.iter().position(|t| t.name.trim().eq_ignore_ascii_case(name)) {
            return Some(i);
        }
    }
    let g = &s.guess;
    let kind = g.kind?;
    if g.form {
        return None; // a shaped bit only matches by name
    }
    tools.iter().position(|t| {
        t.kind == kind
            && g.diameter.is_none_or(|d| (t.diameter - d).abs() < 0.002)
            && (kind != ToolType::VBit || g.angle.is_some_and(|a| (t.angle - a).abs() < 0.5))
            && (g.diameter.is_some() || kind == ToolType::VBit)
    })
}

/// A new bit from what the section says, with the file's feeds and RPM as the preset for `material` (so re-posting
/// keeps them instead of the suggested feeds).
pub fn tool_from_section(s: &ImportSection, material: &str) -> Tool {
    let g = &s.guess;
    let kind = g.kind.unwrap_or(ToolType::EndMill);
    let d = g.diameter.unwrap_or(if kind == ToolType::VBit { 0.5 } else { 0.25 });
    let angle = g.angle.unwrap_or(90.0);
    let name = if !s.tool_text.trim().is_empty() {
        s.tool_text.trim().to_string()
    } else {
        match kind {
            ToolType::VBit => format!("V-Bit ({} deg {}\")", fmt(angle), fmt(d)),
            ToolType::BallNose => format!("Ball Nose ({}\")", fmt(d)),
            ToolType::RoundOver => format!("Round Over ({}\")", fmt(d)),
            ToolType::EndMill => format!("End Mill ({}\")", fmt(d)),
        }
    };
    let rpm = if s.rpm > 0.0 { s.rpm.round() as i32 } else { 18000 };
    let (fxy, fz) = (if s.feed_xy > 0.0 { s.feed_xy } else { 100.0 }, if s.feed_z > 0.0 { s.feed_z } else { 50.0 });
    let mut t = Tool { name, kind, diameter: d, angle, rpm, feed_xy: fxy, feed_z: fz, stepdown: (d / 2.0).max(0.05), ..Default::default() };
    if !material.is_empty() {
        t.presets.push(FeedPreset { material: material.into(), feed_xy: fxy, feed_z: fz, stepdown: t.stepdown, rpm });
    }
    t
}

/// One line for the user: "V-bit 90° · 0.5 in" (what is known).
pub fn guess_text(g: &ToolGuess) -> String {
    let kind = match g.kind {
        Some(ToolType::VBit) => "V-bit",
        Some(ToolType::BallNose) => "Ball nose",
        Some(ToolType::RoundOver) => "Round-over",
        Some(ToolType::EndMill) => "End mill",
        None if g.form => "Shaped bit",
        None => "Unknown bit",
    };
    let mut parts = vec![if g.form && g.kind.is_some() { format!("{kind} (shaped)") } else { kind.to_string() }];
    if let Some(a) = g.angle {
        parts.push(format!("{}°", fmt(a)));
    }
    if let Some(d) = g.diameter {
        parts.push(format!("{} in", fmt(d)));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_fraction_is_the_bit_size() {
        let g = guess_from_text("WS 1572 Point Round Over 1/2");
        assert_eq!(g.kind, Some(ToolType::RoundOver));
        assert_eq!(g.diameter, Some(0.5));
    }

    #[test]
    fn tool_comments() {
        let g = guess_from_text("V-Bit (60 deg 0.25\")");
        assert_eq!((g.kind, g.angle, g.diameter), (Some(ToolType::VBit), Some(60.0), Some(0.25)));
        let g = guess_from_text("1/4 Down End Mill (0.25 inch)");
        assert_eq!((g.kind, g.diameter), (Some(ToolType::EndMill), Some(0.25)));
        let g = guess_from_text("1/8 Down End Mill (CL 1 inch)");
        assert_eq!(g.diameter, Some(0.125));
        let g = guess_from_text("End Mill (1/4\")");
        assert_eq!(g.diameter, Some(0.25));
        let g = guess_from_text("3/8 Downcut (0.375)");
        assert_eq!((g.kind, g.diameter), (Some(ToolType::EndMill), Some(0.375)));
        let g = guess_from_text("Point Roundover (3/8\")");
        assert!(!g.form && g.kind == Some(ToolType::RoundOver));
        assert!(guess_from_text("Ogee (1/2\")").form);
        assert_eq!(g.diameter, Some(0.375));
        let g = guess_from_text("T1 D=6.35 CR=0 - ZMIN=-3 - flat end mill");
        assert_eq!(g.kind, Some(ToolType::EndMill));
        assert!((g.diameter.unwrap() - 0.25).abs() < 1e-9);
        let g = guess_from_text("6mm ball nose");
        assert_eq!(g.kind, Some(ToolType::BallNose));
        assert_eq!(guess_from_text("Core Box 3/4 (0.75 inch)").kind, Some(ToolType::BallNose));
        let ro = guess_from_text("1572 Point Round Over 1/2 (0.5 inch)");
        assert_eq!((ro.kind, ro.form, ro.diameter), (Some(ToolType::RoundOver), false, Some(0.5)));
        assert!((g.diameter.unwrap() - 6.0 / 25.4).abs() < 1e-9);
    }

    #[test]
    fn name_codes() {
        let g = guess_from_name("03_EMD14_Dado");
        assert_eq!((g.kind, g.diameter), (Some(ToolType::EndMill), Some(0.25)));
        let g = guess_from_name("01_V60_Josey");
        assert_eq!((g.kind, g.angle), (Some(ToolType::VBit), Some(60.0)));
        assert_eq!(guess_from_name("03_RDC38_Dado [Clear 1]").diameter, Some(0.375));
        assert!(guess_from_name("02_PRO_Holes").form);
        assert_eq!(guess_from_name("Profile 2"), ToolGuess { source: "name".into(), ..Default::default() });
    }

    #[test]
    fn grbl_two_tools_and_roundtrip() {
        let src = "(T1 D=6.35 CR=0 - flat end mill)\n(T2 D=6.35 - ball end mill)\nG21 G90\nM6 T1\nS18000 M3\nG0 X0 Y0 Z5\nG1 Z-2 F500\nG1 X10 F1000\nG0 Z5\nM5\nM6 T2\nM3\nG0 X0 Y0\nG1 Z-1 F300\nG3 X10 Y0 I5 J0 F800\nM5\nM30\n";
        let p = read_program(src, "part", &Material::default());
        assert_eq!(p.sections.len(), 2, "{:?}", p.errors);
        assert_eq!(p.sections[0].guess.kind, Some(ToolType::EndMill));
        assert_eq!(p.sections[1].guess.kind, Some(ToolType::BallNose));
        assert_eq!(p.sections[0].tnum, 1);
        assert!((p.sections[0].feed_xy - 1000.0 / 25.4).abs() < 1e-6);
        let tps = section_toolpaths(&p.sections[0].gcode, "a");
        assert_eq!(tps.len(), 1);
        assert!((tps[0].feed_z - 500.0 / 25.4).abs() < 1e-3, "{:?}", tps[0]);
        let v = section_vectors(&p.sections[0].gcode);
        assert_eq!(v.len(), 1);
        assert!(!v[0].closed);
    }

    #[test]
    fn vcarve_reference() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference/01_V90.tap")).unwrap();
        let p = read_program(&src, "01_V90", &Material::default());
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert_eq!(p.board, Some((48.0, 48.0, 0.484)));
        assert_eq!(p.z_zero_table, Some(false));
        assert_eq!(p.sections.len(), 1);
        let s = &p.sections[0];
        assert_eq!(s.tool_text, "V-Bit (90 deg 0.5\")");
        assert_eq!((s.guess.kind, s.guess.angle, s.guess.diameter), (Some(ToolType::VBit), Some(90.0), Some(0.5)));
        assert_eq!((s.feed_xy, s.feed_z, s.rpm), (100.0, 50.0, 18000.0));
        assert!((s.min_z + 0.484).abs() < 1e-6);
        let tps = section_toolpaths(&s.gcode, &s.name);
        assert!(!tps.is_empty());
        assert_eq!(tps[0].name, "Profile 2");
        let src2 = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference/02_EMD14.tap")).unwrap();
        let p2 = read_program(&src2, "02_EMD14", &Material::default());
        assert_eq!(p2.sections[0].guess.kind, Some(ToolType::EndMill));
        assert_eq!(p2.sections[0].guess.diameter, Some(0.25));
        // matching: the default library has a 90 deg V-bit and a 1/4 end mill
        let tools = crate::document::default_tools();
        let i = match_tool(s, &tools).expect("v-bit");
        assert_eq!((tools[i].kind, tools[i].angle), (ToolType::VBit, 90.0));
        let t = tool_from_section(s, "Plywood");
        assert_eq!((t.name.as_str(), t.rpm, t.presets[0].feed_xy), ("V-Bit (90 deg 0.5\")", 18000, 100.0));
        // into a document, then out again through the WinCNC post: same cut, safe rapids, spindle off
        let mut doc = Document::default();
        let msg = apply_import(&mut doc, &src, "01_V90", &[None], true, true);
        assert!(msg.starts_with("01_V90: 1 imported toolpath(s)"), "{msg}");
        assert_eq!((doc.material.w, doc.material.t), (48.0, 0.484));
        assert!(!doc.shapes.is_empty());
        let g = crate::pipeline::regenerate(&doc);
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);
        let jobs = crate::pipeline::all_jobs(&doc, &g, "", "now");
        assert_eq!(jobs.len(), 1);
        let out = crate::post_wincnc(&jobs[0]);
        let back = read_program(&out, "back", &Material::default());
        assert!(back.errors.is_empty(), "{:?}", back.errors);
        let (o, r) = (&p.sections[0], &back.sections[0]);
        assert!((o.min_z - r.min_z).abs() < 1e-6 && (o.cut_length - r.cut_length).abs() < 1e-3 * o.cut_length, "{} {} / {} {}", o.min_z, o.cut_length, r.min_z, r.cut_length);
        assert_eq!((r.feed_xy, r.feed_z, r.rpm), (100.0, 50.0, 18000.0));
        // saved and loaded: the imported toolpath survives the project file
        let again = crate::project::load_project(&crate::project::save_project(&doc)).unwrap();
        assert_eq!((again.ops[0].kind, &again.ops[0].gcode), (OpType::Imported, &doc.ops[0].gcode));
        if let Ok(out) = std::env::var("GCARVE_IMPORT_SAVE") {
            std::fs::write(out, crate::project::save_project(&doc)).unwrap();
        }
    }
}

/// Survey of a folder of real files: `GCARVE_IMPORT_DIR=... cargo test -p gcarve-core --lib import_survey -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore]
fn import_survey() {
    fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| ["tap", "nc", "gcode", "ngc", "cnc"].contains(&x.to_string_lossy().to_lowercase().as_str())) {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(std::path::Path::new(&std::env::var("GCARVE_IMPORT_DIR").unwrap()), &mut files);
    let (mut secs, mut known, mut errs) = (0, 0, 0);
    for f in &files {
        let text = String::from_utf8_lossy(&std::fs::read(f).unwrap()).to_string();
        let stem = f.file_stem().unwrap().to_string_lossy().to_string();
        let p = read_program(&text, &stem, &crate::document::Material::default());
        errs += p.errors.len();
        for s in &p.sections {
            secs += 1;
            let g = &s.guess;
            let ok = g.kind.is_some() && (g.diameter.is_some() || g.angle.is_some()) || g.form;
            known += usize::from(ok);
            if !ok || !p.errors.is_empty() || s.hints.iter().any(|h| h.contains("check the bit")) {
                println!("{stem} | {} | tool '{}' | {:?} | {:?} | {:?}", s.name, s.tool_text, g, s.hints, p.errors.first());
            }
        }
    }
    println!("{} files, {secs} sections, {known} bits identified, {errs} skipped lines", files.len());
}
