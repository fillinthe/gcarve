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
