# Rust app ↔ Qt app parity

Inventory of every user-visible feature of the Qt app (app/Main.qml menus, tbDefs, paletteCommands, railItems,
shortcuts, popups, app/*.qml dialogs, app/canvas*.cpp) and its state in the Rust/egui app.
Status: **present** · **partial** (what is missing) · **missing** · **n/a** (deliberately not ported, see QUESTIONS.md).
GCarve Control (ControlMain.qml, control/*, machine.*, programview.*) is ported separately and not listed.

Initial state (2026-10-08, before this pass) → state after this pass. **TODO** = still missing.

## File menu
| Qt | before | after |
|---|---|---|
| New (⌘N, new tab) | partial: replaces the document | present: new tab |
| Close Tab (⌘W) | missing | present (asks to save a dirty tab) |
| Open Project / Import… (⌘O) → open in tab (reuse an empty tab) | partial: always into the current doc | present |
| Cloud Projects… | present | present (opens in a new tab unless empty) |
| Jobs… (⌘J, desktop) | present (menu; ⌘J opened Board) | present |
| Import 3D Model (STL)… | present | present |
| Open Recent ▸ (+ Clear Menu, desktop) | missing | present |
| Save (⌘S: to the project path, else Save As) | partial: always a save dialog, path forgotten | present |
| Save As… (⇧⌘S) | partial | present (path, title, recent) |
| Save to Cloud / Download Project (web) | present | present |
| Account / Sign In… | present | present |
| Unsaved-changes prompt (close tab, quit) | missing | present |
| Window title "• name — GCarve", job name in the mode bar title | partial (no window title, no job) | present |

## Edit menu
| Qt | before | after |
|---|---|---|
| Undo / Redo (⌘Z / ⇧⌘Z) | present | present |
| Cut / Copy / Paste (⌘X/C/V) | present | present |
| Select All (⌘A) | present | present |
| Duplicate (⌘D) | present | present |
| Group (⌘G) / Ungroup (⇧⌘G) | present | present |
| Delete (also a selected dimension) | present | present |
| Add Text… (T) | present | present |
| Preferences… (⌘,) | present | present (+ New documents: 60 × 96 × 0.75 in, Plywood, XY0 lower-left, Z top, inches, wincnc-single, Snap off; Snap & grid; synced to the account via GET/PUT /api/account/prefs with the Qt JSON names) |
| Shortcut labels: ⌘ ⇧ ⌥ on Apple (macOS, Mac/iPhone/iPad browsers), Ctrl+Shift+… elsewhere | missing | present (`theme::keys` in menus, tooltips, palette, hints) |

## Vectors menu
Weld, Subtract, Intersect, Offset…, Dogbone…, Join, Close, Flip H/V, Rotate 90°, Centre on Board, Align & Distribute…
(⇧⌘A), Array Copy…, Nest on Board…, Move to Next Sheet, New Sheet: **present** (⇧⌘A missing → added).
| Constrain Two Shapes… (Link) | missing (menu disabled) | present (ConstraintDialog) |

## Variables menu
| Variables & Constraints… (⇧⌘E) | missing | present (VariablesDialog: variables with usage badges, built-ins, constraints list) |
| Add Variable | missing | present |
| Bindings: shape x/y/w/h, node n<i>.x/y, segment s<i>.len, op + board fields re-evaluated on every variable / board / sheet change and on load (Canvas::applyBindings) | missing (bindings saved but never applied) | present (`Document::apply_bindings` in core) |
| Inspector X/Y/W/H: typing a variable binds, a number unbinds, bound expression shown | partial (expressions evaluated once) | present |
| Node popup: X / Y / Length to next, bindable | partial (X/Y only) | present |

## Toolpaths menu
| New Profile/Pocket/Drill/V-Carve/Bore/Inlay/Flute… (⇧⌘P K D V B I F) | present (no shortcuts) | present (shortcuts; ⇧⌘P = side panel, see QUESTIONS) |
| Export G-code… (⌘E) / Export Air-Cut Test… | present | present |
| Send to GCarve Control / Open GCarve Control (desktop) | missing | present (`App::control` → crate::control::send / open) |
| Tool Library… / Board / Job Setup… / 3D Models… / Post-Processors… | present | present (3D toolpath dialog: Boundary row = selected closed vectors limit the machined area) |

## View menu
| Show Toolpaths (⇧⌘T) | present (no shortcut) | present |
| Show / Hide All Toolpaths | present | present |
| Show Cut Width | missing (disabled) | present |
| Simulate Cut (⌘R) / Watch the Cut (⇧⌘R) / 3D View (⌘3) | present | present (Watch the Cut stays in Design, as in Qt) |
| Show Simulation | missing | present (sim overlay on/off in Design like Qt canvas.showSim; leaves Simulate mode) |
| Zoom In/Out (⌘+/⌘−), Fit (⌘0), Board (⌘1), Actual Size (⌘2) | partial (menu only; ⌘+/⌘− zoomed the whole UI) | present |
| Layout ▸ Modes / Classic | missing | n/a: only the modes layout is ported (QUESTIONS) |
| Command Palette… (⌘K) | partial | present |
| Side Panel (⇧⌘P) / Tool Rail | present (not remembered) | present (remembered) |
| Toolbar Labels / Customize Toolbar… / Expand / Collapse All Panels | missing | n/a: classic-layout only (QUESTIONS) |
| Snap, Units ▸ | present | present |
| Snap Settings… (status-bar Snap ▾ popover: Points / Midpoints / Centres / Quadrants / On edge / Grid, snap distance px; grid show / spacing Auto or fixed / major every N; Rulers) | missing | present (`prefs::snap_grid_ui`; `snap_at` honours the kinds, Measure / Dimension keep object snaps with Snap off) |
| Show Grid, Rulers (top/left in project units, adaptive ticks, board extent, pointer markers; fits leave room) | missing | present (`Canvas::paint_rulers`, `view_area`) |

## Help menu
| ♥ Donate… / Manage Account… or Sign In… / User Management… (admins) / GCarve Website | partial (no Manage Account / User Management) | present |
| About GCarve | missing (Qt has none) | present (version, licence, website) |

## Command palette (⌘K)
| every toolbar command (tbDefs), every tool, every toolpath kind, views (modes, zoom, 3D, variables, prefs, 3D models, STL import, add text, align, nest, new sheet, select all), every post (Post: picks it), every variable | partial (fixed list of ~40) | present |
| ↑/↓ moves the highlight, Enter runs it, hover highlights, Esc closes | partial (Enter = first hit) | present |

## Canvas
| Select / move / marquee (window vs crossing), shift add/remove, groups | present | present |
| Selection resize / rotate handles (+ Keep proportions) | missing (Qt has none either) | n/a until answered (QUESTIONS: Qt has none; inspector W/H + Keep proportions cover it) |
| Node editing (drag, handles, insert, remove, smooth/corner, context menu) | present | present |
| Shape tools + size popup (W/H, Square/Circle lock, Centre X/Y) | partial (no centre fields, no lock) | present |
| Polygon sides / star points under the rail (Qt rail spin boxes) | partial (size popup only) | present (Sides / Points spin box under the Design rail, 3–64) |
| Polyline, curve, arc, measure, dimension, text tools | present | present |
| Dimension popup (label, drive, anchor note) | present | present |
| Shape context menu (Arrange ▸, Combine ▸, Toolpath ▸, zoom) | partial (flat list, toolpaths disabled) | present (Qt structure) |
| 3D model preview on the board + drag to move it (snapped corner) | missing | present (reliefs.rs: shaded preview cached per model, dashed outline, Select-tool drag, one undo step, regenerate on release) |
| Larger handles preference (hit / snap / node radii ×1.7) | missing (saved only) | present |
| Keyboard: tool keys, Delete, Esc, Enter, arrows nudge (Shift ×10), F fit, 1 / 3 modes, 2 = Toolpaths tab | present | present (1/2/3 no longer fire with ⌘) |
| Right-drag / space-drag pan, wheel / pinch zoom | present | present |

## Inspector
| Properties: X/Y/W/H, Keep proportions, Arrange / Boolean grids, Board summary + feeds summary | present | present (+ board bindings line) |
| Selected text row (wording, font, edit) | missing | present (Edit Text / Convert to curves) |
| Layers: rename, colour picker, visible, lock, double-click selects its shapes, Move selection to active layer, add/remove | partial (no colour picker, no double-click, no move) | present (layers.rs; rename / colour / remove are undo steps, visible / lock are view state like Qt; Qt has no layer reordering, so neither does Rust) |
| Toolpaths: list, eye, double-click edit, click selects the op's shapes, warnings, Edit/Delete | partial (click selected one shape) | present (row click → `opselect:i`, layer / group scopes included) |
| Toolpaths tab: NEW TOOLPATH tiles (8 kinds) + SETUP (Bits / Posts / Board) above the TOOLPATHS list, same tile grid as ARRANGE / BOOLEAN, disabled tiles explain why | n/a (Toolpaths-mode rail) | present (`inspector::tile_grid`) |

## Dialogs
Text, Profile/Pocket/Drill/V-Carve/Bore/Inlay/Flute/3D (op dialog), Offset, Dogbone, Array, Align, Nest, 3D Models,
Board/Job Setup (machine picker), Post-Processors (search, upload .pp/.json, export, delete), Preferences, Jobs,
Export guard (safety list, air-cut raise), Account, Cloud Projects, Sim bar: **present**.
| Tool Library: Vectric .vtdb import (desktop) | missing | present (vtdb.rs, rusqlite bundled, desktop; material rows → presets; test on samples/vectric-tools.vtdb; the web shows a hint) |
| Tool Library: Fusion / GCarve JSON import, export, bit database | present | present |
| Tool changer slot per tool (ATC machines) | missing | partial: core done (Tool::slot saved when > 0, T number in posts, export warnings + test); slot field in the Tool Library (prominent "ATC slot" on tool-changer machines, T numbers in the list), slot on the toolpath dialog's tool line, free slots auto-assigned when bits are added / imported, re-imports keep the slot |
| Desktop licence gate (Pro, 14-day offline grace) | missing | present (cloud.rs `licensed()`: Pro or admin online, else 14 days since the last online check with a stored token; blocking modal with Check again; GCARVE_SHOT runs skip it; `GCARVE_DIALOG=licence`) |
| Constraint dialog (anchor/target, swap, relation, gap) | missing | present |

## Multiple documents
| Tabs (DocCanvas per tab, strip shown when > 1, middle-click / × closes, + new) | missing | present |
| Open in tab (reuse an empty untouched tab), switch to an already-open path | missing | present |

## Look (app/style/*.qml)
| Shared controls in `widgets.rs` (one implementation each): Dialog frame (title, separator, OK/Cancel or Close footer, Esc/Enter, modal backdrop, scrolls when taller than the window, modeless anchored like Qt), Button (primary / flat), TextField metrics + accent focus ring, ComboBox (30 px, chevron), CheckBox, Switch, RadioButton, SpinBox, TabButton, NumField badge, MenuItem / MenuSeparator / submenu (28 px rows, blue highlight, shortcut text, tick boxes), ToolTip colours (`.tip()`), Popup frame, ItemDelegate list rows | partial (egui defaults) | present |
| Every dialog / popup on the shared frame (op, tools, posts, job, relief, prefs, variables, constraint, jobs, text, offset, dogbone, array, align, nest, size / node / dimension popups, account, cloud, unsaved, about, safety check, Control machine settings / start) | partial | present |
| Status bar (cursor readout 230 px mono, message left, Paths/Width, zoom, Snap, units), SimBar, start card, context menu | partial | present |
| Screenshot hooks GCARVE_DIALOG=shapemenu / sizepopup / palette, GCARVE_SELECT=<id> | missing | present |

## Modes (user decision 2026-10-08)
| Qt | before | after |
|---|---|---|
| Mode bar Design \| Simulate (Toolpaths mode dropped; its rail moved into the inspector's Toolpaths tab; key 2 / palette "Toolpaths tab" open that tab) | Design / Toolpaths / Simulate | present |
| Mode bar without the file buttons (new / open / jobs / save: File menu) | had them | present |

## Look pass 3
| Qt | before | after |
|---|---|---|
| Jobs dialog: edge-to-edge list (status dot, count, New Job + folder footer), header row (name, status combo, folder, Export All), notes, Projects + New / Add…, bordered project cards with icon buttons, history chevrons, empty-state Blanks, name prompt (new project = empty .tlp in the job, opened in a tab) | egui-style layout, inline name fields | present (`Dialog::flush`) |
| 3D view overlay: Z ×n exaggeration slider, Reset view, double-click resets, mouse hint; Ortho \| Persp switch (35° perspective, remembered); pan with right / middle / shift-drag and two fingers (Reset view resets it) | missing | present |
| Export dialog (ExportGuard.qml) on every export: editable file names (cleaned: no / \\ :, post extension kept; `pipeline::clean_file_name`) with a dim "tool · toolpaths" line (`ExportResult::plan`), Save to <folder> + Choose… (job gcode folder, else the last folder, remembered; written without a second picker, revisions to history/), web: downloads note, air-cut Raise Z, safety issues, Cancel / Export / Export anyway / Download; `GCARVE_DIALOG=export\|aircut` | safety-check dialog only, folder picker after | present |
| Arrow glyphs the web fonts lack (⤒ ⇄ ▸): app/icons align-* / arrows-* / swap on `widgets::Button::icon`; ▸ → › in web-visible text | boxes on the web | present |
| Size popup focuses Width with the text selected | missing | present (`units::focus_next`) |
| Tool Library number format: feeds / V angle 1 decimal, diameter unit decimals + 1 | trimmed numbers | present |
| Offset dialog: 0.125 (3 mm) in project units, Outwards / Round / Keep original on at first, remembered between opens | reset each open, Keep off | present |
