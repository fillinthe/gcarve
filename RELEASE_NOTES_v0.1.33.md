**What's new in 0.1.33**

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

Free to use, no account needed. Downloads: macOS (Apple silicon and Intel, signed and notarized), Windows 10/11 and 7/8 (64 and 32-bit), Linux x86-64 and ARM64. Checksums are in SHA256SUMS.txt. The Windows files are not code-signed yet (see the README for the first start).
