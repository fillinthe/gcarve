//! Cut report and part labels for nested sheets (File › Cut Report and Labels…): per sheet a numbered picture of the
//! parts, a table of their sizes, how much of the board they use and the biggest offcut left; then one label per
//! part to print and stick on. Plain data in, HTML out (no I/O), like the setup sheet.

use crate::document::Document;
use crate::geom::{bounds, flatten, point_in_poly, Pt, FLATTEN_TOL};

/// One part: an outermost closed vector of a sheet (holes and pockets inside it belong to it).
#[derive(Clone, Debug)]
pub struct Part {
    /// number on its sheet, from 1, in reading order (top row first, left to right)
    pub n: usize,
    pub sheet: usize,
    pub layer: String,
    pub w: f64,
    pub h: f64,
    pub area: f64,
    /// flattened outline, sheet coordinates
    pub outline: Vec<Pt>,
}

fn area(p: &[Pt]) -> f64 {
    (0..p.len()).map(|i| { let (a, b) = (p[i], p[(i + 1) % p.len()]); a.x * b.y - b.x * a.y }).sum::<f64>().abs() / 2.0
}

/// The parts of every sheet: closed vectors of a square inch or more on layers that are shown and not keep-out, not
/// text, and not inside another such vector.
pub fn parts(doc: &Document) -> Vec<Part> {
    let mut out = Vec::new();
    for sheet in 0..doc.sheets.len().max(1) {
        let cand: Vec<(Vec<Pt>, &crate::document::Shape)> = doc
            .shapes
            .iter()
            .filter(|s| s.sheet == sheet as i32 && s.c.closed && s.text == 0)
            .filter(|s| doc.layers.get(s.layer as usize).is_none_or(|l| l.visible && !l.keep_out))
            .map(|s| (flatten(&s.c, FLATTEN_TOL), s))
            .filter(|(p, _)| p.len() > 2 && area(p) >= 1.0)
            .collect();
        let inside = |q: Pt, p: &[Pt]| point_in_poly(q, p.len(), |i| (p[i].x, p[i].y));
        let mut here: Vec<Part> = cand
            .iter()
            .enumerate()
            .filter(|(i, (p, _))| !cand.iter().enumerate().any(|(j, (o, _))| j != *i && area(o) > area(p) && inside(p[0], o)))
            .map(|(_, (p, s))| {
                let b = bounds(&s.c);
                Part { n: 0, sheet, layer: doc.layers.get(s.layer as usize).map(|l| l.name.clone()).unwrap_or_default(), w: b.w(), h: b.h(), area: area(p), outline: p.clone() }
            })
            .collect();
        // reading order: rows from the top (parts whose middles are within half a part's height share a row)
        let mid = |p: &Part| { let (mut x, mut y) = (0.0, 0.0); for q in &p.outline { x += q.x; y += q.y; } (x / p.outline.len() as f64, y / p.outline.len() as f64) };
        here.sort_by(|a, b| { let (ma, mb) = (mid(a), mid(b)); if (ma.1 - mb.1).abs() > a.h.min(b.h) / 2.0 { mb.1.total_cmp(&ma.1) } else { ma.0.total_cmp(&mb.0) } });
        for (i, p) in here.iter_mut().enumerate() {
            p.n = i + 1;
        }
        out.extend(here);
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The report as one HTML page. `len` writes a length in the project's unit (the app's formatter).
pub fn html(doc: &Document, project: &str, len: &dyn Fn(f64) -> String) -> String {
    let all = parts(doc);
    let mut s = String::from("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>");
    s += &esc(project);
    s += " cut report</title><style>body{font:14px/1.4 system-ui,sans-serif;color:#111;margin:24px}h1{font-size:20px;margin:0 0 4px}h2{font-size:16px;margin:24px 0 6px}table{border-collapse:collapse;margin:8px 0}td,th{border:1px solid #bbb;padding:3px 10px;text-align:left}th{background:#eee}svg{max-width:100%;height:auto;border:1px solid #888;background:#f6f1e6}.facts{color:#444}section{page-break-after:always}.labels{display:grid;grid-template-columns:repeat(3,1fr);gap:8px}.label{border:1px dashed #888;padding:10px;break-inside:avoid}.label b{font-size:22px}.label small{color:#444;display:block}@media print{body{margin:10mm}}</style></head><body>\n";
    for sheet in 0..doc.sheets.len().max(1) {
        let m = doc.sheets.get(sheet).unwrap_or(&doc.material);
        let here: Vec<&Part> = all.iter().filter(|p| p.sheet == sheet).collect();
        let used: f64 = here.iter().map(|p| p.area).sum();
        let name = doc.sheet_name(sheet);
        s += &format!("<section><h1>{} · {}</h1><div class=\"facts\">{} × {} × {} · {} part(s) · {:.0}% of the board used", esc(project), esc(&name), len(m.w), len(m.h), len(m.t), here.len(), 100.0 * used / (m.w * m.h).max(1e-9));
        // the biggest clear rectangle beside or above everything: the offcut worth keeping
        let (mx, my) = here.iter().flat_map(|p| &p.outline).fold((0.0f64, 0.0f64), |a, q| (a.0.max(q.x), a.1.max(q.y)));
        let (right, top) = ((m.w - mx).max(0.0) * m.h, (m.h - my).max(0.0) * m.w);
        if !here.is_empty() && right.max(top) > 36.0 {
            let (a, b) = if right >= top { (m.w - mx, m.h) } else { (m.w, m.h - my) };
            s += &format!(" · largest offcut about {} × {}", len(a), len(b));
        }
        s += "</div>\n";
        let k = 900.0 / m.w.max(1e-9);
        s += &format!("<svg viewBox=\"0 0 {:.1} {:.1}\" width=\"{:.0}\" height=\"{:.0}\" role=\"img\" aria-label=\"The parts on {}\">", m.w * k, m.h * k, m.w * k, m.h * k, esc(&name));
        for p in &here {
            let pts: Vec<String> = p.outline.iter().map(|q| format!("{:.1},{:.1}", q.x * k, (m.h - q.y) * k)).collect();
            let b = p.outline.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, q| [b[0].min(q.x), b[1].min(q.y), b[2].max(q.x), b[3].max(q.y)]);
            let size = ((b[2] - b[0]).min(b[3] - b[1]) * k * 0.5).clamp(9.0, 34.0);
            s += &format!("<polygon points=\"{}\" fill=\"#fff\" stroke=\"#222\" stroke-width=\"1.5\"/><text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{:.0}\" text-anchor=\"middle\" dominant-baseline=\"central\" font-family=\"sans-serif\">{}</text>", pts.join(" "), (b[0] + b[2]) / 2.0 * k, (m.h - (b[1] + b[3]) / 2.0) * k, size, p.n);
        }
        s += "</svg>\n<table><tr><th>Part</th><th>Width</th><th>Height</th><th>Layer</th></tr>\n";
        for p in &here {
            s += &format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n", p.n, len(p.w), len(p.h), esc(&p.layer));
        }
        s += "</table></section>\n";
    }
    s += "<section><h2>Labels</h2><div class=\"labels\">\n";
    for p in &all {
        s += &format!("<div class=\"label\"><b>{}-{}</b><small>{} · {}</small>{} × {}<small>{}</small></div>\n", p.sheet + 1, p.n, esc(project), esc(&doc.sheet_name(p.sheet)), len(p.w), len(p.h), esc(&p.layer));
    }
    s += "</div></section></body></html>\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::make_rect;

    #[test]
    fn parts_are_the_outer_vectors_numbered_in_reading_order_and_the_report_lists_them() {
        let mut d = Document::default();
        (d.material.w, d.material.h) = (48.0, 24.0);
        d.sync_sheet();
        d.add(make_rect(20.0, 2.0, 30.0, 10.0), None); // bottom row
        d.add(make_rect(13.0, 14.0, 20.0, 22.0), None); // top row, second
        let door = d.add(make_rect(2.0, 14.0, 12.0, 22.0), None); // top row, first
        d.add(make_rect(4.0, 16.0, 10.0, 20.0), None); // a panel inside the door: not a part
        d.add(make_rect(40.0, 1.0, 40.5, 1.5), None); // a speck: not a part
        let p = parts(&d);
        assert_eq!(p.len(), 3);
        assert_eq!((p[0].w, p[0].h, p[0].n), (10.0, 8.0, 1));
        assert!(p[0].outline.iter().all(|q| q.x <= 12.0) && d.find(door).is_some(), "the door at the top left is part 1");
        assert_eq!((p[1].w, p[2].w), (7.0, 10.0));
        let h = html(&d, "Kitchen <1>", &|v| format!("{v:.2} in"));
        assert!(h.contains("Kitchen &lt;1&gt;") && h.contains("3 part(s)") && h.contains("<td>7.00 in</td>"));
        assert!(h.contains("19% of the board used"), "216 of 1152 sq in");
        assert!(h.contains("largest offcut about 18.00 in × 24.00 in"), "the strip right of x = 30");
        assert_eq!(h.matches("class=\"label\"").count(), 3);
    }
}
