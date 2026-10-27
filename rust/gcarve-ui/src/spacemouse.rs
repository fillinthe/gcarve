//! 3Dconnexion SpaceMouse for the 3D view: the puck's six axes (−1..1) read straight from the HID device, no
//! 3Dconnexion driver API. Web = WebHID (Chrome, Edge, Opera on a computer; the browser's chooser needs the click on
//! Connect, a device allowed before reconnects by itself). Desktop = hidapi (feature `spacemouse`) on a reader thread.
//! One device per app. The app polls it (`App::puck`): the 3D view orbits, the drawing pans and zooms.

/// full deflection of an axis in device counts
const FULL: f32 = 350.0;

/// One input report into the axes [x, y, z, rx, ry, rz]: report 1 = translation (wireless pucks: + rotation in the
/// same report), report 2 = rotation; three little-endian i16 each. Other reports (3 = buttons) are ignored.
pub fn parse(id: u8, d: &[u8], a: &mut [f32; 6]) {
    let mut put = |at: usize, d: &[u8]| {
        for (k, c) in d.as_chunks::<2>().0.iter().take(3).enumerate() {
            a[at + k] = (f32::from(i16::from_le_bytes(*c)) / FULL).clamp(-1.0, 1.0);
        }
    };
    match (id, d.len()) {
        (1, 12..) => {
            put(0, &d[..6]);
            put(3, &d[6..12]);
        }
        (1, 6..) => put(0, d),
        (2, 6..) => put(3, d),
        _ => {}
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = r#"
let dev = null, rep = {}, state = "", heard = false;
const vendors = [0x256f, 0x046d];
const puck = d => d.collections.some(c => c.usagePage === 1 && c.usage === 8);
if (navigator.hid) navigator.hid.addEventListener("disconnect", e => { if (e.device === dev) { dev = null; rep = {}; state = ""; heard = false; } });
export function gc_sm_supported() { return !!navigator.hid; }
export function gc_sm_state() { return state; }
export function gc_sm_open(ask) {
  if (!navigator.hid || dev) return;
  const list = ask ? navigator.hid.requestDevice({ filters: [{ vendorId: vendors[0] }, { vendorId: vendors[1], usagePage: 1, usage: 8 }] })
                   : navigator.hid.getDevices().then(l => l.filter(d => vendors.includes(d.vendorId)));
  list.then(async l => {
    const d = l.find(puck) || (ask ? l[0] : null);
    if (!d) return;
    if (!d.opened) await d.open();
    dev = d; rep = {}; state = "open:" + (d.productName || "SpaceMouse");
    d.addEventListener("inputreport", e => { heard = true; rep[e.reportId] = new Uint8Array(e.data.buffer.slice(e.data.byteOffset, e.data.byteOffset + e.data.byteLength)); });
  }).catch(e => { state = "error: " + e.message; });
}
export function gc_sm_report(id) { return rep[id] || new Uint8Array(0); }
export function gc_sm_heard() { return heard; }
"#)]
    extern "C" {
        fn gc_sm_supported() -> bool;
        fn gc_sm_state() -> String;
        fn gc_sm_open(ask: bool);
        fn gc_sm_report(id: u8) -> Vec<u8>;
        fn gc_sm_heard() -> bool;
    }

    pub fn supported() -> bool {
        gc_sm_supported()
    }
    pub fn state() -> String {
        gc_sm_state()
    }
    pub fn connect(ask: bool) {
        gc_sm_open(ask);
    }
    pub fn heard() -> bool {
        gc_sm_heard()
    }
    pub fn axes() -> Option<[f32; 6]> {
        if !gc_sm_state().starts_with("open") {
            return None;
        }
        let mut a = [0.0; 6];
        for id in [1, 2] {
            super::parse(id, &gc_sm_report(id), &mut a); // the newest report of each kind = where the puck is now
        }
        Some(a)
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "spacemouse"))]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    /// 3Dconnexion, and Logitech for the older pucks (those are told apart by the multi-axis usage 1:8)
    const VENDORS: [u16; 2] = [0x256f, 0x046d];
    /// "" / "open:<name>" / "error: …", and the axes; written by the reader thread
    static SHARED: Mutex<(String, [f32; 6])> = Mutex::new((String::new(), [0.0; 6]));
    static RUNNING: AtomicBool = AtomicBool::new(false);
    static HEARD: AtomicBool = AtomicBool::new(false);

    fn set(state: String, axes: [f32; 6]) {
        *SHARED.lock().unwrap_or_else(|e| e.into_inner()) = (state, axes);
    }

    pub fn supported() -> bool {
        true
    }
    pub fn state() -> String {
        SHARED.lock().unwrap_or_else(|e| e.into_inner()).0.clone()
    }
    fn open() -> Result<Option<(hidapi::HidDevice, String)>, hidapi::HidError> {
        let api = hidapi::HidApi::new()?;
        #[cfg(target_os = "macos")]
        api.set_open_exclusive(false); // the 3Dconnexion driver, when installed, keeps the device open too
        let Some(info) = api.device_list().find(|i| VENDORS.contains(&i.vendor_id()) && i.usage_page() == 1 && i.usage() == 8) else { return Ok(None) };
        Ok(Some((info.open_device(&api)?, info.product_string().unwrap_or(crate::i18n::t("SpaceMouse")).to_string())))
    }
    /// Own thread: on macOS hidapi spins the calling thread's run loop, which re-enters the window's event handling
    /// (winit aborts). `ask` = the user pressed Connect: say so when nothing is found (the silent try at start doesn't).
    pub fn connect(ask: bool) {
        if RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(move || {
            match open() {
                Ok(Some((hid, name))) => {
                    let (mut axes, mut buf) = ([0.0; 6], [0u8; 64]);
                    set(format!("open:{name}"), axes);
                    // a quarter second without a report = still where it was (the puck reports only while it moves)
                    while let Ok(n) = hid.read_timeout(&mut buf, 250) {
                        if n > 0 {
                            HEARD.store(true, Ordering::Relaxed);
                            super::parse(buf[0], &buf[1..n], &mut axes); // byte 0 = report number
                            set(format!("open:{name}"), axes);
                        }
                    }
                    set(String::new(), [0.0; 6]); // unplugged
                    HEARD.store(false, Ordering::Relaxed);
                }
                Ok(None) => set(if ask { "error: no SpaceMouse found".to_string() } else { String::new() }, [0.0; 6]),
                Err(e) => set(if ask { format!("error: {e}") } else { String::new() }, [0.0; 6]),
            }
            RUNNING.store(false, Ordering::SeqCst);
        });
    }
    pub fn heard() -> bool {
        HEARD.load(Ordering::Relaxed)
    }
    pub fn axes() -> Option<[f32; 6]> {
        let s = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        s.0.starts_with("open").then_some(s.1)
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "spacemouse")))]
mod imp {
    pub fn supported() -> bool {
        false
    }
    pub fn state() -> String {
        String::new()
    }
    pub fn connect(_ask: bool) {}
    pub fn heard() -> bool {
        false
    }
    pub fn axes() -> Option<[f32; 6]> {
        None
    }
}

