//! The archive work: what kind an archive is, safe extraction, compression.
//!
//! Extraction never writes outside a fresh folder it creates itself:
//! entries with ".." or that would land elsewhere are skipped, symbolic
//! links are kept only when they point inside the extracted folder (and are
//! made last, so no file is ever written through one), hard links, devices
//! and pipes are skipped, set-user-id bits are dropped, and the total size
//! is capped by the free space of the drive (a tiny archive can't fill it).
//! Everything goes into a hidden staging folder first; only a complete
//! extraction is renamed into place, so a failure or Stop leaves nothing.
use serde_json::{json, Value};
use std::cell::Cell;
use std::ffi::{CString, OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// Set by SIGTERM/SIGINT (the Files window's Stop button).
pub static STOP: AtomicBool = AtomicBool::new(false);

pub const MAX_ENTRIES: usize = 200_000;
const MAX_LINK: u64 = 4096;
const MAX_LISTED: usize = 5000;
const SKIPPED_SHOWN: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Zip,
    Tar,
    Gz,
    Xz,
    Bz2,
    Zst,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Zip => "zip",
            Kind::Tar => "tar",
            Kind::Gz => "gzip",
            Kind::Xz => "xz",
            Kind::Bz2 => "bzip2",
            Kind::Zst => "zstd",
        }
    }
}

fn stopped() -> io::Error {
    io::Error::other("Stopped")
}

fn check_stop() -> io::Result<()> {
    if STOP.load(Ordering::Relaxed) {
        Err(stopped())
    } else {
        Ok(())
    }
}

fn read_up_to<R: Read + ?Sized>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

fn is_tar_header(b: &[u8]) -> bool {
    b.len() >= 262 && &b[257..262] == b"ustar"
}

/// What kind of archive `path` is, by its first bytes (not its name).
pub fn sniff(path: &Path) -> io::Result<Kind> {
    let mut head = [0u8; 512];
    let n = read_up_to(&mut File::open(path)?, &mut head)?;
    let b = &head[..n];
    Ok(if b.starts_with(b"PK\x03\x04") || b.starts_with(b"PK\x05\x06") || b.starts_with(b"PK\x07\x08") {
        Kind::Zip
    } else if b.starts_with(&[0x1f, 0x8b]) {
        Kind::Gz
    } else if b.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        Kind::Xz
    } else if b.starts_with(b"BZh") {
        Kind::Bz2
    } else if b.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        Kind::Zst
    } else if is_tar_header(b) || path.extension() == Some(OsStr::new("tar")) {
        Kind::Tar
    } else {
        return Err(io::Error::other("This isn't an archive NexaOS can open (zip, tar, gz, xz, bz2 or zst)"));
    })
}

const SUFFIXES: [&str; 15] = [
    ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tgz", ".txz", ".tbz2", ".tbz", ".tzst", ".zip", ".tar", ".gz", ".xz",
    ".bz2", ".zst",
];

/// "photos.tar.gz" -> "photos": the name of the folder it unpacks to.
pub fn stem(name: &OsStr) -> OsString {
    let b = name.as_bytes();
    let lower = b.to_ascii_lowercase();
    for s in SUFFIXES {
        if lower.len() > s.len() && lower.ends_with(s.as_bytes()) {
            return OsString::from_vec(b[..b.len() - s.len()].to_vec());
        }
    }
    name.to_os_string()
}

