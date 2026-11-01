//! Tags, length and cover art of a song file: MP3 (ID3v2.2/2.3/2.4,
//! ID3v1, Xing/VBRI/CBR length), FLAC (STREAMINFO, Vorbis comments,
//! PICTURE), Ogg Vorbis and Opus (comment packets, last granule), M4A/MP4
//! (mvhd, ilst) and WAV (fmt/data, LIST INFO). Everything read is
//! bounds-checked: a damaged file gives what could be read, never a panic.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_TAG: u64 = 32 << 20;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    /// Seconds.
    pub duration: Option<f64>,
    /// (MIME type, bytes) of the front cover, or the first picture.
    pub picture: Option<(String, Vec<u8>)>,
    picture_is_front: bool,
}

impl Tags {
    fn set(slot: &mut Option<String>, v: String) {
        let v = v.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
        if slot.is_none() && !v.is_empty() {
            *slot = Some(v);
        }
    }

    fn number(s: &str) -> Option<u32> {
        s.trim().split('/').next()?.trim().parse().ok().filter(|&n| n > 0)
    }

    fn year(s: &str) -> Option<u32> {
        s.trim().get(..4)?.parse().ok().filter(|y| (1000..3000).contains(y))
    }

    /// A Vorbis-comment style KEY=value (FLAC, Ogg).
    fn comment(&mut self, key: &str, v: &str) {
        match key.to_ascii_uppercase().as_str() {
            "TITLE" => Self::set(&mut self.title, v.into()),
            "ARTIST" => Self::set(&mut self.artist, v.into()),
            "ALBUM" => Self::set(&mut self.album, v.into()),
            "ALBUMARTIST" | "ALBUM ARTIST" | "ALBUM_ARTIST" => Self::set(&mut self.album_artist, v.into()),
            "GENRE" => Self::set(&mut self.genre, v.into()),
            "DATE" | "YEAR" | "ORIGINALDATE" => self.year = self.year.or_else(|| Self::year(v)),
            "TRACKNUMBER" | "TRACK" => self.track = self.track.or_else(|| Self::number(v)),
            "DISCNUMBER" | "DISC" => self.disc = self.disc.or_else(|| Self::number(v)),
            "METADATA_BLOCK_PICTURE" => {
                if let Some(b) = base64(v) {
                    flac_picture(self, &b);
                }
            }
            _ => {}
        }
    }

    fn picture(&mut self, mime: &str, kind: u8, data: &[u8]) {
        if data.len() < 16 || (self.picture.is_some() && (self.picture_is_front || kind != 3)) {
            return;
        }
        let mime = if data.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" }
            else if data.starts_with(&[0xff, 0xd8]) { "image/jpeg" }
            else { mime };
        if mime != "image/png" && mime != "image/jpeg" {
            return;
        }
        self.picture = Some((mime.to_string(), data.to_vec()));
        self.picture_is_front = kind == 3;
    }
}

fn be32(b: &[u8], o: usize) -> Option<u32> { Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?)) }
fn le32(b: &[u8], o: usize) -> Option<u32> { Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?)) }
fn be16(b: &[u8], o: usize) -> Option<u16> { Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?)) }
fn le16(b: &[u8], o: usize) -> Option<u16> { Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?)) }
fn be64(b: &[u8], o: usize) -> Option<u64> { Some(u64::from_be_bytes(b.get(o..o + 8)?.try_into().ok()?)) }
fn le64(b: &[u8], o: usize) -> Option<u64> { Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?)) }

fn read_at(f: &mut File, at: u64, len: u64) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(at)).ok()?;
    let mut b = Vec::new();
    f.take(len).read_to_end(&mut b).ok()?;
    Some(b)
}

pub fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// ---------- ID3 ----------

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn utf16(b: &[u8], le: bool) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
    String::from_utf16_lossy(&units)
}

