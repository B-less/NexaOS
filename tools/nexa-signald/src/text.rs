//! What apps send, made safe to show: markup tags removed, entities
//! decoded, control characters dropped, length capped.

/// At most `max` characters; a cut ends with "…".
pub fn cap(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out = out.trim_end().to_string();
    out.push('…');
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let n = name.strip_prefix('#')?;
            let code = match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(code).filter(|c| !c.is_control() || *c == '\n')
        }
    }
}

/// Body markup (what the spec allows: b, i, u, a, img) to plain text.
/// Something that isn't a tag (a "<" in "a < b") stays as it is.
pub fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if c == '<' {
            let after = &rest[1..];
            let looks_like_tag = after.chars().next().is_some_and(|n| n.is_ascii_alphabetic() || n == '/' || n == '!');
            if let (true, Some(end)) = (looks_like_tag, after.find('>')) {
                let tag = after[..end].trim_start_matches('/').to_ascii_lowercase();
                if tag.starts_with("br") || tag.starts_with('p') && (tag.len() == 1 || tag.as_bytes()[1] == b' ') {
                    out.push('\n');
                }
                rest = &after[end + 1..];
                continue;
            }
        } else if c == '&' {
            if let Some(end) = rest[1..].find(';').filter(|e| *e <= 10) {
                if let Some(ch) = entity(&rest[1..1 + end]) {
                    out.push(ch);
                    rest = &rest[end + 2..];
                    continue;
                }
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// One line of plain text (app names, summaries, action labels).
pub fn line(s: &str, max: usize) -> String {
    let flat: String = plain(s).chars().map(|c| if c.is_whitespace() { ' ' } else { c }).filter(|c| !c.is_control()).collect();
    cap(flat.split_whitespace().collect::<Vec<_>>().join(" ").as_str(), max)
}

/// A body: lines kept (at most two blank lines in a row become one), the
/// rest of the control characters dropped.
pub fn body(s: &str, max: usize) -> String {
    let p = plain(s).replace("\r\n", "\n").replace('\t', " ");
    let mut out = String::new();
    let mut blank = 0;
    for l in p.split('\n') {
        let l: String = l.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
        if l.is_empty() {
            blank += 1;
            if blank > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&l);
    }
    cap(out.trim_end(), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_and_entities() {
        assert_eq!(plain("<b>Bold</b> &amp; <i>it</i>"), "Bold & it");
        assert_eq!(plain("a < b and c > d"), "a < b and c > d");
        assert_eq!(plain("5 &lt; 6 &#233; &#x1F3B5; &bogus; &"), "5 < 6 é 🎵 &bogus; &");
        assert_eq!(plain("<a href=\"https://x\">link</a><br/>next"), "link\nnext");
        assert_eq!(plain("&#0;&#7;x"), "&#0;&#7;x", "control characters are never made from entities");
    }

    #[test]
    fn lines_and_bodies() {
        assert_eq!(line("  Two\twords\nhere \u{7} ", 50), "Two words here");
        assert_eq!(line("abcdefghij", 5), "abcd…");
        assert_eq!(body("one\n\n\n\ntwo\r\nthree\u{1b}[31m", 100), "one\n\ntwo\nthree[31m");
        assert_eq!(body("\n\nstart", 100), "start");
        assert_eq!(cap("héllo wörld", 6), "héllo…");
        assert_eq!(cap("short", 6), "short");
    }
}
