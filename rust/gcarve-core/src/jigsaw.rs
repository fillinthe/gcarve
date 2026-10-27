//! Jigsaw puzzle cut lines: a grid of cols × rows pieces over a w × h rectangle (corner at 0,0), every inside edge a
//! curve with a knob that locks the two pieces, knobs flipped and varied by a seeded random (same seed = same
//! puzzle). Returns open lines (one per grid line, edge to edge) plus the closed border: cut them all on the line.
//! Own design: the knob is six cubics in edge coordinates (u along 0..1, v across, height relative to the piece).

use crate::geom::{pt, Builder, Contour, Pt};

/// xorshift: the same tiny generator the texture toolpath uses, so a seed gives the same puzzle everywhere
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    /// -1..1
    fn sym(&mut self) -> f64 {
        self.next() * 2.0 - 1.0
    }
}

/// One edge from a to b with a knob on side `flip` (+1 left of a→b, -1 right); `h` = knob height (inches).
fn edge(b: &mut Builder, a: Pt, z: Pt, h: f64, flip: f64, rng: &mut Rng) {
    let (d, len) = (z - a, crate::geom::dist(a, z));
    let (ux, uy) = (d.x / len, d.y / len);
    let (nx, ny) = (-uy * flip, ux * flip);
    let c = 0.5 + 0.06 * rng.sym(); // knob centre along the edge
    let s = 1.0 + 0.1 * rng.sym(); // knob size
    let p = |u: f64, v: f64| {
        let (u, v) = (c + (u - 0.5) * s, v * s);
        pt(a.x + ux * u * len + nx * v / 0.25 * h, a.y + uy * u * len + ny * v / 0.25 * h)
    };
    b.line(p(0.36, 0.0));
    b.cubic(p(0.41, 0.0), p(0.44, 0.06), p(0.40, 0.12)); // into the neck
    b.cubic(p(0.34, 0.20), p(0.38, 0.25), p(0.50, 0.25)); // round the head
    b.cubic(p(0.62, 0.25), p(0.66, 0.20), p(0.60, 0.12));
    b.cubic(p(0.56, 0.06), p(0.59, 0.0), p(0.64, 0.0)); // out of the neck
    b.line(z);
}

/// The cut lines of the puzzle; `seed` picks the knob directions and shapes.
pub fn puzzle(w: f64, h: f64, cols: usize, rows: usize, seed: u64) -> Vec<Contour> {
    let (cols, rows) = (cols.clamp(1, 100), rows.clamp(1, 100));
    let (cw, ch) = (w / cols as f64, h / rows as f64);
    // knob height ~ a quarter of the smaller piece side: the head stays inside the neighbour piece
    let kh = 0.25 * cw.min(ch);
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut out = Vec::new();
    for r in 1..rows {
        let y = r as f64 * ch;
        let mut b = Builder::default();
        b.start(pt(0.0, y));
        for c in 0..cols {
            let flip = if rng.next() < 0.5 { 1.0 } else { -1.0 };
            edge(&mut b, pt(c as f64 * cw, y), pt((c + 1) as f64 * cw, y), kh, flip, &mut rng);
        }
        out.push(b.c);
    }
    for c in 1..cols {
        let x = c as f64 * cw;
        let mut b = Builder::default();
        b.start(pt(x, 0.0));
        for r in 0..rows {
            let flip = if rng.next() < 0.5 { 1.0 } else { -1.0 };
            edge(&mut b, pt(x, r as f64 * ch), pt(x, (r + 1) as f64 * ch), kh, flip, &mut rng);
        }
        out.push(b.c);
    }
    out.push(crate::geom::make_rect(0.0, 0.0, w, h));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{bounds, flatten};

    #[test]
    fn puzzle_lines_cross_the_board_and_knobs_stay_inside_their_neighbour() {
        let (w, h, cols, rows) = (12.0, 9.0, 4, 3);
        let lines = puzzle(w, h, cols, rows, 7);
        assert_eq!(lines.len(), (rows - 1) + (cols - 1) + 1);
        let (cw, ch) = (w / cols as f64, h / rows as f64);
        for (i, l) in lines.iter().enumerate().take(rows - 1) {
            let b = bounds(l);
            let y = (i + 1) as f64 * ch;
            assert!((b.x0).abs() < 1e-9 && (b.x1 - w).abs() < 1e-9, "edge to edge");
            assert!(b.y0 > y - ch / 2.0 && b.y1 < y + ch / 2.0, "knobs reach less than half a piece: {b:?}");
            assert!(b.y1 - b.y0 > 0.1 * ch, "it has knobs");
        }
        for l in &lines[rows - 1..lines.len() - 1] {
            let b = bounds(l);
            assert!((b.y0).abs() < 1e-9 && (b.y1 - h).abs() < 1e-9);
            assert!(b.x1 - b.x0 < cw, "vertical knobs stay within their column band");
        }
        // the same seed gives the same puzzle, another seed another one
        assert_eq!(flatten(&puzzle(w, h, cols, rows, 7)[0], 0.01), flatten(&lines[0], 0.01));
        assert_ne!(flatten(&puzzle(w, h, cols, rows, 8)[0], 0.01), flatten(&lines[0], 0.01));
    }
}