/// An ID3 text in encoding `enc`; the first of several NUL-separated values.
fn id3_text(enc: u8, b: &[u8]) -> String {
    let s = match enc {
        0 => latin1(b),
        1 => match b.get(..2) {
            Some([0xff, 0xfe]) => utf16(&b[2..], true),
            Some([0xfe, 0xff]) => utf16(&b[2..], false),
            _ => utf16(b, true),
        },
        2 => utf16(b, false),
        _ => String::from_utf8_lossy(b).into_owned(),
    };
    s.split('\0').next().unwrap_or("").to_string()
}

/// Where an encoded, NUL-terminated string in `b` ends (after the NUL).
fn id3_skip_string(enc: u8, b: &[u8], from: usize) -> Option<usize> {
    if enc == 1 || enc == 2 {
        let mut i = from;
        while i + 1 < b.len() {
            if b[i] == 0 && b[i + 1] == 0 {
                return Some(i + 2);
            }
            i += 2;
        }
        None
    } else {
        b.get(from..)?.iter().position(|&c| c == 0).map(|p| from + p + 1)
    }
}

fn unsync(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        out.push(b[i]);
        if b[i] == 0xff && b.get(i + 1) == Some(&0) {
            i += 1;
        }
        i += 1;
    }
    out
}

fn syncsafe(b: &[u8], o: usize) -> Option<u32> {
    let s = b.get(o..o + 4)?;
    Some((s[0] as u32 & 0x7f) << 21 | (s[1] as u32 & 0x7f) << 14 | (s[2] as u32 & 0x7f) << 7 | (s[3] as u32 & 0x7f))
}

/// ID3v2 at the start of the file; returns where the audio starts.
fn id3v2(f: &mut File, t: &mut Tags) -> Option<u64> {
    let h = read_at(f, 0, 10)?;
    if h.get(..3) != Some(b"ID3") {
        return None;
    }
    let (major, flags) = (h[3], h[5]);
    let size = syncsafe(&h, 6)? as u64;
    let end = 10 + size + if flags & 0x10 != 0 { 10 } else { 0 };
    if !(2..=4).contains(&major) {
        return Some(end);
    }
    let mut tag = read_at(f, 10, size.min(MAX_TAG))?;
    if flags & 0x80 != 0 && major <= 3 {
        tag = unsync(&tag);
    }
    let mut p = 0usize;
    if flags & 0x40 != 0 && major >= 3 {
        p = if major == 3 { 4 + be32(&tag, 0)? as usize } else { syncsafe(&tag, 0)? as usize };
    }
    let (idlen, hlen) = if major == 2 { (3, 6) } else { (4, 10) };
    while p + hlen <= tag.len() {
        let id = &tag[p..p + idlen];
        if id[0] == 0 {
            break;
        }
        let len = match major {
            2 => (tag[p + 3] as usize) << 16 | (tag[p + 4] as usize) << 8 | tag[p + 5] as usize,
            3 => be32(&tag, p + 4)? as usize,
            _ => syncsafe(&tag, p + 4)? as usize,
        };
        let start = p + hlen;
        let Some(raw) = tag.get(start..start + len) else { break };
        p = start + len;
        let mut data = raw.to_vec();
        if major == 4 {
            let fl = tag[start - 1];
            if fl & 0x02 != 0 {
                data = unsync(&data);
            }
            if fl & 0x01 != 0 && data.len() >= 4 {
                data.drain(..4);
            }
        }
        let Some((&enc, body)) = data.split_first() else { continue };
        let text = || id3_text(enc, body);
        match id {
            b"TIT2" | b"TT2" => Tags::set(&mut t.title, text()),
            b"TPE1" | b"TP1" => Tags::set(&mut t.artist, text()),
            b"TALB" | b"TAL" => Tags::set(&mut t.album, text()),
            b"TPE2" | b"TP2" => Tags::set(&mut t.album_artist, text()),
            b"TCON" | b"TCO" => {
                // "(17)Rock", "(17)" or "Rock": the words if there are any.
                let g = text();
                let words = g.rsplit(')').next().unwrap_or("").trim().to_string();
                if !words.is_empty() {
                    Tags::set(&mut t.genre, words);
                }
            }
            b"TRCK" | b"TRK" => t.track = t.track.or_else(|| Tags::number(&text())),
            b"TPOS" | b"TPA" => t.disc = t.disc.or_else(|| Tags::number(&text())),
            b"TYER" | b"TYE" | b"TDRC" | b"TDOR" => t.year = t.year.or_else(|| Tags::year(&text())),
            b"APIC" => {
                let mime_end = body.iter().position(|&c| c == 0).map(|i| i + 1);
                if let Some(me) = mime_end {
                    let mime = latin1(&body[..me - 1]).to_ascii_lowercase();
                    let kind = *body.get(me).unwrap_or(&0);
                    if let Some(de) = id3_skip_string(enc, body, me + 1) {
                        t.picture(if mime == "image/jpg" { "image/jpeg" } else { &mime }, kind, &body[de..]);
                    }
                }
            }
            b"PIC" => {
                if body.len() > 5 {
                    let mime = if &body[..3] == b"PNG" { "image/png" } else { "image/jpeg" };
                    if let Some(de) = id3_skip_string(enc, body, 4) {
                        t.picture(mime, body[3], &body[de..]);
                    }
                }
            }
            _ => {}
        }
    }
    Some(end)
}

