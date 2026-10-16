"""Validate the data/ JSON databases (stdlib only). Run: python3 -I data/check.py"""
import json, re, sys
from pathlib import Path

DATA = Path(__file__).resolve().parent
POSTDEF = DATA.parent / "rust/gcarve-core/src/postdef.rs"
errors = []


def err(where, msg):
    errors.append(f"{where}: {msg}")


def post_ids():
    s = POSTDEF.read_text()
    ids = re.findall(r'(?:grbl_like|mach_like|iso)\(&mut v, "([a-z0-9-]+)"|(?:base|shopbot)\("([a-z0-9-]+)"', s)
    return {a or b for a, b in ids}


def num(v):
    return v is None or (isinstance(v, (int, float)) and not isinstance(v, bool) and v >= 0)


def check_urls(where, rec):
    src = rec.get("sources")
    if not isinstance(src, list) or not src:
        err(where, "needs >= 1 source URL")
    elif not all(isinstance(u, str) and u.startswith("http") for u in src):
        err(where, "sources must be http(s) URLs")


def check_rpm(where, s):
    for k in ("power_kw", "power_hp", "rpm_min", "rpm_max"):
        if not num(s.get(k)):
            err(where, f"{k} must be a number or null")
    lo, hi = s.get("rpm_min"), s.get("rpm_max")
    if isinstance(lo, (int, float)) and isinstance(hi, (int, float)) and lo >= hi:
        err(where, f"rpm_min {lo} >= rpm_max {hi}")
    if s.get("type") not in ("router", "spindle"):
        err(where, f"bad spindle type {s.get('type')!r}")
    if s.get("cooling") not in ("air", "water", None):
        err(where, f"bad cooling {s.get('cooling')!r}")


def pair(where, rec, a, b, keys, factor):
    """a (inch) and b (mm) objects must both exist and agree within 2 %."""
    va, vb = rec.get(a), rec.get(b)
    if va is None and vb is None:
        return
    if not isinstance(va, dict) or not isinstance(vb, dict):
        err(where, f"{a} and {b} must both be objects (or both null)")
        return
    for k in keys:
        x, y = va.get(k), vb.get(k)
        if not num(x) or not num(y):
            err(where, f"{a}.{k}/{b}.{k} must be numbers or null")
        elif (x is None) != (y is None):
            err(where, f"{a}.{k} and {b}.{k}: one is null")
        elif x is not None and abs(x * factor - y) > 0.02 * max(y, 1):
            err(where, f"{a}.{k}={x} disagrees with {b}.{k}={y}")


CLASSES = {"desktop", "hobby", "prosumer", "industrial"}
TC = {"manual", "atc-linear", "atc-carousel", "atc-drum", "multi-head", "quick-change"}
MOTORS = {"stepper", "closed-loop", "servo", None}
REQ = ["id", "maker", "model", "variant", "years", "class", "work_area_in", "work_area_mm",
       "gantry_clearance_in", "spindles", "max_feed_ipm", "max_feed_mm_min", "rapid_ipm",
       "rapid_mm_min", "accel", "drive", "motors", "controllers", "tool_change", "tool_setter",
       "z_zero", "table", "sources", "notes"]


def check_machines():
    posts = post_ids()
    ids, counts = set(), {}
    for f in sorted((DATA / "machines").glob("*.json")):
        if f.name == "spindles.json":
            continue
        try:
            doc = json.loads(f.read_text())
        except ValueError as e:
            err(f.name, f"bad JSON: {e}")
            continue
        if not all(isinstance(doc.get(k), str) for k in ("maker", "website")) or not isinstance(doc.get("machines"), list):
            err(f.name, "needs maker, website (strings) and machines (list)")
            continue
        counts[doc["maker"]] = len(doc["machines"])
        for i, m in enumerate(doc["machines"]):
            w = f"{f.name}[{i}] {m.get('id')}"
            for k in REQ:
                if k not in m:
                    err(w, f"missing {k}")
            mid = m.get("id")
            if not isinstance(mid, str) or not re.fullmatch(r"[a-z0-9]+(-[a-z0-9]+)*", mid):
                err(w, "id must be kebab-case")
            elif mid in ids:
                err(w, "duplicate id")
            ids.add(mid)
            if m.get("maker") != doc["maker"]:
                err(w, "maker differs from file maker")
            if m.get("class") not in CLASSES:
                err(w, f"bad class {m.get('class')!r}")
            if m.get("motors") not in MOTORS:
                err(w, f"bad motors {m.get('motors')!r}")
            pair(w, m, "work_area_in", "work_area_mm", "xyz", 25.4)
            pair(w, m, "max_feed_ipm", "max_feed_mm_min", ("xy", "z"), 25.4)
            pair(w, m, "rapid_ipm", "rapid_mm_min", ("xy", "z"), 25.4)
            if not num(m.get("gantry_clearance_in")):
                err(w, "gantry_clearance_in must be a number or null")
            for j, s in enumerate(m.get("spindles") or []):
                check_rpm(f"{w} spindle {j}", s)
            ctl = m.get("controllers")
            if not isinstance(ctl, list):
                err(w, "controllers must be a list")
            else:
                for c in ctl:
                    p = c.get("gcarve_post")
                    if p is not None and p not in posts:
                        err(w, f"unknown gcarve_post {p!r}")
                    if not isinstance(c.get("name"), str):
                        err(w, "controller needs a name")
            tc = m.get("tool_change")
            if not isinstance(tc, dict) or tc.get("kind") not in TC:
                err(w, f"bad tool_change {tc!r}")
            elif tc.get("slots") is not None and not isinstance(tc["slots"], int):
                err(w, "tool_change.slots must be int or null")
            ts = m.get("tool_setter")
            if not isinstance(ts, dict) or not isinstance(ts.get("available"), bool):
                err(w, "tool_setter.available must be bool")
            if not isinstance(m.get("z_zero"), list):
                err(w, "z_zero must be a list")
            t = m.get("table")
            if not isinstance(t, dict) or not isinstance(t.get("hold_down"), list) or not num(t.get("max_material_thickness_in")):
                err(w, "table needs hold_down list and max_material_thickness_in number/null")
            if "estimated" in m and not isinstance(m["estimated"], dict):
                err(w, "estimated must be an object {field: note}")
            check_urls(w, m)
    return counts


