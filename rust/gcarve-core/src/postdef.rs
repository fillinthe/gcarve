//! Template-driven post-processors (C++ `post/postdef.*`). A [`PostDef`] describes a controller's dialect;
//! [`post_jobs`] renders Jobs with it. Checked byte for byte against the C++ output in tests/golden_cpp.rs.
//!
//! Placeholders in templates: {FILENAME} {DATE} {TIME} {TOOLNAME} {TOOLNUM} {RPM} {FEEDXY} {FEEDZ} {SAFEZ} {ZZERO}
//! {MATERIAL} {UNITS} {PATHNAME} {XMIN} {XMAX} {YMIN} {YMAX} {ZMIN} {ZMAX} {N} (line number) and in move templates
//! {X} {Y} {Z} {I} {J} {F} (axis letter + number; omitted when absent) or {x} {y} {z} {i} {j} {f} (bare numbers,
//! always emitted; ShopBot style).
//!
//! SAFETY (user rule): every file ends with the spindle off and every tool change stops it first, for every post.
//! [`post_jobs`] enforces it via [`spindle_safe_footer`] / [`spindle_safe_tool_change`]; never bypass them.

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::PI;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::geom::{dist, pt, Pt, Rect};
use crate::job::{Job, Move, MoveType};
use crate::wincnc::{num, post_wincnc};

/// Same fields and JSON keys as the C++ struct (user posts are saved as this JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PostDef {
    pub id: String,
    pub name: String,
    /// for searching: e.g. "GRBL"
    pub controller: String,
    /// for searching: e.g. "Shapeoko, X-Carve, Longmill"
    pub machines: String,
    pub extension: String,
    /// output units (jobs are always inches internally; mm posts convert)
    pub inches: bool,
    pub decimals: i32,
    /// false: arcs are written as short lines
    pub arcs: bool,
    /// split arcs beyond this
    pub max_arc_sweep_deg: f64,
    /// {} = text; "" = no comments
    pub comment: String,
    /// once per file
    pub header: String,
    /// between tools in a combined file; empty = one file per tool
    pub tool_change: String,
    /// before each toolpath
    pub path_start: String,
    /// after each toolpath
    pub path_end: String,
    pub footer: String,
    pub rapid: String,
    pub linear: String,
    #[serde(rename = "arcCW")]
    pub arc_cw: String,
    #[serde(rename = "arcCCW")]
    pub arc_ccw: String,
    /// emitted on its own when the feed changes (WinCNC "F{f} XY"); then {F} in moves is dropped
    pub feed_change: String,
    /// Z-only feed move (Vectric PLUNGE_MOVE); empty = linear
    pub plunge: String,
    /// Z-only rapid up (Vectric RETRACT_MOVE); empty = rapid
    pub retract: String,
    /// first feed move after a rapid (Vectric FIRST_FEED_MOVE); empty = linear
    pub first_linear: String,
    /// char pairs applied to names in the output, Vectric SUBSTITUTE ("({)}" = ( -> {, ) -> })
    pub substitute: String,
    /// digits after the point for {F}/{f}/{FEEDXY}/{FEEDZ}
    pub feed_decimals: i32,
    /// {N} in templates = running line number
    pub line_number_start: i32,
    pub line_number_step: i32,
    /// omit unchanged axes in {X}{Y}{Z}
    pub modal_axes: bool,
    pub crlf: bool,
}

