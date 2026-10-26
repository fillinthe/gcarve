//! ⌘K command palette: GCarve's command list (Main.qml paletteCommands): every toolbar command, tool, toolpath
//! kind, view command, post and variable. The palette itself (filter, keys, drawing) is gcarve-ui's, shared with GSketch.

use crate::shell;
pub use gcarve_ui::palette::{Command, Palette};

/// Main.qml tbDefs order
const TB: [&str; 40] = ["new", "open", "jobs", "save", "undo", "redo", "copy", "paste", "clone", "group", "delete", "fliph", "flipv", "rotate", "align", "array", "nest", "link", "weld", "subtract", "intersect", "offset", "dogbone", "join", "close", "paths", "width", "sim", "watch", "vars", "tools", "posts", "board", "export", "edittext", "tocurves", "opedit", "opdelete", "restart", "view3d"];
const TOOLS: [(&str, &str, &str); 14] = [("Select", "V", "tool:select"), ("Nodes", "N", "tool:nodes"), ("Rect", "R", "tool:rect"), ("Ellipse", "E", "tool:ellipse"), ("Polygon", "G", "tool:polygon"), ("Star", "S", "tool:star"), ("Polyline", "P", "tool:polyline"), ("Curve", "B", "tool:curve"), ("Arc", "A", "tool:arc"), ("Freehand", "H", "tool:freehand"), ("Measure", "M", "tool:measure"), ("Dimension", "D", "tool:dimension"), ("Angle", "L", "tool:angle"), ("Text", "T", "tool:text")];
const VIEWS: [(&str, &str, &str); 54] = [
    ("Getting started guide (checklist)", "", "tour:show"),
    ("How do I…? (help)", "", "guide"),
    ("Check vectors (open, duplicate, tiny)", "", "validate"),
    ("Surface the board (plane the top flat)", "", "surface"),
    ("Extend open vectors to the next vector", "", "extend"),
    ("Simplify vectors: fine (fewer nodes)", "", "simplify:0.001"),
    ("Simplify vectors: medium", "", "simplify:0.005"),
    ("Simplify vectors: coarse", "", "simplify:0.02"),
    ("Fit arcs and curves to vectors (smooth)", "", "fitcurves:0.005"),
    ("Fillet / dogbone corners…", "", "dogbone"),
    ("Vector Selector (open, closed, circles, size, layer)…", "", "selector"),
    ("Vector texture (wavy lines filling shapes)…", "", "vtexture"),
    ("Distort: bend shapes along a curve", "", "distort:1"),
    ("Distort: fit shapes between two curves", "", "distort:2"),
    ("Distort: fit shapes into four corners (corner pin)", "", "distort:4"),
    ("Plate production (name plates / badges from a list)…", "", "plates"),
    ("Shelf-pin holes (drill bank, system 32)…", "", "shelfpins"),
    ("Box maker: finger-jointed box…", "", "boxmaker"),
    ("New sign: carved words, border, cut out…", "", "sign"),
    ("Rotary layout: wrapped spiral or flutes…", "", "rotarylayout"),
    ("Copy along a vector (array)…", "", "array"),
    ("Export vectors as SVG", "", "vexport:svg"),
    ("Export vectors as DXF", "", "vexport:dxf"),
    ("Export vectors as DWG", "", "vexport:dwg"),
    ("Export vectors as PDF", "", "vexport:pdf"),
    ("Send to GBurn (laser cut / engrave)", "", "sendgburn"),
    ("New back side: flip left to right (two-sided job)", "", "sheet:back-lr"),
    ("New back side: flip top to bottom (two-sided job)", "", "sheet:back-tb"),
    ("Clear guide lines", "", "guides:clear"),
    ("Connect a SpaceMouse", "", "spacemouse"),
    ("Choose machine…", "", "machine"),
    ("Design mode", "1", "mode:Design"),
    ("Toolpaths tab", "2", "tab:toolpaths"),
    ("Simulate mode", "3", "mode:Simulate"),
    ("Zoom to fit", "⌘0", "zoom:fit"),
    ("Zoom to board", "⌘1", "zoom:board"),
    ("3D view", "⌘3", "view3d"),
    ("Variables & constraints…", "⇧⌘E", "vars"),
    ("Preferences…", "⌘,", "prefs"),
    ("3D models…", "", "reliefs"),
    ("Import 3D model (STL)…", "", "importstl"),
    ("Import G-code…", "", "importgcode"),
    ("Trace image (bitmap to vectors)…", "", "trace"),
    ("Shapes library (clipart)…", "", "shapes"),
    ("Report a bug…", "", "bug"),
    ("Request a change…", "", "request"),
    ("Check for updates…", "", "update"),
    ("Add text…", "T", "tool:text"),
    ("Align & distribute…", "⇧⌘A", "align"),
    ("Nest on board…", "", "nest"),
    ("New sheet", "", "sheet:add"),
    ("Setup sheet…", "", "setupsheet"),
    ("Select all", "⌘A", "selectall"),
    ("Command palette", "⌘K", "palette"),
];