def check_spindles():
    f = DATA / "machines" / "spindles.json"
    if not f.exists():
        return 0
    doc = json.loads(f.read_text())
    ids = set()
    for i, s in enumerate(doc.get("spindles", [])):
        w = f"spindles.json[{i}] {s.get('id')}"
        for k in ("id", "maker", "model", "type", "power_kw", "rpm_min", "rpm_max", "collet", "collet_min_in", "collet_max_in", "cooling", "sources"):
            if k not in s:
                err(w, f"missing {k}")
        if s.get("id") in ids:
            err(w, "duplicate id")
        ids.add(s.get("id"))
        check_rpm(w, s)
        for k in ("collet_min_in", "collet_max_in"):
            if not num(s.get(k)):
                err(w, f"{k} must be a number or null")
        check_urls(w, s)
    return len(ids)


BIT_FAMILIES = {"end-mill-up", "end-mill-down", "compression", "mortise-compression", "straight", "v-bit", "ball-nose",
                "tapered-ball", "bowl", "surfacing", "drill", "chamfer", "engraving", "roundover", "ogee", "cove", "keyhole",
                "dovetail", "t-slot", "o-flute", "chipbreaker", "thread-mill", "other"}
BIT_HELIX = {"up", "down", "compression", "straight", "o-flute"}
BIT_LENGTHS = ("diameter", "shank", "cut_length", "up_cut_length", "oal", "tip_diameter", "tip_radius", "corner_radius")
CL_MIN, CL_MAX = 0.0002, 0.05  # in/tooth


def chip_range(where, v, allow_equal=False):
    if not (isinstance(v, list) and len(v) == 2 and all(isinstance(x, (int, float)) and not isinstance(x, bool) for x in v)):
        err(where, f"chip load must be [min, max], got {v!r}")
        return
    lo, hi = v
    if not (lo < hi or (allow_equal and lo == hi)):
        err(where, f"chip load min {lo} must be < max {hi}")
    if lo < CL_MIN or hi > CL_MAX:
        err(where, f"chip load {v} outside {CL_MIN}-{CL_MAX} in/tooth")


