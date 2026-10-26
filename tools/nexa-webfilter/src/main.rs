//! nexa-webfilter: Nexa Browser's ad and tracker blocking. Turns filter
//! lists into WebKit's content-blocker rules (JSON). WebKit compiles those
//! into a matcher inside the engine, so blocking costs no code per request
//! and nothing leaves the computer.
//!
//!   nexa-webfilter compile [--allow SITE]... LIST...      rules JSON on stdout,
//!                                                         a summary on stderr
//!   nexa-webfilter test URL [--from PAGE] [--allow SITE]... LIST...
//!                                                         would it be blocked? why?
//!
//! Understood: hosts files ("0.0.0.0 ads.example"), plain domain lines,
//! Adblock network rules ("||ads.example^", "/banner/ads.", with $third-party,
//! $first-party, $domain=a|~b, $script, $image, $stylesheet, $font, $media,
//! $xmlhttprequest, $subdocument, $popup, $match-case), exceptions ("@@..."),
//! and element hiding ("##.ad-box", "example.com##.promo"). Rules WebKit's
//! blocker can't express (redirects, CSP, scriptlets, regular expressions,
//! negated types) are skipped and counted, never guessed at.
//!
//! --allow SITE: blocking off on that site (and its subdomains), what the
//! browser's "Allow ads on this site" does.

use serde_json::{json, Value};
use std::collections::BTreeSet;

const MAX_RULES: usize = 50_000;
const SELECTORS_PER_RULE: usize = 200;

#[derive(Clone, Debug, PartialEq)]
enum Pattern {
    /// "||host^rest": the host or a subdomain, then `rest` right after it.
    Host { host: String, rest: String },
    /// Anywhere in the URL ("*" any run, "^" a separator).
    Anywhere(String),
}

