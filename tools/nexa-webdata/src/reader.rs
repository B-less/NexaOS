//! Reader view: the article in a web page, without menus, ads, sidebars and
//! scripts. Paragraph-rich containers are scored (text length, commas,
//! class and id words, how much of it is links); the best one is written
//! out again from scratch with a short list of safe tags, so nothing from
//! the page that could run or track survives.

use crate::html::{escape, Doc};
use std::collections::HashMap;

const UNLIKELY: [&str; 24] = ["comment", "share", "social", "sidebar", "footer", "nav", "menu", "related", "promo", "advert", "ad-", "ads",
    "banner", "cookie", "subscribe", "newsletter", "popup", "modal", "sponsor", "breadcrumb", "masthead", "widget", "skip", "signup"];
const LIKELY: [&str; 9] = ["article", "content", "main", "post", "story", "body", "entry", "text", "page"];
const DROP: [&str; 16] = ["script", "style", "noscript", "nav", "aside", "footer", "form", "button", "iframe", "svg", "select", "input",
    "textarea", "template", "object", "embed"];

pub struct Article {
    pub title: String,
    pub byline: String,
    pub html: String,
    pub words: usize,
}

fn class_words(d: &Doc, n: usize) -> String {
    let node = &d.nodes[n];
    format!("{} {} {}", node.attr("class").unwrap_or(""), node.attr("id").unwrap_or(""), node.attr("role").unwrap_or("")).to_ascii_lowercase()
}

fn unlikely(d: &Doc, n: usize) -> bool {
    let w = class_words(d, n);
    if w.trim().is_empty() || matches!(d.nodes[n].tag.as_str(), "body" | "article" | "main") {
        return false;
    }
    UNLIKELY.iter().any(|u| w.contains(u)) && !LIKELY.iter().any(|l| w.contains(l))
}

fn dropped(d: &Doc, n: usize) -> bool {
    let node = &d.nodes[n];
    DROP.contains(&node.tag.as_str()) || unlikely(d, n) || node.attr("hidden").is_some()
        || node.attr("aria-hidden") == Some("true") || node.attr("style").map_or(false, |s| s.replace(' ', "").contains("display:none"))
}

/// Text length inside links under n, and all text length, skipping dropped parts.
fn lengths(d: &Doc, n: usize) -> (usize, usize) {
    let (mut links, mut all) = (0, 0);
    let mut todo = vec![(n, false)];
    while let Some((x, in_link)) = todo.pop() {
        let node = &d.nodes[x];
        if node.tag.is_empty() {
            let len = node.text.trim().chars().count();
            all += len;
            if in_link {
                links += len;
            }
            continue;
        }
        if x != n && dropped(d, x) {
            continue;
        }
        let link = in_link || node.tag == "a";
        todo.extend(node.children.iter().map(|&c| (c, link)));
    }
    (links, all)
}

/// Resolve a link from the page against its address.
pub fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(href.to_string());
    }
    if lower.starts_with("data:image/") && !lower.starts_with("data:image/svg") {
        return Some(href.to_string());
    }
    // A scheme (javascript:, mailto: ...) is a ':' before any '/': left out.
    let scheme_like = match (lower.find(':'), lower.find('/')) {
        (Some(c), Some(s)) => c < s,
        (Some(_), None) => true,
        _ => false,
    };
    if href.is_empty() || href.starts_with('#') || scheme_like {
        return None;
    }
    let scheme_end = base.find("://")?;
    let scheme = &base[..scheme_end];
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    if let Some(rest) = href.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    let after = &base[scheme_end + 3..];
    let host_end = after.find(['/', '?', '#']).map_or(base.len(), |p| scheme_end + 3 + p);
    let origin = &base[..host_end];
    if href.starts_with('/') {
        return Some(format!("{origin}{href}"));
    }
    let path = &base[host_end..];
    let path = &path[..path.find(['?', '#']).unwrap_or(path.len())];
    let dir = &path[..path.rfind('/').map_or(0, |p| p + 1)];
    let dir = if dir.is_empty() { "/" } else { dir };
    // Tidy "./" and "../".
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    let (rel, query) = href.split_at(href.find(['?', '#']).unwrap_or(href.len()));
    for seg in rel.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let trailing = if rel.ends_with('/') { "/" } else { "" };
    Some(format!("{origin}/{}{trailing}{query}", parts.join("/")))
}

