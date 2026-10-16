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
