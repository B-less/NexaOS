//! nexa-imageinfo: a picture's details for the Image Viewer, and lossless
//! turning of JPEG photos. One request per run, as the user:
//!
//!   echo '{"op": "info", "path": "/home/u/Pictures/a.jpg"}' | nexa-imageinfo
//!
//! Ops:
//! - info (path): {format, width, height, animated, bytes, mtime_ns, exif:
//!   {make, model, lens, software, date, exposure, fnumber, iso, focal,
//!   orientation, gps}}. From the file's header only (JPEG, PNG/APNG, GIF,
//!   WebP, BMP, TIFF); the picture itself isn't decoded.
//! - turn (path, how: "left" | "right" | "flip_h" | "flip_v", expect:
//!   {mtime_ns, size}): turns a JPEG losslessly by changing the EXIF
//!   orientation value in place (two bytes; the picture's data is
//!   untouched). Code "no_lossless" when the file has no such value (a PNG,
//!   or a JPEG without EXIF orientation), "changed" when another program
//!   changed the file since it was read.
//!
//! Reply: one line, {"ok": true, "result": ...} or {"ok": false, "error":
//! "...", "code": "..."}. It confines itself with Landlock to that one file.

mod exif;

use nexa_sys::sandbox;
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Component, Path, PathBuf};

/// Headers and EXIF are near the start; GIF frames are counted within this.
const HEAD: u64 = 4 << 20;

#[derive(Debug, Default, PartialEq)]
struct Info {
    format: &'static str,
    width: u32,
    height: u32,
    animated: bool,
    exif: Option<exif::Exif>,
    /// Where the EXIF TIFF block starts in the file (JPEG and WebP).
    exif_at: Option<usize>,
}

fn be16(b: &[u8], o: usize) -> Option<u32> { b.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as u32) }
fn le16(b: &[u8], o: usize) -> Option<u32> { b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as u32) }
fn be32(b: &[u8], o: usize) -> Option<u32> { b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])) }
fn le32(b: &[u8], o: usize) -> Option<u32> { b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])) }
fn le24(b: &[u8], o: usize) -> Option<u32> { b.get(o..o + 3).map(|s| s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16) }

fn jpeg(b: &[u8]) -> Option<Info> {
    let mut i = Info { format: "JPEG", ..Default::default() };
    let mut p = 2;
    while p + 4 <= b.len() {
        if b[p] != 0xff {
            p += 1; // padding between segments
            continue;
        }
        let marker = b[p + 1];
        if marker == 0xff || marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            p += if marker == 0xff { 1 } else { 2 };
            continue;
        }
        if marker == 0xd9 || marker == 0xda {
            break; // end, or the picture data starts
        }
        let len = be16(b, p + 2)? as usize;
        if len < 2 {
            break;
        }
        let seg = p + 4;
        match marker {
            0xe1 if b.get(seg..seg + 6) == Some(b"Exif\0\0") && i.exif.is_none() => {
                let end = (p + 2 + len).min(b.len());
                if let Some(t) = b.get(seg + 6..end) {
                    i.exif = exif::parse(t);
                    i.exif_at = i.exif.as_ref().map(|_| seg + 6);
                }
            }
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                i.height = be16(b, seg + 1)?;
                i.width = be16(b, seg + 3)?;
            }
            _ => {}
        }
        p += 2 + len;
    }
    (i.width > 0 && i.height > 0).then_some(i)
}

