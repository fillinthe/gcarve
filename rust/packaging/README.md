# Rust GCarve packaging

One script per target; artifacts land in `rust/packaging/out/` (gitignored). Builds stay local: never upload
them anywhere. Release profile for packages = `[profile.dist]` in
`rust/Cargo.toml` (release + fat LTO, 1 codegen unit, stripped; opt-level stays 3 so CAM speed is unchanged).
Packages keep the `screenshot` feature (inactive unless `GCARVE_SHOT` is set): the self-checks use it.

| Target | Command | Runs on | Output |
|---|---|---|---|
| macOS universal | `sh rust/packaging/macos/build.sh` (test: `macos/test.sh`) | Mac | `GCarve.app`, `GCarve-<v>-macOS.dmg` |
| Windows 10/11 x64 | `sh rust/packaging/windows/build.sh x64` | Mac, `brew install mingw-w64` | `GCarve-<v>-Windows-x64.zip` |
| Windows 7/8 x64 | `sh rust/packaging/windows/build.sh win7-x64` | Docker (auto) | `GCarve-<v>-Windows-win7-x64.zip` |
| Windows 7/8 32-bit | `sh rust/packaging/windows/build.sh win7-x86` | Docker (auto) | `GCarve-<v>-Windows-win7-x86.zip` |
| Linux x86_64 + arm64 | `sh rust/packaging/linux/build.sh` (test: `linux/test.sh [aarch64\|x86_64]`) | Docker (auto) | `GCarve-<v>-Linux-<arch>.tar.gz` |

Docker = Colima. The build image `gcarve-rust-build` (`docker/Dockerfile.build`, Debian 11 from
archive.debian.org) has rustup stable + nightly, the x86_64 Linux cross gcc and Debian's mingw-w64.
Caches (mine, safe to delete): `rust/target/docker/`, Docker volume `gcarve-rust-registry`, image
`gcarve-rust-build`; Mac builds use `rust/target/<triple>/dist`.

## Details per target
- **macOS**: arm64 slice min 11.0, x86_64 slice min 10.13 (`LC_VERSION_MIN_MACOSX 10.13`); `lipo` universal;
  Info.plist = `app/Info.plist.in` (registers `.tlp`, docicon) with LSMinimumSystemVersion 10.13; signed ad-hoc,
  or `GCARVE_CODESIGN_IDENTITY="GCarve Dev"`; `.dmg` via hdiutil (with an /Applications link).