impl Default for PostDef {
    fn default() -> Self {
        PostDef {
            id: String::new(),
            name: String::new(),
            controller: String::new(),
            machines: String::new(),
            extension: "nc".into(),
            inches: true,
            decimals: 4,
            arcs: true,
            max_arc_sweep_deg: 180.0,
            comment: "({})".into(),
            header: String::new(),
            tool_change: String::new(),
            path_start: String::new(),
            path_end: String::new(),
            footer: String::new(),
            rapid: "G0 {X} {Y} {Z}".into(),
            linear: "G1 {X} {Y} {Z} {F}".into(),
            arc_cw: "G2 {X} {Y} {Z} {I} {J} {F}".into(),
            arc_ccw: "G3 {X} {Y} {Z} {I} {J} {F}".into(),
            feed_change: String::new(),
            plunge: String::new(),
            retract: String::new(),
            first_linear: String::new(),
            substitute: String::new(),
            feed_decimals: 0,
            line_number_start: 10,
            line_number_step: 10,
            modal_axes: true,
            crlf: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostOutput {
    pub filename: String,
    pub text: String,
}

fn fmt(v: f64, dec: i32) -> String {
    num(v, dec.max(0) as usize)
}

/// std::getline semantics: split on '\n', no empty piece after a trailing newline, '\r' kept.
fn getlines(s: &str) -> impl Iterator<Item = &str> {
    s.strip_suffix('\n').map_or((!s.is_empty()).then_some(s), Some).into_iter().flat_map(|t| t.split('\n'))
}

/// collapse runs of spaces left by omitted placeholders
fn trim_spaces(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut sp = false;
    for c in s.chars() {
        if c == ' ' {
            if !sp && !o.is_empty() {
                o.push(' ');
            }
            sp = true;
        } else {
            o.push(c);
            sp = false;
        }
    }
    while o.ends_with(' ') {
        o.pop();
    }
    o
}

/// Vectric SUBSTITUTE pairs, e.g. "({)}" so names can't close a comment (byte pairs, like C++)
fn substituted(d: &PostDef, s: &str) -> String {
    let sub = d.substitute.as_bytes();
    let mut b = s.as_bytes().to_vec();
    for p in sub.as_chunks::<2>().0 {
        for c in b.iter_mut() {
            if *c == p[0] {
                *c = p[1];
            }
        }
    }
    String::from_utf8(b).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// {N} -> running line number, in output order
pub fn number_lines(out: &str, d: &PostDef) -> String {
    let mut n = i64::from(d.line_number_start);
    let mut parts = out.split("{N}");
    let mut o = parts.next().unwrap_or("").to_string();
    for p in parts {
        o += &n.to_string();
        o += p;
        n += i64::from(d.line_number_step);
    }
    o
}

struct Ctx<'a> {
    def: &'a PostDef,
    scale: f64, // inches -> output units
    vars: BTreeMap<String, String>, // sorted like the C++ std::map: expansion order matters when values hold braces
    x: Option<f64>, // last emitted
    y: Option<f64>,
    z: Option<f64>,
    f: Option<f64>,
    after_rapid: bool,
}

impl Ctx<'_> {
    fn expand(&self, tpl: &str) -> String {
        let mut s = tpl.to_string();
        for (k, v) in &self.vars {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }
    fn set(&mut self, k: &str, v: String) {
        self.vars.insert(k.into(), v);
    }
    fn nl(&self) -> &'static str {
        if self.def.crlf { "\r\n" } else { "\n" }
    }
    fn emit_lines(&self, out: &mut String, tpl: &str) {
        if tpl.is_empty() {
            return;
        }
        let s = self.expand(tpl);
        for l in getlines(&s) {
            out.push_str(l);
            out.push_str(self.nl());
        }
    }
    fn axis_tok(&self, letter: &str, v: Option<f64>, last: Option<f64>, force: bool) -> String {
        let Some(v) = v else { return String::new() };
        if self.def.modal_axes && !force && last.is_some_and(|l| (l - v).abs() < 1e-12) {
            return String::new();
        }
        format!("{letter}{}", fmt(v * self.scale, self.def.decimals))
    }
}

/// arcs -> lines when the post can't take them, or arcs split when the sweep is too large
fn linearize(m: &Move, from: Pt, max_sweep_deg: f64, allow_arcs: bool) -> Vec<Move> {
    let ctr = from + pt(m.i, m.j);
    let to = pt(m.x.unwrap_or(from.x), m.y.unwrap_or(from.y));
    let r = dist(ctr, from);
    let a0 = (from.y - ctr.y).atan2(from.x - ctr.x);
    let a1 = (to.y - ctr.y).atan2(to.x - ctr.x);
    let mut sweep = a1 - a0;
    let ccw = m.kind == MoveType::ArcCcw;
    if ccw && sweep <= 1e-12 {
        sweep += 2.0 * PI;
    }
    if !ccw && sweep >= -1e-12 {
        sweep -= 2.0 * PI;
    }
    let mut out = Vec::new();
    if allow_arcs {
        let parts = ((sweep.abs() * 180.0 / PI / max_sweep_deg - 1e-9).ceil() as i32).max(1);
        for k in 1..=parts {
            let a = a0 + sweep * f64::from(k) / f64::from(parts);
            let ap = a0 + sweep * f64::from(k - 1) / f64::from(parts);
            let p = pt(ctr.x + r * a.cos(), ctr.y + r * a.sin());
            let pp = pt(ctr.x + r * ap.cos(), ctr.y + r * ap.sin());
            let mut mm = *m;
            mm.x = Some(p.x);
            mm.y = Some(p.y);
            mm.i = ctr.x - pp.x;
            mm.j = ctr.y - pp.y;
            if k == parts {
                mm.x = Some(to.x);
                mm.y = Some(to.y);
            }
            out.push(mm);
        }
        return out;
    }
    let n = ((sweep.abs() / (5.0 * PI / 180.0)).ceil() as i32).max(2);
    for k in 1..=n {
        let a = a0 + sweep * f64::from(k) / f64::from(n);
        let mut mm = Move { kind: MoveType::Linear, x: Some(ctr.x + r * a.cos()), y: Some(ctr.y + r * a.sin()), z: m.z, ..Default::default() };
        if k == n {
            mm.x = Some(to.x);
            mm.y = Some(to.y);
        }
        out.push(mm);
    }
    out
}

fn emit_move(out: &mut String, c: &mut Ctx, m: &Move, feed_xy: f64, feed_z: f64) {
    let d = c.def;
    let arc = matches!(m.kind, MoveType::ArcCw | MoveType::ArcCcw);
    let same = |a: Option<f64>, b: Option<f64>| a.is_none_or(|a| b.is_some_and(|b| (a - b).abs() < 1e-9));
    // plunge / retract: only Z moves
    let z_only = m.z.is_some() && same(m.x, c.x) && same(m.y, c.y) && !c.z.zip(m.z).is_some_and(|(cz, mz)| (mz - cz).abs() < 1e-9);
    let feed = if z_only { feed_z } else { feed_xy };
    let mut tpl = match m.kind {
        MoveType::Rapid => &d.rapid,
        MoveType::Linear => &d.linear,
        MoveType::ArcCw => &d.arc_cw,
        MoveType::ArcCcw => &d.arc_ccw,
    };
    if z_only && m.kind == MoveType::Linear && !d.plunge.is_empty() {
        tpl = &d.plunge;
    }
    if z_only && m.kind == MoveType::Rapid && !d.retract.is_empty() && c.z.zip(m.z).is_some_and(|(cz, mz)| mz > cz) {
        tpl = &d.retract;
    }
    if m.kind == MoveType::Linear && c.after_rapid && !d.first_linear.is_empty() {
        tpl = &d.first_linear;
    }
    let feed_changed = m.kind != MoveType::Rapid && c.f.is_none_or(|f| (f - feed).abs() > 1e-9);
    if feed_changed && !d.feed_change.is_empty() {
        c.set("f", fmt(feed * c.scale, d.feed_decimals));
        c.set("F", format!("F{}", fmt(feed * c.scale, d.feed_decimals)));
        c.emit_lines(out, &d.feed_change);
    }
    let sc = c.scale;
    let mut s = c.expand(tpl);
    s = s.replace("{X}", &c.axis_tok("X", m.x, c.x, arc));
    s = s.replace("{Y}", &c.axis_tok("Y", m.y, c.y, arc));
    s = s.replace("{Z}", &c.axis_tok("Z", m.z, c.z, false));
    s = s.replace("{I}", &if arc { format!("I{}", fmt(m.i * sc, d.decimals)) } else { String::new() });
    s = s.replace("{J}", &if arc { format!("J{}", fmt(m.j * sc, d.decimals)) } else { String::new() });
    let fword = if m.kind != MoveType::Rapid && feed_changed && d.feed_change.is_empty() { format!("F{}", fmt(feed * sc, d.feed_decimals)) } else { String::new() };
    s = s.replace("{F}", &fword);
    s = s.replace("{x}", &fmt(m.x.or(c.x).unwrap_or(0.0) * sc, d.decimals));
    s = s.replace("{y}", &fmt(m.y.or(c.y).unwrap_or(0.0) * sc, d.decimals));
    s = s.replace("{z}", &fmt(m.z.or(c.z).unwrap_or(0.0) * sc, d.decimals));
    s = s.replace("{i}", &fmt(m.i * sc, d.decimals));
    s = s.replace("{j}", &fmt(m.j * sc, d.decimals));
    s = s.replace("{f}", &fmt(feed * sc, d.feed_decimals));
    let s = trim_spaces(&s);
    if !s.is_empty() {
        // move templates may span lines (Vectric blocks); emit_lines expands again, like C++
        c.emit_lines(out, &s);
    }
    if m.x.is_some() {
        c.x = m.x;
    }
    if m.y.is_some() {
        c.y = m.y;
    }
    if m.z.is_some() {
        c.z = m.z;
    }
    if m.kind != MoveType::Rapid {
        c.f = Some(feed);
    }
    c.after_rapid = m.kind == MoveType::Rapid;
}

fn emit_job(out: &mut String, c: &mut Ctx, job: &Job, tool_num: i32) {
    let d = c.def;
    c.set("TOOLNAME", substituted(d, &job.tool));
    c.set("TOOLNUM", tool_num.to_string());
    c.set("RPM", job.rpm.to_string());
    c.set("ZZERO", job.z_zero.clone());
    c.set("MATERIAL", job.material.clone());
    for tp in &job.toolpaths {
        let sc = c.scale;
        c.set("PATHNAME", substituted(d, &tp.name));
        c.set("FEEDXY", fmt(tp.feed_xy * sc, d.feed_decimals));
        c.set("FEEDZ", fmt(tp.feed_z * sc, d.feed_decimals));
        let mut b = Rect::default();
        let mut z: Option<(f64, f64)> = None;
        for m in &tp.moves {
            if let (Some(x), Some(y)) = (m.x, m.y) {
                b.add(pt(x, y));
            }
            if let Some(mz) = m.z {
                z = Some(z.map_or((mz, mz), |(lo, hi)| (lo.min(mz), hi.max(mz))));
            }
        }
        let (zmin, zmax) = z.unwrap_or((0.0, 0.0));
        let e = b.is_empty();
        let bx = |v: f64| fmt(if e { 0.0 } else { v * sc }, d.decimals);
        c.set("XMIN", bx(b.x0));
        c.set("XMAX", bx(b.x1));
        c.set("YMIN", bx(b.y0));
        c.set("YMAX", bx(b.y1));
        c.set("ZMIN", fmt(zmin * sc, d.decimals));
        c.set("ZMAX", fmt(zmax * sc, d.decimals));
        c.emit_lines(out, &d.path_start);
        c.f = None; // force a feed word at the start of each path
        let mut cur = pt(c.x.unwrap_or(0.0), c.y.unwrap_or(0.0));
        for m in &tp.moves {
            if matches!(m.kind, MoveType::ArcCw | MoveType::ArcCcw) {
                for mm in linearize(m, cur, d.max_arc_sweep_deg, d.arcs) {
                    emit_move(out, c, &mm, tp.feed_xy, tp.feed_z);
                    cur = pt(mm.x.unwrap_or(cur.x), mm.y.unwrap_or(cur.y));
                }
            } else {
                emit_move(out, c, m, tp.feed_xy, tp.feed_z);
                cur = pt(m.x.unwrap_or(cur.x), m.y.unwrap_or(cur.y));
            }
        }
        c.emit_lines(out, &d.path_end);
    }
}

/// header notes as comments in the post's style ("({})", "[{}]", ";{}"...); posts without comments skip them
fn emit_notes(out: &mut String, def: &PostDef, notes: &[String]) {
    let Some(at) = def.comment.find("{}") else { return };
    for n in notes {
        out.push_str(&def.comment[..at]);
        out.push_str(&substituted(def, n));
        out.push_str(&def.comment[at + 2..]);
        out.push_str(if def.crlf { "\r\n" } else { "\n" });
    }
}

fn c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Does `l` contain one of `words` (case-insensitive) as a whole token: preceded by start/whitespace/`;` (when
/// `anywhere`, else only by leading whitespace) and followed by end/whitespace(/`;` when `semi`)?
/// Stands in for the C++ regexes (no regex dependency).
fn has_word(l: &str, words: &[&str], anywhere: bool, semi: bool) -> bool {
    let b = l.as_bytes();
    let starts: Vec<usize> = if anywhere {
        (0..=b.len()).filter(|&p| p == 0 || c_space(b[p - 1]) || b[p - 1] == b';').collect()
    } else {
        vec![b.iter().position(|&c| !c_space(c)).unwrap_or(b.len())]
    };
    let term = |p: usize| p == b.len() || c_space(b[p]) || (semi && b[p] == b';');
    starts.into_iter().any(|p| {
        words.iter().any(|w| b.len() >= p + w.len() && b[p..p + w.len()].eq_ignore_ascii_case(w.as_bytes()) && term(p + w.len()))
            || (anywhere && shopbot_off(b, p).is_some_and(term))
    })
}

/// `SO,\s*\d+,\s*0` at p (ShopBot output off): returns the end
fn shopbot_off(b: &[u8], p: usize) -> Option<usize> {
    let mut k = p;
    if !b.get(k..k + 3)?.eq_ignore_ascii_case(b"SO,") {
        return None;
    }
    k += 3;
    while b.get(k).is_some_and(|&c| c_space(c)) {
        k += 1;
    }
    let d0 = k;
    while b.get(k).is_some_and(u8::is_ascii_digit) {
        k += 1;
    }
    if k == d0 || b.get(k) != Some(&b',') {
        return None;
    }
    k += 1;
    while b.get(k).is_some_and(|&c| c_space(c)) {
        k += 1;
    }
    (b.get(k) == Some(&b'0')).then_some(k + 1)
}

/// spindle off: M5 / M05 (any G-code), C7 (ShopBot), SO,n,0 (ShopBot output off), !MC0 (Roland)
fn spindle_off(l: &str) -> bool {
    has_word(l, &["M5", "M05", "C7", "!MC0"], true, true)
}

/// Safety: every file must end with the spindle off. A footer without a spindle-off command (user .json / Vectric
/// .pp posts) gets "M5" before its program end (M30/M2/%/END), or at the end.
pub fn spindle_safe_footer(footer: &str) -> String {
    if getlines(footer).any(spindle_off) {
        return footer.to_string();
    }
    let mut out = String::new();
    let mut added = false;
    for l in getlines(footer) {
        if !added && has_word(l, &["M30", "M030", "M2", "M02", "%", "END"], false, false) {
            out += "M5\n";
            added = true;
        }
        out += l;
        out += "\n";
    }
    if !added {
        out += "M5\n";
    }
    out
}

/// Safety: the spindle stops before every tool change (a user post's change block without M5 gets one first).
pub fn spindle_safe_tool_change(block: &str) -> String {
    if spindle_safe_footer(block) == block { block.to_string() } else { format!("M5\n{block}") }
}

/// Renders one or more files. With `tool_change` empty: one file per job (tool). Otherwise a single combined file.
pub fn post_jobs(jobs: &[Job], def: &PostDef) -> Vec<PostOutput> {
    let mut outs = Vec::new();
    if jobs.is_empty() {
        return outs;
    }
    if def.id == "wincnc-single" {
        // byte-exact VCarve-compatible writer
        return jobs.iter().map(|j| PostOutput { filename: format!("{}.{}", j.filename, def.extension), text: post_wincnc(j) }).collect();
    }
    let scale = if def.inches { 1.0 } else { 25.4 };
    let make_ctx = |j: &Job| {
        let vars = [
            ("FILENAME", substituted(def, &j.filename)),
            ("DATE", j.timestamp.clone()),
            ("TIME", j.timestamp.clone()),
            ("UNITS", (if def.inches { "inches" } else { "mm" }).into()),
            ("SAFEZ", fmt(0.8 * scale, def.decimals)),
            ("MATERIAL", substituted(def, &j.material)),
            ("TOOLNAME", substituted(def, &j.tool)),
            ("TOOLNUM", j.slot.max(1).to_string()),
            ("RPM", j.rpm.to_string()),
            ("ZZERO", j.z_zero.clone()),
            ("PATHNAME", String::new()),
            ("FEEDXY", "0".into()),
            ("FEEDZ", "0".into()),
        ];
        Ctx { def, scale, vars: vars.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), x: None, y: None, z: None, f: None, after_rapid: true }
    };
    if def.tool_change.is_empty() {
        for (n, j) in (1..).zip(jobs) {
            let mut c = make_ctx(j);
            let mut out = String::new();
            c.emit_lines(&mut out, &def.header);
            emit_notes(&mut out, def, &j.notes);
            emit_job(&mut out, &mut c, j, if j.slot > 0 { j.slot } else { n });
            c.emit_lines(&mut out, &spindle_safe_footer(&def.footer));
            outs.push(PostOutput { filename: format!("{}.{}", j.filename, def.extension), text: number_lines(&out, def) });
        }
    } else {
        let mut c = make_ctx(&jobs[0]);
        let mut out = String::new();
        let mut base = jobs[0].filename.as_str();
        if let Some(us) = base.find('_').filter(|&us| us <= 2) {
            base = &base[us + 1..];
        }
        c.set("FILENAME", base.into());
        c.emit_lines(&mut out, &def.header);
        emit_notes(&mut out, def, &jobs[0].notes);
        let change = spindle_safe_tool_change(&def.tool_change);
        for (n, j) in (1..).zip(jobs) {
            c.set("TOOLNAME", substituted(def, &j.tool));
            let num = if j.slot > 0 { j.slot } else { n };
            c.set("TOOLNUM", num.to_string());
            c.set("RPM", j.rpm.to_string());
            c.emit_lines(&mut out, &change);
            (c.x, c.y, c.z, c.f, c.after_rapid) = (None, None, None, None, true);
            emit_job(&mut out, &mut c, j, num);
        }
        c.emit_lines(&mut out, &spindle_safe_footer(&def.footer));
        outs.push(PostOutput { filename: format!("{base}.{}", def.extension), text: number_lines(&out, def) });
    }
    outs
}

/// built-in machine posts (47 checked against the C++ app + the industrial ISO family)
pub fn post_library() -> &'static [PostDef] {
    static LIB: OnceLock<Vec<PostDef>> = OnceLock::new();
    LIB.get_or_init(build_library)
}

/// None if unknown
pub fn find_post(id: &str) -> Option<&'static PostDef> {
    post_library().iter().find(|d| d.id == id)
}

