//! Setup sheet: a printable job sheet for the machine operator (one page per sheet of material), as one HTML string.
//! Plain data in, HTML out (C++ `core/setupsheet.cpp`, same output byte for byte: tests/setupsheet_golden.rs).
//! `pipeline::setup_sheet` collects the data from a document.

use crate::document::{Material, Tool, ToolType};
use crate::job::{Job, MoveType};
use crate::order::estimate_minutes_with;
use crate::wincnc::num;

#[derive(Clone, Debug, Default)]
pub struct SetupPage {
    pub board: Material,
    /// this sheet's jobs in run order (export naming, origin shift applied)
    pub jobs: Vec<Job>,
    /// per job: the G-code file it is in (export dialog names)
    pub files: Vec<String>,
    /// per job: tool-changer slot, 0 = manual change (empty = no slot column)
    pub tool_slots: Vec<i32>,
    /// CAM warnings + safety check findings for this sheet
    pub warnings: Vec<String>,
    /// top-down picture as a data: URI ("" = none)
    pub image: String,
    /// where the picture sits on the board: x0, y0, w, h (inches, y up)
    pub image_rect: [f64; 4],
}

#[derive(Clone, Debug, Default)]
pub struct SetupSheet {
    pub project: String,
    pub date: String,
    pub post: String,
    /// project units: lengths in mm and feeds in mm/min, else inches
    pub mm: bool,
    /// tools as they cut (feeds per material), matched to Job::tool by name
    pub tools: Vec<Tool>,
    pub pages: Vec<SetupPage>,
    /// for the times (default = the C++ estimate: rapids 400 in/min, no acceleration)
    pub motion: crate::order::Motion,
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            _ => o.push(c),
        }
    }
    o
}

const CSS: &str = concat!(
    ":root{--ink:#1b1d21;--dim:#5b6270;--line:#c9ced6;--accent:#a8431a;--wood:#efe4cf}\n",
    "*{box-sizing:border-box}\n",
    "body{margin:0;background:#e9ebee;color:var(--ink);font:13px/1.4 -apple-system,\"Segoe UI\",Roboto,Helvetica,Arial,sans-serif}\n",
    ".bar{max-width:1000px;margin:16px auto 0;padding:0 16px;display:flex;gap:12px;align-items:center;color:var(--dim)}\n",
    ".bar button{font:inherit;padding:6px 16px;border:0;background:var(--ink);color:#fff;border-radius:4px;cursor:pointer}\n",
    ".page{max-width:1000px;margin:16px auto;background:#fff;padding:28px 32px;box-shadow:0 1px 3px rgba(0,0,0,.15)}\n",
    "header{display:flex;justify-content:space-between;align-items:flex-end;gap:16px;border-bottom:2px solid var(--ink);padding-bottom:8px;margin-bottom:16px}\n",
    "h1{font-size:22px;margin:0}\n",
    "header p{margin:2px 0 0;color:var(--dim)}\n",
    ".sheetno{font-weight:600;white-space:nowrap}\n",
    ".top{display:flex;gap:24px;align-items:flex-start;margin-bottom:18px}\n",
    ".facts{border-collapse:collapse;flex:0 0 48%}\n",
    ".facts th{text-align:left;color:var(--dim);font-weight:600;padding:4px 12px 4px 0;vertical-align:top;white-space:nowrap}\n",
    ".facts td{padding:4px 0}\n",
    ".facts svg{vertical-align:middle;margin-right:8px}\n",
    ".pic{flex:1;margin:0;min-width:0}\n",
    ".pic svg{width:100%;height:auto;max-height:320px;display:block}\n",
    ".run{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}\n",
    ".run th{text-align:left;font-size:11px;text-transform:uppercase;letter-spacing:.04em;color:var(--dim);border-bottom:1.5px solid var(--ink);padding:6px}\n",
    ".run td{border-bottom:1px solid var(--line);padding:6px;vertical-align:top}\n",
    ".run .n{text-align:right;white-space:nowrap}\n",
    ".run .file{font-family:ui-monospace,Menlo,Consolas,monospace;font-size:12px;word-break:break-all}\n",
    ".run small{color:var(--dim)}\n",
    ".run tfoot td{border:0;font-weight:700;padding-top:8px}\n",
    "h2{font-size:12px;text-transform:uppercase;letter-spacing:.05em;margin:20px 0 6px}\n",
    ".warn{margin:0;padding-left:18px;color:var(--accent)}\n",
    ".ok{margin:0;color:var(--dim)}\n",
    ".notes{min-height:96px;border:1px solid var(--line);border-radius:4px;padding:8px;white-space:pre-wrap}\n",
    ".notes:empty::before{content:\"Click to type notes before printing\";color:#9aa1ad}\n",
    "@page{margin:12mm}\n",
    "@media print{body{background:#fff}.noprint{display:none}.page{box-shadow:none;margin:0;padding:0;max-width:none;break-after:page}",
    ".page:last-child{break-after:auto}.run tr{break-inside:avoid}.notes:empty::before{content:\"\"}}\n",
    "@media (max-width:700px){.page{padding:16px}.top{flex-direction:column}.facts{flex:none}}\n",
);

