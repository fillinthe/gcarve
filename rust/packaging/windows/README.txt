GCarve (Rust build, @VARIANT@)

Run gcarve.exe. Nothing to install; settings and the bit library live in %APPDATA%\GCarve.
Open a project by dropping a .tlp file on gcarve.exe, or with File > Open. Samples are in samples\.

Needs OpenGL 2.0 or newer (any graphics driver from the last ~15 years). If the window does not
open, update the graphics driver; on a PC or VM without one, put Mesa's opengl32.dll (llvmpipe,
e.g. from https://github.com/pal1000/mesa-dist-win) next to gcarve.exe.

Self-check: double-click run-tests.cmd. It starts GCarve with samples\showcase.tlp, saves a
screenshot, closes it again and writes test-results.txt (+ test-shot.png) in this folder.
Please send both files back.
