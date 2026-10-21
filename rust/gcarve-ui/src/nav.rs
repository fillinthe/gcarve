//! Mouse styles for a 3D view, shared by GCarve and GSketch: what each button drag does. A plain left-drag orbits in
//! every style where the left button is free (GCarve's 3D view; GSketch's Orbit / Pan / Zoom tools), so a mouse without
//! a middle button always works. Remembered per device by the app (`Nav::key` / `Nav::from_key`).

use eframe::egui::{self, PointerButton};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Nav {
    Fusion,
    Blender,
    VCarve,
    SketchUp,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    Orbit,
    Pan,
    Zoom,
}

impl Nav {
    /// (style, store name, label, hint line)
    pub const ALL: [(Nav, &'static str, &'static str, &'static str); 4] = [
        (Nav::Fusion, "fusion", "Fusion 360", "Middle-drag pans (Shift: orbit) · left-drag orbits · right-drag pans · wheel zooms"),
        (Nav::Blender, "blender", "Blender", "Middle-drag orbits (Shift: pan, Ctrl: zoom) · left-drag orbits · wheel zooms"),
        (Nav::VCarve, "vcarve", "VCarve / Aspire", "Left-drag orbits · right-drag zooms · middle-drag or Ctrl+left-drag pans · wheel zooms"),
        (Nav::SketchUp, "sketchup", "SketchUp", "Middle-drag orbits (Shift: pan) · left-drag orbits · wheel zooms"),
    ];

    pub fn row(self) -> (Nav, &'static str, &'static str, &'static str) {
        Nav::ALL[self as usize]
    }
    pub fn key(self) -> &'static str {
        self.row().1
    }
    pub fn from_key(k: &str) -> Option<Nav> {
        Nav::ALL.iter().find(|r| r.1 == k).map(|r| r.0)
    }

    pub fn drag(self, b: PointerButton, shift: bool, ctrl: bool) -> Act {
        use PointerButton::{Middle, Primary, Secondary};
        match (self, b) {
            (Nav::VCarve, Primary) if ctrl => Act::Pan,
            (_, Primary) => if shift { Act::Pan } else { Act::Orbit },
            (Nav::VCarve, Secondary) => Act::Zoom,
            (_, Secondary) => Act::Pan,
            (Nav::VCarve, Middle) => Act::Pan,
            (Nav::Blender, Middle) if ctrl => Act::Zoom,
            (Nav::Fusion, Middle) if ctrl => Act::Zoom,
            (Nav::Blender | Nav::SketchUp, Middle) => if shift { Act::Pan } else { Act::Orbit },
            (_, Middle) => if shift { Act::Orbit } else { Act::Pan },
            _ => Act::Orbit,
        }
    }

    /// a two-finger swipe on a trackpad
    pub fn swipe(self, shift: bool) -> Act {
        if matches!(self, Nav::Blender | Nav::SketchUp) != shift { Act::Orbit } else { Act::Pan }
    }
}

/// A mouse set up by hand (Preferences › Mouse): what a left / middle / right drag does plain, with Shift and with
/// Ctrl (⌘ on a Mac), the wheel direction, zoom and orbit speeds, and what a two-finger swipe does. It starts as a copy
/// of a style (`from_nav`); `base` stays that style until a cell is changed. Stored as one line (`encode` / `decode`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MouseMap {
    /// [left, middle, right][plain, Shift, Ctrl]
    pub drags: [[Act; 3]; 3],
    pub wheel_reverse: bool,
    /// 0.25 … 4: how far one wheel notch or drag zooms / one drag pixel turns
    pub zoom_speed: f32,
    pub orbit_speed: f32,
    /// a plain two-finger swipe (Shift gives the other one)
    pub swipe: Act,
    pub base: Option<Nav>,
}

pub const BUTTONS: [(PointerButton, &str); 3] = [(PointerButton::Primary, "Left"), (PointerButton::Middle, "Middle"), (PointerButton::Secondary, "Right")];
pub const ACTS: [(Act, &str); 3] = [(Act::Orbit, "Orbit"), (Act::Pan, "Pan"), (Act::Zoom, "Zoom")];

impl Default for MouseMap {
    fn default() -> Self {
        MouseMap::from_nav(Nav::Fusion)
    }
}