fn png(b: &[u8]) -> Option<Info> {
    if b.get(12..16) != Some(b"IHDR") {
        return None;
    }
    let mut i = Info { format: "PNG", width: be32(b, 16)?, height: be32(b, 20)?, ..Default::default() };
    // An animated PNG has acTL before the picture data.
    let mut p = 8;
    while p + 8 <= b.len() {
        let (len, kind) = (be32(b, p)? as usize, b.get(p + 4..p + 8)?);
        match kind {
            b"acTL" => {
                i.animated = be32(b, p + 8).map_or(false, |frames| frames > 1);
                i.format = "PNG (animated)";
            }
            b"eXIf" => {
                i.exif = b.get(p + 8..p + 8 + len).and_then(exif::parse);
            }
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        p += 12 + len;
    }
    Some(i)
}

/// Counts a GIF's frames (up to 2: enough to know it moves).
fn gif_frames(b: &[u8]) -> usize {
    let mut p = 13;
    if b.get(10).map_or(false, |f| f & 0x80 != 0) {
        p += 3 << ((b[10] & 7) + 1);
    }
    let skip_blocks = |mut p: usize| -> Option<usize> {
        loop {
            let n = *b.get(p)? as usize;
            p += 1 + n;
            if n == 0 {
                return Some(p);
            }
        }
    };
    let mut frames = 0;
    while let Some(&k) = b.get(p) {
        match k {
            0x2c => {
                frames += 1;
                if frames >= 2 {
                    break;
                }
                let flags = *b.get(p + 9).unwrap_or(&0);
                p += 10;
                if flags & 0x80 != 0 {
                    p += 3 << ((flags & 7) + 1);
                }
                p += 1; // LZW code size
                match skip_blocks(p) { Some(n) => p = n, None => break }
            }
            0x21 => match skip_blocks(p + 2) { Some(n) => p = n, None => break },
            _ => break, // 0x3b trailer, or damage
        }
    }
    frames
}

fn webp(b: &[u8]) -> Option<Info> {
    let mut i = Info { format: "WebP", ..Default::default() };
    let mut p = 12;
    while p + 8 <= b.len() {
        let (kind, len) = (b.get(p..p + 4)?, le32(b, p + 4)? as usize);
        let d = p + 8;
        match kind {
            b"VP8 " if i.width == 0 => {
                i.width = le16(b, d + 6)? & 0x3fff;
                i.height = le16(b, d + 8)? & 0x3fff;
            }
            b"VP8L" if i.width == 0 => {
                let v = le32(b, d + 1)?;
                i.width = (v & 0x3fff) + 1;
                i.height = ((v >> 14) & 0x3fff) + 1;
            }
            b"VP8X" => {
                i.animated = b.get(d).map_or(false, |f| f & 0x02 != 0);
                i.width = le24(b, d + 4)? + 1;
                i.height = le24(b, d + 7)? + 1;
            }
            b"EXIF" => {
                let raw = b.get(d..(d + len).min(b.len()))?;
                // Some writers keep JPEG's "Exif\0\0" in front.
                let skip = if raw.starts_with(b"Exif\0\0") { 6 } else { 0 };
                i.exif = exif::parse(&raw[skip..]);
                i.exif_at = i.exif.as_ref().map(|_| d + skip);
            }
            _ => {}
        }
        p = d + len + (len & 1);
    }
    (i.width > 0).then_some(i)
}

fn identify(b: &[u8]) -> Option<Info> {
    if b.starts_with(&[0xff, 0xd8, 0xff]) {
        jpeg(b)
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(b)
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some(Info { format: "GIF", width: le16(b, 6)?, height: le16(b, 8)?, animated: gif_frames(b) > 1, ..Default::default() })
    } else if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        webp(b)
    } else if b.starts_with(b"BM") {
        let h = le32(b, 22)? as i32;
        Some(Info { format: "BMP", width: le32(b, 18)?, height: h.unsigned_abs(), ..Default::default() })
    } else if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") {
        let x = exif::parse(b)?;
        Some(Info { format: "TIFF", width: x.width.unwrap_or(0), height: x.height.unwrap_or(0), exif: Some(x), ..Default::default() })
    } else if b.starts_with(b"<svg") || b.starts_with(b"<?xml") {
        Some(Info { format: "SVG", ..Default::default() })
    } else {
        None
    }
}

// ---------- requests ----------

#[derive(Debug)]
struct Fail {
    error: String,
    code: &'static str,
}
impl From<String> for Fail {
    fn from(error: String) -> Self { Fail { error, code: "" } }
}
impl From<&str> for Fail {
    fn from(e: &str) -> Self { Fail { error: e.into(), code: "" } }
}

fn plain(e: &std::io::Error, p: &Path) -> String {
    let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
    match e.raw_os_error() {
        Some(libc::EACCES) | Some(libc::EPERM) => format!("You don't have permission to change {name}."),
        Some(libc::EROFS) => format!("{name} is on a drive that can't be written to."),
        Some(libc::ENOENT) => format!("{name} isn't there any more."),
        _ => format!("{name}: {e}"),
    }
}

