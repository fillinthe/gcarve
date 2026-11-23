//! Turning on a rotary axis (blank held in a chuck, steadied by a tailstock): the blank spins under the bit like on
//! a lathe while the bit travels its length, cutting it to a drawn side profile (a spindle, a leg, a baluster).
//! The profile is an open vector drawn at true size: left to right = along the blank, its highest point = the
//! blank's surface, and how far it dips below that = how deep the cut is there. The bit's own shape is allowed for,
//! so it never cuts into a shoulder. Passes go down a stepdown at a time, each only along the stretches that still
//! have wood to take, one stepover per turn; on the unrolled board a turn runs Y 0 → circumference and a "seam"
//! move back to Y 0, which `wrap_jobs` turns into carrying on round (as `cam::generate_rounding`).
//! The chuck and tailstock lengths (Job Setup) are never cut, and any toolpath coming down over them is flagged.

use crate::cam::{cut_to, rapid_to, step_down, CamResult, Material, Op, Tool, ToolType};
use crate::geom::{flatten, pt, Contour, Pt};
use crate::job::{Move, MoveType, Toolpath};

/// pieces of one turn: the profile is followed this finely along the blank
const PER_TURN: usize = 12;
/// wood left round the axis at the least
const CORE: f64 = 0.1;
/// most turns of the blank in one pass, and most passes (a tiny stepover on a long blank would never finish)
const MAX_TURNS: usize = 5000;
const MAX_PASSES: f64 = 200.0;

