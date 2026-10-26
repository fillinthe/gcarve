//! One connection to a GRBL-family controller (TCP or USB serial, desktop) or the built-in simulator (everywhere),
//! C++ `app/machine.*` MachineLink. Protocol logic lives in gcarve_core::grbl; this moves bytes, polls status at 10 Hz
//! and applies the safety rules. No threads besides the TCP connect: the UI calls `update(now)` every frame.

use gcarve_core::gcode::{parse_program, GProgram, Vec3};
use gcarve_core::grbl::{self, GrblSim, GrblStreamer, MachineStatus};
use gcarve_core::job::MoveType;

enum Transport {
    Sim(Box<GrblSim>),
    #[cfg(not(target_arch = "wasm32"))]
    Tcp { stream: Option<std::net::TcpStream>, connecting: Option<std::sync::mpsc::Receiver<std::io::Result<std::net::TcpStream>>>, peer: String, outbox: Vec<u8> },
    #[cfg(all(feature = "serial", not(target_arch = "wasm32")))]
    Serial(Box<dyn serialport::SerialPort>, String),
    /// Web Serial (browser USB); the name arrives once the user picked the device
    #[cfg(target_arch = "wasm32")]
    WebSerial(Option<String>),
}

pub struct Link {
    transport: Option<Transport>,
    pub st: MachineStatus,
    streamer: GrblStreamer,
    pub program: GProgram,
    pub program_name: String,
    pub last_error: String,
    pub log: Vec<String>,
    /// a program is being sent (also while on hold)
    pub streaming: bool,
    zeroed: [bool; 3],
    pub probing: bool,
    probe_plate: f64,
    probe_lift: f64,
    travel_from_controller: bool,
    /// inches; from the controller when it reports $130-$132
    pub travel: Vec3,
    inbuf: Vec<u8>,
    last_tick: Option<f64>,
    last_poll: f64,
    /// bumped when the program changes (preview cache key)
    pub program_rev: u64,
    /// the tool change already announced (once per change)
    tool_said: String,
    /// held after a program line was rejected: Resume is refused until Stop / a new start
    stopped_on_error: bool,
}

impl Default for Link {
    fn default() -> Self {
        Link {
            transport: None,
            st: MachineStatus::default(),
            streamer: GrblStreamer::default(),
            program: GProgram::default(),
            program_name: String::new(),
            last_error: String::new(),
            log: Vec::new(),
            streaming: false,
            zeroed: [false; 3],
            probing: false,
            probe_plate: 0.0,
            probe_lift: 0.0,
            travel_from_controller: false,
            travel: Vec3 { x: 60.0, y: 96.0, z: 6.0 },
            inbuf: Vec::new(),
            last_tick: None,
            last_poll: 0.0,
            program_rev: 0,
            tool_said: String::new(),
            stopped_on_error: false,
        }
    }
}

fn status_is(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}

impl Link {
    pub fn connected(&self) -> bool {
        self.transport.is_some()
    }
    /// "Simulator" / "host:port" / port name
    pub fn transport(&self) -> String {
        match &self.transport {
            Some(Transport::Sim(_)) => "Simulator".into(),
            #[cfg(not(target_arch = "wasm32"))]
            Some(Transport::Tcp { peer, .. }) => peer.clone(),
            #[cfg(all(feature = "serial", not(target_arch = "wasm32")))]
            Some(Transport::Serial(_, name)) => name.clone(),
            #[cfg(target_arch = "wasm32")]
            Some(Transport::WebSerial(name)) => name.clone().unwrap_or_else(|| "Choosing a USB device…".into()),
            None => String::new(),
        }
    }
    /// Idle Run Hold:0 Jog Alarm Home ... / Disconnected
    pub fn state(&self) -> &str {
        if self.connected() { &self.st.state } else { "Disconnected" }
    }
    pub fn idle(&self) -> bool {
        self.state() == "Idle"
    }
    pub fn alarm(&self) -> bool {
        status_is(self.state(), "Alarm")
    }
    pub fn hold(&self) -> bool {
        status_is(self.state(), "Hold")
    }
    pub fn work_zero_set(&self) -> bool {
        self.zeroed.iter().all(|z| *z)
    }
    /// last program line the controller accepted
    pub fn progress_line(&self) -> i32 {
        self.streamer.source_line()
    }
    /// 0..1
    pub fn progress(&self) -> f64 {
        if self.streamer.total() > 0 { self.streamer.acked() as f64 / self.streamer.total() as f64 } else { 0.0 }
    }
    pub fn minutes_left(&self) -> f64 {
        self.program.minutes * (1.0 - self.progress())
    }

    pub fn add_log(&mut self, s: impl Into<String>) {
        self.log.push(s.into());
        if self.log.len() > 300 {
            let n = self.log.len() - 300;
            self.log.drain(..n);
        }
    }

    /// machine travel for the start check (inches); the controller's $130-$132 win
    pub fn set_travel(&mut self, t: Vec3) {
        if !self.travel_from_controller {
            self.travel = t;
        }
    }

