//! USB serial from the browser (Web Serial: Chrome, Edge, Opera on desktop), shared by GCarve and GBurn. The browser picks the device in its own
//! chooser (needs the click that pressed Connect, still within the page's user activation), then a JS read loop fills
//! an inbox the Link drains every frame. One port per page.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
let port = null, writer = null, reader = null, inbox = [], state = "", gen = 0, onHide = null, heldOnHide = false;
// a hidden tab gets no frames, so nothing feeds the controller: send the app's "pause" bytes the moment the page hides
document.addEventListener("visibilitychange", () => {
  if (document.hidden && writer && onHide && onHide.length) {
    writer.write(Uint8Array.from(onHide)).catch(() => {});
    heldOnHide = true;
  }
});
export function gc_serial_supported() { return !!(navigator.serial); }
export function gc_serial_state() { return state; }
export function gc_serial_open(baud, hardware) {
  if (!navigator.serial) { state = "error: this browser has no Web Serial (use Chrome or Edge on a computer)"; return; }
  const my = ++gen; // a later open or close makes this one stale: its loop and its late port never touch the new state
  state = "opening"; inbox = [];
  navigator.serial.requestPort({}).then(async p => {
    if (my !== gen) return; // closed or reopened while the chooser was up: leave the device alone
    await p.open({ baudRate: baud, bufferSize: 8192, flowControl: hardware ? "hardware" : "none" });
    if (my !== gen) { try { await p.close(); } catch (_) {} return; }
    port = p;
    writer = port.writable.getWriter();
    const info = port.getInfo();
    state = "open:" + (info.usbVendorId ? "USB " + info.usbVendorId.toString(16).padStart(4, "0") + ":" + info.usbProductId.toString(16).padStart(4, "0") : "serial port");
    const r = reader = port.readable.getReader();
    try {
      for (;;) {
        const { value, done } = await r.read();
        if (done) break;
        if (my === gen) for (const b of value) inbox.push(b);
      }
    } catch (e) { if (my === gen && state.startsWith("open:")) state = "lost: " + e.message; }
    finally { try { r.releaseLock(); } catch (_) {} }
    if (my === gen && state.startsWith("open:")) state = "lost: the device closed";
  }).catch(e => { if (my === gen) state = e.name === "NotFoundError" ? "cancelled" : "error: " + e.message; });
}
export function gc_serial_take() { const out = Uint8Array.from(inbox); inbox = []; return out; }
export function gc_serial_write(bytes) {
  if (!writer) return false;
  writer.write(Uint8Array.from(bytes)).catch(e => { state = "lost: " + e.message; });
  return true;
}
export function gc_serial_on_hide(bytes) { onHide = bytes.length ? Uint8Array.from(bytes) : null; }
export function gc_serial_held_on_hide() { const h = heldOnHide; heldOnHide = false; return h; }
export function gc_serial_close() {
  gen++; state = ""; onHide = null;
  const p = port, r = reader, w = writer;
  port = reader = writer = null; inbox = [];
  (async () => {
    try { if (r) await r.cancel(); } catch (_) {}
    try { if (w) { w.releaseLock(); } } catch (_) {}
    try { if (p) await p.close(); } catch (_) {}
  })();
}
"#)]
extern "C" {
    fn gc_serial_supported() -> bool;
    fn gc_serial_state() -> String;
    fn gc_serial_open(baud: u32, hardware: bool);
    fn gc_serial_take() -> Vec<u8>;
    fn gc_serial_write(bytes: &[u8]) -> bool;
    fn gc_serial_close();
    fn gc_serial_on_hide(bytes: &[u8]);
    fn gc_serial_held_on_hide() -> bool;
}

pub fn supported() -> bool {
    gc_serial_supported()
}

/// Shows the browser's device chooser and opens the picked port.
pub fn open(baud: u32) {
    gc_serial_open(baud, false);
}
/// `open` with RTS / CTS hardware flow control (a Ruida's FTDI port, as the desktop opens it)
pub fn open_hardware_flow(baud: u32) {
    gc_serial_open(baud, true);
}

pub enum State {
    Opening,
    /// open; the device description
    Open(String),
    Cancelled,
    /// failed to open, or the device went away
    Failed(String),
}

pub fn state() -> State {
    let s = gc_serial_state();
    if let Some(d) = s.strip_prefix("open:") {
        State::Open(d.to_string())
    } else if s == "cancelled" {
        State::Cancelled
    } else if let Some(e) = s.strip_prefix("error: ").or_else(|| s.strip_prefix("lost: ")) {
        State::Failed(e.to_string())
    } else {
        State::Opening
    }
}

pub fn take() -> Vec<u8> {
    gc_serial_take()
}

pub fn write(bytes: &[u8]) -> bool {
    gc_serial_write(bytes)
}

pub fn close() {
    gc_serial_close();
}

/// Bytes the page writes by itself the moment it is hidden (a hidden tab gets no frames to keep a job fed); empty = none.
pub fn on_hide(bytes: &[u8]) {
    gc_serial_on_hide(bytes);
}
/// true once after the page wrote the `on_hide` bytes
pub fn held_on_hide() -> bool {
    gc_serial_held_on_hide()
}
