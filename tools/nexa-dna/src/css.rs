//! Turns a resolved DNA into GTK CSS. Apps load it above their own styles
//! and the theme's palette, so a trait wins wherever the app doesn't draw
//! something itself. Layout and navigation aren't CSS: apps read those.

use crate::model::{accent_rgb, Dna};
use serde_json::Value;
use std::fmt::Write;

fn get<'a>(d: &'a Dna, k: &str) -> &'a str {
    d.get(k).and_then(Value::as_str).unwrap_or("")
}

fn scale(d: &Dna) -> f64 {
    d.get("text_scale").and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_f64())).unwrap_or(1.0).clamp(0.8, 1.6)
}

fn mix(c: (f64, f64, f64), toward: f64, t: f64) -> String {
    let f = |x: f64| ((x + (toward - x) * t).clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", f(c.0), f(c.1), f(c.2))
}

pub fn css(d: &Dna) -> String {
    let mut out = String::from("/* Interface DNA (nexa-dna); edits are overwritten. */\n");
    let w = &mut out;
    match get(d, "density") {
        "compact" => {
            let _ = writeln!(w, "button {{ padding: 2px 8px; min-height: 22px; }}");
            let _ = writeln!(w, "entry, spinbutton, combobox button {{ min-height: 24px; padding-top: 1px; padding-bottom: 1px; }}");
            let _ = writeln!(w, "treeview.view {{ padding: 1px 4px; }} list row, flowboxchild {{ padding: 1px; }}");
        }
        "spacious" => {
            let _ = writeln!(w, "button {{ padding: 8px 16px; min-height: 36px; }}");
            let _ = writeln!(w, "entry, spinbutton, combobox button {{ min-height: 38px; }}");
            let _ = writeln!(w, "treeview.view {{ padding: 6px 8px; }} list row, flowboxchild {{ padding: 6px; }}");
        }
        _ => {}
    }
    let radius = match get(d, "geometry") {
        "angular" => Some("0px"),
        "pill" => Some("999px"),
        "rounded" => Some("8px"),
        _ => None,
    };
    if let Some(r) = radius {
        let field = if r == "999px" { "14px" } else { r };
        let _ = writeln!(w, "button, .nexa-hexbtn {{ border-radius: {r}; }}");
        let _ = writeln!(w, "entry, spinbutton, combobox button, popover, menu, .card, frame > border {{ border-radius: {field}; }}");
    }
    match get(d, "borders") {
        "none" => {
            let _ = writeln!(w, "button, entry, spinbutton, frame > border, treeview header button {{ border-color: transparent; box-shadow: none; }}");
        }
        "strong" => {
            let _ = writeln!(w, "button, entry, spinbutton, frame > border {{ border: 2px solid alpha(@nexa_text, 0.55); }}");
        }
        _ => {}
    }
    let accent = get(d, "accent");
    if let Some(rgb) = accent_rgb(accent) {
        let _ = writeln!(w, "@define-color nexa_accent {accent};");
        let _ = writeln!(w, "@define-color nexa_accent_hover {};", mix(rgb, 1.0, 0.15));
        let _ = writeln!(w, "@define-color nexa_selected alpha({accent}, 0.35);");
    }
    if get(d, "contrast") == "high" {
        let _ = writeln!(w, "@define-color nexa_text #ffffff;\n@define-color nexa_text_strong #ffffff;\n@define-color nexa_muted #e3e8ff;");
        let _ = writeln!(w, "*:focus {{ outline: 3px solid @nexa_accent; outline-offset: 1px; }}");
        let _ = writeln!(w, "button, entry {{ border: 2px solid @nexa_text; }}");
    }
    let family = match get(d, "typography") {
        "mono" => Some("\"JetBrains Mono\", monospace"),
        "system" => Some("\"Noto Sans\", sans-serif"),
        "inter" => Some("Inter, sans-serif"),
        _ => None,
    };
    let s = scale(d);
    if family.is_some() || (s - 1.0).abs() > 0.01 {
        let mut decl = String::new();
        if let Some(f) = family {
            let _ = write!(decl, " font-family: {f};");
        }
        if (s - 1.0).abs() > 0.01 {
            let _ = write!(decl, " font-size: {:.1}pt;", 10.0 * s);
        }
        let _ = writeln!(w, "window, window *, popover, menu {{{decl} }}");
    }
    match get(d, "motion") {
        "none" => {
            let _ = writeln!(w, "* {{ transition: none; animation: none; }}");
        }
        "minimal" => {
            let _ = writeln!(w, "* {{ transition-duration: 60ms; animation-duration: 60ms; }}");
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn d(v: serde_json::Value) -> Dna {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn traits_become_css() {
        let c = css(&d(json!({"density": "compact", "geometry": "angular", "accent": "#6B5CFF", "motion": "none",
                              "typography": "mono", "text_scale": "1.2", "contrast": "high", "borders": "strong"})));
        assert!(c.contains("min-height: 22px"));
        assert!(c.contains("border-radius: 0px"));
        assert!(c.contains("@define-color nexa_accent #6B5CFF;"));
        assert!(c.contains("transition: none"));
        assert!(c.contains("JetBrains Mono") && c.contains("font-size: 12.0pt"));
        assert!(c.contains("outline: 3px"));
    }

    #[test]
    fn defaults_add_almost_nothing() {
        let c = css(&d(json!({"density": "comfortable", "geometry": "", "accent": "theme", "motion": "standard", "contrast": "normal"})));
        assert!(!c.contains("@define-color") && !c.contains("transition") && !c.contains("font-size"), "{c}");
    }
}