fn id3v1(f: &mut File, len: u64, t: &mut Tags) -> bool {
    if len < 128 {
        return false;
    }
    let Some(b) = read_at(f, len - 128, 128) else { return false };
    if b.get(..3) != Some(b"TAG") {
        return false;
    }
    let s = |r: std::ops::Range<usize>| latin1(&b[r]).trim_end_matches(['\0', ' ']).to_string();
    Tags::set(&mut t.title, s(3..33));
    Tags::set(&mut t.artist, s(33..63));
    Tags::set(&mut t.album, s(63..93));
    t.year = t.year.or_else(|| Tags::year(&s(93..97)));
    if b[125] == 0 && b[126] != 0 {
        t.track = t.track.or(Some(b[126] as u32));
    }
    true
}

/// MP3 length from the first frame: Xing/Info or VBRI frame count, else
/// the bitrate (constant bitrate files).
fn mp3_duration(f: &mut File, audio: u64, len: u64, v1: bool) -> Option<f64> {
    let b = read_at(f, audio, 64 << 10)?;
    let at = (0..b.len().saturating_sub(4)).find(|&i| b[i] == 0xff && b[i + 1] & 0xe0 == 0xe0 && (b[i + 1] >> 1) & 3 == 1
        && (b[i + 2] >> 4) != 0 && (b[i + 2] >> 4) != 15 && (b[i + 2] >> 2) & 3 != 3)?;
    let (version, bitrate_i, rate_i, mono) = ((b[at + 1] >> 3) & 3, (b[at + 2] >> 4) as usize, ((b[at + 2] >> 2) & 3) as usize, b[at + 3] >> 6 == 3);
    let mpeg1 = version == 3;
    let rate = [44100.0, 48000.0, 32000.0][rate_i] / if mpeg1 { 1.0 } else if version == 2 { 2.0 } else { 4.0 };
    let kbps = if mpeg1 { [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320][bitrate_i] }
               else { [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160][bitrate_i] } as f64;
    let spf = if mpeg1 { 1152.0 } else { 576.0 };
    let side = match (mpeg1, mono) { (true, false) => 32, (true, true) => 17, (false, false) => 17, (false, true) => 9 };
    let x = at + 4 + side;
    if matches!(b.get(x..x + 4), Some(b"Xing") | Some(b"Info")) {
        let flags = be32(&b, x + 4)?;
        if flags & 1 != 0 {
            return Some(be32(&b, x + 8)? as f64 * spf / rate).filter(|d| *d > 0.0);
        }
    }
    if b.get(at + 36..at + 40) == Some(b"VBRI") {
        return Some(be32(&b, at + 36 + 14)? as f64 * spf / rate).filter(|d| *d > 0.0);
    }
    let bytes = len.saturating_sub(audio + at as u64 + if v1 { 128 } else { 0 });
    (kbps > 0.0).then(|| bytes as f64 * 8.0 / (kbps * 1000.0))
}

