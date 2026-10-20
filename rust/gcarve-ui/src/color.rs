//! Colours you can type, mix, keep and pick off the screen: hex / RGB / HSV boxes over egui's picker, a row of
//! colours saved on this device, and the browser's EyeDropper (Chrome / Edge sample anywhere on the screen).

use crate::{store, theme};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};

/// "#a1b2c3", "a1b2c3", "#abc", "rgb(10, 20, 30)", "10,20,30" or "10 20 30".
pub fn parse(s: &str) -> Option<[u8; 3]> {
    let t = s.trim().trim_start_matches("rgb").trim_matches(|c| c == '(' || c == ')').trim();
    let h = t.trim_start_matches('#');
    if h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        let v = u32::from_str_radix(h, 16).ok()?;
        return Some([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    }
    if h.len() == 3 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        let d: Vec<u8> = h.chars().map(|c| c.to_digit(16).unwrap() as u8 * 17).collect();
        return Some([d[0], d[1], d[2]]);
    }
    let n: Vec<u8> = t.split(|c: char| c == ',' || c.is_whitespace()).filter(|x| !x.is_empty()).map(|x| x.parse().ok()).collect::<Option<_>>()?;
    (n.len() == 3).then(|| [n[0], n[1], n[2]])
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Hue 0–360°, saturation and value 0–100 %, of the screen colour (as paint programs show it).
pub fn to_hsv(c: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = c.map(|x| x as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    [h, if max == 0.0 { 0.0 } else { d / max * 100.0 }, max * 100.0]
}

pub fn from_hsv([h, s, v]: [f32; 3]) -> [u8; 3] {
    let (s, v) = (s.clamp(0.0, 100.0) / 100.0, v.clamp(0.0, 100.0) / 100.0);
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r, g, b].map(|k| ((k + v - c) * 255.0).round() as u8)
}

/// Colours saved on this device (store key `my_colors`, hex, newest first).
pub fn saved() -> Vec<[u8; 3]> {
    store::load("my_colors").map(|s| s.split(';').filter_map(parse).collect()).unwrap_or_default()
}
fn set_saved(v: &[[u8; 3]]) {
    store::save("my_colors", &v.iter().map(|&c| hex(c)).collect::<Vec<_>>().join(";"));
}
/// Keep `c` (first in the row; at most 48 kept).
pub fn save(c: [u8; 3]) {
    let mut v = saved();
    v.retain(|&x| x != c);
    v.insert(0, c);
    v.truncate(48);
    set_saved(&v);
}
pub fn forget(c: [u8; 3]) {
    let mut v = saved();
    v.retain(|&x| x != c);
    set_saved(&v);
}

/// A row of swatches: (the one clicked, the one removed through its right-click menu when `removable`). `on` is
/// outlined.
pub fn swatches(ui: &mut egui::Ui, colors: &[[u8; 3]], on: Option<[u8; 3]>, removable: bool) -> (Option<[u8; 3]>, Option<[u8; 3]>) {
    let (mut hit, mut gone) = (None, None);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(4.0);
        for &c in colors {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
            let sel = on == Some(c);
            ui.painter().rect(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]), Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { theme::ACCENT } else { theme::BORDER_STRONG }), egui::StrokeKind::Inside);
            let resp = resp.on_hover_text(hex(c));
            if resp.clicked() {
                hit = Some(c);
            }
            if removable {
                resp.context_menu(|ui| {
                    if ui.button("Remove from My colours").clicked() {
                        gone = Some(c);
                        ui.close();
                    }
                });
            }
        }
    });
    (hit, gone)
}

