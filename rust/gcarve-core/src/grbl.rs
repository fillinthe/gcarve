//! Machine control, protocol side (C++ `control/grbl.*`): a grblHAL-compatible simulated controller and the streaming
//! client used by GCarve Control. No I/O and no threads: the host moves bytes and steps the simulator with its own clock.

use std::collections::VecDeque;

use crate::gcode::{interpret_line, motion_points, GMotion, GState, Vec3};
use crate::job::MoveType;

// Realtime command bytes (Grbl 1.1 / grblHAL)
pub const STATUS: u8 = b'?';
pub const HOLD: u8 = b'!';
pub const RESUME: u8 = b'~';
pub const RESET: u8 = 0x18;
pub const JOG_CANCEL: u8 = 0x85;
pub const FEED_100: u8 = 0x90;
pub const FEED_PLUS_10: u8 = 0x91;
pub const FEED_MINUS_10: u8 = 0x92;
/// spindle (laser power) overrides
pub const SPINDLE_100: u8 = 0x99;
pub const SPINDLE_PLUS_10: u8 = 0x9A;
pub const SPINDLE_MINUS_10: u8 = 0x9B;
/// controller serial RX buffer (character-counting protocol)
pub const RX_BUFFER: usize = 128;

const fn v3(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    v3(a.x - b.x, a.y - b.y, a.z - b.z)
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    v3(a.x + b.x, a.y + b.y, a.z + b.z)
}
fn len3(a: Vec3, b: Vec3) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt()
}
fn fmt3(p: Vec3) -> String {
    format!("{:.4},{:.4},{:.4}", p.x, p.y, p.z)
}
/// C atof: leading number of a string, 0 when there is none
fn atof(s: &str) -> f64 {
    let s = s.trim_start();
    let b = s.as_bytes();
    let mut k = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    while k < b.len() && (b[k].is_ascii_digit() || b[k] == b'.') {
        k += 1;
    }
    let mut t = &s[..k];
    while !t.is_empty() && t.parse::<f64>().is_err() {
        t = &t[..t.len() - 1];
    }
    t.parse().unwrap_or(0.0)
}

#[derive(Clone, Debug, PartialEq)]
pub struct MachineStatus {
    /// Idle Run Hold:0 Jog Alarm Home Door Check
    pub state: String,
    pub mpos: Vec3,
    pub wco: Vec3,
    pub feed: f64,
    pub rpm: f64,
    pub feed_ov: i32,
    /// spindle / laser power override, %
    pub spindle_ov: i32,
    pub valid: bool,
}

impl Default for MachineStatus {
    fn default() -> Self {
        MachineStatus { state: "Unknown".into(), mpos: Vec3::default(), wco: Vec3::default(), feed: 0.0, rpm: 0.0, feed_ov: 100, spindle_ov: 100, valid: false }
    }
}

impl MachineStatus {
    pub fn wpos(&self) -> Vec3 {
        sub(self.mpos, self.wco)
    }
}

/// "<Idle|MPos:1.000,2.000,0.000|FS:0,0|WCO:...|Ov:100,100,100>" -> fields present in the report (others kept)
pub fn parse_status(line: &str, st: &mut MachineStatus) -> bool {
    if line.len() < 3 || !line.starts_with('<') || !line.ends_with('>') {
        return false;
    }
    let mut fields = line[1..line.len() - 1].split('|');
    st.state = fields.next().unwrap_or("").to_string();
    for f in fields {
        let Some((k, rest)) = f.split_once(':') else { continue };
        let v: Vec<f64> = rest.split(',').map(atof).collect();
        match k {
            "MPos" if v.len() >= 3 => st.mpos = v3(v[0], v[1], v[2]),
            "WPos" if v.len() >= 3 => st.mpos = add(v3(v[0], v[1], v[2]), st.wco),
            "WCO" if v.len() >= 3 => st.wco = v3(v[0], v[1], v[2]),
            "FS" if v.len() >= 2 => {
                st.feed = v[0];
                st.rpm = v[1];
            }
            "F" if !v.is_empty() => st.feed = v[0],
            "Ov" if !v.is_empty() => {
                st.feed_ov = v[0] as i32;
                if v.len() >= 3 {
                    st.spindle_ov = v[2] as i32;
                }
            }
            _ => {}
        }
    }
    st.valid = true;
    true
}

/// One "$N=value" settings line (reply to "$$"); None for anything else.
pub fn parse_setting(line: &str) -> Option<(i32, f64)> {
    let b = line.as_bytes();
    if b.len() < 4 || b[0] != b'$' || !b[1].is_ascii_digit() {
        return None;
    }
    let (n, v) = line[1..].split_once('=')?;
    Some((atof(n) as i32, atof(v)))
}

/// Strips comments/whitespace, uppercases; "" for nothing to send.
pub fn clean_line(raw: &str) -> String {
    let mut out = String::new();
    let mut paren = 0;
    for c in raw.chars() {
        match c {
            ';' => break,
            '(' => paren += 1,
            ')' => paren = (paren - 1).max(0),
            _ if paren > 0 || matches!(c, '\r' | '\n' | ' ' | '\t' | '%') => {}
            _ => out.push(c.to_ascii_uppercase()),
        }
    }
    out
}

