//! `nexac lsp` (0.6): a small language server over stdin/stdout, for
//! editors (VS Code, NexaOS's own editors).
//!
//! It does four things, each with the same code as the command line:
//! - mistakes as you type (`textDocument/publishDiagnostics` after open,
//!   change and save), from the same checks as `nexac check`;
//! - formatting (`textDocument/formatting`), from `nexac fmt`;
//! - an explanation when the pointer rests on a mistake
//!   (`textDocument/hover`), from `nexac explain`;
//! - nothing else: no completion or go-to-definition yet.
//!
//! Hostile input is expected (it's text from any file and any client):
//! messages over 64 MiB, headers over 8 KiB, bad JSON, bad UTF-8, wrong
//! types and unknown methods are answered or skipped, never a crash. A
//! panic inside the compiler (none known: it's fuzzed) is caught and shown
//! as one diagnostic instead of ending the server.
//!
//! Positions: LSP counts lines from 0 and characters in UTF-16 units;
//! nexac counts both from 1, characters as Unicode letters. `utf16_col`
//! converts.

use crate::diag::Report;
use crate::json::{self, Json};
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;

const MAX_BODY: usize = 64 << 20;
const MAX_HEADER: usize = 8 << 10;

struct Doc {
    text: String,
    path: PathBuf,
    /// The last mistakes found, for hover.
    reports: Vec<Report>,
}

