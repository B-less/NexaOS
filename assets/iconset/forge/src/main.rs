//! nexa-iconforge: builds the NexaHexa icon set from one description.
//!
//!   cargo run --release -- OUT_DIR
//!
//! Every icon is a list of shapes on a 24-unit grid (lines, polylines,
//! circles, arcs, rounded rectangles, pointy-top hexagons and filled
//! shapes). From that one description come:
//!
//! * symbolic icons (`scalable/<context>/<name>-symbolic.svg`, 24x24): every
//!   stroke is turned into a *filled* outline, because GTK 3 recolours a
//!   symbolic icon by setting its fill - strokes would keep their colour and
//!   not follow the theme;
//! * app icons (`scalable/apps/nexa-<app>.svg`, 128x128): the same glyph,
//!   drawn white on the NexaOS hexagon tile in the app's own gradient;
//! * `index.theme` and `preview.svg`, a contact sheet of the whole set.
//!
//! Hexa rules (docs/WRITING_NEXA_APPS.md): the pointy-top hexagon is the
//! mark; strokes are 2 units with round ends; nothing is copied from another
//! platform's icons.

mod icons;

use std::f64::consts::PI;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

pub type Pt = (f64, f64);

/// One shape on the 24-unit grid. Angles are in degrees, 0 = right,
/// increasing clockwise on screen (y grows downwards).
#[derive(Clone, Debug)]
pub enum S {
    L(f64, f64, f64, f64),
    P(Vec<Pt>),
    Pc(Vec<Pt>),
    C(f64, f64, f64),
    E(f64, f64, f64, f64),
    A(f64, f64, f64, f64, f64),
    R(f64, f64, f64, f64, f64),
    H(f64, f64, f64),
    Fc(f64, f64, f64),
    Fp(Vec<Pt>),
    Fr(f64, f64, f64, f64, f64),
    Faded(Box<S>),
}

pub struct Icon {
    pub name: &'static str,
    pub context: &'static str,
    pub shapes: Vec<S>,
}

pub struct App {
    pub id: &'static str,
    pub title: &'static str,
    pub glyph: &'static str,
    pub top: &'static str,
    pub bottom: &'static str,
}

/// Half the symbolic stroke width (strokes are 2 units on the 24 grid).
const HALF: f64 = 1.0;
/// Colour of symbolic icons as files (GTK replaces it with the theme's).
const SYMBOLIC_FILL: &str = "#2e3436";

fn n(v: f64) -> String {
    let s = format!("{:.2}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn polar(cx: f64, cy: f64, r: f64, deg: f64) -> Pt {
    let a = deg * PI / 180.0;
    (cx + r * a.cos(), cy + r * a.sin())
}

fn hex_points(cx: f64, cy: f64, r: f64) -> Vec<Pt> {
    (0..6).map(|k| polar(cx, cy, r, -90.0 + 60.0 * k as f64)).collect()
}

fn arc_points(cx: f64, cy: f64, r: f64, a0: f64, a1: f64) -> Vec<Pt> {
    let steps = (((a1 - a0).abs() / 6.0).ceil() as usize).max(2);
    (0..=steps).map(|i| polar(cx, cy, r, a0 + (a1 - a0) * i as f64 / steps as f64)).collect()
}

fn poly_d(points: &[Pt]) -> String {
    let mut d = String::new();
    for (i, (x, y)) in points.iter().enumerate() {
        let _ = write!(d, "{}{} {} ", if i == 0 { "M" } else { "L" }, n(*x), n(*y));
    }
    d.push('Z');
    d
}

fn circle_d(cx: f64, cy: f64, rx: f64, ry: f64, reverse: bool) -> String {
    let sweep = if reverse { 0 } else { 1 };
    format!("M{} {} A{} {} 0 1 {} {} {} A{} {} 0 1 {} {} {} Z",
            n(cx + rx), n(cy), n(rx), n(ry), sweep, n(cx - rx), n(cy), n(rx), n(ry), sweep, n(cx + rx), n(cy))
}

fn rrect_d(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r == 0.0 {
        return poly_d(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)]);
    }
    format!("M{} {} H{} A{r} {r} 0 0 1 {} {} V{} A{r} {r} 0 0 1 {} {} H{} A{r} {r} 0 0 1 {} {} V{} A{r} {r} 0 0 1 {} {} Z",
            n(x + r), n(y), n(x + w - r), n(x + w), n(y + r), n(y + h - r), n(x + w - r), n(y + h), n(x + r), n(x), n(y + h - r),
            n(y + r), n(x + r), n(y), r = n(r))
}