fn meta(d: &Doc, names: &[&str]) -> Option<String> {
    d.find_all("meta").into_iter().find_map(|m| {
        let n = &d.nodes[m];
        let key = n.attr("property").or_else(|| n.attr("name")).unwrap_or("").to_ascii_lowercase();
        names.contains(&key.as_str()).then(|| n.attr("content").unwrap_or("").trim().to_string()).filter(|s| !s.is_empty())
    })
}

/// Write the safe copy of n's contents.
fn emit(d: &Doc, n: usize, base: &str, out: &mut String, depth: usize, title: &str) {
    if depth > 64 {
        return;
    }
    for &c in &d.nodes[n].children {
        let node = &d.nodes[c];
        if node.tag.is_empty() {
            out.push_str(&escape(&node.text));
            continue;
        }
        if dropped(d, c) {
            continue;
        }
        let tag = node.tag.as_str();
        match tag {
            "p" | "ul" | "ol" | "li" | "blockquote" | "pre" | "code" | "em" | "i" | "strong" | "b" | "figure" | "figcaption"
            | "table" | "thead" | "tbody" | "tr" | "th" | "td" | "sup" | "sub" | "dl" | "dt" | "dd" | "h3" | "h4" | "h5" | "h6" | "small" | "mark" | "q" | "cite" => {
                let mut inner = String::new();
                emit(d, c, base, &mut inner, depth + 1, title);
                if tag == "p" && inner.trim().is_empty() {
                    continue;
                }
                out.push_str(&format!("<{tag}>{inner}</{tag}>"));
            }
            "h1" | "h2" => {
                let text = d.text(c);
                // The page's title repeated as a heading: once is enough.
                if !text.is_empty() && text != title {
                    out.push_str(&format!("<h2>{}</h2>", escape(&text)));
                }
            }
            "a" => {
                let mut inner = String::new();
                emit(d, c, base, &mut inner, depth + 1, title);
                match node.attr("href").and_then(|h| resolve(base, h)).filter(|h| h.starts_with("http")) {
                    Some(h) => out.push_str(&format!("<a href=\"{}\" rel=\"noreferrer\">{inner}</a>", escape(&h))),
                    None => out.push_str(&inner),
                }
            }
            "img" => {
                let src = node.attr("src").or_else(|| node.attr("data-src")).and_then(|s| resolve(base, s));
                if let Some(src) = src {
                    out.push_str(&format!("<img src=\"{}\" alt=\"{}\" loading=\"lazy\">", escape(&src), escape(node.attr("alt").unwrap_or(""))));
                }
            }
            "br" => out.push_str("<br>"),
            "hr" => out.push_str("<hr>"),
            "div" | "section" | "article" | "main" | "span" | "header" | "center" | "font" | "u" | "picture" | "abbr" | "time" | "big" | "s" | "del" | "ins" => {
                let block = matches!(tag, "div" | "section" | "article" | "main" | "header" | "center");
                let mut inner = String::new();
                emit(d, c, base, &mut inner, depth + 1, title);
                // A block with bare text in it is a paragraph.
                if block && !inner.contains('<') && !inner.trim().is_empty() {
                    out.push_str(&format!("<p>{inner}</p>"));
                } else {
                    out.push_str(&inner);
                }
            }
            _ => {}
        }
    }
}