impl MouseMap {
    pub fn from_nav(nav: Nav) -> Self {
        let cell = |b, shift, ctrl| nav.drag(b, shift, ctrl);
        let row = |b| [cell(b, false, false), cell(b, true, false), cell(b, false, true)];
        MouseMap {
            drags: [row(PointerButton::Primary), row(PointerButton::Middle), row(PointerButton::Secondary)],
            wheel_reverse: false,
            zoom_speed: 1.0,
            orbit_speed: 1.0,
            swipe: nav.swipe(false),
            base: Some(nav),
        }
    }
    pub fn drag(&self, b: PointerButton, shift: bool, ctrl: bool) -> Act {
        let r = BUTTONS.iter().position(|x| x.0 == b).unwrap_or(0);
        self.drags[r][if ctrl { 2 } else if shift { 1 } else { 0 }]
    }
    pub fn swipe(&self, shift: bool) -> Act {
        match (self.swipe, shift) {
            (Act::Orbit, true) => Act::Pan,
            (_, true) => Act::Orbit,
            (a, false) => a,
        }
    }
    /// the style's name, or "Custom" once anything was changed by hand
    pub fn label(&self) -> &'static str {
        self.base.filter(|n| MouseMap::from_nav(*n).drags == self.drags && MouseMap::from_nav(*n).swipe == self.swipe).map_or("Custom", |n| n.row().2)
    }
    pub fn hint(&self) -> String {
        if let Some(n) = self.base.filter(|_| self.label() != "Custom") {
            return n.row().3.to_string();
        }
        let name = |a: Act| ACTS.iter().find(|x| x.0 == a).map_or("", |x| x.1).to_lowercase();
        BUTTONS.iter().enumerate().map(|(i, (_, b))| format!("{}-drag {}s", b.to_lowercase(), name(self.drags[i][0]))).collect::<Vec<_>>().join(" · ") + " · wheel zooms"
    }
    pub fn encode(&self) -> String {
        let ch = |a: Act| match a {
            Act::Orbit => 'o',
            Act::Pan => 'p',
            Act::Zoom => 'z',
        };
        let drags: Vec<String> = self.drags.iter().map(|r| r.iter().map(|a| ch(*a)).collect()).collect();
        format!("v1 {} {} {:.2} {:.2} {} {}", drags.join("."), u8::from(self.wheel_reverse), self.zoom_speed, self.orbit_speed, ch(self.swipe), self.base.map_or("-", |n| n.key()))
    }
    pub fn decode(s: &str) -> Option<MouseMap> {
        let p: Vec<&str> = s.split_whitespace().collect();
        if p.len() != 7 || p[0] != "v1" {
            return None;
        }
        let act = |c: char| match c {
            'o' => Some(Act::Orbit),
            'p' => Some(Act::Pan),
            'z' => Some(Act::Zoom),
            _ => None,
        };
        let mut drags = [[Act::Orbit; 3]; 3];
        let rows: Vec<&str> = p[1].split('.').collect();
        if rows.len() != 3 {
            return None;
        }
        for (r, row) in rows.iter().enumerate() {
            let cells: Vec<char> = row.chars().collect();
            if cells.len() != 3 {
                return None;
            }
            for (c, ch) in cells.iter().enumerate() {
                drags[r][c] = act(*ch)?;
            }
        }
        let speed = |t: &str| t.parse::<f32>().ok().filter(|v| v.is_finite()).map(|v| v.clamp(0.25, 4.0));
        Some(MouseMap { drags, wheel_reverse: p[2] == "1", zoom_speed: speed(p[3])?, orbit_speed: speed(p[4])?, swipe: act(p[5].chars().next()?)?, base: Nav::from_key(p[6]) })
    }
}

/// A mouse wheel notch (zoom) vs a trackpad swipe (pan / orbit). Native: wheels report lines. Browsers report pixels
/// for both; a wheel notch is a large, vertical-only, whole-number step.
/// ponytail: heuristic for the web; a smooth-scrolling mouse may read as a trackpad (then it pans; pinch still zooms)
pub fn wheel_is_mouse(unit: egui::MouseWheelUnit, d: egui::Vec2) -> bool {
    match unit {
        egui::MouseWheelUnit::Point => d.x == 0.0 && d.y.fract() == 0.0,
        _ => true,
    }
}

/// Mouse wheel or trackpad, decided per scroll gesture (events under 0.3 s apart) instead of per event: a wheel with
/// acceleration or smooth scrolling sends steps of every size mid-spin, so judging each one made a spin zoom and
/// orbit by turns (owner, 2026-10-19). A sideways or fractional step still turns the gesture into a trackpad one.
#[derive(Default, Clone, Copy)]
pub struct WheelKind {
    last: f64,
    mouse: bool,
}

impl WheelKind {
    pub fn is_mouse(&mut self, now: f64, unit: egui::MouseWheelUnit, d: egui::Vec2) -> bool {
        let this = wheel_is_mouse(unit, d);
        self.mouse = if now - self.last > 0.3 { this } else { self.mouse && this };
        self.last = now;
        self.mouse
    }
}