- **Windows**: icon (`app/branding/appicon.ico`) + VERSIONINFO from `windows/gcarve.rc.in` (windres object
  passed as a link arg, no build.rs change); GUI subsystem via `-Wl,--subsystem,windows`. Zip = gcarve.exe,
  README.txt, run-tests.cmd, samples\. No DLLs to ship (libgcc linked statically).
  - x64 (Win10/11): stable `x86_64-pc-windows-gnu`, Homebrew mingw (UCRT; built into Windows 10).
  - Win7/8: tier-3 `x86_64-win7-windows-gnu` / `i686-win7-windows-gnu`, nightly `-Zbuild-std`, Debian mingw
    (msvcrt.dll, present on every Windows; Homebrew's mingw would need the UCRT update KB2999226).
    Found and fixed: `windows-core` 0.62 imports `CoTaskMemFree`/`CoCreateFreeThreadedMarshaler` from
    **combase.dll (Windows 8+)** → the exe would not start on Windows 7. `windows/imports.py --fix-combase`
    renames that import to ole32.dll (Windows 7's ole32 exports both; on 8+ ole32 forwards to combase) and then
    fails the build if any Windows 8+ DLL is still imported. The remaining kernel32/user32 import list was checked
    by eye against Win7 (no WaitOnAddress, GetSystemTimePreciseAsFileTime, *ForDpi, …: winit loads those dynamically).
    32-bit uses `-C panic=abort` (mingw i686 unwinding is SJLJ, Rust wants DWARF): a panic exits instead of unwinding.
    getrandom/std pick RtlGenRandom (SystemFunction036) on the win7 targets.
  - `windows/run-tests.cmd` (in every zip): starts gcarve.exe with samples\showcase.tlp, screenshot after 20 frames,
    writes test-results.txt + test-shot.png. OpenGL 2+ needed; README.txt says how to use Mesa's opengl32.dll
    on PCs/VMs without a GL driver.
- **Linux**: built on Debian 11 → max symbol GLIBC_2.30 (runs on Debian 11+, Ubuntu 20.04+, RHEL 9+). tar.gz with
  gcarve, gcarve.desktop, gcarve.png, install.sh (~/.local). X11/Wayland/GL libs are dlopened from the system
  (needs libGL/libEGL, libxkbcommon-x11 or wayland); file dialogs need an xdg-desktop-portal. No AppImage
  (it would bundle nothing useful: the GL/X libs must come from the system anyway).

Rebuilt 2026-10-13 (all six packages; macOS, Windows x64, Windows 7 x64 under Wine and Linux arm64 self-tests passed). Sizes below are from 2026-10-08: binaries are now ≈47–54 MB and packages ≈10–24 MB because the embedded bit / machine database (`data/`) grew to 30 MB. Build caches (`rust/target/<triple>`, `rust/target/docker`, `out/obj-*`) and the `gcarve-wine` Colima profile + its disk (`~/.colima/_lima/_disks/colima-gcarve-wine`, not removed by `colima delete`) were deleted after the run to free disk; the scripts recreate them.

## Sizes (0.1.0, dist profile)
| Target | Binary | Package |
|---|---|---|
| macOS universal | 32.6 MB (arm64 15.6, x86_64 17.0) | dmg 14.3 MB |
| Windows x64 | 21.1 MB | zip 8.3 MB |
| Windows 7 x64 | 21.0 MB | zip 8.2 MB |
| Windows 7 x86 | 20.2 MB | zip 8.0 MB |
| Linux arm64 | 19.7 MB | tar.gz 8.0 MB |
| Linux x86_64 | 22.1 MB | tar.gz 8.6 MB |
(≈4.4 MB of that is the embedded machine/bit databases from data/; Windows sizes include bundled SQLite (rusqlite),
which cross-compiles with mingw for all three Windows targets.)

## What was tested how
- macOS: `macos/test.sh` ran the universal app natively (arm64) and under Rosetta (x86_64) on macOS 26; both
  rendered showcase.tlp correctly (screenshot checked). `codesign --verify --deep --strict` passes.
- Linux arm64: `linux/test.sh` in debian:bookworm-slim with Xvfb + Mesa llvmpipe: renders showcase.tlp (checked).
- Linux x86_64: `linux/test.sh x86_64` (same, amd64 container under qemu): renders showcase.tlp (checked).
- Windows under Wine: `windows/wine-test.sh [x64] [win7-x64] [win7-x86]` (Docker image `windows/Dockerfile.wine`:
  Debian 13 amd64, Wine 10, Xvfb, Mesa llvmpipe GL 4.5). Runs in its own Colima profile `gcarve-wine` (30 GB disk,
  Rosetta for x86_64) because unpacking the image on the default profile's shared 20 GB disk filled it and crashed
  that VM. Per build: showcase.tlp screenshot (`out/shot-wine-<v>.png`), `run-tests.cmd` under `wine cmd`
  (`out/wine-test-results-<v>.txt`), `--control` screenshot. Wine 8 (Debian 12) can't start the x64 build: no
  bcryptprimitives.dll (Rust std's ProcessPrng; real Windows 10 has it).
  - x64, win7-x64: start, render showcase.tlp correctly, run-tests.cmd PASSED.
  - win7-x86: cannot run on an Apple-silicon Mac. Classic wine32 runs i386 code under qemu-i386 user emulation,
    which gives Wine no trap number in the signal context (`segv_handler Got unexpected trap -1`): the first page
    fault Wine must handle itself (stack growth etc.) kills the process. New-WoW64 Wine (Arch, wine 11) runs 32-bit
    code in a 64-bit process via a 32-bit code segment, which neither Rosetta nor qemu-x86_64 supports. Needs an
    x86_64 Linux host (same script) or a real Windows.
  - `--control` panicked on every platform (also macOS natively): `FontFamily::Name("bold") is not bound`.

## Unverified
- Any real run on Windows (10/11, 7, 8, 32-bit): run `run-tests.cmd` on the shop PC / a Win7 VM. Wine only says
  the x64 builds load, link and render; it doesn't prove Windows 7's DLLs (Wine reports Windows 10).
- The 32-bit build has never run anywhere (see Wine above).
- macOS older than 26 (10.13–15): only the load commands say 10.13/11.0; nobody ran it there.
- Linux on a real desktop (Wayland, GPU drivers, file dialog portal).
