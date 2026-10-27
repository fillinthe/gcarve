"""Lists every UI string the app can translate: the literals inside `i18n::t("…")` / `crate::i18n::t("…")` calls and
the const tables translated with `i18n::tr` (marked with a `// i18n` comment on the const line) in gcarve-app and
gcarve-ui, writes them to gcarve-ui/lang/en.json (keys only), and reports what each pack still lacks.
Run from rust/: python3 -I tools/i18n_keys.py
"""
import json, re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LIT = r'"((?:[^"\\]|\\.)*)"'
CALL = re.compile(r"i18n::t\(" + LIT + r"\)")
TABLE = re.compile(r"^[ \t]*(?:pub(?:\(crate\))? )?(?:const|static) [^\n]*// i18n\n?(.*?);", re.M | re.S)


def keys():
    out = []
    for crate in ("gcarve-app", "gcarve-ui"):
        for p in sorted((ROOT / crate / "src").rglob("*.rs")):
            src = p.read_text()
            at = src.find("\n#[cfg(test)]")
            if at > 0:
                src = src[:at]
            found = [json.loads(f'"{m.group(1)}"') for m in CALL.finditer(src)]
            for m in TABLE.finditer(src):
                found += [json.loads(f'"{x}"') for x in re.findall(LIT, m.group(0)) if re.search(r"[a-z]", x) and (x[:1].isupper() or " " in x)]
            out += [k for k in found if k not in out]
    return out


def main():
    ks = keys()
    (ROOT / "gcarve-ui/lang/en.json").write_text(json.dumps({k: "" for k in ks}, ensure_ascii=False, indent=0) + "\n")
    print(f"{len(ks)} strings, {sum(len(k) for k in ks)} characters")
    for f in sorted((ROOT / "gcarve-ui/lang").glob("*.json")):
        if f.name == "en.json":
            continue
        d = json.loads(f.read_text())
        miss = [k for k in ks if not d.get(k)]
        extra = [k for k in d if k not in ks]
        print(f"{f.stem}: {len(ks) - len(miss)} translated, {len(miss)} missing, {len(extra)} unused")


if __name__ == "__main__":
    main()