fn build_library() -> Vec<PostDef> {
    let mut v: Vec<PostDef> = Vec::new();
    let base = |id: &str, name: &str, controller: &str, machines: &str, ext: &str| PostDef {
        id: id.into(),
        name: name.into(),
        controller: controller.into(),
        machines: machines.into(),
        extension: ext.into(),
        ..Default::default()
    };
    let grbl_like = |v: &mut Vec<PostDef>, id, name, controller, machines, inch: bool, ext| {
        let mut d = base(id, name, controller, machines, ext);
        d.inches = inch;
        d.decimals = if inch { 4 } else { 3 };
        d.max_arc_sweep_deg = 180.0;
        d.header = format!("({{FILENAME}})\n(Tool: {{TOOLNAME}})\nG90 {} G17 G94\nG54\nM3 S{{RPM}}\nG4 P2\n", if inch { "G20" } else { "G21" });
        d.path_start = "({PATHNAME})\n".into();
        d.footer = "M5\nG0 Z{SAFEZ}\nM30\n".into();
        v.push(d);
    };
    let mach_like = |v: &mut Vec<PostDef>, id, name, controller, machines, inch: bool, atc: bool, ext, retract: &str| {
        let mut d = base(id, name, controller, machines, ext);
        d.inches = inch;
        d.decimals = if inch { 4 } else { 3 };
        d.header = format!("({{FILENAME}})\nG90 {} G17 G40 G49 G80\n", if inch { "G20" } else { "G21" });
        if atc {
            d.tool_change = format!("({{TOOLNAME}})\nM5\n{retract}\nT{{TOOLNUM}} M6\nG43 H{{TOOLNUM}}\nS{{RPM}} M3\n");
        } else {
            d.header += "(Tool: {TOOLNAME})\nT1 M6\nS{RPM} M3\n";
        }
        d.path_start = "({PATHNAME})\n".into();
        d.footer = format!("M5\n{retract}\nM30\n");
        v.push(d);
    };
    const G53: &str = "G53 G0 Z0";
    // ---- WinCNC family (the exact VCarve dialect we validated, plus ATC)
    v.push(PostDef { crlf: true, ..base("wincnc-single", "CAMaster / WinCNC single tool (inch, .tap)", "WinCNC", "CAMaster Stinger, Panther, Cobra; ShopSabre; Legacy", "tap") });
    v.push(PostDef {
        crlf: true,
        comment: "[{}]".into(),
        header: "[{FILENAME}]\n[{DATE}]\nG90 G20\nG53 Z0\n".into(),
        tool_change: "[TOOL: {TOOLNAME}]\nM5\nG53 Z0\nT{TOOLNUM} M6\nS{RPM} M3\nG53 Z0\n".into(),
        path_start: "[TOOLPATH NAME: {PATHNAME}]\n".into(),
        feed_change: "F{f}".into(),
        rapid: "G0 {X} {Y} {Z}".into(),
        linear: "G1 {X} {Y} {Z}".into(),
        arc_cw: "G2 {X} {Y} {Z} {I} {J}".into(),
        arc_ccw: "G3 {X} {Y} {Z} {I} {J}".into(),
        footer: "G53 Z0\nM5\nM30\n".into(),
        ..base("wincnc-atc", "CAMaster / WinCNC ATC (inch, .tap)", "WinCNC", "CAMaster ATC, ShopSabre ATC", "tap")
    });
    // ---- GRBL family
    const GRBL_MACHINES: &str = "Shapeoko, X-Carve, Longmill, Onefinity, Genmitsu, BobsCNC, MillRight, OpenBuilds, Maslow, Nomad";
    grbl_like(&mut v, "grbl-in", "GRBL (inch, .nc)", "GRBL", GRBL_MACHINES, true, "nc");
    grbl_like(&mut v, "grbl-mm", "GRBL (mm, .nc)", "GRBL", GRBL_MACHINES, false, "nc");
    grbl_like(&mut v, "shapeoko-in", "Carbide 3D Shapeoko / Nomad (inch, .nc)", "GRBL", "Shapeoko 3/4/5 Pro, Nomad", true, "nc");
    grbl_like(&mut v, "shapeoko-mm", "Carbide 3D Shapeoko / Nomad (mm, .nc)", "GRBL", "Shapeoko 3/4/5 Pro, Nomad", false, "nc");
    grbl_like(&mut v, "xcarve-in", "Inventables X-Carve (inch, .gcode)", "GRBL", "X-Carve, X-Carve Pro", true, "gcode");
    grbl_like(&mut v, "xcarve-mm", "Inventables X-Carve (mm, .gcode)", "GRBL", "X-Carve, X-Carve Pro", false, "gcode");
    grbl_like(&mut v, "longmill-mm", "Sienci LongMill / AltMill (mm, .gcode)", "GRBL / grblHAL", "LongMill MK1/MK2, AltMill", false, "gcode");
    grbl_like(&mut v, "longmill-in", "Sienci LongMill / AltMill (inch, .gcode)", "GRBL / grblHAL", "LongMill MK1/MK2, AltMill", true, "gcode");
    grbl_like(&mut v, "onefinity-in", "Onefinity (inch, .ngc)", "Buildbotics", "Onefinity Woodworker, Journeyman, Foreman, Elite (Masso: see Masso)", true, "ngc");
    grbl_like(&mut v, "onefinity-mm", "Onefinity (mm, .ngc)", "Buildbotics", "Onefinity Woodworker, Journeyman, Foreman", false, "ngc");
    grbl_like(&mut v, "genmitsu-mm", "SainSmart Genmitsu (mm, .nc)", "GRBL", "Genmitsu 3018, 4030, PROVerXL", false, "nc");
    grbl_like(&mut v, "openbuilds-mm", "OpenBuilds BlackBox (mm, .gcode)", "GRBL", "OpenBuilds LEAD, WorkBee, Sphinx", false, "gcode");
    grbl_like(&mut v, "millright-in", "MillRight CNC (inch, .nc)", "GRBL", "MillRight Mega V, Carve King, Power Route", true, "nc");
    grbl_like(&mut v, "bobscnc-in", "BobsCNC (inch, .gcode)", "GRBL", "BobsCNC Evolution, Quantum", true, "gcode");
    grbl_like(&mut v, "maslow-mm", "Maslow CNC (mm, .nc)", "GRBL", "Maslow 4", false, "nc");
    grbl_like(&mut v, "grblhal-mm", "grblHAL (mm, .nc)", "grblHAL", "PrintNC, custom builds", false, "nc");
    grbl_like(&mut v, "fluidnc-mm", "FluidNC (mm, .nc)", "FluidNC", "custom ESP32 builds", false, "nc");
    // ---- Mach / LinuxCNC family
    mach_like(&mut v, "mach3-in", "Mach3 / Mach4 (inch, .tap)", "Mach3/Mach4", "generic routers", true, false, "tap", G53);
    mach_like(&mut v, "mach3-mm", "Mach3 / Mach4 (mm, .tap)", "Mach3/Mach4", "generic routers", false, false, "tap", G53);
    mach_like(&mut v, "mach3-atc-in", "Mach3 / Mach4 with tool changer (inch, .tap)", "Mach3/Mach4", "ATC routers", true, true, "tap", G53);
    mach_like(&mut v, "avid-in", "Avid CNC / CNC Router Parts (inch, .tap)", "Mach4", "Avid PRO, Benchtop PRO, CRP", true, true, "tap", G53);
    mach_like(&mut v, "avid-mm", "Avid CNC / CNC Router Parts (mm, .tap)", "Mach4", "Avid PRO, Benchtop PRO", false, true, "tap", G53);
    mach_like(&mut v, "linuxcnc-in", "LinuxCNC (inch, .ngc)", "LinuxCNC", "generic, PrintNC", true, true, "ngc", G53);
    mach_like(&mut v, "linuxcnc-mm", "LinuxCNC (mm, .ngc)", "LinuxCNC", "generic, PrintNC", false, true, "ngc", G53);
    mach_like(&mut v, "pathpilot-in", "Tormach PathPilot (inch, .nc)", "PathPilot (LinuxCNC)", "Tormach PCNC, 24R router", true, true, "nc", "G30");
    mach_like(&mut v, "uccnc-in", "UCCNC (inch, .tap)", "UCCNC", "Stepcraft, generic", true, false, "tap", G53);
    mach_like(&mut v, "uccnc-mm", "UCCNC (mm, .tap)", "UCCNC", "Stepcraft, generic", false, false, "tap", G53);
    mach_like(&mut v, "centroid-in", "Centroid Acorn / Oak (inch, .nc)", "Centroid CNC12", "Acorn, Oak, Allin1DC", true, true, "nc", G53);
    mach_like(&mut v, "centroid-mm", "Centroid Acorn / Oak (mm, .nc)", "Centroid CNC12", "Acorn, Oak", false, true, "nc", G53);
    mach_like(&mut v, "masso-in", "Masso G3 / Touch (inch, .nc)", "Masso", "Masso G3, Touch, Onefinity Elite", true, true, "nc", G53);
    mach_like(&mut v, "masso-mm", "Masso G3 / Touch (mm, .nc)", "Masso", "Masso G3, Touch, Onefinity Elite", false, true, "nc", G53);
    mach_like(&mut v, "acorn-laguna-in", "Laguna IQ / Swift (Centroid, inch)", "Centroid", "Laguna IQ, Swift", true, true, "nc", G53);
    mach_like(&mut v, "eding-mm", "Eding CNC (mm, .nc)", "Eding CNC", "generic", false, true, "nc", G53);
    mach_like(&mut v, "planetcnc-mm", "PlanetCNC (mm, .gcode)", "PlanetCNC", "generic", false, false, "gcode", G53);
    // ---- Industrial G-code
    v.push(PostDef {
        decimals: 4,
        header: "%\nO0001 ({FILENAME})\nG20 G17 G40 G49 G80 G90\n".into(),
        tool_change: "({TOOLNAME})\nM5\nG53 G0 Z0\nT{TOOLNUM} M06\nG54 G43 H{TOOLNUM}\nS{RPM} M03\n".into(),
        path_start: "({PATHNAME})\n".into(),
        footer: "M05\nG53 G0 Z0\nM30\n%\n".into(),
        ..base("haas-in", "Haas (inch, .nc)", "Haas", "Haas VF, TM, Mini Mill", "nc")
    });
    v.push(PostDef {
        inches: false,
        decimals: 3,
        header: "%\nO0001 ({FILENAME})\nG21 G17 G40 G49 G80 G90\n".into(),
        tool_change: "({TOOLNAME})\nM5\nG91 G28 Z0\nG90\nT{TOOLNUM} M06\nG54 G43 H{TOOLNUM}\nS{RPM} M03\n".into(),
        path_start: "({PATHNAME})\n".into(),
        footer: "M05\nG91 G28 Z0\nG90\nM30\n%\n".into(),
        ..base("fanuc-mm", "Fanuc generic (mm, .nc)", "Fanuc", "Fanuc-controlled mills", "nc")
    });
    v.push(PostDef {
        inches: false,
        decimals: 3,
        header: "%\n({FILENAME})\nG90 G21 G17 G40 G49\nG54\nS{RPM} M03\n".into(),
        path_start: "({PATHNAME})\n".into(),
        footer: "M05\nG00 Z30\nM30\n%\n".into(),
        ..base("syntec-mm", "Syntec / Weihong (mm, .nc)", "Syntec, Weihong NcStudio", "Chinese 6090/1325 routers", "nc")
    });
    v.push(PostDef {
        inches: false,
        decimals: 3,
        comment: String::new(),
        header: "G90 G21 G17\nS{RPM} M03\n".into(),
        footer: "M05\nG00 Z50\nM30\n".into(),
        ..base("richauto-mm", "RichAuto DSP A11/B11 (mm, .nc)", "RichAuto DSP", "Axiom, Laguna Swift (DSP), generic 1325 routers", "nc")
    });
    v.push(PostDef {
        decimals: 4,
        comment: "({})".into(),
        header: "({FILENAME})\nG20 G90 G17\nS{RPM} M3\n".into(),
        path_start: "({PATHNAME})\n".into(),
        footer: "M5\nG0 Z1\nM30\n".into(),
        ..base("nextwave-in", "Next Wave Shark / Piranha (inch, .tap)", "Next Wave", "Shark HD, Piranha", "tap")
    });
    v.push(PostDef {
        inches: false,
        decimals: 3,
        arcs: false,
        header: ";{FILENAME}\nG90\nG21\nM3 S{RPM}\nG4 S2\n".into(),
        comment: ";{}".into(),
        path_start: ";{PATHNAME}\n".into(),
        footer: "M5\nG0 Z{SAFEZ}\nM2\n".into(),
        ..base("marlin-mm", "Marlin CNC (mm, .gcode)", "Marlin", "Snapmaker, converted 3D printers", "gcode")
    });
    v.push(PostDef {
        inches: false,
        decimals: 3,
        header: ";{FILENAME}\nG90\nG21\nM3 S{RPM}\n".into(),
        comment: ";{}".into(),
        path_start: ";{PATHNAME}\n".into(),
        footer: "M5\nG0 Z{SAFEZ}\nM2\n".into(),
        ..base("smoothie-mm", "Smoothieware (mm, .gcode)", "Smoothie", "generic", "gcode")
    });
    // ---- ShopBot (own language)
    let shopbot = |id: &str, name: &str, machines: &str, inch: bool, footer: &str| PostDef {
        inches: inch,
        decimals: if inch { 4 } else { 3 },
        arcs: false,
        comment: "'{}".into(),
        header: "'{FILENAME}\nSA\nTR,{RPM}\nC6\nPAUSE 2\n".into(),
        path_start: "'{PATHNAME}\nMS,{FEEDXY},{FEEDZ}\n".into(),
        rapid: "J3,{x},{y},{z}".into(),
        linear: "M3,{x},{y},{z}".into(),
        modal_axes: false,
        footer: footer.into(),
        ..base(id, name, "ShopBot", machines, "sbp")
    };
    v.push(shopbot("shopbot-in", "ShopBot (inch, .sbp)", "ShopBot PRSalpha, PRSstandard, Desktop", true, "JZ,1.0\nC7\nEND\n"));
    v.push(shopbot("shopbot-mm", "ShopBot (mm, .sbp)", "ShopBot", false, "JZ,25\nC7\nEND\n"));
    // ---- Others
    v.push(PostDef {
        inches: false,
        decimals: 3,
        header: ";{FILENAME}\nG90\nG21\nM3 S{RPM}\nG4 S2\n".into(),
        comment: ";{}".into(),
        path_start: ";{PATHNAME}\n".into(),
        footer: "M5\nG0 Z{SAFEZ}\nM0\n".into(),
        ..base("duet-mm", "Duet RepRapFirmware CNC (mm, .gcode)", "RRF", "Duet-based routers", "gcode")
    });
    v.push(PostDef {
        inches: false,
        decimals: 0,
        arcs: false,
        comment: String::new(),
        header: ";;^IN;!MC1;!PZ0,{SAFEZ};V{FEEDXY};\n".into(),
        rapid: "PU{x},{y};".into(),
        linear: "Z{x},{y},{z};".into(),
        modal_axes: false,
        footer: "!MC0;^IN;\n".into(),
        ..base("roland-mm", "Roland RML-1 (mm, .prn)", "RML-1", "Roland MDX, SRM-20", "prn")
    });
    // ---- Industrial ISO controllers (Rust only, 2026-10-09; the 47 above are byte-checked against the C++ app).
    // Plain ISO G-code with each controller's units word, comment style and tool call; G0/G1/G2/G3 with I/J
    // relative to the arc start, which all of them accept. Verify a new controller with File ▸ Export Air-Cut Test.
    struct Iso<'a> {
        /// G20/G21 (Fanuc family) or G70/G71 (OSAI, Fagor, Siemens)
        units: (&'a str, &'a str),
        /// "({})" or ";{}"
        comment: &'a str,
        /// tool call; {T} = tool number
        tool: &'a str,
        /// Z to the top before a tool change and at the end
        lift: &'a str,
        /// "%" lines around the program
        percent: bool,
    }
    let iso = |v: &mut Vec<PostDef>, id: &str, name: &str, controller: &str, machines: &str, inch: bool, ext: &str, k: &Iso| {
        let c = |t: &str| k.comment.replace("{}", t);
        let pc = if k.percent { "%\n" } else { "" };
        let tool = k.tool.replace("{T}", "{TOOLNUM}");
        let mut d = base(id, name, controller, machines, ext);
        d.inches = inch;
        d.decimals = if inch { 4 } else { 3 };
        d.comment = k.comment.into();
        d.header = format!("{pc}{}\nG90 {} G17 G40\n", c("{FILENAME}"), if inch { k.units.0 } else { k.units.1 });
        d.tool_change = format!("{}\nM05\n{}\n{tool}\nS{{RPM}} M03\n", c("{TOOLNAME}"), k.lift);
        d.path_start = format!("{}\n", c("{PATHNAME}"));
        d.footer = format!("M05\n{}\nM30\n{pc}", k.lift);
        v.push(d);
    };
    let fanuc = Iso { units: ("G20", "G21"), comment: "({})", tool: "T{T} M06\nG43 H{T}", lift: "G53 G0 Z0", percent: true };
    let osai = Iso { units: ("G70", "G71"), comment: ";{}", tool: "T{T}.{T} M06", lift: "G79 G0 Z0", percent: false };
    let fagor = Iso { units: ("G70", "G71"), comment: ";{}", tool: "T{T} D{T}\nM06", lift: "G53 G0 Z0", percent: false };
    let siemens = Iso { units: ("G70", "G71"), comment: ";{}", tool: "T{T} M6\nD1", lift: "SUPA G0 Z0 D0", percent: false };
    let plain = Iso { units: ("G20", "G21"), comment: "({})", tool: "T{T} M06", lift: "G53 G0 Z0", percent: false };
    let no_atc = Iso { units: ("G20", "G21"), comment: "({})", tool: "T{T} M06", lift: "G0 Z{SAFEZ}", percent: false };
    iso(&mut v, "fanuc-in", "Fanuc generic (inch, .nc)", "Fanuc / LNC", "Fanuc and LNC (Taiwan) controlled routers", true, "nc", &fanuc);
    iso(&mut v, "osai-mm", "OSAI (mm, .iso)", "OSAI", "Techno, CMS, Biesse ISO, Italian routers with OSAI", false, "iso", &osai);
    iso(&mut v, "osai-in", "OSAI (inch, .iso)", "OSAI", "Techno, CMS and other OSAI routers", true, "iso", &osai);
    iso(&mut v, "fagor-mm", "Fagor 8055 / 8065 / 8070 (mm, .pim)", "Fagor", "Fagor-controlled routers and mills", false, "pim", &fagor);
    iso(&mut v, "fagor-in", "Fagor 8055 / 8065 / 8070 (inch, .pim)", "Fagor", "Fagor-controlled routers and mills", true, "pim", &fagor);
    iso(&mut v, "siemens-mm", "Siemens Sinumerik 828D / 840D (mm, .mpf)", "Siemens Sinumerik", "Sinumerik-controlled routers and machining centres", false, "mpf", &siemens);
    iso(&mut v, "smartcore-in", "CAMaster SmartCore / Yaskawa (inch, .nc)", "Yaskawa SmartCore", "CAMaster Stinger / Panther / Cobra with SmartCore", true, "nc", &plain);
    iso(&mut v, "multicam-in", "MultiCam EZ Control (inch, .nc)", "MultiCam EZ", "MultiCam 1000 / 3000 / 5000 / Apex", true, "nc", &plain);
    iso(&mut v, "multicam-mm", "MultiCam EZ Control (mm, .nc)", "MultiCam EZ", "MultiCam 1000 / 3000 / 5000 / Apex", false, "nc", &plain);
    iso(&mut v, "axyz-in", "AXYZ A2MC (inch, .nc)", "AXYZ A2MC", "AXYZ Infinite, Pacer, Trident", true, "nc", &plain);
    iso(&mut v, "axyz-mm", "AXYZ A2MC (mm, .nc)", "AXYZ A2MC", "AXYZ Infinite, Pacer, Trident", false, "nc", &plain);
    iso(&mut v, "thermwood-in", "Thermwood SuperControl (inch, .nc)", "Thermwood SuperControl", "Thermwood Cut Center, Model 43 / 45 / 53 / 67 / 90", true, "nc", &plain);
    iso(&mut v, "techno-in", "Techno CNC G-code interface (inch, .nc)", "Techno G-code interface", "Techno LC, HD, Goliath", true, "nc", &plain);
    iso(&mut v, "techno-mm", "Techno CNC G-code interface (mm, .nc)", "Techno G-code interface", "Techno LC, HD, Goliath", false, "nc", &plain);
    iso(&mut v, "laguna-touch-in", "Laguna SmartShop / Touch (inch, .nc)", "Laguna Touch", "Laguna SmartShop II / M, Elite", true, "nc", &plain);
    iso(&mut v, "starcnc-in", "Larken StarCNC (inch, .nc)", "StarCNC", "Larken routers", true, "nc", &no_atc);
    iso(&mut v, "delta-in", "Delta CNC (Legacy) (inch, .nc)", "Delta CNC", "Legacy Maverick, Explorer, Phoenix", true, "nc", &no_atc);
    iso(&mut v, "din-mm", "DIN 66025 G-code (mm, .nc): cncGraF, WinPC-NC, Beamicon, Mecanumeric", "DIN 66025", "isel, Sorotec, Mecanumeric, cncGraF, WinPC-NC, Beamicon", false, "nc", &no_atc);
    iso(&mut v, "format4-mm", "Felder Format-4 F4Integrate (ISO import, mm, .iso)", "F4Integrate", "Felder Format-4 profit", false, "iso", &osai);
    v
}