/// A segment with round ends as a filled outline (the caps are sampled
/// half-circles, so no arc-direction ambiguity).
fn capsule(a: Pt, b: Pt, h: f64) -> String {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        return circle_d(a.0, a.1, h, h, false);
    }
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let mut pts = vec![(a.0 + nx * h, a.1 + ny * h), (b.0 + nx * h, b.1 + ny * h)];
    for k in 1..12 {
        let f = PI * k as f64 / 12.0;
        pts.push((b.0 + (nx * f.cos() + ux * f.sin()) * h, b.1 + (ny * f.cos() + uy * f.sin()) * h));
    }
    pts.push((b.0 - nx * h, b.1 - ny * h));
    pts.push((a.0 - nx * h, a.1 - ny * h));
    for k in 1..12 {
        let f = PI * k as f64 / 12.0;
        pts.push((a.0 - (nx * f.cos() + ux * f.sin()) * h, a.1 - (ny * f.cos() + uy * f.sin()) * h));
    }
    poly_d(&pts)
}

fn polyline_outline(points: &[Pt], closed: bool, h: f64) -> Vec<String> {
    let mut out: Vec<String> = points.windows(2).map(|w| capsule(w[0], w[1], h)).collect();
    if closed && points.len() > 2 {
        out.push(capsule(points[points.len() - 1], points[0], h));
    }
    out
}

/// Symbolic: every shape as filled paths `(d, evenodd)`.
fn outline(s: &S) -> Vec<(String, bool)> {
    let h = HALF;
    match s {
        S::L(x1, y1, x2, y2) => vec![(capsule((*x1, *y1), (*x2, *y2), h), false)],
        S::P(p) => polyline_outline(p, false, h).into_iter().map(|d| (d, false)).collect(),
        S::Pc(p) => polyline_outline(p, true, h).into_iter().map(|d| (d, false)).collect(),
        S::C(cx, cy, r) => vec![(format!("{} {}", circle_d(*cx, *cy, r + h, r + h, false), circle_d(*cx, *cy, r - h, r - h, true)), true)],
        S::E(cx, cy, rx, ry) => vec![(format!("{} {}", circle_d(*cx, *cy, rx + h, ry + h, false), circle_d(*cx, *cy, rx - h, ry - h, true)), true)],
        S::A(cx, cy, r, a0, a1) => polyline_outline(&arc_points(*cx, *cy, *r, *a0, *a1), false, h).into_iter().map(|d| (d, false)).collect(),
        S::R(x, y, w, hh, r) => vec![(format!("{} {}", rrect_d(x - h, y - h, w + 2.0 * h, hh + 2.0 * h, r + h),
                                               rrect_d(x + h, y + h, w - 2.0 * h, hh - 2.0 * h, (r - h).max(0.0))), true)],
        S::H(cx, cy, r) => {
            let k = h / (PI / 6.0).cos();
            vec![(format!("{} {}", poly_d(&hex_points(*cx, *cy, r + k)), poly_d(&hex_points(*cx, *cy, r - k))), true)]
        }
        S::Fc(cx, cy, r) => vec![(circle_d(*cx, *cy, *r, *r, false), false)],
        S::Fp(p) => vec![(poly_d(p), false)],
        S::Fr(x, y, w, hh, r) => vec![(rrect_d(*x, *y, *w, *hh, *r), false)],
        S::Faded(inner) => outline(inner),
    }
}

fn symbolic_svg(icon: &Icon) -> String {
    let mut body = String::new();
    for s in &icon.shapes {
        let faded = matches!(s, S::Faded(_));
        for (d, evenodd) in outline(s) {
            let _ = writeln!(body, "  <path d=\"{d}\"{}{}/>", if evenodd { " fill-rule=\"evenodd\"" } else { "" },
                             if faded { " opacity=\"0.35\"" } else { "" });
        }
    }
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\">\n\
             <g fill=\"{SYMBOLIC_FILL}\">\n{body}</g>\n</svg>\n")
}

