# Changelog

What changed in each GCarve desktop release, newest first. Downloads are on the [releases page](https://github.com/fillinthe/gcarve/releases).

## 0.1.41 (2026-11-27)

Fixes
- A toolpath warning that every shape earns (for example "open shapes can't be v-carved") is listed once, not once per shape: 40 open lines no longer fill the side panel.
- Uploaded posts are read by what is in the file, not by its name: a Vectric post saved under another ending loads.
- Tool Library › Import reads a Fusion 360 .tools file directly (no unzipping first), and a file that is not a bit library says so in plain words.
- A 3MF from a slicer that holds only the printer's G-code says so, and asks for the model itself, instead of "no model in the 3MF file".

## 0.1.40 (2026-11-24)

New
- A G-code style for the Onefinity Redline (RealTime CNC) controller, in inch and mm: the units and plane are stated at the top (G17 G20 G90, or G21), the spindle starts with its speed on one line (M3 S…) before the first plunge, and the file ends with M5. Choosing an Onefinity with the Redline controller in Job Setup selects it.
- G-code styles for CNC-STEP KinetiC-NC, Felder Hammer (Eding CNC), Snapmaker, Two Trees ED1 and Techno LC Plus.
- 46 more machines in the machine list choose their G-code style by themselves.

Fixes
- A Vectric post (.pp) without "end" lines, which is how Vectric writes its own posts, was read wrong when uploaded: its move lines went into the file header once and the cuts used GCarve's defaults. **If you uploaded such a post, upload it again.**

These new styles were built from the makers' published posts and have not been run on those machines by us yet: run File › Export Air-Cut Test first and tell us how it goes.

## 0.1.39 (2026-11-23)

Fixes
- In a millimeter project the feed and plunge of a bit (Tool Library, and the feeds saved per material) are now typed and shown in mm/min. The fields were labeled mm/min but took the number as in/min, so 28 came out as F711 in the G-code. **If you set up bits in a mm project, open the Tool Library and check their feeds:** a number you typed as mm/min was saved 25.4 times too fast.
- The feed line under the bit in every toolpath dialog and the chip load on the feeds meter follow the project unit too (mm/min, mm/tooth).

Inch projects are unchanged.

## 0.1.38 (2026-11-23)

New
- Waterline 3D finishing: a 3D toolpath can go round the model in laps at constant height and cut only the steep sides that rows leave stepped (3D toolpath › Steep sides).
- STEP files (.step / .stp) from Fusion and other CAD programs open as 3D models, like STL. A part modeled standing up is laid flat.
- Suggest Toolpaths (Toolpaths menu): GCarve reads the drawing and proposes the cuts (holes, text, lines, inside shapes, cut-outs last with tabs). Untick what you don't want and add the rest.
- Turning on a rotary axis: a Pocket on a rotary job can turn the blank to a side profile you draw, with a preview of the part. Job Setup takes the lengths the chuck and the tailstock hold; toolpaths stay clear of them.
- Plasma cutting, first version: a Plasma torch tool (kerf, pierce height, pierce delay, cut height, cut speed), single-pass Profile cuts that pierce in the waste, and G-code styles for Langmuir FireControl, LinuxCNC QtPlasmaC, Mach3 / Mach4 and grblHAL. The torch is switched off before every move between cuts and at the end of every file. These files have not been run on a table by us yet: run Export Air-Cut Test first (it never fires the torch) and tell us how it goes.
- Toolpath previews are worked out in the background while you draw, so editing no longer waits for them.
- AI assistants (Help › AI Assistants) can ask for suggested toolpaths.

Fixes
- Tiled exports (long boards cut in sections) wrote curves as a few straight lines. Curves now stay true. If you tiled a job with circles or arcs in 0.1.37 or earlier, export it again.
- Binary STL files with padding or a wrong triangle count in their header open.

All of it is in the 12 languages.

## 0.1.37 (2026-11-22)

Privacy
- A short card at the first start says what GCarve sends to help improve it (which features are used, and problem reports without file names) and offers one switch: Preferences › Help improve GCarve. Switch it off and nothing is sent.
- Preferences has Delete my data (removes your usage and problem records from our server) and a link to the privacy policy.

Fixes
- Dimension tool: clicking an existing dimension with it no longer stops the app.

## 0.1.36 (2026-11-22)

3D carving
- Tapered ball nose bits: give the tip radius, the taper per side and the shank. 3D toolpaths, their stepover and the simulation use the real shape, so the cone never cuts into steep walls next to fine detail.
- Rest machining: a second 3D toolpath with a smaller bit can cut only where an earlier one couldn't reach (tick it in the 3D toolpath, under Boundary).
- Several 3D models combine into one carving: add, subtract, keep the higher or the lower, with lift, height slice and smoothing. One 3D toolpath can cut them all combined, and the outline of the result can be made into vectors.
- OBJ models can be imported, as well as STL and 3MF.

Toolpaths
- Drag knife: cuts in passes when the pass depth is shallower than the cut depth. The blade offset now starts at 0.25 mm, with a warning when it looks like the wrong units.
- A lead in / out keeps the start point picked with Make Start Point.
- Toolpaths that take a while are worked out one at a time behind a card that names each one, with a progress bar.

Working in the app
- In Polish, Dutch, Turkish and Korean too (12 languages), with every message translated.
- Tab into a number field selects its value, so typing replaces it.
- The toolpath panel wraps long hints instead of scrolling sideways.
- Help › User Manual opens the online manual (gcarve.app/docs/) in your language.
- Shorter dialogs: rows appear only when they apply.
- Files saved by Windows editors with a byte-order mark (bit libraries, posts, templates, drawings) open.
- macOS: the close button hides GCarve and keeps your tabs; Quit ends it.
- Preferences › Send problem reports (on by default): when something goes wrong, what the app said is sent to us without file names, and with your account if you are signed in, so we can help. Switch it off there.
- The Donate button can be hidden (right-click it, or Preferences).

## 0.1.35 (2026-11-14)

Post processors
- Upload posts… with many files at once adds them all to the list and keeps the board's post as it was, with one message saying how many came in (and how many couldn't be read). One file is still added and picked for the board.