/// Human text for "error:N" (grbl 1.1 / grblHAL numbering); empty when unknown.
pub fn error_text(c: i32) -> &'static str {
    match c {
        1 => "G-code words consist of a letter and a value; letter was not found",
        2 => "Bad number format",
        3 => "Unrecognized $ system command",
        4 => "Negative value for a setting that needs a positive one",
        5 => "Homing is not enabled in the settings",
        7 => "EEPROM read failed",
        8 => "Command needs the machine to be idle",
        9 => "G-code locked out during alarm or jog",
        10 => "Soft limits need homing enabled",
        11 => "Line too long (over 80 characters)",
        14 => "Build info or startup line too long",
        15 => "Jog target exceeds the machine travel",
        16 => "Invalid jog command",
        17 => "Laser mode needs PWM output",
        20 => "Unsupported or invalid G-code command",
        21 => "More than one G-code command from the same group in one line",
        22 => "Feed rate not set or undefined",
        23 => "G-code command needs an integer value",
        24 => "Two G-code commands both need the same axis words",
        25 => "A G-code word was repeated in the line",
        26 => "A G-code command needs axis words and none were given",
        27 => "Line number is out of range",
        28 => "A G-code command needs a value that is missing",
        29 => "G59.x work coordinate systems are not supported",
        30 => "G53 is only valid with G0 or G1",
        31 => "Axis words found where none are allowed",
        32 => "Arc needs at least one in-plane axis word",
        33 => "Motion target is invalid",
        34 => "Arc radius cannot reach the target",
        35 => "Arc needs an in-plane offset (I/J)",
        36 => "Unused value words in the line",
        37 => "Tool length offset only on the Z axis",
        38 => "Tool number greater than the maximum",
        _ => "",
    }
}

/// Human text for "ALARM:N"; empty when unknown.
pub fn alarm_text(c: i32) -> &'static str {
    match c {
        1 => "Hard limit hit: position lost. Re-home.",
        2 => "Soft limit: a move would leave the machine travel. Position kept; unlock and check the job.",
        3 => "Reset while moving: position lost. Re-home.",
        4 => "Probe not in the expected state before probing",
        5 => "Probe did not touch within the move",
        6 => "Homing was reset / cancelled",
        7 => "Safety door opened during homing",
        8 => "Homing: limit switch still active after pull-off",
        9 => "Homing: limit switch not found",
        10 => "E-stop pressed",
        11 => "Homing required",
        12 => "Limit switch active at startup",
        13 => "Spindle at speed timeout",
        14 => "Motor fault",
        _ => "",
    }
}

// ---- simulated controller ------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct SimConfig {
    /// machine travel, inches (Panther 5x8 working area; Z down from home)
    pub travel: Vec3,
    /// max rate
    pub rapid_ipm: f64,
    /// starts in Alarm until $H or $X, like a homed machine
    pub homing_required: bool,
    /// machine Z where the touch plate / material top is (G38.2 trips here); NaN = no plate
    pub probe_z: f64,
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig { travel: v3(60.0, 96.0, 6.0), rapid_ipm: 1000.0, homing_required: true, probe_z: -4.0 }
    }
}

#[derive(Clone, Copy, Debug)]
struct Seg {
    a: Vec3,
    b: Vec3,
    ipm: f64,
    spindle: bool,
    rpm: f64,
    block: i32,
    probe: bool,
}

/// grblHAL plans ~16 blocks ahead
const PLANNER: usize = 16;

pub struct GrblSim {
    cfg: SimConfig,
    st: MachineStatus,
    gs: GState,
    rx: Vec<u8>,
    out: String,
    queue: VecDeque<Seg>,
    /// lines received while the planner was full: parsed (and acked) as it drains
    pending: VecDeque<String>,
    /// planner block id of the line being parsed
    block_seq: i32,
    /// inches along queue.front()
    seg_done: f64,
    dwell: f64,
    hold: bool,
    alarm: bool,
    homing: bool,
    jogging: bool,
    probe_hit: bool,
    probe_alarm_on_miss: bool,
}

impl Default for GrblSim {
    fn default() -> Self {
        Self::new(SimConfig::default())
    }
}

impl GrblSim {
    pub fn new(cfg: SimConfig) -> Self {
        let mut s = GrblSim {
            cfg,
            st: MachineStatus::default(),
            gs: GState::default(),
            rx: Vec::new(),
            out: String::new(),
            queue: VecDeque::new(),
            pending: VecDeque::new(),
            block_seq: 0,
            seg_done: 0.0,
            dwell: 0.0,
            hold: false,
            alarm: false,
            homing: false,
            jogging: false,
            probe_hit: false,
            probe_alarm_on_miss: true,
        };
        s.reset(cfg.homing_required);
        s
    }