// ---------- FLAC ----------

fn flac_picture(t: &mut Tags, b: &[u8]) {
    (|| -> Option<()> {
        let kind = be32(b, 0)?;
        let ml = be32(b, 4)? as usize;
        let mime = String::from_utf8_lossy(b.get(8..8 + ml)?).to_ascii_lowercase();
        let dl = be32(b, 8 + ml)? as usize;
        let at = 8 + ml + 4 + dl + 16;
        let n = be32(b, at)? as usize;
        t.picture(&mime, kind.min(255) as u8, b.get(at + 4..at + 4 + n)?);
        Some(())
    })();
}

fn vorbis_comments(t: &mut Tags, b: &[u8]) {
    let Some(vl) = le32(b, 0) else { return };
    let mut p = 4 + vl as usize;
    let Some(n) = le32(b, p) else { return };
    p += 4;
    for _ in 0..n.min(10_000) {
        let Some(l) = le32(b, p) else { return };
        let Some(c) = b.get(p + 4..p + 4 + l as usize) else { return };
        p += 4 + l as usize;
        let c = String::from_utf8_lossy(c);
        if let Some((k, v)) = c.split_once('=') {
            t.comment(k, v);
        }
    }
}

fn flac(f: &mut File, start: u64, t: &mut Tags) -> Option<()> {
    if read_at(f, start, 4)? != b"fLaC" {
        return None;
    }
    let mut p = start + 4;
    for _ in 0..256 {
        let h = read_at(f, p, 4)?;
        if h.len() < 4 {
            break;
        }
        let (last, kind) = (h[0] & 0x80 != 0, h[0] & 0x7f);
        let len = (h[1] as u64) << 16 | (h[2] as u64) << 8 | h[3] as u64;
        if matches!(kind, 0 | 4 | 6) {
            let b = read_at(f, p + 4, len.min(MAX_TAG))?;
            match kind {
                0 => {
                    let bits = be64(&b, 10)?;
                    let (rate, samples) = (bits >> 44, bits & ((1 << 36) - 1));
                    if rate > 0 && samples > 0 {
                        t.duration = Some(samples as f64 / rate as f64);
                    }
                }
                4 => vorbis_comments(t, &b),
                _ => flac_picture(t, &b),
            }
        }
        p += 4 + len;
        if last {
            break;
        }
    }
    Some(())
}

// ---------- Ogg (Vorbis, Opus) ----------

/// The first `want` packets of the first stream in an Ogg file's start.
fn ogg_packets(b: &[u8], want: usize) -> Vec<Vec<u8>> {
    let (mut packets, mut cur, mut p) = (Vec::new(), Vec::new(), 0usize);
    while p + 27 <= b.len() && packets.len() < want {
        if &b[p..p + 4] != b"OggS" {
            break;
        }
        let n = b[p + 26] as usize;
        let Some(segs) = b.get(p + 27..p + 27 + n) else { break };
        let mut d = p + 27 + n;
        for &s in segs {
            let Some(chunk) = b.get(d..d + s as usize) else { return packets };
            cur.extend_from_slice(chunk);
            d += s as usize;
            if s < 255 {
                packets.push(std::mem::take(&mut cur));
                if packets.len() >= want {
                    break;
                }
            }
        }
        p = d;
    }
    packets
}

