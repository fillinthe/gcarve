//! G-code reader (C++ `core/gcode.*`): one interpreter for previewing programs and for the controller simulator.
//! Standard G-code (GRBL / grblHAL dialect): G0 G1 G2 G3 G4 G10 L20 G17 G20 G21 G53 G54 G90 G91 G92 G92.1 G94, F S M3 M4 M5
//! M30 M2, comments in ( ) or after ;, N line numbers. Positions are inches in work coordinates unless a line uses G53.

use std::f64::consts::PI;

use crate::job::MoveType;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

const fn v3(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct GMotion {
    pub kind: MoveType,
    /// work coordinates, inches
    pub from: Vec3,
    pub to: Vec3,
    /// arc centre offset from `from`
    pub i: f64,
    pub j: f64,
    /// inches/min (0 for rapids)
    pub feed: f64,
    /// G53 move: `to` is in machine coordinates, axes not given are NaN (stay put); `from` is unknown (NaN)
    pub machine_coords: bool,
    /// spindle state while this move runs
    pub spindle_on: bool,
    /// source line (1-based)
    pub line: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GState {
    /// work position, inches
    pub pos: Vec3,
    pub absolute: bool,
    pub inches: bool,
    pub spindle_on: bool,
    pub program_end: bool,
    /// modal G0/1/2/3
    pub motion: i32,
    pub feed: f64,
    pub rpm: f64,
}

impl Default for GState {
    fn default() -> Self {
        GState { pos: Vec3::default(), absolute: true, inches: true, spindle_on: false, program_end: false, motion: 0, feed: 0.0, rpm: 0.0 }
    }
}

/// Side effects of a line besides motions (the C++ out-params).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LineEffect {
    /// G10 L20 P1: new work zero so the current position reads these values
    pub set_work: Option<Vec3>,
    /// G4 dwell, seconds
    pub dwell_sec: Option<f64>,
}

fn strip_comments(s: &str) -> String {
    let mut out = String::new();
    let mut paren = 0;
    for c in s.chars() {
        match c {
            ';' => break,
            '(' => paren += 1,
            ')' => paren = (paren - 1).max(0),
            _ if paren == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn arc_sweep(a: Vec3, b: Vec3, cx: f64, cy: f64, ccw: bool) -> f64 {
    let mut s = (b.y - cy).atan2(b.x - cx) - (a.y - cy).atan2(a.x - cx);
    if ccw {
        while s <= 1e-12 {
            s += 2.0 * PI;
        }
    } else {
        while s >= -1e-12 {
            s -= 2.0 * PI;
        }
    }
    s
}

fn c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Parses one line into motions (0..1 pushed onto `out`) and updates the state. Returns an error message for words
/// it can't run (unsupported G/M codes, bad numbers); the state is left unchanged then.
pub fn interpret_line(raw: &str, st: &mut GState, out: &mut Vec<GMotion>, line_no: i32) -> Result<LineEffect, String> {
    let s = strip_comments(raw);
    let b = s.as_bytes();
    let mut words: Vec<(u8, f64)> = Vec::new();
    let mut k = 0;
    while k < b.len() {
        let c = b[k].to_ascii_uppercase();
        if c_space(c) || c == b'%' {
            k += 1;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            return Err(format!("unexpected '{}'", s[k..].chars().next().unwrap_or('?')));
        }
        k += 1;
        while k < b.len() && c_space(b[k]) {
            k += 1;
        }
        // decimal only: strtod would read "G0X500" (spaces stripped) as hex 0X500
        let start = k;
        if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
            k += 1;
        }
        let d0 = k;
        while k < b.len() && (b[k].is_ascii_digit() || b[k] == b'.') {
            k += 1;
        }
        if k == d0 {
            return Err(format!("missing number after {}", c as char));
        }
        words.push((c, atof(&s[start..k])));
    }
    if words.is_empty() {
        return Ok(LineEffect::default());
    }
    let mut n = *st;
    let (mut motion, mut g53, mut g10, mut g92, mut g4, mut l) = (-1, false, false, false, false, 0);
    let [mut x, mut y, mut z, mut i, mut j, mut p]: [Option<f64>; 6] = [None; 6];
    for &(c, v) in &words {
        let code = (v * 10.0).round() as i64; // G-codes with decimals (G38.2) arrive as 382
        match c {
            b'G' => match code {
                0 | 10 | 20 | 30 => motion = (code / 10) as i32,
                40 => g4 = true,
                100 => g10 = true,
                // G92: the current position reads these values (GRBL lasers' "start from here"); G92.1 clears it
                920 => {
                    g10 = true;
                    g92 = true;
                }
                921 => {}
                170 | 400 | 490 | 540 | 800 | 940 | 610 | 640 => {}
                // cutter compensation: our files keep the bit-centre path, D holds only the wear, so the preview ignores it
                410 | 420 => {}
                200 => n.inches = true,
                210 => n.inches = false,
                530 => g53 = true,
                900 => n.absolute = true,
                910 => n.absolute = false,
                _ => return Err(format!("unsupported G{}{}", code / 10, if code % 10 != 0 { format!(".{}", code % 10) } else { String::new() })),
            },
            b'M' => match code {
                30 | 40 => n.spindle_on = true,
                50 => n.spindle_on = false,
                20 | 300 => {
                    n.program_end = true;
                    n.spindle_on = false;
                }
                0 | 10 | 60 | 70 | 80 | 90 => {}
                _ => return Err(format!("unsupported M{}", code / 10)),
            },
            b'F' => n.feed = v * if n.inches { 1.0 } else { 1.0 / 25.4 },
            b'S' => n.rpm = v,
            b'X' => x = Some(v),
            b'Y' => y = Some(v),
            b'Z' => z = Some(v),
            b'I' => i = Some(v),
            b'J' => j = Some(v),
            b'P' => p = Some(v),
            b'L' => l = v as i32,
            b'N' | b'T' | b'H' | b'K' | b'D' => {}
            _ => return Err(format!("unsupported word {}", c as char)),
        }
    }
    let u = if n.inches { 1.0 } else { 1.0 / 25.4 };
    if g4 {
        *st = n;
        return Ok(LineEffect { dwell_sec: Some(p.unwrap_or(0.0)), ..Default::default() });
    }
    if g10 {
        if l != 20 && !g92 {
            return Err("only G10 L20 is supported".into());
        }
        let w = v3(x.map_or(n.pos.x, |v| v * u), y.map_or(n.pos.y, |v| v * u), z.map_or(n.pos.z, |v| v * u));
        n.pos = w;
        *st = n;
        return Ok(LineEffect { set_work: Some(w), ..Default::default() });
    }
    if motion >= 0 {
        n.motion = motion;
    }
    if x.is_some() || y.is_some() || z.is_some() {
        let mut m = GMotion { line: line_no, from: n.pos, ..Default::default() };
        let mut to = n.pos;
        if g53 {
            m.machine_coords = true;
            m.from = v3(f64::NAN, f64::NAN, f64::NAN);
            to = v3(x.map_or(f64::NAN, |v| v * u), y.map_or(f64::NAN, |v| v * u), z.map_or(f64::NAN, |v| v * u));
        } else if n.absolute {
            to = v3(x.map_or(to.x, |v| v * u), y.map_or(to.y, |v| v * u), z.map_or(to.z, |v| v * u));
        } else {
            to = v3(x.map_or(to.x, |v| to.x + v * u), y.map_or(to.y, |v| to.y + v * u), z.map_or(to.z, |v| to.z + v * u));
        }
        let mo = if g53 { if n.motion <= 1 { n.motion } else { 0 } } else { n.motion };
        m.kind = match mo {
            0 => MoveType::Rapid,
            1 => MoveType::Linear,
            2 => MoveType::ArcCw,
            _ => MoveType::ArcCcw,
        };
        if mo >= 1 && n.feed <= 0.0 {
            return Err("feed move without a feed rate (F)".into());
        }
        if mo >= 2 {
            if i.is_none() && j.is_none() {
                return Err("arc needs I/J (R arcs are not supported)".into());
            }
            m.i = i.unwrap_or(0.0) * u;
            m.j = j.unwrap_or(0.0) * u;
        }
        m.feed = if mo == 0 { 0.0 } else { n.feed };
        m.spindle_on = n.spindle_on;
        m.to = to;
        if !g53 {
            n.pos = to;
        }
        out.push(m);
    }
    *st = n;
    Ok(LineEffect::default())
}

/// C atof over the `[+-]?[0-9.]+` run the reader collected: longest numeric prefix ("1.2.3" -> 1.2, "." -> 0)
fn atof(s: &str) -> f64 {
    let b = s.as_bytes();
    let mut k = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    while k < b.len() && b[k].is_ascii_digit() {
        k += 1;
    }
    if k < b.len() && b[k] == b'.' {
        k += 1;
        while k < b.len() && b[k].is_ascii_digit() {
            k += 1;
        }
    }
    s[..k].parse().unwrap_or(0.0)
}

/// Points along a motion (arcs flattened), for drawing and simulation. Tolerance in inches (C++ default 0.002).
pub fn motion_points(m: &GMotion, tol: f64) -> Vec<Vec3> {
    if matches!(m.kind, MoveType::Rapid | MoveType::Linear) {
        return vec![m.from, m.to];
    }
    let (cx, cy, r) = (m.from.x + m.i, m.from.y + m.j, m.i.hypot(m.j));
    let ccw = m.kind == MoveType::ArcCcw;
    let mut sw = arc_sweep(m.from, m.to, cx, cy, ccw);
    if (m.to.x - m.from.x).hypot(m.to.y - m.from.y) < 1e-9 {
        sw = if ccw { 2.0 * PI } else { -2.0 * PI }; // full circle
    }
    let step = if r > tol { 2.0 * (1.0 - tol / r).max(-1.0).acos() } else { PI / 8.0 };
    let n = ((sw.abs() / step.max(1e-3)).ceil() as i32).max(2);
    let a0 = (m.from.y - cy).atan2(m.from.x - cx);
    let mut pts: Vec<Vec3> = (0..=n)
        .map(|k| {
            let t = f64::from(k) / f64::from(n);
            v3(cx + r * (a0 + sw * t).cos(), cy + r * (a0 + sw * t).sin(), m.from.z + (m.to.z - m.from.z) * t)
        })
        .collect();
    *pts.last_mut().unwrap() = m.to;
    pts
}

#[derive(Clone, Debug, PartialEq)]
pub struct GProgram {
    /// original text, one entry per line
    pub lines: Vec<String>,
    pub motions: Vec<GMotion>,
    /// "line N: ..."
    pub errors: Vec<String>,
    /// cutting + rapid extents in work coordinates (G53 moves excluded)
    pub min: Vec3,
    pub max: Vec3,
    /// M5 (or program end with the spindle off) after the last motion
    pub spindle_off_at_end: bool,
    /// at programmed feeds; rapids at `rapid_ipm`
    pub minutes: f64,
}

impl Default for GProgram {
    fn default() -> Self {
        GProgram {
            lines: Vec::new(),
            motions: Vec::new(),
            errors: Vec::new(),
            min: v3(1e300, 1e300, 1e300),
            max: v3(-1e300, -1e300, -1e300),
            spindle_off_at_end: false,
            minutes: 0.0,
        }
    }
}

/// `rapid_ipm`: rapid speed for the time estimate (C++ default 1000).
pub fn parse_program(text: &str, rapid_ipm: f64) -> GProgram {
    let mut p = GProgram::default();
    let mut st = GState::default();
    let (mut n, mut last_motion, mut last_off) = (0, -1, -1);
    // std::getline: no empty line after a trailing newline
    let body = text.strip_suffix('\n').unwrap_or(text);
    for raw in body.split('\n').filter(|_| !text.is_empty()) {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        p.lines.push(line.to_string());
        n += 1;
        let before = p.motions.len();
        let was_on = st.spindle_on;
        if let Err(e) = interpret_line(line, &mut st, &mut p.motions, n) {
            p.errors.push(format!("line {n}: {e}"));
            continue;
        }
        for k in before..p.motions.len() {
            let m = p.motions[k];
            if m.machine_coords {
                continue; // G53: machine position, not part of the program's extents or timing
            }
            let pts = motion_points(&m, 0.002);
            let len: f64 = pts.windows(2).map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2) + (w[1].z - w[0].z).powi(2)).sqrt()).sum();
            p.minutes += len / if m.kind == MoveType::Rapid { rapid_ipm } else { m.feed.max(1e-6) };
            for q in &pts[usize::from(k == 0)..] {
                // the start of the first move is unknown
                p.min = v3(p.min.x.min(q.x), p.min.y.min(q.y), p.min.z.min(q.z));
                p.max = v3(p.max.x.max(q.x), p.max.y.max(q.y), p.max.z.max(q.z));
            }
            let sideways = (m.to.x - m.from.x).hypot(m.to.y - m.from.y) > 1e-9;
            if m.kind != MoveType::Rapid || sideways {
                last_motion = n; // a Z-only lift after M5 is fine
            }
        }
        if was_on && !st.spindle_on {
            last_off = n;
        }
    }
    p.spindle_off_at_end = last_off > last_motion && !st.spindle_on;
    p
}
