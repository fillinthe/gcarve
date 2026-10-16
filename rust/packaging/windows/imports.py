"""Windows 7 import check for a PE file (PE32 or PE32+), no dependencies.
usage: python3 imports.py gcarve.exe [--fix-combase]
Prints every imported DLL with its functions, flags DLLs Windows 7 does not have, exits 1 if any remain.
--fix-combase renames the import of combase.dll (Windows 8+) to ole32.dll in place: windows-core links
CoTaskMemFree / CoCreateFreeThreadedMarshaler from combase, and Windows 7's ole32 exports both
(on Windows 8+ ole32 forwards them to combase, so the patched exe still runs there)."""
import struct, sys

# ponytail: DLL-level check only; a Win8+ function inside a Win7 DLL still needs the printed list read
# by eye (done for the current build, see README) or a run on real Windows 7.
WIN8_PLUS = {"combase.dll", "bcryptprimitives.dll", "api-ms-win-core-synch-l1-2-0.dll", "shcore.dll",
             "api-ms-win-core-winrt-l1-1-0.dll", "api-ms-win-core-winrt-string-l1-1-0.dll", "d3d12.dll", "dxcore.dll"}

path, fix = sys.argv[1], "--fix-combase" in sys.argv
data = bytearray(open(path, "rb").read())
pe = struct.unpack_from("<I", data, 0x3C)[0]
nsect, optsize = struct.unpack_from("<H", data, pe + 6)[0], struct.unpack_from("<H", data, pe + 20)[0]
opt = pe + 24
pe32plus = struct.unpack_from("<H", data, opt)[0] == 0x20B
imp_rva = struct.unpack_from("<I", data, opt + (120 if pe32plus else 104))[0]  # data directory 1 = imports
sections = [struct.unpack_from("<IIII", data, opt + optsize + 40 * i + 8) for i in range(nsect)]  # vsize, va, rawsize, raw


def off(rva):
    for vsize, va, rawsize, raw in sections:
        if va <= rva < va + max(vsize, rawsize):
            return rva - va + raw
    raise ValueError(hex(rva))


def cstr(o):
    return bytes(data[o:data.index(0, o)]).decode()


imports, d = {}, off(imp_rva)
while True:
    thunk, _, _, name_rva, iat = struct.unpack_from("<IIIII", data, d)
    if name_rva == 0:
        break
    name_off = off(name_rva)
    dll = cstr(name_off).lower()
    if fix and dll == "combase.dll":
        data[name_off:name_off + 12] = b"ole32.dll\0\0\0"
        dll = "ole32.dll"
    fmt, size, flag = ("<Q", 8, 1 << 63) if pe32plus else ("<I", 4, 1 << 31)
    t, funcs = off(thunk or iat), imports.setdefault(dll, set())
    while (v := struct.unpack_from(fmt, data, t)[0]) != 0:
        funcs.add(f"#{v & 0xFFFF}" if v & flag else cstr(off(v & 0x7FFFFFFF) + 2))
        t += size
    d += 20
if fix:
    open(path, "wb").write(data)
bad = sorted(x for x in imports if x in WIN8_PLUS)
for x in sorted(imports):
    if not x.startswith("api-ms-win-crt"):
        print(f"{x}{'  <-- NOT ON WINDOWS 7' if x in bad else ''}: {' '.join(sorted(imports[x]))}")
crt = "UCRT (api-ms-win-crt-*): Windows 7/8.1 need update KB2999226" if any(x.startswith("api-ms-win-crt") for x in imports) else "C runtime: msvcrt.dll"
print(crt)
sys.exit(1 if bad else 0)
