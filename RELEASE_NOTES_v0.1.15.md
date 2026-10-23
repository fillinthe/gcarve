**What's new in 0.1.15**
- **Rotary jobs**: Job Setup › Rotary (A axis, along X or Y) with the blank diameter; the board is the unrolled surface and every G-code post writes the turn as A. Rounding toolpath for a square blank, Vectors › Rotary Layout for wrapped spirals and flutes, and the 3D view shows the carved cylinder.
- **New toolpath options**: Moulding (sweep a profile along rails, with an optional clearing bit), keyhole slots, drag knife, sketch carving (from a picture or a 3D model), follow a 3D model, merged toolpaths (one tour per bit), cut in drawing order, cutter compensation (G41/G42 for posts that know it), park position at the end, and a ghost-run twin of every file (spindle off, above the board).
- **Two-sided jobs**: the back sheet shows the front mirrored, its simulation starts with the front's cuts, and nesting moves both sides together.
- **Cabinet work**: import a CSV cut list (labelled parts nested onto sheets), shelf-pin hole rows, Plate Production from a list.
- **Drawing**: Freehand tool, Distort (bend along a curve, fit between two curves, fit into four corners), Vector Selector, Fit Arcs and Curves, Vector Texture, angular / horizontal / vertical dimensions, arched text, line spacing, kerning by hand, a "Grows from" anchor for sizes.
- **Nesting**: from any corner, mirrored parts, inside an offcut, fill the rest with the smallest part.
- **Pictures**: trace one colour or one area, brightness / contrast / gamma for Photo V-Carve, WebP pictures.
- **Bits**: folders in the Tool Library, bit names from a pattern, custom form cutters drawn as a vector.
- **More**: job templates (.tlpt), PDF vector export, Save Preview Image, lithophane view, wood colours by board material, shop name and logo on the setup sheet, Gerber aperture macros, LightBurn pictures, File › Send to GBurn (laser).

Free to use, no account needed. Downloads: macOS (Apple silicon + Intel), Windows 10/11 and 7/8 (64 and 32-bit), Linux x86-64 and ARM64. Checksums in SHA256SUMS.txt. The macOS app is not notarized and the Windows files are not code-signed yet (see the README for the first start).