    pub fn connect_sim(&mut self) {
        self.disconnect();
        self.transport = Some(Transport::Sim(Box::default()));
        self.travel = Vec3 { x: 60.0, y: 96.0, z: 6.0 };
        self.last_tick = None;
        self.add_log("Connected to the simulator (5x8 ft machine, homing required)");
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn connect_tcp(&mut self, host: &str, port: u16) {
        self.disconnect();
        let (tx, rx) = std::sync::mpsc::channel();
        let addr = format!("{host}:{port}");
        let target = addr.clone();
        std::thread::spawn(move || {
            use std::net::ToSocketAddrs;
            let r = target.to_socket_addrs().and_then(|mut a| a.next().ok_or_else(|| std::io::Error::other("host not found"))).and_then(|a| std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_secs(5)));
            let _ = tx.send(r);
        });
        self.transport = Some(Transport::Tcp { stream: None, connecting: Some(rx), peer: addr, outbox: Vec::new() });
        self.last_tick = None;
    }

    pub fn has_serial() -> bool {
        #[cfg(target_arch = "wasm32")]
        return super::webserial::supported();
        #[cfg(not(target_arch = "wasm32"))]
        cfg!(feature = "serial")
    }

    /// "name — description"
    pub fn serial_ports() -> Vec<String> {
        #[cfg(all(feature = "serial", not(target_arch = "wasm32")))]
        {
            serialport::available_ports()
                .unwrap_or_default()
                .into_iter()
                .map(|p| match p.port_type {
                    serialport::SerialPortType::UsbPort(u) => format!("{} — {}", p.port_name, u.product.unwrap_or_else(|| "USB".into())),
                    _ => p.port_name,
                })
                .collect()
        }
        #[cfg(not(all(feature = "serial", not(target_arch = "wasm32"))))]
        Vec::new()
    }

