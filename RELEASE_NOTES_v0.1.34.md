**What's new in 0.1.34**

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

Free to use, no account needed. Downloads: macOS (Apple silicon and Intel, signed and notarized), Windows 10/11 and 7/8 (64 and 32-bit), Linux x86-64 and ARM64. Checksums are in SHA256SUMS.txt. The Windows files are not code-signed yet (see the README for the first start).
