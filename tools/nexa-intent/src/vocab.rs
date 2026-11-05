//! Each intent's words: every word its patterns (data.rs) can say, the
//! fillers around them, and its numbers. A sentence that leans on words
//! its intent never uses is about something else ("turn down the
//! heating" isn't the volume), whatever the scores say: a closed-set
//! model can't know that by itself. Typos still count as known (one
//! letter off from a known word of five letters or more, two from eight).

use crate::data::{INTENTS, LAYERS, NUMBERS, PREFIXES, SUFFIXES};
use crate::features::words;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Small words that say nothing about what is asked.
const FILLER: &[&str] = &["the", "and", "for", "with", "just", "some", "really", "very", "bit", "little", "way", "too", "quite",
                          "again", "also", "now", "today", "its", "im", "this", "that", "there", "here", "what", "when", "how",
                          "all", "any", "one", "please", "thanks", "thank", "you", "your", "our", "from", "into", "onto", "about"];

fn sets() -> &'static Vec<HashSet<String>> {
    static S: OnceLock<Vec<HashSet<String>>> = OnceLock::new();
    S.get_or_init(|| {
        let mut common: HashSet<String> = HashSet::new();
        for t in PREFIXES.iter().chain(SUFFIXES) {
            common.extend(words(t));
        }
        INTENTS.iter().map(|(_, pats)| {
            let mut s = common.clone();
            for p in pats.iter() {
                let mut text = p.to_string();
                if p.contains("{NUM}") {
                    text.push(' ');
                    text.push_str(&NUMBERS.join(" "));
                }
                if p.contains("{LAYER}") {
                    text.push(' ');
                    text.push_str(&LAYERS.join(" "));
                }
                // ({APP} {FILE} {WS} {OUT} are names or chat: not this intent's own words)
                for name in ["{APP}", "{FILE}", "{WS}", "{OUT}"] {
                    text = text.replace(name, " ");
                }
                s.extend(words(&text.replace(['[', ']', '|'], " ")));
            }
            s
        }).collect()
    })
}

fn near(a: &str, b: &str) -> bool {
    let (x, y): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let n = x.len().min(y.len());
    if n < 5 || x.len().abs_diff(y.len()) > 2 {
        return false;
    }
    // optimal string alignment distance (a swap is one typo)
    let mut d = vec![vec![0usize; y.len() + 1]; x.len() + 1];
    for i in 0..=x.len() { d[i][0] = i; }
    for j in 0..=y.len() { d[0][j] = j; }
    for i in 1..=x.len() {
        for j in 1..=y.len() {
            let c = usize::from(x[i - 1] != y[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + c);
            if i > 1 && j > 1 && x[i - 1] == y[j - 2] && x[i - 2] == y[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[x.len()][y.len()] <= if n >= 8 { 2 } else { 1 }
}

/// The words of `ws` (outside the name `arg`) that intent's sentences never use.
pub fn unfamiliar(intent: &str, ws: &[String], arg: Option<&str>) -> Vec<String> {
    let Some(i) = INTENTS.iter().position(|(n, _)| *n == intent) else { return vec![] };
    let known = &sets()[i];
    let name: HashSet<String> = arg.map(words).unwrap_or_default().into_iter().collect();
    ws.iter()
        .filter(|w| w.chars().count() >= 3 && !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !FILLER.contains(&w.as_str()) && !name.contains(*w))
        .filter(|w| !known.contains(*w) && !known.iter().any(|k| near(w, k)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unfamiliar_words() {
        let w = |s: &str| words(s);
        assert!(unfamiliar("volume_down", &w("turn down the volume"), None).is_empty());
        assert_eq!(unfamiliar("volume_down", &w("turn down the heating"), None), vec!["heating"]);
        assert!(unfamiliar("brightness_down", &w("reduce the brightnes"), None).is_empty(), "a typo is still known");
        assert!(unfamiliar("launch", &w("open spotify"), Some("spotify")).is_empty(), "the name isn't judged");
        assert_eq!(unfamiliar("lock", &w("lock the front door"), None), vec!["front", "door"]);
    }
}