/// The colour editor: a saturation / brightness square and a hue bar to drag (as wide as the box), a hex box,
/// R G B and H S V boxes, and (where the browser has it) a screen eyedropper. True when `c` changed.
pub fn editor(ui: &mut egui::Ui, id: &str, c: &mut [u8; 3]) -> bool {
    let mut changed = false;
    if let Some(p) = take_picked(id) {
        *c = p;
        changed = true;
    }
    // the hue of a grey is kept while you drag saturation up from 0 (shared with the H S V boxes)
    let hsv_key = egui::Id::new(("hsv", id));
    let mut hsv: [f32; 3] = ui.data_mut(|d| d.get_temp(hsv_key)).filter(|h| from_hsv(*h) == *c).unwrap_or_else(|| to_hsv(*c));
    if sv_square(ui, &mut hsv) | hue_bar(ui, &mut hsv) {
        *c = from_hsv(hsv);
        ui.data_mut(|d| d.insert_temp(hsv_key, hsv));
        changed = true;
    }
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(28.0, 20.0), Sense::hover());
        ui.painter().rect(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]), Stroke::new(1.0, theme::BORDER_STRONG), egui::StrokeKind::Inside);
        // hex: kept as typed while the box has focus, read on Enter / leaving it
        let key = egui::Id::new(("hex", id));
        let mut text: String = ui.data_mut(|d| d.get_temp(key)).unwrap_or_else(|| hex(*c));
        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(72.0).font(egui::TextStyle::Monospace)).on_hover_text("Hex (#a1b2c3) or R, G, B");
        if r.has_focus() {
            ui.data_mut(|d| d.insert_temp(key, text.clone()));
        } else {
            ui.data_mut(|d| d.remove::<String>(key));
        }
        if r.lost_focus() {
            if let Some(p) = parse(&text) {
                *c = p;
                changed = true;
            }
        }
        if eyedropper_available() && ui.button("Screen").on_hover_text("Pick a colour from anywhere on the screen (as shown, with its shading)").clicked() {
            eyedropper(ui.ctx(), id);
        }
    });
    let num = |ui: &mut egui::Ui, label: &str, v: &mut f32, max: f32, unit: &str| {
        ui.label(RichText::new(label).color(theme::TEXT_DIM));
        ui.add(egui::DragValue::new(v).range(0.0..=max).speed(1.0).max_decimals(0).suffix(unit)).changed()
    };
    ui.horizontal(|ui| {
        let mut rgb = c.map(|x| x as f32);
        let mut any = false;
        for (k, l) in ["R", "G", "B"].iter().enumerate() {
            any |= num(ui, l, &mut rgb[k], 255.0, "");
        }
        if any {
            *c = rgb.map(|x| x.round().clamp(0.0, 255.0) as u8);
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        let key = hsv_key;
        let mut hsv: [f32; 3] = ui.data_mut(|d| d.get_temp(key)).filter(|h| from_hsv(*h) == *c).unwrap_or_else(|| to_hsv(*c));
        let mut any = false;
        any |= num(ui, "H", &mut hsv[0], 360.0, "°");
        any |= num(ui, "S", &mut hsv[1], 100.0, "%");
        any |= num(ui, "V", &mut hsv[2], 100.0, "%");
        if any {
            *c = from_hsv(hsv);
            ui.data_mut(|d| d.insert_temp(key, hsv));
            changed = true;
        }
    });
    changed
}

/// Saturation across, brightness up: drag in the square.
fn sv_square(ui: &mut egui::Ui, hsv: &mut [f32; 3]) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 110.0), Sense::click_and_drag());
    let n = 16;
    let mut mesh = egui::Mesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let (s, v) = (i as f32 / n as f32, 1.0 - j as f32 / n as f32);
            let c = from_hsv([hsv[0], s * 100.0, v * 100.0]);
            mesh.colored_vertex(rect.lerp_inside(Vec2::new(s, 1.0 - v)), Color32::from_rgb(c[0], c[1], c[2]));
        }
    }
    for j in 0..n {
        for i in 0..n {
            let k = (j * (n + 1) + i) as u32;
            mesh.add_triangle(k, k + 1, k + n as u32 + 2);
            mesh.add_triangle(k, k + n as u32 + 2, k + n as u32 + 1);
        }
    }
    ui.painter().add(mesh);
    let at = rect.lerp_inside(Vec2::new(hsv[1] / 100.0, 1.0 - hsv[2] / 100.0));
    let ring = if hsv[2] > 55.0 { Color32::BLACK } else { Color32::WHITE };
    ui.painter().circle_stroke(at, 5.0, Stroke::new(1.5, ring));
    if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on()) {
        let t = (p - rect.min) / rect.size();
        hsv[1] = (t.x.clamp(0.0, 1.0) * 100.0).round();
        hsv[2] = ((1.0 - t.y.clamp(0.0, 1.0)) * 100.0).round();
        return true;
    }
    false
}

