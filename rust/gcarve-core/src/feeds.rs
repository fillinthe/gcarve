//! Feeds & speeds advisor (C++ `core/feeds.*`): chip-load tables per material class -> suggested feed/rpm/pass
//! depth, and a risk assessment ("meter") of the values a user typed. Inches, in/min. Tables follow published
//! router-bit chip loads (Onsrud/Amana style) for 2-flute spiral bits in common CNC materials; starting points, not gospel.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialClass {
    Softwood,
    Hardwood,
    Plywood,
    Mdf,
    Plastic,
    Aluminum,
    Foam,
}

/// The parts of a tool the advisor looks at. C++ takes the whole `Tool` (core/cam.h); the Rust `Tool` (cam.rs,
/// not ported yet) should provide a `cutter()` view. ponytail: separate struct only until cam.rs lands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cutter {
    pub vbit: bool,
    pub diameter: f64,
    pub flutes: i32,
}

fn has(s: &str, words: &[&str]) -> bool {
    words.iter().any(|w| s.contains(w))
}

/// free text ("Baltic birch", "red oak", "HDPE"...) -> class
pub fn classify_material(name: &str) -> MaterialClass {
    let s = name.to_ascii_lowercase();
    if has(&s, &["mdf", "fiberboard", "hdf", "particle"]) {
        return MaterialClass::Mdf;
    }
    if has(&s, &["ply", "birch", "osb", "laminate", "melamine"]) {
        return MaterialClass::Plywood;
    }
    if has(&s, &["alumin", "brass", "copper", "metal"]) {
        return MaterialClass::Aluminum;
    }
    if has(&s, &["acrylic", "plexi", "hdpe", "pvc", "plastic", "polycarb", "lexan", "delrin", "acetal", "abs", "uhmw", "nylon", "corian", "solid surface"]) {
        return MaterialClass::Plastic;
    }
    if has(&s, &["foam", "eps", "xps", "styro", "renshape", "tooling board", "sign foam", "hdu", "precision board"]) {
        return MaterialClass::Foam;
    }
    if has(&s, &["pine", "cedar", "fir", "spruce", "poplar", "basswood", "redwood", "softwood", "alder", "cypress", "hemlock"]) {
        return MaterialClass::Softwood;
    }
    MaterialClass::Hardwood // oak, maple, walnut, cherry, hardwoods, unknown
}

/// Bit-database chip-load groups (data/bits/chipload.json) to try for a class, best match first.
pub fn chip_groups(c: MaterialClass) -> &'static [&'static str] {
    match c {
        MaterialClass::Softwood => &["softwood", "wood"],
        MaterialClass::Hardwood => &["hardwood", "wood"],
        MaterialClass::Plywood => &["plywood", "plywood-hard", "laminate", "wood"],
        MaterialClass::Mdf => &["mdf"],
        MaterialClass::Plastic => &["plastic-hard", "plastic-soft", "acrylic", "solid-surface"],
        MaterialClass::Aluminum => &["aluminum", "brass"],
        MaterialClass::Foam => &["foam"],
    }
}

pub fn material_class_name(c: MaterialClass) -> &'static str {
    match c {
        MaterialClass::Softwood => "softwood",
        MaterialClass::Hardwood => "hardwood",
        MaterialClass::Plywood => "plywood",
        MaterialClass::Mdf => "MDF",
        MaterialClass::Plastic => "plastic",
        MaterialClass::Aluminum => "aluminum",
        MaterialClass::Foam => "foam",
    }
}

/// inches per tooth
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChipRange {
    pub lo: f64,
    pub hi: f64,
}