/// A left-drag with a key held, for a trackpad or a one-button mouse: ⌥ orbits, ⌃ pans (like a right-drag), ⌘
/// zooms. None without one of them.
pub fn key_drag(alt: bool, ctrl: bool, cmd: bool) -> Option<Act> {
    if alt {
        Some(Act::Orbit)
    } else if ctrl {
        Some(Act::Pan)
    } else if cmd {
        Some(Act::Zoom)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PointerButton::{Middle, Primary, Secondary};

    #[test]
    fn wheel_vs_trackpad_and_key_drags() {
        assert!(wheel_is_mouse(egui::MouseWheelUnit::Line, egui::vec2(0.0, 1.0)));
        assert!(wheel_is_mouse(egui::MouseWheelUnit::Point, egui::vec2(0.0, -100.0)));
        assert!(!wheel_is_mouse(egui::MouseWheelUnit::Point, egui::vec2(0.0, -3.5)));
        assert!(!wheel_is_mouse(egui::MouseWheelUnit::Point, egui::vec2(12.0, -60.0)));
        assert!(wheel_is_mouse(egui::MouseWheelUnit::Point, egui::vec2(0.0, -4.0)), "a small wheel step (macOS acceleration)");
        // one gesture keeps its kind: a trackpad swipe ending in whole-number steps doesn't start zooming
        let mut k = WheelKind::default();
        assert!(!k.is_mouse(1.0, egui::MouseWheelUnit::Point, egui::vec2(0.5, -3.5)));
        assert!(!k.is_mouse(1.1, egui::MouseWheelUnit::Point, egui::vec2(0.0, -2.0)));
        assert!(k.is_mouse(2.0, egui::MouseWheelUnit::Point, egui::vec2(0.0, -8.0)), "a new gesture after a pause");
        assert_eq!(key_drag(true, false, false), Some(Act::Orbit));
        assert_eq!(key_drag(false, true, false), Some(Act::Pan));
        assert_eq!(key_drag(false, false, true), Some(Act::Zoom));
        assert_eq!(key_drag(false, false, false), None);
    }

    #[test]
    fn a_hand_made_mouse_map_round_trips_and_names_itself() {
        for (nav, ..) in Nav::ALL {
            let m = MouseMap::from_nav(nav);
            for b in [Primary, Middle, Secondary] {
                for (sh, ct) in [(false, false), (true, false), (false, true)] {
                    assert_eq!(m.drag(b, sh, ct), nav.drag(b, sh, ct), "{nav:?} copies its style");
                }
            }
            assert_eq!(m.swipe(false), nav.swipe(false));
            assert_eq!(m.swipe(true), nav.swipe(true));
            assert_eq!(m.label(), nav.row().2);
            assert_eq!(MouseMap::decode(&m.encode()), Some(m));
        }
        let mut m = MouseMap::from_nav(Nav::Fusion);
        m.drags[2][0] = Act::Zoom; // right-drag zooms
        m.wheel_reverse = true;
        m.zoom_speed = 2.5;
        assert_eq!(m.label(), "Custom");
        assert_eq!(m.drag(Secondary, false, false), Act::Zoom);
        assert_eq!(MouseMap::decode(&m.encode()), Some(m));
        assert!(m.hint().contains("right-drag zooms"));
        assert_eq!(MouseMap::decode("v1 ooo.ppp.zzz 0 99 0.1 p blender").map(|m| (m.zoom_speed, m.orbit_speed)), Some((4.0, 0.25)), "speeds are clamped");
        assert!(MouseMap::decode("v1 oox.ppp.zzz 0 1 1 p -").is_none() && MouseMap::decode("rubbish").is_none());
    }

    #[test]
    fn each_style_maps_its_buttons() {
        assert_eq!(Nav::Fusion.drag(Middle, false, false), Act::Pan);
        assert_eq!(Nav::Fusion.drag(Middle, true, false), Act::Orbit);
        assert_eq!(Nav::Blender.drag(Middle, false, false), Act::Orbit);
        assert_eq!(Nav::Blender.drag(Middle, true, false), Act::Pan);
        assert_eq!(Nav::Blender.drag(Middle, false, true), Act::Zoom);
        assert_eq!(Nav::VCarve.drag(Secondary, false, false), Act::Zoom);
        assert_eq!(Nav::VCarve.drag(Primary, false, true), Act::Pan);
        assert_eq!(Nav::SketchUp.drag(Middle, false, false), Act::Orbit);
        assert_eq!(Nav::SketchUp.drag(Middle, true, false), Act::Pan);
        assert_eq!(Nav::SketchUp.drag(Middle, false, true), Act::Orbit, "SketchUp has no Ctrl zoom");
        for (nav, key, ..) in Nav::ALL {
            assert_eq!(nav.drag(Primary, false, false), Act::Orbit, "{key}: a plain left-drag orbits");
            assert_eq!(Nav::from_key(key), Some(nav));
            assert_eq!(nav.row().0, nav);
        }
    }
}
