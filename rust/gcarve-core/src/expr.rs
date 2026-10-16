//! Length expressions (C++ `core/expr.*`): numbers with optional unit suffix (in, ft, mm, cm, m, ' and "),
//! `+ - * /` (and `x` between numbers) and parentheses, and named variables ("box-w"). Identifiers may contain `-`
//! when followed by a letter, so write "a - b" for subtraction.

fn ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn ident_char(s: &[u8], i: usize) -> bool {
    let c = s[i];
    c.is_ascii_alphanumeric() || c == b'_' || (c == b'-' && i + 1 < s.len() && ident_start(s[i + 1])) // "box-w"
}

fn c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// C strtod at the start of `s` (which begins with a digit or '.'+digit): decimal with optional exponent, or C99 hex
/// ("0x1A", "0x1.8p1"). Returns (value, bytes consumed).
fn strtod(s: &[u8]) -> (f64, usize) {
    let digits = |k: &mut usize, hex: bool| {
        let st = *k;
        while *k < s.len() && (if hex { s[*k].is_ascii_hexdigit() } else { s[*k].is_ascii_digit() }) {
            *k += 1;
        }
        *k - st
    };
    if s.len() > 2 && s[0] == b'0' && (s[1] | 0x20) == b'x' && (s[2].is_ascii_hexdigit() || (s[2] == b'.' && s.get(3).is_some_and(u8::is_ascii_hexdigit))) {
        let mut k = 2;
        let mut v = 0.0;
        while k < s.len() && s[k].is_ascii_hexdigit() {
            v = v * 16.0 + f64::from((s[k] as char).to_digit(16).unwrap());
            k += 1;
        }
        if k < s.len() && s[k] == b'.' {
            k += 1;
            let mut scale = 1.0 / 16.0;
            while k < s.len() && s[k].is_ascii_hexdigit() {
                v += f64::from((s[k] as char).to_digit(16).unwrap()) * scale;
                scale /= 16.0;
                k += 1;
            }
        }
        if k < s.len() && (s[k] | 0x20) == b'p' {
            let mut e = k + 1;
            if e < s.len() && (s[e] == b'+' || s[e] == b'-') {
                e += 1;
            }
            if digits(&mut e, false) > 0 {
                let p: i32 = std::str::from_utf8(&s[k + 1..e]).unwrap().parse().unwrap_or(if s[k + 1] == b'-' { i32::MIN } else { i32::MAX });
                v *= 2f64.powi(p);
                k = e;
            }
        }
        return (v, k);
    }
    let mut k = 0;
    digits(&mut k, false);
    if k < s.len() && s[k] == b'.' {
        k += 1;
        digits(&mut k, false);
    }
    if k < s.len() && (s[k] | 0x20) == b'e' {
        let mut e = k + 1;
        if e < s.len() && (s[e] == b'+' || s[e] == b'-') {
            e += 1;
        }
        if digits(&mut e, false) > 0 {
            k = e;
        }
    }
    (std::str::from_utf8(&s[..k]).unwrap().parse().unwrap_or(0.0), k)
}

fn unit_scale(u: &str) -> Option<f64> {
    match u.to_ascii_lowercase().as_str() {
        "in" | "inch" | "inches" => Some(1.0),
        "ft" | "feet" | "foot" => Some(12.0),
        "mm" => Some(1.0 / 25.4),
        "cm" => Some(1.0 / 2.54),
        "m" => Some(1.0 / 0.0254),
        _ => None,
    }
}