    pub fn status(&self) -> &MachineStatus {
        &self.st
    }
    /// the running move's spindle state
    pub fn spindle_on(&self) -> bool {
        self.queue.front().map_or(self.gs.spindle_on, |s| s.spindle)
    }
    pub fn set_probe_z(&mut self, z: f64) {
        self.cfg.probe_z = z;
    }
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
    /// to the host (lines end with "\r\n")
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.out)
    }

    fn reset(&mut self, alarm: bool) {
        self.queue.clear();
        self.rx.clear();
        self.pending.clear();
        self.seg_done = 0.0;
        self.dwell = 0.0;
        self.hold = false;
        self.homing = false;
        self.jogging = false;
        self.gs.spindle_on = false;
        self.gs.motion = 0;
        self.gs.absolute = true;
        self.gs.inches = true;
        self.gs.program_end = false;
        self.gs.pos = self.st.wpos();
        self.st.feed_ov = 100;
        self.alarm = alarm;
        self.st.state = if alarm { "Alarm" } else { "Idle" }.into();
        self.out += "\r\nGrbl 1.1f ['$' for help]\r\n";
        if alarm {
            self.out += "[MSG:'$H'|'$X' to unlock]\r\n";
        }
    }

    fn raise_alarm(&mut self, code: i32, why: &str) {
        self.queue.clear();
        self.pending.clear();
        self.seg_done = 0.0;
        self.gs.spindle_on = false;
        self.alarm = true;
        self.st.state = "Alarm".into();
        self.out += &format!("ALARM:{code}\r\n[MSG:{why}]\r\n");
    }

    fn report(&mut self) {
        let moving = !self.queue.is_empty() && !self.hold;
        let ov = if self.jogging || self.homing { 1.0 } else { f64::from(self.st.feed_ov) / 100.0 };
        let feed = if moving { self.queue[0].ipm * ov } else { 0.0 };
        let rpm = match self.queue.front() {
            Some(s) if s.spindle => s.rpm,
            Some(_) => 0.0,
            None if self.gs.spindle_on => self.gs.rpm,
            None => 0.0,
        };
        self.out += &format!("<{}|MPos:{}|FS:{feed:.0},{rpm:.0}|WCO:{}|Ov:{},100,{}>\r\n", self.st.state, fmt3(self.st.mpos), fmt3(self.st.wco), self.st.feed_ov, self.st.spindle_ov);
    }

    /// from the host
    pub fn receive(&mut self, bytes: &[u8]) {
        for &c in bytes {
            match c {
                STATUS => self.report(),
                RESET => {
                    // reset while moving loses position: grbl raises ALARM:3 and stays locked
                    let moving = !self.queue.is_empty() && !self.jogging;
                    self.reset(self.alarm || moving);
                    if moving {
                        self.out += "ALARM:3\r\n[MSG:Reset while in motion. Re-home or unlock]\r\n";
                    }
                }
                HOLD => {
                    if !self.queue.is_empty() || self.st.state == "Run" || self.st.state == "Jog" {
                        self.hold = true;
                        self.st.state = if self.jogging { "Idle" } else { "Hold:0" }.into();
                        if self.jogging {
                            self.queue.clear();
                            self.jogging = false;
                        }
                    }
                }
                RESUME => {
                    if self.hold {
                        self.hold = false;
                        self.st.state = if self.queue.is_empty() { "Idle" } else { "Run" }.into();
                    }
                }
                JOG_CANCEL => {
                    if self.jogging {
                        self.queue.clear();
                        self.seg_done = 0.0;
                        self.jogging = false;
                        self.st.state = "Idle".into();
                    }
                }
                FEED_100 => self.st.feed_ov = 100,
                FEED_PLUS_10 => self.st.feed_ov = (self.st.feed_ov + 10).min(200),
                FEED_MINUS_10 => self.st.feed_ov = (self.st.feed_ov - 10).max(10),
                SPINDLE_100 => self.st.spindle_ov = 100,
                SPINDLE_PLUS_10 => self.st.spindle_ov = (self.st.spindle_ov + 10).min(200),
                SPINDLE_MINUS_10 => self.st.spindle_ov = (self.st.spindle_ov - 10).max(10),
                b'\n' => {
                    let l = String::from_utf8_lossy(&std::mem::take(&mut self.rx)).into_owned();
                    if !self.pending.is_empty() || self.planner_blocks() >= PLANNER {
                        self.pending.push_back(l); // no "ok" until it fits the planner
                    } else {
                        self.line(&l);
                    }
                }
                b'\r' => {}
                _ => {
                    if self.rx.len() < RX_BUFFER {
                        self.rx.push(c);
                    }
                }
            }
        }
    }

    fn to_machine(&self, w: Vec3) -> Vec3 {
        add(w, self.st.wco)
    }
    fn to_work(&self, m: Vec3) -> Vec3 {
        sub(m, self.st.wco)
    }

    /// machine-coordinate points of a move starting at `cursor` (G53 axes left out stay where the machine is)
    fn machine_points(&self, m: &GMotion, cursor: Vec3) -> Vec<Vec3> {
        if m.machine_coords {
            let pick = |t: f64, c: f64| if t.is_nan() { c } else { t };
            return vec![cursor, v3(pick(m.to.x, cursor.x), pick(m.to.y, cursor.y), pick(m.to.z, cursor.z))];
        }
        motion_points(m, 0.002).into_iter().map(|p| self.to_machine(p)).collect()
    }

    /// where the planner ends (machine coordinates)
    fn cursor(&self) -> Vec3 {
        self.queue.back().map_or(self.st.mpos, |s| s.b)
    }

    fn exec_motions(&mut self, ms: &[GMotion]) {
        for m in ms {
            let pts = self.machine_points(m, self.cursor());
            let ipm = if m.kind == MoveType::Rapid { self.cfg.rapid_ipm } else { m.feed.min(self.cfg.rapid_ipm) };
            for w in pts.windows(2) {
                self.queue.push_back(Seg { a: w[0], b: w[1], ipm, spindle: m.spindle_on, rpm: self.gs.rpm, block: self.block_seq, probe: false });
            }
        }
    }

    /// G-code blocks with motion still queued
    fn planner_blocks(&self) -> usize {
        let mut n = 0;
        let mut last = -1;
        for s in &self.queue {
            if s.block != last {
                n += 1;
                last = s.block;
            }
        }
        n
    }

    fn drain_pending(&mut self) {
        while self.planner_blocks() < PLANNER && !self.alarm {
            let Some(l) = self.pending.pop_front() else { break };
            self.line(&l);
        }
    }

    fn line(&mut self, raw: &str) {
        self.block_seq += 1;
        let l = clean_line(raw);
        if l.is_empty() {
            self.out += "ok\r\n";
            return;
        }
        if l.starts_with('$') {
            self.system(&l);
            return;
        }
        if self.alarm {
            self.out += "error:9\r\n"; // locked until $H / $X
            return;
        }
        // probe toward the plate: stops where it trips. ponytail: the C++ sim only took G38.x at the start of the line,
        // so the app's own "G91 G38.2 Z.." got error:20; here modal words may come first.
        if let Some(k) = l.find("G38.2").or_else(|| l.find("G38.3")) {
            self.probe(&format!("{}G1{}", &l[..k], &l[k + 5..]), l[k..].starts_with("G38.2"));
            return;
        }
        let mut ms = Vec::new();
        let before = self.gs;
        if self.queue.is_empty() {
            self.gs.pos = self.st.wpos();
        }
        let eff = match interpret_line(&l, &mut self.gs, &mut ms, 0) {
            Ok(e) => e,
            Err(e) => {
                self.gs = before;
                self.out += &format!("error:20\r\n[MSG:{e}]\r\n");
                return;
            }
        };
        if let Some(w) = eff.set_work {
            // G10 L20 P1: work zero so the current machine position reads `w`
            self.st.wco = sub(self.st.mpos, w);
        }
        let mut cur = self.cursor();
        let t = self.cfg.travel;
        for m in &ms {
            // soft limits: the whole move must stay inside the machine
            let mp = self.machine_points(m, cur);
            cur = *mp.last().unwrap();
            if mp.iter().any(|q| q.x < -1e-6 || q.y < -1e-6 || q.x > t.x + 1e-6 || q.y > t.y + 1e-6 || q.z > 1e-6 || q.z < -t.z - 1e-6) {
                self.raise_alarm(2, "Soft limit: move outside the machine travel");
                return;
            }
        }
        self.exec_motions(&ms);
        if ms.iter().any(|m| m.machine_coords) {
            if let Some(b) = self.queue.back().map(|s| s.b) {
                self.gs.pos = self.to_work(b);
            }
        }
        self.dwell += eff.dwell_sec.unwrap_or(0.0);
        if let Some(last) = ms.last() {
            if !self.hold {
                self.st.state = "Run".into();
            }
            self.st.feed = if last.feed > 0.0 { last.feed } else { self.cfg.rapid_ipm };
        }
        self.out += "ok\r\n";
    }

    fn system(&mut self, l: &str) {
        if l == "$H" {
            if !self.queue.is_empty() {
                self.out += "error:8\r\n";
                return;
            }
            self.alarm = false;
            self.homing = true;
            self.st.state = "Home".into();
            let from = self.st.mpos;
            self.block_seq += 1;
            let ipm = self.cfg.rapid_ipm / 2.0;
            let top = v3(from.x, from.y, 0.0);
            self.queue.push_back(Seg { a: from, b: top, ipm, spindle: false, rpm: 0.0, block: self.block_seq, probe: false });
            self.queue.push_back(Seg { a: top, b: Vec3::default(), ipm, spindle: false, rpm: 0.0, block: self.block_seq, probe: false });
            self.out += "ok\r\n"; // grbl answers after homing; the host waits for Idle either way
            return;
        }
        if l == "$X" {
            if self.alarm {
                self.alarm = false;
                self.st.state = "Idle".into();
                self.out += "[MSG:Caution: Unlocked]\r\n";
            }
            self.out += "ok\r\n";
            return;
        }
        if let Some(cmd) = l.strip_prefix("$J=") {
            if self.alarm {
                self.out += "error:9\r\n";
                return;
            }
            let mut js = self.gs;
            js.pos = self.st.wpos();
            js.motion = 1;
            let mut ms = Vec::new();
            if interpret_line(cmd, &mut js, &mut ms, 0).is_err() || ms.is_empty() {
                self.out += "error:16\r\n";
                return;
            }
            let t = self.cfg.travel;
            for m in &mut ms {
                // jogs clip at soft limits instead of alarming
                let p = *self.machine_points(m, self.cursor()).last().unwrap();
                let p = v3(p.x.clamp(0.0, t.x), p.y.clamp(0.0, t.y), p.z.clamp(-t.z, 0.0));
                m.to = if m.machine_coords { p } else { self.to_work(p) };
                if !m.machine_coords {
                    m.from = self.st.wpos();
                }
                m.kind = MoveType::Linear;
            }
            self.exec_motions(&ms);
            self.jogging = true;
            self.st.state = "Jog".into();
            self.st.feed = ms[0].feed;
            self.out += "ok\r\n";
            return;
        }
        if l == "$$" {
            // settings report: travel in mm, like grbl ($13=0 = metric reports)
            let t = self.cfg.travel;
            self.out += &format!("$110={:.3}\r\n$130={:.3}\r\n$131={:.3}\r\n$132={:.3}\r\n", self.cfg.rapid_ipm * 25.4, t.x * 25.4, t.y * 25.4, t.z * 25.4);
        }
        self.out += "ok\r\n"; // settings queries etc.
    }

    fn probe(&mut self, as_g1: &str, alarm_on_miss: bool) {
        let mut ps = self.gs;
        ps.motion = 1;
        ps.pos = self.to_work(self.cursor());
        let mut ms = Vec::new();
        if interpret_line(as_g1, &mut ps, &mut ms, 0).is_err() || ms.len() != 1 {
            self.out += "error:20\r\n";
            return;
        }
        (self.gs.absolute, self.gs.inches) = (ps.absolute, ps.inches); // modal words on the probe line stick
        let a = self.cursor();
        let b = *self.machine_points(&ms[0], a).last().unwrap();
        if b.z < -self.cfg.travel.z - 1e-6 || b.z > 1e-6 {
            self.raise_alarm(2, "Soft limit: probe move outside the machine travel");
            return;
        }
        let pz = self.cfg.probe_z;
        let hit = !pz.is_nan() && b.z <= pz + 1e-9 && a.z > pz;
        let stop = if hit {
            let t = (a.z - pz) / (a.z - b.z);
            v3(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, pz)
        } else {
            b
        };
        self.queue.push_back(Seg { a, b: stop, ipm: ms[0].feed.min(self.cfg.rapid_ipm), spindle: self.gs.spindle_on, rpm: self.gs.rpm, block: self.block_seq, probe: true });
        self.probe_hit = hit;
        self.probe_alarm_on_miss = alarm_on_miss;
        self.gs.pos = self.to_work(stop);
        self.st.state = "Run".into();
        self.out += "ok\r\n";
    }

    /// advance the motion
    pub fn tick(&mut self, mut seconds: f64) {
        if self.hold || self.alarm {
            return;
        }
        self.drain_pending();
        if self.dwell > 0.0 {
            let d = self.dwell.min(seconds);
            self.dwell -= d;
            seconds -= d;
        }
        while seconds > 1e-9 && !self.queue.is_empty() {
            let s = self.queue[0];
            let len = len3(s.a, s.b);
            let ipm = s.ipm * if self.jogging || self.homing { 1.0 } else { f64::from(self.st.feed_ov) / 100.0 };
            let can = ipm / 60.0 * seconds;
            let left = len - self.seg_done;
            if can >= left {
                seconds -= left / (ipm / 60.0);
                self.st.mpos = s.b;
                self.seg_done = 0.0;
                self.queue.pop_front();
                if s.probe {
                    let p = self.st.mpos;
                    self.out += &format!("[PRB:{:.4},{:.4},{:.4}:{}]\r\n", p.x, p.y, p.z, i32::from(self.probe_hit));
                    if !self.probe_hit && self.probe_alarm_on_miss {
                        self.raise_alarm(5, "Probe did not touch");
                        return;
                    }
                }
            } else {
                self.seg_done += can;
                let t = if len > 0.0 { self.seg_done / len } else { 1.0 };
                self.st.mpos = v3(s.a.x + (s.b.x - s.a.x) * t, s.a.y + (s.b.y - s.a.y) * t, s.a.z + (s.b.z - s.a.z) * t);
                seconds = 0.0;
            }
        }
        self.drain_pending();
        if self.queue.is_empty() && self.pending.is_empty() && self.dwell <= 0.0 && matches!(self.st.state.as_str(), "Run" | "Jog" | "Home") {
            if self.homing {
                self.homing = false;
                self.st.mpos = Vec3::default();
            }
            self.jogging = false;
            self.st.state = "Idle".into();
        }
    }
}