/// App glyphs: native SVG strokes (an app icon isn't recoloured).
fn native(s: &S, out: &mut String) {
    let pts = |p: &[Pt]| p.iter().map(|(x, y)| format!("{},{}", n(*x), n(*y))).collect::<Vec<_>>().join(" ");
    let _ = match s {
        S::L(x1, y1, x2, y2) => writeln!(out, "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" fill=\"none\"/>", n(*x1), n(*y1), n(*x2), n(*y2)),
        S::P(p) => writeln!(out, "<polyline points=\"{}\" fill=\"none\"/>", pts(p)),
        S::Pc(p) => writeln!(out, "<polygon points=\"{}\" fill=\"none\"/>", pts(p)),
        S::C(cx, cy, r) => writeln!(out, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\"/>", n(*cx), n(*cy), n(*r)),
        S::E(cx, cy, rx, ry) => writeln!(out, "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"none\"/>", n(*cx), n(*cy), n(*rx), n(*ry)),
        S::A(cx, cy, r, a0, a1) => {
            let (p0, p1) = (polar(*cx, *cy, *r, *a0), polar(*cx, *cy, *r, *a1));
            let large = if (a1 - a0).abs() > 180.0 { 1 } else { 0 };
            let sweep = if a1 > a0 { 1 } else { 0 };
            if (a1 - a0).abs() >= 359.9 {
                writeln!(out, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\"/>", n(*cx), n(*cy), n(*r))
            } else {
                writeln!(out, "<path d=\"M{} {} A{} {} 0 {large} {sweep} {} {}\" fill=\"none\"/>", n(p0.0), n(p0.1), n(*r), n(*r), n(p1.0), n(p1.1))
            }
        }
        S::R(x, y, w, h, r) => writeln!(out, "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"none\"/>", n(*x), n(*y), n(*w), n(*h), n(*r)),
        S::H(cx, cy, r) => writeln!(out, "<polygon points=\"{}\" fill=\"none\"/>", pts(&hex_points(*cx, *cy, *r))),
        S::Fc(cx, cy, r) => writeln!(out, "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"currentColor\" stroke=\"none\"/>", n(*cx), n(*cy), n(*r)),
        S::Fp(p) => writeln!(out, "<polygon points=\"{}\" fill=\"currentColor\" stroke=\"none\"/>", pts(p)),
        S::Fr(x, y, w, h, r) => writeln!(out, "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"currentColor\" stroke=\"none\"/>", n(*x), n(*y), n(*w), n(*h), n(*r)),
        S::Faded(inner) => {
            out.push_str("<g opacity=\"0.45\">\n");
            native(inner, out);
            out.push_str("</g>\n");
            Ok(())
        }
    };
}

/// A pointy-top hexagon with softly rounded corners.
fn rounded_hex_d(cx: f64, cy: f64, r: f64, round: f64) -> String {
    let p = hex_points(cx, cy, r);
    let mut d = String::new();
    for i in 0..6 {
        let (prev, cur, next) = (p[(i + 5) % 6], p[i], p[(i + 1) % 6]);
        let towards = |a: Pt, b: Pt, t: f64| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l = (dx * dx + dy * dy).sqrt();
            (a.0 + dx / l * t, a.1 + dy / l * t)
        };
        let (s, e) = (towards(cur, prev, round), towards(cur, next, round));
        let _ = write!(d, "{}{} {} Q{} {} {} {} ", if i == 0 { "M" } else { "L" }, n(s.0), n(s.1), n(cur.0), n(cur.1), n(e.0), n(e.1));
    }
    d.push('Z');
    d
}