GCarve Control
- A program line that writes the controller's memory (G10 L2 / L20 work zero, G28.1 / G30.1, `$` settings) is sent on its own, and nothing follows until the controller answers it: grbl stops listening while it writes, so lines sent then could arrive damaged.
- Checked end to end against a machine that counts down from home like stock grbl (FoxAlien Masuter 3S): an inch job homes, passes the start check and runs to the end.

## 0.1.34 (2026-11-13)

Variables
- Every box that takes a size also takes a variable, and now offers them as you type: start a name and a list of matching variables shows under the box with their values. Click one, or use ↓ / ↑ and Tab, and keep typing (`2*thickness + 1/8in`). Works in every dialog, toolpath depths, Job Setup, the size popups, the Properties panel, dimension lengths and Variables & Constraints.

Post processors
- Search by words in any order: maker, machine, controller, mm or inch, or the file ending (`shark inch`, `fanuc atc`, `.sbp`). Best matches first.
- Upload posts… takes many `.pp` / `.json` files at once, so a whole VCarve PostP folder comes in one go.
- Corrected against the makers' own posts: Roland writes machine steps (and a Modela post), MultiCam writes Z positive down with per-second feeds, WinPC-NC machines get their own post with per-second feeds, Next Wave waits for the spindle. New KOSY post. If you cut with one of these, do an air cut first.

Drawing
- Menus are no longer cut short on Windows, Linux and the web (File menu in 0.1.33).
- Refusals (locked shapes, dimension clicks on empty space) show a red banner.
- Locked shapes also refuse the Properties panel, Align, Distribute, Rotate, Nest and Distort.
- Nodes tool: drag a box to pick several nodes.
- Box Maker draws the box as you change the numbers. The Cabinet maker takes inside or outside sizes.
- Toolpaths and File menus grouped like the Vectors menu.

## 0.1.33 (2026-11-12)

Dimensions, variables and locks
- The Dimension tool ties to shapes: click a corner or a straight side (it highlights), then another. Two sides measure square across. A click on empty space is refused (Measure is the tool for loose points).
- The length box opens as soon as the dimension is placed. Type a size, a variable name, or `width = 12` to make a variable and tie the dimension to it. Change the variable later and every dimension tied to it follows.
- Setting a length between sides moves the whole side, so shapes keep their angles. "Stays still" picks which end stays.
- Lock a corner or a side in place: right-click it. A padlock shows; it can't be dragged, the shape can't be moved, and dimensions move their other end. A dimension with both ends locked says so instead of moving anything.
- Fixed: with Snap off, the Dimension tool took clicks on empty space as if they were on a shape.

