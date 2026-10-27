"""Wraps the UI string literals of gcarve-app and gcarve-ui in `t("…")` (gcarve-ui/src/i18n.rs) so language packs can
translate them. Idempotent: a literal already inside t(…) is left alone. Run from rust/: python3 -I tools/i18n_wrap.py
Then `python3 -I tools/i18n_keys.py` lists the keys and what each pack still lacks.

What gets wrapped: the first string literal of the calls in CALLS when it reads like UI text (starts with a capital
letter or has a space, and has letters); ids ("clock", "tool:select"), units ("in") and format! strings are left.
"""
import re, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LIT = r'"((?:[^"\\]|\\.)*)"'
# call text up to the literal (a regex), for both crates
CALLS = [
    r"ui\.(?:label|button|small_button|link|menu_button|heading|strong|small|weak)\(",
    r"\.(?:tip|on_hover_text|hint_text|selected_text|text)\(",
    r"(?:RichText|Dialog|Window|Button|Checkbox)::new\(",
    r"\bhint\(ui, ", r"\bsubmenu\(ui, ", r"\bsection\(ui, ", r"\brow\(ui, ", r"\bunit_label\(",
    r"widgets::checkbox\(ui, [^,()]+, ", r"\bcheckbox\(ui, [^,()]+, ", r"ui\.checkbox\([^,()]+, ",
    r"\bitem\(ui, [^,()]+, ", r"\bmenu_item\(ui, ", r"selectable_value\([^,()]+, [^,()]+, ", r"radio_value\([^,()]+, [^,()]+, ", r"radio_value\(ui, [^,()]+, [^,()]+, ",
    r"\.add_filter\(", r"ComboBox::from_label\(", r"Label::new\(",
    # a lone literal in braces (if / else arms), a match arm, a status line, a string made from a literal
    r"\{ ", r"=> ", r"(?:status|message) = ", r"\*host\.message = ",
]
# a literal made into a String / the element of a tuple or array (table rows inside functions; const tables cannot
# call t and are translated at their use sites with i18n::tr)
TAIL = re.compile(LIT + r"(\.into\(\)|\.to_string\(\))")
TUPLE = re.compile(r"(\(|, )" + LIT + r"(?=,|\))")
PAT = re.compile("(" + "|".join(CALLS) + ")" + LIT)
# shell.rs's icon items: item(ui, sh, id, icon, "Name", "Hover text", …): both literals are text
ICON_ITEM = re.compile(r'(\bitem\(ui, sh, "[^"]*", "[^"]*", )' + LIT + r"(, )" + LIT)


def ui_text(s):
    return re.search(r"[a-z]", s) and (s[:1].isupper() or " " in s) and "{" not in s


def unwrap_consts(src, call):
    """const / static items cannot call t: their literals go back to plain strings (translated at the use site)"""
    out, i = [], 0
    for m in re.finditer(r"^[ \t]*(?:pub(?:\(crate\))? )?(?:const|static) ", src, re.M):
        if m.start() < i:
            continue
        # the item ends at the first ; outside strings and brackets
        depth, j, in_str = 0, m.end(), False
        while j < len(src):
            c = src[j]
            if in_str:
                if c == "\\":
                    j += 1
                elif c == '"':
                    in_str = False
            elif c == '"':
                in_str = True
            elif c in "([{":
                depth += 1
            elif c in ")]}":
                depth -= 1
            elif c == ";" and depth == 0:
                break
            j += 1
        out.append(src[i:m.start()])
        out.append(src[m.start():j].replace(f'{call}("', '"').replace('")', '"') if False else re.sub(re.escape(call) + r'\(("(?:[^"\\]|\\.)*")\)', r"\1", src[m.start():j]))
        i = j
    out.append(src[i:])
    return "".join(out)


def wrap(src, call):
    """call = the t() path to use in this file; the tests at the end of a file and raw strings (inline JavaScript,
    `r#"…"#`) are left alone"""
    at = src.find("\n#[cfg(test)]")
    if at > 0:
        return wrap(src[:at], call) + src[at:]
    m = re.search(r'r#"', src)
    if m:
        end = src.find('"#', m.end())
        if end > 0:
            return wrap(src[:m.start()], call) + src[m.start():end + 2] + wrap(src[end + 2:], call)
    def one(m):
        head, s = m.group(1), m.group(2)
        if not ui_text(s) or head.endswith("i18n::t("):
            return m.group(0)
        return f'{head}{call}("{s}")'
    # comment lines are not code
    out = "\n".join(line if line.lstrip().startswith("//") else PAT.sub(one, line) for line in src.split("\n"))
    out = TAIL.sub(lambda m: f'{call}("{m.group(1)}"){m.group(2)}' if ui_text(m.group(1)) else m.group(0), out)
    # tuple / array elements, not on lines with a macro call (format! / assert! / env! need plain literals)
    lines = out.split("\n")
    for k, line in enumerate(lines):
        if "!(" not in line and not line.lstrip().startswith("//") and not re.match(r"\s*\(.*\) =>", line):  # not a macro's literal, not a match pattern
            lines[k] = TUPLE.sub(lambda m: f'{m.group(1)}{call}("{m.group(2)}")' if ui_text(m.group(2)) and not line[max(0, m.start() - 8):m.start() + 1].endswith("t(") else m.group(0), line)
    out = "\n".join(lines)
    out = ICON_ITEM.sub(lambda m: f'{m.group(1)}{call}("{m.group(2)}"){m.group(3)}{call}("{m.group(4)}")' if ui_text(m.group(2)) else m.group(0), out)
    return unwrap_consts(out, call)


def main():
    n_files = n_wrapped = 0
    # the module is imported, not `t` itself: `t` is a common local name in this code base
    for crate, call, imp in [("gcarve-app", "i18n::t", "#[allow(unused_imports)] // a file may only use it under one cfg\nuse gcarve_ui::i18n;"), ("gcarve-ui", "crate::i18n::t", "")]:
        for p in sorted((ROOT / crate / "src").rglob("*.rs")):
            if p.name == "i18n.rs":
                continue
            src = p.read_text()
            out = wrap(src, call)
            if out == src or f'{call}("' not in out:
                continue
            if imp and imp not in out:
                # after the module doc comment and before the first use / item
                m = re.search(r"^(?!//!|#!\[)(?=\S)", out, re.M)
                at = m.start() if m else 0
                out = out[:at] + imp + "\n" + out[at:]
            p.write_text(out)
            n_files += 1
            n_wrapped += out.count(f'{call}("') - src.count(f'{call}("')
    print(f"{n_wrapped} literals wrapped in {n_files} files")


if __name__ == "__main__":
    sys.exit(main())