fn ogg(f: &mut File, len: u64, t: &mut Tags) -> Option<()> {
    let head = read_at(f, 0, MAX_TAG.min(len))?;
    let packets = ogg_packets(&head, 2);
    let first = packets.first()?;
    let (rate, preskip) = if first.starts_with(b"\x01vorbis") {
        (le32(first, 12)? as f64, 0.0)
    } else if first.starts_with(b"OpusHead") {
        (48000.0, le16(first, 10)? as f64)
    } else {
        return None;
    };
    if let Some(c) = packets.get(1) {
        if let Some(body) = c.strip_prefix(b"\x03vorbis").or_else(|| c.strip_prefix(b"OpusTags")) {
            vorbis_comments(t, body);
        }
    }
    let tail_at = len.saturating_sub(64 << 10);
    let tail = read_at(f, tail_at, 64 << 10)?;
    if let Some(i) = tail.windows(4).rposition(|w| w == b"OggS") {
        let granule = le64(&tail, i + 6)?;
        if granule != u64::MAX && rate > 0.0 {
            t.duration = Some(((granule as f64 - preskip) / rate).max(0.0));
        }
    }
    Some(())
}

// ---------- MP4 / M4A ----------

/// Child atoms in b[from..to]: (type, data start, data end).
fn atoms(b: &[u8], from: usize, to: usize) -> Vec<([u8; 4], usize, usize)> {
    let mut out = Vec::new();
    let mut p = from;
    while p + 8 <= to.min(b.len()) && out.len() < 4096 {
        let size = be32(b, p).unwrap_or(0) as usize;
        let kind: [u8; 4] = b[p + 4..p + 8].try_into().unwrap();
        let (hdr, size) = if size == 1 { (16, be64(b, p + 8).unwrap_or(0) as usize) } else if size == 0 { (8, to - p) } else { (8, size) };
        if size < hdr || p + size > to.min(b.len()) {
            break;
        }
        out.push((kind, p + hdr, p + size));
        p += size;
    }
    out
}

fn mp4(f: &mut File, len: u64, t: &mut Tags) -> Option<()> {
    // Find moov at the top level (it may come after the audio).
    let mut p = 0u64;
    let mut moov = None;
    for _ in 0..64 {
        let h = read_at(f, p, 16)?;
        if h.len() < 8 {
            break;
        }
        let mut size = be32(&h, 0)? as u64;
        if size == 1 {
            size = be64(&h, 8)?;
        } else if size == 0 {
            size = len - p;
        }
        if size < 8 {
            break;
        }
        if &h[4..8] == b"moov" {
            moov = Some(read_at(f, p, size.min(MAX_TAG * 2))?);
            break;
        }
        p += size;
        if p >= len {
            break;
        }
    }
    let m = moov?;
    let find = |kind: &[u8; 4], from: usize, to: usize| atoms(&m, from, to).into_iter().find(|a| &a.0 == kind);
    let (_, ms, me) = find(b"moov", 0, m.len())?;
    if let Some((_, hs, _)) = find(b"mvhd", ms, me) {
        let v = m[hs];
        let (scale, dur) = if v == 1 { (be32(&m, hs + 20)? as f64, be64(&m, hs + 24)? as f64) } else { (be32(&m, hs + 12)? as f64, be32(&m, hs + 16)? as f64) };
        if scale > 0.0 {
            t.duration = Some(dur / scale);
        }
    }
    let meta = find(b"udta", ms, me).and_then(|(_, us, ue)| find(b"meta", us, ue)).or_else(|| find(b"meta", ms, me));
    let (_, ts, te) = meta?;
    let (_, is, ie) = find(b"ilst", ts + 4, te)?;
    for (kind, s, e) in atoms(&m, is, ie) {
        let Some((_, ds, de)) = atoms(&m, s, e).into_iter().find(|a| &a.0 == b"data") else { continue };
        let Some(v) = m.get(ds + 8..de) else { continue };
        let text = || String::from_utf8_lossy(v).into_owned();
        match &kind {
            b"\xa9nam" => Tags::set(&mut t.title, text()),
            b"\xa9ART" => Tags::set(&mut t.artist, text()),
            b"\xa9alb" => Tags::set(&mut t.album, text()),
            b"aART" => Tags::set(&mut t.album_artist, text()),
            b"\xa9gen" => Tags::set(&mut t.genre, text()),
            b"\xa9day" => t.year = t.year.or_else(|| Tags::year(&text())),
            b"trkn" => t.track = t.track.or_else(|| be16(v, 2).map(u32::from).filter(|&n| n > 0)),
            b"disk" => t.disc = t.disc.or_else(|| be16(v, 2).map(u32::from).filter(|&n| n > 0)),
            b"covr" => {
                let kind = be32(&m, ds).unwrap_or(0) & 0xffffff;
                t.picture(if kind == 14 { "image/png" } else { "image/jpeg" }, 3, v);
            }
            _ => {}
        }
    }
    Some(())
}