Drawing
- Nodes tool: Shift, Ctrl or Cmd-click picks several nodes; they move and are deleted together.
- Box Maker: sizes as outside or as the space inside the box; the other measure is shown.
- Array Copy, circular: the selection is placed on a radius for you, with buttons for the board center and the selection center, and an angle to fill. In every array layout, grouped vectors are copied as groups.
- Vectors menu: the everyday edits on top, the rest grouped under Create, Copy and Arrange, Clean Up, Distort and Sheets. Menus scroll when the window is too short for them, and greyed items say why.
- American spelling throughout the app.

Machines
- Post processors: Laguna by control (hand-held controller, iQ Pro / SmartShop M, Syntec, B&R, FANUC) and Techno Titan, written the way the maker's own post writes them. Laguna and Titan machines in the machine list pick them. The Techno Titan is no longer listed under WinCNC. New posts have not been run on those machines: do an air cut first.
- GCarve Control: an arc a GRBL controller would refuse is found when the file is loaded, with its line number.

## 0.1.32 (2026-11-12)

GCarve Control (GRBL, grblHAL, FluidNC machines)
- Positions from the controller are read in millimeters or inches, whichever it reports. With a controller reporting millimeters (the GRBL default) the position display and Follow the surface used wrong numbers. If you used Follow the surface in 0.1.30 or 0.1.31, probe again with this version.
- Machines without homing: Go to work zero and Resume lift in work coordinates and never move down; Park needs homing; the travel check is a warning instead of a block.
- Park picks the right corner on machines that count down from home.
- Touch plate and surface probing no longer leave the controller in relative mode after a failed probe, and take the height from where the probe touched.
- Stop holds first and resets once the machine stands still, so the position is kept.
- Start states inches and absolute mode first, so the job runs the way the preview shows it.
- Before Start, the file is checked for lines over 79 characters and for words GRBL refuses (G41, G42, G64); a warning for M7 and for files that name no unit.
- Resume from a line works inside a run of arcs; programs in relative mode are refused.
- Unlock after a limit alarm resets the controller. Short jobs no longer read "finished" while the machine is still starting. USB on Windows raises DTR for boards that need it.

Post processors
- ShopBot: speeds are written per second, as ShopBot expects. Earlier files asked for the feed per minute as if it were per second: re-export ShopBot files made with earlier versions.
- New: Komo (Fanuc with tool changer), FoxAlien, Two Trees, TinyG / g2core, Yeti SmartBench, Stepcraft, Probotix, MachMotion, myCNC, RosettaCNC, C.R. Onsrud (Fanuc), Mitsubishi, GSK / KND / HNC, DDCS, FlashCut, Estlcam controller; experimental: isel Remote and Heidenhain plain language. New posts are written from the makers' documentation: run an air cut first.
- Uploaded Vectric .pp posts: rate multipliers (per-second feeds) are honored; posts that scale coordinates into machine steps are refused with a message.

## 0.1.31 (2026-11-12)

- GCarve Control: Start Program no longer refuses a job on machines whose machine coordinates count down from home (stock GRBL after homing, for example FoxAlien). The travel check now takes both ways of counting.
- GCarve Control: Park goes to the back-left corner on those machines too.

## 0.1.30 (2026-11-12)

- Automatic dogbones: Profile and Pocket toolpaths have an "Inside corners" choice (As drawn / Dogbone / T-bone). Every inside corner is relieved for that toolpath's bit; the drawing is not changed.
- Cabinet maker (File › New Cabinet…): type a base or wall cabinet's outside size, shelves and how many. Every part is nested on sheets with its toolpaths: mirrored sides with dados, 5 mm shelf-pin holes and the toe-kick notch, bottom, top, shelves, toe kick, and the backs on a sheet of their own thickness.
- Joinery (Vectors › Joinery: Slots and Fit Test…): a fit test comb of slots from tight to loose, each marked with its fit, and slots sized to your board plus the fit you found.
- Cut report and part labels (File menu): the parts numbered per sheet, their sizes, how much of the board is used, the biggest offcut, and a label for every part.
- GCarve Control: Follow the surface. Probe a grid over the job's cuts and the program follows the board, so engraving and V-carving stay an even depth on a board that isn't flat.
- Arc tool: 3 points, center, or 2 points (a half circle).
- Tabs: with "One every", a least and a most per part; "First tab at the start point".
- Corners tool: a round too big for the corner takes the largest that fits.
- Windows and Linux: Preferences names Shift and Ctrl where it showed Mac key symbols.

