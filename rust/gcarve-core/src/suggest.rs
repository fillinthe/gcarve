//! "Suggest my toolpaths": reads the active sheet's vectors and proposes a set of cuts for the user to approve.
//! Plain rules, nothing learnt: letters are V-carved, small circles are holes, open lines are engraved, shapes
//! inside a part are cut through (or pocketed), and each part is cut out last with tabs. The app turns the ticked
//! proposals into ordinary toolpaths (bits from the library), so everything stays editable.

use crate::cam::{OpType, Side, ToolType};
use crate::document::Document;
use crate::geom::{bounds, flatten, point_in_poly, pt, Pt, Rect};

/// circles up to this across are holes
const HOLE_MAX: f64 = 0.75;
/// holes up to this across are drilled with a bit their own size; bigger ones are bored with a 1/4 in end mill
const DRILL_MAX: f64 = 0.26;

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub name: String,
    /// one line saying what was seen and what the cut does
    pub why: String,
    pub kind: OpType,
    pub side: Side,
    pub depth: f64,
    /// V-carve: flat floor depth (0 = none)
    pub flat_depth: f64,
    pub tabs: bool,
    pub bit: ToolType,
    pub bit_dia: f64,
    pub shape_ids: Vec<i32>,
    /// ticked when the list opens (an alternative to another proposal starts unticked)
    pub on: bool,
}

/// Proposals for the active sheet, in cutting order (holes and carving first, each part cut free last).
pub fn suggest(doc: &Document) -> Vec<Suggestion> {
    let (t, m) = (doc.material.t, &doc.material);
    struct S {
        id: i32,
        poly: Vec<Pt>,
        b: Rect,
        text: bool,
        hole: f64,
        depth: usize,
    }
    let keep_out = |layer: i32| doc.layers.get(layer as usize).is_some_and(|l| l.keep_out);
    let seen: Vec<_> = doc.shapes.iter().filter(|s| doc.shape_visible(s) && !keep_out(s.layer)).collect();
    let open: Vec<i32> = seen.iter().filter(|s| !s.c.closed).map(|s| s.id).collect(); // single-line text too
    let mut closed: Vec<S> = seen
        .iter()
        .filter(|s| s.c.closed)
        .map(|s| {
            let poly = flatten(&s.c, 0.005);
            S { id: s.id, b: bounds(&s.c), text: s.text != 0, hole: circle(&poly).filter(|d| *d <= HOLE_MAX && s.text == 0).unwrap_or(0.0), poly, depth: 0 } // a full stop is not a hole
        })
        .filter(|s| s.poly.len() >= 3)
        .collect();
    // a rectangle drawn round the board edge is the board, not a part
    closed.retain(|s| !(s.b.x0.abs() < 0.02 && s.b.y0.abs() < 0.02 && (s.b.x1 - m.w).abs() < 0.02 && (s.b.y1 - m.h).abs() < 0.02));
    // how many other shapes each one sits inside. ponytail: every pair whose boxes nest is tested; a grid index if
    // drawings of many thousand shapes get slow here
    let depths: Vec<usize> = closed
        .iter()
        .map(|a| closed.iter().filter(|o| o.id != a.id && !o.text && o.b.x0 <= a.b.x0 && o.b.y0 <= a.b.y0 && o.b.x1 >= a.b.x1 && o.b.y1 >= a.b.y1 && point_in_poly(a.poly[0], o.poly.len(), |i| (o.poly[i].x, o.poly[i].y))).count())
        .collect();
    for (s, d) in closed.iter_mut().zip(depths) {
        s.depth = d;
    }
    let ids = |f: &dyn Fn(&S) -> bool| closed.iter().filter(|s| f(s)).map(|s| s.id).collect::<Vec<i32>>();
    let base = |name: &str, why: String, kind: OpType, shape_ids: Vec<i32>| Suggestion { name: name.into(), why, kind, side: Side::Outside, depth: t, flat_depth: 0.0, tabs: false, bit: ToolType::EndMill, bit_dia: 0.25, shape_ids, on: true };
    let mut out = Vec::new();

    // holes, one toolpath per size
    let mut sizes: Vec<i64> = closed.iter().filter(|s| s.hole > 0.0).map(|s| (s.hole * 1000.0).round() as i64).collect();
    sizes.sort_unstable();
    sizes.dedup();
    for k in sizes {
        let (d, holes) = (k as f64 / 1000.0, ids(&|s| s.hole > 0.0 && (s.hole * 1000.0).round() as i64 == k));
        let n = holes.len();
        out.push(if d <= DRILL_MAX {
            Suggestion { bit_dia: d, ..base("Drill holes", format!("{n} small circle(s) {d:.3} in across: drilled through with a bit that size"), OpType::Drill, holes) }
        } else {
            Suggestion { side: Side::Inside, ..base("Bore holes", format!("{n} circle(s) {d:.3} in across: bored through with a 1/4 in end mill"), OpType::Bore, holes) }
        });
    }
    let letters = ids(&|s| s.text);
    if !letters.is_empty() {
        let d = (t / 3.0).min(0.25);
        out.push(Suggestion { depth: d, flat_depth: d, bit: ToolType::VBit, bit_dia: 0.5, ..base("V-carve the letters", format!("{} letter outline(s) of text: V-carved with a 90° V-bit, flat floor {d:.3} in down", letters.len()), OpType::VCarve, letters) });
    }
    if !open.is_empty() {
        out.push(Suggestion { side: Side::On, depth: 0.03, bit: ToolType::VBit, bit_dia: 0.5, ..base("Engrave the lines", format!("{} open line(s): engraved on the line with a V-bit, 0.03 in deep", open.len()), OpType::Profile, open) });
    }
    // shapes inside a part: cut through, or (the alternative) pocketed with whatever stands inside them left as islands
    let plain = |s: &S| !s.text && s.hole == 0.0;
    let inner = ids(&|s| plain(s) && s.depth % 2 == 1);
    if !inner.is_empty() {
        let n = inner.len();
        let d = (t / 2.0).min(0.25);
        let with_islands = ids(&|s| plain(s) && s.depth >= 1);
        out.push(Suggestion { side: Side::Inside, ..base("Cut out the inside shapes", format!("{n} shape(s) inside a part: cut right through on the inside of the line"), OpType::Profile, inner) });
        out.push(Suggestion { depth: d, on: false, ..base("Pocket the inside shapes", format!("Instead of cutting them through: {n} shape(s) cleared {d:.3} in deep"), OpType::Pocket, with_islands) });
    }
    // outlines with nothing round them, and parts standing inside another part's cut-out
    let parts = ids(&|s| plain(s) && s.depth % 2 == 0);
    if !parts.is_empty() {
        out.push(Suggestion { tabs: true, ..base("Cut out the parts", format!("{} part outline(s): cut through on the outside of the line, last, held by tabs", parts.len()), OpType::Profile, parts) });
    }
    out
}

