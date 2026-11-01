//! EXIF: a TIFF structure (byte order, IFDs of 12-byte entries) inside a
//! JPEG's APP1 segment, a WebP EXIF chunk or a TIFF file. Every read is
//! bounds-checked; a damaged block gives what could be read.

#[derive(Debug, Default, PartialEq)]
pub struct Exif {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub software: Option<String>,
    pub date: Option<String>,
    pub exposure: Option<String>,
    pub fnumber: Option<String>,
    pub iso: Option<u32>,
    pub focal: Option<String>,
    pub orientation: Option<u16>,
    /// Where the orientation value is, from the start of the TIFF data.
    pub orientation_at: Option<usize>,
    pub gps: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

struct Tiff<'a> {
    b: &'a [u8],
    le: bool,
}

impl<'a> Tiff<'a> {
    fn u16(&self, o: usize) -> Option<u16> {
        let s = self.b.get(o..o + 2)?;
        Some(if self.le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    }
    fn u32(&self, o: usize) -> Option<u32> {
        let s: [u8; 4] = self.b.get(o..o + 4)?.try_into().ok()?;
        Some(if self.le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    }

    /// The entries of the IFD at `at`: (tag, type, count, where the value is).
    fn entries(&self, at: usize) -> Vec<(u16, u16, u32, usize)> {
        let Some(n) = self.u16(at) else { return vec![] };
        (0..n.min(512) as usize).filter_map(|i| {
            let e = at + 2 + i * 12;
            let (tag, kind, count) = (self.u16(e)?, self.u16(e + 2)?, self.u32(e + 4)?);
            let size = match kind { 1 | 2 | 6 | 7 => 1, 3 | 8 => 2, 4 | 9 | 11 => 4, 5 | 10 | 12 => 8, _ => return None } as u64 * count as u64;
            let value = if size <= 4 { e + 8 } else { self.u32(e + 8)? as usize };
            (value as u64 + size <= self.b.len() as u64).then_some((tag, kind, count, value))
        }).collect()
    }

    fn ascii(&self, at: usize, count: u32) -> Option<String> {
        let raw = self.b.get(at..at + count as usize)?;
        let s = String::from_utf8_lossy(raw.split(|&c| c == 0).next().unwrap_or(&[])).trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    fn number(&self, kind: u16, at: usize) -> Option<u32> {
        match kind {
            3 => self.u16(at).map(u32::from),
            4 => self.u32(at),
            _ => None,
        }
    }

    fn rational(&self, at: usize) -> Option<(u32, u32)> {
        let (n, d) = (self.u32(at)?, self.u32(at + 4)?);
        (d != 0).then_some((n, d))
    }
}

fn trim_float(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// EXIF from TIFF-structured bytes (starting with "II*\0" or "MM\0*").
pub fn parse(b: &[u8]) -> Option<Exif> {
    let le = match b.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let t = Tiff { b, le };
    let ifd0 = t.u32(4)? as usize;
    let mut x = Exif::default();
    let mut sub = None;
    for (tag, kind, count, at) in t.entries(ifd0) {
        match tag {
            0x0100 => x.width = t.number(kind, at),
            0x0101 => x.height = t.number(kind, at),
            0x010f => x.make = t.ascii(at, count),
            0x0110 => x.model = t.ascii(at, count),
            0x0112 if kind == 3 => {
                x.orientation = t.u16(at).filter(|o| (1..=8).contains(o));
                x.orientation_at = x.orientation.map(|_| at);
            }
            0x0131 => x.software = t.ascii(at, count),
            0x0132 => x.date = t.ascii(at, count),
            0x8769 => sub = t.number(kind, at).map(|v| v as usize),
            0x8825 => x.gps = true,
            _ => {}
        }
    }
    if let Some(sub) = sub.filter(|&s| s != ifd0) {
        for (tag, kind, count, at) in t.entries(sub) {
            match tag {
                0x829a => x.exposure = t.rational(at).map(|(n, d)| {
                    if n == 0 { "0 s".into() } else if n < d { format!("1/{} s", (d as f64 / n as f64).round()) } else { format!("{} s", trim_float(n as f64 / d as f64, 1)) }
                }),
                0x829d => x.fnumber = t.rational(at).map(|(n, d)| format!("f/{}", trim_float(n as f64 / d as f64, 1))),
                0x8827 => x.iso = t.number(kind, at),
                0x9003 => x.date = t.ascii(at, count).or(x.date.take()),
                0x920a => x.focal = t.rational(at).map(|(n, d)| format!("{} mm", trim_float(n as f64 / d as f64, 1))),
                0xa434 => x.lens = t.ascii(at, count),
                _ => {}
            }
        }
    }
    Some(x)
}

/// EXIF orientation as how to show the stored picture: a 2x2 matrix on
/// (x, y) with y going down. 2 and 4 are mirror images, 6 is a quarter turn
/// clockwise, 5 and 7 a mirror and a turn.
const ORIENT: [[i8; 4]; 8] = [
    [1, 0, 0, 1],    // 1
    [-1, 0, 0, 1],   // 2 mirror left-right
    [-1, 0, 0, -1],  // 3 half turn
    [1, 0, 0, -1],   // 4 mirror top-bottom
    [0, 1, 1, 0],    // 5
    [0, -1, 1, 0],   // 6 quarter turn clockwise
    [0, -1, -1, 0],  // 7
    [0, 1, -1, 0],   // 8 quarter turn anticlockwise
];

fn mul(a: [i8; 4], b: [i8; 4]) -> [i8; 4] {
    [a[0] * b[0] + a[1] * b[2], a[0] * b[1] + a[1] * b[3], a[2] * b[0] + a[3] * b[2], a[2] * b[1] + a[3] * b[3]]
}

/// The orientation after turning or mirroring a picture shown with `o`.
pub fn turn(o: u16, how: &str) -> Option<u16> {
    let m = match how {
        "right" => ORIENT[5],
        "left" => ORIENT[7],
        "flip_h" => ORIENT[1],
        "flip_v" => ORIENT[3],
        _ => return None,
    };
    let now = *ORIENT.get((o as usize).checked_sub(1)?)?;
    let next = mul(m, now);
    ORIENT.iter().position(|x| *x == next).map(|i| i as u16 + 1)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A little-endian TIFF block with IFD0 entries and an EXIF sub-IFD.
    pub fn tiff(orientation: u16) -> Vec<u8> {
        let mut b = b"II*\0".to_vec();
        b.extend(8u32.to_le_bytes());
        // IFD0: Make, Model, Orientation, ExifIFD, GPS: 5 entries.
        let ifd0_len = 2 + 5 * 12 + 4;
        let data0 = 8 + ifd0_len;
        let make = b"NexaCam\0";
        let model = b"Hex 1\0";
        let sub = data0 + make.len() + model.len();
        let mut ifd = vec![];
        ifd.extend(5u16.to_le_bytes());
        let e = |tag: u16, kind: u16, count: u32, val: u32| [tag.to_le_bytes().to_vec(), kind.to_le_bytes().to_vec(), count.to_le_bytes().to_vec(), val.to_le_bytes().to_vec()].concat();
        ifd.extend(e(0x010f, 2, make.len() as u32, data0 as u32));
        ifd.extend(e(0x0110, 2, model.len() as u32, (data0 + make.len()) as u32));
        ifd.extend(e(0x0112, 3, 1, orientation as u32));
        ifd.extend(e(0x8769, 4, 1, sub as u32));
        ifd.extend(e(0x8825, 4, 1, 0));
        ifd.extend(0u32.to_le_bytes());
        b.extend(ifd);
        b.extend(make);
        b.extend(model);
        // EXIF sub-IFD: exposure 1/250, f/2.8, ISO 200, 35 mm, date.
        let sub_len = 2 + 5 * 12 + 4;
        let rats = sub + sub_len;
        let date = b"2026:10:31 18:04:05\0";
        let mut s = vec![];
        s.extend(5u16.to_le_bytes());
        s.extend(e(0x829a, 5, 1, rats as u32));
        s.extend(e(0x829d, 5, 1, (rats + 8) as u32));
        s.extend(e(0x8827, 3, 1, 200));
        s.extend(e(0x920a, 5, 1, (rats + 16) as u32));
        s.extend(e(0x9003, 2, date.len() as u32, (rats + 24) as u32));
        s.extend(0u32.to_le_bytes());
        b.extend(s);
        for (n, d) in [(1u32, 250u32), (28, 10), (35, 1)] {
            b.extend(n.to_le_bytes());
            b.extend(d.to_le_bytes());
        }
        b.extend(date);
        b
    }

    #[test]
    fn reads_exif() {
        let x = parse(&tiff(6)).unwrap();
        assert_eq!(x.make.as_deref(), Some("NexaCam"));
        assert_eq!(x.model.as_deref(), Some("Hex 1"));
        assert_eq!(x.orientation, Some(6));
        assert_eq!(x.exposure.as_deref(), Some("1/250 s"));
        assert_eq!(x.fnumber.as_deref(), Some("f/2.8"));
        assert_eq!(x.iso, Some(200));
        assert_eq!(x.focal.as_deref(), Some("35 mm"));
        assert_eq!(x.date.as_deref(), Some("2026:10:31 18:04:05"));
        assert!(x.gps);
        let at = x.orientation_at.unwrap();
        assert_eq!(&tiff(6)[at..at + 2], &6u16.to_le_bytes());
        // Damaged: cut anywhere, never a panic.
        let full = tiff(1);
        for n in 0..full.len() {
            let _ = parse(&full[..n]);
        }
        assert_eq!(parse(b"nope"), None);
    }

    #[test]
    fn turning() {
        assert_eq!(turn(1, "right"), Some(6));
        assert_eq!(turn(6, "right"), Some(3));
        assert_eq!(turn(3, "right"), Some(8));
        assert_eq!(turn(8, "right"), Some(1));
        assert_eq!(turn(1, "left"), Some(8));
        assert_eq!(turn(6, "left"), Some(1));
        assert_eq!(turn(1, "flip_h"), Some(2));
        assert_eq!(turn(2, "flip_h"), Some(1));
        assert_eq!(turn(1, "flip_v"), Some(4));
        // Mirror then turn right: 7 ("mirror and a quarter turn clockwise").
        assert_eq!(turn(2, "right"), Some(7));
        assert_eq!(turn(9, "right"), None);
        for o in 1..=8 {
            for how in ["right", "left", "flip_h", "flip_v"] {
                let n = turn(o, how).unwrap();
                let back = match how { "right" => "left", "left" => "right", x => x };
                assert_eq!(turn(n, back), Some(o), "{o} {how}");
            }
        }
    }
}