## 0.1.29 (2026-11-12)

- Corners tool (C) in the tool rail: pick Dogbone, T-bone or Round and a radius, hover a corner to see the result in red, click to make it. Click a dogbone or round again to put the sharp corner back. A T-bone runs along the side you click nearer.
- Tabs: in a Profile toolpath, "Edit tabs on the drawing…" now sits next to "Add tabs". The evenly spaced tabs show as hollow squares; click an outline to add a tab, click a tab to delete it, drag one to move it.
- The toolpath form docks at the right side of the window instead of floating over the drawing.
- A tool options bar above the drawing shows the active tool's settings (polygon sides, star points, corner style and radius) and a line on how to use it.
- The tool rail can show two columns of tools (the » button at its top).
- Properties: X and Y are read and typed at the "Grows from" point, like VCarve's anchor.
- Fixed: the Design / Simulate buttons did nothing when the app was set to a language other than English.
- Preferences: background color for the drawing and the 3D view.
- Bit catalog: 42 bits from CIC Workshop (Cutting It Close) with their starting feeds.
- Pictures: 16-bit grey TIFF scans open.
- Linux: the menu entry made by install.sh shows the GCarve icon.

## 0.1.28 (2026-11-06)

- Fixed: some lettering made V-Carve toolpaths run out of memory and the app stopped (seen in the online app). Those drawings now carve normally.
- Post-processors: Syntec / Weihong, RichAuto, Next Wave and ShopBot end each file at the job's lift height instead of a fixed height (a fixed 1 in could lower a stopped bit into a thick board with Z zero on the table). A post can set its own with homeZ.

## 0.1.27 (2026-11-04)

- Fixed: on the desktop, Board / Job Setup squeezed its right-hand column (where you zero the machine, rotary, cut order, park position) to a sliver. Dialogs size to their content again; phones keep their sideways scroll.
- Job Setup › "Into the spoilboard": how far below the board a cut may go before Export warns about it (was always 1/16 in).
- Post-processors: {XH} {YH} now follow Job Setup's "Park the bit at X / Y" (Vectric .pp posts that use the home position get it).

## 0.1.26 (2026-11-04)

- Post-processors: the lift height at the end of a file ({SAFEZ}) now follows Job Setup's "Lift for moves" instead of a fixed 0.8 in. A post can also set its own lift (`safeZ`) and home position (`homeX`, `homeY`, `homeZ`).
- New post placeholders: {XH} {YH} {ZH} (home) and {XLENGTH} {YLENGTH} {ZLENGTH} (board size). Vectric .pp posts that use these now get real numbers.
- Help › How Do I…? has a new topic: change a post-processor for your machine, or write your own.
- Phone-size screens: check boxes, the community dialogs and the How Do I…? list wrap their text instead of running off the edge.

## 0.1.25 (2026-11-03)

- Mac: the menus (File, Edit, Vectors, Variables, Toolpaths, View, Help) are now in the menu bar at the top of the screen, with the usual GCarve menu (About, Settings, Hide, Quit). The command search moved up next to Export G-code. Keyboard shortcuts work as before.
- Windows, Linux and the online app keep their menus in the window.

## 0.1.24 (2026-11-03)

- Fixed: on macOS 26, closing the window with the red close button could make GCarve "quit unexpectedly".
- Phones and narrow windows: dialogs now fit the screen (wider ones scroll sideways), and OK / Cancel always stay in view.
- A new Profile toolpath now starts with tabs on, so a first cut-out doesn't throw its part.
- Small parts get as many tabs as fit instead of none; tiny holes are cut without tabs and without a warning.
- Whole jobs are now tested automatically before each release, the way a person does them: every New… wizard, drawing and cutting shapes and text, opening files from other programs, Undo / Redo, and saving and opening a project again.

## 0.1.23 (2026-11-03)

- Every menu command, field, checkbox, drop-down and button is now tested automatically before each release, the way a person uses the app. These fixes came out of it:
- Fixed: on a Mac with no SpaceMouse plugged in, View › Connect a SpaceMouse… closed the app.
- Fixed: the command search (⌘K) could close the app while VoiceOver or another accessibility tool was on.
- Fixed: on older graphics cards, a tall or long, narrow board could close the app when showing the toolpath preview or the simulation.
- Fixed: the shop name (Preferences) and the bit name pattern (Tool Library) didn't take typing when settings couldn't be saved, as in a private browser window.
- Buttons, checkboxes, switches and tabs now have names for screen readers.