pub fn serve() -> i32 {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut docs: HashMap<String, Doc> = HashMap::new();
    let mut initialized = false;
    let mut shutdown = false;
    loop {
        let body = match read_message(&mut input) {
            Ok(Some(b)) => b,
            Ok(None) => return if shutdown { 0 } else { 1 }, // end of input
            Err(Skip(msg)) => {
                eprintln!("nexac lsp: {}", msg);
                send(&mut out, &error_reply(Json::Null, -32700, &msg));
                continue;
            }
        };
        let msg = match std::str::from_utf8(&body).map_err(|_| "the message isn't UTF-8".to_string()).and_then(json::parse) {
            Ok(m) => m,
            Err(e) => {
                send(&mut out, &error_reply(Json::Null, -32700, &format!("can't read the message: {}", e)));
                continue;
            }
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Json::as_str).unwrap_or("").to_string();
        let params = msg.get("params").cloned().unwrap_or(Json::Null);
        if method.is_empty() {
            // a reply to something we never ask, or nonsense
            if let Some(id) = id {
                if msg.get("result").is_none() && msg.get("error").is_none() {
                    send(&mut out, &error_reply(id, -32600, "a request needs a method"));
                }
            }
            continue;
        }
        if method == "exit" {
            return if shutdown { 0 } else { 1 };
        }
        if !initialized && method != "initialize" {
            if let Some(id) = id {
                send(&mut out, &error_reply(id, -32002, "the server isn't initialized yet"));
            }
            continue;
        }
        if shutdown {
            if let Some(id) = id {
                send(&mut out, &error_reply(id, -32600, "the server is shutting down"));
            }
            continue;
        }
        match method.as_str() {
            "initialize" => {
                initialized = true;
                if let Some(id) = id {
                    send(&mut out, &reply(id, capabilities()));
                }
            }
            "shutdown" => {
                shutdown = true;
                if let Some(id) = id {
                    send(&mut out, &reply(id, Json::Null));
                }
            }
            "textDocument/didOpen" => {
                let td = params.get("textDocument");
                let uri = td.and_then(|t| t.get("uri")).and_then(Json::as_str);
                let text = td.and_then(|t| t.get("text")).and_then(Json::as_str);
                if let (Some(uri), Some(text)) = (uri, text) {
                    let doc = Doc { text: text.to_string(), path: uri_to_path(uri), reports: Vec::new() };
                    docs.insert(uri.to_string(), doc);
                    check_and_publish(&mut out, uri, &mut docs);
                }
            }
            "textDocument/didChange" => {
                let uri = params.path(&["textDocument", "uri"]).and_then(Json::as_str);
                // full sync: the last change holds the whole text
                let text = params
                    .get("contentChanges")
                    .and_then(Json::as_arr)
                    .and_then(|c| c.last())
                    .filter(|c| c.get("range").is_none())
                    .and_then(|c| c.get("text"))
                    .and_then(Json::as_str);
                if let (Some(uri), Some(text)) = (uri, text) {
                    if let Some(doc) = docs.get_mut(uri) {
                        doc.text = text.to_string();
                        check_and_publish(&mut out, uri, &mut docs);
                    }
                }
            }
            "textDocument/didSave" => {
                if let Some(uri) = params.path(&["textDocument", "uri"]).and_then(Json::as_str) {
                    if docs.contains_key(uri) {
                        check_and_publish(&mut out, uri, &mut docs);
                    }
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = params.path(&["textDocument", "uri"]).and_then(Json::as_str) {
                    docs.remove(uri);
                    send(&mut out, &publish(uri, Vec::new()));
                }
            }
            "textDocument/formatting" => {
                let Some(id) = id else { continue };
                let uri = params.path(&["textDocument", "uri"]).and_then(Json::as_str).unwrap_or("");
                match docs.get(uri) {
                    Some(doc) => send(&mut out, &reply(id, formatting(&doc.text))),
                    None => send(&mut out, &error_reply(id, -32602, "that file isn't open")),
                }
            }
            "textDocument/hover" => {
                let Some(id) = id else { continue };
                let uri = params.path(&["textDocument", "uri"]).and_then(Json::as_str).unwrap_or("");
                let line = params.path(&["position", "line"]).and_then(Json::as_u32);
                let ch = params.path(&["position", "character"]).and_then(Json::as_u32);
                let result = match (docs.get(uri), line, ch) {
                    (Some(doc), Some(l), Some(c)) => hover(doc, l, c),
                    _ => Json::Null,
                };
                send(&mut out, &reply(id, result));
            }
            _ => {
                // unknown requests get an answer; unknown notifications
                // (`initialized`, `$/cancelRequest`, ...) are skipped
                if let Some(id) = id {
                    send(&mut out, &error_reply(id, -32601, &format!("nexac doesn't do `{}`", method)));
                }
            }
        }
    }
}

struct Skip(String);

/// One message's body; `None` at the end of input.
fn read_message(input: &mut impl BufRead) -> Result<Option<Vec<u8>>, Skip> {
    let mut length: Option<Result<usize, String>> = None;
    let mut header_bytes = 0;
    let mut any = false;
    loop {
        let mut line = Vec::new();
        // read one header line, at most MAX_HEADER bytes
        let n = input.by_ref().take((MAX_HEADER + 1) as u64).read_until(b'\n', &mut line).map_err(|e| Skip(e.to_string()))?;
        if n == 0 {
            if any {
                return Err(Skip("input ended inside a message's headers".into()));
            }
            return Ok(None);
        }
        any = true;
        header_bytes += n;
        if header_bytes > MAX_HEADER {
            // skip the rest of this too-long header block
            loop {
                let mut rest = Vec::new();
                let n = input.by_ref().take((MAX_HEADER + 1) as u64).read_until(b'\n', &mut rest).map_err(|e| Skip(e.to_string()))?;
                if n == 0 || rest == b"\r\n" || rest == b"\n" {
                    break;
                }
            }
            return Err(Skip("a message's headers are too long".into()));
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            break;
        }
        if let Some((name, value)) = text.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                // read on to the end of the headers before saying it's bad
                length = Some(value.trim().parse::<usize>().map_err(|_| format!("bad Content-Length: {}", value.trim())));
            }
        }
    }
    let len = match length {
        Some(Ok(n)) => n,
        Some(Err(msg)) => return Err(Skip(msg)),
        None => return Err(Skip("a message has no Content-Length".into())),
    };
    if len > MAX_BODY {
        // read it and drop it, so the next message can still be read
        let mut sink = std::io::sink();
        let copied = std::io::copy(&mut input.by_ref().take(len as u64), &mut sink).map_err(|e| Skip(e.to_string()))?;
        if copied < len as u64 {
            return Ok(None);
        }
        return Err(Skip(format!("a message of {} bytes is too big (the limit is {} bytes)", len, MAX_BODY)));
    }
    let mut body = vec![0u8; len];
    match input.read_exact(&mut body) {
        Ok(()) => Ok(Some(body)),
        Err(_) => Ok(None), // the input ended inside the message
    }
}

fn send(out: &mut impl Write, msg: &Json) {
    let body = msg.write();
    // if the editor has gone, there's nobody to tell; the next read ends
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = out.flush();
}

fn reply(id: Json, result: Json) -> Json {
    Json::obj(vec![("jsonrpc", Json::str("2.0")), ("id", id), ("result", result)])
}

fn error_reply(id: Json, code: i32, message: &str) -> Json {
    Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("id", id),
        ("error", Json::obj(vec![("code", Json::num(code)), ("message", Json::str(message))])),
    ])
}

fn capabilities() -> Json {
    Json::obj(vec![
        (
            "capabilities",
            Json::obj(vec![
                (
                    "textDocumentSync",
                    Json::obj(vec![
                        ("openClose", Json::Bool(true)),
                        ("change", Json::num(1)), // full text on every change
                        ("save", Json::obj(vec![("includeText", Json::Bool(false))])),
                    ]),
                ),
                ("documentFormattingProvider", Json::Bool(true)),
                ("hoverProvider", Json::Bool(true)),
            ]),
        ),
        ("serverInfo", Json::obj(vec![("name", Json::str("nexac")), ("version", Json::str(crate::VERSION))])),
    ])
}