fn app_svg(app: &App, glyph: &Icon) -> String {
    let mut g = String::new();
    for s in &glyph.shapes {
        native(s, &mut g);
    }
    let tile = rounded_hex_d(64.0, 64.0, 60.0, 9.0);
    let rim = rounded_hex_d(64.0, 64.0, 57.0, 8.0);
    let id = app.id;
    // Glyph: 24-unit grid scaled to 66 px, centred; a soft shadow under it.
    format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
<title>{title}</title>
<defs>
  <linearGradient id="{id}-bg" x1="0.15" y1="0" x2="0.85" y2="1">
    <stop offset="0" stop-color="{top}"/>
    <stop offset="1" stop-color="{bottom}"/>
  </linearGradient>
  <linearGradient id="{id}-sheen" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#ffffff" stop-opacity="0.35"/>
    <stop offset="0.55" stop-color="#ffffff" stop-opacity="0"/>
  </linearGradient>
</defs>
<path d="{tile}" fill="url(#{id}-bg)"/>
<path d="{rim}" fill="none" stroke="url(#{id}-sheen)" stroke-width="2"/>
<g color="#000000" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round" opacity="0.2" transform="translate(31 32.6) scale(2.75)">
{g}</g>
<g color="#ffffff" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round" transform="translate(31 31) scale(2.75)">
{g}</g>
</svg>
"##, title = app.title, top = app.top, bottom = app.bottom)
}

fn inner_svg(svg: &str) -> String {
    let start = svg.find('>').map(|i| i + 1).unwrap_or(0);
    let end = svg.rfind("</svg>").unwrap_or(svg.len());
    svg[start..end].to_string()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;")
}