#[derive(Clone, Debug, PartialEq)]
struct Net {
    allow: bool,
    pattern: Pattern,
    third_party: Option<bool>,
    types: Vec<&'static str>,
    if_domains: Vec<String>,
    unless_domains: Vec<String>,
    case_sensitive: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Hide {
    selector: String,
    domains: Vec<String>,
}

#[derive(Default, Debug)]
struct Parsed {
    nets: Vec<Net>,
    hides: Vec<Hide>,
    skipped: usize,
}

fn valid_host(h: &str) -> bool {
    !h.is_empty() && h.len() <= 253 && h.contains('.') && !h.starts_with('.') && !h.ends_with('.')
        && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn parse_options(opts: &str, n: &mut Net) -> bool {
    for o in opts.split(',').map(str::trim).filter(|o| !o.is_empty()) {
        let o = o.to_ascii_lowercase();
        match o.as_str() {
            "third-party" | "3p" => n.third_party = Some(true),
            "~third-party" | "first-party" | "1p" | "~3p" => n.third_party = Some(false),
            "script" => n.types.push("script"),
            "image" => n.types.push("image"),
            "stylesheet" | "css" => n.types.push("style-sheet"),
            "font" => n.types.push("font"),
            "media" => n.types.push("media"),
            "xmlhttprequest" | "xhr" | "websocket" | "ping" | "other" | "object" => n.types.push("raw"),
            "subdocument" | "frame" => n.types.push("document"),
            "popup" => n.types.push("popup"),
            "document" | "doc" => n.types.push("document"),
            "match-case" => n.case_sensitive = true,
            "important" | "all" => {}
            _ if o.starts_with("domain=") => {
                for d in o["domain=".len()..].split('|') {
                    let (neg, d) = d.strip_prefix('~').map(|d| (true, d)).unwrap_or((false, d));
                    if !valid_host(d) {
                        return false;
                    }
                    if neg { n.unless_domains.push(d.into()) } else { n.if_domains.push(d.into()) }
                }
            }
            // Redirects, CSP, scriptlets, negated types, ...: not expressible.
            _ => return false,
        }
    }
    // WebKit takes if-domain or unless-domain on one rule, not both.
    !(n.if_domains.len() > 0 && n.unless_domains.len() > 0)
}

fn parse_line(line: &str, out: &mut Parsed) {
    let l = line.trim();
    if l.is_empty() || l.starts_with('!') || l.starts_with('[') || (l.starts_with('#') && !l.starts_with("##")) {
        return;
    }
    // Element hiding.
    if let Some(i) = l.find("##") {
        let (doms, sel) = (&l[..i], l[i + 2..].trim());
        if sel.is_empty() || sel.starts_with('+') || sel.contains(":has-text(") || sel.contains(":-abp-") {
            out.skipped += 1;
            return;
        }
        let domains: Vec<String> = doms.split(',').map(|d| d.trim().to_ascii_lowercase()).filter(|d| !d.is_empty()).collect();
        if domains.iter().any(|d| d.starts_with('~') || !valid_host(d)) {
            out.skipped += 1;
            return;
        }
        out.hides.push(Hide { selector: sel.to_string(), domains });
        return;
    }
    if l.contains("#@#") || l.contains("#?#") || l.contains("#$#") || l.contains("#%#") {
        out.skipped += 1;
        return;
    }
    // Hosts files.
    let words: Vec<&str> = l.split_whitespace().collect();
    if words.len() >= 2 && ["0.0.0.0", "127.0.0.1", "::", "::1"].contains(&words[0]) {
        let h = words[1].to_ascii_lowercase();
        if valid_host(&h) && h != "localhost.localdomain" {
            out.nets.push(Net { allow: false, pattern: Pattern::Host { host: h, rest: String::new() }, third_party: None,
                                types: vec![], if_domains: vec![], unless_domains: vec![], case_sensitive: false });
        }
        return;
    }
    if words.len() != 1 {
        out.skipped += 1;
        return;
    }
    let (allow, body) = match l.strip_prefix("@@") { Some(b) => (true, b), None => (false, l) };
    let (pat, opts) = match body.rfind('$') {
        Some(i) if !body[i + 1..].contains('/') => (&body[..i], &body[i + 1..]),
        _ => (body, ""),
    };
    // Regular-expression rules and start/end anchors: not expressible safely.
    if pat.starts_with('/') && pat.ends_with('/') && pat.len() > 1 || (pat.starts_with('|') && !pat.starts_with("||"))
        || pat.ends_with('|') {
        out.skipped += 1;
        return;
    }
    let mut n = Net { allow, pattern: Pattern::Anywhere(String::new()), third_party: None, types: vec![], if_domains: vec![],
                      unless_domains: vec![], case_sensitive: false };
    if !parse_options(opts, &mut n) {
        out.skipped += 1;
        return;
    }
    if let Some(rest) = pat.strip_prefix("||") {
        let end = rest.find(|c: char| c == '^' || c == '/' || c == '*' || c == ':').unwrap_or(rest.len());
        let host = rest[..end].to_ascii_lowercase();
        if !valid_host(&host) {
            out.skipped += 1;
            return;
        }
        n.pattern = Pattern::Host { host, rest: rest[end..].to_string() };
    } else if valid_host(pat) && opts.is_empty() && !allow {
        // A plain domain line (domain lists).
        n.pattern = Pattern::Host { host: pat.to_ascii_lowercase(), rest: String::new() };
    } else {
        // Too short to mean anything ("ad"), or nothing but wildcards: skipped.
        if pat.trim_matches(|c| c == '*' || c == '^').len() < 4 {
            out.skipped += 1;
            return;
        }
        n.pattern = Pattern::Anywhere(pat.to_string());
    }
    out.nets.push(n);
}

fn parse(text: &str) -> Parsed {
    let mut p = Parsed::default();
    for line in text.lines() {
        parse_line(line, &mut p);
    }
    p
}

/// A wildcard pattern piece as WebKit's (restricted) regular expression.
fn regex_of(s: &str) -> String {
    let mut r = String::new();
    for c in s.chars() {
        match c {
            '*' => r.push_str(".*"),
            '^' => r.push_str("[/:?=&]"),
            '.' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '\\' | '|' | '$' => {
                r.push('\\');
                r.push(c);
            }
            _ => r.push(c),
        }
    }
    r
}

fn url_filter(p: &Pattern) -> String {
    match p {
        Pattern::Host { host, rest } => {
            let tail = if rest.is_empty() || rest == "^" { "[:/]".to_string() } else { regex_of(rest) };
            format!("^[^:]+://+([^:/]+\\.)?{}{tail}", regex_of(host))
        }
        Pattern::Anywhere(s) => regex_of(s.trim_start_matches('*')),
    }
}

fn trigger(n: &Net) -> Value {
    let mut t = json!({"url-filter": url_filter(&n.pattern)});
    if n.case_sensitive {
        t["url-filter-is-case-sensitive"] = json!(true);
    }
    if let Some(tp) = n.third_party {
        t["load-type"] = json!([if tp { "third-party" } else { "first-party" }]);
    }
    if !n.types.is_empty() {
        let set: BTreeSet<&str> = n.types.iter().copied().collect();
        t["resource-type"] = json!(set);
    }
    if !n.if_domains.is_empty() {
        t["if-domain"] = json!(n.if_domains.iter().map(|d| format!("*{d}")).collect::<Vec<_>>());
    }
    if !n.unless_domains.is_empty() {
        t["unless-domain"] = json!(n.unless_domains.iter().map(|d| format!("*{d}")).collect::<Vec<_>>());
    }
    t
}

struct Compiled {
    rules: Vec<Value>,
    blocks: usize,
    hides: usize,
    exceptions: usize,
    skipped: usize,
    truncated: usize,
}

fn compile(lists: &[String], allow_sites: &[String]) -> Compiled {
    let mut p = Parsed::default();
    for text in lists {
        let q = parse(text);
        p.nets.extend(q.nets);
        p.hides.extend(q.hides);
        p.skipped += q.skipped;
    }
    let mut seen = BTreeSet::new();
    let mut blocks = Vec::new();
    let mut exceptions = Vec::new();
    for n in &p.nets {
        let rule = json!({"trigger": trigger(n), "action": {"type": if n.allow { "ignore-previous-rules" } else { "block" }}});
        if seen.insert(rule.to_string()) {
            if n.allow { exceptions.push(rule) } else { blocks.push(rule) }
        }
    }
    // Element hiding: many selectors per rule (one per set of sites).
    let mut hides = Vec::new();
    let mut groups: std::collections::BTreeMap<Vec<String>, BTreeSet<String>> = Default::default();
    for h in &p.hides {
        groups.entry(h.domains.clone()).or_default().insert(h.selector.clone());
    }
    for (domains, sels) in groups {
        let sels: Vec<String> = sels.into_iter().collect();
        for chunk in sels.chunks(SELECTORS_PER_RULE) {
            let mut t = json!({"url-filter": ".*"});
            if !domains.is_empty() {
                t["if-domain"] = json!(domains.iter().map(|d| format!("*{d}")).collect::<Vec<_>>());
            }
            hides.push(json!({"trigger": t, "action": {"type": "css-display-none", "selector": chunk.join(", ")}}));
        }
    }
    let (nb, nh, ne) = (blocks.len(), hides.len(), exceptions.len());
    // Exceptions come after what they except; sites with blocking off last.
    let mut rules: Vec<Value> = blocks.into_iter().chain(hides).chain(exceptions).collect();
    // A site switch may name a one-word host too (localhost, a home server).
    let site_ok = |s: &&String| !s.is_empty() && s.len() <= 253 && !s.starts_with('.')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    let site_rules: Vec<Value> = allow_sites.iter().filter(site_ok)
        .map(|s| json!({"trigger": {"url-filter": ".*", "if-domain": [format!("*{}", s.to_ascii_lowercase())]},
                        "action": {"type": "ignore-previous-rules"}}))
        .collect();
    let room = MAX_RULES.saturating_sub(site_rules.len());
    let truncated = rules.len().saturating_sub(room);
    rules.truncate(room);
    rules.extend(site_rules);
    Compiled { rules, blocks: nb, hides: nh, exceptions: ne, skipped: p.skipped, truncated }
}

// ---- matching (for `test`: the same rules, checked here) ------------------------

fn wild(pat: &[u8], s: &[u8]) -> bool {
    // Anchored at the start of `s`; may end anywhere.
    match pat.first() {
        None => true,
        Some(b'*') => (0..=s.len()).any(|i| wild(&pat[1..], &s[i..])),
        Some(b'^') => match s.first() {
            None => pat.len() == 1,
            Some(c) if !c.is_ascii_alphanumeric() && !b"_-.%".contains(c) => wild(&pat[1..], &s[1..]),
            _ => false,
        },
        Some(p) => s.first().is_some_and(|c| c == p) && wild(&pat[1..], &s[1..]),
    }
}

fn host_of(url: &str) -> String {
    let after = url.split_once("://").map(|x| x.1).unwrap_or(url);
    after.split(|c| c == '/' || c == '?' || c == '#').next().unwrap_or("").rsplit('@').next().unwrap_or("")
        .split(':').next().unwrap_or("").to_ascii_lowercase()
}

fn under(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// The last two labels: good enough to tell sites apart for these tests.
fn site(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').collect();
    parts[parts.len().saturating_sub(2)..].join(".")
}

fn net_matches(n: &Net, url: &str, page: &str) -> bool {
    let host = host_of(url);
    let (u, phost) = (if n.case_sensitive { url.to_string() } else { url.to_ascii_lowercase() }, host_of(page));
    let hit = match &n.pattern {
        Pattern::Host { host: h, rest } => {
            if !under(&host, h) {
                return false;
            }
            let after_host = u.split_once("://").map(|x| x.1).unwrap_or(&u);
            let tail = &after_host[after_host.find(&host).map(|i| i + host.len()).unwrap_or(0)..];
            rest.is_empty() || wild(rest.to_ascii_lowercase().as_bytes(), tail.as_bytes())
        }
        Pattern::Anywhere(p) => {
            let p = if n.case_sensitive { p.clone() } else { p.to_ascii_lowercase() };
            (0..u.len()).any(|i| wild(p.as_bytes(), &u.as_bytes()[i..]))
        }
    };
    hit && n.third_party.map_or(true, |tp| tp == (site(&host) != site(&phost)))
        && (n.if_domains.is_empty() || n.if_domains.iter().any(|d| under(&phost, d)))
        && !n.unless_domains.iter().any(|d| under(&phost, d))
}

fn verdict(lists: &[String], allow: &[String], url: &str, page: &str) -> (bool, String) {
    let mut p = Parsed::default();
    for t in lists {
        let q = parse(t);
        p.nets.extend(q.nets);
    }
    let phost = host_of(page);
    if allow.iter().any(|s| under(&phost, &s.to_ascii_lowercase())) {
        return (false, format!("blocking is off on {phost}"));
    }
    if let Some(e) = p.nets.iter().find(|n| n.allow && net_matches(n, url, page)) {
        return (false, format!("allowed by an exception ({:?})", e.pattern));
    }
    match p.nets.iter().find(|n| !n.allow && net_matches(n, url, page)) {
        Some(b) => (true, format!("blocked ({:?})", b.pattern)),
        None => (false, "no rule matches".into()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut allow = Vec::new();
    let mut from = String::new();
    let mut files = Vec::new();
    let mut it = args.iter().skip(1).peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--allow" => allow.extend(it.next().cloned()),
            "--from" => from = it.next().cloned().unwrap_or_default(),
            _ => files.push(a.clone()),
        }
    }
    let read = |fs: &[String]| -> Vec<String> {
        fs.iter().filter_map(|f| match std::fs::read(f) {
            Ok(b) => Some(String::from_utf8_lossy(&b[..b.len().min(32 << 20)]).into_owned()),
            Err(e) => {
                eprintln!("nexa-webfilter: {f}: {e}");
                None
            }
        }).collect()
    };
    match args.first().map(String::as_str) {
        Some("compile") => {
            let c = compile(&read(&files), &allow);
            println!("{}", Value::Array(c.rules));
            eprintln!("{{\"blocks\": {}, \"hiding\": {}, \"exceptions\": {}, \"skipped\": {}, \"truncated\": {}, \"sites_off\": {}}}",
                      c.blocks, c.hides, c.exceptions, c.skipped, c.truncated, allow.len());
        }
        Some("test") if !files.is_empty() => {
            let url = files.remove(0);
            let page = if from.is_empty() { url.clone() } else { from };
            let (blocked, why) = verdict(&read(&files), &allow, &url, &page);
            println!("{} {why}", if blocked { "BLOCKED" } else { "ALLOWED" });
            std::process::exit(if blocked { 0 } else { 1 });
        }
        _ => {
            eprintln!("usage: nexa-webfilter compile [--allow SITE]... LIST...\n       \
                       nexa-webfilter test URL [--from PAGE] [--allow SITE]... LIST...");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "! a comment\n[Adblock Plus 2.0]\n0.0.0.0 tracker.example\n127.0.0.1 localhost.localdomain\n\
        ||ads.example^\n||cdn.example/banners/*\n||pixel.example^$third-party,image\n||site.example^$domain=news.example|blog.example\n\
        @@||ads.example/allowed/\n/adframe.\n##.ad-box\nnews.example##.promo\nplain-domain.example\n\
        ||redirected.example^$redirect=noop.js\n/^https?:\\/\\/regex/\nexample.com#@#.x\n||both.example^$domain=a.example|~b.example\n";

    #[test]
    fn parses_what_it_can_and_counts_the_rest() {
        let p = parse(LIST);
        assert!(p.nets.iter().any(|n| n.pattern == Pattern::Host { host: "tracker.example".into(), rest: String::new() }));
        assert!(!p.nets.iter().any(|n| matches!(&n.pattern, Pattern::Host { host, .. } if host == "localhost.localdomain")));
        assert!(p.nets.iter().any(|n| n.pattern == Pattern::Host { host: "plain-domain.example".into(), rest: String::new() }));
        assert_eq!(p.hides.len(), 2);
        // redirect, regex, #@#, if+unless domains: skipped, not guessed.
        assert_eq!(p.skipped, 4, "{p:?}");
    }

    #[test]
    fn compiles_to_webkit_rules_in_the_right_order() {
        let c = compile(&[LIST.to_string()], &["mysite.example".into()]);
        let r = &c.rules;
        let first_exception = r.iter().position(|x| x["action"]["type"] == "ignore-previous-rules").unwrap();
        let last_block = r.iter().rposition(|x| x["action"]["type"] == "block").unwrap();
        assert!(last_block < first_exception, "exceptions after blocks");
        assert_eq!(r.last().unwrap()["trigger"]["if-domain"][0], "*mysite.example", "the site switch is last");
        let c2 = compile(&[LIST.to_string()], &["localhost".into(), "bad site".into()]);
        assert_eq!(c2.rules.last().unwrap()["trigger"]["if-domain"][0], "*localhost", "one-word hosts can be switched off too");
        assert_eq!(c2.rules.len(), c.rules.len(), "a malformed site name is ignored");
        let px = r.iter().find(|x| x["trigger"]["url-filter"].as_str().unwrap().contains("pixel")).unwrap();
        assert_eq!(px["trigger"]["load-type"][0], "third-party");
        assert_eq!(px["trigger"]["resource-type"][0], "image");
        assert_eq!(px["trigger"]["url-filter"], "^[^:]+://+([^:/]+\\.)?pixel\\.example[:/]");
        let hide = r.iter().find(|x| x["action"]["type"] == "css-display-none" && x["trigger"].get("if-domain").is_none()).unwrap();
        assert_eq!(hide["action"]["selector"], ".ad-box");
        assert!(r.iter().all(|x| !x["trigger"]["url-filter"].as_str().unwrap().contains('|')), "WebKit has no alternation");
    }

    #[test]
    fn decisions_match_what_people_expect() {
        let l = [LIST.to_string()];
        let t = |url: &str, page: &str| verdict(&l, &[], url, page).0;
        assert!(t("https://ads.example/x.js", "https://news.example/"));
        assert!(t("https://img.ads.example/x.js", "https://news.example/"), "subdomains too");
        assert!(!t("https://notads.example/x.js", "https://news.example/"), "not a lookalike");
        assert!(!t("https://ads.example/allowed/x.js", "https://news.example/"), "exception wins");
        assert!(t("https://cdn.example/banners/1.png", "https://a.example/"));
        assert!(!t("https://cdn.example/fonts/1.woff", "https://a.example/"));
        assert!(t("https://pixel.example/p.gif", "https://news.example/"), "third-party: blocked");
        assert!(!t("https://pixel.example/p.gif", "https://pixel.example/"), "first-party: allowed");
        assert!(t("https://site.example/w.js", "https://www.news.example/"));
        assert!(!t("https://site.example/w.js", "https://other.example/"), "only on the named sites");
        assert!(t("https://x.example/adframe.html", "https://a.example/"));
        assert!(t("https://tracker.example/", "https://a.example/"));
        assert!(!verdict(&l, &["news.example".into()], "https://ads.example/x.js", "https://www.news.example/").0,
                "blocking off on this site");
    }

    #[test]
    fn bounded_and_deduplicated() {
        let big: String = (0..60_000).map(|i| format!("||d{i}.example^\n")).collect();
        let c = compile(&[big.clone(), big], &["keep.example".into()]);
        assert_eq!(c.rules.len(), MAX_RULES);
        assert_eq!(c.blocks, 60_000, "duplicates across lists counted once");
        assert_eq!(c.rules.last().unwrap()["trigger"]["if-domain"][0], "*keep.example", "the site switch survives the cut");
    }

    #[test]
    fn the_built_in_list_parses_cleanly() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../browser/nexa_browser/filters/nexa-basic.txt")).unwrap();
        let p = parse(&text);
        assert!(p.nets.len() >= 60, "{}", p.nets.len());
        assert_eq!(p.skipped, 0);
    }
}
