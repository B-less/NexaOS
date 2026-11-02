//! Bookmarks in the "Netscape bookmark file" HTML that Firefox, Chrome,
//! Edge and Safari export: folders (<H3> followed by a <DL> list) and links
//! (<A HREF ADD_DATE>). Import reads one; export writes one.

use crate::html::escape;
use serde_json::{json, Value};

/// Only addresses worth opening: web pages (no javascript: bookmarklets,
/// no place: queries, no local files).
fn usable(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    (u.starts_with("https://") || u.starts_with("http://")) && url.len() < 8192
}

/// The text from `from` up to the end tag `</tag`, decoded; and where it ends.
fn until_end(html: &str, lower: &str, from: usize, tag: &str) -> (String, usize) {
    let end = lower[from..].find(&format!("</{tag}")).map_or(html.len(), |p| from + p);
    let text = crate::html::decode_entities(&html[from..end]);
    // Tags inside the text (rare): dropped.
    let mut clean = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => clean.push(c),
            _ => {}
        }
    }
    (clean.split_whitespace().collect::<Vec<_>>().join(" "), end)
}

/// (bookmarks [{title, url, folder, added}], links skipped). The file is
/// read as a sequence of tags: a folder's name (<H3>), its list (<DL> ...
/// </DL>), links (<A>) in whatever list is open.
pub fn import(html: &str) -> (Vec<Value>, usize) {
    let lower = html.to_ascii_lowercase();
    let b = html.as_bytes();
    let (mut out, mut skipped) = (Vec::new(), 0);
    let mut folders: Vec<Option<String>> = Vec::new();
    let mut pending: Option<String> = None;
    let mut i = 0;
    while let Some(p) = lower[i..].find('<') {
        let at = i + p;
        let close = b.get(at + 1) == Some(&b'/');
        let ns = at + 1 + close as usize;
        let ne = ns + lower[ns..].bytes().position(|c| !c.is_ascii_alphanumeric()).unwrap_or(lower.len() - ns);
        let tag = &lower[ns..ne];
        // The tag ends at the first '>' outside quotes.
        let mut j = ne;
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
        let attrs = &html[ne..j.min(b.len())];
        i = (j + 1).min(b.len());
        match (close, tag) {
            (false, "h3") => {
                let (name, end) = until_end(html, &lower, i, "h3");
                pending = Some(name.replace('/', "-"));
                i = end;
            }
            (false, "dl") => {
                if folders.len() < 64 {
                    folders.push(pending.take().filter(|n| !n.is_empty()));
                }
            }
            (true, "dl") => {
                folders.pop();
            }
            (false, "a") => {
                let a = crate::html::parse_attrs(attrs);
                let get = |k: &str| a.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim().to_string());
                let url = get("href").unwrap_or_default();
                let (title, end) = until_end(html, &lower, i, "a");
                i = end;
                if usable(&url) {
                    let added: u64 = get("add_date").and_then(|s| s.parse().ok()).unwrap_or(0);
                    // Some exports count microseconds.
                    let added = if added > 100_000_000_000 { added / 1_000_000 } else { added };
                    let folder = folders.iter().flatten().cloned().collect::<Vec<_>>().join("/");
                    out.push(json!({"title": if title.is_empty() { url.clone() } else { title }, "url": url, "folder": folder, "added": added}));
                } else if !url.is_empty() {
                    skipped += 1;
                }
            }
            _ => {}
        }
        if out.len() >= 100_000 {
            break;
        }
    }
    (out, skipped)
}

/// A bookmark file other browsers can import.
pub fn export(items: &[Value]) -> String {
    let mut s = String::from("<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<!-- This is an automatically generated file. -->\n\
<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n<TITLE>Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n<DL><p>\n");
    let mut by_folder: std::collections::BTreeMap<Vec<String>, Vec<&Value>> = Default::default();
    for b in items {
        let f: Vec<String> = b["folder"].as_str().unwrap_or("").split('/').filter(|p| !p.is_empty()).map(String::from).collect();
        by_folder.entry(f).or_default().push(b);
    }
    // Every folder on the way to a folder with links exists too.
    let keys: Vec<Vec<String>> = by_folder.keys().cloned().collect();
    for k in keys {
        for n in 1..k.len() {
            by_folder.entry(k[..n].to_vec()).or_default();
        }
    }
    fn write(s: &mut String, path: &[String], map: &std::collections::BTreeMap<Vec<String>, Vec<&Value>>, depth: usize) {
        let pad = "    ".repeat(depth + 1);
        for b in map.get(path).map(Vec::as_slice).unwrap_or(&[]) {
            let url = b["url"].as_str().unwrap_or("");
            if !usable(url) {
                continue;
            }
            s.push_str(&format!("{pad}<DT><A HREF=\"{}\" ADD_DATE=\"{}\">{}</A>\n", escape(url), b["added"].as_f64().unwrap_or(0.0) as u64,
                escape(b["title"].as_str().unwrap_or(url))));
        }
        for sub in map.keys().filter(|k| k.len() == path.len() + 1 && k.starts_with(path)) {
            s.push_str(&format!("{pad}<DT><H3>{}</H3>\n{pad}<DL><p>\n", escape(sub.last().unwrap())));
            write(s, sub, map, depth + 1);
            s.push_str(&format!("{pad}</DL><p>\n"));
        }
    }
    write(&mut s, &[], &by_folder, 0);
    s.push_str("</DL><p>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks Menu</H1>
<DL><p>
    <DT><A HREF="https://nexaos.example/" ADD_DATE="1700000000">NexaOS &amp; friends</A>
    <DT><H3 ADD_DATE="1" PERSONAL_TOOLBAR_FOLDER="true">Bookmarks Toolbar</H3>
    <DL><p>
        <DT><A HREF="https://news.example/">News</A>
        <DT><H3>Work</H3>
        <DL><p>
            <DT><A HREF="https://mail.example/" ADD_DATE="1700000000000000">Mail</A>
            <DT><A HREF="javascript:alert(1)">Bookmarklet</A>
        </DL><p>
    </DL><p>
    <DT><A HREF="place:sort=8">Recent</A>
</DL>"#;

    #[test]
    fn reads_firefox_and_round_trips() {
        let (items, skipped) = import(FIREFOX);
        let got: Vec<(String, String, String)> = items.iter().map(|b| (b["title"].as_str().unwrap().into(), b["url"].as_str().unwrap().into(), b["folder"].as_str().unwrap().into())).collect();
        assert_eq!(got, vec![
            ("NexaOS & friends".into(), "https://nexaos.example/".into(), "".into()),
            ("News".into(), "https://news.example/".into(), "Bookmarks Toolbar".into()),
            ("Mail".into(), "https://mail.example/".into(), "Bookmarks Toolbar/Work".into()),
        ], "{items:?}");
        assert_eq!(skipped, 2);
        assert_eq!(items[2]["added"], 1_700_000_000);
        let again = import(&export(&items)).0;
        assert_eq!(again.iter().map(|b| (&b["url"], &b["folder"], &b["title"])).collect::<Vec<_>>(),
                   items.iter().map(|b| (&b["url"], &b["folder"], &b["title"])).collect::<Vec<_>>());
        assert!(export(&[json!({"url": "javascript:x", "title": "x"})]).matches("<A ").count() == 0);
        assert_eq!(import("not a bookmark file").0.len(), 0);
    }
}