/// One sheet with every icon: app tiles on the dark theme colour, then the
/// symbolic icons in the theme's light text colour, with their names.
fn preview(apps: &[(String, &App)], symbolic: &[(String, &Icon)]) -> String {
    let cols_a = 8;
    let cols_s = 10;
    let (cell_a, cell_s) = (150.0, 120.0);
    let rows_a = apps.len().div_ceil(cols_a);
    let rows_s = symbolic.len().div_ceil(cols_s);
    let width = (cols_a as f64 * cell_a).max(cols_s as f64 * cell_s) + 40.0;
    let top_s = 80.0 + rows_a as f64 * cell_a + 60.0;
    let height = top_s + rows_s as f64 * cell_s + 30.0;
    let mut out = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n\
        <rect width=\"{w}\" height=\"{h}\" fill=\"#0c1228\"/>\n\
        <text x=\"20\" y=\"48\" font-family=\"Inter, sans-serif\" font-size=\"26\" font-weight=\"700\" fill=\"#e6ecff\">NexaHexa icons: {} apps, {} symbolic</text>\n",
        apps.len(), symbolic.len(), w = n(width), h = n(height));
    for (i, (svg, app)) in apps.iter().enumerate() {
        let (x, y) = (20.0 + (i % cols_a) as f64 * cell_a, 80.0 + (i / cols_a) as f64 * cell_a);
        let _ = write!(out, "<g transform=\"translate({} {}) scale(0.85)\">{}</g>\n<text x=\"{}\" y=\"{}\" font-family=\"Inter, sans-serif\" font-size=\"13\" fill=\"#8f9bc4\" text-anchor=\"middle\">{}</text>\n",
                       n(x + 11.0), n(y), inner_svg(svg).replace("id=\"", &format!("id=\"p{i}")).replace("url(#", &format!("url(#p{i}")),
                       n(x + 65.0), n(y + 128.0), esc(app.title));
    }
    let _ = writeln!(out, "<text x=\"20\" y=\"{}\" font-family=\"Inter, sans-serif\" font-size=\"20\" font-weight=\"700\" fill=\"#e6ecff\">Symbolic (follow the theme's text colour)</text>", n(top_s - 20.0));
    for (i, (svg, icon)) in symbolic.iter().enumerate() {
        let (x, y) = (20.0 + (i % cols_s) as f64 * cell_s, top_s + (i / cols_s) as f64 * cell_s);
        let _ = write!(out, "<rect x=\"{}\" y=\"{}\" width=\"104\" height=\"100\" rx=\"10\" fill=\"#101732\"/>\n<g transform=\"translate({} {}) scale(2)\">{}</g>\n<text x=\"{}\" y=\"{}\" font-family=\"Inter, sans-serif\" font-size=\"{}\" fill=\"#8f9bc4\" text-anchor=\"middle\">{}</text>\n",
                       n(x), n(y), n(x + 28.0), n(y + 12.0), inner_svg(svg).replace(SYMBOLIC_FILL, "#d9e0f6"),
                       n(x + 52.0), n(y + 88.0), if icon.name.len() > 20 { "7" } else { "10" }, esc(icon.name));
    }
    out.push_str("</svg>\n");
    out
}

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| "NexaHexa".into());
    let out = Path::new(&out_dir);
    let icons = icons::all();
    let apps = icons::apps();
    let mut seen = std::collections::HashSet::new();
    let mut symbolic = Vec::new();
    let mut contexts = std::collections::BTreeSet::new();
    for icon in &icons {
        assert!(seen.insert(icon.name), "duplicate icon {}", icon.name);
        let dir = out.join("scalable").join(icon.context);
        fs::create_dir_all(&dir).expect("output folder");
        let svg = symbolic_svg(icon);
        fs::write(dir.join(format!("{}-symbolic.svg", icon.name)), &svg).expect("write icon");
        contexts.insert(icon.context);
        symbolic.push((svg, icon));
    }
    let app_dir = out.join("scalable/apps");
    fs::create_dir_all(&app_dir).expect("apps folder");
    contexts.insert("apps");
    let mut tiles = Vec::new();
    for app in &apps {
        let glyph = icons.iter().find(|i| i.name == app.glyph).unwrap_or_else(|| panic!("{}: no glyph {}", app.id, app.glyph));
        let svg = app_svg(app, glyph);
        fs::write(app_dir.join(format!("{}.svg", app.id)), &svg).expect("write app icon");
        tiles.push((svg, app));
    }
    let dirs: Vec<String> = contexts.iter().map(|c| format!("scalable/{c}")).collect();
    let mut theme = format!("[Icon Theme]\nName=NexaHexa\nComment=NexaOS icons: Hexa app tiles and symbolic icons\nInherits=Adwaita,hicolor\nDirectories={}\n", dirs.join(","));
    for (c, d) in contexts.iter().zip(&dirs) {
        let context = match *c { "apps" => "Applications", "actions" => "Actions", "status" => "Status", "places" => "Places",
                                 "devices" => "Devices", "mimetypes" => "MimeTypes", other => other };
        let _ = write!(theme, "\n[{d}]\nContext={context}\nSize=128\nMinSize=8\nMaxSize=512\nType=Scalable\n");
    }
    fs::write(out.join("index.theme"), theme).expect("index.theme");
    fs::write(out.join("preview.svg"), preview(&tiles.iter().map(|(s, a)| (s.clone(), *a)).collect::<Vec<_>>(),
                                                  &symbolic.iter().map(|(s, i)| (s.clone(), *i)).collect::<Vec<_>>())).expect("preview");
    println!("NexaHexa: {} app icons, {} symbolic icons in {}", apps.len(), icons.len(), out.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbolic_icons_have_no_strokes() {
        // GTK 3 recolours only fills: a stroke would ignore the theme.
        for icon in icons::all() {
            let svg = symbolic_svg(&icon);
            assert!(!svg.contains("stroke"), "{} has a stroke", icon.name);
            assert!(svg.contains("viewBox=\"0 0 24 24\""));
        }
    }

    #[test]
    fn every_app_has_its_glyph_and_names_are_unique() {
        let icons = icons::all();
        let mut ids = std::collections::HashSet::new();
        for app in icons::apps() {
            assert!(ids.insert(app.id), "duplicate app {}", app.id);
            assert!(app.id.starts_with("nexa-"), "{}", app.id);
            assert!(icons.iter().any(|i| i.name == app.glyph), "{} -> {}", app.id, app.glyph);
        }
    }

    #[test]
    fn shapes_stay_inside_the_grid() {
        // The outlines of every shape fit in 0..24 (with half a stroke).
        for icon in icons::all() {
            for s in &icon.shapes {
                for (d, _) in outline(s) {
                    for tok in d.split(|c: char| c.is_ascii_alphabetic() || c == ' ').filter(|t| !t.is_empty()) {
                        if let Ok(v) = tok.parse::<f64>() {
                            assert!((-0.6..=24.6).contains(&v), "{}: {v} outside the grid", icon.name);
                        }
                    }
                }
            }
        }
    }
}
