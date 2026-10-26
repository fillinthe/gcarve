//! The Control window, laid out like app/ControlMain.qml: header (transport, connect, state, STOP), left = position /
//! zero / jog / touch plate / macros, centre = program preview + console, right = program / start / hold / overrides.
//! Safety: Start opens the start check (errors block, warnings need a tick), STOP = feed hold + reset, jogging only
//! when idle, M5 is appended when the program leaves the spindle on.

use eframe::egui::{self, Color32, RichText};
use crate::widgets::TipExt;
use gcarve_core::gcode::Vec3;

use super::link::Link;
use super::preview::Preview;
use super::Settings;
use crate::files::Files;
use crate::icons::IconButton;
use crate::theme;

const OK_GREEN: Color32 = Color32::from_rgb(0x3f, 0xbf, 0x7f);
const STOP_RED: Color32 = Color32::from_rgb(0xd2, 0x3c, 0x22);
const AXIS: [Color32; 3] = [Color32::from_rgb(0xff, 0x7a, 0x7a), Color32::from_rgb(0x7b, 0xd8, 0x8f), Color32::from_rgb(0x6c, 0xb6, 0xff)];
const TRANSPORTS: [&str; 3] = ["Simulator", "Network (grblHAL)", "USB (serial)"];

#[derive(Default)]
struct StartDialog {
    errors: Vec<String>,
    warnings: Vec<String>,
    ack: bool,
    from_on: bool,
    from: String,
}

pub struct ControlApp {
    link: Link,
    cfg: Settings,
    files: Files,
    preview: Preview,
    jog_feed: String,
    mdi: String,
    port_text: String,
    serial_ports: Vec<String>,
    queue: Vec<(String, String)>,
    queue_scan: f64,
    start: Option<StartDialog>,
    /// settings dialog buffers (travel x/y/z, plate, probe max, probe feed, lift, warm rpm, warm s)
    settings: Option<[String; 9]>,
    /// spoilboard surfacing dialog buffers (width, length, bit, stepover %, depth per pass, passes, feed, rpm)
    surfacing: Option<[String; 8]>,
    started_at: Option<f64>,
    demo: Option<(u32, f64)>,
    #[cfg_attr(not(feature = "screenshot"), allow(dead_code))]
    shot_frame: u64,
    #[cfg_attr(not(feature = "screenshot"), allow(dead_code))]
    shot_ms: Option<f64>,
}

fn section(ui: &mut egui::Ui, t: &str) {
    ui.label(RichText::new(t).size(11.0).family(crate::widgets::bold_family()).color(theme::TEXT_DIM));
}
fn dim(ui: &mut egui::Ui, t: impl Into<String>) -> egui::Response {
    ui.label(RichText::new(t).color(theme::TEXT_DIM))
}
fn small_wrap(ui: &mut egui::Ui, t: &str) {
    ui.add(egui::Label::new(RichText::new(t).size(11.0).color(theme::TEXT_DIM)).wrap());
}
fn state_color(s: &str) -> Color32 {
    if s == "Idle" {
        OK_GREEN
    } else if s.starts_with("Run") || s == "Jog" || s == "Home" {
        theme::ACCENT
    } else if s.starts_with("Hold") {
        theme::WARNING
    } else if s.starts_with("Alarm") {
        theme::DANGER
    } else {
        theme::TEXT_DIM
    }
}
fn fmt(v: f64) -> String {
    format!("{}{v:.4}", if v >= 0.0 { " " } else { "" })
}
fn num(s: &str) -> Option<f64> {
    s.trim().trim_end_matches(|c: char| c.is_alphabetic() || c == '/').trim().parse().ok()
}
fn button(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(text))
}
/// filled action button (Start green, STOP red)
fn filled(ui: &mut egui::Ui, enabled: bool, text: &str, fill: Color32, width: f32) -> egui::Response {
    let b = egui::Button::new(RichText::new(text).family(crate::widgets::bold_family()).color(if enabled { Color32::WHITE } else { theme::TEXT_DISABLED })).fill(if enabled { fill } else { theme::RAISED }).min_size(egui::vec2(width, theme::CONTROL_HEIGHT));
    ui.add_enabled(enabled, b)
}

