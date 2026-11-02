//! A small, forgiving HTML reader: text and tags into a tree, the way
//! browsers mostly do it (unclosed paragraphs and list items closed for
//! you, stray end tags ignored, script/style contents skipped), plus
//! entity decoding and escaping. Used on untrusted pages and files, so it
//! never recurses on input depth and caps what it builds.

pub const MAX_NODES: usize = 300_000;
const MAX_DEPTH: usize = 512;

#[derive(Debug, Clone)]
pub struct Node {
    pub tag: String, // "" for text, "#root" for the root
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<usize>,
    pub parent: usize,
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub struct Doc {
    pub nodes: Vec<Node>,
}

const VOID: [&str; 15] = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr", "keygen"];
const RAW: [&str; 8] = ["script", "style", "noscript", "template", "textarea", "title", "xmp", "iframe"];
const CLOSES_P: [&str; 24] = ["p", "div", "ul", "ol", "dl", "h1", "h2", "h3", "h4", "h5", "h6", "table", "pre", "blockquote", "section",
    "article", "aside", "figure", "header", "footer", "nav", "form", "hr", "main"];

/// "&amp;", "&#39;", "&#x2014;", "&nbsp;"... to text.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[1..].find(|c: char| c == ';' || c == '&' || c.is_whitespace() || c == '<').map(|e| e + 1);
        let (name, after) = match end {
            Some(e) if rest.as_bytes().get(e) == Some(&b';') && e <= 12 => (&rest[1..e], &rest[e + 1..]),
            _ => {
                out.push('&');
                rest = &rest[1..];
                continue;
            }
        };
        let ch = if let Some(n) = name.strip_prefix('#') {
            let v = if let Some(h) = n.strip_prefix('x').or_else(|| n.strip_prefix('X')) { u32::from_str_radix(h, 16).ok() } else { n.parse().ok() };
            v.and_then(char::from_u32).filter(|c| *c != '\0').map(String::from)
        } else {
            match name {
                "amp" => Some("&"), "lt" => Some("<"), "gt" => Some(">"), "quot" => Some("\""), "apos" => Some("'"),
                "nbsp" => Some("\u{a0}"), "ndash" => Some("–"), "mdash" => Some("—"), "hellip" => Some("…"),
                "lsquo" => Some("‘"), "rsquo" => Some("’"), "ldquo" => Some("“"), "rdquo" => Some("”"), "copy" => Some("©"),
                "reg" => Some("®"), "trade" => Some("™"), "euro" => Some("€"), "pound" => Some("£"), "middot" => Some("·"),
                "bull" => Some("•"), "times" => Some("×"), "laquo" => Some("«"), "raquo" => Some("»"), "deg" => Some("°"),
                _ => None,
            }.map(String::from)
        };
        match ch {
            Some(c) => {
                out.push_str(&c);
                rest = after;
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

pub fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() && out.len() < 64 {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let name = s[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let vs = i + 1;
                let ve = s[vs..].bytes().position(|c| c == q).map_or(b.len(), |p| vs + p);
                value = s[vs..ve].to_string();
                i = ve + 1;
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = s[vs..i].to_string();
            }
        }
        out.push((name, decode_entities(&value)));
    }
    out
}

impl Doc {
    pub fn parse(html: &str) -> Doc {
        let mut nodes = vec![Node { tag: "#root".into(), attrs: vec![], text: String::new(), children: vec![], parent: 0 }];
        let mut stack: Vec<usize> = vec![0];
        let lower = html.to_ascii_lowercase();
        let b = html.as_bytes();
        let mut i = 0;
        let add = |nodes: &mut Vec<Node>, stack: &Vec<usize>, n: Node| -> Option<usize> {
            if nodes.len() >= MAX_NODES {
                return None;
            }
            let parent = *stack.last().unwrap();
            let id = nodes.len();
            nodes.push(Node { parent, ..n });
            nodes[parent].children.push(id);
            Some(id)
        };
        while i < b.len() {
            if b[i] != b'<' {
                let end = html[i..].find('<').map_or(b.len(), |p| i + p);
                let text = decode_entities(&html[i..end]);
                if !text.is_empty() {
                    add(&mut nodes, &stack, Node { tag: String::new(), attrs: vec![], text, children: vec![], parent: 0 });
                }
                i = end;
                continue;
            }
            if lower[i..].starts_with("<!--") {
                i = lower[i + 4..].find("-->").map_or(b.len(), |p| i + 4 + p + 3);
                continue;
            }
            if lower[i..].starts_with("<!") || lower[i..].starts_with("<?") {
                i = html[i..].find('>').map_or(b.len(), |p| i + p + 1);
                continue;
            }
            let close = b.get(i + 1) == Some(&b'/');
            let name_start = i + 1 + close as usize;
            let name_end = name_start + lower[name_start..].bytes().position(|c| !(c.is_ascii_alphanumeric() || c == b'-' || c == b':')).unwrap_or(lower.len() - name_start);
            if name_end == name_start {
                add(&mut nodes, &stack, Node { tag: String::new(), attrs: vec![], text: "<".into(), children: vec![], parent: 0 });
                i += 1;
                continue;
            }
            let tag = lower[name_start..name_end].to_string();
            // The tag ends at the first '>' outside quotes.
            let mut j = name_end;
            let mut quote = 0u8;
            while j < b.len() {
                let c = b[j];
                if quote != 0 {
                    if c == quote {
                        quote = 0;
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = c;
                } else if c == b'>' {
                    break;
                }
                j += 1;
            }
            let attr_text = &html[name_end..j.min(b.len())];
            i = (j + 1).min(b.len());
            if close {
                if let Some(pos) = stack.iter().rposition(|&n| nodes[n].tag == tag) {
                    if pos > 0 {
                        stack.truncate(pos);
                    }
                }
                continue;
            }
            // Browsers' implied end tags, the common cases.
            if CLOSES_P.contains(&tag.as_str()) {
                if let Some(pos) = stack.iter().rposition(|&n| nodes[n].tag == "p") {
                    if pos > 0 && stack[pos..].iter().all(|&n| !matches!(nodes[n].tag.as_str(), "div" | "td" | "li" | "blockquote" | "section" | "article")) {
                        stack.truncate(pos);
                    }
                }
            }
            if tag == "li" || tag == "dt" || tag == "dd" {
                if let Some(pos) = stack.iter().rposition(|&n| matches!(nodes[n].tag.as_str(), "li" | "dt" | "dd" | "ul" | "ol" | "dl")) {
                    if pos > 0 && matches!(nodes[stack[pos]].tag.as_str(), "li" | "dt" | "dd") {
                        stack.truncate(pos);
                    }
                }
            }
            if matches!(tag.as_str(), "tr" | "td" | "th") {
                let stop: &[&str] = if tag == "tr" { &["tr", "table"] } else { &["td", "th", "tr", "table"] };
                if let Some(pos) = stack.iter().rposition(|&n| stop.contains(&nodes[n].tag.as_str())) {
                    if pos > 0 && nodes[stack[pos]].tag != "table" && !(tag != "tr" && nodes[stack[pos]].tag == "tr") {
                        stack.truncate(pos);
                    }
                }
            }
            let self_closing = attr_text.trim_end().ends_with('/');
            let Some(id) = add(&mut nodes, &stack, Node { tag: tag.clone(), attrs: parse_attrs(attr_text), text: String::new(), children: vec![], parent: 0 }) else { break };
            if RAW.contains(&tag.as_str()) {
                let end_tag = format!("</{tag}");
                let end = lower[i..].find(&end_tag).map_or(b.len(), |p| i + p);
                if tag == "title" || tag == "textarea" {
                    let t = decode_entities(&html[i..end]);
                    nodes[id].text = t;
                }
                i = html[end..].find('>').map_or(b.len(), |p| end + p + 1);
                continue;
            }
            if !VOID.contains(&tag.as_str()) && !self_closing && stack.len() < MAX_DEPTH {
                stack.push(id);
            }
        }
        Doc { nodes }
    }

    /// All text under a node, spaces tidied.
    pub fn text(&self, n: usize) -> String {
        let mut out = String::new();
        let mut todo = vec![n];
        while let Some(x) = todo.pop() {
            let node = &self.nodes[x];
            if node.tag.is_empty() {
                out.push_str(&node.text);
            } else if node.tag == "br" || node.tag == "p" {
                out.push(' ');
            }
            if !RAW.contains(&node.tag.as_str()) {
                todo.extend(node.children.iter().rev());
            }
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    pub fn find_all(&self, tag: &str) -> Vec<usize> {
        (0..self.nodes.len()).filter(|&i| self.nodes[i].tag == tag).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forgiving() {
        let d = Doc::parse("<html><title>T &amp; t</title><body><p>one<p>two <b>bold<i>x</b> &mdash; &#8212; &#x2014; &bogus; a & b<ul><li>a<li>b</ul><script>if (a<b) {}</script></body>");
        let ps = d.find_all("p");
        assert_eq!(ps.len(), 2);
        assert_eq!(d.text(ps[0]), "one");
        assert_eq!(d.nodes[d.find_all("title")[0]].text, "T & t");
        assert_eq!(d.find_all("li").len(), 2);
        assert!(d.text(0).contains("— — — &bogus; a & b"), "{}", d.text(0));
        assert!(!d.text(0).contains("if (a"));
        let a = Doc::parse(r#"<a HREF="x?a=1&amp;b=2" data-x='q"z' disabled>l</a>"#);
        let n = &a.nodes[a.find_all("a")[0]];
        assert_eq!((n.attr("href"), n.attr("data-x"), n.attr("disabled")), (Some("x?a=1&b=2"), Some("q\"z"), Some("")));
        // Hostile: deep nesting, unclosed everything, junk.
        let deep = "<div>".repeat(5000) + "x";
        assert!(Doc::parse(&deep).nodes.len() > 1000);
        for s in ["<", "<<>>", "<a", "<a href='", "</", "<!--", "&#99999999;", "<p>\u{0}</p>"] {
            let _ = Doc::parse(s);
        }
        assert_eq!(escape("<a href=\"x\">&'"), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
    }
}