/// Interpolated over 1/8 .. 1/2 in, clamped (tiny bits scale down).
pub fn chip_load_range(c: MaterialClass, diameter: f64) -> ChipRange {
    // chip load (in/tooth) at 1/8, 1/4, 1/2 inch diameters: (lo, hi)
    const ROWS: [([f64; 3], [f64; 3]); 7] = [
        ([0.004, 0.009, 0.019], [0.006, 0.011, 0.021]), // Softwood
        ([0.003, 0.008, 0.016], [0.005, 0.010, 0.018]), // Hardwood
        ([0.004, 0.011, 0.021], [0.006, 0.013, 0.023]), // Plywood
        ([0.004, 0.011, 0.021], [0.007, 0.013, 0.023]), // MDF
        ([0.002, 0.004, 0.010], [0.004, 0.006, 0.012]), // Plastic
        ([0.001, 0.003, 0.006], [0.002, 0.004, 0.008]), // Aluminum
        ([0.006, 0.012, 0.025], [0.008, 0.015, 0.030]), // Foam
    ];
    const DIA: [f64; 3] = [0.125, 0.25, 0.5];
    let (lo, hi) = ROWS[c as usize];
    let d = diameter.clamp(DIA[0], DIA[2]);
    let i = if d <= DIA[1] { 0 } else { 1 };
    let f = (d - DIA[i]) / (DIA[i + 1] - DIA[i]);
    let mut cr = ChipRange { lo: lo[i] + (lo[i + 1] - lo[i]) * f, hi: hi[i] + (hi[i + 1] - hi[i]) * f };
    if diameter < DIA[0] {
        // tiny bits: scale down
        let k = diameter / DIA[0];
        cr.lo *= k;
        cr.hi *= k;
    }
    cr
}

fn cutting_diameter(t: &Cutter) -> f64 {
    // ponytail: a V-bit cuts with a small effective diameter; treat it like a 1/8 bit for chip load
    if t.vbit { t.diameter.min(0.125) } else { t.diameter }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedSuggestion {
    pub feed_xy: f64,
    pub feed_z: f64,
    pub stepdown: f64,
    pub rpm: i32,
    pub chip_load: f64,
}

/// C++ defaults: max_rpm 18000, max_feed 600.
pub fn suggest_feeds(t: &Cutter, material: &str, max_rpm: i32, max_feed: f64) -> FeedSuggestion {
    let c = classify_material(material);
    let flutes = t.flutes.max(1);
    let cr = chip_load_range(c, cutting_diameter(t));
    let chip = (cr.lo + cr.hi) / 2.0;
    let mut rpm = max_rpm.min(if t.diameter >= 0.5 { 16000 } else { 18000 });
    if c == MaterialClass::Plastic {
        rpm = rpm.min(16000); // keep plastics from melting
    }
    if c == MaterialClass::Aluminum {
        rpm = rpm.min(14000);
    }
    let feed = (f64::from(rpm * flutes) * chip).min(max_feed); // machine-limited: chip load drops, still fine for wood
    let plunge = if t.vbit { feed / 2.0 } else { feed / 3.0 };
    let mut step = match c {
        MaterialClass::Softwood | MaterialClass::Foam => t.diameter,
        MaterialClass::Plywood | MaterialClass::Mdf => t.diameter * 0.75,
        MaterialClass::Hardwood | MaterialClass::Plastic => t.diameter * 0.5,
        MaterialClass::Aluminum => t.diameter * 0.2,
    };
    if t.vbit {
        step = 0.25f64.min(0.1f64.max(t.diameter * 0.5));
    }
    FeedSuggestion { feed_xy: feed.round(), feed_z: plunge.round(), stepdown: (step * 1000.0).round() / 1000.0, rpm, chip_load: chip }
}

/// 0..1 meter: ~0.5 is the sweet spot, < 0.25 rubbing/burning, > 0.75 overloading the bit. level: 0 ok, 1 caution, 2 danger.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedAssessment {
    pub chip_load: f64,
    pub meter: f64,
    pub level: i32,
    pub message: String,
}

pub fn assess_feeds(t: &Cutter, material: &str, feed_xy: f64, feed_z: f64, rpm: i32, stepdown: f64) -> FeedAssessment {
    let cr = chip_load_range(classify_material(material), cutting_diameter(t));
    assess_feeds_range(t, cr, material, feed_xy, feed_z, rpm, stepdown)
}