pub fn generate_turning(op: &Op, tool: &Tool, contours: &[Contour], m: &Material) -> CamResult {
    let mut res = CamResult::default();
    let Some(dia) = m.rotary else {
        res.warnings.push(format!("{}: turning needs a rotary job (Job Setup › Rotary)", op.name));
        return res;
    };
    // the profile: every picked vector, flattened; where several lie over the same place the highest counts
    let lines: Vec<Vec<Pt>> = contours.iter().map(|c| flatten(c, 0.002)).filter(|p| p.len() >= 2).collect();
    let top = lines.iter().flatten().map(|p| p.y).fold(f64::MIN, f64::max);
    let (mut x0, mut x1) = lines.iter().flatten().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
    // the bit's side stays out of the chuck and the tailstock, not only its middle
    x0 = x0.max(if m.chuck > 0.0 { m.chuck + tool.diameter / 2.0 } else { 0.0 });
    x1 = x1.min(m.w - if m.tail > 0.0 { m.tail + tool.diameter / 2.0 } else { 0.0 });
    if lines.is_empty() || x1 - x0 < 1e-6 {
        res.warnings.push(format!("{}: draw the side profile as an open line along the blank (clear of the chuck and tailstock) and pick it", op.name));
        return res;
    }
    let drawn = |x: f64| drawn(&lines, top, x);
    let deepest = -(dia / 2.0 - CORE).max(0.0);
    let (r, vee) = (tool.diameter / 2.0, tool.kind == ToolType::VBit);
    let tan_half = (tool.angle / 2.0).to_radians().tan().max(1e-6);
    let mut too_deep = false;
    // the lowest the tip may be at x so that no part of the bit is inside the drawn shape
    let mut tip = |x: f64| -> f64 {
        let n = ((r / 0.004).ceil() as usize).clamp(4, 200) * 2; // even: the tip itself (u = 0) is one of the places
        let mut z = f64::MIN;
        for k in 0..=n {
            let u = -r + 2.0 * r * k as f64 / n as f64;
            let rise = if vee { u.abs() / tan_half } else { tool.ball_rise(u) };
            z = z.max(drawn(x + u) - rise);
        }
        too_deep |= z < deepest - 1e-9;
        z.max(deepest).min(0.0)
    };
    let step = (tool.cut_diameter() * if op.stepover > 0.0 { op.stepover.min(1.0) } else { 0.25 }).max(0.004);
    let mut turns = ((x1 - x0) / step).ceil().max(1.0) as usize;
    if turns > MAX_TURNS {
        res.warnings.push(format!("{}: that stepover means {turns} turns of the blank; cut with {MAX_TURNS}, a wider stepover", op.name));
        turns = MAX_TURNS;
    }
    let n = turns * PER_TURN;
    let xs: Vec<f64> = (0..=n).map(|k| x0 + (x1 - x0) * k as f64 / n as f64).collect();
    let goal: Vec<f64> = xs.iter().map(|x| tip(*x)).collect();
    if too_deep {
        res.warnings.push(format!("{}: the profile goes deeper than the blank allows; {CORE} in is left round the axis", op.name));
    }
    let lowest = goal.iter().copied().fold(0.0, f64::min);
    if lowest > -1e-6 {
        res.warnings.push(format!("{}: the profile is flat: nothing to turn", op.name));
        return res;
    }
    let circ = std::f64::consts::PI * dia;
    // at most MAX_PASSES passes: a stepdown too small for the depth is widened
    let down = step_down(op.stepdown, tool).max(0.005).max(-lowest / MAX_PASSES);
    let mut tp = Toolpath { name: op.name.clone(), tool: -1, feed_xy: tool.feed_xy, feed_z: tool.feed_z, moves: Vec::new(), comp: 0 };
    let mut level = 0.0;
    let mut at = pt(x0, 0.0);
    while level > lowest + 1e-9 {
        let above = level;
        level = (level - down).max(lowest);
        // the stretches (whole turns) that still have wood under the pass before
        let busy: Vec<bool> = (0..turns).map(|t| goal[t * PER_TURN..=(t + 1) * PER_TURN].iter().any(|g| *g < above - 1e-9)).collect();
        let mut t = 0;
        while t < turns {
            if !busy[t] {
                t += 1;
                continue;
            }
            let end = (t..turns).find(|k| !busy[*k]).unwrap_or(turns);
            let z = |k: usize| goal[k].max(level);
            if !tp.moves.is_empty() {
                rapid_to(&mut tp, at, Some(m.retract_z));
            }
            rapid_to(&mut tp, pt(xs[t * PER_TURN], 0.0), Some(m.retract_z));
            cut_to(&mut tp, pt(xs[t * PER_TURN], 0.0), Some(z(t * PER_TURN)));
            for turn in t..end {
                for j in 1..=PER_TURN {
                    let k = turn * PER_TURN + j;
                    cut_to(&mut tp, pt(xs[k], circ * j as f64 / PER_TURN as f64), Some(z(k)));
                }
                tp.moves.push(Move::linear(None, Some(0.0), None)); // the seam: the same place on the blank
            }
            // once more round where it stops, so the last step is cut all the way round
            cut_to(&mut tp, pt(xs[end * PER_TURN], circ), None);
            tp.moves.push(Move::linear(None, Some(0.0), None));
            at = pt(xs[end * PER_TURN], 0.0);
            t = end;
        }
    }
    if !tp.moves.is_empty() {
        rapid_to(&mut tp, at, Some(m.retract_z));
        res.paths.push(tp);
    }
    res
}

/// The drawn surface (z <= 0) at a place along the blank: how far the profile `lines` dip there below their highest
/// point `top`; beside the profile the blank is left as it is
fn drawn(lines: &[Vec<Pt>], top: f64, x: f64) -> f64 {
    let mut y = f64::MIN;
    for l in lines {
        for w in l.windows(2) {
            let (a, b) = if w[0].x <= w[1].x { (w[0], w[1]) } else { (w[1], w[0]) };
            if x >= a.x - 1e-9 && x <= b.x + 1e-9 {
                y = y.max(if b.x - a.x > 1e-12 { a.y + (b.y - a.y) * (x - a.x) / (b.x - a.x) } else { a.y.max(b.y) });
            }
        }
    }
    if y == f64::MIN { 0.0 } else { y - top }
}

