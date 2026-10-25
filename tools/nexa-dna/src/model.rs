//! Interface DNA: the traits that make up how an app looks and is laid out,
//! what each app says it supports (its contract), and the checks that keep
//! a combination sensible. Everything here is plain data and pure checks.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const VERSION: u32 = 1;

/// The structural families an app may offer (only those that make sense
/// for it: Files offers matrix, river and focus; Settings constellation and
/// matrix).
pub const LAYOUTS: &[(&str, &str)] = &[
    ("constellation", "Constellation: connected objects"),
    ("river", "River: things in the order they happened"),
    ("origami", "Origami: regions that fold away"),
    ("matrix", "Matrix: a dense grid"),
    ("portal", "Portal: zoom in step by step"),
    ("habitat", "Habitat: grouped by activity"),
    ("thread", "Thread: follow cause and effect"),
    ("focus", "Focus: one task, nothing else"),
];

/// Every trait, its allowed values (empty = free-form, checked separately)
/// and its default.
pub const TRAITS: &[(&str, &[&str], &str)] = &[
    ("layout", &[], ""),
    ("navigation", &["sidebar", "contextual", "command"], "sidebar"),
    ("density", &["compact", "comfortable", "spacious"], "comfortable"),
    ("geometry", &["theme", "rounded", "angular", "pill"], "theme"),
    ("appearance", &["theme", "dark", "light"], "theme"),
    ("accent", &[], "theme"),
    ("typography", &["theme", "inter", "system", "mono"], "theme"),
    ("borders", &["none", "subtle", "strong"], "subtle"),
    ("motion", &["none", "minimal", "standard"], "standard"),
    ("contrast", &["normal", "high"], "normal"),
    ("text_scale", &[], "1.0"),
];

/// What an app says it can do. First-party apps ship one in
/// /usr/share/nexa/interface/<app>.json; an app without one only follows
/// the system theme (Interface Studio says so).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Contract {
    pub app: String,
    pub name: String,
    #[serde(default)]
    pub layouts: Vec<String>,
    #[serde(default)]
    pub default_layout: Option<String>,
    #[serde(default)]
    pub navigation: Vec<String>,
    /// Which of the style traits it honours (all of them for apps built on
    /// NexaOS's shared title bar and palette).
    #[serde(default)]
    pub traits: Vec<String>,
}

pub type Dna = Map<String, Value>;

#[derive(Debug, PartialEq, Serialize)]
pub struct Problem {
    pub trait_name: String,
    pub why: String,
}

fn problem(t: &str, why: impl Into<String>) -> Problem {
    Problem { trait_name: t.into(), why: why.into() }
}

pub fn defaults() -> Dna {
    TRAITS.iter().filter(|(k, _, _)| *k != "layout").map(|(k, _, d)| (k.to_string(), Value::String(d.to_string()))).collect()
}

fn hex_rgb(s: &str) -> Option<(f64, f64, f64)> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let p = |i| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
    Some((p(0)?, p(2)?, p(4)?))
}

