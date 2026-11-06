**What's new in 0.1.28**
- Fixed: some lettering made V-Carve toolpaths run out of memory and the app stopped (seen in the online app). Those drawings now carve normally.
- Post-processors: Syntec / Weihong, RichAuto, Next Wave and ShopBot end each file at the job's lift height instead of a fixed height (a fixed 1 in could lower a stopped bit into a thick board with Z zero on the table). A post can set its own with homeZ.

Free to use, no account needed. Downloads: macOS (Apple silicon and Intel, signed and notarized), Windows 10/11 and 7/8 (64 and 32-bit), Linux x86-64 and ARM64. Checksums are in SHA256SUMS.txt. The Windows files are not code-signed yet (see the README for the first start).
