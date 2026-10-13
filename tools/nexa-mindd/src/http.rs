//! Just enough HTTP/1.1 for a local socket: one request per connection
//! (Connection: close), Content-Length bodies, and a streaming proxy to the
//! model server so answers flow through word by word (server-sent events).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

pub const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_HEADER_LINES: usize = 64;

#[derive(Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

/// Ok(None) if the client closed without sending anything.
pub fn read_request(r: &mut impl BufRead) -> io::Result<Option<Request>> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (method, path) = match (parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(p), Some(v)) if v.starts_with("HTTP/1.") => (m.to_string(), p.to_string()),
        _ => return Err(bad("bad request line")),
    };
    let mut length = 0usize;
    for _ in 0..MAX_HEADER_LINES {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Err(bad("headers cut off"));
        }
        let l = line.trim_end();
        if l.is_empty() {
            let mut body = vec![0; length];
            r.read_exact(&mut body)?;
            return Ok(Some(Request { method, path, body }));
        }
        if let Some((k, v)) = l.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "content-length" {
                length = v.parse().map_err(|_| bad("bad content-length"))?;
                if length > MAX_BODY {
                    return Err(bad("body too large"));
                }
            } else if k == "transfer-encoding" {
                return Err(bad("chunked bodies are not supported"));
            }
        }
    }
    Err(bad("too many headers"))
}

pub fn respond(w: &mut impl Write, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK", 202 => "Accepted", 400 => "Bad Request", 403 => "Forbidden", 404 => "Not Found",
        409 => "Conflict", 500 => "Internal Server Error", 503 => "Service Unavailable", _ => "Status",
    };
    write!(w, "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
    w.write_all(body)?;
    w.flush()
}

pub fn respond_json(w: &mut impl Write, status: u16, v: &serde_json::Value) -> io::Result<()> {
    respond(w, status, "application/json", v.to_string().as_bytes())
}

/// Send a request to a Unix-socket HTTP server and return its stream,
/// positioned at the start of the response.
pub fn send(socket: &Path, method: &str, path: &str, body: &[u8]) -> io::Result<UnixStream> {
    let mut s = UnixStream::connect(socket)?;
    write!(s, "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
    s.write_all(body)?;
    s.flush()?;
    Ok(s)
}

/// Status code and body of a complete (non-streaming) response.
pub fn read_response(s: UnixStream) -> io::Result<(u16, Vec<u8>)> {
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let status = line.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let mut length = None;
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 || line.trim_end().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                length = v.trim().parse::<usize>().ok();
            }
        }
    }
    let mut body = Vec::new();
    match length {
        Some(n) => { body.resize(n, 0); r.read_exact(&mut body)?; }
        None => { r.read_to_end(&mut body)?; }
    }
    Ok((status, body))
}

/// Forward a request upstream and copy the whole response (headers and
/// all, streamed) back to the client.
pub fn proxy(upstream: &Path, req: &Request, client: &mut impl Write) -> io::Result<u64> {
    let mut up = send(upstream, &req.method, &req.path, &req.body)?;
    let n = io::copy(&mut up, client)?;
    client.flush()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_a_request() {
        let raw = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\n{}";
        let r = read_request(&mut Cursor::new(&raw[..])).unwrap().unwrap();
        assert_eq!(r, Request { method: "POST".into(), path: "/v1/chat/completions".into(), body: b"{}".to_vec() });
        assert!(read_request(&mut Cursor::new(&b""[..])).unwrap().is_none());
    }

    #[test]
    fn rejects_hostile_requests() {
        let too_big = format!("POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1);
        for raw in [too_big.as_str(), "GARBAGE\r\n\r\n", "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
                    "GET / HTTP/1.1\r\nContent-Length: -1\r\n\r\n"] {
            assert!(read_request(&mut Cursor::new(raw.as_bytes())).is_err(), "{raw:?}");
        }
        let many = format!("GET / HTTP/1.1\r\n{}\r\n", "X: y\r\n".repeat(100));
        assert!(read_request(&mut Cursor::new(many.as_bytes())).is_err());
    }

    #[test]
    fn writes_a_response() {
        let mut out = Vec::new();
        respond_json(&mut out, 200, &serde_json::json!({"ok": true})).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n") && text.ends_with("{\"ok\":true}") && text.contains("Content-Length: 11"));
    }
}