/// User post as JSON (same keys as the C++ writer). The C++ writer prints `maxArcSweepDeg` as `180`, this one as
/// `180.0`; both readers accept either.
pub fn post_to_json(d: &PostDef) -> String {
    serde_json::to_string_pretty(d).expect("PostDef serializes") + "\n"
}

/// User-defined post (same fields as [`PostDef`]; missing keys keep their defaults, unknown keys are ignored).
pub fn post_from_json(json: &str) -> Result<PostDef, String> {
    let mut d: PostDef = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if d.id.is_empty() {
        d.id = "custom".into();
    }
    if d.name.is_empty() {
        d.name = d.id.clone();
    }
    Ok(d)
}

/// C atoi: leading whitespace, optional sign, digits; 0 when none (saturating instead of UB on overflow)
fn atoi(s: &str) -> i32 {
    let t = s.trim_start_matches(|c: char| c.is_ascii() && c_space(c as u8));
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let v = t.bytes().take_while(u8::is_ascii_digit).fold(0i64, |a, c| (a * 10 + i64::from(c - b'0')).min(1 << 40));
    (if neg { -v } else { v }).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Vectric .pp post-processor file -> PostDef (header/footer/move blocks and [VAR] formats are translated;
/// unsupported sections are ignored). Good enough for the common milling posts; verify the output on your controller.
///
/// Format: free text with "+" comment lines, settings like  POST_NAME = "name"  FILE_EXTENSION = "tap"
/// UNITS = "INCHES", variables  VAR X_POSITION = [X|A|X|1.4]  (name, A/C incremental, output prefix, digits
/// before.after), and blocks  begin HEADER / RAPID_MOVE / FEED_MOVE / CW_ARC_MOVE / CCW_ARC_MOVE / FOOTER /
/// TOOLCHANGE / NEW_SEGMENT ... end, whose lines are quoted strings using [VAR] substitutions.
pub fn post_from_vectric_pp(text: &str) -> Result<PostDef, String> {
    struct Var {
        token: String,
        prefix: String,
        always: bool,
    }
    let mut d = PostDef { comment: String::new(), ..Default::default() };
    let mut vars: Vec<Var> = Vec::new();
    let mut blocks: HashMap<String, Vec<String>> = HashMap::new();
    let mut cur = String::new();
    let unquote = |v: &str| -> String {
        match (v.find('"'), v.rfind('"')) {
            (Some(a), Some(b)) if b > a => v[a + 1..b].to_string(),
            _ => v.to_string(),
        }
    };
    let rtrim = |v: &str| v.trim_end_matches(|c: char| c.is_ascii() && c_space(c as u8)).to_string();
    for raw in getlines(text) {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let Some(ns) = line.find(|c| c != ' ' && c != '\t') else { continue };
        let t = &line[ns..];
        if t.starts_with('+') {
            continue; // comment
        }
        let ut = t.to_ascii_uppercase();
        if !cur.is_empty() {
            if ut == "END" {
                cur.clear();
            } else if t.starts_with('"') {
                blocks.entry(cur.clone()).or_default().push(unquote(t));
            }
            continue;
        }
        if let Some(rest) = ut.strip_prefix("BEGIN ") {
            cur = rtrim(rest);
            continue;
        }
        let Some(eq) = t.find('=') else { continue };
        let key = rtrim(&ut[..eq]);
        let val = &t[eq + 1..];
        match key.as_str() {
            "POST_NAME" => d.name = unquote(val),
            "FILE_EXTENSION" => d.extension = unquote(val),
            "UNITS" => d.inches = unquote(val).to_ascii_uppercase().starts_with("INCH"),
            "LINE_ENDING" => {
                let le = unquote(val);
                d.crlf = le.contains("13") || le.contains("\\r");
            }
            "SUBSTITUTE" => d.substitute = unquote(val),
            "LINE_NUMBER_START" => d.line_number_start = atoi(val),
            "LINE_NUMBER_INCREMENT" => d.line_number_step = atoi(val),
            // absolute centres: not supported, use lines
            "ARC_CENTRE_I_J_ABSOLUTE" | "ARC_CENTER_I_J_ABSOLUTE" => {
                if unquote(val).eq_ignore_ascii_case("YES") {
                    d.arcs = false;
                }
            }
            _ if key.starts_with("VAR ") => {
                // VAR X_POSITION = [X|A|X|1.4]  -> token X, A always / C when changed, output prefix X, 1 digit before, 4 after
                let (Some(lb), Some(rb)) = (val.find('['), val.find(']')) else { continue };
                let spec = if rb > lb { &val[lb + 1..rb] } else { &val[lb + 1..] }; // C++ substr(lb+1, rb-lb-1) wraps to "the rest"
                let parts: Vec<&str> = spec.split('|').collect();
                let mut vr = Var { token: parts[0].to_string(), prefix: String::new(), always: false };
                let mut decimals = 4;
                if parts.len() >= 2 {
                    vr.always = parts[1].eq_ignore_ascii_case("A");
                }
                if parts.len() >= 3 {
                    vr.prefix = parts[2].to_string();
                }
                if parts.len() >= 4 {
                    decimals = parts[3].find('.').map_or(0, |dot| atoi(&parts[3][dot + 1..]));
                }
                let name = rtrim(&key[4..]);
                if name == "X_POSITION" {
                    d.decimals = decimals;
                }
                if name == "FEED_RATE" {
                    d.feed_decimals = decimals;
                }
                vars.push(vr);
            }
            _ => {}
        }
    }
    if d.name.is_empty() {
        return Err("no POST_NAME found: not a Vectric post file".into());
    }
    d.id = "pp-".to_string() + &d.name.bytes().map(|c| if c.is_ascii_alphanumeric() { c as char } else { '-' }).collect::<String>();
    d.controller = "Vectric .pp".into();
    d.machines = d.name.clone();
    // translate [VAR] tokens into our placeholders. Axis vars: A (always) -> prefix + bare number, C (changed) -> modal word.
    let xlate = |line: &str| -> String {
        let mut s = line.to_string();
        for v in &vars {
            let up = v.token.to_ascii_uppercase();
            let p = &v.prefix;
            let rep = match up.as_str() {
                "X" | "Y" | "Z" | "I" | "J" => {
                    if v.always {
                        format!("{p}{{{}}}", up.to_ascii_lowercase())
                    } else {
                        format!("{{{up}}}")
                    }
                }
                "F" | "FC" | "FP" => {
                    if v.always {
                        format!("{p}{{f}}")
                    } else {
                        "{F}".into()
                    }
                }
                "S" => format!("{p}{{RPM}}"),
                "T" => format!("{p}{{TOOLNUM}}"),
                "N" => format!("{p}{{N}}"),
                "SAFEZ" | "ZH" => format!("{p}{{SAFEZ}}"),
                "XH" | "YH" => format!("{p}0"),
                "XMIN" | "XMAX" | "YMIN" | "YMAX" | "ZMIN" | "ZMAX" => format!("{p}{{{up}}}"),
                "XLENGTH" | "YLENGTH" | "ZLENGTH" => String::new(),
                _ => continue,
            };
            s = s.replace(&format!("[{}]", v.token), &rep);
        }
        const FIXED: &[(&str, &str)] = &[
            ("[X]", "{X}"),
            ("[Y]", "{Y}"),
            ("[Z]", "{Z}"),
            ("[I]", "{I}"),
            ("[J]", "{J}"),
            ("[F]", "{F}"),
            ("[FC]", "{F}"),
            ("[FP]", "{F}"),
            ("[S]", "S{RPM}"),
            ("[T]", "T{TOOLNUM}"),
            ("[N]", ""),
            ("[SAFEZ]", "Z{SAFEZ}"),
            ("[ZH]", "Z{SAFEZ}"),
            ("[XH]", "X0"),
            ("[YH]", "Y0"),
            ("[TOOL_NAME]", "{TOOLNAME}"),
            ("[TOOLS_USED]", "{TOOLNAME}"),
            ("[TOOLPATH_NAME]", "{PATHNAME}"),
            ("[TP_PATHNAME]", "{PATHNAME}"),
            ("[TOOLPATH_NOTES]", ""),
            ("[TOOL_NOTES]", ""),
            ("[TP_FILENAME]", "{FILENAME}"),
            ("[FILE_NAME]", "{FILENAME}"),
            ("[DATE]", "{DATE}"),
            ("[TIME]", "{TIME}"),
            ("[FEED_RATE]", "{FEEDXY}"),
            ("[PLUNGE_RATE]", "{FEEDZ}"),
            ("[XMIN]", "{XMIN}"),
            ("[XMAX]", "{XMAX}"),
            ("[YMIN]", "{YMIN}"),
            ("[YMAX]", "{YMAX}"),
            ("[ZMIN]", "{ZMIN}"),
            ("[ZMAX]", "{ZMAX}"),
            ("[XLENGTH]", "{MATERIAL}"),
            ("[YLENGTH]", ""),
            ("[ZLENGTH]", ""),
            ("[Z_ORIGIN]", "{ZZERO}"),
            ("[TOOLPATHS_OUTPUT_DIRECTORY]", ""),
            ("[PRODUCT]", "GCarve"),
            ("[PRODUCT_VERSION]", ""),
        ];
        for (from, to) in FIXED {
            s = s.replace(from, to);
        }
        // any remaining [NAME] we don't know: drop it
        while let Some(a) = s.find('[') {
            let Some(b) = s[a..].find(']') else { break };
            s.replace_range(a..=a + b, "");
        }
        s
    };
    let join = |key: &str| -> String { blocks.get(key).map_or(String::new(), |ls| ls.iter().map(|l| xlate(l) + "\n").collect()) };
    d.header = join("HEADER");
    d.footer = join("FOOTER");
    d.path_start = join("NEW_SEGMENT");
    d.tool_change = join("TOOLCHANGE");
    d.feed_change = join("FEED_RATE_CHANGE").trim_end_matches('\n').to_string();
    // multi-line blocks keep their lines; trailing newline dropped
    let move_tpl = |key: &str, def: &str| -> String {
        match blocks.get(key) {
            Some(ls) if !ls.is_empty() => ls.iter().map(|l| trim_spaces(&xlate(l))).collect::<Vec<_>>().join("\n"),
            _ => def.to_string(),
        }
    };
    d.rapid = move_tpl("RAPID_MOVE", "G0 {X} {Y} {Z}");
    d.linear = move_tpl("FEED_MOVE", "G1 {X} {Y} {Z} {F}");
    d.plunge = move_tpl("PLUNGE_MOVE", "");
    d.retract = move_tpl("RETRACT_MOVE", "");
    d.first_linear = move_tpl("FIRST_FEED_MOVE", "");
    d.arc_cw = move_tpl("CW_ARC_MOVE", "");
    d.arc_ccw = move_tpl("CCW_ARC_MOVE", "");
    if d.arc_cw.is_empty() || d.arc_ccw.is_empty() || !d.arcs {
        d.arcs = false;
        d.arc_cw = d.linear.clone();
        d.arc_ccw = d.linear.clone();
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getlines_matches_std_getline() {
        assert_eq!(getlines("").count(), 0);
        assert_eq!(getlines("\n").collect::<Vec<_>>(), [""]);
        assert_eq!(getlines("a\n\nb").collect::<Vec<_>>(), ["a", "", "b"]);
        assert_eq!(getlines("a\r\nb\n").collect::<Vec<_>>(), ["a\r", "b"]);
    }

    #[test]
    fn spindle_words() {
        assert!(spindle_off("M5") && spindle_off("g0 z1 m05;") && spindle_off("C7") && spindle_off("SO, 1, 0") && spindle_off("!MC0;^IN;"));
        assert!(!spindle_off("M50") && !spindle_off("XM5") && !spindle_off("SO,1,1") && !spindle_off("SO,,0"));
        assert_eq!(spindle_safe_footer("G0 Z1\nM30\n"), "G0 Z1\nM5\nM30\n");
        assert_eq!(spindle_safe_footer(""), "M5\n");
        assert_eq!(spindle_safe_footer("  m02"), "M5\n  m02\n");
        assert_eq!(spindle_safe_footer("M20"), "M20\nM5\n");
    }

    #[test]
    fn line_numbers_and_atoi() {
        let d = PostDef { line_number_start: 100, line_number_step: 5, ..Default::default() };
        assert_eq!(number_lines("N{N}a\nN{N}b{N}", &d), "N100a\nN105b110");
        assert_eq!(atoi(" 100"), 100);
        assert_eq!(atoi("-3x"), -3);
        assert_eq!(atoi("x"), 0);
    }
}
