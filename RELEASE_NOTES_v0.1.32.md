**What's new in 0.1.32**

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

Free to use, no account needed. Downloads: macOS (Apple silicon and Intel, signed and notarized), Windows 10/11 and 7/8 (64 and 32-bit), Linux x86-64 and ARM64. Checksums are in SHA256SUMS.txt. The Windows files are not code-signed yet (see the README for the first start).