/// An entry's path inside the archive, made safe: leading "/" dropped,
/// "." ignored, ".." refused (None), Windows "\" read as "/".
pub fn clean(raw: &[u8]) -> Option<PathBuf> {
    let unified: Vec<u8> = raw.iter().map(|&c| if c == b'\\' { b'/' } else { c }).collect();
    if unified.contains(&0) {
        return None;
    }
    let mut out = PathBuf::new();
    for c in Path::new(OsStr::from_bytes(&unified)).components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Does a symbolic link at `entry` pointing to `target` stay inside the
/// extracted folder?
pub fn link_inside(entry: &Path, target: &Path) -> bool {
    if target.is_absolute() || target.as_os_str().is_empty() {
        return false;
    }
    let mut depth: Vec<&OsStr> = entry.parent().map(|p| p.iter().collect()).unwrap_or_default();
    for c in target.components() {
        match c {
            Component::Normal(s) => depth.push(s),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth.pop().is_none() {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

/// "name", "name (2)", "name (3)"... keeping the extension after the number.
fn candidate(name: &OsStr, n: usize, is_dir: bool) -> OsString {
    if n == 0 {
        return name.to_os_string();
    }
    let b = name.as_bytes();
    let lower = b.to_ascii_lowercase();
    let ext_len = if is_dir {
        0
    } else {
        SUFFIXES
            .iter()
            .find(|s| lower.len() > s.len() && lower.ends_with(s.as_bytes()))
            .map(|s| s.len())
            .or_else(|| b.iter().rposition(|&c| c == b'.').filter(|&i| i > 0).map(|i| b.len() - i))
            .unwrap_or(0)
    };
    let mut v = b[..b.len() - ext_len].to_vec();
    v.extend_from_slice(format!(" ({})", n + 1).as_bytes());
    v.extend_from_slice(&b[b.len() - ext_len..]);
    OsString::from_vec(v)
}

/// Claim the first free name for `name` in `dir`; `make` must fail with
/// AlreadyExists when the name is taken.
pub fn place(dir: &Path, name: &OsStr, is_dir: bool, mut make: impl FnMut(&Path) -> io::Result<()>) -> io::Result<PathBuf> {
    for n in 0..10_000 {
        let target = dir.join(candidate(name, n, is_dir));
        match make(&target) {
            Ok(()) => return Ok(target),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::from_raw_os_error(libc::EEXIST))
}

/// Rename that fails with AlreadyExists instead of replacing anything.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let a = CString::new(from.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let b = CString::new(to.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    const RENAME_NOREPLACE: libc::c_uint = 1;
    // SAFETY: a and b are valid C strings; renameat2 only reads them.
    let rc = unsafe { libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), RENAME_NOREPLACE) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Bytes free for us on the drive holding `dir`.
pub fn free_bytes(dir: &Path) -> u64 {
    let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else { return u64::MAX };
    // SAFETY: zeroed is a valid statvfs; statvfs fills it.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c is a valid C string and st a valid out-pointer.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return u64::MAX;
    }
    (st.f_bavail as u64).saturating_mul(st.f_frsize as u64)
}

/// Progress lines on stdout, at most ~7 a second.
pub struct Progress {
    pub total: u64,
    pub quiet: bool,
    last: Option<Instant>,
}

impl Progress {
    pub fn new(total: u64) -> Progress {
        Progress { total, quiet: false, last: None }
    }

    pub fn show(&mut self, done: u64, current: &Path) {
        if self.quiet || self.last.is_some_and(|t| t.elapsed() < Duration::from_millis(150)) {
            return;
        }
        self.last = Some(Instant::now());
        let name = current.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut out = io::stdout().lock();
        let _ = writeln!(out, "{}", json!({"progress": {"done": done.min(self.total), "total": self.total, "current": name}}));
        let _ = out.flush();
    }
}

/// Counts what is read through it (archive bytes, for progress) and stops
/// at once when asked to.
pub struct Counted<R> {
    inner: R,
    pub n: Rc<Cell<u64>>,
}

impl<R> Counted<R> {
    pub fn new(inner: R) -> Counted<R> {
        Counted { inner, n: Rc::new(Cell::new(0)) }
    }
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        check_stop()?;
        let k = self.inner.read(b)?;
        self.n.set(self.n.get() + k as u64);
        Ok(k)
    }
}

impl<R: Seek> Seek for Counted<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// What an extraction may still write.
pub struct Budget {
    pub bytes_left: u64,
    pub entries: usize,
    pub files: usize,
    pub skipped: Vec<String>,
}

impl Budget {
    pub fn new(bytes: u64) -> Budget {
        Budget { bytes_left: bytes, entries: 0, files: 0, skipped: Vec::new() }
    }

    fn skip(&mut self, name: &[u8], why: &str) {
        self.skipped.push(format!("{}: {why}", String::from_utf8_lossy(name)));
    }

    fn entry(&mut self) -> io::Result<()> {
        self.entries += 1;
        if self.entries > MAX_ENTRIES {
            return Err(io::Error::other(format!("The archive has more than {MAX_ENTRIES} items; not extracted")));
        }
        check_stop()
    }
}

fn too_big() -> io::Error {
    io::Error::other("It doesn't fit: the archive unpacks to more than the free space on this drive")
}

fn make_dirs(root: &Path, rel: &Path) -> io::Result<()> {
    fs::create_dir_all(root.join(rel))
}

/// Write one file at `root/rel` from `src`: a new file only (never
/// following a link, never replacing), within the budget.
fn write_file(root: &Path, rel: &Path, src: &mut dyn Read, mode: u32, mtime: Option<i64>, budget: &mut Budget) -> io::Result<bool> {
    let dest = root.join(rel);
    if let Some(dir) = rel.parent() {
        make_dirs(root, dir)?;
    }
    let mut f = match fs::OpenOptions::new().write(true).create_new(true).custom_flags(libc::O_NOFOLLOW).mode(0o600).open(&dest) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    };
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let k = match src.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if k as u64 > budget.bytes_left {
            return Err(too_big());
        }
        budget.bytes_left -= k as u64;
        f.write_all(&buf[..k])?;
    }
    // Keep read/write/run for owner, group and others; never set-id or sticky.
    let perm = (mode & 0o777) | 0o400;
    f.set_permissions(fs::Permissions::from_mode(perm))?;
    if let Some(t) = mtime.filter(|t| *t > 0) {
        let _ = f.set_modified(UNIX_EPOCH + Duration::from_secs(t as u64));
    }
    budget.files += 1;
    Ok(true)
}

/// Extract a tar stream into `root`. Links are made last.
fn extract_tar(r: impl Read, root: &Path, budget: &mut Budget, prog: &mut Progress, read: &Rc<Cell<u64>>) -> io::Result<()> {
    let mut ar = tar::Archive::new(r);
    let mut links: Vec<(PathBuf, PathBuf)> = Vec::new();
    for e in ar.entries()? {
        let mut e = e?;
        budget.entry()?;
        let raw = e.path_bytes().into_owned();
        let Some(rel) = clean(&raw) else {
            budget.skip(&raw, "it would land outside the folder");
            continue;
        };
        let kind = e.header().entry_type();
        let mode = e.header().mode().unwrap_or(0o644);
        let mtime = e.header().mtime().ok().map(|t| t as i64);
        if kind.is_dir() {
            make_dirs(root, &rel)?;
        } else if kind.is_file() || kind == tar::EntryType::Continuous {
            if !write_file(root, &rel, &mut e, mode, mtime, budget)? {
                budget.skip(&raw, "it appears twice");
            }
        } else if kind.is_symlink() {
            match e.link_name_bytes() {
                Some(t) if link_inside(&rel, Path::new(OsStr::from_bytes(&t))) => {
                    links.push((rel.clone(), PathBuf::from(OsStr::from_bytes(&t))));
                }
                _ => budget.skip(&raw, "a link pointing outside the folder"),
            }
        } else if kind.is_hard_link() {
            budget.skip(&raw, "a hard link");
        } else if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() || kind.is_gnu_longname() || kind.is_gnu_longlink() {
            // Read by the tar library itself (long names, extended headers).
        } else {
            budget.skip(&raw, "a special file (device or pipe)");
        }
        prog.show(read.get(), &rel);
    }
    make_links(root, links, budget)
}

fn make_links(root: &Path, links: Vec<(PathBuf, PathBuf)>, budget: &mut Budget) -> io::Result<()> {
    for (rel, target) in links {
        if let Some(dir) = rel.parent() {
            make_dirs(root, dir)?;
        }
        if symlink(&target, root.join(&rel)).is_err() {
            budget.skip(rel.as_os_str().as_bytes(), "a link that couldn't be made");
        }
    }
    Ok(())
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn zip_time(t: Option<zip::DateTime>) -> Option<i64> {
    let t = t?;
    let days = days_from_civil(t.year() as i64, t.month() as i64, t.day() as i64);
    Some(days * 86_400 + t.hour() as i64 * 3600 + t.minute() as i64 * 60 + t.second() as i64)
}

fn zip_err(e: zip::result::ZipError) -> io::Error {
    match e {
        zip::result::ZipError::Io(e) => e,
        zip::result::ZipError::UnsupportedArchive(m) if m.to_lowercase().contains("password") => {
            io::Error::other("This archive is protected with a password; NexaOS can't open those yet")
        }
        other => io::Error::other(format!("The archive is damaged or unsupported ({other})")),
    }
}

fn extract_zip(path: &Path, root: &Path, budget: &mut Budget, prog: &mut Progress) -> io::Result<()> {
    let mut z = zip::ZipArchive::new(Counted::new(BufReader::new(File::open(path)?))).map_err(zip_err)?;
    let mut links = Vec::new();
    let mut done = 0u64;
    for i in 0..z.len() {
        budget.entry()?;
        let mut f = z.by_index(i).map_err(zip_err)?;
        done += f.compressed_size();
        let raw = f.name_raw().to_vec();
        let Some(rel) = clean(&raw) else {
            budget.skip(&raw, "it would land outside the folder");
            continue;
        };
        let mode = f.unix_mode().unwrap_or(0o644);
        if f.is_dir() {
            make_dirs(root, &rel)?;
        } else if mode & 0o170000 == 0o120000 {
            let mut t = Vec::new();
            (&mut f).take(MAX_LINK).read_to_end(&mut t)?;
            if link_inside(&rel, Path::new(OsStr::from_bytes(&t))) {
                links.push((rel.clone(), PathBuf::from(OsString::from_vec(t))));
            } else {
                budget.skip(&raw, "a link pointing outside the folder");
            }
        } else if mode & 0o170000 == 0 || mode & 0o170000 == 0o100000 {
            let mtime = zip_time(f.last_modified());
            if !write_file(root, &rel, &mut f, mode, mtime, budget)? {
                budget.skip(&raw, "it appears twice");
            }
        } else {
            budget.skip(&raw, "a special file (device or pipe)");
        }
        prog.show(done, &rel);
    }
    make_links(root, links, budget)
}

/// A compressed stream: a tar inside (most), or one compressed file.
fn extract_stream(r: impl Read, single_name: &OsStr, root: &Path, budget: &mut Budget, prog: &mut Progress, read: &Rc<Cell<u64>>) -> io::Result<()> {
    let mut r = r;
    let mut head = vec![0u8; 512];
    let n = read_up_to(&mut r, &mut head)?;
    head.truncate(n);
    let whole = Cursor::new(head.clone()).chain(r);
    if is_tar_header(&head) {
        extract_tar(whole, root, budget, prog, read)
    } else {
        budget.entry()?;
        let mut whole = whole;
        write_file(root, Path::new(single_name), &mut whole, 0o644, None, budget)?;
        prog.show(read.get(), Path::new(single_name));
        Ok(())
    }
}

/// Unpack `archive` next to it in `dest_dir`: one item comes out as itself,
/// several in a new folder named after the archive.
pub fn extract(archive: &Path, dest_dir: &Path, prog: &mut Progress) -> io::Result<Value> {
    let kind = sniff(archive)?;
    let file_name = archive.file_name().unwrap_or(OsStr::new("Archive"));
    let base = stem(file_name);
    let free = free_bytes(dest_dir);
    let mut budget = Budget::new(free.saturating_sub(free / 20 + (64 << 20)));
    let mut hidden = OsString::from(".");
    hidden.push(&base);
    hidden.push(".extracting");
    let staging = place(dest_dir, &hidden, true, |t| fs::create_dir(t))?;
    let work = (|| -> io::Result<()> {
        let src = || -> io::Result<Counted<BufReader<File>>> { Ok(Counted::new(BufReader::new(File::open(archive)?))) };
        match kind {
            Kind::Zip => extract_zip(archive, &staging, &mut budget, prog),
            Kind::Tar => {
                let s = src()?;
                let n = s.n.clone();
                extract_tar(s, &staging, &mut budget, prog, &n)
            }
            Kind::Gz => {
                let s = src()?;
                let n = s.n.clone();
                extract_stream(flate2::read::MultiGzDecoder::new(s), &base, &staging, &mut budget, prog, &n)
            }
            Kind::Xz => {
                let s = src()?;
                let n = s.n.clone();
                extract_stream(xz2::read::XzDecoder::new_multi_decoder(s), &base, &staging, &mut budget, prog, &n)
            }
            Kind::Bz2 => {
                let s = src()?;
                let n = s.n.clone();
                extract_stream(bzip2::read::MultiBzDecoder::new(s), &base, &staging, &mut budget, prog, &n)
            }
            Kind::Zst => {
                let s = src()?;
                let n = s.n.clone();
                extract_stream(zstd::stream::read::Decoder::new(s)?, &base, &staging, &mut budget, prog, &n)
            }
        }
    })();
    if let Err(e) = work {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    let mut inside: Vec<PathBuf> = fs::read_dir(&staging)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    let fin = match inside.len() {
        0 => {
            let _ = fs::remove_dir_all(&staging);
            return Err(io::Error::other(if budget.skipped.is_empty() {
                "The archive is empty".to_string()
            } else {
                format!("Nothing could be extracted safely ({})", budget.skipped[0])
            }));
        }
        1 => {
            let only = inside.pop().unwrap();
            let is_dir = fs::symlink_metadata(&only).map(|m| m.is_dir()).unwrap_or(false);
            let name = only.file_name().unwrap_or(OsStr::new("item")).to_os_string();
            let at = place(dest_dir, &name, is_dir, |t| rename_noreplace(&only, t))?;
            let _ = fs::remove_dir(&staging);
            at
        }
        _ => place(dest_dir, &base, true, |t| rename_noreplace(&staging, t))?,
    };
    let shown: Vec<&String> = budget.skipped.iter().take(SKIPPED_SHOWN).collect();
    Ok(json!({
        "done": format!("Extracted {}", fin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        "path": fin,
        "format": kind.name(),
        "files": budget.files,
        "skipped": shown,
        "skipped_count": budget.skipped.len(),
        "undo": [{"op": "created", "path": fin}],
    }))
}

/// List what an archive holds (at most MAX_LISTED entries).
pub fn list(archive: &Path) -> io::Result<Value> {
    let kind = sniff(archive)?;
    let mut entries = Vec::new();
    let mut total = 0u64;
    let mut count = 0usize;
    let mut add = |name: &[u8], size: u64, dir: bool| {
        count += 1;
        total += size;
        if entries.len() < MAX_LISTED {
            entries.push(json!({"name": String::from_utf8_lossy(name), "size": size, "dir": dir}));
        }
    };
    let tar_list = |r: &mut dyn Read, add: &mut dyn FnMut(&[u8], u64, bool)| -> io::Result<()> {
        let mut head = vec![0u8; 512];
        let n = read_up_to(r, &mut head)?;
        head.truncate(n);
        if !is_tar_header(&head) {
            // One compressed file: its size is what it unpacks to.
            let more = io::copy(r, &mut io::sink())?;
            add(stem(archive.file_name().unwrap_or_default()).as_bytes(), n as u64 + more, false);
            return Ok(());
        }
        let mut ar = tar::Archive::new(Cursor::new(head).chain(r));
        for e in ar.entries()? {
            check_stop()?;
            let e = e?;
            add(&e.path_bytes(), e.size(), e.header().entry_type().is_dir());
        }
        Ok(())
    };
    let open = || -> io::Result<BufReader<File>> { Ok(BufReader::new(File::open(archive)?)) };
    match kind {
        Kind::Zip => {
            let mut z = zip::ZipArchive::new(open()?).map_err(zip_err)?;
            for i in 0..z.len() {
                let f = z.by_index_raw(i).map_err(zip_err)?;
                add(f.name_raw(), f.size(), f.is_dir());
            }
        }
        Kind::Tar => {
            let mut ar = tar::Archive::new(open()?);
            for e in ar.entries()? {
                let e = e?;
                add(&e.path_bytes(), e.size(), e.header().entry_type().is_dir());
            }
        }
        Kind::Gz => tar_list(&mut flate2::read::MultiGzDecoder::new(open()?), &mut add)?,
        Kind::Xz => tar_list(&mut xz2::read::XzDecoder::new_multi_decoder(open()?), &mut add)?,
        Kind::Bz2 => tar_list(&mut bzip2::read::MultiBzDecoder::new(open()?), &mut add)?,
        Kind::Zst => tar_list(&mut zstd::stream::read::Decoder::new(open()?)?, &mut add)?,
    }
    Ok(json!({"format": kind.name(), "count": count, "total_size": total, "entries": entries, "truncated": count > MAX_LISTED}))
}

/// One thing to put in an archive.
struct Item {
    path: PathBuf,
    name: PathBuf,
    meta: fs::Metadata,
}

/// Everything under `paths` (links not followed), parents before children,
/// and how many bytes of files that is. `skip` (the archive being written)
/// is left out.
fn collect(paths: &[PathBuf], skip: &Path) -> io::Result<(Vec<Item>, u64)> {
    let mut items = Vec::new();
    let mut total = 0u64;
    let mut todo: Vec<(PathBuf, PathBuf)> = paths
        .iter()
        .rev()
        .map(|p| (p.clone(), PathBuf::from(p.file_name().unwrap_or(OsStr::new("item")))))
        .collect();
    while let Some((path, name)) = todo.pop() {
        check_stop()?;
        if path == skip {
            continue;
        }
        let meta = fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            let mut kids: Vec<PathBuf> = fs::read_dir(&path)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
            kids.sort();
            for k in kids.into_iter().rev() {
                let kn = name.join(k.file_name().unwrap_or_default());
                todo.push((k, kn));
            }
        } else if meta.is_file() {
            total += meta.len();
        } else if !meta.file_type().is_symlink() {
            continue; // devices, pipes, sockets: not archived
        }
        items.push(Item { path, name, meta });
        if items.len() > MAX_ENTRIES {
            return Err(io::Error::other(format!("More than {MAX_ENTRIES} items; not compressed")));
        }
    }
    Ok((items, total))
}

/// Copies `src` into `dst`, adding to `done` and showing progress.
fn pump(src: &mut impl Read, dst: &mut impl Write, done: &mut u64, prog: &mut Progress, name: &Path) -> io::Result<()> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        check_stop()?;
        let k = match src.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(k) => k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        dst.write_all(&buf[..k])?;
        *done += k as u64;
        prog.show(*done, name);
    }
}

fn write_zip(out: File, items: &[Item], prog: &mut Progress) -> io::Result<File> {
    use zip::write::SimpleFileOptions;
    let mut z = zip::ZipWriter::new(BufWriter::new(out));
    let mut done = 0u64;
    for it in items {
        let name = it.name.to_string_lossy().into_owned();
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(it.meta.mode() & 0o777)
            .large_file(it.meta.len() >= 0xFFFF_FFFF);
        if it.meta.is_dir() {
            z.add_directory(format!("{name}/"), opts).map_err(zip_err)?;
        } else if it.meta.file_type().is_symlink() {
            let target = fs::read_link(&it.path)?;
            z.add_symlink(name, target.to_string_lossy(), opts).map_err(zip_err)?;
        } else {
            z.start_file(name, opts).map_err(zip_err)?;
            pump(&mut File::open(&it.path)?, &mut z, &mut done, prog, &it.name)?;
        }
    }
    let w = z.finish().map_err(zip_err)?;
    w.into_inner().map_err(|e| e.into_error())
}

fn write_tar_gz(out: File, items: &[Item], prog: &mut Progress) -> io::Result<File> {
    let gz = flate2::write::GzEncoder::new(BufWriter::new(out), flate2::Compression::default());
    let mut b = tar::Builder::new(gz);
    b.follow_symlinks(false);
    let mut done = 0u64;
    for it in items {
        check_stop()?;
        let mut h = tar::Header::new_gnu();
        h.set_metadata_in_mode(&it.meta, tar::HeaderMode::Deterministic);
        h.set_mtime(it.meta.mtime().max(0) as u64);
        if it.meta.is_dir() {
            h.set_size(0);
            b.append_data(&mut h, &it.name, io::empty())?;
        } else if it.meta.file_type().is_symlink() {
            h.set_size(0);
            b.append_link(&mut h, &it.name, fs::read_link(&it.path)?)?;
        } else {
            let mut f = File::open(&it.path)?;
            let mut counted = Pumped { f: &mut f, done: &mut done, prog, name: &it.name };
            b.append_data(&mut h, &it.name, &mut counted)?;
        }
    }
    let gz = b.into_inner()?;
    let w = gz.finish()?;
    w.into_inner().map_err(|e| e.into_error())
}

/// A file being read into the tar, showing progress as it goes.
struct Pumped<'a> {
    f: &'a mut File,
    done: &'a mut u64,
    prog: &'a mut Progress,
    name: &'a Path,
}

