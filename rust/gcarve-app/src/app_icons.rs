//! GCarve's icon set for `icons::install` (the Qt app's app/icons/*.svg)

macro_rules! icons {
    ($($n:literal),* $(,)?) => { &[ $( ($n, include_str!(concat!("../../../app/icons/", $n, ".svg"))) ),* ] };
}
pub const RAW: &[(&str, &str)] = icons![
    "add", "align", "align-bottom", "align-left", "align-right", "align-top", "arc", "array", "arrows-h", "arrows-v", "board", "bore", "chamfer", "chevron-down", "chevron-left", "chevron-right", "close", "copy", "delete", "dimension", "dismiss", "drill",
    "duplicate", "edit", "ellipse", "export", "eye-off", "eye", "fit", "flip-h", "flip-v", "flute", "grip", "group", "heart", "inlay", "intersect", "jobs",
    "join", "layers", "link", "machine", "lock", "measure", "nest", "new", "nodes", "offset", "open", "paste", "pen", "photo", "pocket", "polygon", "polyline", "prism", "profile",
    "rect", "redo", "relief", "remove", "rotate", "save", "search", "select", "simulate", "star", "subtract", "swap", "text", "texture", "undo", "ungroup", "unlock",
    "variable", "vcarve", "weld", "zoom-in", "zoom-out",
];