/// The diameter when the outline is a circle (every point the same distance from the middle of its box, within 2 %)
fn circle(poly: &[Pt]) -> Option<f64> {
    let mut b = Rect::default();
    for p in poly {
        b.add(*p);
    }
    let (w, h) = (b.x1 - b.x0, b.y1 - b.y0);
    let (c, r) = (pt((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0), (w + h) / 4.0);
    (poly.len() >= 8 && r > 0.0 && (w - h).abs() <= 0.02 * w.max(h) && poly.iter().all(|p| ((p.x - c.x).hypot(p.y - c.y) - r).abs() <= 0.02 * r + 0.003)).then_some(2.0 * r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Shape;
    use crate::geom::Contour;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Contour {
        crate::geom::make_polyline(&[pt(x, y), pt(x + w, y), pt(x + w, y + h), pt(x, y + h)], true)
    }
    fn ring(cx: f64, cy: f64, d: f64) -> Contour {
        crate::geom::make_polyline(&(0..48).map(|k| (k as f64 / 48.0 * std::f64::consts::TAU).sin_cos()).map(|(s, c)| pt(cx + c * d / 2.0, cy + s * d / 2.0)).collect::<Vec<_>>(), true)
    }

    #[test]
    fn a_plate_with_holes_a_window_text_and_a_line_gets_the_cuts_a_person_would_pick() {
        let mut d = Document::default();
        (d.material.w, d.material.h, d.material.t) = (24.0, 12.0, 0.75);
        let mut add = |c: Contour, text: i32| {
            let id = d.shapes.len() as i32 + 1;
            d.shapes.push(Shape { id, c, text, ..Default::default() });
            id
        };
        let board = add(rect(0.0, 0.0, 24.0, 12.0), 0); // the board's own outline: not a part
        let plate = add(rect(2.0, 2.0, 12.0, 8.0), 0);
        let window = add(rect(4.0, 4.0, 4.0, 3.0), 0);
        let island = add(rect(5.0, 5.0, 1.0, 1.0), 0);
        let small = [add(ring(3.0, 3.0, 0.25), 0), add(ring(13.0, 3.0, 0.25), 0)];
        let big = add(ring(11.0, 8.0, 0.5), 0);
        let letter = add(rect(9.0, 4.0, 0.5, 1.0), 7);
        let dot = add(ring(10.0, 4.1, 0.2), 7); // a full stop: text, never a hole to drill
        let line = add(crate::geom::make_polyline(&[pt(16.0, 2.0), pt(20.0, 9.0)], false), 0);
        let wheel = add(ring(19.0, 4.0, 3.0), 0); // a circle too big to be a hole: a part

        let s = suggest(&d);
        let names: Vec<&str> = s.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Drill holes", "Bore holes", "V-carve the letters", "Engrave the lines", "Cut out the inside shapes", "Pocket the inside shapes", "Cut out the parts"]);
        assert!(s.iter().all(|x| !x.shape_ids.contains(&board)));
        assert_eq!((s[0].shape_ids.as_slice(), s[0].bit_dia, s[0].depth), (&small[..], 0.25, 0.75));
        assert_eq!((s[1].shape_ids.as_slice(), s[1].kind), (&[big][..], OpType::Bore));
        assert_eq!((s[2].shape_ids.as_slice(), s[2].bit, s[2].depth), (&[letter, dot][..], ToolType::VBit, 0.25));
        assert_eq!((s[3].shape_ids.as_slice(), s[3].side), (&[line][..], Side::On));
        assert_eq!((s[4].shape_ids.as_slice(), s[4].side, s[4].on), (&[window][..], Side::Inside, true));
        // the pocket is the alternative: off, and it takes the island along so it is left standing
        assert_eq!((s[5].shape_ids.as_slice(), s[5].on), (&[window, island][..], false));
        assert_eq!((s[6].shape_ids.as_slice(), s[6].tabs, s[6].depth), (&[plate, island, wheel][..], true, 0.75)); // the island in the window is a part of its own
        // the cut that frees the parts comes last
        assert_eq!(s.last().unwrap().name, "Cut out the parts");
        assert!(suggest(&Document::default()).is_empty());
    }
}
