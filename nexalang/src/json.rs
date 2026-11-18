//! A small JSON reader and writer for `check --json` and the language
//! server (0.6). nexac has no dependencies, so this is its own; it's only
//! used for messages between nexac and editors, never for NexaLang values.
//!
//! The reader is strict (RFC 8259), refuses nesting deeper than 64 and
//! never panics on any input (tested by fuzzing).

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

const MAX_DEPTH: u32 = 64;

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    /// `a.b.c` through objects.
    pub fn path(&self, keys: &[&str]) -> Option<&Json> {
        keys.iter().try_fold(self, |j, k| j.get(k))
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    /// A whole number that fits in a u32 (LSP lines and characters).
    pub fn as_u32(&self) -> Option<u32> {
        let n = self.as_f64()?;
        if n >= 0.0 && n <= u32::MAX as f64 && n.fract() == 0.0 {
            Some(n as u32)
        } else {
            None
        }
    }
    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }
    pub fn num(n: impl Into<f64>) -> Json {
        Json::Num(n.into())
    }
    pub fn obj(fields: Vec<(&str, Json)>) -> Json {
        Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    pub fn opt_str(s: &Option<String>) -> Json {
        match s {
            Some(s) => Json::Str(s.clone()),
            None => Json::Null,
        }
    }

    pub fn write(&self) -> String {
        let mut out = String::new();
        self.write_to(&mut out);
        out
    }
    fn write_to(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if !n.is_finite() {
                    out.push_str("null")
                } else if n.fract() == 0.0 && n.abs() < 1e15 {
                    out.push_str(&format!("{}", *n as i64))
                } else {
                    out.push_str(&format!("{}", n))
                }
            }
            Json::Str(s) => write_str(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write_to(out);
                }
                out.push(']');
            }
            Json::Obj(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    v.write_to(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn parse(text: &str) -> Result<Json, String> {
    let mut p = P { b: text.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err(format!("unexpected text after the value at byte {}", p.i));
    }
    Ok(v)
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{} at byte {}", what, self.i))
    }
    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            self.err("unknown word")
        }
    }
    fn value(&mut self, depth: u32) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return self.err("nested too deeply");
        }
        match self.b.get(self.i) {
            None => self.err("unexpected end"),
            Some(b'n') => self.lit("null", Json::Null),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return self.err("expected , or ]"),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(fields));
                }
                loop {
                    self.ws();
                    if self.b.get(self.i) != Some(&b'"') {
                        return self.err("expected a key");
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') {
                        return self.err("expected :");
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value(depth + 1)?;
                    fields.push((k, v));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(fields));
                        }
                        _ => return self.err("expected , or }"),
                    }
                }
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => self.err("unexpected character"),
        }
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        let digits = |p: &mut P| {
            let s = p.i;
            while p.i < p.b.len() && p.b[p.i].is_ascii_digit() {
                p.i += 1;
            }
            p.i - s
        };
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        if self.b.get(self.i) == Some(&b'0') {
            self.i += 1;
        } else if digits(self) == 0 {
            return self.err("bad number");
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            if digits(self) == 0 {
                return self.err("bad number");
            }
        }
        if matches!(self.b.get(self.i), Some(b'e') | Some(b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            if digits(self) == 0 {
                return self.err("bad number");
            }
        }
        // the bytes are ASCII digits and signs, so this is valid UTF-8
        let s = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| "bad number".to_string())?;
        match s.parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(Json::Num(n)),
            _ => self.err("number out of range"),
        }
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let Some(h) = self.b.get(self.i..self.i + 4) else { return self.err("short \\u escape") };
        let Ok(h) = std::str::from_utf8(h) else { return self.err("bad \\u escape") };
        let Ok(v) = u32::from_str_radix(h, 16) else { return self.err("bad \\u escape") };
        if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return self.err("bad \\u escape");
        }
        self.i += 4;
        Ok(v)
    }
    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // "
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&c) = self.b.get(self.i) else { return self.err("string never ends") };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.b.get(self.i) else { return self.err("string never ends") };
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if self.b.get(self.i) == Some(&b'\\') && self.b.get(self.i + 1) == Some(&b'u') {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..0xE000).contains(&lo) {
                                        return self.err("bad surrogate pair");
                                    }
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else {
                                    return self.err("lone surrogate");
                                }
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return self.err("lone surrogate");
                            } else {
                                hi
                            };
                            match char::from_u32(cp) {
                                Some(c) => c,
                                None => return self.err("bad \\u escape"),
                            }
                        }
                        _ => return self.err("unknown escape"),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c if c < 0x20 => return self.err("control character in string"),
                c => out.push(c),
            }
        }
        // the input was a &str and escapes add whole characters, so this
        // can only fail if a string was cut inside a letter, which can't
        // happen at a `"`; still, refuse rather than panic
        String::from_utf8(out).map_err(|_| "invalid UTF-8 in string".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let src = r#"{"a":[1,2.5,-3e2,true,false,null],"b":"x\"\\\n\u00e9\ud83d\ude00","c":{}}"#;
        let v = parse(src).unwrap();
        assert_eq!(v.get("b").unwrap().as_str().unwrap(), "x\"\\\né😀");
        assert_eq!(parse(&v.write()).unwrap(), v);
        assert_eq!(Json::str("\u{1}\u{2028}").write(), "\"\\u0001\\u2028\"");
    }

    #[test]
    fn refuses_bad_input_without_panicking() {
        for bad in [
            "", "{", "[1,]", "{\"a\"}", "01", "1.", "-", "\"\\ud800\"", "\"\\udc00x\"", "\"\\u12\"", "\"a\nb\"", "[1] x",
            "1e999", "nul", "\"\\x\"", "{\"a\":1,}",
        ] {
            assert!(parse(bad).is_err(), "{:?}", bad);
        }
        let deep = "[".repeat(100_000);
        assert!(parse(&deep).is_err());
        // every prefix of a valid document, and every byte flipped
        let doc = r#"{"jsonrpc":"2.0","id":1,"params":{"text":"fn main() {\n}\n","n":[1,2]}}"#;
        for i in 0..doc.len() {
            let _ = parse(&doc[..i]);
            for b in [b'"', b'\\', b'{', b']', b'0', b' '] {
                let mut v = doc.as_bytes().to_vec();
                v[i] = b;
                if let Ok(s) = std::str::from_utf8(&v) {
                    let _ = parse(s);
                }
            }
        }
    }
}