/// datum position on the board, y up
const FX: [f64; 5] = [0.0, 1.0, 0.0, 1.0, 0.5];
const FY: [f64; 5] = [0.0, 0.0, 1.0, 1.0, 0.5];
const ORIGIN: [&str; 5] = ["Bottom-left corner", "Bottom-right corner", "Top-left corner", "Top-right corner", "Centre of the board"];

struct Fmt {
    mm: bool,
}
impl Fmt {
    fn len(&self, inch: f64) -> String {
        if self.mm { num(inch * 25.4, 1) + " mm" } else { num(inch, 3) + " in" }
    }
    fn bare(&self, inch: f64) -> String {
        if self.mm { num(inch * 25.4, 1) } else { num(inch, 3) }
    }
    fn feed(&self, ipm: f64) -> String {
        if self.mm { num(ipm * 25.4, 0) } else { num(ipm, 0) }
    }
}

fn origin(m: &Material) -> usize {
    m.origin.clamp(0, 4) as usize
}

/// little board with the five datum points, the chosen one filled (like OriginPicker)
fn datum_svg(m: &Material) -> String {
    let aspect = (if m.h > 0.0 { m.w / m.h } else { 1.5 }).clamp(0.5, 3.0);
    let (w, h) = (64.0, (64.0 / aspect).round());
    let org = origin(m);
    let mut s = format!(
        "<svg width=\"{0}\" height=\"{1}\" viewBox=\"-4 -4 {0} {1}\" aria-hidden=\"true\"><rect width=\"{2}\" height=\"{3}\" rx=\"2\" fill=\"#efe4cf\" stroke=\"#8a7a5a\"/>",
        num(w + 8.0, 0),
        num(h + 8.0, 0),
        num(w, 0),
        num(h, 0)
    );
    for i in 0..5 {
        let (cx, cy) = (num(FX[i] * w, 1), num((1.0 - FY[i]) * h, 1));
        s += &if i == org { format!("<circle cx=\"{cx}\" cy=\"{cy}\" r=\"4\" fill=\"#a8431a\"/>") } else { format!("<circle cx=\"{cx}\" cy=\"{cy}\" r=\"2\" fill=\"#8a7a5a\"/>") };
    }
    s + "</svg>"
}

/// side view of the board on the bed, the Z zero line marked
fn zero_svg(m: &Material) -> String {
    let y = if m.z_bottom { "22" } else { "8" };
    format!(
        "<svg width=\"72\" height=\"30\" viewBox=\"0 0 72 30\" aria-hidden=\"true\"><rect x=\"8\" y=\"8\" width=\"56\" height=\"14\" fill=\"#efe4cf\" stroke=\"#8a7a5a\"/><line x1=\"0\" y1=\"26\" x2=\"72\" y2=\"26\" stroke=\"#5b6270\" stroke-width=\"2\"/><line x1=\"0\" y1=\"{y}\" x2=\"72\" y2=\"{y}\" stroke=\"#a8431a\" stroke-width=\"2\" stroke-dasharray=\"4 3\"/></svg>"
    )
}

