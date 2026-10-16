//! Built-in shapes library (VCarve clipart, the parts a sign / woodworking shop reaches for): drawn here in code, so
//! nothing is copied from anyone's clipart. Every entry is normalized to 1 inch wide (aspect kept), bottom-left at
//! (0, 0); the app scales it to the size the user asks for. Curves are Béziers where the shape is round, so the
//! node editor works on them.

use crate::geom::{bounds, make_ellipse, make_polyline, pt, transformed, Affine, Builder, Contour, Pt};
use crate::vecops::{boolean_op, make_polygon, make_star, BoolOp};
use std::f64::consts::PI;

pub struct ClipShape {
    pub name: &'static str,
    pub category: &'static str,
    pub contours: Vec<Contour>,
}

pub const CATEGORIES: [&str; 4] = ["Basic", "Signs & plaques", "Woodworking", "Decorative"];

fn poly(p: &[(f64, f64)]) -> Contour {
    make_polyline(&p.iter().map(|&(x, y)| pt(x, y)).collect::<Vec<Pt>>(), true)
}

fn circle(x: f64, y: f64, r: f64) -> Contour {
    make_ellipse(x, y, r, r)
}

fn rounded_rect(w: f64, h: f64, r: f64) -> Contour {
    let mut b = Builder::default();
    b.start(pt(r, 0.0));
    b.line(pt(w - r, 0.0));
    b.arc(w - r, r, r, r, 0.0, -PI / 2.0, PI / 2.0);
    b.line(pt(w, h - r));
    b.arc(w - r, h - r, r, r, 0.0, 0.0, PI / 2.0);
    b.line(pt(r, h));
    b.arc(r, h - r, r, r, 0.0, PI / 2.0, PI / 2.0);
    b.line(pt(0.0, r));
    b.arc(r, r, r, r, 0.0, PI, PI / 2.0);
    b.close();
    b.c
}

fn heart() -> Contour {
    let mut b = Builder::default();
    b.start(pt(0.0, -1.0));
    b.cubic(pt(-0.25, -0.65), pt(-1.0, -0.3), pt(-1.0, 0.3));
    b.cubic(pt(-1.0, 0.85), pt(-0.25, 1.05), pt(0.0, 0.55));
    b.cubic(pt(0.25, 1.05), pt(1.0, 0.85), pt(1.0, 0.3));
    b.cubic(pt(1.0, -0.3), pt(0.25, -0.65), pt(0.0, -1.0));
    b.close();
    b.c
}

fn arch_plaque() -> Contour {
    let mut b = Builder::default();
    b.start(pt(0.0, 0.0));
    b.line(pt(1.0, 0.0));
    b.line(pt(1.0, 0.7));
    b.arc(0.5, 0.7, 0.5, 0.5, 0.0, 0.0, PI);
    b.close();
    b.c
}

/// Keyhole hanger: screw-head circle on top, slot below with a round end (one outline).
fn keyhole() -> Contour {
    let (r, w, cy, bottom): (f64, f64, f64, f64) = (0.5, 0.2, 1.0, -0.4); // head radius, slot half-width, head centre, slot end centre
    let y = cy - (r * r - w * w).sqrt();
    let a0 = (y - cy).atan2(w);
    let mut b = Builder::default();
    b.start(pt(w, bottom));
    b.line(pt(w, y));
    b.arc(0.0, cy, r, r, 0.0, a0, PI - 2.0 * a0); // counter-clockwise over the top to the mirrored point
    b.line(pt(-w, bottom));
    b.arc(0.0, bottom, w, w, 0.0, PI, PI);
    b.close();
    b.c
}

fn shield() -> Contour {
    let mut b = Builder::default();
    b.start(pt(0.0, 1.0));
    b.line(pt(1.0, 1.0));
    b.line(pt(1.0, 0.55));
    b.cubic(pt(1.0, 0.2), pt(0.7, 0.02), pt(0.5, -0.12));
    b.cubic(pt(0.3, 0.02), pt(0.0, 0.2), pt(0.0, 0.55));
    b.close();
    b.c
}

fn leaf() -> Contour {
    let mut b = Builder::default();
    b.start(pt(0.0, 0.0));
    b.cubic(pt(0.25, 0.45), pt(0.75, 0.45), pt(1.0, 0.0));
    b.cubic(pt(0.75, -0.45), pt(0.25, -0.45), pt(0.0, 0.0));
    b.close();
    b.c
}

