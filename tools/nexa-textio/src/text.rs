//! Bytes to text and back: encodings (UTF-8, UTF-8 and UTF-16 with a byte
//! order mark, Windows-1252) and line endings (LF, CRLF, CR).

/// Windows-1252's 0x80..0x9F; the five holes keep their own code point
/// (as browsers do).
const CP1252_HIGH: [u16; 32] = [
    0x20ac, 0x0081, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008d, 0x017d, 0x008f,
    0x0090, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0x009d, 0x017e, 0x0178,
];

pub const ENCODINGS: [&str; 5] = ["utf-8", "utf-8-bom", "utf-16le", "utf-16be", "windows-1252"];

#[derive(Debug, PartialEq)]
pub struct Decoded {
    /// With "\n" line endings only.
    pub text: String,
    pub encoding: &'static str,
    /// "lf", "crlf" or "cr": the most used one.
    pub line_ending: &'static str,
    /// More than one kind of line ending was in the file.
    pub mixed: bool,
    /// Some bytes weren't valid and were replaced.
    pub replaced: bool,
}

fn utf16(b: &[u8], le: bool) -> (String, bool) {
    let units = b.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) });
    let mut replaced = b.len() % 2 == 1;
    let s = char::decode_utf16(units).map(|r| r.unwrap_or_else(|_| {
        replaced = true;
        '\u{fffd}'
    })).collect();
    (s, replaced)
}

fn cp1252(b: &[u8]) -> String {
    b.iter().map(|&c| match c {
        0x80..=0x9f => char::from_u32(CP1252_HIGH[(c - 0x80) as usize] as u32).unwrap_or('\u{fffd}'),
        _ => c as char,
    }).collect()
}

/// The line endings in `s`, as (most used, mixed), and `s` with "\n" only.
pub fn line_endings(s: &str) -> (&'static str, bool, String) {
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut start = 0;
    while i < b.len() {
        match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                out.push_str(&s[start..i]);
                out.push('\n');
                i += 2;
                start = i;
                continue;
            }
            b'\r' => {
                cr += 1;
                out.push_str(&s[start..i]);
                out.push('\n');
                start = i + 1;
            }
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    out.push_str(&s[start..]);
    let kinds = [crlf, lf, cr].iter().filter(|&&n| n > 0).count();
    let most = if crlf > lf && crlf >= cr { "crlf" } else if cr > lf && cr > crlf { "cr" } else { "lf" };
    (most, kinds > 1, out)
}

/// Text from a file's bytes, or why it can't be shown as text.
pub fn decode(b: &[u8]) -> Result<Decoded, String> {
    let (text, encoding, replaced) = if let Some(rest) = b.strip_prefix(b"\xef\xbb\xbf") {
        let s = String::from_utf8_lossy(rest);
        let r = matches!(s, std::borrow::Cow::Owned(_));
        (s.into_owned(), "utf-8-bom", r)
    } else if let Some(rest) = b.strip_prefix(b"\xff\xfe") {
        let (s, r) = utf16(rest, true);
        (s, "utf-16le", r)
    } else if let Some(rest) = b.strip_prefix(b"\xfe\xff") {
        let (s, r) = utf16(rest, false);
        (s, "utf-16be", r)
    } else {
        // A NUL byte: a program, an image, a database... not text.
        if b.contains(&0) {
            return Err("This isn't a text file (it looks like a program, a picture or another kind of file). Open it with the app made for it.".into());
        }
        match std::str::from_utf8(b) {
            Ok(s) => (s.to_string(), "utf-8", false),
            Err(_) => (cp1252(b), "windows-1252", false),
        }
    };
    let (line_ending, mixed, text) = line_endings(&text);
    Ok(Decoded { text, encoding, line_ending, mixed, replaced })
}

/// File bytes for `text` ("\n" line endings) in an encoding and line ending.
/// Fails, saying where, when a character can't be written in that encoding.
pub fn encode(text: &str, encoding: &str, line_ending: &str) -> Result<Vec<u8>, String> {
    let nl = match line_ending {
        "lf" => "\n",
        "crlf" => "\r\n",
        "cr" => "\r",
        other => return Err(format!("unknown line ending {other}")),
    };
    let text = if nl == "\n" { std::borrow::Cow::Borrowed(text) } else { std::borrow::Cow::Owned(text.replace('\n', nl)) };
    Ok(match encoding {
        "utf-8" => text.as_bytes().to_vec(),
        "utf-8-bom" => [&b"\xef\xbb\xbf"[..], text.as_bytes()].concat(),
        "utf-16le" => [vec![0xff, 0xfe], text.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat(),
        "utf-16be" => [vec![0xfe, 0xff], text.encode_utf16().flat_map(u16::to_be_bytes).collect()].concat(),
        "windows-1252" => {
            let mut out = Vec::with_capacity(text.len());
            for (line, l) in text.split(nl).enumerate() {
                if line > 0 {
                    out.extend(nl.as_bytes());
                }
                for c in l.chars() {
                    let u = c as u32;
                    let byte = if u < 0x80 || (0xa0..0x100).contains(&u) {
                        Some(u as u8)
                    } else {
                        CP1252_HIGH.iter().position(|&h| h as u32 == u).map(|i| 0x80 + i as u8)
                    };
                    match byte {
                        Some(x) => out.push(x),
                        None => return Err(format!("\"{c}\" (line {}) can't be saved as Windows-1252. Save as UTF-8 instead.", line + 1)),
                    }
                }
            }
            out
        }
        other => return Err(format!("unknown encoding {other}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_round_trip() {
        let text = "Café — “quoted” €5\nline two\n";
        for enc in ENCODINGS {
            for le in ["lf", "crlf", "cr"] {
                let bytes = encode(text, enc, le).unwrap();
                let d = decode(&bytes).unwrap();
                assert_eq!(d.text, text, "{enc} {le}");
                assert_eq!(d.line_ending, le, "{enc} {le}");
                // UTF-8 without a BOM and Windows-1252 look alike only for ASCII.
                assert_eq!(d.encoding, enc, "{enc} {le}");
                assert!(!d.mixed && !d.replaced);
            }
        }
    }

    #[test]
    fn guesses_and_refusals() {
        // Old Windows file: not UTF-8, so Windows-1252.
        let d = decode(b"caf\xe9 \x93hi\x94\r\n").unwrap();
        assert_eq!((d.text.as_str(), d.encoding, d.line_ending), ("café “hi”\n", "windows-1252", "crlf"));
        assert!(decode(b"\x7fELF\x02\x01\x01\0\0\0").unwrap_err().contains("isn't a text file"));
        let d = decode(b"a\r\nb\nc\r\nd\re").unwrap();
        assert_eq!((d.line_ending, d.mixed, d.text.as_str()), ("crlf", true, "a\nb\nc\nd\ne"));
        let d = decode(b"").unwrap();
        assert_eq!((d.text.as_str(), d.encoding, d.line_ending), ("", "utf-8", "lf"));
        // A broken UTF-16 file: shown, with the bad part replaced.
        let d = decode(b"\xff\xfea\0\x00\xd8b\0").unwrap();
        assert!(d.replaced && d.text.contains('\u{fffd}'));
        let e = encode("ok\nsnow ☃\n", "windows-1252", "lf").unwrap_err();
        assert!(e.contains("line 2") && e.contains("UTF-8"), "{e}");
        assert!(encode("x", "latin-9", "lf").is_err());
    }
}