pub fn extract(html: &str, url: &str) -> Option<Article> {
    let d = Doc::parse(html);
    let title = meta(&d, &["og:title", "twitter:title"])
        .or_else(|| d.find_all("title").first().map(|&t| d.nodes[t].text.trim().to_string()))
        .or_else(|| d.find_all("h1").first().map(|&h| d.text(h)))
        .unwrap_or_default();
    let byline = meta(&d, &["author", "article:author"])
        .or_else(|| (0..d.nodes.len()).find(|&n| { let w = class_words(&d, n); w.contains("byline") || w.contains("author") }).map(|n| d.text(n)).filter(|t| t.len() < 120))
        .unwrap_or_default();

    // Which nodes are inside something dropped (cheap: walk down once).
    let mut hidden = vec![false; d.nodes.len()];
    for i in 1..d.nodes.len() {
        hidden[i] = hidden[d.nodes[i].parent] || (!d.nodes[i].tag.is_empty() && dropped(&d, i));
    }
    let mut score: HashMap<usize, f64> = HashMap::new();
    for i in 0..d.nodes.len() {
        if hidden[i] || !matches!(d.nodes[i].tag.as_str(), "p" | "pre" | "td" | "blockquote") {
            continue;
        }
        let text = d.text(i);
        let len = text.chars().count();
        if len < 25 {
            continue;
        }
        let s = 1.0 + text.matches(',').count() as f64 + (len as f64 / 100.0).min(3.0);
        let parent = d.nodes[i].parent;
        *score.entry(parent).or_insert(0.0) += s;
        if parent != 0 {
            *score.entry(d.nodes[parent].parent).or_insert(0.0) += s / 2.0;
        }
    }
    let best = score.iter().map(|(&n, &s)| {
        let w = class_words(&d, n);
        let mut s = s;
        if LIKELY.iter().any(|l| w.contains(l)) || matches!(d.nodes[n].tag.as_str(), "article" | "main") {
            s *= 1.25;
        }
        let (links, all) = lengths(&d, n);
        let density = if all > 0 { links as f64 / all as f64 } else { 1.0 };
        (n, s * (1.0 - density))
    }).max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).map(|(n, _)| n)?;
    let mut body = String::new();
    emit(&d, best, url, &mut body, 0, &title);
    let words = d.text(best).split_whitespace().count();
    (words >= 120).then(|| Article { title, byline, html: body, words })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> String {
        let para = "NexaOS readers like calm pages, with long sentences, commas, and no clutter around them at all. ";
        format!(r#"<html><head><title>Hexagons in Nature</title><meta name="author" content="Ama Mensah">
<script>track()</script></head><body>
<nav class="menu"><a href="/">Home</a> <a href="/news">News</a></nav>
<div id="sidebar" class="related"><p>{p}</p><a href="/x">Read more</a></div>
<article class="post-content"><h1>Hexagons in Nature</h1>
<p>{p}{p}</p><p>{p} <a href="../wiki/Bees">bees</a> and <a href="javascript:evil()">this</a>.</p>
<img src="/img/comb.jpg" alt="Honeycomb" onerror="steal()"><p onclick="x()">{p}{p}{p}</p>
<div class="share-buttons"><button>Share</button></div><script>evil()</script>
<p>{p}{p}</p><p>{p}{p}</p></article>
<footer>Copyright</footer></body></html>"#, p = para)
    }

    #[test]
    fn finds_the_article() {
        let a = extract(&page(), "https://site.example/blog/2026/hex.html").unwrap();
        assert_eq!(a.title, "Hexagons in Nature");
        assert_eq!(a.byline, "Ama Mensah");
        assert!(a.words > 120);
        assert!(a.html.contains("<a href=\"https://site.example/blog/wiki/Bees\""), "{}", a.html);
        assert!(a.html.contains("<img src=\"https://site.example/img/comb.jpg\" alt=\"Honeycomb\""));
        for bad in ["script", "onclick", "onerror", "javascript", "Share", "Copyright", "Read more", "Home</a>", "<h2>Hexagons in Nature"] {
            assert!(!a.html.contains(bad), "{bad} in {}", a.html);
        }
        assert!(extract("<html><body><p>Too short.</p></body></html>", "https://a.example/").is_none());
    }

    #[test]
    fn links() {
        let b = "https://a.example/x/y/page.html?q=1";
        assert_eq!(resolve(b, "img.png").as_deref(), Some("https://a.example/x/y/img.png"));
        assert_eq!(resolve(b, "../z/./i.png").as_deref(), Some("https://a.example/x/z/i.png"));
        assert_eq!(resolve(b, "/top?a=b").as_deref(), Some("https://a.example/top?a=b"));
        assert_eq!(resolve(b, "//cdn.example/i.png").as_deref(), Some("https://cdn.example/i.png"));
        assert_eq!(resolve(b, "javascript:alert(1)"), None);
        assert_eq!(resolve(b, "data:image/svg+xml,<svg onload=x>"), None);
        assert_eq!(resolve(b, "#top"), None);
    }
}