fn teardrop() -> Contour {
    let mut b = Builder::default();
    b.start(pt(0.0, 1.0));
    b.cubic(pt(0.15, 0.7), pt(0.45, 0.45), pt(0.45, 0.2));
    b.arc(0.0, 0.2, 0.45, 0.45, 0.0, 0.0, -PI);
    b.cubic(pt(-0.45, 0.45), pt(-0.15, 0.7), pt(0.0, 1.0));
    b.close();
    b.c
}

/// Spur gear: `teeth` trapezoid teeth around a bore.
fn gear(teeth: usize) -> Vec<Contour> {
    let (ro, ri) = (1.0, 0.8);
    let step = 2.0 * PI / teeth as f64;
    let mut pts = Vec::new();
    for i in 0..teeth {
        let a = i as f64 * step;
        for (da, r) in [(0.0, ri), (0.18, ro), (0.42, ro), (0.6, ri)] {
            let t = a + da * step;
            pts.push((r * t.cos(), r * t.sin()));
        }
    }
    vec![poly(&pts), circle(0.0, 0.0, 0.25)]
}

fn paw() -> Vec<Contour> {
    let mut v = vec![make_ellipse(0.0, 0.0, 0.42, 0.34)];
    for (x, y, rx, ry) in [(-0.5, 0.42, 0.14, 0.18), (-0.18, 0.66, 0.15, 0.2), (0.18, 0.66, 0.15, 0.2), (0.5, 0.42, 0.14, 0.18)] {
        v.push(make_ellipse(x, y, rx, ry));
    }
    v
}

fn speech_bubble() -> Vec<Contour> {
    boolean_op(BoolOp::Weld, &[rounded_rect(1.4, 0.8, 0.18)], &[poly(&[(0.25, 0.2), (0.6, 0.2), (0.15, -0.3)])])
}

fn cloud() -> Vec<Contour> {
    let parts = [circle(0.3, 0.3, 0.3), circle(0.75, 0.45, 0.42), circle(1.2, 0.3, 0.3), poly(&[(0.3, 0.0), (1.2, 0.0), (1.2, 0.3), (0.3, 0.3)])];
    boolean_op(BoolOp::Weld, &parts[..1], &parts[1..])
}

fn crescent() -> Vec<Contour> {
    boolean_op(BoolOp::Subtract, &[circle(0.0, 0.0, 1.0)], &[circle(0.45, 0.25, 0.85)])
}