// ---------- WAV ----------

fn wav(f: &mut File, len: u64, t: &mut Tags) -> Option<()> {
    let mut p = 12u64;
    let mut byte_rate = 0u32;
    for _ in 0..64 {
        let h = read_at(f, p, 8)?;
        if h.len() < 8 {
            break;
        }
        let size = le32(&h, 4)? as u64;
        match &h[..4] {
            b"fmt " => byte_rate = le32(&read_at(f, p + 8, 16)?, 8).unwrap_or(0),
            b"data" if byte_rate > 0 => t.duration = Some(size.min(len) as f64 / byte_rate as f64),
            b"LIST" => {
                let b = read_at(f, p + 8, size.min(1 << 20))?;
                if b.get(..4) == Some(b"INFO") {
                    let mut q = 4;
                    while q + 8 <= b.len() {
                        let n = le32(&b, q + 4)? as usize;
                        let v = String::from_utf8_lossy(b.get(q + 8..q + 8 + n)?).into_owned();
                        match &b[q..q + 4] {
                            b"INAM" => Tags::set(&mut t.title, v),
                            b"IART" => Tags::set(&mut t.artist, v),
                            b"IPRD" => Tags::set(&mut t.album, v),
                            b"IGNR" => Tags::set(&mut t.genre, v),
                            b"ICRD" => t.year = t.year.or_else(|| Tags::year(&v)),
                            b"ITRK" => t.track = t.track.or_else(|| Tags::number(&v)),
                            _ => {}
                        }
                        q += 8 + n + (n & 1);
                    }
                }
            }
            _ => {}
        }
        p += 8 + size + (size & 1);
        if p >= len {
            break;
        }
    }
    Some(())
}