pub use imp::{axes, connect, heard, state, supported};

/// What to tell the user: (line, can connect now). A puck that is open but silent is almost always held by the
/// 3Dconnexion driver.
pub fn status() -> (String, bool) {
    let s = state();
    if let Some(name) = s.strip_prefix("open:") {
        let quiet = if heard() { "" } else { crate::i18n::t(": nothing received yet. Move the puck; if it stays silent, quit the 3Dconnexion driver and reload") };
        (format!("{name} connected{quiet}"), false)
    } else if !supported() {
        (crate::i18n::t("SpaceMouse: needs Chrome or Edge on a computer").to_string(), false)
    } else {
        (s.strip_prefix("error: ").unwrap_or("").to_string(), true)
    }
}

/// Turns the puck's raw axes into what the view should follow. A real puck is noisy and its axes lean on each other
/// (twisting also pushes sideways a little), which showed as a jittery view: a dead zone round the centre, a curve
/// that makes small pushes gentle, the stronger group (slide or turn) wins when one clearly dominates, and a short
/// low-pass so a 60 Hz stream doesn't step the picture. `step` returns None once the puck is at rest and the filter
/// has settled.
#[derive(Default)]
pub struct Filter {
    s: [f32; 6],
}

const DEAD: f32 = 0.07;