/// the board from above: the picture (shaded simulation) in place, the XY datum marked
fn picture_svg(p: &SetupPage) -> String {
    let m = &p.board;
    let (pad, r) = (m.w.max(m.h) * 0.03, m.w.max(m.h) * 0.012);
    let org = origin(m);
    let mut s = format!(
        "<svg viewBox=\"{} {} {} {}\" role=\"img\" aria-label=\"Top view of the board\"><rect width=\"{}\" height=\"{}\" fill=\"#fff\" stroke=\"#8a7a5a\" stroke-width=\"1\" vector-effect=\"non-scaling-stroke\"/>",
        num(-pad, 4),
        num(-pad, 4),
        num(m.w + 2.0 * pad, 4),
        num(m.h + 2.0 * pad, 4),
        num(m.w, 4),
        num(m.h, 4)
    );
    if !p.image.is_empty() {
        let ir = &p.image_rect;
        s += &format!("<image href=\"{}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\"/>", p.image, num(ir[0], 4), num(m.h - ir[1] - ir[3], 4), num(ir[2], 4), num(ir[3], 4));
    }
    s + &format!("<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#a8431a\"/></svg>", num(FX[org] * m.w, 4), num((1.0 - FY[org]) * m.h, 4), num(r, 4))
}

/// "<0.1 min", "2.5 min", "12 min", "1 h 05 min"
pub fn format_minutes(minutes: f64) -> String {
    if minutes < 0.05 {
        return "&lt;0.1 min".into();
    }
    if minutes < 9.95 {
        return num(minutes, 1) + " min";
    }
    let r = minutes.round() as i64;
    if r < 60 {
        return format!("{r} min");
    }
    format!("{} h {:02} min", r / 60, r % 60)
}