/// The path a `file:` URI names; other URIs (an unsaved buffer) get a
/// path in a folder that doesn't exist, so `use` of a local module there
/// says it can't be found rather than reading some other folder.
pub fn uri_to_path(uri: &str) -> PathBuf {
    const NOWHERE: &str = "/nonexistent-nexalang-folder/untitled.nx";
    let Some(rest) = uri.strip_prefix("file://") else {
        return PathBuf::from(NOWHERE);
    };
    // "file://host/path": the host is empty or "localhost" for this computer
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = if host.eq_ignore_ascii_case("localhost") { "" } else { host };
    let bytes = percent_decode(path);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        if !host.is_empty() {
            // another computer's file: not some local file with its path
            return PathBuf::from(NOWHERE);
        }
        PathBuf::from(std::ffi::OsString::from_vec(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(windows_path(host, &String::from_utf8_lossy(&bytes)))
    }
}

/// A `file:` URI's (decoded) path as Windows writes it (0.9):
/// `/c:/Users/x.nx` is `c:\Users\x.nx`, and a host is a shared folder,
/// `file://server/share/x.nx` is `\\server\share\x.nx`.
#[cfg_attr(unix, allow(dead_code))]
fn windows_path(host: &str, path: &str) -> String {
    let b = path.as_bytes();
    let path = if host.is_empty() && b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        &path[1..]
    } else {
        path
    };
    let path = path.replace('/', "\\");
    if host.is_empty() {
        path
    } else {
        format!("\\\\{}{}", host, path)
    }
}

fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Check the document and send its mistakes.
fn check_and_publish(out: &mut impl Write, uri: &str, docs: &mut HashMap<String, Doc>) {
    let Some(doc) = docs.get_mut(uri) else { return };
    let path = doc.path.clone();
    let text = doc.text.clone();
    let result = std::panic::catch_unwind(move || crate::compile_source(&path, Some(text), true));
    doc.reports = match result {
        Ok(Ok(_)) => Vec::new(),
        Ok(Err(reports)) => reports,
        Err(_) => vec![Report::general(
            &crate::display_name(&doc.path),
            "nexac crashed while checking this file. This is a bug in nexac; please report it with your .nx file",
        )],
    };
    let main = crate::display_name(&doc.path);
    let diags: Vec<Json> = doc.reports.iter().map(|r| diagnostic(r, &main, &doc.text)).collect();
    send(out, &publish(uri, diags));
}

fn publish(uri: &str, diags: Vec<Json>) -> Json {
    Json::obj(vec![
        ("jsonrpc", Json::str("2.0")),
        ("method", Json::str("textDocument/publishDiagnostics")),
        ("params", Json::obj(vec![("uri", Json::str(uri)), ("diagnostics", Json::Arr(diags))])),
    ])
}

/// LSP position of nexac's 1-based (line, character) in `text`.
fn lsp_pos(text: &str, line: u32, col: u32) -> (u32, u32) {
    if line == 0 {
        return (0, 0);
    }
    let l = line - 1;
    let line_text = text.split('\n').nth(l as usize).unwrap_or("");
    (l, utf16_col(line_text, col.saturating_sub(1)))
}

/// UTF-16 units before the `chars`-th character of `line`.
pub fn utf16_col(line: &str, chars: u32) -> u32 {
    line.chars().take(chars as usize).map(|c| c.len_utf16() as u32).sum()
}

fn range(start: (u32, u32), end: (u32, u32)) -> Json {
    let pos = |p: (u32, u32)| Json::obj(vec![("line", Json::num(p.0)), ("character", Json::num(p.1))]);
    Json::obj(vec![("start", pos(start)), ("end", pos(end))])
}

fn diagnostic(r: &Report, main: &str, text: &str) -> Json {
    let topic = crate::explain::classify(&r.msg);
    let mut message = if r.file == main || r.line == 0 {
        r.msg.clone()
    } else {
        // a mistake in a module this file uses: shown at the top
        format!("in {}:{}:{}: {}", r.file, r.line, r.col, r.msg)
    };
    if let Some(h) = &r.hint {
        message += &format!("\nhint: {}", h);
    }
    let (start, end) = if r.file == main && r.line > 0 {
        (lsp_pos(text, r.line, r.col), lsp_pos(text, r.line, r.col + r.length.max(1)))
    } else {
        ((0, 0), (0, 0))
    };
    Json::obj(vec![
        ("range", range(start, end)),
        ("severity", Json::num(1)),
        ("source", Json::str("nexac")),
        ("code", Json::str(topic.name)),
        ("message", Json::Str(message)),
    ])
}

