//! The kernel log (/dev/kmsg), read from "now" onwards, for the one thing
//! no other interface says precisely: which process the out-of-memory killer
//! chose ("Out of memory: Killed process 4242 (firefox) ... UID:1000 ...").

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

#[derive(Debug, PartialEq)]
pub struct OomKill {
    pub pid: i32,
    pub name: String,
    pub uid: Option<u32>,
    /// A container/cgroup limit, not the whole computer.
    pub cgroup: bool,
}

pub struct Kmsg {
    f: File,
}

impl AsRawFd for Kmsg {
    fn as_raw_fd(&self) -> RawFd {
        self.f.as_raw_fd()
    }
}

impl Kmsg {
    /// Open the log positioned at its end: only what happens from now on.
    pub fn open(path: &Path) -> io::Result<Kmsg> {
        let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
        // SAFETY: lseek on an fd we own; SEEK_END skips the existing records.
        unsafe { libc::lseek(f.as_raw_fd(), 0, libc::SEEK_END) };
        Ok(Kmsg { f })
    }

    /// OOM kills logged since the last call.
    pub fn drain(&mut self) -> Vec<OomKill> {
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            // Each read returns exactly one record.
            match self.f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Some(k) = parse(&String::from_utf8_lossy(&buf[..n])) {
                        out.push(k);
                    }
                }
                // Records overwritten before we read them: carry on.
                Err(e) if e.raw_os_error() == Some(libc::EPIPE) => continue,
                Err(_) => break,
            }
        }
        out
    }
}

/// One /dev/kmsg record ("prio,seq,usec,flags;message").
pub fn parse(record: &str) -> Option<OomKill> {
    let msg = record.split_once(';').map(|(_, m)| m).unwrap_or(record);
    let at = msg.find("Killed process ")?;
    let rest = &msg[at + "Killed process ".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let pid = digits.parse().ok()?;
    let name = rest[digits.len()..].strip_prefix(" (")?.split(')').next()?.to_string();
    let uid = msg.split("UID:").nth(1).and_then(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok());
    Some(OomKill { pid, name, uid, cgroup: msg[..at].contains("cgroup") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_victim() {
        let r = "3,2847,96811234,-;Out of memory: Killed process 4242 (firefox) total-vm:4096kB, anon-rss:2048kB, file-rss:0kB, shmem-rss:0kB, UID:1000 pgtables:120kB oom_score_adj:200";
        assert_eq!(parse(r), Some(OomKill { pid: 4242, name: "firefox".into(), uid: Some(1000), cgroup: false }));
        let c = "3,1,1,-;Memory cgroup out of memory: Killed process 77 (llama-server) total-vm:1kB, UID:105 pgtables:1kB";
        assert_eq!(parse(c).unwrap().cgroup, true);
        assert_eq!(parse("6,1,1,-;usb 1-1: new high-speed USB device"), None);
        assert_eq!(parse("3,1,1,-;Killed process abc (x)"), None);
    }
}