/// The tags of a song file, by its first bytes (not its name).
pub fn read(path: &Path) -> Option<Tags> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let head = read_at(&mut f, 0, 12)?;
    let mut t = Tags::default();
    if head.starts_with(b"fLaC") {
        flac(&mut f, 0, &mut t)?;
    } else if head.starts_with(b"OggS") {
        ogg(&mut f, len, &mut t)?;
    } else if head.get(4..8) == Some(b"ftyp") {
        mp4(&mut f, len, &mut t)?;
    } else if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WAVE") {
        wav(&mut f, len, &mut t)?;
    } else {
        // MP3: ID3v2 (maybe), frames, ID3v1 (maybe). FLAC behind ID3 too.
        let audio = id3v2(&mut f, &mut t).unwrap_or(0);
        if read_at(&mut f, audio, 4).as_deref() == Some(b"fLaC") {
            flac(&mut f, audio, &mut t)?;
            return Some(t);
        }
        let v1 = id3v1(&mut f, len, &mut t);
        t.duration = mp3_duration(&mut f, audio, len, v1);
        if t.duration.is_none() && audio == 0 && !v1 {
            return None; // not a song file
        }
    }
    Some(t)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn id3_frame(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [id.to_vec(), (body.len() as u32).to_be_bytes().to_vec(), vec![0, 0], body.to_vec()].concat()
    }

    fn syncsafe_bytes(n: u32) -> [u8; 4] {
        [(n >> 21 & 0x7f) as u8, (n >> 14 & 0x7f) as u8, (n >> 7 & 0x7f) as u8, (n & 0x7f) as u8]
    }

    /// An MP3: ID3v2.3 (title in UTF-16, artist, album, track, year, a
    /// front cover) and 100 CBR 128 kb/s frames.
    pub fn mp3(title: &str, artist: &str, album: &str, track: &str) -> Vec<u8> {
        let mut frames = Vec::new();
        let utf16: Vec<u8> = [vec![1, 0xff, 0xfe], title.encode_utf16().flat_map(u16::to_le_bytes).collect()].concat();
        frames.extend(id3_frame(b"TIT2", &utf16));
        frames.extend(id3_frame(b"TPE1", &[&[3u8][..], artist.as_bytes()].concat()));
        frames.extend(id3_frame(b"TALB", &[&[0u8][..], album.as_bytes()].concat()));
        frames.extend(id3_frame(b"TRCK", &[&[0u8][..], track.as_bytes()].concat()));
        frames.extend(id3_frame(b"TYER", b"\x002019"));
        frames.extend(id3_frame(b"TCON", b"\x00(17)Rock"));
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0];
        jpeg.extend([7u8; 40]);
        frames.extend(id3_frame(b"APIC", &[&b"\x00image/jpeg\x00\x03cover\x00"[..], &jpeg].concat()));
        let mut b = b"ID3\x03\x00\x00".to_vec();
        b.extend(syncsafe_bytes(frames.len() as u32));
        b.extend(frames);
        // MPEG1 layer 3, 128 kb/s, 44.1 kHz: 417 bytes a frame.
        for _ in 0..100 {
            let mut fr = vec![0xff, 0xfb, 0x90, 0x00];
            fr.resize(417, 0);
            b.extend(fr);
        }
        b
    }

    fn le32b(n: u32) -> [u8; 4] { n.to_le_bytes() }

    pub fn flac_file(title: &str, artist: &str, seconds: u64) -> Vec<u8> {
        let mut b = b"fLaC".to_vec();
        let mut si = vec![0u8; 34];
        let bits: u64 = (44100u64 << 44) | (1u64 << 41) | (15u64 << 36) | (44100 * seconds);
        si[10..18].copy_from_slice(&bits.to_be_bytes());
        b.extend([0, 0, 0, 34]);
        b.extend(si);
        let mut vc = Vec::new();
        vc.extend(le32b(4));
        vc.extend(b"nexa");
        let cs = [format!("TITLE={title}"), format!("ARTIST={artist}"), "ALBUM=Hexagons".into(), "TRACKNUMBER=2/9".into(), "DATE=2021-05-01".into()];
        vc.extend(le32b(cs.len() as u32));
        for c in &cs {
            vc.extend(le32b(c.len() as u32));
            vc.extend(c.as_bytes());
        }
        b.push(0x84);
        b.extend(&(vc.len() as u32).to_be_bytes()[1..]);
        b.extend(vc);
        b
    }

    fn ogg_page(granule: u64, packets: &[&[u8]]) -> Vec<u8> {
        let mut segs = Vec::new();
        let mut data: Vec<u8> = Vec::new();
        for p in packets {
            let mut n = p.len();
            while n >= 255 {
                segs.push(255u8);
                n -= 255;
            }
            segs.push(n as u8);
            data.extend(*p);
        }
        let mut b = b"OggS\x00\x00".to_vec();
        b.extend(granule.to_le_bytes());
        b.extend([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        b.push(segs.len() as u8);
        b.extend(segs);
        b.extend(data);
        b
    }

    pub fn opus_file(title: &str, seconds: u64) -> Vec<u8> {
        let mut head = b"OpusHead\x01\x02".to_vec();
        head.extend(312u16.to_le_bytes());
        head.extend([0x80, 0xbb, 0, 0, 0, 0, 0]);
        let mut tags = b"OpusTags".to_vec();
        tags.extend(le32b(4));
        tags.extend(b"nexa");
        let c = format!("TITLE={title}");
        let a = "ARTIST=Opus Band";
        tags.extend(le32b(2));
        tags.extend(le32b(c.len() as u32));
        tags.extend(c.as_bytes());
        tags.extend(le32b(a.len() as u32));
        tags.extend(a.as_bytes());
        let mut b = ogg_page(0, &[&head]);
        b.extend(ogg_page(0, &[&tags]));
        b.extend(ogg_page(48000 * seconds + 312, &[&[0u8; 30]]));
        b
    }

    pub fn m4a_file(title: &str, seconds: u32) -> Vec<u8> {
        fn atom(kind: &[u8], body: &[u8]) -> Vec<u8> {
            [((body.len() + 8) as u32).to_be_bytes().to_vec(), kind.to_vec(), body.to_vec()].concat()
        }
        let data = |v: &[u8]| atom(b"data", &[&[0, 0, 0, 1, 0, 0, 0, 0][..], v].concat());
        let mut mvhd = vec![0u8; 20];
        mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&(seconds * 1000).to_be_bytes());
        let ilst = atom(b"ilst", &[atom(b"\xa9nam", &data(title.as_bytes())), atom(b"\xa9ART", &data(b"M4A Artist")),
                                   atom(b"trkn", &data(&[0, 0, 0, 4, 0, 10, 0, 0]))].concat());
        let meta = atom(b"meta", &[&[0u8, 0, 0, 0][..], &ilst].concat());
        let moov = atom(b"moov", &[atom(b"mvhd", &mvhd), atom(b"udta", &meta)].concat());
        [atom(b"ftyp", b"M4A \0\0\0\0"), atom(b"mdat", &[0u8; 64]), moov].concat()
    }

    fn tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("nexa-musiclib-tags-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn formats() {
        let t = read(&tmp("a.mp3", &mp3("Ünïcode Song", "Nexa Band", "Hexagons", "3/12"))).unwrap();
        assert_eq!(t.title.as_deref(), Some("Ünïcode Song"));
        assert_eq!((t.artist.as_deref(), t.album.as_deref(), t.track, t.year), (Some("Nexa Band"), Some("Hexagons"), Some(3), Some(2019)));
        assert_eq!(t.genre.as_deref(), Some("Rock"));
        assert_eq!(t.picture.as_ref().map(|p| p.0.as_str()), Some("image/jpeg"));
        let d = t.duration.unwrap();
        assert!((d - 41700.0 * 8.0 / 128000.0).abs() < 0.05, "{d}");
        let t = read(&tmp("b.flac", &flac_file("Flac Song", "Flac Band", 125))).unwrap();
        assert_eq!((t.title.as_deref(), t.track, t.year), (Some("Flac Song"), Some(2), Some(2021)));
        assert_eq!(t.duration, Some(125.0));
        let t = read(&tmp("c.opus", &opus_file("Opus Song", 61))).unwrap();
        assert_eq!((t.title.as_deref(), t.artist.as_deref()), (Some("Opus Song"), Some("Opus Band")));
        assert!((t.duration.unwrap() - 61.0).abs() < 0.01);
        let t = read(&tmp("d.m4a", &m4a_file("M4A Song", 200))).unwrap();
        assert_eq!((t.title.as_deref(), t.artist.as_deref(), t.track, t.duration), (Some("M4A Song"), Some("M4A Artist"), Some(4), Some(200.0)));
        assert!(read(&tmp("e.mp3", b"just some text, not a song")).is_none());
        assert_eq!(base64("aGV4YWdvbg==").as_deref(), Some(&b"hexagon"[..]));
    }

    #[test]
    fn damaged_files_never_panic() {
        for full in [mp3("x", "y", "z", "1"), flac_file("x", "y", 3), opus_file("x", 3), m4a_file("x", 3)] {
            for n in (0..full.len()).step_by(if full.len() > 4096 { 97 } else { 7 }) {
                let _ = read(&tmp("cut", &full[..n]));
            }
            let mut junk = full.clone();
            for i in (10..junk.len()).step_by(13) {
                junk[i] = 0xff;
            }
            let _ = read(&tmp("junk", &junk));
        }
    }
}