/// Like `assess_feeds` against a given chip-load range (the bit maker's, when the tool came from the catalog).
/// Feed factor for a pass deeper than the bit is wide: 100% up to 1×D, 80% at 2×D, 60% from 3×D (the usual
/// shop chart rule: a deeper pass loads the bit more, so the chip has to get thinner).
pub fn depth_factor(diameter: f64, stepdown: f64) -> f64 {
    if diameter <= 0.0 {
        return 1.0;
    }
    (1.0 - 0.2 * (stepdown / diameter - 1.0)).clamp(0.6, 1.0)
}

pub fn assess_feeds_range(t: &Cutter, cr: ChipRange, material: &str, feed_xy: f64, feed_z: f64, rpm: i32, stepdown: f64) -> FeedAssessment {
    let c = classify_material(material);
    let k = if t.vbit { 1.0 } else { depth_factor(t.diameter, stepdown) };
    let cr = ChipRange { lo: cr.lo * k, hi: cr.hi * k };
    let flutes = t.flutes.max(1);
    let chip = if rpm > 0 { feed_xy / f64::from(rpm * flutes) } else { 0.0 };
    // map chip load onto the meter: lo -> 0.3, hi -> 0.7 (log-ish beyond)
    let (mid, half) = ((cr.lo + cr.hi) / 2.0, (cr.hi - cr.lo) / 2.0);
    let mut m = 0.5 + (chip - mid) / half * 0.2;
    if m < 0.3 {
        m = 0.3 - 0.3 * (1.0 - chip / cr.lo); // 0 at zero chip
    }
    if m > 0.7 {
        m = 0.7 + 0.3 * 1f64.min((chip - cr.hi) / (cr.hi * 2.0));
    }
    let cls = material_class_name(c);
    let (lo, hi) = (cr.lo, cr.hi);
    let (mut level, mut message) = if chip < lo * 0.5 {
        (2, format!("Rubbing: {chip:.4} in/tooth is far below {lo:.3}–{hi:.3} for {cls}. Burn marks and a dull bit. Raise feed or lower RPM."))
    } else if chip < lo {
        (1, format!("A bit slow: {chip:.4} in/tooth, {cls} likes {lo:.3}–{hi:.3}. Fine, but may burn in hardwood."))
    } else if chip > hi * 1.6 {
        (2, format!("Overloaded: {chip:.4} in/tooth vs {lo:.3}–{hi:.3} for {cls}. Deflection, tear-out, snapped bit. Slow down or raise RPM."))
    } else if chip > hi {
        (1, format!("Aggressive: {chip:.4} in/tooth, above {hi:.3} for {cls}. OK for roughing with a sharp bit."))
    } else {
        (0, format!("Good: {chip:.4} in/tooth is in the {lo:.3}–{hi:.3} range for {cls}."))
    };
    let max_step = match c {
        MaterialClass::Aluminum => t.diameter * 0.3,
        MaterialClass::Hardwood | MaterialClass::Plastic => t.diameter,
        _ => t.diameter * 1.5,
    };
    if k < 1.0 {
        message += &format!(" Pass is {:.1}× the bit: range lowered to {:.0}%.", stepdown / t.diameter, k * 100.0);
    }
    if !t.vbit && stepdown > max_step * 1.3 {
        level = 2;
        message += &format!(" Pass depth is too deep for this bit in {cls}.");
    } else if !t.vbit && stepdown > max_step {
        level = level.max(1);
        message += " Deep pass: keep the feed modest.";
    }
    if feed_z > feed_xy * 0.6 && feed_z > 0.0 {
        level = level.max(1);
        message += " Plunge is fast; straight bits plunge poorly.";
    }
    FeedAssessment { chip_load: chip, meter: m.clamp(0.0, 1.0), level, message }
}
