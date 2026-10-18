//! The kernel's process connector (netlink, NETLINK_CONNECTOR / CN_IDX_PROC):
//! one message per fork, exec and exit, pushed by the kernel as it happens.
//! Exits carry the wait status, so a crash (death by SIGSEGV, SIGABRT...) is
//! seen the instant it happens without polling /proc. Needs root in the
//! initial namespaces.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

const NETLINK_CONNECTOR: i32 = 11;
const CN_IDX_PROC: u32 = 1;
const CN_VAL_PROC: u32 = 1;
const PROC_CN_MCAST_LISTEN: u32 = 1;
const NLMSG_HDRLEN: usize = 16;
const CN_MSG_LEN: usize = 20;

const PROC_EVENT_FORK: u32 = 0x0000_0001;
const PROC_EVENT_EXEC: u32 = 0x0000_0002;
const PROC_EVENT_COREDUMP: u32 = 0x4000_0000;
const PROC_EVENT_EXIT: u32 = 0x8000_0000;

#[derive(Debug, PartialEq)]
pub enum Event {
    Fork { parent: i32, child: i32 },
    Exec { pid: i32 },
    /// A whole process ended (not just a thread): exit status or signal.
    Exit { pid: i32, status: i32, signal: i32 },
    CoreDump { pid: i32 },
}

pub struct Connector {
    fd: OwnedFd,
}

impl AsRawFd for Connector {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl Connector {
    pub fn open() -> io::Result<Connector> {
        // SAFETY: plain socket(2); the fd is owned below.
        let raw = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, NETLINK_CONNECTOR) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: raw is a fresh, valid fd we now own.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: sockaddr_nl is plain data; zeroed is valid.
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        addr.nl_groups = CN_IDX_PROC;
        // SAFETY: addr is a valid sockaddr_nl of the given size.
        if unsafe { libc::bind(raw, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_nl>() as u32) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // Subscribe: nlmsghdr + cn_msg + PROC_CN_MCAST_LISTEN.
        let mut msg = Vec::with_capacity(NLMSG_HDRLEN + CN_MSG_LEN + 4);
        let total = (NLMSG_HDRLEN + CN_MSG_LEN + 4) as u32;
        msg.extend_from_slice(&total.to_ne_bytes());
        msg.extend_from_slice(&(libc::NLMSG_DONE as u16).to_ne_bytes());
        msg.extend_from_slice(&0u16.to_ne_bytes());
        msg.extend_from_slice(&0u32.to_ne_bytes());
        // SAFETY: getpid can't fail.
        msg.extend_from_slice(&(unsafe { libc::getpid() } as u32).to_ne_bytes());
        for v in [CN_IDX_PROC, CN_VAL_PROC, 0, 0] {
            msg.extend_from_slice(&v.to_ne_bytes());
        }
        msg.extend_from_slice(&4u16.to_ne_bytes());
        msg.extend_from_slice(&0u16.to_ne_bytes());
        msg.extend_from_slice(&PROC_CN_MCAST_LISTEN.to_ne_bytes());
        // SAFETY: msg is a valid buffer of msg.len() bytes.
        if unsafe { libc::send(raw, msg.as_ptr() as *const libc::c_void, msg.len(), 0) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Connector { fd })
    }