struct Parser<'a> {
    src: &'a str,
    s: &'a [u8],
    i: usize,
    bare: f64,
    lookup: &'a dyn Fn(&str) -> Option<(f64, bool)>,
    err: String,
    depth: i32,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && c_space(self.s[self.i]) {
            self.i += 1;
        }
    }
    fn fail<T>(&mut self, m: String) -> Option<T> {
        if self.err.is_empty() {
            self.err = m;
        }
        None
    }
    fn unexpected<T>(&mut self) -> Option<T> {
        let c = self.src[self.i..].chars().next().unwrap_or('?');
        self.fail(format!("unexpected \"{c}\""))
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// a plain number meeting a length in + / - is a length in the project unit
    fn as_length(&self, v: (f64, bool)) -> (f64, bool) {
        if v.1 { v } else { (v.0 * self.bare, true) }
    }
    fn expr(&mut self) -> Option<(f64, bool)> {
        let mut v = self.term()?;
        loop {
            self.ws();
            match self.peek() {
                Some(op @ (b'+' | b'-')) => {
                    self.i += 1;
                    let mut r = self.term()?;
                    if v.1 != r.1 {
                        v = self.as_length(v);
                        r = self.as_length(r);
                    }
                    v.0 = if op == b'+' { v.0 + r.0 } else { v.0 - r.0 };
                }
                _ => return Some(v),
            }
        }
    }
    fn term(&mut self) -> Option<(f64, bool)> {
        let mut v = self.factor()?;
        loop {
            self.ws();
            let Some(op @ (b'*' | b'/' | b'x' | b'X')) = self.peek() else { return Some(v) };
            // 'x' as multiply only between numbers ("2 x 4"), never as the start of an identifier
            if (op == b'x' || op == b'X') && self.i + 1 < self.s.len() && ident_char(self.s, self.i + 1) {
                return Some(v);
            }
            self.i += 1;
            let r = self.factor()?;
            if op == b'/' {
                if r.0.abs() < 1e-300 {
                    return self.fail("division by zero".into());
                }
                v = (v.0 / r.0, v.1 != r.1); // in / in = plain number
            } else {
                v = (v.0 * r.0, v.1 || r.1);
            }
        }
    }
    fn factor(&mut self) -> Option<(f64, bool)> {
        self.ws();
        if let Some(op @ (b'-' | b'+')) = self.peek() {
            self.i += 1;
            let v = self.factor()?;
            return Some(if op == b'-' { (-v.0, v.1) } else { v });
        }
        self.primary()
    }
    fn primary(&mut self) -> Option<(f64, bool)> {
        self.ws();
        let Some(c) = self.peek() else { return self.fail("unexpected end".into()) };
        if c == b'(' {
            self.i += 1;
            let v = self.expr()?;
            self.ws();
            if self.peek() != Some(b')') {
                return self.fail("missing )".into());
            }
            self.i += 1;
            return Some(v);
        }
        if c.is_ascii_digit() || (c == b'.' && self.s.get(self.i + 1).is_some_and(u8::is_ascii_digit)) {
            let (mut v, len) = strtod(&self.s[self.i..]);
            self.i += len;
            // unit suffix
            let save = self.i;
            self.ws();
            let mut j = self.i;
            while j < self.s.len() && self.s[j].is_ascii_alphabetic() {
                j += 1;
            }
            if let Some(k) = unit_scale(&self.src[self.i..j]) {
                v *= k;
                self.i = j;
            } else if self.peek() == Some(b'"') {
                self.i += 1;
            } else if self.peek() == Some(b'\'') {
                v *= 12.0;
                self.i += 1;
            } else {
                self.i = save;
                return Some((v, false)); // plain number
            }
            return Some((v, true));
        }
        if ident_start(c) {
            let mut j = self.i;
            while j < self.s.len() && ident_char(self.s, j) {
                j += 1;
            }
            let name = &self.src[self.i..j];
            self.i = j;
            self.depth += 1;
            if self.depth > 50 {
                return self.fail("variables reference each other in a loop".into());
            }
            let Some(v) = (self.lookup)(name) else { return self.fail(format!("unknown variable \"{name}\"")) };
            self.depth -= 1;
            return Some(v);
        }
        self.unexpected()
    }
}

/// `bare_unit_in`: inches per unit for numbers without a suffix (1 = in, 12 = ft, 1/25.4 = mm). `lookup` resolves a
/// variable name to inches. Result is in inches; Err holds the message. Plain numbers are unitless factors
/// ("2in * 3" = 6 in, "6in / 2in" = 3); a unitless result becomes a length in the project unit.
pub fn eval_expr(text: &str, bare_unit_in: f64, lookup: &dyn Fn(&str) -> Option<f64>) -> Result<f64, String> {
    let (v, len) = eval_expr_dim(text, bare_unit_in, &|n| lookup(n).map(|v| (v, true)))?;
    Ok(if len { v } else { v * bare_unit_in })
}

/// Like `eval_expr` but keeps the dimension: lookup and result are (value, is_length); lengths in inches.
pub fn eval_expr_dim(text: &str, bare_unit_in: f64, lookup: &dyn Fn(&str) -> Option<(f64, bool)>) -> Result<(f64, bool), String> {
    let mut p = Parser { src: text, s: text.as_bytes(), i: 0, bare: bare_unit_in, lookup, err: String::new(), depth: 0 };
    let mut r = p.expr();
    p.ws();
    if r.is_some() && p.i < text.len() {
        r = p.unexpected();
    }
    if r.is_some_and(|v| !v.0.is_finite()) {
        r = p.fail("not a number".into());
    }
    if p.i == 0 && text.bytes().all(|c| c == b' ' || c == b'\t') {
        r = p.fail("empty".into());
    }
    r.ok_or(p.err)
}

/// variable names used
pub fn identifiers(text: &str) -> Vec<String> {
    let s = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i].is_ascii_digit() || s[i] == b'.' {
            // skip numbers and their unit suffix
            while i < s.len() && (s[i].is_ascii_digit() || s[i] == b'.') {
                i += 1;
            }
            let mut k = i;
            while k < s.len() && s[k] == b' ' {
                k += 1;
            }
            let mut j = k;
            while j < s.len() && s[j].is_ascii_alphabetic() {
                j += 1;
            }
            if unit_scale(&text[k..j]).is_some() {
                i = j;
            }
            continue;
        }
        if ident_start(s[i]) {
            let mut j = i;
            while j < s.len() && ident_char(s, j) {
                j += 1;
            }
            let name = &text[i..j];
            if name != "x" && name != "X" {
                out.push(name.to_string());
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

pub fn rename_ident(text: &str, from: &str, to: &str) -> String {
    let s = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < s.len() {
        if ident_start(s[i]) && (i == 0 || !s[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < s.len() && ident_char(s, j) {
                j += 1;
            }
            let name = &text[i..j];
            out += if name == from { to } else { name };
            i = j;
        } else {
            let c = text[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strtod;
    #[test]
    fn strtod_like_c() {
        assert_eq!(strtod(b"1.5e2x"), (150.0, 5));
        assert_eq!(strtod(b"2e"), (2.0, 1));
        assert_eq!(strtod(b"0x1A "), (26.0, 4));
        assert_eq!(strtod(b"0x1.8p1"), (3.0, 7));
        assert_eq!(strtod(b"0xg"), (0.0, 1));
        assert_eq!(strtod(b".5.5"), (0.5, 2));
    }
}
