//! Document model (C++ core/document.*): shapes, layers, sheets, groups, variables, constraints, dimensions, reliefs,
//! texts, undo. Also the CAM data types declared in C++ core/cam.h (Tool, Material, Op, ...) and the `Relief` record
//! from core/relief.h, so cam/relief code builds on these instead of redefining them.

use std::cell::Cell;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::geom::{self, dist, pt, transformed, Affine, Contour, Pt, Rect};

pub type Bind = BTreeMap<String, String>;

// ---- CAM types (C++ core/cam.h) -----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ToolType {
    #[default]
    EndMill,
    VBit,
    BallNose,
    /// Point-cutting round-over: quarter-circle flanks of radius diameter / 2 meeting at the tip (Rust only; Qt
    /// loads it as an end mill)
    RoundOver,
}

/// Feeds & speeds for one material. Matched to Material::kind, case-insensitive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedPreset {
    pub material: String,
    pub feed_xy: f64,
    pub feed_z: f64,
    pub stepdown: f64,
    pub rpm: i32,
}

impl Default for FeedPreset {
    fn default() -> Self {
        FeedPreset { material: String::new(), feed_xy: 100.0, feed_z: 50.0, stepdown: 0.25, rpm: 18000 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tool {
    /// also printed in the G-code header, e.g. "1/4 Down End Mill (0.25 inch)"
    pub name: String,
    pub kind: ToolType,
    pub diameter: f64,
    /// V-bit included angle, degrees
    pub angle: f64,
    pub stepdown: f64,
    /// inches/min (defaults when no preset matches the material)
    pub feed_xy: f64,
    pub feed_z: f64,
    pub rpm: i32,
    pub flutes: i32,
    pub presets: Vec<FeedPreset>,
    /// maker chip loads (in/tooth) by material group (data/bits/chipload.json groups), from the bit catalog
    pub chip_loads: BTreeMap<String, [f64; 2]>,
    /// catalog id ("whiteside-ru2100") when the tool came from the bit database
    pub catalog: String,
    /// tool-changer pocket (ATC rack slot) for machines with one; 0 = none (manual change, files numbered in order)
    pub slot: i32,
    /// round-over bits: the round's radius; 0 = half the diameter (a true point). Smaller leaves a flat tip of
    /// diameter − 2 × radius. Ball nose: the corner radius; smaller than half the diameter = a bowl / tray bit (a flat
    /// bottom with rounded corners)
    pub radius: f64,
    /// length of the flutes (inches); 0 = not known. A cut deeper than this rubs the shank (export safety check).
    pub cut_length: f64,
    /// how far the bit sticks out of the collet / holder (inches); 0 = not known. A cut deeper than this drives the
    /// holder into the board (JSON "stickOut" when set, Rust only)
    pub stick_out: f64,
    /// width of the collet / holder (inches); 0 = not known. With stick-out, its radius counts against keep-out areas
    /// when the holder comes down to clamp height (JSON "holderDia" when set, Rust only)
    pub holder_dia: f64,
    /// Tool Library folder ("V-bits", "Amana"); empty = none (JSON "group" when set, Rust only)
    pub group: String,
    /// custom form cutter (VCarve "Custom Form Cutters"): the cutting edge in side view as [radius, height above the
    /// tip] points by radius, from a drawn profile; empty = the shape of `kind` (JSON "form" when set, Rust only)
    pub form: Vec<[f64; 2]>,
}

impl Tool {
    /// Ball nose / bowl bit: how far above its lowest point the bit's edge is, `dist` from its axis (flat across the
    /// middle of a bowl bit, then the corner round).
    pub fn ball_rise(&self, dist: f64) -> f64 {
        let r = self.diameter / 2.0;
        let rr = if self.radius > 0.0 { self.radius.min(r) } else { r };
        let x = (dist.abs() - (r - rr)).clamp(0.0, rr);
        rr - (rr * rr - x * x).max(0.0).sqrt()
    }

    /// Height of the drawn form's cutting edge above the tip at `r` from the axis (straight between points);
    /// None = no form, or outside the bit
    pub fn form_rise(&self, r: f64) -> Option<f64> {
        let f = &self.form;
        if f.len() < 2 || r > f[f.len() - 1][0] + 1e-9 {
            return None;
        }
        let i = f.partition_point(|p| p[0] < r).clamp(1, f.len() - 1);
        let (a, b) = (f[i - 1], f[i]);
        let t = if b[0] > a[0] { ((r - a[0]) / (b[0] - a[0])).clamp(0.0, 1.0) } else { 0.0 };
        Some(a[1] + (b[1] - a[1]) * t)
    }

    /// A form from a drawn side-view profile (inches): the leftmost point is the axis, the lowest the tip; where the
    /// line doubles back the points are taken in order of radius. Diameter follows the widest point.
    pub fn form_from_profile(pts: &[crate::geom::Pt]) -> Vec<[f64; 2]> {
        if pts.len() < 2 {
            return Vec::new();
        }
        let (x0, y0) = (pts.iter().map(|p| p.x).fold(f64::MAX, f64::min), pts.iter().map(|p| p.y).fold(f64::MAX, f64::min));
        let mut f: Vec<[f64; 2]> = pts.iter().map(|p| [p.x - x0, p.y - y0]).collect();
        f.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        // a vertical step keeps both ends (lower first): just past that radius the edge is at the upper one
        f.dedup_by(|b, a| (b[0] - a[0]).abs() < 1e-9 && (b[1] - a[1]).abs() < 1e-9);
        f
    }
}

impl Default for Tool {
    fn default() -> Self {
        Tool {
            name: String::new(),
            kind: ToolType::EndMill,
            diameter: 0.25,
            angle: 90.0,
            stepdown: 0.25,
            feed_xy: 100.0,
            feed_z: 50.0,
            rpm: 18000,
            flutes: 2,
            presets: Vec::new(),
            chip_loads: BTreeMap::new(),
            catalog: String::new(),
            slot: 0,
            radius: 0.0,
            cut_length: 0.0,
            stick_out: 0.0,
            holder_dia: 0.0,
            group: String::new(),
            form: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Material {
    /// free text: Baltic birch, MDF, oak... (printed in the G-code header)
    pub kind: String,
    /// inches; Z zero = material surface, XY origin = bottom-left
    pub w: f64,
    pub h: f64,
    pub t: f64,
    pub safe_z: f64,
    pub retract_z: f64,
    /// XY zero on the board: 0 bottom-left, 1 bottom-right, 2 top-left, 3 top-right, 4 centre
    pub origin: i32,
    /// Z zero: false = material surface (touch top), true = machine bed (touch bottom)
    pub z_bottom: bool,
    /// post-processor id
    pub post: String,
    /// bits with no preset for this material get chip-load suggested feeds
    pub auto_feeds: bool,
    /// the machine this job is for (from the machine database): its limits clamp suggested feeds and are checked at export
    pub machine: Option<MachineLimits>,
    /// w|h|t|safeZ|retractZ -> expression
    pub bind: Bind,
    /// the sheet's name ("Doors", "12a"); empty = "Sheet N" (JSON "name" when set, Rust only)
    pub name: String,
    /// VCarve "Home Position": where the bit parks at the end of each file, X / Y from the XY zero, at the travel
    /// height (JSON "home" [x, y] when set, Rust only); None = it stays where the last cut ended
    pub home: Option<[f64; 2]>,
    /// VCarve "Merged toolpath": in each bit's file the toolpaths of every operation are cut in one shortest tour,
    /// cut-outs (down to the board's bottom) last (JSON "merge" when set, Rust only)
    pub merge: bool,
    /// two-sided jobs: this sheet is the back of sheet `.0`, the board flipped left to right (`.1` true) or top to
    /// bottom; the front's outlines show mirrored under it (JSON "backOf" {sheet, lr}, Rust only)
    pub back_of: Option<(i32, bool)>,
    /// VCarve "Rotary job": a round blank of this diameter turned by an A axis along X. The board is its surface
    /// unrolled (w = length, h = circumference π·d); at export Y becomes A in degrees. JSON "rotary", Rust only
    pub rotary: Option<f64>,
    /// rotary axis along the machine's Y instead of X: the drawing is the same (length left to right), only the
    /// G-code letters change (X words → Y, Y → A). JSON "rotaryAlongY" when set, Rust only
    pub rotary_y: bool,
    /// what the board costs and the shop's rate per hour of machine time (JSON "price" / "rate" when set, Rust
    /// only): the setup sheet and the toolpath list show the job's cost when either is set
    pub price: f64,
    pub rate: f64,
}

impl Material {
    /// board price + machine time at the shop rate; None when neither is set
    pub fn cost(&self, minutes: f64) -> Option<f64> {
        (self.price > 0.0 || self.rate > 0.0).then(|| self.price + self.rate * minutes / 60.0)
    }
}

/// What GCarve needs from a machine record (copied into the project so it travels with it). 0 = not published.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineLimits {
    pub id: String,
    pub name: String,
    pub spindle: String,
    pub rpm_min: f64,
    pub rpm_max: f64,
    /// in/min
    pub feed_xy: f64,
    pub feed_z: f64,
    /// cutting travel, inches
    pub travel_x: f64,
    pub travel_y: f64,
    pub travel_z: f64,
    /// tool changer slots (0 = manual changes)
    pub tool_slots: i32,
    /// rapid speed in X / Y, in/min (time estimates; 0 = not known)
    pub rapid_xy: f64,
    /// industrial | prosumer | hobby | desktop ("" = not known): scales suggested chip loads (pipeline::class_factor)
    pub class: String,
}

impl Default for Material {
    fn default() -> Self {
        Material {
            kind: "Plywood".into(),
            w: 48.0,
            h: 24.0,
            t: 0.75,
            safe_z: 0.8,
            retract_z: 0.2,
            origin: 0,
            z_bottom: false,
            post: "wincnc-single".into(),
            auto_feeds: true,
            machine: None,
            bind: Bind::new(),
            name: String::new(),
            home: None,
            merge: false,
            back_of: None,
            rotary: None,
            rotary_y: false,
            price: 0.0,
            rate: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Side {
    #[default]
    Outside,
    Inside,
    On,
}

/// Ramp: descend over `ramp_length`, then flat
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Entry {
    Plunge,
    #[default]
    Spiral,
    Ramp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OpType {
    #[default]
    Profile,
    Pocket,
    Drill,
    VCarve,
    Bore,
    Inlay,
    Flute,
    Relief,
    Chamfer,
    Prism,
    /// moves read from a G-code file (`Op::gcode`, gimport.rs): replayed as is, only the bit and name change
    Imported,
    /// VCarve "Texture": rows of short random scoop strokes filling the shapes (`ramp_length` = stroke length,
    /// `stepover` = row spacing × bit, `vary` = randomness, `raster_angle` = row direction)
    Texture,
    /// VCarve "Photo V-Carve": V-bit rows whose depth follows the picture's darkness (`photo*`, `depth` = deepest,
    /// `raster_angle` = row direction, `photo_invert` = light parts deep)
    Photo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Op {
    pub kind: OpType,
    pub name: String,
    /// explicit shapes (used when no layer/group scope)
    pub shape_ids: Vec<i32>,
    /// >= 0: the toolpath applies to every shape on this layer (new shapes inherit it)
    pub layer_scope: i32,
    /// > 0: applies to every shape in this group
    pub group_scope: i32,
    pub tool: i32,
    pub side: Side,
    pub depth: f64,
    /// 0 = tool's stepdown
    pub stepdown: f64,
    pub entry: Entry,
    pub climb: bool,
    pub tabs: bool,
    pub tab_count: i32,
    /// tab width is material left; the toolpath gap adds the tool diameter
    pub tab_width: f64,
    pub tab_height: f64,
    /// 3D tabs: ramp up/down over the tab instead of square steps
    pub tab_ramped: bool,
    /// Entry::Ramp distance (profile, pocket)
    pub ramp_length: f64,
    /// pocket stepover (fraction of tool diameter)
    pub stepover: f64,
    /// pocket: material left on the walls
    pub allowance: f64,
    /// drill: peck depth, 0 = straight down
    pub peck: f64,
    /// v-carve: 0 = carve to full depth, else flat-bottom at this depth
    pub flat_depth: f64,
    /// v-carve: carving starts this far below the surface
    pub start_depth: f64,
    /// inlay: male start depth (gap left for glue)
    pub glue_gap: f64,
    /// inlay: male blank extends this far around the design
    pub inlay_margin: f64,
    /// flute: ramp lengths at the start / end of the vector (0 = none).
    /// profile: `ramp_in` = lead-in / lead-out arc radius in the waste (0 = plunge on the line)
    pub ramp_in: f64,
    pub ramp_out: f64,
    /// chamfer: bevel width (> 0 sizes the chamfer by width, else `depth` is the V depth)
    pub chamfer_width: f64,
    /// v-carve: end mill that clears the flat floor (-1 = the V-bit does it); relief: roughing tool
    pub clear_tool: i32,
    /// drawn in the 2D/3D previews (still exported)
    pub visible: bool,
    /// texture: 0..1 random change of stroke length and depth
    pub vary: f64,
    /// user's operation group in the toolpath list ("Sign face", "Cabinet sides"); empty = none
    pub op_group: String,
    /// Relief: Document::reliefs id
    pub relief: i32,
    /// Relief finishing: 0 along X, 90 along Y
    pub raster_angle: i32,
    /// Relief: layered roughing with clear_tool first
    pub rough: bool,
    pub rough_stepdown: f64,
    pub rough_stepover: f64,
    /// Imported: the section's normalized program in document coordinates (gimport::section_toolpaths)
    pub gcode: String,
    /// Photo: grayscale picture (photo.rs), width × height pixels as base64, fitted into the shapes' bounds
    pub photo: String,
    pub photo_w: u32,
    pub photo_h: u32,
    pub photo_invert: bool,
    /// Photo tone (VCarve "Edit Picture"): brightness −1..1 (lighter = shallower), contrast around mid grey (1 =
    /// as is), gamma on the darkness (> 1 = lighter mid tones). JSON in "photo" when not the defaults (Rust only)
    pub photo_brightness: f64,
    pub photo_contrast: f64,
    pub photo_gamma: f64,
    /// Photo as VCarve "Sketch Carving": cross-hatching at one depth instead of rows of varying depth (JSON in "photo")
    pub sketch: bool,
    /// pocket: clear with back-and-forth rows along `raster_angle` (then one lap round the walls) instead of rings
    pub raster: bool,
    /// pocket: adaptive clearing (constant load: `stepover` is the width of the cut, as a fraction of the bit) instead of
    /// rings (JSON "adaptive", Rust only; Qt cuts rings)
    pub adaptive: bool,
    /// adaptive pocket: feed up for the thin cut (chip thinning, JSON "feedBoost", Rust only; off by default)
    pub feed_boost: bool,
    /// bore: > 0 = thread mill: one helix rising from the bottom, this far per turn (the thread pitch); 0 = plain bore
    pub thread_pitch: f64,
    /// drill with a V-bit: > 0 = countersink this wide at the surface; the depth follows from the bit's angle
    /// (JSON "countersink" when set, Rust only)
    pub countersink: f64,
    /// drill with a V-bit: each hole plunged until the cone is as wide as that hole (halftone dots, countersinks of
    /// several sizes), never past `depth` (JSON "fitHoles" when set, Rust only)
    pub fit_holes: bool,
    /// profile: onion skin: every pass stops this far above the full depth, then one last lap per part (after all the
    /// parts of this toolpath) cuts the skin, so small parts stay held without tabs (JSON "skin" when set, Rust only)
    pub skin: f64,
    /// profile: each cut starts at the point of the ring nearest the vector's first node (Node tool ▸ right-click a
    /// node ▸ Make Start Point) instead of the lowest point
    pub start_first: bool,
    /// profile (inside / outside): G41 / G42 round the cuts so the size can be trimmed at the machine with a wear
    /// offset in D (the moves stay the bit-centre path; JSON "cutterComp", Rust only)
    pub cutter_comp: bool,
    /// profile: keyhole slot for a keyhole cutter (VCarve's Keyhole gadget): one plunge to full depth at each
    /// vector's start (the screw entry), along it, back the same way, lift where it went in (JSON "keyhole", Rust only)
    pub keyhole: bool,
    /// profile: drag knife with this blade offset (tip behind the pivot), inches; 0 = off. One pass on the line,
    /// swivels round sharp corners, spindle kept off (the bit's RPM 0). JSON "dragKnife", Rust only
    pub drag_knife: f64,
    /// pocket on a rotary job: VCarve "Rounding" — the blank is square with this side (inches); the bit spirals the
    /// pocket shape's length from the square's corners down to the job's diameter. 0 = a normal pocket. JSON
    /// "rounding", Rust only
    pub rounding: f64,
    /// flute: VCarve "Moulding": this open vector is the cross-section (its width across the rail, its height = depth
    /// below the top) swept along every rail; 0 = a plain flute. JSON "section", Rust only
    pub section: i32,
    /// VCarve "Project toolpath onto 3D model": every cutting move rides this 3D model's surface (its id); 0 = flat.
    /// JSON "projectOn", Rust only
    pub project_on: i32,
    /// VCarve "Use vector selection order": shapes are cut in the order they were drawn, not reordered for the
    /// shortest travel (JSON "keepOrder", Rust only)
    pub keep_order: bool,
    /// profile: tabs placed by hand (document points on the shape's outline): a cut that has some of them near it
    /// uses them instead of `tab_count` evenly spaced tabs (JSON "tabPoints", Rust only)
    pub tab_points: Vec<[f64; 2]>,
    /// profile: evenly spaced tabs slide along the outline off corners (VCarve "avoid corners"; JSON
    /// "tabAvoidCorners" when set, Rust only). Off by default so older projects cut as before; new toolpaths turn it on
    pub tab_corners: bool,
    /// profile: > 0 = evenly spaced tabs about this far apart on every part (so a big part gets more than a small one)
    /// instead of `tab_count` per part (JSON "tabSpacing" when set, Rust only)
    pub tab_spacing: f64,
    /// the sheet it cuts on (index), -1 = every sheet (JSON "sheet" when set, Rust only)
    pub sheet_scope: i32,
    /// cuts every vector (all layers but keep-out ones) instead of picked shapes / a layer / a group (JSON
    /// "allVectors", Rust only)
    pub all_vectors: bool,
    /// array copies (VCarve "Array Copy Toolpath"): the toolpath is cut `array_rows` × `array_cols` times, each copy
    /// moved by `array_dx` / `array_dy` (inches, copy to copy) from the last; 1 × 1 = no copies (JSON "array" when set)
    pub array_rows: i32,
    pub array_cols: i32,
    pub array_dx: f64,
    pub array_dy: f64,
    /// depth|stepdown|tabWidth|tabHeight|allowance|peck|flatDepth -> expression
    pub bind: Bind,
}

impl Default for Op {
    fn default() -> Self {
        Op {
            kind: OpType::Profile,
            name: "Profile".into(),
            shape_ids: Vec::new(),
            layer_scope: -1,
            group_scope: 0,
            tool: 0,
            side: Side::Outside,
            depth: 0.75,
            stepdown: 0.0,
            entry: Entry::Spiral,
            climb: true,
            tabs: false,
            tab_count: 4,
            tab_width: 0.5,
            tab_height: 0.125,
            tab_ramped: false,
            ramp_length: 1.0,
            stepover: 0.4,
            allowance: 0.0,
            peck: 0.0,
            flat_depth: 0.0,
            start_depth: 0.0,
            glue_gap: 0.1,
            inlay_margin: 0.5,
            ramp_in: 0.0,
            ramp_out: 0.0,
            chamfer_width: 0.0,
            clear_tool: -1,
            visible: true,
            op_group: String::new(),
            vary: 0.0,
            relief: -1,
            raster_angle: 0,
            rough: false,
            rough_stepdown: 0.2,
            rough_stepover: 0.5,
            gcode: String::new(),
            photo: String::new(),
            photo_w: 0,
            photo_brightness: 0.0,
            photo_contrast: 1.0,
            sketch: false,
            photo_gamma: 1.0,
            photo_h: 0,
            photo_invert: false,
            raster: false,
            adaptive: false,
            feed_boost: false,
            thread_pitch: 0.0,
            countersink: 0.0,
            skin: 0.0,
            fit_holes: false,
            start_first: false,
            cutter_comp: false,
            keyhole: false,
            drag_knife: 0.0,
            rounding: 0.0,
            section: 0,
            project_on: 0,
            keep_order: false,
            tab_corners: false,
            tab_spacing: 0.0,
            sheet_scope: -1,
            all_vectors: false,
            array_rows: 1,
            array_cols: 1,
            array_dx: 0.0,
            array_dy: 0.0,
            tab_points: Vec::new(),
            bind: Bind::new(),
        }
    }
}

/// 3D relief record (C++ core/relief.h `Relief`): placement + the rasterized height field. The relief module adds
/// rasterizing / sampling as `impl Relief` blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct Relief {
    pub id: i32,
    pub name: String,
    /// STL path (re-rasterized on load)
    pub source: String,
    /// placement on the board, inches (model is stretched to w x h)
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// total carved depth: model top at the surface, base at -depth
    pub depth: f64,
    pub mirror: bool,
    pub nx: i32,
    pub ny: i32,
    /// 0 = base .. 1 = top, row-major, y up (nx*ny)
    pub height: Vec<f32>,
    /// made from a picture instead of a 3D model (Rust only): 8-bit grey as base64, row 0 at the top. The picture is
    /// saved in the project, so it comes back on every device (a model is re-read from its file).
    pub pic: String,
    pub pic_w: u32,
    pub pic_h: u32,
    /// dark parts stand high (a lithophane: thick where dark) instead of light parts
    pub pic_invert: bool,
    /// blur radius in picture pixels (0 = none), against grain and noise stepping the surface
    pub pic_smooth: u32,
    /// stretch the picture's own light / dark range over the whole depth (2nd to 98th percentile)
    pub pic_levels: bool,
}

impl Default for Relief {
    fn default() -> Self {
        Relief {
            id: 0,
            name: String::new(),
            source: String::new(),
            x: 0.0,
            y: 0.0,
            w: 4.0,
            h: 4.0,
            depth: 0.25,
            mirror: false,
            nx: 0,
            ny: 0,
            height: Vec::new(),
            pic: String::new(),
            pic_w: 0,
            pic_h: 0,
            pic_invert: false,
            pic_smooth: 1,
            pic_levels: true,
        }
    }
}

// ---- document records ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Shape {
    pub id: i32,
    pub c: Contour,
    pub layer: i32,
    /// which board/sheet this shape sits on (Document::sheets)
    pub sheet: i32,
    /// 0 = not grouped; shapes sharing a group id select and move together (one level, no nesting)
    pub group: i32,
    /// "x","y","w","h", "n<i>.x", "s<i>.len" -> expression
    pub bind: Bind,
    /// glyph outline of a TextItem (Document::texts); 0 = plain vector
    pub text: i32,
    /// where a size change grows from (owner 2026-10-22: "give them an anchor"): 0..8 = column (left / centre /
    /// right) + 3 × row (bottom / middle / top); 0 = bottom-left, the old behaviour. JSON "anchor" when set
    pub anchor: i32,
}

/// the anchor point's place in a box: (0 left … 1 right, 0 bottom … 1 top)
pub fn anchor_frac(anchor: i32) -> (f64, f64) {
    let a = anchor.clamp(0, 8);
    (f64::from(a % 3) / 2.0, f64::from(a / 3) / 2.0)
}

pub const ANCHOR_NAMES: [&str; 9] = ["bottom-left", "bottom centre", "bottom-right", "middle left", "centre", "middle right", "top-left", "top centre", "top-right"];

/// Editable text: the glyph Shapes carry its id, so editing regenerates them in place.
#[derive(Clone, Debug, PartialEq)]
pub struct TextItem {
    pub id: i32,
    pub text: String,
    pub family: String,
    /// cap height, inches
    pub height: f64,
    pub bold: bool,
    pub italic: bool,
    /// along this curve when > 0
    pub path_shape: i32,
    /// inches
    pub start_offset: f64,
    pub spacing: f64,
    pub above: bool,
    /// block text, several lines: 0 left (what it always did), 1 centre, 2 right (Rust only; Qt keeps left)
    pub align: i32,
    /// merge overlapping letters into one outline (script fonts), keeping the letters' own holes
    pub weld: bool,
    /// block text turned by this many degrees, counter-clockwise (Rust only; Qt sets it level)
    pub angle: f64,
    /// block text: line pitch as a multiple of the font's own (1 = normal; JSON "lineSpacing" when not 1, Rust only)
    pub line_spacing: f64,
    /// block text bent round a circle of this radius (VCarve "text arching"): > 0 = arched up (letters on top of
    /// the circle), < 0 = bowed down; 0 = straight. First line only. JSON "arch" when set, Rust only
    pub arch: f64,
    /// extra gap after each character of `text` (by char index, newlines counted), inches: kerning by hand.
    /// JSON "kerns" when any is set, Rust only
    pub kerns: Vec<f64>,
}

impl TextItem {
    /// extra gap after character `i` of the text (0 when none set)
    pub fn kern(&self, i: usize) -> f64 {
        self.kerns.get(i).copied().unwrap_or(0.0)
    }
}

impl Default for TextItem {
    fn default() -> Self {
        TextItem {
            id: 0,
            text: String::new(),
            family: String::new(),
            height: 1.0,
            bold: false,
            italic: false,
            path_shape: 0,
            start_offset: 0.0,
            spacing: 0.0,
            above: true,
            align: 0,
            weld: false,
            angle: 0.0,
            line_spacing: 1.0,
            arch: 0.0,
            kerns: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Variable {
    pub name: String,
    pub expr: String,
}

/// Positional relation between two shapes: `target` is moved so that the relation to `anchor` holds.
/// kind: right-of | left-of | above | below | align-left | align-right | align-top | align-bottom | center-x | center-y
/// expr: gap / offset (length expression, may use variables), "" = 0
#[derive(Clone, Debug, PartialEq)]
pub struct Constraint {
    pub anchor: i32,
    pub target: i32,
    pub kind: String,
    pub expr: String,
}

impl Default for Constraint {
    fn default() -> Self {
        Constraint { anchor: 0, target: 0, kind: "right-of".into(), expr: String::new() }
    }
}

/// Linear dimension between two points, drawn `offset` (inches, signed, along the left normal) from the line a-b.
/// Endpoints can anchor to shape nodes; unanchored points are static.
#[derive(Clone, Debug, PartialEq)]
pub struct Dimension {
    pub a: Pt,
    pub b: Pt,
    pub offset: f64,
    pub shape_a: i32,
    pub node_a: i32,
    pub shape_b: i32,
    pub node_b: i32,
    /// override text; "{v}" inserts the measured value. Empty = value only
    pub label: String,
    /// length expression: when both ends are anchored, the geometry is moved to match. Empty = measure only
    pub drive: String,
    /// 0 aligned (the straight distance), 1 horizontal (X only), 2 vertical (Y only), 3 angle at `vertex` between
    /// the arms to a and b (`offset` = arc radius). JSON "axis" when set, Rust only
    pub axis: i32,
    /// angular dimensions: the corner (JSON "vertex")
    pub vertex: Pt,
}

impl Dimension {
    /// the two points the dimension line runs between: a, and b moved onto a's level (horizontal) or a's X
    /// (vertical); the extension line from b still starts at b
    pub fn ends(&self) -> (Pt, Pt) {
        match self.axis {
            1 => (self.a, pt(self.b.x, self.a.y)),
            2 => (self.a, pt(self.a.x, self.b.y)),
            _ => (self.a, self.b),
        }
    }
    pub fn length(&self) -> f64 {
        let (a, b) = self.ends();
        dist(a, b)
    }
    /// angular: the arms' directions (radians) and the sweep from a's arm to b's, the smaller way round (signed)
    pub fn arc(&self) -> (f64, f64) {
        let a0 = (self.a.y - self.vertex.y).atan2(self.a.x - self.vertex.x);
        let a1 = (self.b.y - self.vertex.y).atan2(self.b.x - self.vertex.x);
        let mut sweep = (a1 - a0).rem_euclid(std::f64::consts::TAU);
        if sweep > std::f64::consts::PI {
            sweep -= std::f64::consts::TAU;
        }
        (a0, sweep)
    }
    /// angular: degrees between the arms (0..180)
    pub fn degrees(&self) -> f64 {
        self.arc().1.abs().to_degrees()
    }
}

impl Default for Dimension {
    fn default() -> Self {
        Dimension {
            a: Pt::default(),
            b: Pt::default(),
            offset: 0.5,
            shape_a: -1,
            node_a: -1,
            shape_b: -1,
            node_b: -1,
            label: String::new(),
            drive: String::new(),
            axis: 0,
            vertex: Pt::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: String,
    /// outline colour on the canvas
    pub color: String,
    pub visible: bool,
    pub locked: bool,
    /// clamps, screws, hold-downs: closed shapes on this layer are no-go areas, checked at export
    /// (`pipeline::keep_out_issues`)
    pub keep_out: bool,
    /// how high (inches above the board) the clamps on a keep-out layer reach; 0 = the default 1 in (JSON "clampHeight")
    pub clamp_height: f64,
}

impl Default for Layer {
    fn default() -> Self {
        Layer { name: "Layer 1".into(), color: "#d7dae0".into(), visible: true, locked: false, keep_out: false, clamp_height: 0.0 }
    }
}

/// Tool by name in `tools`, else the same-named tool from `from` is appended; 0 when neither has it. Shared by paste
/// (clipboard) and toolpath templates.
pub fn tool_index_for(tools: &mut Vec<Tool>, name: &str, from: &[Tool]) -> i32 {
    if let Some(i) = tools.iter().position(|t| t.name == name) {
        return i as i32;
    }
    match from.iter().find(|t| t.name == name) {
        Some(t) => {
            tools.push(t.clone());
            tools.len() as i32 - 1
        }
        None => 0,
    }
}

/// Defaults match the CAMaster reference files.
pub fn default_tools() -> Vec<Tool> {
    let t = |name: &str, kind, diameter, stepdown, feed_xy, feed_z| Tool {
        name: name.into(),
        kind,
        diameter,
        angle: 90.0,
        stepdown,
        feed_xy,
        feed_z,
        rpm: 18000,
        ..Default::default()
    };
    vec![
        t("1/4 Down End Mill (0.25 inch)", ToolType::EndMill, 0.25, 0.25, 432.0, 216.0),
        t("1/8 End Mill (0.125 inch)", ToolType::EndMill, 0.125, 0.125, 150.0, 50.0),
        t("V-Bit (90 deg 0.5\")", ToolType::VBit, 0.5, 0.25, 100.0, 50.0),
        t("1/4 Ball Nose (0.25 inch)", ToolType::BallNose, 0.25, 0.125, 200.0, 80.0),
    ]
}

/// Passed in so the document model does not depend on the expression parser's internals.

#[derive(Clone, Debug, PartialEq)]
struct Snap {
    shapes: Vec<Shape>,
    layers: Vec<Layer>,
    active_layer: i32,
    ops: Vec<Op>,
    tools: Vec<Tool>,
    material: Material,
    variables: Vec<Variable>,
    constraints: Vec<Constraint>,
    dimensions: Vec<Dimension>,
    sheets: Vec<Material>,
    active_sheet: i32,
    reliefs: Vec<Relief>,
    texts: Vec<TextItem>,
}

// ponytail: undo stores full snapshots; switch to diffs if documents get huge.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub shapes: Vec<Shape>,
    pub layers: Vec<Layer>,
    pub active_layer: i32,
    pub variables: Vec<Variable>,
    pub constraints: Vec<Constraint>,
    pub dimensions: Vec<Dimension>,
    /// inches per unit for bare numbers in expressions (project display unit)
    pub bare_unit_in: f64,
    pub ops: Vec<Op>,
    pub tools: Vec<Tool>,
    /// the active sheet's board settings (kept in sync with sheets[active_sheet])
    pub material: Material,
    /// 3D models carved into the board
    pub reliefs: Vec<Relief>,
    pub next_relief_id: i32,
    pub texts: Vec<TextItem>,
    pub next_text_id: i32,
    /// one board per sheet; shapes carry a sheet index
    pub sheets: Vec<Material>,
    pub active_sheet: i32,
    /// guide lines: (vertical, position in inches: x of a vertical guide, y of a horizontal one). Not part of undo.
    pub guides: Vec<(bool, f64)>,
    next_id: i32,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
}

impl Default for Document {
    fn default() -> Self {
        Document {
            shapes: Vec::new(),
            layers: vec![Layer::default()],
            active_layer: 0,
            variables: Vec::new(),
            constraints: Vec::new(),
            dimensions: Vec::new(),
            bare_unit_in: 1.0,
            ops: Vec::new(),
            tools: default_tools(),
            material: Material::default(),
            reliefs: Vec::new(),
            next_relief_id: 1,
            texts: Vec::new(),
            next_text_id: 1,
            sheets: vec![Material::default()],
            active_sheet: 0,
            guides: Vec::new(),
            next_id: 1,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}

thread_local! {
    static EVAL_DEPTH: Cell<i32> = const { Cell::new(0) }; // variables may reference each other; stop runaway cycles
}

impl Document {
    pub fn find_relief(&self, id: i32) -> Option<&Relief> {
        self.reliefs.iter().find(|r| r.id == id)
    }
    pub fn find_relief_mut(&mut self, id: i32) -> Option<&mut Relief> {
        self.reliefs.iter_mut().find(|r| r.id == id)
    }
    pub fn find_text(&mut self, id: i32) -> Option<&mut TextItem> {
        self.texts.iter_mut().find(|t| t.id == id)
    }
    pub fn sync_sheet(&mut self) {
        if let Some(s) = usize::try_from(self.active_sheet).ok().and_then(|i| self.sheets.get_mut(i)) {
            *s = self.material.clone();
        }
    }
    /// a sheet's name for the screen: its own, else "Sheet N" (the active one's lives in `material`)
    pub fn sheet_name(&self, j: usize) -> String {
        let m = if j as i32 == self.active_sheet { &self.material } else { match self.sheets.get(j) { Some(m) => m, None => return format!("Sheet {}", j + 1) } };
        if m.name.trim().is_empty() { format!("Sheet {}", j + 1) } else { m.name.clone() }
    }

    /// names sheet `j` ("" = back to "Sheet N")
    pub fn set_sheet_name(&mut self, j: usize, name: &str) {
        if j as i32 == self.active_sheet {
            self.material.name = name.trim().to_string();
        }
        if let Some(m) = self.sheets.get_mut(j) {
            m.name = name.trim().to_string();
        }
    }

    pub fn set_active_sheet(&mut self, i: i32) {
        self.sync_sheet();
        self.active_sheet = clamp(i, 0, self.sheets.len() as i32 - 1);
        self.material = self.sheets[self.active_sheet as usize].clone();
    }

    /// Adds a shape on `layer` (None = the active layer) and the active sheet; returns its id.
    pub fn add(&mut self, c: Contour, layer: Option<i32>) -> i32 {
        let id = self.next_id;
        let layer = layer.filter(|&l| l >= 0).unwrap_or(self.active_layer);
        self.shapes.push(Shape { id, c, layer, sheet: self.active_sheet, ..Default::default() });
        self.next_id += 1;
        id
    }

    /// Finds or creates; "" = active layer.
    pub fn layer_named(&mut self, name: &str) -> i32 {
        if name.is_empty() {
            return self.active_layer;
        }
        if let Some(i) = self.layers.iter().position(|l| l.name == name) {
            return i as i32;
        }
        const PALETTE: [&str; 7] = ["#d7dae0", "#7bd88f", "#6cb6ff", "#f0a8ff", "#ffd34d", "#ff8c69", "#8be9fd"];
        let color = PALETTE[self.layers.len() % 7].to_string();
        self.layers.push(Layer { name: name.into(), color, ..Default::default() });
        self.layers.len() as i32 - 1
    }

    pub fn find_variable(&self, name: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.name == name)
    }
    pub fn find_variable_mut(&mut self, name: &str) -> Option<&mut Variable> {
        self.variables.iter_mut().find(|v| v.name == name)
    }

    /// User variable, else a built-in from the board (user variables of the same name take precedence); inches
    /// (a plain-number variable is taken in the project unit).
    pub fn eval_variable(&self, name: &str) -> Result<f64, String> {
        let (v, len) = self.eval_variable_dim(name)?;
        Ok(if len { v } else { v * self.bare_unit_in })
    }

    /// Keeps the dimension: a variable "3" is (3, false), "2in" is (2, true).
    pub fn eval_variable_dim(&self, name: &str) -> Result<(f64, bool), String> {
        if let Some(v) = self.find_variable(name) {
            return self.eval_dim(&v.expr);
        }
        match name {
            "z" | "board-t" | "thickness" => Ok((self.material.t, true)),
            "board-w" => Ok((self.material.w, true)),
            "board-h" => Ok((self.material.h, true)),
            _ => Err(format!("unknown variable \"{name}\"")),
        }
    }

    /// Evaluates a length expression with this document's variables; inches.
    pub fn eval(&self, expr: &str) -> Result<f64, String> {
        let (v, len) = self.eval_dim(expr)?;
        Ok(if len { v } else { v * self.bare_unit_in })
    }

    /// Evaluates keeping the dimension (value, is_length); lengths in inches.
    pub fn eval_dim(&self, expr: &str) -> Result<(f64, bool), String> {
        if EVAL_DEPTH.get() > 40 {
            return Err("variables reference each other in a loop".into());
        }
        EVAL_DEPTH.set(EVAL_DEPTH.get() + 1);
        let r = crate::expr::eval_expr_dim(expr, self.bare_unit_in, &|n| self.eval_variable_dim(n).ok());
        EVAL_DEPTH.set(EVAL_DEPTH.get() - 1);
        r
    }

    /// name -> description
    pub fn builtin_variables() -> &'static [(&'static str, &'static str)] {
        &[("thickness", "board thickness (alias z)"), ("board-w", "board width"), ("board-h", "board height")]
    }

    /// type -> label
    pub fn constraint_types() -> &'static [(&'static str, &'static str)] {
        &[
            ("right-of", "Right of"),
            ("left-of", "Left of"),
            ("above", "Above"),
            ("below", "Below"),
            ("align-left", "Align left"),
            ("align-right", "Align right"),
            ("align-bottom", "Align bottom"),
            ("align-top", "Align top"),
            ("center-x", "Centre X"),
            ("center-y", "Centre Y"),
        ]
    }

    fn index_of(&self, id: i32) -> Option<usize> {
        self.shapes.iter().position(|s| s.id == id)
    }
    pub fn find(&self, id: i32) -> Option<&Shape> {
        self.shapes.iter().find(|s| s.id == id)
    }
    pub fn find_mut(&mut self, id: i32) -> Option<&mut Shape> {
        self.shapes.iter_mut().find(|s| s.id == id)
    }

    /// Moves targets to satisfy constraints (a few passes, in list order).
    pub fn apply_constraints(&mut self) {
        // ponytail: 3 passes in list order settle chains and most mutual references; a solver can replace this later
        for _ in 0..3 {
            for k in 0..self.constraints.len() {
                let c = &self.constraints[k];
                let (Some(ia), Some(ib)) = (self.index_of(c.anchor), self.index_of(c.target)) else { continue };
                if ia == ib {
                    continue;
                }
                let e = if c.expr.is_empty() {
                    0.0
                } else {
                    match self.eval(&c.expr) {
                        Ok(v) => v,
                        Err(_) => continue,
                    }
                };
                let (ra, rb) = (geom::bounds(&self.shapes[ia].c), geom::bounds(&self.shapes[ib].c));
                if ra.is_empty() || rb.is_empty() {
                    continue;
                }
                let (mut dx, mut dy) = (0.0, 0.0);
                match c.kind.as_str() {
                    "right-of" => dx = ra.x1 + e - rb.x0,
                    "left-of" => dx = ra.x0 - e - rb.x1,
                    "above" => dy = ra.y1 + e - rb.y0,
                    "below" => dy = ra.y0 - e - rb.y1,
                    "align-left" => dx = ra.x0 + e - rb.x0,
                    "align-right" => dx = ra.x1 + e - rb.x1,
                    "align-bottom" => dy = ra.y0 + e - rb.y0,
                    "align-top" => dy = ra.y1 + e - rb.y1,
                    "center-x" => dx = (ra.x0 + ra.x1) / 2.0 + e - (rb.x0 + rb.x1) / 2.0,
                    "center-y" => dy = (ra.y0 + ra.y1) / 2.0 + e - (rb.y0 + rb.y1) / 2.0,
                    _ => {}
                }
                if dx.abs() > 1e-12 || dy.abs() > 1e-12 {
                    let b = &mut self.shapes[ib];
                    b.c = transformed(&b.c, &Affine::translate(dx, dy));
                }
            }
        }
    }

    /// Re-evaluates every binding (C++ app Canvas::applyBindings): op and board fields (bind keys as saved: depth,
    /// stepdown, tabWidth…; w, h, t, safeZ, retractZ), node positions `n<i>.x` / `n<i>.y`, segment lengths `s<i>.len`
    /// (node i+1 moves along the segment), then the shape box x / y / w / h (anchored bottom-left).
    pub fn apply_bindings(&mut self) {
        let mut ops = std::mem::take(&mut self.ops);
        for op in &mut ops {
            for (k, e) in op.bind.clone() {
                if let (Some(f), Ok(v)) = (op_field(op, &k), self.eval(&e)) {
                    *f = v.max(0.0);
                }
            }
        }
        self.ops = ops;
        let mut m = self.material.clone();
        for (k, e) in &m.bind.clone() {
            if let (Some(f), Ok(v)) = (mat_field(&mut m, k), self.eval(e)) {
                *f = v.max(0.0);
            }
        }
        self.material = m;
        for si in 0..self.shapes.len() {
            if self.shapes[si].bind.is_empty() {
                continue;
            }
            let bind = self.shapes[si].bind.clone();
            let val = |k: &str| bind.get(k).and_then(|e| self.eval(e).ok());
            let vals: Vec<(String, Option<f64>)> = bind.keys().map(|k| (k.clone(), val(k))).collect();
            let (box_x, box_y, box_w, box_h) = (val("x"), val("y"), val("w"), val("h"));
            let s = &mut self.shapes[si];
            let n = s.c.nodes.len();
            for (k, v) in &vals {
                let (Some(rest), Some(v)) = (k.strip_prefix('n'), v) else { continue };
                let Some((i, axis)) = rest.split_once('.') else { continue };
                let Some(i) = i.parse::<usize>().ok().filter(|&i| i < n) else { continue };
                let nd = &mut s.c.nodes[i];
                let d = if axis == "x" { pt(v - nd.p.x, 0.0) } else { pt(0.0, v - nd.p.y) };
                nd.p = nd.p + d;
                nd.inh = nd.inh + d;
                nd.out = nd.out + d;
            }
            for (k, v) in &vals {
                let Some(i) = k.strip_prefix('s').and_then(|r| r.strip_suffix(".len")).and_then(|i| i.parse::<usize>().ok()) else { continue };
                let Some(v) = v.filter(|&v| v > 0.0) else { continue };
                if i >= n || (!s.c.closed && i == n - 1) {
                    continue;
                }
                let a = s.c.nodes[i].p;
                let b = &mut s.c.nodes[(i + 1) % n];
                let l = dist(a, b.p);
                let dir = if l > 1e-12 { (b.p - a) * (1.0 / l) } else { pt(1.0, 0.0) };
                let d = (a + dir * v) - b.p;
                b.p = b.p + d;
                b.inh = b.inh + d;
                b.out = b.out + d;
            }
            let b = geom::bounds(&s.c);
            if b.is_empty() {
                continue;
            }
            let (w, h) = (box_w.unwrap_or(b.w()), box_h.unwrap_or(b.h()));
            // a bound size grows away from the shape's anchor; a bound x / y says where the bottom-left goes instead
            let (fx, fy) = anchor_frac(s.anchor);
            let (x, y) = (box_x.unwrap_or(b.x0 + fx * (b.w() - w)), box_y.unwrap_or(b.y0 + fy * (b.h() - h)));
            let sx = if b.w() > 1e-9 && w > 0.0 { w / b.w() } else { 1.0 };
            let sy = if b.h() > 1e-9 && h > 0.0 { h / b.h() } else { 1.0 };
            if (sx - 1.0).abs() > 1e-12 || (sy - 1.0).abs() > 1e-12 || (x - b.x0).abs() > 1e-12 || (y - b.y0).abs() > 1e-12 {
                s.c = transformed(&s.c, &(Affine::translate(x, y) * Affine::scale(sx, sy) * Affine::translate(-b.x0, -b.y0)));
            }
        }
    }

    /// Anchored dimension endpoints follow their nodes; lost anchors detach.
    pub fn refresh_dimensions(&mut self) {
        let shapes = &self.shapes;
        let follow = |sid: &mut i32, nid: &mut i32, p: &mut Pt| {
            if *sid < 0 {
                return;
            }
            match shapes.iter().find(|s| s.id == *sid).and_then(|s| usize::try_from(*nid).ok().and_then(|n| s.c.nodes.get(n))) {
                Some(n) => *p = n.p,
                None => {
                    *sid = -1; // detach, keep last position
                    *nid = -1;
                }
            }
        };
        for d in &mut self.dimensions {
            follow(&mut d.shape_a, &mut d.node_a, &mut d.a);
            follow(&mut d.shape_b, &mut d.node_b, &mut d.b);
        }
    }

    /// Dimensions with `drive` set their length: same shape -> node B moves, else shape B translates.
    pub fn apply_driven_dimensions(&mut self) {
        for k in 0..self.dimensions.len() {
            let d = &self.dimensions[k];
            if d.drive.is_empty() || d.shape_a < 0 || d.shape_b < 0 {
                continue;
            }
            let (Some(ia), Some(ib)) = (self.index_of(d.shape_a), self.index_of(d.shape_b)) else { continue };
            if d.node_a >= self.shapes[ia].c.nodes.len() as i32 || d.node_b >= self.shapes[ib].c.nodes.len() as i32 {
                continue;
            }
            if ia == ib && d.node_a == d.node_b {
                continue;
            }
            let l = match self.eval(&d.drive) {
                Ok(l) if l > 0.0 => l,
                _ => continue,
            };
            let (na, nb) = (d.node_a as usize, d.node_b as usize);
            let (a, b) = (self.shapes[ia].c.nodes[na].p, self.shapes[ib].c.nodes[nb].p);
            if d.axis == 3 {
                continue; // angles measure only
            }
            let side = |v: f64| if v < 0.0 { -1.0 } else { 1.0 };
            let delta = match d.axis {
                1 => pt(a.x + side(b.x - a.x) * l - b.x, 0.0), // horizontal: only X changes
                2 => pt(0.0, a.y + side(b.y - a.y) * l - b.y),
                _ => {
                    let cur = dist(a, b);
                    let dir = if cur > 1e-12 { (b - a) * (1.0 / cur) } else { pt(1.0, 0.0) };
                    (a + dir * l) - b
                }
            };
            if delta.x.abs() < 1e-12 && delta.y.abs() < 1e-12 {
                continue;
            }
            let sb = &mut self.shapes[ib];
            if ia == ib {
                let n = &mut sb.c.nodes[nb];
                n.p = n.p + delta;
                n.inh = n.inh + delta;
                n.out = n.out + delta;
            } else {
                sb.c = transformed(&sb.c, &Affine::translate(delta.x, delta.y));
            }
        }
        self.refresh_dimensions();
    }

    /// After node insert (+1) / remove (-1) in a shape.
    pub fn shift_dimension_nodes(&mut self, shape_id: i32, at: i32, delta: i32) {
        let fix = |sid: &mut i32, nid: &mut i32| {
            if *sid != shape_id {
                return;
            }
            if delta < 0 && *nid == at {
                *sid = -1;
                *nid = -1;
                return;
            }
            if *nid >= at {
                *nid += delta;
            }
        };
        for d in &mut self.dimensions {
            fix(&mut d.shape_a, &mut d.node_a);
            fix(&mut d.shape_b, &mut d.node_b);
        }
    }

    /// project loader
    pub fn set_next_id(&mut self, n: i32) {
        self.next_id = n;
    }
    pub fn new_group_id(&self) -> i32 {
        self.shapes.iter().map(|s| s.group + 1).fold(1, i32::max)
    }
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
    pub fn shape_visible(&self, s: &Shape) -> bool {
        s.sheet == self.active_sheet && self.layers[s.layer as usize].visible
    }
    pub fn shape_editable(&self, s: &Shape) -> bool {
        let l = &self.layers[s.layer as usize];
        s.sheet == self.active_sheet && l.visible && !l.locked
    }

    pub fn bounds(&self) -> Rect {
        let mut r = Rect::default();
        for s in &self.shapes {
            r.add_rect(&geom::bounds(&s.c));
        }
        r
    }

    pub fn contours_of(&self, ids: &[i32]) -> Vec<Contour> {
        ids.iter().filter_map(|&id| self.find(id)).map(|s| s.c.clone()).collect()
    }

    /// A toolpath whose vectors on the active sheet all sit on hidden layers (or whose layer is hidden) is hidden
    /// with them in the previews; export is not affected.
    pub fn op_hidden_by_layers(&self, op: &Op) -> bool {
        let hidden = |l: i32| usize::try_from(l).ok().and_then(|i| self.layers.get(i)).is_some_and(|l| !l.visible);
        if op.layer_scope >= 0 && hidden(op.layer_scope) {
            return true;
        }
        let ids = self.op_shape_ids(op);
        !ids.is_empty() && ids.iter().all(|&id| self.find(id).is_some_and(|s| hidden(s.layer)))
    }

    /// Resolves layer/group scope, else op.shape_ids; only shapes on the active sheet.
    pub fn op_shape_ids(&self, op: &Op) -> Vec<i32> {
        self.op_shape_ids_on(op, self.active_sheet)
    }

    /// The shapes `op` cuts on `sheet`: none when the toolpath is kept to another sheet (`sheet_scope`), else every
    /// vector there (`all_vectors`, keep-out layers excepted), its layer's or group's, or its own picked shapes.
    pub fn op_shape_ids_on(&self, op: &Op, sheet: i32) -> Vec<i32> {
        if op.sheet_scope >= 0 && op.sheet_scope != sheet {
            return Vec::new();
        }
        if op.layer_scope < 0 && op.group_scope == 0 && !op.all_vectors {
            return op.shape_ids.iter().copied().filter(|&id| self.find(id).is_some_and(|s| s.sheet == sheet)).collect();
        }
        let keep_out = |l: i32| usize::try_from(l).ok().and_then(|i| self.layers.get(i)).is_some_and(|l| l.keep_out);
        self.shapes
            .iter()
            .filter(|s| {
                s.sheet == sheet
                    && if op.all_vectors {
                        !keep_out(s.layer)
                    } else {
                        (op.layer_scope >= 0 && s.layer == op.layer_scope) || (op.group_scope > 0 && s.group == op.group_scope)
                    }
            })
            .map(|s| s.id)
            .collect()
    }

    fn snap(&self) -> Snap {
        Snap {
            shapes: self.shapes.clone(),
            layers: self.layers.clone(),
            active_layer: self.active_layer,
            ops: self.ops.clone(),
            tools: self.tools.clone(),
            material: self.material.clone(),
            variables: self.variables.clone(),
            constraints: self.constraints.clone(),
            dimensions: self.dimensions.clone(),
            sheets: self.sheets.clone(),
            active_sheet: self.active_sheet,
            reliefs: self.reliefs.clone(),
            texts: self.texts.clone(),
        }
    }
    fn restore(&mut self, s: Snap) {
        self.shapes = s.shapes;
        self.layers = s.layers;
        self.active_layer = s.active_layer;
        self.ops = s.ops;
        self.tools = s.tools;
        self.material = s.material;
        self.variables = s.variables;
        self.constraints = s.constraints;
        self.dimensions = s.dimensions;
        self.sheets = s.sheets;
        self.active_sheet = s.active_sheet;
        self.reliefs = s.reliefs;
        self.texts = s.texts;
    }

    /// Call once before each user-level mutation.
    pub fn checkpoint(&mut self) {
        self.undo.push(self.snap());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        self.redo.push(self.snap());
        self.restore(s);
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        self.undo.push(self.snap());
        self.restore(s);
        true
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

/// Bindable op field by its saved key (C++ opField).
pub fn op_field<'a>(op: &'a mut Op, k: &str) -> Option<&'a mut f64> {
    Some(match k {
        "depth" => &mut op.depth,
        "stepdown" => &mut op.stepdown,
        "tabWidth" => &mut op.tab_width,
        "tabHeight" => &mut op.tab_height,
        "allowance" => &mut op.allowance,
        "peck" => &mut op.peck,
        "flatDepth" => &mut op.flat_depth,
        "startDepth" => &mut op.start_depth,
        "glueGap" => &mut op.glue_gap,
        "inlayMargin" => &mut op.inlay_margin,
        "rampIn" => &mut op.ramp_in,
        "rampOut" => &mut op.ramp_out,
        "rampLength" => &mut op.ramp_length,
        "chamferWidth" => &mut op.chamfer_width,
        _ => return None,
    })
}

/// Bindable board field by its saved key (C++ matField).
pub fn mat_field<'a>(m: &'a mut Material, k: &str) -> Option<&'a mut f64> {
    Some(match k {
        "w" => &mut m.w,
        "h" => &mut m.h,
        "t" => &mut m.t,
        "safeZ" => &mut m.safe_z,
        "retractZ" => &mut m.retract_z,
        _ => return None,
    })
}

/// std::clamp
pub(crate) fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        lo
    } else if hi < v {
        hi
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_bound_size_grows_away_from_the_anchor() {
        let mut doc = Document::default();
        doc.variables.push(Variable { name: "w".into(), expr: "2".into() });
        let id = doc.add(crate::geom::make_rect(4.0, 4.0, 6.0, 5.0), None);
        let s = doc.find_mut(id).unwrap();
        s.bind.insert("w".into(), "w".into());
        s.anchor = 8; // top-right stays put
        doc.variables[0].expr = "5".into();
        doc.apply_bindings();
        let b = crate::geom::bounds(&doc.find(id).unwrap().c);
        assert!((b.x1 - 6.0).abs() < 1e-9 && (b.x0 - 1.0).abs() < 1e-9 && (b.y1 - 5.0).abs() < 1e-9, "{b:?}");
        doc.find_mut(id).unwrap().anchor = 4; // centre: grows both ways
        doc.variables[0].expr = "3".into();
        doc.apply_bindings();
        let b = crate::geom::bounds(&doc.find(id).unwrap().c);
        assert!((b.x0 - 2.0).abs() < 1e-9 && (b.x1 - 5.0).abs() < 1e-9, "{b:?}");
    }

    #[test]
    fn horizontal_and_vertical_dimensions_measure_and_drive_one_axis() {
        let mut doc = Document::default();
        let a = doc.add(crate::geom::make_rect(0.0, 0.0, 1.0, 1.0), None);
        let b = doc.add(crate::geom::make_rect(4.0, 3.0, 5.0, 4.0), None);
        // node 0 of each rect = its bottom-left corner: (0,0) and (4,3)
        let dim = |axis| Dimension { a: pt(0.0, 0.0), b: pt(4.0, 3.0), shape_a: a, node_a: 0, shape_b: b, node_b: 0, axis, ..Default::default() };
        assert_eq!([dim(0).length(), dim(1).length(), dim(2).length()], [5.0, 4.0, 3.0]);
        doc.dimensions.push(Dimension { drive: "6".into(), ..dim(1) });
        doc.apply_driven_dimensions();
        let p = doc.find(b).unwrap().c.nodes[0].p;
        assert!((p.x - 6.0).abs() < 1e-9 && (p.y - 3.0).abs() < 1e-9, "{p:?}");
    }

    use super::*;
    use crate::geom::make_rect;

    /// Stand-in for crate::expr: numbers and single variable names only.
    #[test]
    fn undo_redo() {
        let mut d = Document::default();
        d.add(make_rect(0.0, 0.0, 1.0, 1.0), None);
        d.checkpoint();
        d.add(make_rect(0.0, 0.0, 2.0, 2.0), None);
        assert!(d.shapes.len() == 2 && d.undo() && d.shapes.len() == 1 && d.redo() && d.shapes.len() == 2);
        assert!(!d.redo() && d.can_undo());
    }

    #[test]
    fn hiding_a_layer_hides_the_toolpaths_that_cut_only_its_vectors() {
        let mut d = Document::default();
        d.layers.push(Layer { name: "Cut".into(), visible: true, ..d.layers[0].clone() });
        let a = d.add(make_rect(0.0, 0.0, 1.0, 1.0), None);
        let b = d.add(make_rect(2.0, 0.0, 1.0, 1.0), None);
        d.find_mut(b).unwrap().layer = 1;
        let only_b = Op { shape_ids: vec![b], ..Default::default() };
        let both = Op { shape_ids: vec![a, b], ..Default::default() };
        let layer = Op { layer_scope: 1, ..Default::default() };
        assert!(!d.op_hidden_by_layers(&only_b) && !d.op_hidden_by_layers(&layer));
        d.layers[1].visible = false;
        assert!(d.op_hidden_by_layers(&only_b) && d.op_hidden_by_layers(&layer));
        assert!(!d.op_hidden_by_layers(&both)); // still cuts a visible vector
        assert!(!d.op_hidden_by_layers(&Op::default())); // nothing picked: nothing to hide it by
    }

    #[test]
    fn variables_and_builtins() {
        let d = Document {
            variables: vec![Variable { name: "w".into(), expr: "board-w".into() }, Variable { name: "loop".into(), expr: "loop".into() }],
            ..Default::default()
        };
        assert_eq!(d.eval("w"), Ok(48.0));
        assert_eq!(d.eval("thickness"), Ok(0.75));
        // a variable can be a plain count, in any project unit
        let mut c = Document { bare_unit_in: 1.0 / 25.4, ..Default::default() };
        c.variables.push(Variable { name: "count".into(), expr: "3".into() });
        c.variables.push(Variable { name: "pitch".into(), expr: "2in".into() });
        assert!((c.eval("pitch * count").unwrap() - 6.0).abs() < 1e-12);
        assert_eq!(c.eval_variable_dim("count"), Ok((3.0, false)));
        assert!((c.eval("board-w / 2").unwrap() - 24.0).abs() < 1e-12);
        assert!((c.eval("2 * 3").unwrap() - 6.0 / 25.4).abs() < 1e-12);
        assert!(d.eval("loop").is_err());
        assert!(d.eval_variable("nope").unwrap_err().contains("nope"));
    }

    #[test]
    fn bindings_follow_variables() {
        let mut d = Document::default();
        d.variables.push(Variable { name: "w".into(), expr: "4".into() });
        let a = d.add(make_rect(1.0, 1.0, 3.0, 2.0), None);
        d.find_mut(a).unwrap().bind.insert("w".into(), "w".into());
        d.find_mut(a).unwrap().bind.insert("x".into(), "board-w".into());
        let b = d.add(make_rect(0.0, 0.0, 1.0, 1.0), None);
        d.find_mut(b).unwrap().bind.insert("n2.y".into(), "3".into());
        d.find_mut(b).unwrap().bind.insert("s0.len".into(), "2".into());
        d.ops.push(Op { bind: [("depth".to_string(), "thickness".to_string())].into(), ..Default::default() });
        d.material.bind.insert("t".into(), "0.5".into());
        d.apply_bindings();
        let ra = geom::bounds(&d.find(a).unwrap().c);
        assert!((ra.w() - 4.0).abs() < 1e-9 && (ra.x0 - 48.0).abs() < 1e-9 && (ra.y0 - 1.0).abs() < 1e-9 && (ra.h() - 1.0).abs() < 1e-9);
        let nb = &d.find(b).unwrap().c.nodes;
        assert!((nb[2].p.y - 3.0).abs() < 1e-9 && (nb[1].p.x - 2.0).abs() < 1e-9, "{:?}", nb.iter().map(|n| n.p).collect::<Vec<_>>());
        assert_eq!(d.material.t, 0.5);
        assert_eq!(d.ops[0].depth, 0.75, "ops are evaluated before the board, like the C++ app");
    }

    #[test]
    fn constraints_dimensions_scope() {
        let mut d = Document::default();
        let a = d.add(make_rect(0.0, 0.0, 2.0, 1.0), None);
        let b = d.add(make_rect(5.0, 5.0, 6.0, 6.0), None);
        d.constraints.push(Constraint { anchor: a, target: b, kind: "right-of".into(), expr: "0.5".into() });
        d.constraints.push(Constraint { anchor: a, target: b, kind: "align-bottom".into(), expr: String::new() });
        d.apply_constraints();
        let rb = geom::bounds(&d.find(b).unwrap().c);
        assert!((rb.x0 - 2.5).abs() < 1e-9 && rb.y0.abs() < 1e-9);

        // driven dimension along one shape: node 1 moves to length 3
        d.dimensions.push(Dimension { shape_a: a, node_a: 0, shape_b: a, node_b: 1, drive: "3".into(), ..Default::default() });
        d.apply_driven_dimensions();
        assert!((d.find(a).unwrap().c.nodes[1].p.x - 3.0).abs() < 1e-9 && (d.dimensions[0].b.x - 3.0).abs() < 1e-9);
        d.shift_dimension_nodes(a, 1, 1);
        assert_eq!(d.dimensions[0].node_b, 2);
        d.shift_dimension_nodes(a, 2, -1);
        assert_eq!((d.dimensions[0].shape_b, d.dimensions[0].node_b), (-1, -1));

        assert_eq!(d.layer_named("Cut"), 1);
        assert_eq!(d.layer_named("Cut"), 1);
        assert_eq!(d.layers[1].color, "#7bd88f");
        d.shapes[1].layer = 1;
        d.shapes[0].group = 3;
        assert_eq!(d.new_group_id(), 4);
        let op = Op { layer_scope: 1, ..Default::default() };
        assert_eq!(d.op_shape_ids(&op), vec![b]);
        let op = Op { shape_ids: vec![a, 99], ..Default::default() };
        assert_eq!(d.op_shape_ids(&op), vec![a]);
        d.sheets.push(Material { w: 10.0, ..Default::default() });
        d.set_active_sheet(1);
        assert!(d.op_shape_ids(&op).is_empty() && d.material.w == 10.0);
    }
}
