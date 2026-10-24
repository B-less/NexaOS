//! The NexaHexa icons, on a 24-unit grid with 2-unit strokes and round
//! ends. Shapes: L line, P polyline, Pc closed polyline, C circle, E
//! ellipse, A arc (degrees, 0 = right, clockwise), R rounded rectangle, H
//! pointy-top hexagon (the NexaOS mark), F* filled shapes, Faded (shown
//! dimmed: the unused bars of a level).

use crate::{App, Icon, Pt, S};
use std::f64::consts::PI;

fn l(x1: f64, y1: f64, x2: f64, y2: f64) -> S { S::L(x1, y1, x2, y2) }
fn p(v: &[Pt]) -> S { S::P(v.to_vec()) }
fn pc(v: &[Pt]) -> S { S::Pc(v.to_vec()) }
fn c(x: f64, y: f64, r: f64) -> S { S::C(x, y, r) }
fn e(x: f64, y: f64, rx: f64, ry: f64) -> S { S::E(x, y, rx, ry) }
fn a(x: f64, y: f64, r: f64, a0: f64, a1: f64) -> S { S::A(x, y, r, a0, a1) }
fn r(x: f64, y: f64, w: f64, h: f64, rr: f64) -> S { S::R(x, y, w, h, rr) }
fn h(x: f64, y: f64, rr: f64) -> S { S::H(x, y, rr) }
fn fc(x: f64, y: f64, rr: f64) -> S { S::Fc(x, y, rr) }
fn fp(v: &[Pt]) -> S { S::Fp(v.to_vec()) }
fn fr(x: f64, y: f64, w: f64, hh: f64, rr: f64) -> S { S::Fr(x, y, w, hh, rr) }
fn faded(s: S) -> S { S::Faded(Box::new(s)) }

/// A star with `k` points (filled).
fn star(cx: f64, cy: f64, outer: f64, inner: f64, k: usize) -> S {
    let pts: Vec<Pt> = (0..2 * k).map(|i| {
        let rad = if i % 2 == 0 { outer } else { inner };
        let ang = -PI / 2.0 + PI * i as f64 / k as f64;
        (cx + rad * ang.cos(), cy + rad * ang.sin())
    }).collect();
    S::Fp(pts)
}

fn icon(name: &'static str, context: &'static str, shapes: Vec<S>) -> Icon {
    Icon { name, context, shapes }
}

// Shared parts.
fn folder() -> Vec<S> {
    vec![pc(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.5), (21.0, 8.5), (21.0, 19.0), (3.0, 19.0)])]
}
fn file_outline() -> Vec<S> {
    vec![pc(&[(6.0, 3.0), (14.0, 3.0), (19.0, 8.0), (19.0, 21.0), (6.0, 21.0)]), p(&[(14.0, 3.0), (14.0, 8.0), (19.0, 8.0)])]
}
fn speaker() -> S {
    fp(&[(3.0, 9.0), (7.0, 9.0), (12.0, 4.5), (12.0, 19.5), (7.0, 15.0), (3.0, 15.0)])
}
fn wifi(bars: usize) -> Vec<S> {
    let mut v = vec![fc(12.0, 19.0, 1.7)];
    for (i, rad) in [5.5, 10.0, 14.5].iter().enumerate() {
        let arc = a(12.0, 19.0, *rad, 228.0, 312.0);
        v.push(if i < bars { arc } else { faded(arc) });
    }
    v
}
fn battery(level: f64) -> Vec<S> {
    let mut v = vec![r(2.5, 7.0, 17.0, 10.0, 2.0), fr(20.5, 10.0, 1.5, 4.0, 0.6)];
    if level > 0.0 {
        v.push(fr(5.0, 9.5, 12.0 * level, 5.0, 0.8));
    }
    v
}
fn cube() -> Vec<S> {
    // The pointy-top hexagon seen as a box: the NexaOS package.
    vec![h(12.0, 12.0, 9.0), l(12.0, 12.0, 12.0, 21.0), l(12.0, 12.0, 4.2, 7.5), l(12.0, 12.0, 19.8, 7.5)]
}
fn search_lens() -> Vec<S> {
    vec![c(10.5, 10.5, 6.0), l(15.0, 15.0, 20.0, 20.0)]
}
fn lock_body() -> Vec<S> {
    vec![r(5.0, 11.0, 14.0, 10.0, 2.0), fc(12.0, 16.0, 1.4)]
}
fn sun() -> Vec<S> {
    let mut v = vec![c(12.0, 12.0, 4.0)];
    for k in 0..8 {
        let ang = PI / 4.0 * k as f64;
        v.push(l(12.0 + 7.0 * ang.cos(), 12.0 + 7.0 * ang.sin(), 12.0 + 9.5 * ang.cos(), 12.0 + 9.5 * ang.sin()));
    }
    v
}