/// WCAG relative luminance and contrast ratio.
pub fn contrast(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let lin = |c: f64| if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let lum = |(r, g, b): (f64, f64, f64)| 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

pub fn accent_rgb(s: &str) -> Option<(f64, f64, f64)> {
    hex_rgb(s)
}

/// Checks a DNA against an app's contract (None = the global profile,
/// which has no layout). Returns every problem, each with a reason a
/// person can act on; an empty list means it can be applied.
pub fn validate(dna: &Dna, contract: Option<&Contract>) -> Vec<Problem> {
    let mut out = Vec::new();
    for (key, value) in dna {
        if key == "profile" || key == "version" {
            continue;
        }
        let Some((_, allowed, _)) = TRAITS.iter().find(|(k, _, _)| k == key) else {
            out.push(problem(key, format!("\u{201c}{key}\u{201d} isn't something NexaOS interfaces have")));
            continue;
        };
        let Some(v) = value.as_str().map(str::to_string).or_else(|| value.as_f64().map(|f| f.to_string())) else {
            out.push(problem(key, "needs a word or a number"));
            continue;
        };
        if !allowed.is_empty() && !allowed.contains(&v.as_str()) {
            out.push(problem(key, format!("\u{201c}{v}\u{201d} isn't one of: {}", allowed.join(", "))));
        }
        match key.as_str() {
            "layout" => match contract {
                None => out.push(problem(key, "a layout belongs to one app, not to all of them")),
                Some(_) if !LAYOUTS.iter().any(|(l, _)| *l == v) => out.push(problem(key, format!("there's no \u{201c}{v}\u{201d} layout"))),
                Some(c) if !c.layouts.contains(&v) => out.push(problem(key, format!(
                    "{} doesn't have a {v} layout; it has {}", c.name, if c.layouts.is_empty() { "only one".to_string() } else { c.layouts.join(", ") }))),
                _ => {}
            },
            "accent" if v != "theme" && hex_rgb(&v).is_none() => out.push(problem(key, "an accent is \u{201c}theme\u{201d} or a colour like #6B5CFF")),
            "text_scale" => match v.parse::<f64>() {
                Ok(s) if (0.8..=1.6).contains(&s) => {}
                _ => out.push(problem(key, "text size goes from 0.8 (smaller) to 1.6 (much larger)")),
            },
            "navigation" => {
                if let Some(c) = contract {
                    if !c.navigation.is_empty() && !c.navigation.contains(&v) {
                        out.push(problem(key, format!("{} offers {} navigation", c.name, c.navigation.join(" or "))));
                    }
                }
            }
            _ => {}
        }
        if let Some(c) = contract {
            if !["layout", "navigation"].contains(&key.as_str()) && !c.traits.iter().any(|t| t == key) {
                out.push(problem(key, format!("{} can't change its {}", c.name, key.replace('_', " "))));
            }
        }
    }
    // Combinations that don't make sense together.
    let get = |k: &str| dna.get(k).and_then(Value::as_str);
    if get("layout") == Some("focus") && get("navigation") == Some("sidebar") {
        out.push(problem("navigation", "Focus hides the sidebar: choose contextual or command navigation"));
    }
    if get("contrast") == Some("high") {
        if let Some(rgb) = get("accent").and_then(hex_rgb) {
            let dark = (0.03, 0.05, 0.12);
            let light = (0.96, 0.97, 1.0);
            let against = if get("appearance") == Some("light") { light } else { dark };
            if contrast(rgb, against) < 4.5 {
                out.push(problem("accent", "with high contrast, this accent is too faint against the background (needs 4.5:1)"));
            }
        }
    }
    if get("density") == Some("compact") && dna.get("text_scale").and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or(v.as_f64())).is_some_and(|s| s >= 1.4) {
        out.push(problem("density", "very large text doesn't fit compact spacing: choose comfortable or spacious"));
    }
    out
}

/// The DNA an app uses: defaults, then the global profile, then the app's
/// own choices (only traits it supports), then its default layout if none.
pub fn resolve(global: &Dna, app: &Dna, contract: Option<&Contract>) -> Dna {
    let mut out = defaults();
    for (k, v) in global.iter().filter(|(k, _)| *k != "layout" && *k != "profile") {
        out.insert(k.clone(), v.clone());
    }
    if let Some(c) = contract {
        for (k, v) in app {
            let ok = match k.as_str() {
                "layout" => v.as_str().is_some_and(|l| c.layouts.iter().any(|x| x == l)),
                "navigation" => true,
                "profile" | "version" => false,
                other => c.traits.iter().any(|t| t == other),
            };
            if ok {
                out.insert(k.clone(), v.clone());
            }
        }
        if !out.contains_key("layout") {
            if let Some(l) = c.default_layout.clone().or_else(|| c.layouts.first().cloned()) {
                out.insert("layout".into(), Value::String(l));
            }
        }
    }
    out
}

/// Older files brought up to date. Version 0 (unversioned) called
/// "motion": "reduced" what is now "minimal", and "size" what is now
/// "text_scale".
pub fn migrate(mut v: Value) -> Value {
    let version = v.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version == 0 {
        if let Some(m) = v.as_object_mut() {
            if m.get("motion").and_then(Value::as_str) == Some("reduced") {
                m.insert("motion".into(), Value::String("minimal".into()));
            }
            if let Some(size) = m.remove("size") {
                m.insert("text_scale".into(), size);
            }
        }
    }
    if let Some(m) = v.as_object_mut() {
        m.insert("version".into(), Value::from(VERSION));
    }
    v
}

