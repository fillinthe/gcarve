"""Publish the machine + bit databases for the web app: site/public/data/catalog.json (stdlib only).
The web build no longer embeds data/ (the desktop app still does, for offline use); it fetches this file once.
Research fields the app never reads (notes, bit sources) are left out to keep it small; data/ stays the master.
Run: python3 -I data/publish.py   (then site/deploy/deploy.sh)"""
import hashlib, json
from pathlib import Path

DATA = Path(__file__).resolve().parent
OUT = DATA.parent / "site/public/data/catalog.json"
DROP = {"source_summary", "retrieved", "notes", "website"}  # machines keep their sources (shown in the picker)


def strip(v, drop):
    if isinstance(v, dict):
        return {k: strip(x, drop) for k, x in v.items() if k not in drop}
    if isinstance(v, list):
        return [strip(x, drop) for x in v]
    return v


def files(sub, skip, drop=()):
    d = DROP | set(drop)
    return {p.name: strip(json.loads(p.read_text()), d) for p in sorted((DATA / sub).glob("*.json")) if p.name not in skip}


cat = {
    "machines": files("machines", {"spindles.json"}),
    "bits": files("bits", {"chipload.json"}, {"sources"}),
    "chipload": json.loads((DATA / "bits/chipload.json").read_text()),
}
body = json.dumps(cat, separators=(",", ":"), ensure_ascii=False)
cat = {"version": hashlib.sha1(body.encode()).hexdigest()[:12], **cat}
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(cat, separators=(",", ":"), ensure_ascii=False))
# the app asks for this tiny file (never cached) and then catalog.json?v=<version>, which caches for a year
OUT.with_name("catalog.ver").write_text(cat["version"])
n_m = sum(len(f.get("machines", [])) for f in cat["machines"].values())
n_b = sum(len(f.get("bits", [])) for f in cat["bits"].values())
print(f"{OUT.relative_to(DATA.parent)}: {n_m} machines, {n_b} bits, {OUT.stat().st_size // 1024} KB, version {cat['version']}")
