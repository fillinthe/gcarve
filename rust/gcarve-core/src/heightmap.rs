//! Bed leveling for GCarve Control: a grid of probed surface heights and a program rewritten to follow them
//! (engraving and V-carving on a board that is not flat: the cut depth stays even). Plain data, no I/O.

use crate::gcode::GProgram;
use crate::MoveType;

/// Surface heights on a grid, work coordinates in inches: `z[row * nx + col]` at (x0 + col·dx, y0 + row·dy), as the
/// work Z read where the probe touched (so about 0 where Z was zeroed).
#[derive(Clone, Debug, PartialEq)]
pub struct HeightMap {
    pub x0: f64,
    pub y0: f64,
    pub dx: f64,
    pub dy: f64,
    pub nx: usize,
    pub ny: usize,
    pub z: Vec<f64>,
}

impl HeightMap {
    /// Where to probe for an `nx` × `ny` grid over the box (x0, y0)–(x1, y1), row by row, each row turning back
    /// (a short path for the machine). At least 2 × 2.
    pub fn points(x0: f64, y0: f64, x1: f64, y1: f64, nx: usize, ny: usize) -> (HeightMap, Vec<(f64, f64)>) {
        let (nx, ny) = (nx.clamp(2, 30), ny.clamp(2, 30));
        let m = HeightMap { x0, y0, dx: (x1 - x0) / (nx - 1) as f64, dy: (y1 - y0) / (ny - 1) as f64, nx, ny, z: vec![0.0; nx * ny] };
        let mut pts = Vec::new();
        for r in 0..ny {
            for k in 0..nx {
                let c = if r % 2 == 0 { k } else { nx - 1 - k };
                pts.push((x0 + c as f64 * m.dx, y0 + r as f64 * m.dy));
            }
        }
        (m, pts)
    }
    /// index into `z` of the i-th point of `points` (rows turn back)
    pub fn index(&self, i: usize) -> usize {
        let (r, k) = (i / self.nx, i % self.nx);
        r * self.nx + if r % 2 == 0 { k } else { self.nx - 1 - k }
    }
    /// The surface height at (x, y): between the four probed points round it; outside the grid, the edge's height.
    pub fn at(&self, x: f64, y: f64) -> f64 {
        let f = |v: f64, v0: f64, d: f64, n: usize| if d.abs() < 1e-12 { 0.0 } else { ((v - v0) / d).clamp(0.0, (n - 1) as f64) };
        let (u, v) = (f(x, self.x0, self.dx, self.nx), f(y, self.y0, self.dy, self.ny));
        let (c, r) = ((u.floor() as usize).min(self.nx - 2), (v.floor() as usize).min(self.ny - 2));
        let (a, b) = (u - c as f64, v - r as f64);
        let z = |c: usize, r: usize| self.z[r * self.nx + c];
        z(c, r) * (1.0 - a) * (1.0 - b) + z(c + 1, r) * a * (1.0 - b) + z(c, r + 1) * (1.0 - a) * b + z(c + 1, r + 1) * a * b
    }
    /// highest − lowest probed point
    pub fn range(&self) -> f64 {
        self.z.iter().fold(f64::MIN, |a, b| a.max(*b)) - self.z.iter().fold(f64::MAX, |a, b| a.min(*b))
    }
}

/// G-code words of a line without its comments: (letter, number text)
pub fn words(line: &str) -> Vec<(char, String)> {
    let mut out = Vec::new();
    let (mut depth, mut cur): (i32, Option<(char, String)>) = (0, None);
    for ch in line.split(';').next().unwrap_or("").chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ if depth > 0 => {}
            c if c.is_ascii_alphabetic() => {
                out.extend(cur.take());
                cur = Some((c.to_ascii_uppercase(), String::new()));
            }
            c if !c.is_whitespace() => {
                if let Some(w) = cur.as_mut() {
                    w.1.push(c);
                }
            }
            _ => {}
        }
    }
    out.extend(cur);
    out
}