pub fn all() -> Vec<Icon> {
    let mut v = vec![
        // ---- the NexaOS mark ----
        icon("nexa", "apps", vec![h(12.0, 12.0, 9.0), c(12.0, 12.0, 4.0), fc(12.0, 12.0, 1.6)]),
        icon("nexa-ai", "apps", vec![star(10.5, 13.0, 8.5, 2.3, 4), star(18.5, 5.5, 3.2, 0.9, 4)]),
        // ---- navigation and windows ----
        icon("go-previous", "actions", vec![p(&[(15.0, 5.0), (8.0, 12.0), (15.0, 19.0)])]),
        icon("go-next", "actions", vec![p(&[(9.0, 5.0), (16.0, 12.0), (9.0, 19.0)])]),
        icon("go-up", "actions", vec![p(&[(5.0, 15.0), (12.0, 8.0), (19.0, 15.0)])]),
        icon("go-down", "actions", vec![p(&[(5.0, 9.0), (12.0, 16.0), (19.0, 9.0)])]),
        icon("go-home", "actions", vec![p(&[(3.0, 11.0), (12.0, 3.5), (21.0, 11.0)]), p(&[(5.5, 9.0), (5.5, 20.0), (18.5, 20.0), (18.5, 9.0)]), r(10.0, 14.0, 4.0, 6.0, 1.0)]),
        icon("open-menu", "actions", vec![l(4.0, 7.0, 20.0, 7.0), l(4.0, 12.0, 20.0, 12.0), l(4.0, 17.0, 20.0, 17.0)]),
        icon("view-more", "actions", vec![fc(5.5, 12.0, 1.8), fc(12.0, 12.0, 1.8), fc(18.5, 12.0, 1.8)]),
        icon("window-close", "actions", vec![l(6.0, 6.0, 18.0, 18.0), l(18.0, 6.0, 6.0, 18.0)]),
        icon("window-minimize", "actions", vec![l(6.0, 17.0, 18.0, 17.0)]),
        icon("window-maximize", "actions", vec![h(12.0, 12.0, 8.0)]),
        icon("window-restore", "actions", vec![h(10.5, 13.5, 6.5), p(&[(10.0, 5.0), (13.5, 3.0), (19.5, 6.5), (19.5, 13.5), (17.0, 15.0)])]),
        icon("view-fullscreen", "actions", vec![p(&[(4.0, 9.0), (4.0, 4.0), (9.0, 4.0)]), p(&[(15.0, 4.0), (20.0, 4.0), (20.0, 9.0)]),
                                                p(&[(20.0, 15.0), (20.0, 20.0), (15.0, 20.0)]), p(&[(9.0, 20.0), (4.0, 20.0), (4.0, 15.0)])]),
        icon("view-grid", "actions", vec![r(4.0, 4.0, 7.0, 7.0, 1.5), r(13.0, 4.0, 7.0, 7.0, 1.5), r(4.0, 13.0, 7.0, 7.0, 1.5), r(13.0, 13.0, 7.0, 7.0, 1.5)]),
        icon("view-list", "actions", vec![fc(5.0, 6.0, 1.4), fc(5.0, 12.0, 1.4), fc(5.0, 18.0, 1.4), l(9.0, 6.0, 20.0, 6.0), l(9.0, 12.0, 20.0, 12.0), l(9.0, 18.0, 20.0, 18.0)]),
        icon("system-search", "actions", search_lens()),
        icon("zoom-in", "actions", [search_lens(), vec![l(7.5, 10.5, 13.5, 10.5), l(10.5, 7.5, 10.5, 13.5)]].concat()),
        icon("zoom-out", "actions", [search_lens(), vec![l(7.5, 10.5, 13.5, 10.5)]].concat()),
        icon("view-refresh", "actions", vec![a(12.0, 12.0, 7.5, -30.0, 270.0), p(&[(9.3, 1.8), (12.0, 4.5), (9.3, 7.2)])]),
        icon("preferences-system", "actions", vec![l(4.0, 7.0, 20.0, 7.0), l(4.0, 12.0, 20.0, 12.0), l(4.0, 17.0, 20.0, 17.0), fc(9.0, 7.0, 2.4), fc(15.5, 12.0, 2.4), fc(7.5, 17.0, 2.4)]),
        // ---- editing ----
        icon("list-add", "actions", vec![l(12.0, 5.0, 12.0, 19.0), l(5.0, 12.0, 19.0, 12.0)]),
        icon("list-remove", "actions", vec![l(5.0, 12.0, 19.0, 12.0)]),
        icon("document-edit", "actions", vec![pc(&[(4.0, 20.0), (5.0, 15.5), (15.5, 5.0), (19.0, 8.5), (8.5, 19.0)]), l(13.0, 7.5, 16.5, 11.0)]),
        icon("edit-delete", "actions", vec![l(4.0, 6.5, 20.0, 6.5), p(&[(9.0, 6.5), (9.0, 3.8), (15.0, 3.8), (15.0, 6.5)]),
                                            p(&[(6.0, 6.5), (7.0, 20.0), (17.0, 20.0), (18.0, 6.5)]), l(10.0, 10.5, 10.0, 16.0), l(14.0, 10.5, 14.0, 16.0)]),
        icon("edit-copy", "actions", vec![r(8.5, 8.5, 11.5, 11.5, 2.0), p(&[(15.5, 4.0), (6.0, 4.0), (4.0, 6.0), (4.0, 15.5)])]),
        icon("edit-cut", "actions", vec![c(7.0, 17.0, 3.0), c(17.0, 17.0, 3.0), l(9.0, 14.8, 17.5, 4.0), l(15.0, 14.8, 6.5, 4.0)]),
        icon("edit-paste", "actions", vec![r(5.0, 5.0, 14.0, 16.0, 2.0), r(9.0, 3.0, 6.0, 4.0, 1.0)]),
        icon("edit-undo", "actions", vec![p(&[(8.5, 4.5), (4.0, 9.0), (8.5, 13.5)]), l(4.0, 9.0, 14.0, 9.0), a(14.0, 14.0, 5.0, 270.0, 450.0), l(14.0, 19.0, 9.0, 19.0)]),
        icon("edit-redo", "actions", vec![p(&[(15.5, 4.5), (20.0, 9.0), (15.5, 13.5)]), l(20.0, 9.0, 10.0, 9.0), a(10.0, 14.0, 5.0, 270.0, 90.0), l(10.0, 19.0, 15.0, 19.0)]),
        icon("document-save", "actions", vec![l(12.0, 4.0, 12.0, 14.5), p(&[(7.5, 10.0), (12.0, 14.5), (16.5, 10.0)]), p(&[(4.0, 14.0), (4.0, 20.0), (20.0, 20.0), (20.0, 14.0)])]),
        icon("document-open", "actions", [folder(), vec![l(12.0, 11.0, 12.0, 16.5), p(&[(9.5, 13.5), (12.0, 11.0), (14.5, 13.5)])]].concat()),
        icon("document-share", "actions", vec![c(18.0, 5.5, 2.5), c(6.0, 12.0, 2.5), c(18.0, 18.5, 2.5), l(8.2, 10.8, 15.8, 6.7), l(8.2, 13.2, 15.8, 17.3)]),
        icon("document-download", "actions", vec![l(12.0, 3.5, 12.0, 15.5), p(&[(7.0, 10.5), (12.0, 15.5), (17.0, 10.5)]), l(5.0, 20.0, 19.0, 20.0)]),
        icon("document-upload", "actions", vec![l(12.0, 15.5, 12.0, 3.5), p(&[(7.0, 8.5), (12.0, 3.5), (17.0, 8.5)]), l(5.0, 20.0, 19.0, 20.0)]),
        icon("starred", "actions", vec![star(12.0, 12.5, 9.5, 4.2, 5)]),
        // ---- media ----
        icon("media-playback-start", "actions", vec![fp(&[(8.0, 5.0), (19.0, 12.0), (8.0, 19.0)])]),
        icon("media-playback-pause", "actions", vec![fr(6.5, 5.0, 4.0, 14.0, 1.0), fr(13.5, 5.0, 4.0, 14.0, 1.0)]),
        icon("media-playback-stop", "actions", vec![fr(6.0, 6.0, 12.0, 12.0, 2.0)]),
        icon("media-skip-forward", "actions", vec![fp(&[(5.0, 5.0), (14.5, 12.0), (5.0, 19.0)]), fr(16.0, 5.0, 3.0, 14.0, 1.0)]),
        icon("media-skip-backward", "actions", vec![fp(&[(19.0, 5.0), (9.5, 12.0), (19.0, 19.0)]), fr(5.0, 5.0, 3.0, 14.0, 1.0)]),
        icon("media-playlist-shuffle", "actions", vec![p(&[(4.0, 7.0), (8.0, 7.0), (15.5, 17.0), (19.5, 17.0)]), p(&[(4.0, 17.0), (8.0, 17.0), (15.5, 7.0), (19.5, 7.0)]),
                                                       p(&[(17.0, 4.5), (19.5, 7.0), (17.0, 9.5)]), p(&[(17.0, 14.5), (19.5, 17.0), (17.0, 19.5)])]),
        icon("media-playlist-repeat", "actions", vec![p(&[(4.0, 13.0), (4.0, 9.5), (6.5, 7.0), (19.0, 7.0)]), p(&[(16.5, 4.5), (19.0, 7.0), (16.5, 9.5)]),
                                                      p(&[(20.0, 11.0), (20.0, 14.5), (17.5, 17.0), (5.0, 17.0)]), p(&[(7.5, 14.5), (5.0, 17.0), (7.5, 19.5)])]),
        icon("audio-volume-high", "status", vec![speaker(), a(11.5, 12.0, 4.5, -50.0, 50.0), a(11.5, 12.0, 8.5, -50.0, 50.0)]),
        icon("audio-volume-medium", "status", vec![speaker(), a(11.5, 12.0, 4.5, -50.0, 50.0), faded(a(11.5, 12.0, 8.5, -50.0, 50.0))]),
        icon("audio-volume-low", "status", vec![speaker(), faded(a(11.5, 12.0, 4.5, -50.0, 50.0)), faded(a(11.5, 12.0, 8.5, -50.0, 50.0))]),
        icon("audio-volume-muted", "status", vec![speaker(), l(15.5, 9.5, 20.5, 14.5), l(20.5, 9.5, 15.5, 14.5)]),
        icon("audio-input-microphone", "devices", vec![r(9.0, 3.0, 6.0, 11.0, 3.0), a(12.0, 11.0, 6.5, 0.0, 180.0), l(12.0, 17.5, 12.0, 21.0), l(8.5, 21.0, 15.5, 21.0)]),
        icon("camera-photo", "devices", vec![r(3.0, 7.0, 18.0, 13.0, 2.5), p(&[(8.0, 7.0), (9.5, 4.5), (14.5, 4.5), (16.0, 7.0)]), c(12.0, 13.5, 3.5)]),
        icon("camera-video", "devices", vec![r(3.0, 7.0, 12.5, 10.0, 2.0), pc(&[(15.5, 10.5), (21.0, 7.5), (21.0, 16.5), (15.5, 13.5)])]),
        // ---- system ----
        icon("system-lock-screen", "actions", [vec![a(12.0, 8.5, 4.0, 180.0, 360.0), l(8.0, 8.5, 8.0, 11.0), l(16.0, 8.5, 16.0, 11.0)], lock_body()].concat()),
        icon("changes-allow", "status", [vec![a(12.0, 8.5, 4.0, 180.0, 345.0), l(8.0, 8.5, 8.0, 11.0)], lock_body()].concat()),
        icon("system-log-out", "actions", vec![p(&[(10.0, 4.0), (5.0, 4.0), (5.0, 20.0), (10.0, 20.0)]), l(9.5, 12.0, 20.0, 12.0), p(&[(16.0, 8.0), (20.0, 12.0), (16.0, 16.0)])]),
        icon("system-shutdown", "actions", vec![a(12.0, 12.5, 7.5, -55.0, 235.0), l(12.0, 3.0, 12.0, 11.0)]),
        icon("system-reboot", "actions", vec![a(12.0, 12.0, 7.5, -100.0, 200.0), p(&[(2.7, 6.9), (4.95, 9.43), (7.75, 7.6)])]),
        icon("system-suspend", "actions", vec![a(12.0, 12.5, 8.5, 9.0, 268.6), a(16.8, 8.3, 6.6, 220.6, 57.0)]),
        icon("preferences-desktop-notification", "status", vec![pc(&[(5.0, 17.0), (6.5, 15.0), (6.5, 10.5), (8.0, 7.0), (12.0, 5.3), (16.0, 7.0), (17.5, 10.5), (17.5, 15.0), (19.0, 17.0)]), l(10.0, 20.0, 14.0, 20.0)]),
        icon("notifications-disabled", "status", vec![pc(&[(5.0, 17.0), (6.5, 15.0), (6.5, 10.5), (8.0, 7.0), (12.0, 5.3), (16.0, 7.0), (17.5, 10.5), (17.5, 15.0), (19.0, 17.0)]), l(4.0, 4.0, 20.0, 20.0)]),
        icon("dialog-information", "status", vec![c(12.0, 12.0, 9.0), l(12.0, 11.0, 12.0, 16.5), fc(12.0, 7.6, 1.3)]),
        icon("dialog-warning", "status", vec![pc(&[(12.0, 3.5), (21.0, 19.5), (3.0, 19.5)]), l(12.0, 9.5, 12.0, 14.0), fc(12.0, 16.8, 1.2)]),
        icon("dialog-error", "status", vec![h(12.0, 12.0, 9.0), l(12.0, 7.5, 12.0, 13.0), fc(12.0, 16.2, 1.25)]),
        icon("emblem-ok", "status", vec![c(12.0, 12.0, 9.0), p(&[(8.0, 12.5), (11.0, 15.5), (16.5, 9.5)])]),
        icon("help-about", "status", vec![c(12.0, 12.0, 9.0), a(12.0, 9.5, 3.0, 180.0, 405.0), p(&[(14.12, 11.62), (12.0, 13.0), (12.0, 14.0)]), fc(12.0, 17.2, 1.2)]),
        icon("security-high", "status", vec![pc(&[(12.0, 3.0), (19.5, 6.0), (19.5, 12.0), (16.5, 17.5), (12.0, 21.0), (7.5, 17.5), (4.5, 12.0), (4.5, 6.0)]), p(&[(8.8, 12.0), (11.2, 14.4), (15.5, 10.0)])]),
        icon("view-reveal", "actions", vec![pc(&[(2.5, 12.0), (6.0, 7.5), (12.0, 5.5), (18.0, 7.5), (21.5, 12.0), (18.0, 16.5), (12.0, 18.5), (6.0, 16.5)]), c(12.0, 12.0, 3.0)]),
        icon("view-conceal", "actions", vec![pc(&[(2.5, 12.0), (6.0, 7.5), (12.0, 5.5), (18.0, 7.5), (21.5, 12.0), (18.0, 16.5), (12.0, 18.5), (6.0, 16.5)]), c(12.0, 12.0, 3.0), l(4.0, 20.0, 20.0, 4.0)]),
        icon("utilities-terminal", "apps", vec![r(3.0, 4.0, 18.0, 16.0, 2.5), p(&[(7.0, 9.0), (10.5, 12.0), (7.0, 15.0)]), l(12.5, 15.0, 17.0, 15.0)]),
        icon("utilities-system-monitor", "apps", vec![p(&[(3.0, 12.0), (7.0, 12.0), (9.5, 6.0), (13.5, 18.0), (16.0, 12.0), (21.0, 12.0)])]),
        icon("accessories-calculator", "apps", vec![r(5.0, 3.0, 14.0, 18.0, 2.5), r(8.0, 6.0, 8.0, 4.0, 1.0), fc(9.0, 14.0, 1.1), fc(12.0, 14.0, 1.1), fc(15.0, 14.0, 1.1),
                                                    fc(9.0, 17.5, 1.1), fc(12.0, 17.5, 1.1), fc(15.0, 17.5, 1.1)]),
        icon("accessories-text-editor", "apps", vec![p(&[(8.0, 7.0), (3.0, 12.0), (8.0, 17.0)]), p(&[(16.0, 7.0), (21.0, 12.0), (16.0, 17.0)]), l(14.0, 5.0, 10.0, 19.0)]),
        icon("accessories-notes", "apps", vec![r(5.0, 3.0, 14.0, 18.0, 2.0), l(8.5, 8.0, 15.5, 8.0), l(8.5, 12.0, 15.5, 12.0), l(8.5, 16.0, 13.0, 16.0)]),
        icon("accessories-character-map", "apps", vec![c(12.0, 12.0, 9.0), fc(9.0, 10.0, 1.3), fc(15.0, 10.0, 1.3), a(12.0, 12.0, 5.0, 30.0, 150.0)]),
        icon("accessories-screenshot", "apps", vec![p(&[(4.0, 9.0), (4.0, 4.0), (9.0, 4.0)]), p(&[(15.0, 4.0), (20.0, 4.0), (20.0, 9.0)]),
                                                    p(&[(20.0, 15.0), (20.0, 20.0), (15.0, 20.0)]), p(&[(9.0, 20.0), (4.0, 20.0), (4.0, 15.0)]), c(12.0, 12.0, 3.5)]),
        icon("color-select", "apps", vec![pc(&[(12.0, 3.5), (16.5, 10.0), (17.5, 13.5), (16.0, 17.5), (12.0, 19.8), (8.0, 17.5), (6.5, 13.5), (7.5, 10.0)])]),
        icon("edit-paste-history", "apps", vec![r(5.0, 5.0, 14.0, 16.0, 2.0), r(9.0, 3.0, 6.0, 4.0, 1.0), l(8.5, 12.0, 15.5, 12.0), l(8.5, 16.0, 13.0, 16.0)]),
        icon("system-software-install", "apps", cube()),
        icon("nexa-store", "apps", vec![r(4.5, 8.5, 15.0, 12.5, 2.5), a(12.0, 8.5, 3.6, 180.0, 360.0), l(12.0, 12.0, 12.0, 17.5), p(&[(9.5, 15.0), (12.0, 17.5), (14.5, 15.0)])]),
        icon("nexa-doctor", "apps", vec![h(12.0, 12.0, 9.0), l(12.0, 8.0, 12.0, 16.0), l(8.0, 12.0, 16.0, 12.0)]),
        icon("document-open-recent", "actions", vec![a(12.0, 12.0, 8.0, 200.0, 500.0), p(&[(1.6, 6.6), (4.48, 9.26), (7.4, 6.6)]), p(&[(12.0, 7.5), (12.0, 12.0), (15.0, 14.0)])]),
        icon("input-gaming", "devices", vec![r(2.5, 7.0, 19.0, 11.0, 5.5), l(6.5, 12.5, 10.5, 12.5), l(8.5, 10.5, 8.5, 14.5), fc(15.5, 11.3, 1.25), fc(17.6, 13.8, 1.25)]),
        // ---- network and power ----
        icon("network-wireless-signal-excellent", "status", wifi(3)),
        icon("network-wireless-signal-good", "status", wifi(2)),
        icon("network-wireless-signal-ok", "status", wifi(1)),
        icon("network-wireless-signal-none", "status", wifi(0)),
        icon("network-wireless-offline", "status", [wifi(0), vec![l(4.0, 4.0, 20.0, 20.0)]].concat()),
        icon("network-wired", "status", vec![r(9.0, 3.0, 6.0, 5.0, 1.0), r(3.0, 16.0, 6.0, 5.0, 1.0), r(15.0, 16.0, 6.0, 5.0, 1.0),
                                             l(12.0, 8.0, 12.0, 12.0), p(&[(6.0, 16.0), (6.0, 12.0), (18.0, 12.0), (18.0, 16.0)])]),
        icon("bluetooth-active", "status", vec![p(&[(7.0, 8.0), (17.0, 16.0), (12.0, 20.0), (12.0, 4.0), (17.0, 8.0), (7.0, 16.0)])]),
        icon("airplane-mode", "status", vec![fp(&[(12.0, 3.0), (13.3, 4.3), (13.3, 9.5), (20.5, 13.5), (20.5, 15.2), (13.3, 13.0), (13.3, 17.5), (15.2, 19.0),
                                                   (15.2, 20.6), (12.0, 19.6), (8.8, 20.6), (8.8, 19.0), (10.7, 17.5), (10.7, 13.0), (3.5, 15.2), (3.5, 13.5), (10.7, 9.5), (10.7, 4.3)])]),
        icon("battery-full", "status", battery(1.0)),
        icon("battery-good", "status", battery(0.7)),
        icon("battery-low", "status", battery(0.35)),
        icon("battery-caution", "status", battery(0.12)),
        icon("battery-empty", "status", battery(0.0)),
        icon("battery-charging", "status", [battery(0.0), vec![fp(&[(12.5, 7.8), (8.0, 12.8), (11.0, 12.8), (10.0, 16.2), (14.5, 11.2), (11.5, 11.2)])]].concat()),
        icon("display-brightness", "status", sun()),
        icon("weather-clear-night", "status", vec![a(12.0, 12.5, 8.5, 9.0, 268.6), a(16.8, 8.3, 6.6, 220.6, 57.0)]),
        // ---- places and people ----
        icon("folder", "places", folder()),
        icon("folder-download", "places", [folder(), vec![l(12.0, 11.0, 12.0, 16.5), p(&[(9.5, 14.0), (12.0, 16.5), (14.5, 14.0)])]].concat()),
        icon("user-home", "places", vec![p(&[(3.0, 11.0), (12.0, 3.5), (21.0, 11.0)]), p(&[(5.5, 9.0), (5.5, 20.0), (18.5, 20.0), (18.5, 9.0)]), r(10.0, 14.0, 4.0, 6.0, 1.0)]),
        icon("user-trash", "places", vec![l(4.0, 6.5, 20.0, 6.5), p(&[(9.0, 6.5), (9.0, 3.8), (15.0, 3.8), (15.0, 6.5)]), p(&[(6.0, 6.5), (7.0, 20.0), (17.0, 20.0), (18.0, 6.5)])]),
        icon("user-desktop", "places", vec![r(3.0, 4.0, 18.0, 12.5, 2.0), l(12.0, 16.5, 12.0, 20.0), l(8.5, 20.0, 15.5, 20.0)]),
        icon("avatar-default", "status", vec![c(12.0, 8.0, 4.0), a(12.0, 21.5, 8.0, 205.0, 335.0)]),
        icon("mail-unread", "status", vec![r(3.0, 5.0, 18.0, 14.0, 2.0), p(&[(3.5, 6.0), (12.0, 13.0), (20.5, 6.0)])]),
        icon("x-office-calendar", "mimetypes", vec![r(3.0, 5.0, 18.0, 16.0, 2.0), l(3.0, 10.0, 21.0, 10.0), l(8.0, 3.0, 8.0, 7.0), l(16.0, 3.0, 16.0, 7.0),
                                                    fc(8.0, 14.0, 1.1), fc(12.0, 14.0, 1.1), fc(16.0, 14.0, 1.1), fc(8.0, 17.5, 1.1), fc(12.0, 17.5, 1.1)]),
        icon("alarm", "status", vec![c(12.0, 12.0, 9.0), p(&[(12.0, 7.0), (12.0, 12.0), (15.5, 14.0)])]),
        icon("web-browser", "apps", vec![c(12.0, 12.0, 9.0), l(3.0, 12.0, 21.0, 12.0), e(12.0, 12.0, 4.0, 9.0)]),
        icon("find-location", "status", vec![pc(&[(12.0, 21.0), (7.2, 14.5), (6.0, 10.5), (7.3, 6.6), (10.2, 4.2), (13.8, 4.2), (16.7, 6.6), (18.0, 10.5), (16.8, 14.5)]), c(12.0, 10.2, 2.4)]),
        // ---- devices ----
        icon("input-keyboard", "devices", vec![r(3.0, 6.0, 18.0, 12.0, 2.0), fc(7.0, 10.0, 1.0), fc(10.3, 10.0, 1.0), fc(13.6, 10.0, 1.0), fc(17.0, 10.0, 1.0), l(8.0, 14.5, 16.0, 14.5)]),
        icon("input-mouse", "devices", vec![r(7.0, 3.0, 10.0, 18.0, 5.0), l(12.0, 7.0, 12.0, 10.0)]),
        icon("computer-laptop", "devices", vec![r(5.0, 5.0, 14.0, 10.5, 1.5), l(3.0, 19.0, 21.0, 19.0)]),
        icon("phone", "devices", vec![r(7.0, 3.0, 10.0, 18.0, 2.5), l(11.0, 18.0, 13.0, 18.0)]),
        icon("printer", "devices", vec![p(&[(7.0, 9.0), (7.0, 3.0), (17.0, 3.0), (17.0, 9.0)]), r(3.0, 9.0, 18.0, 8.0, 2.0), r(7.0, 14.0, 10.0, 7.0, 1.0)]),
        icon("drive-removable-media", "devices", vec![r(7.0, 9.0, 10.0, 12.0, 2.0), p(&[(9.0, 9.0), (9.0, 3.5), (15.0, 3.5), (15.0, 9.0)]), fr(10.5, 5.2, 1.1, 1.6, 0.3), fr(12.4, 5.2, 1.1, 1.6, 0.3)]),
        icon("drive-harddisk", "devices", vec![r(3.0, 7.0, 18.0, 10.0, 2.0), l(3.0, 13.0, 21.0, 13.0), fc(17.0, 15.0, 0.9)]),
        // ---- file types ----
        icon("text-x-generic", "mimetypes", [file_outline(), vec![l(9.0, 13.0, 16.0, 13.0), l(9.0, 17.0, 16.0, 17.0)]].concat()),
        icon("application-pdf", "mimetypes", [file_outline(), vec![fr(8.5, 12.0, 8.0, 5.5, 1.0)]].concat()),
        icon("image-x-generic", "mimetypes", vec![r(3.0, 4.0, 18.0, 16.0, 2.0), fc(8.5, 9.0, 1.8), p(&[(4.0, 18.0), (9.0, 13.0), (12.0, 16.0), (15.0, 12.0), (20.0, 17.0)])]),
        icon("video-x-generic", "mimetypes", vec![r(3.0, 5.0, 18.0, 14.0, 2.0), fp(&[(10.0, 9.0), (15.0, 12.0), (10.0, 15.0)])]),
        icon("audio-x-generic", "mimetypes", vec![fc(7.5, 17.0, 2.8), fc(16.5, 15.0, 2.8), p(&[(10.2, 17.0), (10.2, 6.0), (19.2, 4.0), (19.2, 15.0)])]),
        icon("package-x-generic", "mimetypes", cube()),
        icon("text-x-script", "mimetypes", vec![p(&[(8.0, 7.0), (3.0, 12.0), (8.0, 17.0)]), p(&[(16.0, 7.0), (21.0, 12.0), (16.0, 17.0)]), l(14.0, 5.0, 10.0, 19.0)]),
    ];
    v.sort_by_key(|i| (i.context, i.name));
    v
}