impl Filter {
    pub fn step(&mut self, raw: [f32; 6], dt: f32) -> Option<[f32; 6]> {
        let mut a = raw.map(|v| {
            let m = ((v.abs() - DEAD) / (1.0 - DEAD)).clamp(0.0, 1.0);
            v.signum() * m * m * (0.5 + 0.5 * m) // gentle near the middle, full at the stop
        });
        let (t, r) = (a[..3].iter().map(|v| v * v).sum::<f32>().sqrt(), a[3..].iter().map(|v| v * v).sum::<f32>().sqrt());
        if r > t * 1.6 {
            a[..3].fill(0.0);
        } else if t > r * 1.6 {
            a[3..].fill(0.0);
        }
        let k = 1.0 - (-dt / 0.06).exp();
        for (s, v) in self.s.iter_mut().zip(a) {
            *s += (v - *s) * k;
        }
        let moving = self.s.iter().any(|v| v.abs() > 0.004);
        if !moving {
            self.s = [0.0; 6];
        }
        moving.then_some(self.s)
    }
}

/// The last seconds of raw puck input, for a report when a view misbehaves: (seconds, raw axes) per app frame.
#[derive(Default)]
pub struct Trace {
    rows: std::collections::VecDeque<(f64, [f32; 6])>,
}

impl Trace {
    pub fn push(&mut self, t: f64, raw: [f32; 6]) {
        if self.rows.len() >= 1800 {
            self.rows.pop_front();
        }
        self.rows.push_back((t, raw));
    }
    pub fn csv(&self) -> String {
        let mut out = String::from("seconds,x,y,z,rx,ry,rz\n");
        for (t, a) in &self.rows {
            out += &format!("{t:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}\n", a[0], a[1], a[2], a[3], a[4], a[5]);
        }
        out
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn noise_is_ignored_and_the_turn_wins_over_a_little_slide() {
        let mut f = super::Filter::default();
        assert!(f.step([0.05, -0.04, 0.03, 0.0, 0.02, 0.0], 0.016).is_none());
        let mut out = None;
        for _ in 0..30 {
            out = f.step([0.15, 0.0, 0.1, 0.0, 0.0, 0.8], 0.016); // a twist with some sideways lean
        }
        let o = out.expect("moving");
        assert!(o[5] > 0.25 && o[0] == 0.0 && o[2] == 0.0, "{o:?}");
        for _ in 0..200 {
            out = f.step([0.0; 6], 0.016);
        }
        assert!(out.is_none());
    }

    #[test]
    fn reports_fill_the_axes() {
        let mut a = [0.0; 6];
        super::parse(1, &[94, 1, 0, 0, 81, 255], &mut a); // x = 350, z = -175
        super::parse(2, &[0, 0, 0, 0, 232, 3], &mut a); // rz = 1000: clamped
        assert_eq!(a, [1.0, 0.0, -0.5, 0.0, 0.0, 1.0]);
        let mut b = [0.0; 6];
        super::parse(1, &[0, 0, 175, 0, 0, 0, 162, 254, 0, 0, 0, 0], &mut b); // wireless: both halves in report 1
        assert_eq!(b, [0.0, 0.5, 0.0, -1.0, 0.0, 0.0]);
        super::parse(3, &[1, 0], &mut b); // buttons: ignored
        super::parse(2, &[1], &mut b); // short: ignored
        assert_eq!(b, [0.0, 0.5, 0.0, -1.0, 0.0, 0.0]);
    }
}
