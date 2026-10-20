//! Help ▸ How Do I…? — short answers to "where is that?", searchable. One entry per task: what people type, then
//! where to click. Kept next to the features: when a menu item moves, its line here moves with it.


/// (question, answer, command to run from the "Do it" button or "")
pub const TOPICS: &[gcarve_ui::guide::Topic] = &[
    // getting started
    ("Start a job / set the board size", "Toolpaths tab › Board (or the pencil next to the board line under the canvas): size, thickness, material, where XY zero and Z zero are.", "board"),
    ("Choose my machine", "Toolpaths tab › Machine. Its size and spindle limit the suggested feeds, and its controller picks the right post-processor.", "machine"),
    ("Get the desktop app (Mac, Windows, Linux)", "Help › Download the Desktop App: free, works offline, signing in is optional. The same menu item on the desktop opens the page with the newest version.", "download"),
    ("See an example", "Help › Open the Demo Project: a sign with every toolpath type. It opens as the carved board in 3D (drag to turn it); press 1 for the drawing and its toolpaths.", "demo"),
    ("Import a drawing (SVG, DXF, DWG, PDF, Illustrator, EPS, LightBurn)", "File › Open / Import, or drop the file on the window. AutoCAD .dwg files are read like DXF. LightBurn projects (.lbrn2) bring their shapes on one layer per LightBurn cut layer; EPS files their vector artwork, like PDF. Text in an SVG or DXF comes in as editable text in the standard font at its size, place and angle (double-click it to change the wording or font). PDF and .ai bring in the vector artwork only: convert their text to outlines first. Pictures are skipped.", "import"),
    ("Turn a picture into vectors", "Vectors › Trace Image… (PNG or JPEG, best with a clean black-and-white picture).", "trace"),
    ("Bring in G-code made elsewhere", "File › Import G-code…: it is split per bit, you confirm the bits, then it simulates and re-posts for your machine.", "importgcode"),
    // drawing
    ("Type an exact size or position", "Select the shape and type in X / Y / W / H under Properties. Click a shape tool once on the board (without dragging) to type its size straight away.", ""),
    ("Add text", "Text tool (T), click on the board. Double-click text later to change it. Angle turns it (90 = reading upwards). For text on a curve, select the curve first.", "tool:text"),
    ("Script font cuts through the joins", "In the text dialog tick Weld letters: overlapping letters become one outline (the holes in o, a, e stay), so V-carve and pockets don't cut across the joins.", "tool:text"),
    ("Use my own font", "Text tool, then the Add font… button next to Font and pick a .ttf or .otf. The font is kept on this device; a project only remembers its name, so add the font again on another computer.", "tool:text"),
    ("Round the corners (fillet) / dogbone corners", "Select the shapes, then Vectors › Corners: Dogbone / Fillet…", "dogbone"),
    ("Offset a shape in or out", "Vectors › Offset…", "offset"),
    ("Weld, subtract or intersect shapes", "Select two or more closed shapes, then Vectors › Weld / Subtract / Intersect.", ""),
    ("Cut a vector where it crosses another (trim)", "Right-click the piece you want gone › Trim Here. Then Vectors › Join Open Vectors builds new outlines from what is left.", ""),
    ("Make two lines meet at a corner (extend)", "Select the open vectors, then Vectors › Extend Ends to Next Vector: each end runs straight on until it meets another vector. Then Trim Here removes the overshoot and Join closes the outline.", "extend"),
    ("Fix an imported drawing that won't pocket", "Vectors › Check Vectors finds open, duplicate and tiny vectors and selects them. Join or Close the open ones; delete duplicates.", "validate"),
    ("Too many nodes / slow shapes", "Select them, then Vectors › Simplify (fewer nodes).", "simplify:0.005"),
    ("Copies in a grid, a circle, or along a line", "Vectors › Array Copy…: Grid, Circular, or Along a vector (select the object and the path together).", "array"),
    ("A symmetric part (mirror)", "Draw one half, select it, then Vectors › Mirror Copy to the Right (or Upwards). Weld the two halves into one outline.", "mirror:right"),
    ("Line things up", "Vectors › Align & Distribute…, or drag guide lines out of the rulers: drawing snaps to them.", "align"),
    ("Guide lines", "Drag down out of the top ruler for a horizontal guide, right out of the left ruler for a vertical one. Drag its ruler marker to move it, drop it on the ruler to remove it. View › Clear Guides removes all.", ""),
    ("Measure something", "Measure tool (M): click two points. Distance, ΔX, ΔY and angle show under Properties.", "tool:measure"),
    ("Fit many parts on the board", "Select the parts, then Vectors › Nest on Board…", "nest"),
    ("Layers", "Layers tab: put shapes on layers, hide or lock them, and add a toolpath for a whole layer so new shapes on it are cut too.", ""),
    // toolpaths
    ("Cut a shape out", "Select it, Toolpaths tab › Profile. Outside for parts, Inside for holes. Add tabs so the part stays put.", "kind:0"),
    ("Clean edge on a cut-out", "In the Profile dialog: Last pass allowance (e.g. 0.02 in) leaves a little for one final full-depth pass. Lead in / out moves the plunge mark into the waste.", ""),
    ("Choose where a cut starts", "Node tool (N): right-click a node of the closed shape › Make Start Point. Then in the Profile dialog tick Start point › At the vector's first node. Good for putting the plunge in a corner or a waste area.", "tool:nodes"),
    ("Hollow out an area", "Select closed shapes, Toolpaths tab › Pocket. Rows (raster) clears big open pockets faster; a Clearing bit lets a large end mill do most of it and the small bit only the corners.", "kind:1"),
    ("Carve letters with a V-bit", "Select the text, Toolpaths tab › V-Carve. Wide letters: set a flat depth, or use a wider V-bit.", "kind:3"),
    ("Engrave a filled area (plaques, labels)", "Select the shapes, Toolpaths tab › Pocket, pick a V-bit or engraving bit and a shallow depth (about 0.02 in). The fill lines are spaced by the width the bit cuts at that depth; Rows gives a hatch, Rings follow the outline.", "kind:1"),
    ("Drill holes", "Select circles (or any shapes: their centres), Toolpaths tab › Drill. Bore cuts larger holes with a spiral.", "kind:2"),
    ("Mill a thread", "Toolpaths tab › Bore on a circle the size of the thread's outer diameter, with a thread mill as the bit and the Thread pitch set (distance per turn). It climbs from the bottom up, one turn per pitch.", "kind:4"),
    ("Carve a photo", "Toolpaths tab › Photo: a V-bit cuts lines whose depth follows the picture's darkness.", "kind:11"),
    ("A textured background", "Toolpaths tab › Texture.", "kind:10"),
    ("Cut a free community project", "File › Free Community Projects… opens gcarve.app/projects: pick a project and press Open in GCarve. GCarve asks for your machine, your bits and your board's thickness, then shows the cut in Simulate; export the G-code as usual. Desktop app: download the project (.tlp) on its page, open it, then File › Make It on My Machine….", "community"),
    ("Share my project with everyone", "File › Share to Community…: a title, a few words, tags and a license. A picture of the simulated cut goes with it; add photos of the real thing too. We check it, then it appears on gcarve.app/projects for others to open and cut on their own machine.", "communityshare"),
    ("See toolpaths on the drawing", "Bottom bar, by Width: Paths (on / off, ⇧⌘T; off = nothing drawn or worked out while you edit), then Picked (only the toolpath you click, or the one open in its dialog) or All. Simulate (3) always cuts them all, with their times. Remembered on this computer.", "paths:picked"),
    ("Undo a point while drawing a polyline", "Press Delete (Backspace) while drawing: the last point clicked goes. Esc cancels the whole line; double-click or click the start to finish.", ""),
    ("Change the cutting order", "Toolpaths tab: right-click a toolpath › Cut Earlier / Cut Later. The list order is the order each bit cuts in. Cut parts out last.", ""),
    ("Copy a toolpath", "Toolpaths tab: right-click it › Duplicate, then double-click the copy to change it.", ""),
    ("Reuse toolpath settings on another job", "Toolpaths menu › Templates…: save the toolpaths of this job and apply them to another drawing by layer name.", "templates"),
    ("Feeds and speeds", "Each toolpath dialog shows the feed it will use and a meter. Bits without your own numbers get suggestions from chip load, scaled to your machine. Your own feeds: Toolpaths tab › Bits.", "tools"),
    ("Keyboard shortcuts", "Tools: V select, N nodes, R rectangle, E ellipse, G polygon, S star, P polyline, B curve, A arc, M measure, D dimension, T text. Views: 1 Design, 2 Toolpaths, 3 Simulate. File: ⌘N new, ⌘O open, ⌘S save, ⇧⌘S save as, ⌘W close tab, ⌘E export G-code. Edit: ⌘Z undo, ⇧⌘Z redo, ⌘X / ⌘C / ⌘V cut copy paste, ⌘D duplicate with its toolpaths, ⌘G group, ⇧⌘G ungroup, ⌘A select all, Delete. View: ⌘+ / ⌘− zoom, ⌘0 fit, ⌘1 the board, ⌘2 actual size, ⌘3 the 3D view, ⌘R simulate, ⇧⌘R watch the cut, ⇧⌘T show toolpath lines, ⌘K find any command. Pan: hold Space and drag, or drag with the middle or right button. (⌘ is Ctrl on Windows and Linux.)", ""),
    ("Put tabs where I want them", "Profile toolpath › Add tabs, then tick Click the outline to place tabs and click the part's outline: an orange square marks each tab. Click a square again to take it away. Parts with no squares keep the evenly spaced tabs.", ""),
    ("Mark clamps and screws so I don't hit them", "Draw the clamps or screw heads as closed shapes on their own layer, then in the Layers tab click + on that layer › Keep out. They show in red, and export lists every toolpath that comes down near one.", ""),
    ("Warn me when a cut is deeper than my bit", "Toolpaths tab › Bits: fill in Cutting length (bits from the catalog have it). Export then lists any toolpath that cuts deeper than the flutes reach. A pass deeper than the bit is wide also lowers the feed the meter calls good (80% at 2× the diameter, 60% at 3×).", "tools"),
    ("Find a bit from a maker's catalog", "Toolpaths tab › Bits › Browse bit catalog… (Whiteside, Amana, SpeTool and many more, with their feeds where published).", "tools"),
    // output
    ("Carve a 3D model (STL, 3MF)", "Toolpaths › 3D Models › Import, or drop the .stl / .3mf file on the window (3MF from Bambu Studio, PrusaSlicer, Cura or Fusion works). It is fitted into the board; then make a 3D toolpath for it.", "importstl"),
    ("Watch the cut before exporting", "Press 3 (Simulate). Carve shows the result, Watch plays it at speed, 3D lets you orbit.", "mode:Simulate"),
    ("Turn the 3D view with my mouse like Fusion, Blender or VCarve", "In Simulate › 3D open Mouse (top right) and pick the style; Reverse zoom direction is there too. View has Top, Front, Left, Right and Back.", "view3d"),
    ("Use a 3Dconnexion SpaceMouse", "View › Connect a SpaceMouse… and pick it in the browser's list (Chrome or Edge on a computer). It reconnects by itself next time. It orbits the 3D view and pans / zooms the drawing. If the menu says nothing was received, quit the 3Dconnexion driver and reload.", "spacemouse"),
    ("Export G-code", "Export G-code (top right). One file per bit; the safety check lists anything risky first. New job: run File › Export Air-Cut Test once.", "export"),
    ("My board is longer than my machine", "Export G-code › tick Tile: the job is split into sections you cut one after the other, sliding the board between them.", "export"),
    ("Cut both sides of a board", "Vectors › New Back Side: a second sheet with your vectors mirrored for the flip. Drill two dowel holes through on side 1 to line the board up again.", ""),
    ("Several boards in one project", "The + next to the sheet tabs under the canvas adds a sheet; each exports its own files.", "sheet:add"),
    ("A printout for the machine", "File › Setup Sheet…: bits, order, zero point and a picture of the result.", "setupsheet"),
    ("Place parts around knots or damage using a photo of the board", "View › Photo of the Board…: take a picture of the board on the machine from above (any angle), then click its four corners: bottom-left (your X/Y zero side), bottom-right, top-right, top-left. The photo is straightened onto the board under your drawing, so you can move parts clear of knots, cracks or earlier cuts. View › Hide / Remove the Board Photo. It is kept for this session, not saved in the project.", "boardphoto"),
    ("Put toolpaths in folders", "In the Toolpaths tab set Group to Folders. Drag a toolpath onto a folder heading (or onto a toolpath in it), or ⌘ / Shift-click several and press Group…. Click a heading to open or shut it; its eye hides or shows them all, its export button exports just that folder; right-click it to rename.", ""),
    ("Cut a layer, a sheet or everything with one toolpath", "In the toolpath's Applies to pick Selected shapes, A layer, The selection's group or All vectors, every layer; with several sheets, Sheets picks All sheets or one sheet only. New vectors on that layer (or anywhere, for All vectors) join the toolpath by themselves.", ""),
    ("See all sheets at once / move vectors to another sheet", "Click All sheets next to the sheet tabs (or View › All Sheets Side by Side). Every sheet shows in a grid; the one you're working on has a blue bar. Click another sheet to work on it, or drag selected vectors onto another sheet's board to move them there: their toolpaths go with them.", "allsheets"),
    ("Open a VCarve project (.crv)", "File › Open the .crv file, or drop it on the window. The vectors come in on their layers, each VCarve sheet becomes a GCarve sheet with its board size and thickness. Profile, pocket, drill, V-carve and chamfer toolpaths come with their bits, depths and tabs (merged toolpaths as their parts); check them before cutting. Text comes in as letter outlines grouped per text (text on a curve can sit up to about 1/32 in from VCarve's place); array copies aren't read yet. Save as a GCarve project (.tlp); the .crv file is never changed.", "open"),
    ("Send vectors to other software", "File › Export Vectors › SVG, DXF or DWG (the selection, or everything on the sheet).", "vexport:svg"),
    ("Save my work", "File › Save. With a free account projects are kept online in folders and open on any computer; without one, File › Download Project keeps a .tlp file.", "save"),
    ("Find any command", "Press ⌘K / Ctrl+K and type what you want.", "palette"),
];

pub use gcarve_ui::guide::Guide;

#[cfg(test)]
mod tests {
    #[test]
    fn every_topic_is_filled_in_and_unique() {
        gcarve_ui::guide::check_topics(super::TOPICS);
    }
}
