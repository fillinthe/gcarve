**What's new in 0.1.35**

Post processors
- Upload posts… with many files at once adds them all to the list and keeps the board's post as it was, with one message saying how many came in (and how many couldn't be read). One file is still added and picked for the board.

GCarve Control
- A program line that writes the controller's memory (G10 L2 / L20 work zero, G28.1 / G30.1, `$` settings) is sent on its own, and nothing follows until the controller answers it: grbl stops listening while it writes, so lines sent then could arrive damaged.
- Checked end to end against a machine that counts down from home like stock grbl (FoxAlien Masuter 3S): an inch job homes, passes the start check and runs to the end.

Free to use, no account needed. Downloads: macOS (Apple silicon and Intel, signed and notarized), Windows 10/11 and 7/8 (64 and 32-bit), Linux x86-64 and ARM64. Checksums are in SHA256SUMS.txt. The Windows files are not code-signed yet (see the README for the first start).