## 0.1.22 (2026-11-03)

- **Fixed: the X / Y / W / H fields in Properties took no typing.** They looked editable, but anything typed was replaced at once. Type a position or size and press Enter: the selection moves or resizes again.
- Fixed a crash in adaptive clearing on a large pocket with a big bit.
- G-code saved as a .txt file opens like a .tap or .nc file; TIFF pictures open for tracing and for drawing over.
- AI assistants (Help › AI Assistants…) can now put vectors on layers, hide layers and mark a layer as a keep-out area for clamps.
- Fewer ways for unusual files and settings to stop the app (empty bit list, odd sizes in LightBurn files).

## 0.1.21 (2026-11-02)

- **Let an AI assistant work in GCarve** (Help › AI Assistants…): any assistant that speaks MCP (ChatGPT desktop, Cursor, VS Code / Copilot, LM Studio and others) can draw, add text, set the board and machine, add and edit toolpaths, check times and feeds, preview the carved result and export G-code in the open drawing, while you watch. Off until you switch it on; it only answers on this computer, and every change is one undo step.
- **Picture to Draw Over** (View menu): place a photo or sketch under the drawing to trace over, saved with the project. Move it by dragging while Picture Settings is open; size and fade it there.
- Deleting a sheet keeps toolpaths that were set to a later sheet pointing at the right one.
- Fixes: toolpath names with several words read back right in the cut-order warning; feed presets saved in English apply to boards named in another language; more messages translated.

## 0.1.20 (2026-10-29)

- **Right feeds in every language**: the default board material was saved under its translated name ("Sperrholz", "合板"…) and got hardwood feeds instead of plywood feeds. Material names in all seven languages are now recognised, and the material list is translated.
- **Warnings and safety checks in your language**: toolpath warnings and the export safety check now show in Spanish, German, French, Italian, Portuguese, Japanese and Chinese.
- **Help › How Do I…? fully translated**, with menu names that match the real menus.
- **Sheet names** ("Sheet 1", "Back of …") and the starter projects follow your language.

## 0.1.19 (2026-10-28)

- **More of GCarve in your language**: messages in the status bar, the results of the wizards and makers, and error messages now show in Spanish, German, French, Italian, Portuguese, Japanese and Chinese too (Preferences › Language).
- **Komo Fusion XL 624** in the machine list: the extended 6 × 24 ft table.

## 0.1.18 (2026-10-28)

- **Stencil Bridges** (Vectors › Stencil Bridges…): letters cut right through the board keep their middles. The inside of an O, A, R or B gets a bridge to the rest of the board, up and down or across, as wide as you choose. Use it for stencils and cut-out signs.
- **A new install window on the Mac**: the disk image opens on a branded window that shows you where to drag GCarve.
- **Japanese and Chinese in the online app**: the online app downloads a font for them, and the translated pages of gcarve.app open the app in their language.
- **Sturdier file import**: damaged or oversized ZIP, PDF, Carbide Create, DWG and 3MF files are refused cleanly instead of using up memory.

## 0.1.17 (2026-10-27)