def check_bits():
    """Bit database (data/bits/*.json + chipload.json); schema in data/bits/schema.md."""
    bits = DATA / "bits"
    try:
        cl = json.loads((bits / "chipload.json").read_text())
    except (OSError, ValueError) as e:
        err("chipload.json", f"unreadable: {e}")
        cl = {"groups": {}, "tables": {}}
    groups = set(cl.get("groups", {}))
    for key, t in cl.get("tables", {}).items():
        w = f"chipload.json {key}"
        if not isinstance(t.get("rows"), dict) or not t["rows"]:
            err(w, "needs rows")
            continue
        check_urls(w, t)
        for g, row in t["rows"].items():
            if g not in groups:
                err(w, f"unknown material group {g!r}")
            for d, v in row.items():
                if d != "all":
                    try:
                        float(d)
                    except ValueError:
                        err(w, f"diameter key {d!r} is not a number")
                chip_range(f"{w} {g} {d}", v, allow_equal=bool(t.get("single_values")))
    ids, n = set(), 0
    for f in sorted(bits.glob("*.json")):
        if f.name == "chipload.json":
            continue
        try:
            doc = json.loads(f.read_text())
        except ValueError as e:
            err(f.name, f"bad JSON: {e}")
            continue
        if not isinstance(doc.get("maker"), str) or not isinstance(doc.get("bits"), list):
            err(f.name, "needs maker (string) and bits (list)")
            continue
        for i, b in enumerate(doc["bits"]):
            n += 1
            w = f"{f.name}[{i}] {b.get('id')}"
            for k, typ in (("id", str), ("maker", str), ("part_number", str), ("name", str), ("family", str),
                           ("units", str), ("diameter", (int, float)), ("flutes", int), ("gcarve_tool", dict), ("sources", list)):
                if not isinstance(b.get(k), typ) or isinstance(b.get(k), bool):
                    err(w, f"missing or bad {k}")
            bid = b.get("id")
            if not isinstance(bid, str) or not re.fullmatch(r"[a-z0-9]+(-[a-z0-9]+)*", bid):
                err(w, "id must be lower-case kebab")
            elif bid in ids:
                err(w, "duplicate id")
            ids.add(bid)
            if b.get("family") not in BIT_FAMILIES:
                err(w, f"bad family {b.get('family')!r}")
            if b.get("units") not in ("in", "mm"):
                err(w, f"bad units {b.get('units')!r}")
            if b.get("helix") is not None and b["helix"] not in BIT_HELIX:
                err(w, f"bad helix {b['helix']!r}")
            if isinstance(b.get("flutes"), int) and b["flutes"] < 1:
                err(w, "flutes must be >= 1")
            for k in BIT_LENGTHS + ("angle_deg", "max_rpm"):
                v = b.get(k)
                if v is not None and (not isinstance(v, (int, float)) or isinstance(v, bool) or v < 0):
                    err(w, f"{k} must be a non-negative number")
            if isinstance(b.get("diameter"), (int, float)) and b["diameter"] <= 0:
                err(w, "diameter must be > 0")
            cut, oal = b.get("cut_length"), b.get("oal")
            if isinstance(cut, (int, float)) and isinstance(oal, (int, float)) and cut > oal:
                err(w, f"cut_length {cut} > oal {oal}")
            up = b.get("up_cut_length")
            if isinstance(up, (int, float)) and isinstance(cut, (int, float)) and up > cut:
                err(w, f"up_cut_length {up} > cut_length {cut}")
            rpm = b.get("rpm")
            if rpm is not None and not (isinstance(rpm, list) and len(rpm) == 2 and rpm[0] <= rpm[1]):
                err(w, f"rpm must be [min, max], got {rpm!r}")
            for g, v in (b.get("chip_load") or {}).items():
                if g not in groups:
                    err(w, f"chip_load: unknown material group {g!r}")
                chip_range(f"{w} chip_load.{g}", v)
            for j, p in enumerate(b.get("presets") or []):
                c = p.get("chip_load")
                if c is not None and not (isinstance(c, (int, float)) and CL_MIN <= c <= CL_MAX):
                    err(w, f"presets[{j}].chip_load {c!r} outside {CL_MIN}-{CL_MAX}")
                if p.get("group") not in groups | {"any"}:
                    err(w, f"presets[{j}]: unknown group {p.get('group')!r}")
            ref = b.get("chipload_ref")
            if ref is not None and ref not in cl.get("tables", {}):
                err(w, f"chipload_ref {ref!r} not in chipload.json")
            if b.get("estimated") is not None and not isinstance(b["estimated"], bool):
                err(w, "estimated must be bool")
            if b.get("estimated") and not b.get("notes"):
                err(w, "estimated records need notes")
            g = b.get("gcarve_tool") or {}
            if g.get("kind") not in ("EndMill", "VBit", "BallNose"):
                err(w, f"gcarve_tool.kind {g.get('kind')!r}")
            if not isinstance(g.get("diameter"), (int, float)) or g.get("diameter", 0) <= 0:
                err(w, "gcarve_tool.diameter must be > 0 (inches)")
            elif isinstance(b.get("diameter"), (int, float)):
                want = b["diameter"] / 25.4 if b.get("units") == "mm" else b["diameter"]
                if g.get("kind") != "BallNose" and abs(g["diameter"] - want) > 1e-3 * max(want, 1):
                    err(w, f"gcarve_tool.diameter {g['diameter']} != diameter in inches {want:.5f}")
            if g.get("kind") == "VBit" and not (isinstance(g.get("angle"), (int, float)) and 0 < g["angle"] < 180):
                err(w, "VBit needs angle in (0, 180)")
            if not isinstance(g.get("flutes"), int) or g["flutes"] < 1:
                err(w, "gcarve_tool.flutes must be int >= 1")
            check_urls(w, b)
    return n


def main():
    counts = check_machines()
    n_sp = check_spindles()
    n_bits = check_bits()
    for maker, n in sorted(counts.items(), key=lambda kv: kv[0].lower()):
        print(f"  {maker:28} {n}")
    print(f"machines: {sum(counts.values())} across {len(counts)} makers; spindles: {n_sp}; bits: {n_bits}")
    for e in errors:
        print("ERROR", e)
    print(f"{len(errors)} error(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