    pub fn connect_serial(&mut self, port: &str, baud: u32) {
        #[cfg(all(feature = "serial", not(target_arch = "wasm32")))]
        {
            self.disconnect();
            let name = port.split(" — ").next().unwrap_or(port).to_string();
            // 250 ms: on Windows the timeout covers a whole write, and 128 bytes take ~11 ms at 115200 (1 ms cut writes
            // short and dropped the link mid-job); reads only take what bytes_to_read says is there, so never wait
            match serialport::new(&name, baud).timeout(std::time::Duration::from_millis(250)).open() {
                Ok(p) => {
                    self.transport = Some(Transport::Serial(p, name));
                    self.last_tick = None;
                    self.add_log(format!("Connected to {port} at {baud}"));
                    self.write(&[grbl::RESET]); // fresh controller state, prints the Grbl banner
                }
                Err(e) => {
                    self.last_error = format!("Serial: {e}");
                    self.add_log(self.last_error.clone());
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = port; // the browser's chooser picks the device
            self.disconnect();
            super::webserial::open(baud);
            self.transport = Some(Transport::WebSerial(None));
            self.last_tick = None;
        }
        #[cfg(all(not(feature = "serial"), not(target_arch = "wasm32")))]
        {
            let _ = (port, baud);
            self.last_error = "This build has no serial port support".into();
        }
    }

    pub fn disconnect(&mut self) {
        #[cfg(target_arch = "wasm32")]
        if matches!(self.transport, Some(Transport::WebSerial(_))) {
            super::webserial::close();
        }
        self.transport = None;
        self.probing = false;
        self.streaming = false;
        self.streamer.stop();
        self.zeroed = [false; 3];
        self.travel_from_controller = false;
        self.st = MachineStatus::default();
        self.inbuf.clear();
    }

    /// the transport dropped under us
    fn lost(&mut self, why: &str) {
        if self.streaming {
            self.last_error = "Connection lost while running: check the machine NOW (use its E-stop)".into();
        } else if !why.is_empty() {
            self.last_error = why.to_string();
        }
        self.add_log("Disconnected");
        self.disconnect();
    }

    fn write(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        match &mut self.transport {
            Some(Transport::Sim(s)) => s.receive(bytes),
            #[cfg(not(target_arch = "wasm32"))]
            Some(Transport::Tcp { outbox, .. }) => outbox.extend_from_slice(bytes),
            #[cfg(target_arch = "wasm32")]
            Some(Transport::WebSerial(name)) => {
                if name.is_some() && !super::webserial::write(bytes) {
                    self.lost("USB: the port is closed");
                }
            }
            #[cfg(all(feature = "serial", not(target_arch = "wasm32")))]
            Some(Transport::Serial(p, _)) => {
                if let Err(e) = p.write_all(bytes) {
                    let msg = format!("Serial: {e}");
                    self.lost(&msg);
                }
            }
            None => {}
        }
    }
    /// The operator's own lines (jog, probe, zero, MDI…): booked with the streamer so their replies are never taken
    /// for the program's (a stray "ok" made it send past the controller's buffer and end jobs a line early).
    fn send(&mut self, text: &str) {
        for l in text.split_inclusive('\n').filter(|l| !l.trim().is_empty()) {
            self.streamer.foreign_line(l.len() + usize::from(!l.ends_with('\n')));
        }
        self.write(text.as_bytes());
    }
    /// program lines from the streamer (already booked)
    fn send_program(&mut self, chunk: &str) {
        self.write(chunk.as_bytes());
    }

    /// Every frame: advance the simulator (`now` = seconds, any monotonic clock), move bytes, poll status at 10 Hz,
    /// keep the streamer fed.
    pub fn update(&mut self, now: f64) {
        if !self.connected() {
            return;
        }
        let dt = self.last_tick.map_or(0.02, |t| (now - t).clamp(0.0, 0.2));
        self.last_tick = Some(now);
        let mut incoming = Vec::new();
        let mut dropped: Option<String> = None;
        #[cfg(target_arch = "wasm32")]
        let mut opened = false;
        match &mut self.transport {
            #[cfg(target_arch = "wasm32")]
            Some(Transport::WebSerial(name)) => {
                use super::webserial::State;
                match super::webserial::state() {
                    State::Opening => return, // the browser's chooser is up
                    State::Open(d) => {
                        if name.is_none() {
                            *name = Some(d);
                            opened = true;
                        }
                        incoming = super::webserial::take();
                    }
                    State::Cancelled => dropped = Some("No USB device chosen".into()),
                    State::Failed(e) => dropped = Some(if self.streaming { "USB connection lost while running: check the machine NOW (use its E-stop)".into() } else { format!("USB: {e}") }),
                }
            }
            Some(Transport::Sim(s)) => {
                s.tick(dt);
                incoming = s.take_output().into_bytes();
            }
            #[cfg(not(target_arch = "wasm32"))]
            Some(Transport::Tcp { stream, connecting, peer, outbox }) => {
                use std::io::{ErrorKind, Read, Write};
                if let Some(rx) = connecting {
                    match rx.try_recv() {
                        Ok(Ok(s)) => {
                            let _ = s.set_nonblocking(true);
                            let _ = s.set_nodelay(true);
                            *stream = Some(s);
                            *connecting = None;
                            // ponytail: Qt only sent "\n" here; "$$" too so the travel comes from the controller over TCP
                            outbox.splice(0..0, b"\n$$\n".iter().copied());
                            self.log.push(format!("Connected to {peer}"));
                        }
                        Ok(Err(e)) => dropped = Some(format!("Network: {e}")),
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => dropped = Some("Network: connection failed".into()),
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    }
                }
                if let Some(s) = stream {
                    let mut buf = [0u8; 4096];
                    loop {
                        match s.read(&mut buf) {
                            Ok(0) => {
                                dropped = Some(String::new());
                                break;
                            }
                            Ok(n) => incoming.extend_from_slice(&buf[..n]),
                            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                            Err(e) if e.kind() == ErrorKind::Interrupted => {}
                            Err(e) => {
                                dropped = Some(format!("Network: {e}"));
                                break;
                            }
                        }
                    }
                    while !outbox.is_empty() && dropped.is_none() {
                        match s.write(outbox) {
                            Ok(n) => {
                                outbox.drain(..n);
                            }
                            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                            Err(e) => dropped = Some(format!("Network: {e}")),
                        }
                    }
                }
            }
            #[cfg(all(feature = "serial", not(target_arch = "wasm32")))]
            Some(Transport::Serial(p, _)) => {
                let n = p.bytes_to_read().unwrap_or(0) as usize;
                if n > 0 {
                    let mut buf = vec![0u8; n];
                    match p.read(&mut buf) {
                        Ok(k) => incoming.extend_from_slice(&buf[..k]),
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                        Err(e) => dropped = Some(if self.streaming { "USB connection lost while running: check the machine NOW (use its E-stop)".into() } else { format!("Serial: {e}") }),
                    }
                }
            }
            None => {}
        }
        if let Some(why) = dropped {
            self.lost(&why);
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if opened {
            let t = self.transport();
            self.add_log(format!("Connected to {t}"));
            self.write(&[grbl::RESET]); // fresh controller state, prints the Grbl banner (then $$ for the travel)
        }
        self.incoming(&incoming);
        #[cfg(target_arch = "wasm32")]
        {
            // while a job runs, hiding the tab holds the feed at once (no frames = nothing feeds the controller)
            super::webserial::on_hide(if self.streaming && self.streamer.tool_wait().is_none() { &[grbl::HOLD] } else { &[] });
            if super::webserial::held_on_hide() {
                self.add_log("Paused: this tab was hidden while the job ran. Press Resume to carry on (keep the tab in front)");
            }
        }
        if now - self.last_poll >= 0.1 {
            self.last_poll = now;
            self.write(&[grbl::STATUS]); // status reports at 10 Hz
        }
        self.pump();
    }

    fn incoming(&mut self, bytes: &[u8]) {
        self.inbuf.extend_from_slice(bytes);
        if self.inbuf.len() > 4096 && !self.inbuf.contains(&b'\n') {
            // no line ends in 4 KB: not grbl talking (wrong speed, noise); don't let it grow for ever
            self.inbuf.clear();
            self.add_log("Unreadable data from the controller: check the baud rate (grbl: 115200)");
        }
        while let Some(nl) = self.inbuf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.inbuf.drain(..=nl).collect();
            let l = String::from_utf8_lossy(&raw).trim_end_matches(['\r', '\n']).to_string();
            if !l.is_empty() {
                self.handle_line(&l);
            }
        }
    }

    fn handle_line(&mut self, l: &str) {
        if grbl::parse_status(l, &mut self.st) {
            return;
        }
        if self.streamer.on_response(l) {
            if !self.streamer.error().is_empty() {
                self.last_error = format!("Stopped at {}", self.streamer.error());
                let code: i32 = l.split("error:").nth(1).and_then(|c| c.trim().parse().ok()).unwrap_or(0);
                let why = grbl::error_text(code);
                if !why.is_empty() {
                    self.last_error += &format!(" ({why})");
                }
                self.add_log(self.last_error.clone());
                self.streaming = false;
                self.write(&[grbl::HOLD]); // the controller may still have moves queued: hold them
                self.stopped_on_error = true; // …and never resume them: the rejected line is missing from the job
            }
            return;
        }
        let mut q = l.to_string();
        if let Some(code) = l.strip_prefix("ALARM:") {
            let why = grbl::alarm_text(code.trim().parse().unwrap_or(0));
            self.last_error = format!("ALARM {code}{}", if why.is_empty() { ": home or unlock before continuing".to_string() } else { format!(": {why}") });
            if self.streaming {
                self.streaming = false;
                self.streamer.stop();
            }
            self.probing = false;
        }
        if let Some(code) = l.strip_prefix("error:") {
            let why = grbl::error_text(code.trim().parse().unwrap_or(0));
            if !why.is_empty() {
                q += &format!(" ({why})");
            }
            if self.probing {
                self.probing = false;
                self.last_error = format!("Probe failed: {q}");
            }
        }
        if l.starts_with("[PRB:") && self.probing {
            // [PRB:x,y,z:1] = touched; set Z so the plate top reads the plate thickness, then lift
            self.probing = false;
            if l.ends_with(":1]") {
                self.send(&format!("G20 G10 L20 P1 Z{:.4}\nG20 G91 G0 Z{:.4}\nG90\n", self.probe_plate, self.probe_lift));
                self.zeroed[2] = true;
                self.add_log(format!("Z zeroed by the touch plate (plate {} in)", self.probe_plate));
            } else {
                self.last_error = "Probe did not touch the plate: check the plate wire and the distance".into();
            }
        }
        if l.starts_with("Grbl") {
            // banner = the controller reset: nothing of ours is queued any more
            self.streamer.stop();
            self.streaming = false;
            self.send("$$\n");
        }
        if let Some((n, v)) = grbl::parse_setting(l) {
            if (130..=132).contains(&n) && v > 0.0 {
                // $130-$132 (mm) from the controller = the travel the start check uses
                let inch = v / 25.4;
                match n {
                    130 => self.travel.x = inch,
                    131 => self.travel.y = inch,
                    _ => self.travel.z = inch,
                }
                self.travel_from_controller = true;
                return; // keep the console readable
            }
        }
        if q != "ok" {
            self.add_log(q);
        }
    }

    fn pump(&mut self) {
        if !self.streaming {
            return;
        }
        if let Some(t) = self.tool_wait() {
            if self.tool_said != t {
                self.send("M5\n"); // the post stops it before a change; never leave it to the file
                self.add_log(format!("Tool change: put in {t}, then touch off Z and press Continue"));
                self.tool_said = t;
            }
            return;
        }
        let chunk = self.streamer.next_chunk();
        if !chunk.is_empty() {
            // until the controller's next report: lines just sent mean it's busy (a stale "Idle" ended short jobs
            // and the restart after a tool change as "finished" while the machine was still moving)
            self.st.state = "Run".into();
        }
        self.send_program(&chunk);
        if self.streamer.done() && self.idle() {
            self.streaming = false;
            self.add_log(format!("Program finished: {}", self.program_name));
        }
    }

    /// idle and free for the operator's commands: no program running, or one waiting at a tool change
    fn ready(&self) -> bool {
        self.connected() && (!self.streaming || self.streamer.tool_wait().is_some()) && self.idle()
    }

    /// the bit to put in, while a program waits at a tool change (M6) with the machine stopped
    pub fn tool_wait(&self) -> Option<String> {
        self.streamer.tool_wait().filter(|_| self.streaming && self.idle()).map(str::to_string)
    }

    /// the new bit is in (and touched off): put the program's units back and carry on
    pub fn continue_after_tool(&mut self) {
        if self.tool_wait().is_none() {
            return;
        }
        // our own commands (probe, jog) may have switched to inches: the program's last unit word wins again
        let upto = (self.streamer.source_line().max(1) as usize).min(self.program.lines.len());
        let mm = self.program.lines[..upto].iter().rev().map(|l| grbl::clean_line(l)).find(|l| l.contains("G20") || l.contains("G21")).is_some_and(|l| l.contains("G21"));
        self.send(if mm { "G21 G90\n" } else { "G20 G90\n" });
        self.add_log("Tool change done: carrying on");
        self.tool_said.clear();
        self.streamer.next_tool();
        self.pump();
    }

    /// Outline: the cut's outer box traced at a safe height (above the program's highest point, at least 0.25 in),
    /// then back over the start corner, so the operator sees the job fits the board before anything spins.
    pub fn outline(&mut self) {
        if !self.ready() || self.streaming {
            return;
        }
        let cuts: Vec<_> = self.program.motions.iter().filter(|m| m.kind != MoveType::Rapid && !m.machine_coords).collect();
        if cuts.is_empty() {
            self.last_error = "Load a program first: the outline follows its cuts".into();
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for m in &cuts {
            for p in [m.from, m.to] {
                (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
            }
        }
        let z = self.program.max.z.max(0.25);
        self.add_log(format!("Outline: {:.2} x {:.2} in at Z {z:.2}", x1 - x0, y1 - y0));
        self.send(&format!("G20 G90 G0 Z{z:.4}\nG0 X{x0:.4} Y{y0:.4}\nG0 X{x1:.4}\nG0 Y{y1:.4}\nG0 X{x0:.4}\nG0 Y{y0:.4}\n"));
    }

    pub fn home(&mut self) {
        if self.connected() && !self.streaming {
            self.send("$H\n");
            self.last_error.clear();
        }
    }
    pub fn unlock(&mut self) {
        if self.connected() && !self.streaming {
            self.send("$X\n");
            self.last_error.clear();
        }
    }
    /// inches, relative
    pub fn jog(&mut self, dx: f64, dy: f64, dz: f64, feed: f64) {
        let busy = self.streaming && self.streamer.tool_wait().is_none(); // jogging is fine at a tool change
        if !self.connected() || busy || !(self.idle() || self.state() == "Jog") {
            return;
        }
        if self.streamer.room() < 64 {
            return; // jogs still waiting for the controller: never more than its buffer holds
        }
        self.send(&format!("$J=G91 G20 X{dx:.4} Y{dy:.4} Z{dz:.4} F{:.0}\n", feed.max(1.0)));
    }
    pub fn jog_stop(&mut self) {
        self.write(&[grbl::JOG_CANCEL]);
    }
    /// "X", "Y", "Z", "XY", "XYZ"
    pub fn zero_axes(&mut self, axes: &str) {
        if !self.ready() {
            if self.connected() {
                self.add_log("Zero ignored: wait until the machine is Idle");
            }
            return;
        }
        let axes = axes.to_ascii_uppercase();
        let mut cmd = String::from("G10 L20 P1");
        for (i, a) in ['X', 'Y', 'Z'].into_iter().enumerate() {
            if axes.contains(a) {
                cmd += &format!(" {a}0");
                self.zeroed[i] = true;
            }
        }
        self.send(&format!("{cmd}\n"));
        self.add_log(format!("Zeroed {axes}"));
    }
    /// Touch plate: probe down up to `max_travel` at `feed`; on contact Z = plate thickness (the plate's top), then lift.
    pub fn probe_z(&mut self, plate: f64, max_travel: f64, feed: f64, lift: f64) {
        if !self.ready() {
            return;
        }
        self.probing = true;
        self.probe_plate = plate;
        self.probe_lift = lift.max(0.05);
        self.last_error.clear();
        self.add_log("Probing Z…");
        // G20 on every line with numbers in inches: grbl starts in mm, and a program may have left G21 set
        self.send(&format!("G20 G91 G38.2 Z{:.4} F{:.1}\nG90\n", -max_travel.abs(), feed.max(1.0)));
    }
    /// lift to the top, then to the back-left corner
    pub fn park(&mut self) {
        if self.ready() {
            self.send(&format!("G53 G0 Z0\nG20 G53 G0 X0 Y{:.3}\n", self.travel.y));
        }
    }
    /// M3 at low speed, dwell, then off
    pub fn spindle_warmup(&mut self, rpm: i32, seconds: i32) {
        if self.ready() {
            self.add_log(format!("Spindle warm-up: {rpm} RPM for {seconds} s"));
            self.send(&format!("M3 S{}\nG4 P{}\nM5\n", rpm.max(1000), seconds.max(1)));
        }
    }
    pub fn spindle(&mut self, on: bool, rpm: i32) {
        if self.connected() && !self.streaming {
            self.send(&if on { format!("M3 S{}\n", rpm.max(0)) } else { "M5\n".into() });
        }
    }
    /// lift to machine Z0, then X0 Y0 (never travel low)
    pub fn go_to_work_zero(&mut self) {
        if self.ready() {
            self.send("G53 G0 Z0\nG90 G0 X0 Y0\n");
        }
    }

    pub fn load_program(&mut self, name: &str, text: &str) -> bool {
        if self.streaming {
            self.last_error = "Stop the running program first".into();
            return false;
        }
        self.program = parse_program(text, 1000.0);
        self.program_name = name.to_string();
        self.streamer.load::<&str>(&[]);
        self.program_rev += 1;
        self.add_log(format!("Loaded {name}: {} lines, {} moves", self.program.lines.len(), self.program.motions.len()));
        true
    }

    /// (errors, warnings): errors block Start, warnings need a tick
    pub fn start_check(&self) -> (Vec<String>, Vec<String>) {
        let (mut errors, mut warnings) = (Vec::new(), Vec::new());
        let p = &self.program;
        let conn = self.connected();
        if !conn {
            errors.push("Not connected to a machine.".to_string());
        }
        if self.streaming {
            errors.push("A program is already running.".into());
        }
        if conn && self.alarm() {
            errors.push("The machine is in alarm: Home (or Unlock) first.".into());
        } else if conn && !self.idle() {
            errors.push(format!("The machine is busy ({}): wait for Idle.", self.state()));
        }
        if p.lines.is_empty() {
            errors.push("No program loaded.".into());
        }
        if let Some(first) = p.errors.first() {
            let tap = if self.program_name.to_ascii_lowercase().ends_with(".tap") { " This looks like a WinCNC file: export it with a GRBL post." } else { "" };
            errors.push(format!("The program has {} line(s) this controller can't run ({first}).{tap}", p.errors.len()));
        }
        if p.motions.iter().any(|m| m.kind != MoveType::Rapid && !m.machine_coords && m.from.z.min(m.to.z) < 0.0 && !m.spindle_on) {
            errors.push("The program cuts below Z0 with the spindle off (no M3 before the cut).".into());
        }
        let has = p.min.x < 1e299;
        if has && conn {
            // the whole program must fit the machine travel from the current work zero
            let w = self.st.wco;
            let (lo, hi, t) = (Vec3 { x: p.min.x + w.x, y: p.min.y + w.y, z: p.min.z + w.z }, Vec3 { x: p.max.x + w.x, y: p.max.y + w.y, z: p.max.z + w.z }, self.travel);
            if lo.x < -1e-4 || lo.y < -1e-4 || hi.x > t.x + 1e-4 || hi.y > t.y + 1e-4 || hi.z > 1e-4 || lo.z < -t.z - 1e-4 {
                errors.push(format!("The program goes outside the machine travel from this work zero (X {:.2}..{:.2}, Y {:.2}..{:.2}, Z {:.2}..{:.2} in machine coordinates).", lo.x, hi.x, lo.y, hi.y, lo.z, hi.z));
            }
        }
        if conn && !self.travel_from_controller {
            warnings.push(format!("The controller didn't report its travel ($130-$132): using the machine settings ({:.1} x {:.1} x {:.1} in).", self.travel.x, self.travel.y, self.travel.z));
        }
        if conn && !self.work_zero_set() {
            warnings.push("X, Y and Z zero were not all set since connecting: make sure the current work zero is right.".into());
        }
        if !p.lines.is_empty() && !p.spindle_off_at_end {
            warnings.push("The program does not turn the spindle off at the end: GCarve Control will send M5 after it.".into());
        }
        if has && p.min.z < -1.5 {
            warnings.push(format!("The program cuts {:.3} in deep: check the bit and the material.", -p.min.z));
        }
        (errors, warnings)
    }

    /// grbl takes ~16 blocks ahead of the cut: back up past them so nothing is skipped
    pub fn suggested_resume_line(&self) -> i32 {
        (self.streamer.source_line() - 20).max(1)
    }

    /// `from_line` > 1: resume there (safe re-entry, grbl::resume_preamble)
    pub fn start(&mut self, from_line: usize) -> bool {
        if let Some(e) = self.start_check().0.into_iter().next() {
            self.last_error = e;
            return false;
        }
        let mut lines = self.program.lines.clone();
        if from_line > 1 {
            let mut pre = grbl::resume_preamble(&self.program.lines, from_line, 20.0);
            if pre.is_empty() {
                self.last_error = format!("Can't resume at line {from_line} (out of range, or the program sets its own zero before it)");
                return false;
            }
            pre.extend_from_slice(&self.program.lines[from_line - 1..]);
            lines = pre;
            self.add_log(format!("Resuming at line {from_line}"));
        }
        if !self.program.spindle_off_at_end {
            lines.push("M5".into()); // safety: the spindle always stops after the program
        }
        self.streamer.load(&lines);
        self.streamer.start();
        self.streaming = true;
        self.stopped_on_error = false;
        self.last_error.clear();
        self.add_log(format!("Started {}", self.program_name));
        self.pump();
        true
    }

    pub fn pause(&mut self) {
        if self.connected() {
            self.write(&[grbl::HOLD]);
            self.add_log("Feed hold");
        }
    }
    pub fn resume(&mut self) {
        if self.stopped_on_error {
            self.add_log("A line was rejected: Resume would run the moves after it without it. Press Stop, fix the file, then start again (or from a line)");
            return;
        }
        if self.connected() {
            self.write(&[grbl::RESUME]);
            self.add_log("Resume");
        }
    }
    /// feed hold + soft reset: motion and spindle stop, queue cleared
    pub fn stop(&mut self) {
        if !self.connected() {
            return;
        }
        self.write(&[grbl::HOLD, grbl::RESET]);
        self.streamer.stop();
        self.stopped_on_error = false;
        if self.streaming {
            self.add_log("Stopped by the operator");
        }
        self.streaming = false;
    }
    /// +1 / -1 = +-10 %, 0 = 100 %
    pub fn override_feed(&mut self, step: i32) {
        self.write(&[match step.signum() {
            1 => grbl::FEED_PLUS_10,
            -1 => grbl::FEED_MINUS_10,
            _ => grbl::FEED_100,
        }]);
    }
    /// 100, 50 or 25 %
    pub fn override_rapid(&mut self, pct: i32) {
        self.write(&[match pct {
            25 => grbl::RAPID_25,
            50 => grbl::RAPID_50,
            _ => grbl::RAPID_100,
        }]);
    }
    /// +1 / -1 = +-10 %, 0 = 100 %
    pub fn override_spindle(&mut self, step: i32) {
        self.write(&[match step.signum() {
            1 => grbl::SPINDLE_PLUS_10,
            -1 => grbl::SPINDLE_MINUS_10,
            _ => grbl::SPINDLE_100,
        }]);
    }
    /// MDI
    pub fn send_command(&mut self, line: &str) {
        let l = line.trim();
        if !self.connected() || (self.streaming && self.tool_wait().is_none()) || l.is_empty() {
            return;
        }
        self.add_log(format!("> {l}"));
        self.send(&format!("{l}\n"));
    }
    /// the simulator, for tests and the screenshot demo
    #[cfg(test)]
    pub fn sim(&mut self) -> Option<&mut GrblSim> {
        match &mut self.transport {
            Some(Transport::Sim(s)) => Some(s),
            _ => None,
        }
    }
    #[cfg(test)]
    pub fn streamed_total(&self) -> usize {
        self.streamer.total()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// run the link's frame loop on a fake clock until `done` (or the budget runs out)
    fn run(l: &mut Link, t: &mut f64, done: impl Fn(&Link) -> bool) {
        for _ in 0..100_000 {
            *t += 0.05;
            l.update(*t);
            if done(l) {
                return;
            }
        }
        panic!("timed out: state {} err '{}'\n{}", l.state(), l.last_error, l.log.join("\n"));
    }
    fn settle(l: &mut Link, t: &mut f64) {
        for _ in 0..3 {
            *t += 0.05;
            l.update(*t);
        }
        run(l, t, |l| l.idle() || l.alarm());
    }

    /// A two-bit job in one file (Send to Control): it stops at the M6 with the spindle off, the operator re-probes Z
    /// (in inches although the program runs in mm), Continue puts the program's mm back and the job finishes.
    #[test]
    fn a_tool_change_waits_for_the_new_bit_and_keeps_the_programs_units() {
        let mut l = Link::default();
        let mut t = 0.0;
        let prog = "G90 G21\nM3 S18000\nG0 X0 Y0 Z5\nG1 Z-2 F500\nG1 X50 F1000\nG0 Z20\n(1/8 End Mill)\nM5\nG0 Z20\nT2 M6\nM3 S16000\nG4 P1\nG0 X10 Y10 Z5\nG1 Z-1 F300\nG1 X40 F800\nG0 Z20\nM5\nM30\n";
        assert!(l.load_program("two-bits.nc", prog));
        l.connect_sim();
        settle(&mut l, &mut t);
        l.home();
        settle(&mut l, &mut t);
        l.jog(10.0, 10.0, -3.0, 3000.0);
        settle(&mut l, &mut t);
        l.zero_axes("XYZ");
        settle(&mut l, &mut t);
        l.jog(0.0, 0.0, 1.0, 3000.0);
        settle(&mut l, &mut t);
        // Outline: round the cuts' box (X 0..50 mm, Y 0..10 mm) above the highest point (Z 20 mm), back at the corner
        l.outline();
        let mut far = 0.0_f64;
        for _ in 0..400 {
            t += 0.05;
            l.update(t);
            far = far.max(l.st.wpos().x);
        }
        let w = l.st.wpos();
        assert!((far - 50.0 / 25.4).abs() < 1e-3 && w.x.abs() < 1e-3 && w.y.abs() < 1e-3 && (w.z - 20.0 / 25.4).abs() < 1e-3, "outline: far {far} end {w:?}");
        assert!(l.start(1));
        run(&mut l, &mut t, |l| l.tool_wait().is_some());
        assert_eq!(l.tool_wait().as_deref(), Some("T2: 1/8 End Mill"));
        settle(&mut l, &mut t);
        assert!(!l.sim().unwrap().spindle_on(), "spindle off at the change");
        // the job outline / jog / probe work while it waits; probe with a 0.5 in plate: Z must read 0.5 in, not 0.5 mm
        let z_mm = l.st.mpos.z;
        l.sim().unwrap().set_probe_z(z_mm - 1.0); // plate top 1 in below the bit
        l.probe_z(0.5, 2.0, 10.0, 0.2);
        run(&mut l, &mut t, |l| !l.probing);
        settle(&mut l, &mut t);
        assert!((l.st.wco.z - (z_mm - 1.0 - 0.5)).abs() < 1e-3, "plate read in inches: wco z {} (bit at {z_mm})", l.st.wco.z);
        l.continue_after_tool();
        run(&mut l, &mut t, |l| !l.streaming);
        assert!(l.last_error.is_empty(), "{}", l.last_error);
        // the second bit cut in mm again: 40 mm = 1.575 in along X from X 10 mm
        assert!(l.log.iter().any(|s| s.starts_with("Program finished")), "{}", l.log.join("\n"));
        assert!((l.st.wpos().x - 40.0 / 25.4).abs() < 1e-3, "x {}\n{}", l.st.wpos().x, l.log.join("\n"));
        assert_eq!(l.log.iter().filter(|s| s.starts_with("Tool change: put in")).count(), 1, "announced once");
    }

    /// tests/control_smoke + the --dialog control-run flow: connect, home, jog, zero, start check, stream, hold/resume,
    /// stop (reset in motion = ALARM:3), resume from a line.
    #[test]
    fn full_program_through_the_sim() {
        let mut l = Link::default();
        let mut t = 0.0;
        // no M5 at the end: the start check warns and Control appends it
        let mut prog = String::from("G90 G20\nM3 S18000\nG0 X0 Y0 Z0.2\n");
        for i in 0..60 {
            prog += &format!("G1 X{} Y{:.3} Z-0.1 F300\n", if i % 2 == 1 { 4 } else { 0 }, f64::from(i) * 0.05);
        }
        prog += "G0 Z0.5\n";
        assert!(l.load_program("demo.nc", &prog));
        let (e, _) = l.start_check();
        assert!(e.iter().any(|e| e.contains("Not connected")));
        l.connect_sim();
        settle(&mut l, &mut t);
        assert!(l.alarm() && !l.start(1));
        l.home();
        settle(&mut l, &mut t);
        assert!(l.idle());
        assert!(l.travel_from_controller && (l.travel.y - 96.0).abs() < 1e-6, "{:?}", l.travel);
        l.jog(10.0, 10.0, -3.0, 3000.0);
        settle(&mut l, &mut t);
        l.zero_axes("XYZ");
        settle(&mut l, &mut t);
        l.jog(0.0, 0.0, 1.0, 3000.0);
        settle(&mut l, &mut t);
        let (errors, warnings) = l.start_check();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(warnings.iter().any(|w| w.contains("M5")) && !warnings.iter().any(|w| w.contains("zero")), "{warnings:?}");
        assert!(l.start(1));
        assert_eq!(l.streamed_total(), l.program.lines.len() + 1); // + M5
        // hold mid-run: the machine stops; resume finishes
        run(&mut l, &mut t, |l| l.progress_line() > 20);
        l.pause();
        run(&mut l, &mut t, |l| l.hold());
        let x = l.st.mpos.x;
        for _ in 0..20 {
            t += 0.05;
            l.update(t);
        }
        assert!(l.hold() && (l.st.mpos.x - x).abs() < 1e-9);
        l.resume();
        run(&mut l, &mut t, |l| !l.streaming);
        assert!(l.last_error.is_empty() && l.idle(), "{}", l.last_error);
        assert!(!l.sim().unwrap().spindle_on() && l.st.rpm == 0.0);
        assert!(l.log.iter().any(|s| s.starts_with("Program finished")));
        // stop while cutting: hold + reset -> ALARM:3, streaming over, spindle off
        assert!(l.start(1));
        run(&mut l, &mut t, |l| l.progress_line() > 30);
        let resume_at = l.suggested_resume_line();
        l.stop();
        settle(&mut l, &mut t);
        assert!(l.alarm() && l.last_error.contains("ALARM 3") && !l.streaming && !l.sim().unwrap().spindle_on(), "{}", l.last_error);
        // resume from a line after re-homing keeps the zero (sim keeps WCO across reset)
        l.home();
        settle(&mut l, &mut t);
        assert!(resume_at > 1 && l.start(resume_at as usize), "{}", l.last_error);
        assert!(l.log.iter().any(|s| s.starts_with("Resuming at line")));
        run(&mut l, &mut t, |l| !l.streaming);
        assert!(l.last_error.is_empty(), "{}", l.last_error);
        let w = l.st.wpos();
        assert!((w.z - 0.5).abs() < 1e-3 && !l.sim().unwrap().spindle_on());
        // touch plate: Z zero = plate top reads the plate thickness, then lift
        l.go_to_work_zero();
        settle(&mut l, &mut t);
        l.probe_z(0.75, 6.0, 50.0, 0.25);
        settle(&mut l, &mut t);
        settle(&mut l, &mut t);
        assert!(!l.probing && l.last_error.is_empty(), "{}\n{}", l.last_error, l.log.join("\n"));
        assert!((l.st.wpos().z - 1.0).abs() < 1e-3, "z {}", l.st.wpos().z); // plate top at 0.75, lifted 0.25
        // a WinCNC file is refused
        l.load_program("old.tap", "[ comment ]\nF100 XY\nG1 X1\n");
        assert!(l.start_check().0.iter().any(|e| e.contains("WinCNC")));
    }
}