impl ControlApp {
    pub fn new(cc: &eframe::CreationContext<'_>, file: Option<String>) -> Self {
        theme::apply(&cc.egui_ctx);
        crate::fonts::install(&cc.egui_ctx); // bold / mono families the screen uses (missing = panic at the first frame)
        crate::icons::install(&cc.egui_ctx, crate::app_icons::RAW);
        let cfg = Settings::load();
        let mut link = Link::default();
        link.set_travel(Vec3 { x: cfg.travel[0], y: cfg.travel[1], z: cfg.travel[2] });
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(f) = &file {
            match std::fs::read_to_string(f) {
                Ok(t) => {
                    let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    link.load_program(&name, &t);
                }
                Err(e) => link.last_error = format!("Cannot read {f}: {e}"),
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = file;
        let mut app = ControlApp {
            jog_feed: format!("{:.0}", cfg.jog_feed),
            port_text: cfg.port.to_string(),
            serial_ports: Link::serial_ports(),
            link,
            cfg,
            files: Files::default(),
            preview: Preview::default(),
            mdi: String::new(),
            queue: super::queue(),
            queue_scan: 0.0,
            start: None,
            settings: None,
            surfacing: None,
            started_at: None,
            demo: None,
            shot_frame: 0,
            shot_ms: None,
        };
        #[cfg(target_arch = "wasm32")]
        if let Some((_, key)) = app.queue.first().cloned() {
            app.load_queued(&key); // just sent from the design tab
        }
        // dev hooks (Qt --dialog control-sim|control-start|control-run, + control-surface; GCARVE_SHOT_MS)
        #[cfg(not(target_arch = "wasm32"))]
        {
            app.shot_ms = std::env::var("GCARVE_SHOT_MS").ok().and_then(|v| v.parse().ok());
            match std::env::var("GCARVE_DIALOG").as_deref() {
                Ok("control-sim") => app.link.connect_sim(),
                Ok("control-start") => {
                    app.link.connect_sim();
                    app.open_start();
                }
                Ok("control-surface") => {
                    app.link.connect_sim();
                    app.surfacing = Some(["24", "24", "1.0", "40", "0.02", "1", "100", "18000"].map(String::from));
                }
                Ok("control-run") => {
                    app.link.connect_sim();
                    app.demo = Some((0, 0.0));
                }
                _ => {}
            }
        }
        app
    }

    fn load_queued(&mut self, key: &str) {
        match super::read_queued(key) {
            Some(t) => {
                let name = key.rsplit(['/', '\\']).next().unwrap_or(key).to_string();
                self.link.load_program(&name, &t);
            }
            None => self.link.last_error = format!("Cannot read {key}"),
        }
    }

    fn open_start(&mut self) {
        let (errors, warnings) = self.link.start_check();
        self.start = Some(StartDialog { errors, warnings, ack: false, from_on: false, from: self.link.suggested_resume_line().to_string() });
    }

    fn connect(&mut self) {
        self.link.set_travel(Vec3 { x: self.cfg.travel[0], y: self.cfg.travel[1], z: self.cfg.travel[2] });
        match self.cfg.transport {
            0 => self.link.connect_sim(),
            #[cfg(not(target_arch = "wasm32"))]
            1 => {
                self.cfg.port = self.port_text.trim().parse().unwrap_or(self.cfg.port);
                self.link.connect_tcp(&self.cfg.host.clone(), self.cfg.port);
            }
            #[cfg(target_arch = "wasm32")]
            1 => self.link.last_error = "Network connections need the desktop app (browsers can't open TCP sockets)".into(),
            _ => self.link.connect_serial(&self.cfg.serial_port.clone(), self.cfg.baud),
        }
        self.cfg.save();
    }

    /// --dialog control-run: home, jog over the board, zero, start (screenshots / smoke test of the whole flow)
    fn demo_step(&mut self, now: f64) {
        let Some((step, last)) = self.demo else { return };
        if now - last < 0.25 || (step > 0 && !self.link.idle()) {
            return; // each step waits for the machine
        }
        match step {
            0 => self.link.home(),
            2 => self.link.jog(10.0, 10.0, -3.0, 3000.0),
            4 => self.link.zero_axes("XYZ"),
            5 => self.link.jog(0.0, 0.0, 1.0, 3000.0),
            7 => {
                self.link.start(1);
                self.demo = None;
                return;
            }
            _ => {}
        }
        self.demo = Some((step + 1, now));
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || !self.link.connected() {
            return;
        }
        let (s, f) = (self.cfg.jog_step, self.cfg.jog_feed);
        // first press only: key_pressed also fires on auto-repeat (~30 a second), which queued jog after jog while a
        // key was held and kept the machine moving after it was let go
        let pressed = ctx.input(|i| {
            [egui::Key::Escape, egui::Key::Space, egui::Key::ArrowLeft, egui::Key::ArrowRight, egui::Key::ArrowUp, egui::Key::ArrowDown, egui::Key::PageUp, egui::Key::PageDown]
                .map(|k| i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, repeat: false, .. } if *key == k)))
        });
        if pressed[0] {
            self.link.stop();
            return;
        }
        if pressed[1] && (self.link.streaming || self.link.hold()) {
            if self.link.hold() { self.link.resume() } else { self.link.pause() }
            return;
        }
        let jogs = [(-s, 0.0, 0.0), (s, 0.0, 0.0), (0.0, s, 0.0), (0.0, -s, 0.0), (0.0, 0.0, s), (0.0, 0.0, -s)];
        for (k, (dx, dy, dz)) in jogs.into_iter().enumerate() {
            if pressed[k + 2] {
                self.link.jog(dx, dy, dz, f);
            }
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let connected = self.link.connected();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new("GCarve Control").family(crate::widgets::bold_family()).size(15.0));
            theme::vsep(ui, 24.0);
            let n = if Link::has_serial() { 3 } else { 2 };
            ui.add_enabled_ui(!connected, |ui| {
                crate::widgets::combo_box("transport").width(170.0).selected_text(TRANSPORTS[self.cfg.transport.min(n - 1)]).show_ui(ui, |ui| {
                    for (i, t) in TRANSPORTS.iter().enumerate().take(n) {
                        if ui.selectable_value(&mut self.cfg.transport, i, *t).clicked() && i == 2 {
                            self.serial_ports = Link::serial_ports();
                        }
                    }
                });
                if self.cfg.transport == 1 {
                    ui.add(crate::widgets::field(&mut self.cfg.host).hint_text("controller IP").desired_width(140.0));
                    ui.add(crate::widgets::field(&mut self.port_text).desired_width(60.0));
                }
                if self.cfg.transport == 2 && !cfg!(target_arch = "wasm32") {
                    crate::widgets::combo_box("serial").width(240.0).selected_text(self.cfg.serial_port.clone()).show_ui(ui, |ui| {
                        for p in &self.serial_ports {
                            ui.selectable_value(&mut self.cfg.serial_port, p.clone(), p);
                        }
                    });
                }
                if self.cfg.transport == 2 {
                    crate::widgets::combo_box("baud").width(100.0).selected_text(self.cfg.baud.to_string()).show_ui(ui, |ui| {
                        for b in [115200, 250000, 230400, 57600] {
                            ui.selectable_value(&mut self.cfg.baud, b, b.to_string());
                        }
                    });
                }
            });
            if self.cfg.transport == 2 && cfg!(target_arch = "wasm32") && !connected {
                dim(ui, "Connect, then pick the machine in the browser's list");
            }
            if button(ui, !self.link.streaming, if connected { "Disconnect" } else { "Connect" }).clicked() {
                if connected { self.link.disconnect() } else { self.connect() }
            }
            if IconButton::new("board").tip("Machine settings (travel, touch plate, warm-up)").show(ui).clicked() {
                let c = &self.cfg;
                self.settings = Some([c.travel[0], c.travel[1], c.travel[2], c.plate, c.probe_max, c.probe_feed, c.probe_lift, f64::from(c.warm_rpm), f64::from(c.warm_sec)].map(|v| format!("{}", (v * 1000.0).round() / 1000.0)));
            }
            // state pill
            let state = self.link.state().to_string();
            let col = state_color(&state);
            let g = ui.painter().layout_no_wrap(state, egui::FontId::proportional(theme::FONT_SIZE), col);
            let (r, _) = ui.allocate_exact_size(egui::vec2(g.size().x + 24.0, 24.0), egui::Sense::hover());
            ui.painter().rect(r, 10.0, Color32::from_black_alpha(64), egui::Stroke::new(1.0, col), egui::StrokeKind::Inside);
            ui.painter().galley(r.center() - g.size() / 2.0, g, col);
            dim(ui, self.link.transport());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // always reachable; the machine's own E-stop is still the real one
                if filled(ui, connected, "STOP", STOP_RED, 110.0).tip("Feed hold + reset: motion and spindle stop. Use the machine's E-stop for emergencies.").clicked() {
                    self.link.stop();
                }
                if !self.link.last_error.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&self.link.last_error).color(theme::DANGER)).truncate()).tip(&self.link.last_error);
                }
            });
        });
    }

    fn left(&mut self, ui: &mut egui::Ui) {
        let l = &mut self.link;
        let (idle, free) = (l.idle(), !l.streaming || l.tool_wait().is_some()); // a tool change frees jog / zero / probe
        ui.spacing_mut().item_spacing.y = 10.0;
        section(ui, "POSITION (work, in)");
        let (w, m) = (l.st.wpos(), l.st.mpos);
        for (i, a) in ["X", "Y", "Z"].into_iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized([22.0, 30.0], egui::Label::new(RichText::new(a).size(22.0).family(crate::widgets::bold_family()).color(AXIS[i])));
                ui.vertical(|ui| {
                    ui.set_width(104.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(fmt([w.x, w.y, w.z][i])).monospace().size(24.0).color(theme::TEXT));
                    ui.add(egui::Label::new(RichText::new(format!("machine {}", fmt([m.x, m.y, m.z][i]))).monospace().size(11.0).color(theme::TEXT_DIM)).extend());
                });
                if button(ui, idle && free, "Zero").clicked() {
                    l.zero_axes(a);
                }
            });
        }
        ui.horizontal(|ui| {
            if button(ui, idle && free, "Zero XY").clicked() {
                l.zero_axes("XY");
            }
            if button(ui, idle && free, "Zero XYZ").clicked() {
                l.zero_axes("XYZ");
            }
        });
        ui.horizontal(|ui| {
            if button(ui, l.connected() && free && (idle || l.alarm()), "Home").clicked() {
                l.home();
            }
            if button(ui, l.alarm() && free, "Unlock").tip("Clear the alarm without homing (position may be wrong)").clicked() {
                l.unlock();
            }
            if button(ui, idle && free, "Go to X0 Y0").tip("Lifts to the top first, then moves to the work zero").clicked() {
                l.go_to_work_zero();
            }
        });
        theme::hline(ui);
        section(ui, "JOG");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for s in [0.001, 0.01, 0.1, 1.0, 4.0] {
                let on = (self.cfg.jog_step - s).abs() < 1e-9;
                let b = egui::Button::new(RichText::new(format!("{s}")).size(11.0)).selected(on).min_size(egui::vec2(52.0, 26.0));
                if ui.add(b).clicked() {
                    self.cfg.jog_step = s;
                    self.cfg.save();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Jog feed");
            if theme::unit_field(ui, &mut self.jog_feed, "in/min", 110.0).lost_focus() {
                if let Some(v) = num(&self.jog_feed).filter(|v| *v > 0.0) {
                    self.cfg.jog_feed = v;
                    self.cfg.save();
                }
                self.jog_feed = format!("{:.0}", self.cfg.jog_feed);
            }
        });
        let jog_ok = (idle || l.state() == "Jog") && free;
        let (s, f) = (self.cfg.jog_step, self.cfg.jog_feed);
        ui.add_enabled_ui(jog_ok, |ui| {
            egui::Grid::new("jog").spacing([4.0, 4.0]).show(ui, |ui| {
                let cell = egui::vec2(56.0, 30.0);
                let b = |ui: &mut egui::Ui, t: &str| ui.add(egui::Button::new(t).min_size(cell)).clicked();
                let gap = |ui: &mut egui::Ui| ui.allocate_exact_size(cell, egui::Sense::hover());
                gap(ui);
                if b(ui, "Y+") { l.jog(0.0, s, 0.0, f) }
                gap(ui);
                if b(ui, "Z+") { l.jog(0.0, 0.0, s, f) }
                ui.end_row();
                if b(ui, "X−") { l.jog(-s, 0.0, 0.0, f) }
                if ui.add(egui::Button::new("■").min_size(cell)).tip("Cancel jog").clicked() { l.jog_stop() }
                if b(ui, "X+") { l.jog(s, 0.0, 0.0, f) }
                gap(ui);
                ui.end_row();
                gap(ui);
                if b(ui, "Y−") { l.jog(0.0, -s, 0.0, f) }
                gap(ui);
                if b(ui, "Z−") { l.jog(0.0, 0.0, -s, f) }
                ui.end_row();
            });
        });
        small_wrap(ui, "Keys: arrows = X/Y, Page Up/Down = Z · Space = hold/resume · Esc = stop");
        theme::hline(ui);
        section(ui, "TOUCH PLATE");
        small_wrap(ui, "Put the plate under the bit, clip the lead on the bit, then probe. Z zero = the plate's underside (board top or spoilboard).");
        ui.horizontal(|ui| {
            if button(ui, idle && free && !l.probing, if l.probing { "Probing…" } else { "Probe Z" }).clicked() {
                l.probe_z(self.cfg.plate, self.cfg.probe_max, self.cfg.probe_feed, self.cfg.probe_lift);
            }
            dim(ui, format!("plate {:.3} in", self.cfg.plate));
        });
        theme::hline(ui);
        ui.horizontal(|ui| {
            if button(ui, idle && free, "Park").tip("Lift to the top, then to the back-left corner").clicked() {
                l.park();
            }
            if button(ui, idle && free, "Warm up").tip(format!("{} RPM for {} s, then off", self.cfg.warm_rpm, self.cfg.warm_sec)).clicked() {
                l.spindle_warmup(self.cfg.warm_rpm, self.cfg.warm_sec);
            }
            let on = l.st.rpm > 0.0;
            if button(ui, idle && free, if on { "Spindle off" } else { "Spindle on" }).clicked() {
                l.spindle(!on, self.cfg.warm_rpm);
            }
        });
    }

    fn right(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, now: f64) {
        ui.spacing_mut().item_spacing.y = 8.0;
        section(ui, "PROGRAM");
        let free = !self.link.streaming;
        ui.horizontal(|ui| {
            if button(ui, free, "Open…").clicked() {
                self.files.pick_into(ctx, "G-code", &["nc", "gcode", "ngc", "tap", "txt"], "gcode");
            }
            let name = if self.link.program_name.is_empty() { "No program".to_string() } else { self.link.program_name.clone() };
            ui.add(egui::Label::new(name).truncate());
        });
        if button(ui, free, "Surface the spoilboard…").on_hover_text("Makes a flattening program for the area you give: zero X/Y at its corner and Z on the spoilboard top first").clicked() {
            self.surfacing = Some(["24", "24", "1.0", "40", "0.02", "1", "100", "18000"].map(String::from));
        }
        if !self.queue.is_empty() {
            ui.label(RichText::new("From GCarve").size(11.0).color(theme::TEXT_DIM));
            let mut pick = None;
            egui::ScrollArea::vertical().id_salt("queue").max_height(120.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (name, key) in &self.queue {
                    if ui.add_enabled(free, egui::Button::new(name).frame(false).min_size(egui::vec2(ui.available_width(), 28.0))).clicked() {
                        pick = Some(key.clone());
                    }
                }
            });
            if let Some(k) = pick {
                self.load_queued(&k);
            }
        }
        let p = &self.link.program;
        if !p.lines.is_empty() {
            let has = p.min.x < 1e299;
            let r = |a: f64, b: f64| if has { format!("{a:.3} … {b:.3}") } else { "0.000 … 0.000".into() };
            let row_h = std::mem::replace(&mut ui.spacing_mut().interact_size.y, 16.0); // label rows, like the Qt GridLayout
            egui::Grid::new("info").spacing([10.0, 2.0]).show(ui, |ui| {
                for (k, v) in [("Lines", p.lines.len().to_string()), ("Est. time", format!("{:.1} min", p.minutes)), ("X range", r(p.min.x, p.max.x)), ("Y range", r(p.min.y, p.max.y)), ("Z range", r(p.min.z, p.max.z))] {
                    dim(ui, k);
                    ui.label(v);
                    ui.end_row();
                }
                dim(ui, "Spindle off at end");
                if p.spindle_off_at_end { ui.label("yes") } else { ui.label(RichText::new("no (M5 is added)").color(theme::WARNING)) };
                ui.end_row();
            });
            ui.spacing_mut().interact_size.y = row_h;
            for e in p.errors.iter().take(5) {
                ui.add(egui::Label::new(RichText::new(format!("⚠ {e}")).size(11.0).color(theme::DANGER)).wrap());
            }
        }
        if self.link.streaming || self.link.progress() > 0.0 {
            ui.add(egui::ProgressBar::new(self.link.progress() as f32).desired_height(8.0));
        }
        if self.link.streaming {
            let secs = (now - self.started_at.unwrap_or(now)).max(0.0) as u64;
            dim(ui, format!("Line {} of {} · about {} min left · {}:{:02} elapsed", self.link.progress_line(), p.lines.len(), self.link.minutes_left().ceil(), secs / 60, secs % 60));
        }
        // a program waiting at a tool change (M6): what to put in, then carry on
        if let Some(bit) = self.link.tool_wait() {
            egui::Frame::new().fill(theme::WARNING.gamma_multiply(0.18)).stroke(egui::Stroke::new(1.0, theme::WARNING)).corner_radius(6).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Tool change").family(crate::widgets::bold_family()).color(theme::WARNING));
                ui.add(egui::Label::new(RichText::new(format!("Put in {bit}.")).size(14.0)).wrap());
                small_wrap(ui, "The spindle is off. Change the bit, touch off Z again (Probe Z or zero Z by hand: the new bit is a different length), then Continue.");
                ui.horizontal(|ui| {
                    if filled(ui, self.link.idle(), "Continue", OK_GREEN, 110.0).clicked() {
                        self.link.continue_after_tool();
                    }
                    if ui.button("Stop the job").clicked() {
                        self.link.stop();
                    }
                });
            });
        }
        ui.horizontal(|ui| {
            if filled(ui, self.link.connected() && free && !self.link.program_name.is_empty(), "Start", OK_GREEN, 90.0).clicked() {
                self.open_start();
            }
            let hold = self.link.hold();
            if ui.add_enabled(self.link.connected() && (self.link.streaming || hold), egui::Button::new(if hold { "Resume" } else { "Hold" }).min_size(egui::vec2(90.0, 0.0))).clicked() {
                if hold { self.link.resume() } else { self.link.pause() }
            }
        });
        let c = self.link.connected();
        if ui.add_enabled(c && free && !self.link.program.motions.is_empty(), egui::Button::new("Outline the job")).on_hover_text("Traces the cut's outer box above the board, bit up and spindle off: check it fits before you start").clicked() {
            self.link.outline();
        }
        section(ui, &format!("FEED {}% · RAPID {}% · SPINDLE {}%", self.link.st.feed_ov, self.link.st.rapid_ov, self.link.st.spindle_ov));
        let row = |ui: &mut egui::Ui, name: &str| {
            ui.add_sized([56.0, 20.0], egui::Label::new(RichText::new(name).size(11.0).color(theme::TEXT_DIM)));
        };
        ui.horizontal(|ui| {
            row(ui, "Feed");
            for (t, s) in [("−10%", -1), ("100%", 0), ("+10%", 1)] {
                if button(ui, c, t).clicked() {
                    self.link.override_feed(s);
                }
            }
        });
        ui.horizontal(|ui| {
            row(ui, "Rapid");
            for p in [25, 50, 100] {
                if button(ui, c, &format!("{p}%")).clicked() {
                    self.link.override_rapid(p);
                }
            }
        });
        ui.horizontal(|ui| {
            row(ui, "Spindle");
            for (t, s) in [("−10%", -1), ("100%", 0), ("+10%", 1)] {
                if button(ui, c, t).clicked() {
                    self.link.override_spindle(s);
                }
            }
        });
        dim(ui, format!("Feed {:.0} in/min · Spindle {:.0} RPM", self.link.st.feed, self.link.st.rpm));
    }

    fn console(&mut self, ui: &mut egui::Ui) {
        let h = ui.available_height() - theme::CONTROL_HEIGHT - 4.0;
        egui::ScrollArea::vertical().id_salt("log").max_height(h).min_scrolled_height(h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for l in &self.link.log {
                let bad = l.starts_with("ALARM") || l.starts_with("error");
                ui.label(RichText::new(l).monospace().size(11.0).color(if bad { theme::DANGER } else { theme::TEXT_DIM }));
            }
        });
        let ok = self.link.connected() && !self.link.streaming;
        let r = ui.add_enabled(ok, crate::widgets::field(&mut self.mdi).hint_text("Send a command (MDI), e.g. G0 X0 Y0 — Enter to send").desired_width(f32::INFINITY));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let cmd = std::mem::take(&mut self.mdi);
            self.link.send_command(&cmd);
            r.request_focus();
        }
    }

    fn surfacing_ui(&mut self, ctx: &egui::Context) {
        let Some(buf) = &mut self.surfacing else { return };
        let (mut ok, mut close) = (false, false);
        crate::widgets::Dialog::new("Surface the spoilboard").show(ctx, |ui| {
            small_wrap(ui, "Flattens the area from the work zero: set X/Y zero at its front-left corner and Z zero on the spoilboard top. The bit's centre goes to every edge, so the whole area is cut.");
            egui::Grid::new("surfacing").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                for (i, (label, unit)) in [("Width (X)", "in"), ("Length (Y)", "in"), ("Bit diameter", "in"), ("Stepover", "%"), ("Depth per pass", "in"), ("Passes", ""), ("Feed", "in/min"), ("Spindle", "rpm")].into_iter().enumerate() {
                    ui.label(label);
                    theme::unit_field(ui, &mut buf[i], unit, 120.0);
                    ui.end_row();
                }
            });
            (ok, close) = crate::widgets::ok_cancel(ui);
        });
        if ok {
            let v: Vec<f64> = buf.iter().map(|s| num(s).unwrap_or(0.0)).collect();
            if v[0] <= 0.0 || v[1] <= 0.0 || v[2] <= 0.0 || v[4] <= 0.0 || v[6] <= 0.0 {
                self.link.last_error = "Surfacing: width, length, bit, depth and feed must be above zero".into();
            } else {
                let g = gcarve_core::grbl::surfacing_program(v[0], v[1], v[2], v[3] / 100.0, v[4], v[5].max(1.0) as u32, v[6], v[7]);
                self.link.load_program("Spoilboard surfacing.nc", &g);
            }
        }
        if ok || close {
            self.surfacing = None;
        }
    }

    fn settings_ui(&mut self, ctx: &egui::Context) {
        let Some(buf) = &mut self.settings else { return };
        let mut close = false;
        let mut ok = false;
        crate::widgets::Dialog::new("Machine settings").show(ctx, |ui| {
            egui::Grid::new("machine-settings").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                let rows: [(Option<&str>, &str, &str); 9] = [
                    (Some("Travel X / Y / Z (in)"), "X", "in"),
                    (None, "Y", "in"),
                    (None, "Z (down from the top)", "in"),
                    (Some("Touch plate"), "Plate thickness", "in"),
                    (None, "Max probe travel", "in"),
                    (None, "Probe feed", "in/min"),
                    (None, "Lift after touch", "in"),
                    (Some("Spindle warm-up"), "RPM", "rpm"),
                    (None, "Seconds", "s"),
                ];
                for (i, (head, label, unit)) in rows.into_iter().enumerate() {
                    if let Some(h) = head {
                        ui.label(RichText::new(h).font(crate::widgets::bold(theme::FONT_SIZE)));
                        ui.end_row();
                    }
                    ui.label(label);
                    theme::unit_field(ui, &mut buf[i], unit, 120.0);
                    ui.end_row();
                }
            });
            (ok, close) = crate::widgets::ok_cancel(ui);
        });
        if ok {
            let v: Vec<Option<f64>> = buf.iter().map(|s| num(s)).collect();
            let c = &mut self.cfg;
            let set = |dst: &mut f64, i: usize| {
                if let Some(x) = v[i].filter(|x| *x >= 0.0) {
                    *dst = x;
                }
            };
            for i in 0..3 {
                set(&mut c.travel[i], i);
            }
            set(&mut c.plate, 3);
            set(&mut c.probe_max, 4);
            set(&mut c.probe_feed, 5);
            set(&mut c.probe_lift, 6);
            if let Some(x) = v[7] {
                c.warm_rpm = x as i32;
            }
            if let Some(x) = v[8] {
                c.warm_sec = x as i32;
            }
            c.save();
            self.link.set_travel(Vec3 { x: c.travel[0], y: c.travel[1], z: c.travel[2] });
        }
        if ok || close {
            self.settings = None;
        }
    }

    fn start_ui(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.start else { return };
        let lines = self.link.program.lines.len();
        let mut go = None;
        let mut cancel = false;
        crate::widgets::Dialog::new("Start program").width(480.0).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let wrap = |ui: &mut egui::Ui, t: RichText| {
                ui.add(egui::Label::new(t).wrap());
            };
            if !d.errors.is_empty() {
                ui.label(RichText::new("Can't start:").color(theme::DANGER).font(crate::widgets::bold(theme::FONT_SIZE)));
                for e in &d.errors {
                    wrap(ui, RichText::new(format!("• {e}")).color(theme::DANGER));
                }
            } else {
                if !d.warnings.is_empty() {
                    ui.label(RichText::new("Check before starting:").color(theme::WARNING).font(crate::widgets::bold(theme::FONT_SIZE)));
                    for w in &d.warnings {
                        wrap(ui, RichText::new(format!("• {w}")));
                    }
                }
                wrap(ui, RichText::new("The spindle will start and the machine will move. Bit in, Z zero set, material held down, hands and tools clear of the machine."));
                ui.horizontal(|ui| {
                    theme::checkbox(ui, &mut d.from_on, "Start from line");
                    ui.add_enabled(d.from_on, crate::widgets::field(&mut d.from).desired_width(90.0));
                    dim(ui, format!("of {lines}"));
                });
                if d.from_on {
                    wrap(ui, RichText::new("Resume: the machine lifts to the top, starts the spindle, moves over the restart point and feeds down slowly. Check the zero is unchanged and the bit is the same one.").color(theme::WARNING));
                }
                theme::checkbox(ui, &mut d.ack, "I checked all of this");
            }
            ui.add_space(20.0);
            crate::widgets::right(ui, 84.0 * 2.0 + 8.0, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if ui.add_enabled(d.errors.is_empty() && d.ack, crate::widgets::Button::new("Start").primary(true).min_width(84.0)).clicked() {
                    go = Some(if d.from_on { num(&d.from).map_or(1, |v| v.round().max(1.0) as usize) } else { 1 });
                }
                cancel = ui.add(crate::widgets::Button::new("Cancel").min_width(84.0)).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if let Some(line) = go {
            self.link.start(line);
        }
        if go.is_some() || cancel {
            self.start = None;
        }
    }
}