    /// Every event waiting on the socket (none: an empty Vec).
    pub fn drain(&self) -> Vec<Event> {
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            // SAFETY: buf is valid for buf.len() bytes.
            let n = unsafe { libc::recv(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
            if n <= 0 {
                break;
            }
            let mut off = 0usize;
            let n = n as usize;
            // One datagram may hold several netlink messages.
            while off + NLMSG_HDRLEN <= n {
                let len = u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
                if len < NLMSG_HDRLEN || off + len > n {
                    break;
                }
                if let Some(e) = parse(&buf[off + NLMSG_HDRLEN..off + len]) {
                    out.push(e);
                }
                off += (len + 3) & !3;
            }
        }
        out
    }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_ne_bytes(s.try_into().unwrap()))
}

/// One cn_msg + proc_event (the bytes after the netlink header).
pub fn parse(b: &[u8]) -> Option<Event> {
    if u32_at(b, 0)? != CN_IDX_PROC || u32_at(b, 4)? != CN_VAL_PROC {
        return None;
    }
    let ev = b.get(CN_MSG_LEN..)?;
    // proc_event: what u32, cpu u32, timestamp u64, then the event data.
    let what = u32_at(ev, 0)?;
    let d = 16;
    let i = |k: usize| u32_at(ev, d + 4 * k).map(|v| v as i32);
    Some(match what {
        PROC_EVENT_FORK => Event::Fork { parent: i(1)?, child: i(3)? },
        PROC_EVENT_EXEC => Event::Exec { pid: i(1)? },
        PROC_EVENT_EXIT => {
            let (pid, tgid, code) = (i(0)?, i(1)?, i(2)?);
            if pid != tgid {
                return None; // a thread, not the process
            }
            // The wait status: low 7 bits = signal, else status << 8.
            let signal = code & 0x7f;
            Event::Exit { pid: tgid, status: if signal == 0 { (code >> 8) & 0xff } else { 0 }, signal }
        }
        PROC_EVENT_COREDUMP => Event::CoreDump { pid: i(1)? },
        _ => return None,
    })
}

/// Signals that mean "the program crashed" (not "someone asked it to stop").
pub fn is_crash(signal: i32) -> bool {
    matches!(signal, libc::SIGSEGV | libc::SIGBUS | libc::SIGILL | libc::SIGFPE | libc::SIGABRT | libc::SIGSYS | libc::SIGTRAP)
}

pub fn signal_name(signal: i32) -> &'static str {
    match signal {
        libc::SIGSEGV => "a memory error (SIGSEGV)",
        libc::SIGBUS => "a bus error (SIGBUS)",
        libc::SIGILL => "an illegal instruction (SIGILL)",
        libc::SIGFPE => "a maths error (SIGFPE)",
        libc::SIGABRT => "aborting itself (SIGABRT)",
        libc::SIGSYS => "a forbidden system call (SIGSYS)",
        libc::SIGTRAP => "a trap (SIGTRAP)",
        libc::SIGKILL => "being killed (SIGKILL)",
        libc::SIGTERM => "being asked to stop (SIGTERM)",
        _ => "a signal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(what: u32, data: &[u32]) -> Vec<u8> {
        let mut b = Vec::new();
        for v in [CN_IDX_PROC, CN_VAL_PROC, 0, 0] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        b.extend_from_slice(&((16 + data.len() * 4) as u16).to_ne_bytes());
        b.extend_from_slice(&0u16.to_ne_bytes());
        b.extend_from_slice(&what.to_ne_bytes());
        b.extend_from_slice(&0u32.to_ne_bytes());
        b.extend_from_slice(&0u64.to_ne_bytes());
        for v in data {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        b
    }

    #[test]
    fn decodes_events() {
        assert_eq!(parse(&msg(PROC_EVENT_FORK, &[10, 10, 11, 11])), Some(Event::Fork { parent: 10, child: 11 }));
        assert_eq!(parse(&msg(PROC_EVENT_EXEC, &[42, 42])), Some(Event::Exec { pid: 42 }));
        // Death by SIGSEGV with a core dump: wait status 0x8b.
        assert_eq!(parse(&msg(PROC_EVENT_EXIT, &[42, 42, 0x8b, 17, 1, 1])), Some(Event::Exit { pid: 42, status: 0, signal: 11 }));
        // exit(3): status 3 << 8.
        assert_eq!(parse(&msg(PROC_EVENT_EXIT, &[42, 42, 3 << 8, 17, 1, 1])), Some(Event::Exit { pid: 42, status: 3, signal: 0 }));
        assert_eq!(parse(&msg(PROC_EVENT_EXIT, &[43, 42, 0, 17, 1, 1])), None, "thread exits are ignored");
        assert_eq!(parse(&msg(0x200, &[1, 1])), None, "other events ignored");
        assert_eq!(parse(&[1, 2, 3]), None, "short garbage");
        assert!(is_crash(libc::SIGSEGV) && !is_crash(libc::SIGTERM) && !is_crash(libc::SIGKILL));
    }
}
