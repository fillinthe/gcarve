//! The engine (gcarve-core) writes its warnings and safety checks in English; these are their patterns, so the
//! packs translate them and `i18n::msg` shows them in the UI language. Keep in step with the `format!`s in
//! cam.rs / pipeline.rs: a message with no pattern here just stays English.

#[cfg_attr(not(test), allow(dead_code))]
pub const ENGINE_TEXT: &[&str] = &[ // i18n
    "{name}: no tool",
    "{name}: tabs don't fit, skipped",
    "{name}: depth must be > 0",
    "{name}: depth is deeper than the material",
    "{name}: onion skin holds the parts, tabs are left out",
    "{name}: open shape cut on-line",
    "{name}: a shape is too small for the tool",
    "{name}: no room for the lead-in in a small shape, plunging on the line there",
    "{name}: this machine's post can't write cutter compensation (G41/G42): the file has none",
    "{name}: open shapes can't be pocketed, skipped",
    "{name}: pocket is too small for the tool",
    "{name}: adaptive clearing: {why}; cut with rings instead",
    "{name}: the cross-section must be an open vector with some width: draw its profile from left to right",
    "{name}: a drag knife must not spin: set the bit's spindle speed (RPM) to 0, so the G-code never turns the spindle on",
    "{name}: rounding needs a rotary job (Job Setup › Rotary)",
    "{name}: a {side} in square already fits inside the {dia} in diameter: nothing to round",
    "{name}: select a shape across the length to round",
    "{name}: the travel heights must clear the blank's corners, {top} in above the zero",
    "{name}: a hole is bigger than the drill, only its center is drilled",
    "{name}: needs a V-bit tool",
    "{name}: flat depth is deeper than the material",
    "{name}: start depth must be above the flat depth",
    "{name}: open shapes can't be v-carved, skipped",
    "{name}: nothing to carve",
    "{name}: carves {deep} in deep but the bit's V is only {v} in deep; the widest parts get stepped walls (use a wider bit or a flat depth)",
    "{name}: shape is too wide to carve to a point within the material; floor clamped to the thickness",
    "{name}: clearance tool is too big for the flat area, V-bit clears it",
    "{name}: start depth needs a clearance end mill to remove the material above it",
    "{name}: chamfer size must be > 0",
    "{name}: chamfer is wider than the bit's V (use a wider bit)",
    "{name}: max depth must be > 0",
    "{name}: open shapes can't be prism carved, skipped",
    "{name}: bevel is {deep} in deep but the bit's V is only {v} in deep; the facets stop short of the center (use a wider bit or a smaller max depth)",
    "{name}: inlay needs a flat depth (pocket depth of the female)",
    "{name}: glue gap must be between 0 and the flat depth",
    "{name}: pick a clearance end mill so the male blank is cleared to the glue gap",
    "{name}: hole is not larger than the bit, drilled straight",
    "{name}: open shapes can't hold a texture, skipped",
    "{name}: the shapes are too small for this texture",
    "{name}: pick a picture for the photo carving",
    "{name}: select a closed shape (a rectangle) to hold the picture",
    "{name}: the picture is too light to carve at this depth",
    "Safe Z is not above the material: rapids would drag the bit across the board.",
    "Retract Z is not above the material.",
    "Retract Z is above safe Z.",
    "{who}: feed rate is zero.",
    "{who}: spindle speed is zero.",
    "{who}: rapid sideways move inside the material.",
    "{who}: cuts outside the board.",
    "{who}: cuts {deep} in into the spoilboard (deepest Z {z}, board {t} thick).",
    "{who}: cuts {deep} in deep but the bit's flutes are only {len} in long: the shank would rub (burning, a broken bit). Use a longer bit or a shallower cut.",
    "{who}: cuts {deep} in deep but the bit sticks out only {len} in: the collet / holder would hit the board. Pull the bit out further or cut shallower.",
    "{who}: comes down inside a keep-out area (layer \"{layer}\"): clamp or screw in the way.",
    "\"{name}\" cuts {n} small part{s} free with no tabs: {s2} can shift into the bit or be thrown. Add tabs or an onion skin, or hold {s3} down.",
    "\"{name}\" cuts the part free before {later} {s} cut: move it below {s2} in the toolpath list (right-click › Cut Later), or give it tabs.",
    "Board {w} × {h} in is bigger than the {machine} travel ({x} × {y} in).",
    "Board thickness + safe Z ({z} in) is more than the {machine} Z travel ({tz} in).",
    "This job uses {n} bits but the {machine} tool changer has {slots} slots.",
    "{bit}: slot {slot} is beyond the {slots}-slot tool changer.",
    "{a} and {b} are both in slot {slot}.",
    "{bit}: {rpm} RPM is above the spindle's {max} max.",
    "{bit}: {rpm} RPM is below the spindle's {min} minimum.",
    "{bit}: feed {feed} in/min is above the machine's {max} in/min.",
    "{bit}: plunge {feed} in/min is above the machine's {max} in/min.",
    "Rotary job: the \"{post}\" post writes plain numbers and can't name an A axis; choose a G-code post for the rotary.",
    "{name}: rounding needs a rotary job (Job Setup › Rotary)",
    "Board {w} × {h} in is bigger than the {machine} travel ({tx} × {ty} in).",
    "{name}: 3D model not loaded",
    "{name}: relief has no model loaded",
    "{name}: relief is deeper than the material",
    "Tiling doesn't apply to a rotary job: the files are NOT tiled.",
    "Tiling needs XY zero at the board's bottom-left corner (Board settings): the files are NOT tiled.",
    "Tiling: the tile length must be larger than the overlap: the files are NOT tiled.",
];

#[cfg(test)]
mod tests {
    use super::ENGINE_TEXT;
    use gcarve_ui::i18n;

    #[test]
    fn engine_warnings_match_their_patterns_and_every_pack_has_them() {
        // (no i18n::set here: the language is global and other tests run alongside)
        let packs: Vec<std::collections::HashMap<String, String>> = i18n::LANGS.iter().skip(1).map(|(_, _, j)| serde_json::from_str(j).unwrap()).collect();
        // names with spaces and commas, the way real toolpath, bit and machine names look; plural bits are one word
        let val = |n: &str| if n.starts_with('s') && n.len() <= 2 { "are" } else { "Pocket 2, V-Carve 1" };
        for k in ENGINE_TEXT {
            let filled = i18n::fill(k, |n| Some(val(n)));
            if k.contains('{') {
                let caps = i18n::match_pattern(k, &filled).unwrap_or_else(|| panic!("{k} doesn't match its own text"));
                assert!(caps.iter().all(|(n, c)| *c == val(n)), "{k} reads {caps:?} back");
            }
            assert!(packs.iter().all(|p| p.get(*k).is_some_and(|v| !v.is_empty())), "{k} is not in every pack");
        }
        assert_eq!(i18n::match_pattern("{name}: depth must be > 0", "Cut out: depth must be > 0"), Some(vec![("name", "Cut out")]));
    }
}