/// The part the profile gives, for a picture of it: (place along the blank, radius there) at `n` + 1 places from
/// end to end, as `generate_turning` reads the drawing (not cut over the chuck and tailstock, the core left). Empty
/// without a rotary job or a profile.
pub fn radius_along(contours: &[Contour], m: &Material, n: usize) -> Vec<(f64, f64)> {
    let lines: Vec<Vec<Pt>> = contours.iter().map(|c| flatten(c, 0.002)).filter(|p| p.len() >= 2).collect();
    let (Some(dia), false) = (m.rotary, lines.is_empty() || n == 0) else { return Vec::new() };
    let top = lines.iter().flatten().map(|p| p.y).fold(f64::MIN, f64::max);
    (0..=n)
        .map(|k| m.w * k as f64 / n as f64)
        .map(|x| (x, if x < m.chuck || x > m.w - m.tail { dia / 2.0 } else { (dia / 2.0 + drawn(&lines, top, x)).max(CORE.min(dia / 2.0)) }))
        .collect()
}

/// Rotary jobs: toolpaths that come down over the chuck or the tailstock (Job Setup lengths from each end)
/// `radii`: per toolpath group, half the width of its bit (its side counts, not only its middle)
pub fn holding_issues(m: &Material, names: &[&str], radii: &[f64], paths: &[Vec<Toolpath>]) -> Vec<String> {
    let mut out = Vec::new();
    if m.rotary.is_none() || (m.chuck <= 0.0 && m.tail <= 0.0) {
        return out;
    }
    for (k, (name, gen)) in names.iter().zip(paths).enumerate() {
        let r = radii.get(k).copied().unwrap_or(0.0);
        let (mut chuck, mut tail) = (false, false);
        for tp in gen {
            let (mut x, mut z) = (f64::NAN, f64::MAX);
            for mv in &tp.moves {
                let (nx, nz) = (mv.x.unwrap_or(x), mv.z.unwrap_or(z));
                // low at either end of the move, or cutting: over the holding it would hit
                if (mv.kind != MoveType::Rapid || nz < m.safe_z - 1e-6 || z < m.safe_z - 1e-6) && nz.min(z) < m.safe_z - 1e-6 {
                    let (lo, hi) = (nx.min(x), nx.max(x));
                    chuck |= m.chuck > 0.0 && lo - r < m.chuck - 1e-6;
                    tail |= m.tail > 0.0 && hi + r > m.w - m.tail + 1e-6;
                }
                (x, z) = (nx, nz);
            }
        }
        if chuck {
            out.push(format!("{name}: comes down over the chuck (the first {} in of the blank)", m.chuck));
        }
        if tail {
            out.push(format!("{name}: comes down over the tailstock (the last {} in of the blank)", m.tail));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cam::OpType;
    use crate::geom::make_polyline;

    fn job() -> (Material, Tool, Op) {
        let m = Material { rotary: Some(3.0), w: 20.0, h: std::f64::consts::PI * 3.0, t: 3.0, safe_z: 0.8, retract_z: 0.6, chuck: 2.0, tail: 1.0, ..Default::default() };
        let t = Tool { kind: ToolType::BallNose, diameter: 0.25, stepdown: 0.2, feed_xy: 80.0, feed_z: 30.0, ..Default::default() };
        let op = Op { kind: OpType::Pocket, name: "Leg".into(), turn: true, stepover: 0.4, ..Default::default() };
        (m, t, op)
    }

    #[test]
    fn a_drawn_profile_is_turned_in_passes_without_cutting_into_it_or_the_holding() {
        let (m, t, op) = job();
        // full size at both ends, a 0.5 in deep waist with a square shoulder at x = 8 and a slope up from 12 to 16;
        // the line also runs over the chuck and tailstock, where nothing may be cut
        let line = make_polyline(&[pt(0.0, 5.0), pt(8.0, 5.0), pt(8.0, 4.5), pt(12.0, 4.5), pt(16.0, 5.0), pt(20.0, 5.0)], false);
        let surface = |x: f64| if x < 8.0 { 0.0 } else if x < 12.0 { -0.5 } else if x < 16.0 { -0.5 + (x - 12.0) / 8.0 } else { 0.0 };
        let r = generate_turning(&op, &t, &[line], &m);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let (mut pos, mut lowest, mut cut_len) = ([0.0, 0.0, 1.0], 0.0f64, 0.0);
        for mv in &r.paths[0].moves {
            let to = [mv.x.unwrap_or(pos[0]), mv.y.unwrap_or(pos[1]), mv.z.unwrap_or(pos[2])];
            if to[2] < 0.6 - 1e-9 || pos[2] < 0.6 - 1e-9 {
                assert!(to[0] >= 2.125 - 1e-9 && to[0] <= 18.875 + 1e-9, "down at x {} (chuck to 2, tailstock from 19, the bit 1/8 in clear of both)", to[0]);
            }
            if mv.kind == MoveType::Linear && to[2] < 0.0 {
                // the ball never inside the drawn shape: checked at its tip and half way out
                for u in [-0.0625, 0.0, 0.0625] {
                    assert!(to[2] + t.ball_rise(u) >= surface(to[0] + u) - 2e-3, "gouge at x {} z {}", to[0] + u, to[2]);
                }
                lowest = lowest.min(to[2]);
                cut_len += (to[0] - pos[0]).abs();
            }
            assert!(mv.kind != MoveType::Rapid || to[2] >= 0.6 - 1e-9, "a rapid below the travel height");
            pos = to;
        }
        assert!((lowest + 0.5).abs() < 1e-6, "the waist is cut to its drawn depth: {lowest}");
        // three passes (0.2, 0.4, 0.5): the deeper ones run only where wood is left, so less than 3 × the whole way
        assert!(cut_len > 8.0 && cut_len < 3.0 * 8.5, "{cut_len}");
        // the waist's floor is finished: some move sits on it in its middle
        assert!(r.paths[0].moves.iter().any(|mv| mv.x.is_some_and(|x| (x - 10.0).abs() < 0.2) && mv.z.is_some_and(|z| (z + 0.5).abs() < 1e-6)));
    }

    #[test]
    fn turning_needs_a_rotary_job_and_a_profile_and_flags_cuts_over_the_chuck() {
        let (m, t, op) = job();
        let flat = Material { rotary: None, ..m.clone() };
        let line = make_polyline(&[pt(3.0, 5.0), pt(6.0, 4.8), pt(9.0, 5.0)], false);
        assert!(generate_turning(&op, &t, std::slice::from_ref(&line), &flat).warnings[0].contains("rotary job"));
        assert!(generate_turning(&op, &t, &[], &m).warnings[0].contains("side profile"));
        // the picture of the part: full size over the chuck, 0.2 smaller at the profile's dip
        let r = radius_along(std::slice::from_ref(&line), &m, 200);
        assert!((r[10].1 - 1.5).abs() < 1e-9 && (r[60].1 - 1.3).abs() < 1e-9 && radius_along(&[], &m, 200).is_empty(), "{:?} {:?}", r[10], r[60]);
        let ok = generate_turning(&op, &t, &[line], &m);
        assert!(holding_issues(&m, &["Leg"], &[0.125], std::slice::from_ref(&ok.paths)).is_empty());
        // another toolpath that cuts at x = 1 (inside the chuck's 2 in) and at x = 19.5 (the tailstock's last inch)
        let mut bad = Toolpath { name: "Groove".into(), tool: -1, feed_xy: 50.0, feed_z: 20.0, moves: Vec::new(), comp: 0 };
        rapid_to(&mut bad, pt(1.0, 0.0), Some(0.6));
        cut_to(&mut bad, pt(1.0, 0.0), Some(-0.1));
        cut_to(&mut bad, pt(19.5, 0.0), Some(-0.1));
        let issues = holding_issues(&m, &["Groove"], &[0.125], &[vec![bad]]);
        assert!(issues.len() == 2 && issues[0].contains("chuck") && issues[1].contains("tailstock"), "{issues:?}");
    }
}
