//! CAMaster / WinCNC single-tool writer, byte-compatible with Vectric's "CAMaster Single Tool" post
//! (C++ `post/wincnc.cpp`). Checked byte for byte against `reference/*.tap` in tests/wincnc_golden.rs.

use crate::job::{Job, Move, MoveType};

/// Fixed decimals with C's rounding of `round(v * 10^dec) / 10^dec`, never "-0.000".
pub fn num(v: f64, dec: usize) -> String {
    let k = 10f64.powi(dec as i32);
    let r = (v * k).round() / k;
    format!("{:.*}", dec, if r == 0.0 { 0.0 } else { r })
}

struct Out {
    s: String,
}

impl Out {
    /// WinCNC samples: leading space, CRLF
    fn line(&mut self, l: &str) {
        self.s.push(' ');
        self.s.push_str(l);
        self.s.push_str("\r\n");
    }
    fn blank(&mut self) {
        self.line("");
    }
}

fn move_line(m: &Move) -> String {
    let mut l = String::from(match m.kind {
        MoveType::Rapid => "G0",
        MoveType::Linear => "G1",
        MoveType::ArcCw => "G2",
        MoveType::ArcCcw => "G3",
    });
    let z_only_rapid = m.kind == MoveType::Rapid && m.z.is_some() && m.x.is_none() && m.y.is_none();
    if let Some(x) = m.x {
        l += &format!(" X{}", num(x, 6));
    }
    if let Some(y) = m.y {
        l += &format!(" Y{}", num(y, 6));
    }
    if let Some(z) = m.z {
        l += &format!(" Z{}", num(z, if z_only_rapid { 3 } else { 6 })); // reference quirk: "G0 Z0.800"
    }
    if matches!(m.kind, MoveType::ArcCw | MoveType::ArcCcw) {
        l += &format!(" I{} J{}", num(m.i, 6), num(m.j, 6));
    }
    l
}

pub fn post_wincnc(job: &Job) -> String {
    let mut o = Out { s: String::new() };
    let rpm = format!("S{} [ SET SPINDLE SPEED RPM ]", job.rpm);
    // ponytail: copied from the reference files; drop once WinCNC proves it ignores it
    o.s.push_str("[VECTRIC POST REVISION]\r\n[5BD1538CDA767BCF1F9CE0D7D7B91137]\r\n");
    o.line("[VECTRIC CNC OUTPUT FOR CAMASTER SINGLE TOOL ]");
    o.line("[MACHINES WITH WINCNC CONTROL ONLY]");
    o.blank();
    o.line(&format!("[FILENAME: {}]", job.filename));
    o.line(&format!("[{}]", job.timestamp));
    o.blank();
    o.line(&format!("[MATERIAL size: {}]", job.material));
    o.line(&format!("[Z Zero Position: {}]", job.z_zero));
    for n in &job.notes {
        o.line(&format!("[{n}]"));
    }
    o.blank();
    o.line("G90 [ABSOLUTE MODE]");
    o.blank();
    o.line("M5 [SPINDLE OFF]");
    o.line("G53 Z0 [LIFT Z TO TOP]");
    o.blank();
    o.line(&format!("[{}]", job.tool));
    o.blank();
    o.line(&rpm);
    o.line("M3 [SPINDLE ON]");
    o.line(" ");
    o.line("G53 Z0 [LIFT Z TO TOP]");
    o.blank();
    for (k, tp) in job.toolpaths.iter().enumerate() {
        let first = k == 0;
        o.line(&format!("[TOOLPATH NAME: {}]", tp.name));
        if !first {
            o.line(&rpm);
        }
        o.line(&format!("F{} XY [SET FEEDRATE FOR X AND Y]", num(tp.feed_xy, 0)));
        o.line(&format!("F{} Z [SET FEEDRATE FOR Z]", num(tp.feed_z, 0)));
        if !first {
            o.blank();
        }
        for m in &tp.moves {
            o.line(&move_line(m));
        }
        o.blank();
        o.blank();
    }
    // the reference has no blank lines between the last move and the footer
    let n = o.s.len();
    o.s.truncate(n.saturating_sub(6)); // two trailing blank lines, 3 bytes each: " \r\n"
    o.line("G53 Z0");
    o.line("M5"); // safety: the spindle always stops at the end
    o.s
}

#[cfg(test)]
mod tests {
    use super::num;
    #[test]
    fn num_rounds_like_c_and_never_prints_negative_zero() {
        assert_eq!(num(0.8, 3), "0.800");
        assert_eq!(num(-0.0000001, 6), "0.000000");
        assert_eq!(num(1.6755, 3), "1.676");
        assert_eq!(num(432.0, 0), "432");
    }
}