// ---- streaming client ----------------------------------------------------------------------------------------------

/// Character-counting streaming: send as many lines as fit in the controller's RX buffer, one "ok"/"error" per line.
#[derive(Default)]
pub struct GrblStreamer {
    lines: Vec<String>,
    src: Vec<i32>,
    in_flight: VecDeque<usize>,
    next: usize,
    acked: usize,
    buf_used: usize,
    running: bool,
    error: String,
}

impl GrblStreamer {
    /// comments/blank lines are dropped before sending
    pub fn load<S: AsRef<str>>(&mut self, lines: &[S]) {
        *self = GrblStreamer::default();
        for (i, l) in lines.iter().enumerate() {
            let c = clean_line(l.as_ref());
            if !c.is_empty() {
                self.lines.push(c);
                self.src.push(i as i32 + 1);
            }
        }
    }
    /// bytes to write now (may be empty)
    pub fn next_chunk(&mut self) -> String {
        let mut out = String::new();
        while self.running && self.next < self.lines.len() {
            let need = self.lines[self.next].len() + 1;
            if self.buf_used + need > RX_BUFFER - 1 {
                break;
            }
            out += &self.lines[self.next];
            out.push('\n');
            self.buf_used += need;
            self.in_flight.push_back(self.next);
            self.next += 1;
        }
        out
    }
    /// one response line from the controller; true when it was an ok/error for a streamed line
    pub fn on_response(&mut self, line: &str) -> bool {
        let err = line.starts_with("error");
        if line != "ok" && !err {
            return false;
        }
        let Some(i) = self.in_flight.pop_front() else { return false };
        self.buf_used -= self.lines[i].len() + 1;
        self.acked += 1;
        if err {
            self.error = format!("line {} ({}): {line}", self.src[i], self.lines[i]);
            self.running = false;
        }
        if self.done() {
            self.running = false;
        }
        true
    }
    /// forget queued lines (after a reset)
    pub fn stop(&mut self) {
        self.in_flight.clear();
        self.buf_used = 0;
        self.running = false;
    }
    pub fn running(&self) -> bool {
        self.running
    }
    pub fn done(&self) -> bool {
        self.acked == self.lines.len() && !self.lines.is_empty()
    }
    pub fn acked(&self) -> usize {
        self.acked
    }
    pub fn total(&self) -> usize {
        self.lines.len()
    }
    /// 1-based line in the original program for the last acked line
    pub fn source_line(&self) -> i32 {
        if self.acked > 0 {
            self.src[self.acked - 1]
        } else {
            0
        }
    }
    pub fn error(&self) -> &str {
        &self.error
    }
    pub fn start(&mut self) {
        self.running = !self.lines.is_empty() && !self.done();
        self.error.clear();
    }
    pub fn pause(&mut self) {
        self.running = false;
    }
}