/// toolbar ids whose run id differs (Main.qml tbRun)
fn run_id(id: &str) -> &str {
    match id {
        "group" => "group:toggle",
        "edittext" => "text:edit",
        "tocurves" => "text:curves",
        "restart" => "sim:restart",
        _ => id,
    }
}

/// Every command. `posts` = (id, name), `post` = the board's post, `vars` = (name, expr).
pub fn commands(posts: &[(String, String)], post: &str, vars: &[(String, String)]) -> Vec<Command> {
    let mut c = Vec::new();
    for id in TB {
        let (_, text, tip) = shell::def(id);
        let key = tip.rsplit_once('(').map(|(_, k)| k.trim_end_matches(')')).filter(|k| k.chars().count() <= 5 || k.contains('⌘')).unwrap_or("");
        c.push(Command { cat: "Command", text: format!("{text} — {}", tip.split(" (").next().unwrap_or(tip)), key: key.into(), id: run_id(id).into() });
    }
    for (name, key, id) in TOOLS {
        c.push(Command { cat: "Tool", text: name.into(), key: key.into(), id: id.into() });
    }
    for (i, (name, _, _)) in shell::KINDS.iter().take(crate::inspector::KIND_TYPES.len()).enumerate() {
        c.push(Command { cat: "Toolpath", text: format!("{name} toolpath…"), key: String::new(), id: format!("kind:{i}") });
    }
    for (text, key, id) in VIEWS {
        c.push(Command { cat: "View", text: text.into(), key: key.into(), id: id.into() });
    }
    for (id, name) in posts {
        c.push(Command { cat: "Post", text: name.clone(), key: if id == post { "current".into() } else { String::new() }, id: format!("post:{id}") });
    }
    for (name, expr) in vars {
        c.push(Command { cat: "Variable", text: format!("{name} = {expr}"), key: String::new(), id: "vars".into() });
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcarve_ui::palette::filter;

    #[test]
    fn every_command_and_ranking() {
        let cmds = commands(&[("mach3".into(), "Mach3 (inch)".into())], "mach3", &[("gap".into(), "0.5".into())]);
        assert!(cmds.iter().any(|c| c.id == "vars" && c.key == "⇧⌘E"));
        assert!(cmds.iter().any(|c| c.text.starts_with("Export — Export G-code") && c.key == "⌘E"));
        assert!(cmds.iter().any(|c| c.cat == "Post" && c.id == "post:mach3" && c.key == "current"));
        assert!(cmds.iter().any(|c| c.cat == "Variable" && c.text == "gap = 0.5"));
        assert_eq!(cmds.iter().filter(|c| c.cat == "Toolpath").count(), 12);
        let hits = filter(&cmds, "v-carve");
        assert_eq!(cmds[hits[0]].id, "kind:3");
        let hits = filter(&cmds, "mach");
        assert_eq!(cmds[hits[0]].id, "post:mach3");
        assert_eq!(filter(&cmds, "").len(), 12);
        assert!(filter(&cmds, "zzzz").is_empty());
    }
}