pub fn setup_sheet_html(s: &SetupSheet) -> String {
    let f = Fmt { mm: s.mm };
    let (u, fu) = if s.mm { ("mm", "mm/min") } else { ("in", "in/min") };
    let mut o = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{} &middot; setup sheet</title>\n<style>\n{CSS}</style>\n</head>\n<body>\n<div class=\"bar noprint\"><button onclick=\"window.print()\">Print</button><span>Type notes on each page, then print or save as PDF.</span></div>\n",
        esc(&s.project)
    );
    let npages = s.pages.len();
    for (pi, p) in s.pages.iter().enumerate() {
        let m = &p.board;
        let has_slots = !p.tool_slots.is_empty();
        // rows first: the total goes in the facts table
        let mut rows = String::new();
        let mut total = 0.0;
        let mut warnings = p.warnings.clone();
        for (ji, j) in p.jobs.iter().enumerate() {
            for n in &j.notes {
                if !warnings.contains(n) {
                    warnings.push(n.clone());
                }
            }
            // depth range: shallowest level pass (ramps and plunge entries don't count) to the deepest point
            let (mut mins, mut any_min, mut level_min, mut dmax) = (0.0, 1e18f64, 1e18f64, -1.0f64);
            let top = if m.z_bottom { m.t } else { 0.0 };
            let mut names: Vec<&str> = Vec::new();
            for tp in &j.toolpaths {
                mins += estimate_minutes_with(tp, tp.feed_xy, tp.feed_z, s.motion);
                if !names.contains(&tp.name.as_str()) {
                    names.push(&tp.name);
                }
                let mut z = 0.0;
                for mv in &tp.moves {
                    let pz = z;
                    if let Some(nz) = mv.z {
                        z = nz;
                    }
                    let d = top - z;
                    if mv.kind == MoveType::Rapid || d <= 1e-6 {
                        continue;
                    }
                    any_min = any_min.min(d);
                    dmax = dmax.max(d);
                    if (z - pz).abs() < 1e-9 {
                        level_min = level_min.min(d);
                    }
                }
            }
            let dmin = if level_min < 1e18 { level_min } else { any_min };
            total += mins;
            let depth = if dmax < 0.0 {
                "&mdash;".to_string()
            } else if num(dmin, 3) == num(dmax, 3) {
                f.bare(dmax)
            } else {
                format!("{}&ndash;{}", f.bare(dmin), f.bare(dmax))
            };
            let paths = names.iter().map(|n| esc(n)).collect::<Vec<_>>().join(", ");
            let detail = s.tools.iter().find(|t| t.name == j.tool).map(|t| {
                let kind = match t.kind {
                    ToolType::EndMill => "End mill",
                    ToolType::VBit => "V-bit",
                    ToolType::BallNose => "Ball nose",
                    ToolType::RoundOver => "Round-over",
                };
                let mut d = format!("{kind} &middot; &Oslash; {}", f.len(t.diameter));
                if t.kind == ToolType::VBit {
                    d += &format!(" &middot; {}&deg;", num(t.angle, 0));
                }
                d + &format!(" &middot; {} {}", t.flutes, if t.flutes == 1 { "flute" } else { "flutes" })
            });
            let (fxy, fz) = j.toolpaths.first().map(|t| (t.feed_xy, t.feed_z)).unwrap_or((0.0, 0.0));
            let slot = if has_slots {
                format!("<td class=\"n\">{}</td>", p.tool_slots.get(ji).filter(|&&s| s > 0).map(|s| format!("T{s}")).unwrap_or_else(|| "&mdash;".into()))
            } else {
                String::new()
            };
            rows += &format!(
                "<tr><td class=\"n\">{}</td><td class=\"file\">{}</td><td><b>{}</b>{}</td>{slot}<td>{paths}</td><td class=\"n\">{depth}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>\n",
                ji + 1,
                esc(p.files.get(ji).unwrap_or(&j.filename)),
                esc(&j.tool),
                detail.map(|d| format!("<br><small>{d}</small>")).unwrap_or_default(),
                f.feed(fxy),
                f.feed(fz),
                j.rpm,
                format_minutes(mins)
            );
        }
        let cols = if has_slots { 10 } else { 9 };
        o += &format!("<section class=\"page\">\n<header><div><h1>{}</h1><p>Setup sheet &middot; {}</p></div>", esc(&s.project), esc(&s.date));
        if npages > 1 {
            o += &format!("<div class=\"sheetno\">Sheet {} of {npages}</div>", pi + 1);
        }
        o += "</header>\n<div class=\"top\">\n<table class=\"facts\"><tbody>\n";
        o += &format!("<tr><th>Material</th><td>{}</td></tr>\n", esc(&m.kind));
        o += &format!("<tr><th>Board</th><td>{} &times; {} &times; {} {u} <small>(W &times; H &times; T)</small></td></tr>\n", f.bare(m.w), f.bare(m.h), f.bare(m.t));
        o += &format!("<tr><th>XY zero</th><td>{}{}</td></tr>\n", datum_svg(m), ORIGIN[origin(m)]);
        o += &format!("<tr><th>Z zero</th><td>{}{}</td></tr>\n", zero_svg(m), if m.z_bottom { "Machine bed (spoilboard): measure the board" } else { "Top of the material" });
        o += &format!("<tr><th>Safe Z</th><td>{}</td></tr>\n", f.len(m.safe_z));
        o += &format!("<tr><th>Post</th><td>{}</td></tr>\n", esc(&s.post));
        o += &format!("<tr><th>Run time</th><td><b>{}</b> <small>(estimate)</small></td></tr>\n", format_minutes(total));
        o += &format!("</tbody></table>\n<figure class=\"pic\">{}</figure>\n</div>\n", picture_svg(p));
        o += &format!(
            "<table class=\"run\"><thead><tr><th class=\"n\">#</th><th>File</th><th>Tool</th>{}<th>Toolpaths</th><th class=\"n\">Depth {u}</th><th class=\"n\">Feed {fu}</th><th class=\"n\">Plunge {fu}</th><th class=\"n\">RPM</th><th class=\"n\">Time</th></tr></thead>\n<tbody>\n",
            if has_slots { "<th class=\"n\">Slot</th>" } else { "" }
        );
        if rows.is_empty() {
            o += &format!("<tr><td colspan=\"{cols}\">No toolpaths on this sheet.</td></tr>\n");
        } else {
            o += &rows;
        }
        o += &format!("</tbody>\n<tfoot><tr><td colspan=\"{}\">Total</td><td class=\"n\">{}</td></tr></tfoot></table>\n", cols - 1, format_minutes(total));
        o += "<h2>Warnings</h2>\n";
        if warnings.is_empty() {
            o += "<p class=\"ok\">None: the safety check found no problems.</p>\n";
        } else {
            o += "<ul class=\"warn\">\n";
            for w in &warnings {
                o += &format!("<li>{}</li>\n", esc(w));
            }
            o += "</ul>\n";
        }
        o += "<h2>Notes</h2>\n<div class=\"notes\" contenteditable=\"true\"></div>\n</section>\n";
    }
    o + "</body>\n</html>\n"
}

/// Standard base64 (the apps embed the picture as a PNG data URI).
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let v = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        o.push(T[(v >> 18) as usize & 63] as char);
        o.push(T[(v >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(v >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[v as usize & 63] as char } else { '=' });
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minutes_and_base64() {
        assert_eq!(format_minutes(0.01), "&lt;0.1 min");
        assert_eq!(format_minutes(2.46), "2.5 min");
        assert_eq!(format_minutes(12.4), "12 min");
        assert_eq!(format_minutes(65.0), "1 h 05 min");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }
}