/// Resume a program at `line` (1-based, the first line to run). Replays the modal state of lines 1..line-1 and returns
/// the safe re-entry block: units/absolute, lift to machine Z0 (G53), spindle at the program's speed + spin-up dwell,
/// rapid over the restart XY, feed down to the restart Z at a slow plunge, restore the feed and modal motion. Then
/// stream lines line..end. Empty when the line is out of range or the state can't be rebuilt.
pub fn resume_preamble<S: AsRef<str>>(program: &[S], line: usize, plunge_ipm: f64) -> Vec<String> {
    if line < 2 || line > program.len() {
        return Vec::new();
    }
    let mut st = GState::default();
    let mut ms = Vec::new();
    for (i, l) in program[..line - 1].iter().enumerate() {
        match interpret_line(l.as_ref(), &mut st, &mut ms, i as i32 + 1) {
            Ok(e) if e.set_work.is_none() => {}
            _ => return Vec::new(), // parse error, or the program re-zeroes itself before this line
        }
    }
    let u = if st.inches { 1.0 } else { 25.4 };
    let mut out = vec![if st.inches { "G20" } else { "G21" }.to_string(), "G90".into(), "G53 G0 Z0".into()]; // up before anything moves sideways
    if st.spindle_on {
        out.push(format!("M3 S{:.0}", if st.rpm > 0.0 { st.rpm } else { 12000.0 }));
        out.push("G4 P3".into()); // spin up before touching material
    }
    out.push(format!("G0 X{:.4} Y{:.4}", st.pos.x * u, st.pos.y * u));
    out.push(format!("G0 Z{:.4}", st.pos.z.max(0.0) * u + 0.2 * u));
    out.push(format!("G1 Z{:.4} F{:.1}", st.pos.z * u, plunge_ipm * u));
    if st.feed > 0.0 {
        out.push(format!("F{:.1}", st.feed * u));
    }
    out.push(format!("G{}", st.motion)); // the modal motion the next lines expect
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcode::parse_program;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    /// pump bytes both ways and advance time until idle (tests/control_test.cpp Rig)
    struct Rig {
        sim: GrblSim,
        st: GrblStreamer,
        ms: MachineStatus,
        inbuf: String,
        log: String,
    }
    impl Rig {
        fn new() -> Self {
            Rig { sim: GrblSim::default(), st: GrblStreamer::default(), ms: MachineStatus::default(), inbuf: String::new(), log: String::new() }
        }
        fn pump(&mut self, dt: f64) {
            let c = self.st.next_chunk();
            self.sim.receive(c.as_bytes());
            self.sim.receive(&[STATUS]);
            self.sim.tick(dt);
            self.inbuf += &self.sim.take_output();
            while let Some(nl) = self.inbuf.find('\n') {
                let l: String = self.inbuf.drain(..=nl).collect();
                let l = l.trim_end_matches(['\r', '\n']);
                if l.is_empty() {
                    continue;
                }
                self.log += l;
                self.log.push('\n');
                if !parse_status(l, &mut self.ms) {
                    self.st.on_response(l);
                }
            }
        }
        fn send(&mut self, line: &str) {
            self.sim.receive(format!("{line}\n").as_bytes());
            self.pump(0.0);
        }
        fn run_until_idle(&mut self) {
            for _ in 0..200_000 {
                self.pump(0.05);
                if !self.st.running() && self.ms.state == "Idle" {
                    break;
                }
            }
        }
        fn has(&self, s: &str) -> bool {
            self.log.contains(s)
        }
    }

    #[test]
    fn sim_end_to_end() {
        let mut r = Rig::new();
        // starts locked like a homed machine
        r.send("G0 X1");
        assert!(r.has("error:9"));
        r.send("$H");
        r.run_until_idle();
        assert!(r.ms.state == "Idle" && near(r.ms.mpos.x, 0.0) && near(r.ms.mpos.z, 0.0));
        // jog, then zero the work coordinates there
        r.send("$J=G91 G20 X10 Y5 F500");
        r.run_until_idle();
        assert!(near(r.ms.mpos.x, 10.0) && near(r.ms.mpos.y, 5.0));
        r.send("$J=G91 G20 Z-3 F200");
        r.run_until_idle();
        r.send("G10 L20 P1 X0 Y0 Z0");
        r.pump(0.05);
        assert!(near(r.ms.wpos().x, 0.0) && near(r.ms.wco.x, 10.0) && near(r.ms.wco.z, -3.0));
        // stream a program: ends idle at the last position with the spindle off
        let mut prog = String::from("G90 G20\nM3 S18000\nG0 Z0.2\n");
        for i in 0..40 {
            prog += &format!("G1 X{} Y{:.6} Z-0.1 F600\n", if i % 2 == 1 { 2 } else { 0 }, f64::from(i) * 0.05);
        }
        prog += "G0 Z0.5\nM5\nM30\n";
        let gp = parse_program(&prog, 1000.0);
        r.st.load(&gp.lines);
        r.st.start();
        for _ in 0..3 {
            r.pump(0.01);
        }
        // planner holds ~16 blocks: the rest waits; spindle on while cutting
        assert!(r.st.acked() < r.st.total() && r.sim.spindle_on() && r.ms.rpm == 18000.0);
        r.run_until_idle();
        assert!(r.st.done() && r.st.error().is_empty() && !r.sim.spindle_on(), "{}", r.log);
        assert!(near(r.ms.wpos().z, 0.5) && near(r.ms.wpos().y, 39.0 * 0.05));
        // probing: G38.2 stops at the plate (machine Z -4) and reports [PRB:...:1]; a miss alarms (ALARM:5)
        r.send("G53 G0 Z0");
        r.run_until_idle();
        r.send("G38.2 Z-2.5 F10"); // work -2.5 = machine -5.5; the plate at -4 trips first
        r.run_until_idle();
        assert!(near(r.ms.mpos.z, -4.0) && r.has(":1]"));
        r.send("G53 G0 Z0");
        r.run_until_idle();
        r.sim.set_probe_z(-9.0);
        r.send("G38.2 Z-2 F10"); // machine Z -5: inside the travel, no plate there
        r.run_until_idle();
        assert!(r.ms.state == "Alarm" && r.has("ALARM:5"));
        r.send("$X");
        r.pump(0.05);
        r.sim.set_probe_z(-4.0);
        assert!(alarm_text(5).contains("Probe") && error_text(9).contains("alarm") && error_text(999).is_empty());
        // the app's probe line: relative, after modal words (the C++ sim rejected this)
        r.send("G53 G0 Z0");
        r.run_until_idle();
        r.send("G91 G38.2 Z-5.0000 F50.0");
        r.send("G90");
        r.run_until_idle();
        assert!(near(r.ms.mpos.z, -4.0) && r.ms.state == "Idle", "{}", r.log);
        // settings report: travel comes from the controller
        r.send("$$");
        r.pump(0.05);
        assert_eq!(parse_setting("$130=1524.000"), Some((130, 1524.0)));
        assert_eq!(parse_setting("ok"), None);
        assert!(r.has("$131=2438.400"));
        // G53: machine coordinates, axes not given stay put
        r.send("G53 G0 Z0");
        r.run_until_idle();
        assert!(near(r.ms.mpos.z, 0.0) && near(r.ms.wpos().y, 39.0 * 0.05));
        // feed hold stops motion; resume finishes
        r.st.load(&parse_program("G1 X20 F60\n", 1000.0).lines);
        r.st.start();
        for _ in 0..20 {
            r.pump(0.05);
        }
        r.sim.receive(&[HOLD]);
        r.pump(0.0);
        let x = r.ms.mpos.x;
        for _ in 0..20 {
            r.pump(0.05);
        }
        assert!(r.ms.state.starts_with("Hold") && near(r.ms.mpos.x, x));
        r.sim.receive(&[RESUME]);
        r.run_until_idle();
        assert!(near(r.ms.wpos().x, 20.0));
        // soft limit: a move past the machine travel alarms instead of moving
        r.send("G0 X500");
        r.pump(0.05);
        assert!(r.ms.state == "Alarm" && r.has("ALARM:2"));
        // reset while moving -> ALARM:3, locked
        r.send("$X");
        r.pump(0.05);
        r.st.load(&parse_program("G1 X0 F30\n", 1000.0).lines);
        r.st.start();
        for _ in 0..5 {
            r.pump(0.05);
        }
        r.sim.receive(&[RESET]);
        r.st.stop();
        r.pump(0.0);
        assert!(r.ms.state == "Alarm" && r.has("ALARM:3") && !r.sim.spindle_on());
    }

    #[test]
    fn resume_from_line() {
        // same end state as running the whole program, spindle on before the plunge, lift first
        let prog = ["G90 G20", "M3 S16000", "G0 X1 Y1 Z0.2", "G1 Z-0.25 F50", "G1 X3 F100", "G1 Y3", "G1 X1", "G0 Z0.5", "M5"];
        let pre = resume_preamble(&prog, 6, 20.0); // restart at "G1 Y3": the bit was at X3 Y1 Z-0.25
        let all = pre.join("\n");
        let at = |s: &str| all.find(s).unwrap_or_else(|| panic!("{s} missing in\n{all}"));
        assert!(at("G53 G0 Z0") < at("G0 X3.0000 Y1.0000") && at("M3 S16000") < at("G1 Z-0.2500"));
        assert!(all.contains("F100.0") && pre.last().unwrap() == "G1");
        assert!(resume_preamble(&prog, 1, 20.0).is_empty() && resume_preamble(&prog, 99, 20.0).is_empty());
        assert!(resume_preamble(&["G10 L20 P1 X0", "G0 X1"], 2, 20.0).is_empty()); // re-zeroing before the line: refuse
        let mut r = Rig::new();
        r.send("$H");
        r.run_until_idle();
        r.send("$J=G91 G20 X10 Y10 Z-2 F3000");
        r.run_until_idle();
        r.send("G10 L20 P1 X0 Y0 Z0");
        r.pump(0.05);
        let mut lines = pre.clone();
        lines.extend(prog[5..].iter().map(|s| s.to_string()));
        r.st.load(&lines);
        r.st.start();
        r.run_until_idle();
        let w = r.ms.wpos();
        assert!(r.st.done() && r.st.error().is_empty() && near(w.x, 1.0) && near(w.y, 3.0) && near(w.z, 0.5) && !r.sim.spindle_on());
    }

    #[test]
    fn streamer_buffer_and_errors() {
        // keeps the controller buffer under 128 bytes and stops on the first error
        let mut s = GrblStreamer::default();
        let lines: Vec<String> = (0..50).map(|i| format!("G1 X1.2345 Y6.7890 Z-0.1234 F100 (comment {i})")).collect();
        s.load(&lines);
        s.start();
        let c = s.next_chunk();
        assert!(c.len() < 128 && !c.contains('(') && s.next_chunk().is_empty());
        s.on_response("ok");
        assert!(!s.next_chunk().is_empty());
        s.on_response("error:20");
        assert!(!s.running() && s.error().contains("line 2"));
    }

    #[test]
    fn status_parsing() {
        let mut st = MachineStatus::default();
        assert!(parse_status("<Hold:0|MPos:1.000,2.000,-0.500|FS:120,18000|WCO:1,1,-1|Ov:110,100,80>", &mut st));
        assert!(st.state == "Hold:0" && near(st.wpos().z, 0.5) && st.rpm == 18000.0 && st.feed_ov == 110 && st.spindle_ov == 80);
        assert!(parse_status("<Idle|WPos:0,0,0>", &mut st) && near(st.mpos.x, 1.0));
        assert!(!parse_status("ok", &mut st));
        assert_eq!(clean_line(" g1 x1 (c) ; tail"), "G1X1");
    }
}