fn abs(req: &Value) -> Result<PathBuf, Fail> {
    let s = req.get("path").and_then(Value::as_str).ok_or("missing 'path'")?;
    let p = Path::new(s);
    if !p.is_absolute() || s.contains('\0') || p.components().any(|c| c == Component::ParentDir) {
        return Err(format!("not a full path: {s}").into());
    }
    Ok(p.components().collect())
}

fn confine(p: &Path, rights: u64) -> Result<(), Fail> {
    sandbox::restrict(&sandbox::Rules { fs: vec![(p.to_string_lossy().into_owned(), rights)], connect_ports: vec![], any_network: false })
        .map(|_| ())
        .map_err(|e| format!("couldn't confine the image engine: {e}").into())
}

fn stamp(m: &fs::Metadata) -> String {
    (m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128).to_string()
}

fn read_head(p: &Path) -> Result<(Vec<u8>, fs::Metadata), Fail> {
    let f = fs::File::open(p).map_err(|e| plain(&e, p))?;
    let m = f.metadata().map_err(|e| plain(&e, p))?;
    if !m.is_file() {
        return Err("That isn't a picture file.".into());
    }
    let mut b = Vec::new();
    f.take(HEAD).read_to_end(&mut b).map_err(|e| plain(&e, p))?;
    Ok((b, m))
}

fn exif_json(x: &Option<exif::Exif>) -> Value {
    match x {
        None => Value::Null,
        Some(x) => json!({"make": x.make, "model": x.model, "lens": x.lens, "software": x.software, "date": x.date,
                          "exposure": x.exposure, "fnumber": x.fnumber, "iso": x.iso, "focal": x.focal,
                          "orientation": x.orientation, "gps": x.gps}),
    }
}

fn info(req: &Value) -> Result<Value, Fail> {
    let p = abs(req)?;
    confine(&p, sandbox::READ)?;
    let (b, m) = read_head(&p)?;
    let i = identify(&b).ok_or("This file isn't a picture NexaOS can read, or it's damaged.")?;
    Ok(json!({"format": i.format, "width": i.width, "height": i.height, "animated": i.animated,
              "bytes": m.len(), "mtime_ns": stamp(&m), "exif": exif_json(&i.exif)}))
}

fn turn(req: &Value) -> Result<Value, Fail> {
    let p = abs(req)?;
    let how = req.get("how").and_then(Value::as_str).ok_or("missing 'how'")?;
    confine(&p, sandbox::READ_WRITE)?;
    let (b, m) = read_head(&p)?;
    if let Some(want) = req.get("expect").filter(|v| !v.is_null()) {
        if want.get("mtime_ns").and_then(Value::as_str) != Some(stamp(&m).as_str()) || want.get("size").and_then(Value::as_u64) != Some(m.len()) {
            return Err(Fail { error: "Another program changed this picture since it was opened.".into(), code: "changed" });
        }
    }
    let i = identify(&b).ok_or("This file isn't a picture NexaOS can read, or it's damaged.")?;
    let no = || Fail { error: "This picture can't be turned without saving it again.".into(), code: "no_lossless" };
    if i.format != "JPEG" {
        return Err(no());
    }
    let (Some(x), Some(base)) = (&i.exif, i.exif_at) else { return Err(no()) };
    let (Some(now), Some(at)) = (x.orientation, x.orientation_at) else { return Err(no()) };
    let next = exif::turn(now, how).ok_or("unknown turn")?;
    let le = b.get(base..base + 2) == Some(b"II");
    let bytes = if le { next.to_le_bytes() } else { next.to_be_bytes() };
    let f = fs::OpenOptions::new().write(true).open(&p).map_err(|e| plain(&e, &p))?;
    f.write_all_at(&bytes, (base + at) as u64).and_then(|_| f.sync_all()).map_err(|e| plain(&e, &p))?;
    let m = fs::metadata(&p).map_err(|e| plain(&e, &p))?;
    Ok(json!({"orientation": next, "mtime_ns": stamp(&m), "size": m.len()}))
}

fn handle(input: &[u8]) -> Result<Value, Fail> {
    let req: Value = serde_json::from_slice(input).map_err(|e| format!("bad request: {e}"))?;
    match req.get("op").and_then(Value::as_str).ok_or("missing 'op'")? {
        "info" => info(&req),
        "turn" => turn(&req),
        other => Err(format!("unknown op {other}").into()),
    }
}