/// The program with every work-coordinate move following the surface: its Z is raised or lowered by the map's height
/// there, long moves are cut into pieces of at most `seg` inches, arcs become short straight moves. Lines without
/// moves (spindle, feeds, tool changes, comments) and machine-coordinate (G53) moves stay as they are.
/// Err: a program the rewrite can't follow safely (relative moves, a move sharing its line with other commands).
pub fn level_program(p: &GProgram, map: &HeightMap, seg: f64) -> Result<Vec<String>, String> {
    let seg = seg.max(0.02);
    let (mut out, mut mi) = (Vec::with_capacity(p.lines.len()), 0);
    let (mut mm, mut relative) = (false, false);
    for (n, line) in p.lines.iter().enumerate() {
        let w = words(line);
        for (l, v) in &w {
            if *l == 'G' {
                match v.parse::<f64>().ok().map(|g| (g * 10.0).round() as i32) {
                    Some(200) => mm = false,
                    Some(210) => mm = true,
                    Some(900) => relative = false,
                    Some(910) => relative = true,
                    _ => {}
                }
            }
        }
        let from = mi;
        while mi < p.motions.len() && p.motions[mi].line == n as i32 + 1 {
            mi += 1;
        }
        let ms = &p.motions[from..mi];
        if ms.is_empty() || ms.iter().any(|m| m.machine_coords) {
            out.push(line.clone());
            continue;
        }
        if relative {
            return Err(format!("line {}: relative moves (G91) can't follow the surface", n + 1));
        }
        let plain = w.iter().all(|(l, v)| "XYZIJKRFN".contains(*l) || (*l == 'G' && matches!(v.parse::<f64>().ok(), Some(g) if [0.0, 1.0, 2.0, 3.0, 17.0, 20.0, 21.0, 90.0, 94.0].contains(&g))));
        if !plain {
            return Err(format!("line {}: a move on the same line as another command can't follow the surface", n + 1));
        }
        let k = if mm { 25.4 } else { 1.0 };
        let dec = if mm { 3 } else { 4 };
        let mut first = true;
        for m in ms {
            let mut pts = Vec::new(); // (x, y, z) to visit, the end last
            match m.kind {
                MoveType::Rapid | MoveType::Linear => {
                    let len = (m.to.x - m.from.x).hypot(m.to.y - m.from.y);
                    let steps = if m.kind == MoveType::Rapid { 1 } else { ((len / seg).ceil() as usize).clamp(1, 100_000) };
                    for s in 1..=steps {
                        let t = s as f64 / steps as f64;
                        pts.push((m.from.x + (m.to.x - m.from.x) * t, m.from.y + (m.to.y - m.from.y) * t, m.from.z + (m.to.z - m.from.z) * t));
                    }
                }
                MoveType::ArcCw | MoveType::ArcCcw => {
                    let (cx, cy) = (m.from.x + m.i, m.from.y + m.j);
                    let r = m.i.hypot(m.j);
                    let a0 = (m.from.y - cy).atan2(m.from.x - cx);
                    let mut sweep = (m.to.y - cy).atan2(m.to.x - cx) - a0;
                    let tau = 2.0 * std::f64::consts::PI;
                    if m.kind == MoveType::ArcCcw {
                        if sweep <= 1e-9 { sweep += tau }
                    } else if sweep >= -1e-9 {
                        sweep -= tau;
                    }
                    let steps = (((r * sweep.abs()) / seg.min(0.05_f64.max(r * 0.1))).ceil() as usize).clamp(2, 100_000);
                    for s in 1..=steps {
                        let t = s as f64 / steps as f64;
                        let a = a0 + sweep * t;
                        pts.push(if s == steps { (m.to.x, m.to.y, m.to.z) } else { (cx + r * a.cos(), cy + r * a.sin(), m.from.z + (m.to.z - m.from.z) * t) });
                    }
                }
            }
            let g = if m.kind == MoveType::Rapid { "G0" } else { "G1" };
            for (x, y, z) in pts {
                let mut l = format!("{g} X{:.dec$} Y{:.dec$} Z{:.dec$}", x * k, y * k, (z + map.at(x, y)) * k);
                if first && m.kind != MoveType::Rapid && m.feed > 0.0 {
                    l += &format!(" F{:.1}", m.feed * k);
                }
                first = m.kind == MoveType::Rapid && first;
                out.push(l);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcode::parse_program;

    #[test]
    fn a_program_follows_a_tilted_surface_and_everything_else_stays() {
        // a board 0.1 in higher at x = 4 than at x = 0, level along y
        let (mut map, pts) = HeightMap::points(0.0, 0.0, 4.0, 2.0, 3, 2);
        assert_eq!(pts.len(), 6);
        assert_eq!(pts[3], (4.0, 2.0), "the second row comes back");
        for (i, (x, _)) in pts.iter().enumerate() {
            let k = map.index(i);
            map.z[k] = x * 0.025;
        }
        assert!((map.at(2.0, 1.0) - 0.05).abs() < 1e-12 && (map.at(9.0, 1.0) - 0.1).abs() < 1e-12 && (map.range() - 0.1).abs() < 1e-12);
        let text = "G90 G20\n(engrave)\nS18000 M3\nG0 X0 Y1 Z0.2\nG1 Z-0.02 F20\nG1 X4 F60\nG2 X4 Y0 I0 J-0.5\nG53 G0 Z0\nM5\n";
        let p = parse_program(text, 400.0);
        let out = level_program(&p, &map, 0.5).unwrap();
        assert!(out.contains(&"S18000 M3".to_string()) && out.contains(&"G53 G0 Z0".to_string()) && out.last().unwrap() == "M5");
        // read back: every cutting point is 0.02 under the surface where it is, the cut reaches x = 4
        let q = parse_program(&(out.join("\n") + "\n"), 400.0);
        assert!(q.errors.is_empty(), "{:?}", q.errors);
        let cuts: Vec<_> = q.motions.iter().filter(|m| m.kind != MoveType::Rapid && !m.machine_coords && m.from.z < 0.1).collect();
        assert!(cuts.len() > 10, "the long move and the arc are cut into pieces: {}", cuts.len());
        for m in &cuts {
            assert!((m.to.z - (map.at(m.to.x, m.to.y) - 0.02)).abs() < 1e-3, "{m:?}");
        }
        assert!(cuts.iter().any(|m| (m.to.x - 4.0).abs() < 1e-3 && (m.to.y - 1.0).abs() < 1e-3) && cuts.iter().any(|m| (m.to.y).abs() < 1e-3));
        // mm programs keep their unit; relative moves are refused
        let mm = level_program(&parse_program("G21 G90\nG1 X50.8 Y25.4 Z-1 F500\n", 400.0), &map, 5.0).unwrap();
        assert!(mm.iter().any(|l| l.starts_with("G1 X50.800 Y25.400 Z0.270")), "{mm:?}");
        assert!(level_program(&parse_program("G91\nG1 X1 F10\n", 400.0), &map, 0.5).is_err());
        assert!(level_program(&parse_program("G90\nG1 X1 F10 M3\n", 400.0), &map, 0.5).is_err());
    }
}