/// The hue, left to right: drag along the bar.
fn hue_bar(ui: &mut egui::Ui, hsv: &mut [f32; 3]) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 14.0), Sense::click_and_drag());
    let n = 36;
    let mut mesh = egui::Mesh::default();
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let c = from_hsv([t * 360.0, 100.0, 100.0]);
        let c = Color32::from_rgb(c[0], c[1], c[2]);
        mesh.colored_vertex(rect.lerp_inside(Vec2::new(t, 0.0)), c);
        mesh.colored_vertex(rect.lerp_inside(Vec2::new(t, 1.0)), c);
        if i > 0 {
            let k = (2 * i) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    ui.painter().add(mesh);
    let x = rect.left() + rect.width() * hsv[0] / 360.0;
    ui.painter().rect_stroke(egui::Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::new(4.0, rect.height() + 2.0)), 1.0, Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Middle);
    if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on()) {
        hsv[0] = (((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * 360.0).round();
        return true;
    }
    false
}

// ── the screen eyedropper (the browser's EyeDropper API; Chrome and Edge) ─────────────────────────────────────────

static PICKED: std::sync::Mutex<Option<(String, [u8; 3])>> = std::sync::Mutex::new(None);

fn take_picked(id: &str) -> Option<[u8; 3]> {
    let mut p = PICKED.lock().ok()?;
    if p.as_ref().is_some_and(|(k, _)| k == id) { p.take().map(|x| x.1) } else { None }
}

#[cfg(target_arch = "wasm32")]
pub fn eyedropper_available() -> bool {
    web_sys::window().is_some_and(|w| js_sys::Reflect::has(&w, &"EyeDropper".into()).unwrap_or(false))
}
#[cfg(not(target_arch = "wasm32"))]
pub fn eyedropper_available() -> bool {
    false // ponytail: desktop has no screen sampler here (macOS NSColorSampler would need AppKit calls); Sample in the app picks model colours
}

#[cfg(target_arch = "wasm32")]
fn eyedropper(ctx: &egui::Context, id: &str) {
    use wasm_bindgen::JsCast;
    let (ctx, id) = (ctx.clone(), id.to_string());
    wasm_bindgen_futures::spawn_local(async move {
        let Some(w) = web_sys::window() else { return };
        let Ok(ctor) = js_sys::Reflect::get(&w, &"EyeDropper".into()).and_then(|c| c.dyn_into::<js_sys::Function>()) else { return };
        let Ok(dropper) = js_sys::Reflect::construct(&ctor, &js_sys::Array::new()) else { return };
        let Ok(open) = js_sys::Reflect::get(&dropper, &"open".into()).and_then(|f| f.dyn_into::<js_sys::Function>()) else { return };
        let Ok(promise) = open.call0(&dropper).and_then(|p| p.dyn_into::<js_sys::Promise>()) else { return };
        // Esc while picking rejects the promise: nothing changes
        if let Ok(res) = wasm_bindgen_futures::JsFuture::from(promise).await {
            if let Some(c) = js_sys::Reflect::get(&res, &"sRGBHex".into()).ok().and_then(|v| v.as_string()).and_then(|s| parse(&s)) {
                if let Ok(mut p) = PICKED.lock() {
                    *p = Some((id, c));
                }
                ctx.request_repaint();
            }
        }
    });
}
#[cfg(not(target_arch = "wasm32"))]
fn eyedropper(_: &egui::Context, _: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_read_from_hex_rgb_and_hsv_and_back() {
        assert_eq!(parse("#FF8000"), Some([255, 128, 0]));
        assert_eq!(parse("ff8000"), Some([255, 128, 0]));
        assert_eq!(parse("#f80"), Some([255, 136, 0]));
        assert_eq!(parse("rgb(10, 20, 30)"), Some([10, 20, 30]));
        assert_eq!(parse("10 20 30"), Some([10, 20, 30]));
        assert_eq!(parse("300,0,0"), None);
        assert_eq!(parse("#12345"), None);
        assert_eq!(hex([255, 128, 0]), "#ff8000");
        assert_eq!(to_hsv([255, 0, 0]), [0.0, 100.0, 100.0]);
        assert_eq!(from_hsv([120.0, 100.0, 100.0]), [0, 255, 0]);
        // every colour comes back from its HSV within rounding
        for c in [[12, 200, 77], [255, 255, 255], [0, 0, 0], [128, 128, 128], [201, 154, 91]] {
            let back = from_hsv(to_hsv(c));
            assert!(c.iter().zip(back).all(|(&a, b)| (a as i32 - b as i32).abs() <= 1), "{c:?} → {back:?}");
        }
    }
}