fn raw() -> Vec<(&'static str, &'static str, Vec<Contour>)> {
    vec![
        ("Circle", "Basic", vec![circle(0.0, 0.0, 1.0)]),
        ("Oval", "Basic", vec![make_ellipse(0.0, 0.0, 1.0, 0.6)]),
        ("Rounded rectangle", "Basic", vec![rounded_rect(1.6, 1.0, 0.18)]),
        ("Triangle", "Basic", vec![make_polygon(pt(0.0, 0.0), 1.0, 3, 0.0)]),
        ("Diamond", "Basic", vec![poly(&[(0.0, 0.0), (0.5, -0.75), (1.0, 0.0), (0.5, 0.75)])]),
        ("Hexagon", "Basic", vec![make_polygon(pt(0.0, 0.0), 1.0, 6, PI / 6.0)]),
        ("Octagon", "Basic", vec![make_polygon(pt(0.0, 0.0), 1.0, 8, PI / 8.0)]),
        ("Star", "Basic", vec![make_star(pt(0.0, 0.0), 1.0, 0.42, 5, 0.0)]),
        ("Heart", "Basic", vec![heart()]),
        ("Arrow", "Basic", vec![poly(&[(0.0, -0.15), (0.6, -0.15), (0.6, -0.4), (1.0, 0.0), (0.6, 0.4), (0.6, 0.15), (0.0, 0.15)])]),
        ("Cross", "Basic", vec![poly(&[(0.35, 0.0), (0.65, 0.0), (0.65, 0.35), (1.0, 0.35), (1.0, 0.65), (0.65, 0.65), (0.65, 1.0), (0.35, 1.0), (0.35, 0.65), (0.0, 0.65), (0.0, 0.35), (0.35, 0.35)])]),
        ("Chevron", "Basic", vec![poly(&[(0.0, 0.0), (0.5, 0.5), (1.0, 0.0), (1.0, 0.3), (0.5, 0.8), (0.0, 0.3)])]),
        ("Arch-top plaque", "Signs & plaques", vec![arch_plaque()]),
        ("Banner", "Signs & plaques", vec![poly(&[(0.0, 0.0), (1.6, 0.0), (1.45, 0.25), (1.6, 0.5), (0.0, 0.5), (0.15, 0.25)])]),
        ("Shield", "Signs & plaques", vec![shield()]),
        ("House", "Signs & plaques", vec![poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 0.6), (0.5, 1.0), (0.0, 0.6)])]),
        ("Tag", "Signs & plaques", vec![poly(&[(0.25, 0.0), (1.0, 0.0), (1.0, 0.6), (0.25, 0.6), (0.0, 0.3)]), circle(0.2, 0.3, 0.06)]),
        ("Speech bubble", "Signs & plaques", speech_bubble()),
        ("Picture frame", "Signs & plaques", vec![rounded_rect(1.4, 1.0, 0.05), transformed(&rounded_rect(1.0, 0.6, 0.03), &Affine::translate(0.2, 0.2))]),
        ("Keyhole hanger", "Woodworking", vec![keyhole()]),
        ("Butterfly key", "Woodworking", vec![poly(&[(0.0, 0.0), (0.5, 0.3), (1.0, 0.0), (1.0, 1.0), (0.5, 0.7), (0.0, 1.0)])]),
        ("Washer / ring", "Woodworking", vec![circle(0.0, 0.0, 1.0), circle(0.0, 0.0, 0.45)]),
        ("Gear", "Woodworking", gear(12)),
        ("Leaf", "Decorative", vec![leaf()]),
        ("Teardrop", "Decorative", vec![teardrop()]),
        ("Crescent moon", "Decorative", crescent()),
        ("Cloud", "Decorative", cloud()),
        ("Paw print", "Decorative", paw()),
        ("Sunburst", "Decorative", vec![make_star(pt(0.0, 0.0), 1.0, 0.7, 16, 0.0)]),
    ]
}

/// Bounds of several contours.
pub fn bounds_all(cs: &[Contour]) -> crate::geom::Rect {
    let mut r = crate::geom::Rect::default();
    for c in cs {
        r.add_rect(&bounds(c));
    }
    r
}

/// Scales and moves `cs` so they are `width` wide with the bottom-left at `at`.
pub fn place(cs: &[Contour], width: f64, at: Pt) -> Vec<Contour> {
    let b = bounds_all(cs);
    let k = if b.w() > 0.0 { width / b.w() } else { 1.0 };
    let m = Affine::translate(at.x, at.y) * Affine::scale(k, k) * Affine::translate(-b.x0, -b.y0);
    cs.iter().map(|c| transformed(c, &m)).collect()
}

/// The library, each entry 1 inch wide.
pub fn library() -> Vec<ClipShape> {
    raw().into_iter().map(|(name, category, cs)| ClipShape { name, category, contours: place(&cs, 1.0, Pt::default()) }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_is_closed_and_one_inch_wide() {
        let lib = library();
        assert!(lib.len() >= 25);
        for s in &lib {
            assert!(!s.contours.is_empty() && s.contours.iter().all(|c| c.closed && c.nodes.len() >= 2), "{}", s.name);
            let b = bounds_all(&s.contours);
            assert!((b.w() - 1.0).abs() < 1e-6 && b.x0.abs() < 1e-9 && b.y0.abs() < 1e-9 && b.h() > 0.1, "{} {b:?}", s.name);
            assert!(CATEGORIES.contains(&s.category), "{}", s.name);
        }
        // the keyhole is one outline: head wider than the slot, slot below
        let k = lib.iter().find(|s| s.name == "Keyhole hanger").unwrap();
        assert_eq!(k.contours.len(), 1);
        let b = bounds_all(&k.contours);
        assert!(b.h() > 1.4 * b.w(), "{b:?}");
        // placing at 6 in keeps the aspect
        let p = place(&k.contours, 6.0, pt(10.0, 5.0));
        let pb = bounds_all(&p);
        // bounds come from flattened curves (absolute tolerance), so a few thousandths
        assert!((pb.w() - 6.0).abs() < 0.01 && (pb.x0 - 10.0).abs() < 0.01 && ((pb.h() / pb.w()) - (b.h() / b.w())).abs() < 0.01);
    }
}