impl eframe::App for ControlApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let was_streaming = self.link.streaming;
        self.link.update(now);
        self.demo_step(now);
        if self.link.connected() || self.demo.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(20));
        }
        if self.link.streaming && (!was_streaming || self.started_at.is_none()) {
            self.started_at = Some(now);
        }
        if now - self.queue_scan > 1.0 {
            // files sent from GCarve appear while Control is open
            self.queue_scan = now;
            self.queue = super::queue();
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        let picked: Vec<_> = self.files.aux_rx.try_iter().map(|(_, n, b)| (n, b)).chain(self.files.poll(&ctx).into_iter().map(|p| (p.name, p.bytes))).collect();
        for (name, bytes) in picked {
            self.link.load_program(&name, &String::from_utf8_lossy(&bytes));
        }
        self.keys(&ctx);
        #[cfg(all(feature = "screenshot", not(target_arch = "wasm32")))]
        {
            self.shot_frame += 1;
            let at: u64 = std::env::var("GCARVE_SHOT_FRAME").ok().and_then(|s| s.parse().ok()).unwrap_or(6);
            let frame = match self.shot_ms {
                Some(ms) if now * 1000.0 < ms => 0,
                Some(_) => {
                    self.shot_ms = None;
                    self.shot_frame = at;
                    at
                }
                None => self.shot_frame,
            };
            crate::shot::tick(&ctx, frame);
        }
        let title = if self.link.program_name.is_empty() { "GCarve Control".to_string() } else { format!("GCarve Control — {}", self.link.program_name) };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        egui::Panel::top("control-header").exact_size(34.0).frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin::symmetric(10, 0))).show(ui, |ui| self.header(ui));
        egui::Panel::left("control-left").resizable(false).exact_size(300.0).frame(egui::Frame::new().fill(theme::PANEL).inner_margin(12)).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("left").show(ui, |ui| self.left(ui));
        });
        egui::Panel::right("control-right").resizable(false).exact_size(320.0).frame(egui::Frame::new().fill(theme::PANEL).inner_margin(12)).show(ui, |ui| self.right(ui, &ctx, now));
        egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
            egui::Panel::bottom("control-console").resizable(false).exact_size(170.0).frame(egui::Frame::new().fill(theme::PANEL).inner_margin(6)).show(ui, |ui| self.console(ui));
            egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::CANVAS_BG)).show(ui, |ui| {
                self.preview.ui(ui, &self.link);
                if self.link.program_name.is_empty() {
                    let r = ui.max_rect();
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Open a G-code file, or send one from GCarve (File › Send to GCarve Control)", egui::FontId::proportional(theme::FONT_SIZE), theme::TEXT_DIM);
                }
            });
        });
        self.settings_ui(&ctx);
        self.surfacing_ui(&ctx);
        self.start_ui(&ctx);
        crate::widgets::focus_ring(&ctx);
    }
}