- **GCarve speaks seven more languages**: Spanish, German, French, Italian, Brazilian Portuguese, Japanese and Simplified Chinese. Preferences › Language switches the menus, tools, dialogs and tips at once (Japanese and Chinese use your system's fonts; the online app downloads one). Messages that report numbers are still in English for now.
- **Spoilboard Maker** (File › New Spoilboard…): a board the size of your machine's table with a centred grid of holes for threaded inserts, an optional surfacing pass and engraved grid lines.
- **Batch of name signs** (File › New Sign…, "One sign per line"): every name gets its own sign with the letters sized to fit, laid out over as many sheets as it takes.
- **Clock Maker** (File › New Clock…): numerals or Roman numerals round a dial, hour and minute marks, the movement hole and the cut-out.
- **Tray Maker** (File › New Tray or Cutting Board…) with bowl bits (a ball nose with a corner radius), hand holds and a juice groove; **Cabinet Door Maker** (File › New Cabinet Doors…) from a cut list, nested over sheets.
- **Inlay Wizard** (Toolpaths › Inlay Wizard…): V-bit or flat inlay with the plug mirrored on its own sheet.
- **Carbide Create .c2d projects open** (v6 and v7), with their vectors, text and toolpaths.
- **Halftone from Picture**, **Living Hinge** and **Onion skin** on profiles (parts held by a thin skin, cut free in one last lap).
- **Fonts**: a zipped font download (Google Fonts) can be added as it is; web fonts get a clear message.
- **Fixes**: the signed-out web app no longer logs an empty cloud error; many smaller fixes.

## 0.1.16 (2026-10-26)

- **The Mac app is signed and notarized by Apple**: it opens with a double-click, no "unidentified developer" warning.
- **New Sign** (File › New Sign…): type the words, the board size, a shape and a font; a ready job opens with V-carved letters sized to fit, a V-groove border and a cut-out with tabs.
- **Box Maker** (Vectors › Box Maker…): a finger-jointed box (with or without a lid) as flat panels, with fit play and dogbone corners for your bit.
- **Adaptive clearing** for pockets: the bit's sideways load stays steady everywhere (helix entry, laps along the cut edge), with an optional chip-thinning feed boost.
- **Relief from a picture**: a photo or logo's brightness becomes carved height, or a lithophane.
- **Surface the Board**, **countersinks** on drills with a V-bit, and **job cost** from a board price and your shop rate (setup sheet and toolpath list).
- **Safety**: warnings when a cut is deeper than the bit sticks out of the holder, and when a wide holder would reach a clamp (keep-out layers now have their own clamp height).
- **Getting started**: a five-step checklist for new users and ready starter projects on the start card.
- **GCarve Control**: tool-change pauses, outline the job, rapid and spindle overrides, spoilboard surfacing.
- **More**: one-click problem reports after an error or a crash, zip archives are refused with a clear message (unpack them first), and many smaller fixes.

## 0.1.15 (2026-10-23)

- **Rotary jobs**: Job Setup › Rotary (A axis, along X or Y) with the blank diameter; the board is the unrolled surface and every G-code post writes the turn as A. Rounding toolpath for a square blank, Vectors › Rotary Layout for wrapped spirals and flutes, and the 3D view shows the carved cylinder.
- **New toolpath options**: Moulding (sweep a profile along rails, with an optional clearing bit), keyhole slots, drag knife, sketch carving (from a picture or a 3D model), follow a 3D model, merged toolpaths (one tour per bit), cut in drawing order, cutter compensation (G41/G42 for posts that know it), park position at the end, and a ghost-run twin of every file (spindle off, above the board).
- **Two-sided jobs**: the back sheet shows the front mirrored, its simulation starts with the front's cuts, and nesting moves both sides together.
- **Cabinet work**: import a CSV cut list (labelled parts nested onto sheets), shelf-pin hole rows, Plate Production from a list.
- **Drawing**: Freehand tool, Distort (bend along a curve, fit between two curves, fit into four corners), Vector Selector, Fit Arcs and Curves, Vector Texture, angular / horizontal / vertical dimensions, arched text, line spacing, kerning by hand, a "Grows from" anchor for sizes.
- **Nesting**: from any corner, mirrored parts, inside an offcut, fill the rest with the smallest part.
- **Pictures**: trace one colour or one area, brightness / contrast / gamma for Photo V-Carve, WebP pictures.
- **Bits**: folders in the Tool Library, bit names from a pattern, custom form cutters drawn as a vector.
- **More**: job templates (.tlpt), PDF vector export, Save Preview Image, lithophane view, wood colours by board material, shop name and logo on the setup sheet, Gerber aperture macros, LightBurn pictures, File › Send to GBurn (laser).

## 0.1.14 (2026-10-21)

- **Set up your own mouse** (Edit › Preferences… › Mouse): start from the Fusion 360, Blender, VCarve / Aspire or SketchUp style, then choose what a left, middle or right drag does on its own, with Shift and with Ctrl / ⌘ (orbit, pan or zoom), what a trackpad swipe does, the zoom and orbit speeds and the wheel direction. The 3D view uses all of it; the drawing's middle and right buttons and wheel follow it too.
- The 3D view's Mouse menu has a shortcut to these settings.

## 0.1.13 (2026-10-21)

- **Hidden layers hide their toolpaths**: when you hide a layer, toolpaths that cut only vectors on that layer (or are set to a hidden layer) disappear from the drawing too. Export still writes every toolpath.
- **HPGL plot files**: open or import .plt / .hpgl files (CorelDRAW, sign cutters, laser shops); each pen comes in as its own layer.
- **One account for every G Workshop app**: signing in, your dashboard and settings now live at gworkshop.app, with one free account for GCarve, GBurn, GSketch and Phosphor. Community projects moved to gworkshop.app/projects (old links still work).

## 0.1.12 (2026-10-20)

- **Paths, Picked, All on the bottom bar**: choose which toolpaths the drawing shows while you edit. Paths off draws and works out nothing; Picked shows only the toolpath you click; All shows every one. Simulate still cuts them all.
- **3D view**: the carved surface now reaches the edges, so cut-outs near the board edge no longer show a false lower ledge.
- **Polyline / curve drawing**: Delete or Backspace takes back the last point you clicked.

## 0.1.11 (2026-10-20)

- **Share to Community, better pictures**: pick which picture is the cover (the simulated cut or a photo of the real thing); a photo you add becomes the cover unless you choose otherwise. Up to 4 pictures.
- **Your name, or not**: show your name on a shared project, with a page listing all your projects on gcarve.app/projects, or stay "A GCarve maker". Your email is never shown. You can change it later on the project's page.

## 0.1.10 (2026-10-20)

- **Projects made on other setups fit yours**: when you open a project saved for another machine, post-processor or bits than yours, GCarve says what differs and offers to switch it: your machine and G-code style, your own bits of the same size, and your board's thickness (through cuts follow it). Any time: File › Make It on My Machine….
- **Free community projects**: File › Free Community Projects… opens gcarve.app/projects. Download a project (.tlp), open it, and fit it to your machine as above.
- **Share to Community**: File › Share to Community… puts your project on gcarve.app/projects for others to cut on their own machines: a title, a few words, tags, a license, a picture of the simulated cut and your own photos. It is checked before it appears.

## 0.1.9 (2026-10-20)

- **More files open**: EPS (Illustrator, Affinity and other PostScript), LightBurn projects (.lbrn2) with their layers, and 3MF 3D models (Bambu and other multi-part files too) for 3D carving.
- **Quicker editing**: while drawing, only the toolpath you click in the list (or the one open in its dialog) is shown and worked out, a moment after you stop editing. Simulate and Export always work out every toolpath. Want them all while drawing? View › Every Toolpath While Drawing.

## 0.1.8 (2026-10-19)

- **Phone-sized layout**: on a narrow screen the menus fold under one ☰ Menu, panels start folded, and one finger pans the drawing (two fingers pan and pinch to zoom).
- **Tabs**: "One every" a set distance gives bigger parts more tabs, or keep the same number on every part. Placed tabs can be dragged along the outline.
- **Sheet names**: double-click a sheet tab (or right-click › Rename) to name it; VCarve sheet names come along. The sheet tabs scroll sideways when they don't fit.
- **Job setup in plain words**: "Lift for moves", "Where you zero the machine", "Touch off on top of the board", with an "At the machine" summary of what to do before you cut.
- **Photo of the board**: View › Photo of the Board…: pick a photo of the board on the machine, click its four corners, and it's straightened under the drawing so you can place parts around knots and old cuts.

## 0.1.7 (2026-10-19)

- **All sheets on one screen**: the All sheets button by the sheet tabs (or View › All Sheets Side by Side) shows every sheet in a grid. Click a sheet to work on it; drag vectors onto another sheet to move them there, toolpaths and all.
- **Toolpath list**: folders (drag toolpaths in, or ⌘ / Shift-click several and Group…), each with an eye and its own Export; view by folders, by sheet or by bit; show all toolpaths or only this sheet's, and filter by layer. Each toolpath shows the sheets it cuts on and is greyed when it cuts nothing on the sheet you're on.
- **Where a toolpath cuts**: Applies to picked vectors, a layer, a group or all vectors, on all sheets or one sheet only. Right-click a toolpath to cut it on all sheets, on this sheet only, or its layer on every sheet.
- **Tabs**: evenly spaced tabs can keep off corners (on for new toolpaths), and a clearer Place tabs on the drawing button.
- **3D view**: zoom in and hold still to see the part on screen at full detail; a lighter view while the cut plays.
- **Top bar**: the command search sits at the right of the menus, and Design / Simulate is centred.

## 0.1.6 (2026-10-19)

- **3D view**: the carved board is drawn more accurately. Cuts are no longer widened and thin strips between cuts no longer disappear, with more detail overall.
- **Mouse wheel in 3D**: a spin of the wheel now only zooms (it could zoom and orbit by turns on mice with smooth scrolling); two-finger trackpad swipes still orbit or pan.
- **VCarve projects (.crv)**: tabs placed by hand in VCarve land in the same places.

## 0.1.5 (2026-10-19)

- **VCarve projects (.crv) open with their toolpaths**: profile, pocket, drill, V-carve and chamfer toolpaths come across with their bits (feeds, speeds, V angle), depths, tabs and ramps. Toolpath folders and merged toolpaths become groups in the toolpath list.
- **Text from VCarve** comes in as letter outlines, grouped per text (text on a curve can sit up to about 1/32 in from VCarve's position).
- **Array copies**: any toolpath can now be repeated across a grid (rows × columns, with a spacing), like VCarve's Array Copy Toolpath. VCarve array copies open as this.
- **Simulation**: playback speed slider (1× to 200×), and a Show menu: all moves, only what has been cut so far (the tool's trail) or none, plus rapid moves and the cutter on or off, in 2D and 3D. The status bar's Paths switch now also hides the lines in the 3D view.
- **Opening files**: the Open screen leads with "Open a file from this computer", and big files show an "Opening…" card while they load.

## 0.1.4 (2026-10-19)

- **Open VCarve projects (.crv)**: File › Open (or drop the file on the window) brings in every vector with its layer, and each VCarve sheet with its board size and thickness. The .crv file is never changed. Toolpaths aren't read yet: add them in GCarve.
- Fixed a stray line drawn across the screen by tiny shapes that double back on themselves.
- Security update of the encryption library used for sign-in and update checks (rustls 0.23.46).

## 0.1.3 (2026-10-18)

- **Update now** (Help › Check for Updates): downloads the new version for your computer and opens it, with the steps to install it. If the download doesn't work, the download page opens in your browser instead.
- Fixed a crash when the toolpaths or a 3D model are one long, very thin strip (a texture too tall for the graphics card).
- Esc closes the Start a job card.

## 0.1.2 (2026-10-17)

**What's fixed in 0.1.2**
- Links open again: Donate, Download, Support, the website and **sign in through the browser** did nothing in 0.1.1 (the browser-opening feature was left out of the build).
- The Dock (macOS) and taskbar show the GCarve icon instead of a generic one.
- Help › Check for Updates: a dev switch and a live test behind it; 0.1.1 users get the update notice.
- The ⌘K command palette takes the first letter typed right after opening.
- About GCarve counts the built-in post-processors instead of a stale number.

## 0.1.1 (2026-10-16)

- Help menu: **Report a Bug**, **Request a Change**, **Check for Updates** and **Donate**, all inside the app.
- The app now tells you at start when a newer version is out (Help › Check for Updates).
- A clearer sign-in box: one button for sign-in or sign-up (email link, passkey or password), password fields as a shortcut.
- Reports sent while signed out now go through.

## 0.1.0 (2026-10-16)

First public desktop build of the Rust version of GCarve.

**Highlights**
- Design, toolpaths, 3D simulation and G-code export: profile, pocket, V-carve, inlay, drill, bore, thread, chamfer, prism, flute, texture, photo V-carve, 3D relief.
- Open VCarve / Fusion / any G-code (.tap .nc .gcode .ngc .cnc): the bit is recognised, the program is simulated, checked and can be re-posted.
- Import DXF, DWG, SVG, PDF, Illustrator, STL and pictures to trace. Export SVG, DXF, DWG.
- SpaceMouse (3Dconnexion) support in the 3D view and on the drawing.
- Much faster pocket, V-carve and cut-out generation on busy drawings.
- 66 post-processors, 29,000+ bits, 2,100+ machines.
- Crash recovery: unsaved work is offered back at the next start.
- Free to use with no account. Signing in is optional (File › Sign In…): it opens your browser, you approve the computer on gcarve.app, and cloud projects and shared libraries appear.

**Notes**
- macOS app is not notarized and Windows files are not code-signed yet (see the README for the first start).
- Checked here: macOS (Apple silicon + Intel), Linux x86-64 and ARM64 (headless), Windows 10/11 x64 and Windows 7 x64 (under Wine). The 32-bit Windows 7 build has not been run yet: please report how it goes.