/// The app icons: id (the .desktop Icon= name), title, glyph, gradient.
pub fn apps() -> Vec<App> {
    let app = |id, title, glyph, top, bottom| App { id, title, glyph, top, bottom };
    vec![
        app("nexa-nexus", "Nexus", "nexa", "#5fb0ff", "#1c4fb8"),
        app("nexa-files", "Files", "folder", "#ffc45e", "#e8891c"),
        app("nexa-browser", "Browser", "web-browser", "#5fb4ff", "#2a62d8"),
        app("nexa-settings", "Settings", "preferences-system", "#94a2c4", "#46527a"),
        app("nexa-terminal", "Terminal", "utilities-terminal", "#56607e", "#1d2336"),
        app("nexa-store", "Store", "nexa-store", "#8a8cff", "#4b3fd6"),
        app("nexa-ai", "Nexa AI", "nexa-ai", "#b98bff", "#e0509a"),
        app("nexa-notes", "Notes", "accessories-notes", "#ffd56b", "#f0a33a"),
        app("nexa-music", "Music", "audio-x-generic", "#ff86ab", "#dd2f68"),
        app("nexa-video", "Video Player", "video-x-generic", "#ff9566", "#e1452b"),
        app("nexa-viewer", "Photos", "image-x-generic", "#57dccb", "#1c9a8b"),
        app("nexa-pdf", "PDF Viewer", "application-pdf", "#ff7676", "#c22c37"),
        app("nexa-archive", "Archive", "package-x-generic", "#dcae76", "#9a5f2c"),
        app("nexa-calc", "Calculator", "accessories-calculator", "#62e8ad", "#169e69"),
        app("nexa-edit", "Text Editor", "accessories-text-editor", "#55d6ff", "#1683cc"),
        app("nexa-pulse", "Pulse", "utilities-system-monitor", "#4be3a6", "#0f8a6a"),
        app("nexa-screenshot", "Screenshot", "accessories-screenshot", "#ad92ff", "#6a47dc"),
        app("nexa-chars", "Characters", "accessories-character-map", "#ffb35c", "#ea6d2d"),
        app("nexa-clipboard", "Clipboard", "edit-paste-history", "#8fb3ff", "#4568cc"),
        app("nexa-colorpicker", "Colour Picker", "color-select", "#ff8ad8", "#8a5cff"),
        app("nexa-power", "Power", "system-shutdown", "#ff7a7a", "#c3283b"),
        app("nexa-lock", "Lock", "system-lock-screen", "#7d8cff", "#3a3fb8"),
        app("nexa-network", "Network", "network-wireless-signal-excellent", "#58b8ff", "#1f72d6"),
        app("nexa-volume", "Sound", "audio-volume-high", "#4fd8d0", "#178f9a"),
        app("nexa-installer", "Installer", "system-software-install", "#5fd0ff", "#2563c9"),
        app("nexa-setup", "Setup", "document-download", "#7aa2ff", "#3646b8"),
        app("nexa-software", "Software", "view-grid", "#8f9bc4", "#4a5578"),
        app("nexa-doctor", "Nexa Doctor", "nexa-doctor", "#5ee6a8", "#14946a"),
        app("nexa-patterns", "For You", "starred", "#ffc861", "#e86f9a"),
        app("nexa-privacy", "Privacy", "security-high", "#7d93b8", "#3a4a70"),
        app("nexa-mail", "Mail", "mail-unread", "#62c0ff", "#2b6fd8"),
        app("nexa-calendar", "Calendar", "x-office-calendar", "#ff7d7d", "#d23a4f"),
        app("nexa-clock", "Clock", "alarm", "#4d5a86", "#1b2246"),
        app("nexa-camera", "Camera", "camera-photo", "#6b7590", "#2c3248"),
        app("nexa-weather", "Weather", "display-brightness", "#ffd15c", "#f4842a"),
        app("nexa-maps", "Maps", "find-location", "#6ee08c", "#1f9e5a"),
        app("nexa-backup", "Backup", "document-open-recent", "#56d4e8", "#1a7fa8"),
        app("nexa-contacts", "Contacts", "avatar-default", "#9a8cff", "#5a46c8"),
        app("nexa-recorder", "Recorder", "audio-input-microphone", "#ff7f8f", "#c8304c"),
        app("nexa-games", "Games", "input-gaming", "#78e07a", "#2b9a46"),
        app("nexa-help", "Help", "help-about", "#6fb6ff", "#2d6bd0"),
    ]
}