fn main() {
    let mut input = Vec::new();
    let _ = std::io::stdin().take(1 << 20).read_to_end(&mut input);
    let reply = match handle(&input) {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(f) => json!({"ok": false, "error": f.error, "code": f.code}),
    };
    let _ = writeln!(std::io::stdout().lock(), "{reply}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg_with(orientation: Option<u16>) -> Vec<u8> {
        let mut b = vec![0xff, 0xd8];
        if let Some(o) = orientation {
            let t = exif::tests::tiff(o);
            b.extend([0xff, 0xe1]);
            b.extend(((t.len() + 8) as u16).to_be_bytes());
            b.extend(b"Exif\0\0");
            b.extend(t);
        }
        b.extend([0xff, 0xc0, 0, 17, 8]);
        b.extend(480u16.to_be_bytes());
        b.extend(640u16.to_be_bytes());
        b.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        b.extend([0xff, 0xda, 0, 2, 1, 2, 3, 0xff, 0xd9]);
        b
    }

    #[test]
    fn formats() {
        let i = identify(&jpeg_with(Some(6))).unwrap();
        assert_eq!((i.format, i.width, i.height), ("JPEG", 640, 480));
        assert_eq!(i.exif.as_ref().unwrap().model.as_deref(), Some("Hex 1"));
        let mut png_b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png_b.extend(300u32.to_be_bytes());
        png_b.extend(200u32.to_be_bytes());
        png_b.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        png_b.extend(b"\0\0\0\x08acTL\0\0\0\x03\0\0\0\0\0\0\0\0");
        let i = identify(&png_b).unwrap();
        assert_eq!((i.format, i.width, i.height, i.animated), ("PNG (animated)", 300, 200, true));
        // GIF with two frames.
        let mut g = b"GIF89a".to_vec();
        g.extend([10, 0, 5, 0, 0, 0, 0]);
        for _ in 0..2 {
            g.extend([0x21, 0xf9, 4, 0, 10, 0, 0, 0]);
            g.extend([0x2c, 0, 0, 0, 0, 10, 0, 5, 0, 0, 2, 2, 0x4c, 0x01, 0]);
        }
        g.push(0x3b);
        let i = identify(&g).unwrap();
        assert_eq!((i.width, i.height, i.animated), (10, 5, true));
        let mut w = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\x02\0\0\0".to_vec();
        w.extend([0x1f, 0x03, 0, 0xdf, 0x01, 0]);
        let i = identify(&w).unwrap();
        assert_eq!((i.width, i.height, i.animated), (800, 480, true));
        let mut bmp = b"BM".to_vec();
        bmp.resize(30, 0);
        bmp[18..22].copy_from_slice(&64u32.to_le_bytes());
        bmp[22..26].copy_from_slice(&(-32i32).to_le_bytes());
        assert_eq!(identify(&bmp).map(|i| (i.width, i.height)), Some((64, 32)));
        assert_eq!(identify(b"hello"), None);
        // Cut short anywhere: no panic.
        for src in [jpeg_with(Some(3)), png_b.clone(), g.clone(), w.clone()] {
            for n in 0..src.len() {
                let _ = identify(&src[..n]);
            }
        }
    }

    #[test]
    fn turns_only_where_lossless() {
        let d = std::env::temp_dir().join(format!("nexa-imageinfo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let p = d.join("photo.jpg");
        let orig = jpeg_with(Some(1));
        fs::write(&p, &orig).unwrap();
        // Same steps as turn(), without Landlock (it would confine the test).
        let (b, _) = read_head(&p).unwrap();
        let i = identify(&b).unwrap();
        let (base, at) = (i.exif_at.unwrap(), i.exif.unwrap().orientation_at.unwrap());
        let f = fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.write_all_at(&exif::turn(1, "right").unwrap().to_le_bytes(), (base + at) as u64).unwrap();
        let after = fs::read(&p).unwrap();
        assert_eq!(identify(&after).unwrap().exif.unwrap().orientation, Some(6));
        // Only those two bytes changed.
        assert_eq!(after.iter().zip(&orig).filter(|(a, b)| a != b).count(), 1);
        fs::remove_dir_all(&d).unwrap();
    }
}
