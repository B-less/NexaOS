//! Words and the features the model sees. Words are lowercased, letters
//! and digits only ("what's" -> "whats"). The intent model sees each word,
//! each pair of neighbouring words and the 3- to 5-letter pieces of every
//! word (so a typo still shares most pieces with the right word); the
//! tagger sees a word with its neighbours. Features are hashed into a fixed
//! number of buckets, so the model's size never grows with its vocabulary.

pub fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            cur.push(c);
        } else if c == '\'' || c == '\u{2019}' {
            // "what's" stays one word
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.truncate(40);
    out
}

/// FNV-1a: small, fast, the same everywhere.
pub fn hash(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h ^= 0x1f;
            h = h.wrapping_mul(0x100000001b3);
        }
        for b in p.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

fn word_key(w: &str) -> &str {
    if w.chars().all(|c| c.is_ascii_digit()) { "<num>" } else { w }
}

/// The intent model's features for a sentence (bucket numbers).
pub fn intent_features(ws: &[String], buckets: usize) -> Vec<usize> {
    let mut f = Vec::with_capacity(ws.len() * 12);
    let b = |parts: &[&str]| (hash(parts) % buckets as u64) as usize;
    for (i, w) in ws.iter().enumerate() {
        let k = word_key(w);
        f.push(b(&["w", k]));
        if i + 1 < ws.len() {
            f.push(b(&["b", k, word_key(&ws[i + 1])]));
        }
        let marked: Vec<char> = format!("<{k}>").chars().collect();
        for n in 3..=5 {
            if marked.len() < n {
                break;
            }
            for s in 0..=marked.len() - n {
                let piece: String = marked[s..s + n].iter().collect();
                f.push(b(&["c", &piece]));
            }
        }
    }
    if ws.is_empty() {
        f.push(b(&["<empty>"]));
    }
    f
}

pub const TAGS: usize = 3; // O, B (first word of the name), I (more of it)

/// The tagger's features for word i, given the sentence's intent and the
/// tag before it.
pub fn tag_features(ws: &[String], i: usize, intent: &str, prev: usize, buckets: usize) -> Vec<usize> {
    let b = |parts: &[&str]| (hash(parts) % buckets as u64) as usize;
    let at = |j: isize| -> &str {
        if j < 0 { "<s>" } else { ws.get(j as usize).map(|w| word_key(w)).unwrap_or("</s>") }
    };
    let i0 = i as isize;
    let w = at(i0);
    let prev_tag = ["O", "B", "I"][prev.min(2)];
    let mut f = vec![
        b(&["bias"]),
        b(&["w", w]),
        b(&["w-1", at(i0 - 1)]),
        b(&["w+1", at(i0 + 1)]),
        b(&["w-2", at(i0 - 2)]),
        b(&["w+2", at(i0 + 2)]),
        b(&["w-1w", at(i0 - 1), w]),
        b(&["ww+1", w, at(i0 + 1)]),
        b(&["it", intent]),
        b(&["itw", intent, w]),
        b(&["itw-1", intent, at(i0 - 1)]),
        b(&["pt", prev_tag]),
        b(&["ptit", prev_tag, intent]),
        b(&["ptw", prev_tag, w]),
        b(&["first", if i == 0 { "y" } else { "n" }]),
        b(&["last", if i + 1 == ws.len() { "y" } else { "n" }]),
    ];
    let marked: Vec<char> = format!("<{w}>").chars().collect();
    if marked.len() >= 3 {
        for s in 0..=marked.len() - 3 {
            let piece: String = marked[s..s + 3].iter().collect();
            f.push(b(&["c3", &piece]));
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_features() {
        assert_eq!(words("Hey Nexa, what's up?! Open VS-Code 2"), vec!["hey", "nexa", "whats", "up", "open", "vs", "code", "2"]);
        assert!(words("").is_empty() && words("?!").is_empty());
        let a = intent_features(&words("open files"), 1 << 15);
        let b = intent_features(&words("open fiels"), 1 << 15);
        let shared = a.iter().filter(|x| b.contains(x)).count();
        assert!(shared * 3 > a.len(), "a typo still shares many pieces ({shared} of {})", a.len());
        assert_eq!(intent_features(&words("set it to 40"), 1 << 15), intent_features(&words("set it to 70"), 1 << 15), "numbers look alike");
    }
}