pub fn presets() -> Vec<(&'static str, &'static str, Dna)> {
    let make = |pairs: &[(&str, &str)]| -> Dna { pairs.iter().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect() };
    vec![
        ("nexa", "NexaOS: the default", make(&[])),
        ("calm", "Calm: fewer lines, no motion, more space",
         make(&[("density", "spacious"), ("borders", "none"), ("motion", "none"), ("geometry", "rounded")])),
        ("dense", "Dense: more on screen, for work",
         make(&[("density", "compact"), ("geometry", "angular"), ("motion", "minimal")])),
        ("clear", "Clear: high contrast, larger text",
         make(&[("contrast", "high"), ("text_scale", "1.25"), ("borders", "strong"), ("motion", "minimal")])),
        ("developer", "Developer: compact, angular, monospaced",
         make(&[("density", "compact"), ("geometry", "angular"), ("typography", "mono"), ("accent", "#6B5CFF"), ("motion", "minimal")])),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn files() -> Contract {
        Contract { app: "nexa-files".into(), name: "Files".into(), layouts: vec!["matrix".into(), "river".into(), "focus".into()],
                   default_layout: Some("matrix".into()), navigation: vec!["sidebar".into(), "command".into()],
                   traits: TRAITS.iter().map(|(k, _, _)| k.to_string()).filter(|k| k != "layout" && k != "navigation").collect() }
    }

    fn dna(v: Value) -> Dna {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn the_example_profile_is_accepted_where_supported() {
        let d = dna(json!({"profile": "My Development Interface", "layout": "matrix", "navigation": "command", "density": "compact",
                           "geometry": "angular", "appearance": "dark", "accent": "#6B5CFF", "motion": "minimal"}));
        assert_eq!(validate(&d, Some(&files())), vec![]);
    }

    #[test]
    fn unsupported_and_nonsense_combinations_are_explained() {
        let p = validate(&dna(json!({"layout": "constellation"})), Some(&files()));
        assert!(p[0].why.contains("doesn't have a constellation layout") && p[0].why.contains("matrix, river, focus"), "{p:?}");
        let p = validate(&dna(json!({"layout": "focus", "navigation": "sidebar"})), Some(&files()));
        assert!(p.iter().any(|x| x.why.starts_with("Focus hides the sidebar")));
        let p = validate(&dna(json!({"accent": "blue", "text_scale": "3", "sparkle": "yes"})), None);
        assert_eq!(p.len(), 3, "{p:?}");
        let p = validate(&dna(json!({"contrast": "high", "accent": "#202040"})), None);
        assert!(p.iter().any(|x| x.why.contains("too faint")), "a dark accent on dark surfaces in high contrast");
        assert!(validate(&dna(json!({"contrast": "high", "accent": "#FFD166"})), None).is_empty());
        let p = validate(&dna(json!({"density": "compact", "text_scale": "1.5"})), None);
        assert!(p.iter().any(|x| x.trait_name == "density"));
        assert!(validate(&dna(json!({"layout": "matrix"})), None)[0].why.contains("one app"), "no layout in the global profile");
    }

    #[test]
    fn an_app_that_cant_change_something_says_so() {
        let mut c = files();
        c.traits = vec!["accent".into()];
        let p = validate(&dna(json!({"density": "compact"})), Some(&c));
        assert_eq!(p[0].why, "Files can't change its density");
    }

    #[test]
    fn resolving_layers_global_then_app() {
        let g = dna(json!({"density": "compact", "accent": "#FF0000", "layout": "river"}));
        let a = dna(json!({"accent": "#00FF00", "layout": "river", "bogus": "x"}));
        let r = resolve(&g, &a, Some(&files()));
        assert_eq!(r["density"], "compact", "from the global profile");
        assert_eq!(r["accent"], "#00FF00", "the app's own wins");
        assert_eq!(r["layout"], "river");
        assert!(!r.contains_key("bogus"));
        assert_eq!(resolve(&g, &Dna::new(), Some(&files()))["layout"], "matrix", "a global layout is ignored; the app's default");
    }

    #[test]
    fn old_files_are_migrated() {
        let v = migrate(json!({"motion": "reduced", "size": "1.2", "density": "compact"}));
        assert_eq!(v, json!({"motion": "minimal", "text_scale": "1.2", "density": "compact", "version": 1}));
        assert!(presets().iter().all(|(_, _, d)| validate(d, None).is_empty()), "every preset is valid");
    }
}