impl Read for Pumped<'_> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        check_stop()?;
        let k = self.f.read(b)?;
        *self.done += k as u64;
        self.prog.show(*self.done, self.name);
        Ok(k)
    }
}

/// Pack `paths` into a new archive in `dest_dir` ("zip" or "tar.gz").
pub fn compress(paths: &[PathBuf], dest_dir: &Path, format: &str, name: Option<&str>, prog: &mut Progress) -> io::Result<Value> {
    let ext = match format {
        "zip" => ".zip",
        "tar.gz" => ".tar.gz",
        other => return Err(io::Error::other(format!("Can't make a \"{other}\" archive (zip or tar.gz)"))),
    };
    let mut file_name = match name {
        Some(n) if !n.is_empty() && !n.contains('/') && n != "." && n != ".." => OsString::from(n),
        _ if paths.len() == 1 => {
            let p = &paths[0];
            let n = p.file_name().unwrap_or(OsStr::new("Archive"));
            if p.is_dir() {
                n.to_os_string()
            } else {
                // "report.pdf" -> "report"
                let b = n.as_bytes();
                match b.iter().rposition(|&c| c == b'.').filter(|&i| i > 0) {
                    Some(i) => OsString::from_vec(b[..i].to_vec()),
                    None => n.to_os_string(),
                }
            }
        }
        _ => OsString::from("Archive"),
    };
    if !file_name.as_bytes().to_ascii_lowercase().ends_with(ext.as_bytes()) {
        file_name.push(ext);
    }
    let mut out_file = None;
    let out = place(dest_dir, &file_name, false, |t| {
        out_file = Some(fs::OpenOptions::new().write(true).create_new(true).custom_flags(libc::O_NOFOLLOW).mode(0o644).open(t)?);
        Ok(())
    })?;
    let file = out_file.take().expect("placed");
    let work = (|| -> io::Result<usize> {
        let (items, total) = collect(paths, &out)?;
        prog.total = total.max(1);
        let f = if format == "zip" { write_zip(file, &items, prog)? } else { write_tar_gz(file, &items, prog)? };
        f.sync_all()?;
        Ok(items.len())
    })();
    match work {
        Ok(n) => Ok(json!({
            "done": format!("Made {}", out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            "path": out,
            "items": n,
            "undo": [{"op": "created", "path": out}],
        })),
        Err(e) => {
            let _ = fs::remove_file(&out);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static N: AtomicUsize = AtomicUsize::new(0);

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("nexa-archiver-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn quiet() -> Progress {
        let mut p = Progress::new(1);
        p.quiet = true;
        p
    }

    #[test]
    fn clean_refuses_escapes() {
        assert_eq!(clean(b"a/b.txt"), Some(PathBuf::from("a/b.txt")));
        assert_eq!(clean(b"/etc/passwd"), Some(PathBuf::from("etc/passwd")));
        assert_eq!(clean(b"./x"), Some(PathBuf::from("x")));
        assert_eq!(clean(b"a/../../x"), None);
        assert_eq!(clean(b"..\\evil.txt"), None);
        assert_eq!(clean(b""), None);
    }

    #[test]
    fn links_must_stay_inside() {
        assert!(link_inside(Path::new("a/link"), Path::new("b.txt")));
        assert!(link_inside(Path::new("a/link"), Path::new("../c.txt")));
        assert!(!link_inside(Path::new("a/link"), Path::new("../../etc/passwd")));
        assert!(!link_inside(Path::new("link"), Path::new("/etc/passwd")));
    }

    #[test]
    fn names() {
        assert_eq!(stem(OsStr::new("photos.tar.gz")), "photos");
        assert_eq!(stem(OsStr::new("Report.ZIP")), "Report");
        assert_eq!(candidate(OsStr::new("a.tar.gz"), 1, false), "a (2).tar.gz");
        assert_eq!(candidate(OsStr::new("notes.txt"), 2, false), "notes (3).txt");
        assert_eq!(candidate(OsStr::new("Folder"), 1, true), "Folder (2)");
    }

    fn make_tree(d: &Path) -> PathBuf {
        let src = d.join("proj");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), b"hello").unwrap();
        fs::write(src.join("sub/b.bin"), vec![7u8; 200_000]).unwrap();
        symlink("a.txt", src.join("link")).unwrap();
        src
    }

    #[test]
    fn zip_and_tar_gz_round_trip() {
        for format in ["zip", "tar.gz"] {
            let d = tmp();
            let src = make_tree(&d);
            let out_dir = d.join("out");
            fs::create_dir(&out_dir).unwrap();
            let made = compress(&[src.clone()], &d, format, None, &mut quiet()).unwrap();
            let archive = PathBuf::from(made["path"].as_str().unwrap());
            assert!(archive.file_name().unwrap().to_string_lossy().starts_with("proj."), "{format}: {archive:?}");
            assert_eq!(made["items"], 5);
            let got = extract(&archive, &out_dir, &mut quiet()).unwrap();
            // One folder inside: it comes out as itself, not proj/proj.
            let root = PathBuf::from(got["path"].as_str().unwrap());
            assert_eq!(root, out_dir.join("proj"), "{format}");
            assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"hello");
            assert_eq!(fs::read(root.join("sub/b.bin")).unwrap().len(), 200_000);
            assert_eq!(fs::read_link(root.join("link")).unwrap(), PathBuf::from("a.txt"), "{format}");
            // Again: a new name, nothing replaced.
            let again = extract(&archive, &out_dir, &mut quiet()).unwrap();
            assert_eq!(PathBuf::from(again["path"].as_str().unwrap()), out_dir.join("proj (2)"));
            // No staging folder left behind.
            assert!(fs::read_dir(&out_dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with('.')));
            let listed = list(&archive).unwrap();
            assert_eq!(listed["count"], 5, "{format}: {listed}");
            fs::remove_dir_all(&d).unwrap();
        }
    }

    #[test]
    fn several_items_go_in_a_folder_named_after_the_archive() {
        let d = tmp();
        fs::write(d.join("one.txt"), b"1").unwrap();
        fs::write(d.join("two.txt"), b"2").unwrap();
        let made = compress(&[d.join("one.txt"), d.join("two.txt")], &d, "zip", None, &mut quiet()).unwrap();
        assert_eq!(PathBuf::from(made["path"].as_str().unwrap()), d.join("Archive.zip"));
        let out = d.join("out");
        fs::create_dir(&out).unwrap();
        let got = extract(&d.join("Archive.zip"), &out, &mut quiet()).unwrap();
        let root = PathBuf::from(got["path"].as_str().unwrap());
        assert_eq!(root, out.join("Archive"));
        assert_eq!(fs::read(root.join("two.txt")).unwrap(), b"2");
        fs::remove_dir_all(&d).unwrap();
    }

    /// A hand-made hostile tar: "../" escape, absolute path, outside link,
    /// device file, and one good file.
    #[test]
    fn hostile_tar_is_contained() {
        let d = tmp();
        let mut b = tar::Builder::new(Vec::new());
        let mut add = |name: &[u8], kind: tar::EntryType, data: &[u8], link: Option<&str>| {
            let mut h = tar::Header::new_gnu();
            {
                let raw = h.as_old_mut();
                raw.name[..name.len()].copy_from_slice(name);
            }
            h.set_entry_type(kind);
            h.set_mode(0o4755);
            h.set_size(data.len() as u64);
            if let Some(l) = link {
                h.set_link_name(l).unwrap();
            }
            h.set_cksum();
            b.append(&h, data).unwrap();
        };
        add(b"../escape.txt", tar::EntryType::Regular, b"bad", None);
        add(b"/abs.txt", tar::EntryType::Regular, b"abs", None);
        add(b"good.txt", tar::EntryType::Regular, b"good", None);
        add(b"out", tar::EntryType::Symlink, b"", Some("../../etc/passwd"));
        add(b"dev", tar::EntryType::Char, b"", None);
        let data = b.into_inner().unwrap();
        let archive = d.join("evil.tar");
        fs::write(&archive, data).unwrap();
        let out = d.join("out");
        fs::create_dir(&out).unwrap();
        let got = extract(&archive, &out, &mut quiet()).unwrap();
        let root = PathBuf::from(got["path"].as_str().unwrap());
        assert_eq!(root, out.join("evil"));
        assert!(!d.join("escape.txt").exists() && !out.join("escape.txt").exists());
        assert_eq!(fs::read(root.join("abs.txt")).unwrap(), b"abs"); // "/" dropped: lands inside
        assert_eq!(fs::read(root.join("good.txt")).unwrap(), b"good");
        assert_eq!(fs::metadata(root.join("good.txt")).unwrap().mode() & 0o7000, 0, "set-id bits dropped");
        assert!(fs::symlink_metadata(root.join("out")).is_err());
        assert!(fs::symlink_metadata(root.join("dev")).is_err());
        assert_eq!(got["skipped_count"], 3, "{got}");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn single_compressed_file_and_size_cap() {
        let d = tmp();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&vec![0u8; 5_000_000]).unwrap();
        fs::write(d.join("zeros.bin.gz"), gz.finish().unwrap()).unwrap();
        let out = d.join("out");
        fs::create_dir(&out).unwrap();
        let got = extract(&d.join("zeros.bin.gz"), &out, &mut quiet()).unwrap();
        assert_eq!(PathBuf::from(got["path"].as_str().unwrap()), out.join("zeros.bin"));
        assert_eq!(fs::metadata(out.join("zeros.bin")).unwrap().len(), 5_000_000);
        // With only 1 MB allowed, the same archive is refused and leaves nothing.
        let staging = out.join(".x.extracting");
        fs::create_dir(&staging).unwrap();
        let mut budget = Budget::new(1_000_000);
        let n = Rc::new(Cell::new(0));
        let r = flate2::read::MultiGzDecoder::new(File::open(d.join("zeros.bin.gz")).unwrap());
        let e = extract_stream(r, OsStr::new("zeros.bin"), &staging, &mut budget, &mut quiet(), &n).unwrap_err();
        assert!(e.to_string().contains("doesn't fit"), "{e}");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn not_an_archive() {
        let d = tmp();
        fs::write(d.join("x.txt"), b"just text, long enough to not be anything").unwrap();
        assert!(sniff(&d.join("x.txt")).is_err());
        fs::remove_dir_all(&d).unwrap();
    }
}
