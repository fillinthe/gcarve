#!/usr/bin/env python3
"""Turn user-facing `format!("Message {x}…", …)` in the app into `i18n::tf("Message {x}…", &[("x", &x)])`, so the
language packs can translate them. Positional `{}` take a name from their argument (last identifier), format specs
(`{:.1}`) are applied before the substitution. Prints what it changes; `--write` writes the files.
Skips tests, literals that don't start with a capital letter or have no word in them, and the files in SKIP (file
contents, protocols, G-code)."""
import re, sys, glob

SKIP = ("setupsheet.rs", "files.rs", "vtdb.rs", "smoke", "control/", "main.rs", "recover.rs", "cloud.rs")
FMT = re.compile(r'format!\(\s*"')

def literal_end(s, i):  # i = index after the opening quote
    while s[i] != '"':
        i += 2 if s[i] == '\\' else 1
    return i

def args_end(s, i):  # split args after the literal up to the closing paren of format!
    depth, out, cur = 0, [], ''
    while True:
        c = s[i]
        if c in '([{': depth += 1
        elif c in ')]}':
            if depth == 0:
                if cur.strip(): out.append(cur.strip())
                return out, i
            depth -= 1
        elif c == '"':
            j = literal_end(s, i + 1); cur += s[i:j + 1]; i = j + 1; continue
        elif c == ',' and depth == 0:
            if cur.strip(): out.append(cur.strip())
            cur = ''; i += 1; continue
        cur += c; i += 1

def name_of(expr, used):
    e = expr
    while True:
        e2 = re.sub(r'\.\w+\((?:[^()]|\([^()]*\))*\)', '', e)
        if e2 == e: break
        e = e2
    ids = re.findall(r'[A-Za-z_][A-Za-z0-9_]*', e)
    ids = [x for x in ids if x not in ('self', 'len', 'to_string', 'as', 'f64', 'usize', 'i32', 'round', 'abs', 'unwrap_or', 'clone', 'display', 'i18n', 't', 'tr', 'fmt', 'theme', 'Some', 'None')] or ['v']
    n = 'extra' if ids[-1] in ('else', 'if', 'match', 'move', 'ref', 'type', 'fn', 'in') else ids[-1].lower()
    n = n if n == 'extra' else ids[-1].lower() if ids[-1] != ids[-1].upper() else ids[-1].lower()
    k, b = n, 2
    while k in used: k = f'{n}{b}'; b += 1
    used.add(k); return k

def convert(lit, args):
    used, pos, pairs, out, seen = set(), 0, [], '', {}
    named = {}
    for a in args:
        if re.match(r'^[a-z_][a-z0-9_]*\s*=', a) and not a.startswith('=='):
            k, v = a.split('=', 1); named[k.strip()] = v.strip()
    posargs = [a for a in args if not re.match(r'^[a-z_][a-z0-9_]*\s*=[^=]', a)]
    i = 0
    while i < len(lit):
        if lit.startswith('{{', i) or lit.startswith('}}', i):
            out += lit[i:i + 2]; i += 2; continue
        if lit[i] == '{':
            j = lit.index('}', i); inner = lit[i + 1:j]; i = j + 1
            nm, _, spec = inner.partition(':')
            if nm == '':
                expr = posargs[pos]; pos += 1
            elif nm.isdigit():
                expr = posargs[int(nm)]
            else:
                expr = named.get(nm, nm)
            if (expr, spec) in seen:
                out += '{' + seen[(expr, spec)] + '}'; continue
            key = name_of(expr, used) if nm == '' or nm.isdigit() or nm in named else (nm if nm not in used else name_of(nm, used))
            used.add(key); seen[(expr, spec)] = key
            val = f'&format!("{{:{spec}}}", {expr})' if spec else ('&' + (expr if re.fullmatch(r'[\w.]+', expr) else f'({expr})') if not expr.startswith('&') else expr)
            if re.fullmatch(r'[a-z_][a-z0-9_]*', expr) and key == expr and not spec:
                val = '&' + expr
            pairs.append(f'("{key}", {val})'); out += '{' + key + '}'
            continue
        out += lit[i]; i += 1
    return out, pairs

def want(lit):
    plain = re.sub(r'\{[^{}]*\}', '', lit)
    if not re.match(r'[A-Z¿]', lit.lstrip('\\n ')): return False
    if len(re.findall(r'[A-Za-z]{3,}', plain)) < 2: return False
    if re.search(r'HTTP|https?:|\.tlp|G-?code line|^[A-Z]{2,}\b', plain) and ' ' not in plain.strip(): return False
    return True

write = '--write' in sys.argv
n = 0
for f in sorted(glob.glob('gcarve-app/src/**/*.rs', recursive=True) + glob.glob('gcarve-ui/src/**/*.rs', recursive=True)):
    if any(k in f for k in SKIP): continue
    s = open(f).read(); cut = s.find('#[cfg(test)]'); body, tail = (s, '') if cut < 0 else (s[:cut], s[cut:])
    res, last = '', 0
    for m in FMT.finditer(body):
        if m.start() < last: continue
        e = literal_end(body, m.end()); lit = body[m.end():e]
        args, close = args_end(body, e + 1)
        if not want(lit) or '{:?}' in lit or '#?' in lit: continue
        line = body[:m.start()].rfind('\n'); ltxt = body[line + 1:body.find('\n', m.start())]
        if ltxt.lstrip().startswith('//') or 'report(' in ltxt or 'id:' in ltxt or 'store::' in ltxt or 'log' in ltxt.lower() and 'dialog' not in ltxt.lower(): continue
        try: new, pairs = convert(lit, args)
        except Exception as ex: print('SKIP', f, lit, ex); continue
        rep = f'i18n::tf("{new}", &[{", ".join(pairs)}])' if pairs else f'i18n::t("{new}").to_string()'
        res += body[last:m.start()] + rep; last = close + 1; n += 1
        print(f'{f}:{body[:m.start()].count(chr(10))+1}: {rep}')
    res += body[last:]
    if write and res != body:
        if 'use gcarve_ui::i18n' not in res and 'i18n::' in res and 'mod i18n' not in res and not f.startswith('gcarve-ui'):
            pass  # files already import i18n (checked by the compiler)
        open(f, 'w').write(res + tail)
print(n, 'changed' if write else 'to change')
