"""Turns index-keyed draft files (scratchpad/app-i18n/<lang>-N.json: {"index": "translation"}, indexes into the
key snapshot) into the English-keyed pack gcarve-ui/lang/<lang>.json. Bare double quotes inside a draft value are
turned into curly quotes. Usage: python3 -I tools/i18n_draft.py <snapshot en.json> <draft dir> <lang>..."""
import json, re, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    snap, drafts, langs = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3:]
    en = list(json.loads(snap.read_text()))
    for lang in langs:
        out = {}
        for f in sorted(drafts.glob(f"{lang}-*.json")):
            for line in f.read_text().splitlines():
                m = re.match(r'^"(\d+)": "(.*)",?$', line)
                if not m:
                    continue
                i, inner = int(m.group(1)), m.group(2).replace('\\"', "\x01")
                if '"' in inner:
                    q = iter(["“", "”"] * 40)
                    inner = re.sub('"', lambda _: next(q), inner)
                v = json.loads('"' + inner.replace("\x01", '\\"') + '"')
                if v and i < len(en):
                    out[en[i]] = v
        (ROOT / "gcarve-ui/lang" / f"{lang}.json").write_text(json.dumps(out, ensure_ascii=False, indent=0) + "\n")
        print(lang, len(out))


if __name__ == "__main__":
    main()
