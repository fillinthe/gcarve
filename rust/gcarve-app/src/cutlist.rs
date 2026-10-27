//! Cut-list import (VCarve's Cabinet Cut-list Import, the plain-CSV part): a spreadsheet of parts — name, length,
//! width, quantity — becomes one rectangle per part (copies for the quantity) on the "Cut list" layer, each with its
//! name as editable text on "Part labels" inside it (part and label grouped). The app then nests them across as many sheets as they need.
//! Columns are found by their heading (name / part, length / height, width, qty / quantity / count); without
//! headings the order is name, length, width, quantity. Numbers take units like any field ("600mm", "23 1/2").

#[allow(unused_imports)] // a file may only use it under one cfg
use gcarve_ui::i18n;
use gcarve_core::document::{Document, TextItem};
use gcarve_core::geom::{make_rect, pt};

pub struct Part {
    pub name: String,
    pub length: f64,
    pub width: f64,
    pub qty: usize,
}

/// The parts in `text` (inches through the document's units); Err says what is wrong with which line.
pub fn parse(doc: &Document, text: &str) -> Result<Vec<Part>, String> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let Some(first) = lines.first() else { return Err(i18n::t("the cut list is empty").into()) };
    let sep = [('\t', first.matches('\t').count()), (';', first.matches(';').count()), (',', first.matches(',').count())].into_iter().max_by_key(|s| s.1).map_or(',', |s| s.0);
    let cells = |l: &str| -> Vec<String> { l.split(sep).map(|c| c.trim().trim_matches('"').trim().to_string()).collect() };
    let head = cells(first).iter().map(|c| c.to_lowercase()).collect::<Vec<_>>();
    let find = |names: &[&str]| head.iter().position(|h| names.iter().any(|n| h == n || h.starts_with(n)));
    let (ni, li, wi, qi) = (find(&["name", "part", "description", "label"]), find(&["length", "len", "height", "long"]), find(&["width", "wide"]), find(&["qty", "quantity", "count", "pcs", "number"]));
    let has_head = li.is_some() && wi.is_some();
    let (ni, li, wi, qi) = if has_head { (ni, li.unwrap(), wi.unwrap(), qi) } else { (Some(0), 1, 2, Some(3)) };
    let mut parts = Vec::new();
    for (n, l) in lines.iter().enumerate().skip(usize::from(has_head)) {
        let c = cells(l);
        let get = |i: usize| c.get(i).map(String::as_str).unwrap_or("");
        let num = |i: usize| doc.eval(get(i)).map_err(|e| format!("line {}: \"{}\": {e}", n + 1, get(i)));
        let (length, width) = (num(li)?, num(wi)?);
        if length <= 0.0 || width <= 0.0 {
            return Err(format!("line {}: a part needs a length and a width above zero", n + 1));
        }
        let qty = qi.map_or(Ok(1.0), |i| if get(i).is_empty() { Ok(1.0) } else { doc.eval_dim(get(i)).map(|(v, _)| v) }).map_err(|e| format!("line {}: quantity: {e}", n + 1))?;
        if !(1.0..=500.0).contains(&qty) {
            return Err(format!("line {}: quantity must be 1 to 500", n + 1));
        }
        let name = ni.map(|i| get(i).to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Part {}", parts.len() + 1));
        parts.push(Part { name, length, width, qty: qty.round() as usize });
    }
    if parts.is_empty() {
        return Err(i18n::t("no parts found under the headings").into());
    }
    Ok(parts)
}

/// The parts as rectangles (length along X) in a row above the board, with their labels; returns the shape ids
/// (parts and labels) for nesting.
pub fn add(doc: &mut Document, parts: &[Part]) -> Vec<i32> {
    doc.checkpoint();
    let (cut, labels) = (doc.layer_named(i18n::t("Cut list")), doc.layer_named(i18n::t("Part labels")));
    let mut ids = Vec::new();
    let mut x = 0.0;
    let y = doc.material.h + 2.0;
    for p in parts {
        for k in 0..p.qty {
            // the part and its label are one group: they select, move and nest together
            let group = doc.new_group_id();
            let rect = doc.add(make_rect(x, y, x + p.length, y + p.width), Some(cut));
            doc.find_mut(rect).unwrap().group = group;
            ids.push(rect);
            // the name, small and inside the part so it travels with it when nested
            let height = (p.width * 0.18).clamp(0.12, 0.75);
            let text = if p.qty > 1 { format!("{} {}/{}", p.name, k + 1, p.qty) } else { p.name.clone() };
            let t = TextItem { id: doc.next_text_id, text, height, ..Default::default() };
            let (glyphs, _) = crate::text::build_text(&t, pt(x + p.length / 2.0, y + p.width / 2.0), None);
            let fits = glyphs.iter().map(gcarve_core::geom::bounds).all(|b| b.x0 > x && b.x1 < x + p.length);
            if !glyphs.is_empty() && fits {
                doc.next_text_id += 1;
                for g in glyphs {
                    let id = doc.add(g, Some(labels));
                    let s = doc.find_mut(id).unwrap();
                    s.text = t.id;
                    s.group = group;
                    ids.push(id);
                }
                doc.texts.push(t);
            }
            x += p.length + 1.0;
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_cut_list_reads_headings_units_and_quantities() {
        let doc = gcarve_core::document::Document::default();
        let csv = "Part,Qty,Length,Width\nSide,2,30,11.25\n\"Top\",1,600mm,11.25\nShelf,,24.5,11\n";
        let parts = super::parse(&doc, csv).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!((parts[0].name.as_str(), parts[0].qty, parts[0].length), ("Side", 2, 30.0));
        assert!((parts[1].length - 600.0 / 25.4).abs() < 1e-9 && parts[2].qty == 1);
        // no headings: name, length, width, quantity; tabs from a spreadsheet
        let plain = super::parse(&doc, "Door\t20\t15\t4").unwrap();
        assert_eq!((plain[0].width, plain[0].qty), (15.0, 4));
        assert!(super::parse(&doc, "Part,Length,Width\nBad,ten,5").is_err());
        let mut d = doc.clone();
        let ids = super::add(&mut d, &parts);
        assert_eq!(d.shapes.iter().filter(|s| ids.contains(&s.id) && s.text == 0).count(), 4, "2 sides + top + shelf");
        assert_eq!(d.texts.len(), 4);
    }
}