/// One edit replacing the whole text, or none when it's already formatted
/// or can't be read (its mistake is already shown as a diagnostic).
fn formatting(text: &str) -> Json {
    let owned = text.to_string();
    match std::panic::catch_unwind(move || crate::fmt::format(&owned)) {
        Ok(Ok(new)) if new != text => {
            let lines: Vec<&str> = text.split('\n').collect();
            let last = lines.len() as u32 - 1;
            let end = (last, lines.last().map(|l| l.encode_utf16().count() as u32).unwrap_or(0));
            Json::Arr(vec![Json::obj(vec![("range", range((0, 0), end)), ("newText", Json::Str(new))])])
        }
        _ => Json::Arr(Vec::new()),
    }
}

/// The explanation of the mistake under the pointer, if any.
fn hover(doc: &Doc, line: u32, ch: u32) -> Json {
    let main = crate::display_name(&doc.path);
    let here = doc.reports.iter().filter(|r| r.file == main && r.line == line + 1).find(|r| {
        let (_, s) = lsp_pos(&doc.text, r.line, r.col);
        let (_, e) = lsp_pos(&doc.text, r.line, r.col + r.length.max(1));
        ch >= s && ch <= e
    });
    let Some(r) = here else { return Json::Null };
    let t = crate::explain::classify(&r.msg);
    let mut md = format!("**{}**\n\n", escape_md(&r.msg));
    if let Some(h) = &r.hint {
        md += &format!("Hint: {}\n\n", escape_md(h));
    }
    md += &format!("**{}** — {}\n\n```text\n{}\n```\n\nMore: `nexac explain {}`", t.title, t.summary, t.body, t.name);
    Json::obj(vec![("contents", Json::obj(vec![("kind", Json::str("markdown")), ("value", Json::Str(md))]))])
}

/// Messages quote code with backticks, which is fine in Markdown; only
/// characters that would start HTML or links are escaped.
fn escape_md(s: &str) -> String {
    s.replace('\\', "\\\\").replace('<', "\\<").replace('[', "\\[").replace('*', "\\*").replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
    }

    #[test]
    fn reads_messages_and_survives_bad_framing() {
        let mut data = frame("{\"a\":1}");
        data.extend(b"Content-Length: x\r\n\r\n");
        data.extend(b"No-Length: 1\r\n\r\n");
        data.extend(frame("{}"));
        let mut r = std::io::Cursor::new(data);
        assert_eq!(read_message(&mut r).ok().flatten().unwrap(), b"{\"a\":1}");
        assert!(read_message(&mut r).is_err());
        assert!(read_message(&mut r).is_err());
        assert_eq!(read_message(&mut r).ok().flatten().unwrap(), b"{}");
        assert!(matches!(read_message(&mut r), Ok(None)));
        // a body cut short is the end
        let mut r = std::io::Cursor::new(b"Content-Length: 10\r\n\r\n{}".to_vec());
        assert!(matches!(read_message(&mut r), Ok(None)));
    }

    #[test]
    fn positions_are_utf16() {
        assert_eq!(utf16_col("a😀b", 2), 3);
        assert_eq!(lsp_pos("x\n é😀z\n", 2, 4), (1, 4));
        assert_eq!(lsp_pos("", 5, 5), (4, 0));
    }

    #[test]
    fn uris() {
        assert_eq!(uri_to_path("file:///home/a%20b/x.nx"), PathBuf::from("/home/a b/x.nx"));
        assert_eq!(uri_to_path("file://localhost/tmp/x.nx"), PathBuf::from("/tmp/x.nx"));
        assert_eq!(uri_to_path("file:///t/%zz%4"), PathBuf::from("/t/%zz%4"));
        assert_eq!(uri_to_path("file://LOCALHOST/tmp/x.nx"), PathBuf::from("/tmp/x.nx"));
        #[cfg(unix)]
        assert!(uri_to_path("file://server/share/x.nx").starts_with("/nonexistent-nexalang-folder"));
        assert!(uri_to_path("untitled:Untitled-1").starts_with("/nonexistent-nexalang-folder"));
    }

    #[test]
    fn windows_uris() {
        // as VS Code sends them on Windows (`file:///c%3A/...`, decoded first)
        assert_eq!(windows_path("", "/c:/Users/Bless Boateng/app.nx"), "c:\\Users\\Bless Boateng\\app.nx");
        assert_eq!(windows_path("", "/C:/x.nx"), "C:\\x.nx");
        assert_eq!(windows_path("server", "/share/apps/x.nx"), "\\\\server\\share\\apps\\x.nx");
        assert_eq!(windows_path("", "/tmp/x.nx"), "\\tmp\\x.nx");
        // not a drive: kept as a path from the root
        assert_eq!(windows_path("", "/1:/x"), "\\1:\\x");
    }
}
