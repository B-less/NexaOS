//! Landlock: a process confines itself (and everything it later starts) to
//! a fixed set of paths and, on ABI 4+, TCP ports. Used by nexa-mindd (the
//! AI service) and nexa-tools (each assistant action). Raw syscalls, no
//! library. Restrictions can never be lifted once applied.

use std::ffi::CString;
use std::io;
use std::path::Path;

const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;
const SYS_LANDLOCK_ADD_RULE: libc::c_long = 445;
const SYS_LANDLOCK_RESTRICT_SELF: libc::c_long = 446;
const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
const RULE_PATH_BENEATH: u32 = 1;
const RULE_NET_PORT: u32 = 2;

pub const EXECUTE: u64 = 1 << 0;
pub const WRITE_FILE: u64 = 1 << 1;
pub const READ_FILE: u64 = 1 << 2;
pub const READ_DIR: u64 = 1 << 3;
const REMOVE_DIR: u64 = 1 << 4;
const REMOVE_FILE: u64 = 1 << 5;
const MAKE_CHAR: u64 = 1 << 6;
const MAKE_DIR: u64 = 1 << 7;
const MAKE_REG: u64 = 1 << 8;
const MAKE_SOCK: u64 = 1 << 9;
const MAKE_FIFO: u64 = 1 << 10;
const MAKE_BLOCK: u64 = 1 << 11;
pub const MAKE_SYM: u64 = 1 << 12;
const REFER: u64 = 1 << 13; // ABI 2
const TRUNCATE: u64 = 1 << 14; // ABI 3
const NET_BIND_TCP: u64 = 1 << 0;
const NET_CONNECT_TCP: u64 = 1 << 1;
// Landlock ABI 6 "scoping": keep the process from reaching OUT of its own
// Landlock domain. SIGNAL stops it signalling processes outside the domain
// (its own children stay allowed), so a confined app can't kill the desktop
// or your other apps. ABSTRACT_UNIX_SOCKET (1 << 0) is intentionally left
// off: the session D-Bus bus is an abstract socket apps legitimately use.
const SCOPE_SIGNAL: u64 = 1 << 1;

pub const READ: u64 = READ_FILE | READ_DIR;
pub const READ_EXEC: u64 = READ | EXECUTE;
pub const READ_WRITE: u64 = READ | WRITE_FILE | REMOVE_DIR | REMOVE_FILE | MAKE_DIR | MAKE_REG | MAKE_SOCK
    | REFER | TRUNCATE;
/// Everything, for workers that legitimately change the whole system (a
/// package manager): still confined in network and the rest.
pub const ALL: u64 = READ_WRITE | EXECUTE | MAKE_CHAR | MAKE_FIFO | MAKE_BLOCK | MAKE_SYM;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

#[repr(C)]
struct NetPortAttr {
    allowed_access: u64,
    port: u64,
}

/// Landlock ABI version of the running kernel, or 0 if unavailable.
pub fn abi() -> i64 {
    // SAFETY: with a null attr and size 0 this only queries the version.
    let v = unsafe { libc::syscall(SYS_LANDLOCK_CREATE_RULESET, std::ptr::null::<u8>(), 0usize, LANDLOCK_CREATE_RULESET_VERSION) };
    if v < 0 { 0 } else { v }
}

pub struct Rules {
    pub fs: Vec<(String, u64)>,
    /// Outbound TCP ports allowed (all other TCP connects and binds denied).
    pub connect_ports: Vec<u16>,
    /// Leave TCP alone entirely (an app with the "network" permission).
    pub any_network: bool,
}

/// What this ABI can control (older kernels know fewer rights).
fn handled_fs(abi: i64) -> u64 {
    let mut all = EXECUTE | WRITE_FILE | READ_FILE | READ_DIR | REMOVE_DIR | REMOVE_FILE | MAKE_CHAR | MAKE_DIR
        | MAKE_REG | MAKE_SOCK | MAKE_FIFO | MAKE_BLOCK | MAKE_SYM;
    if abi >= 2 { all |= REFER; }
    if abi >= 3 { all |= TRUNCATE; }
    all
}

/// Landlock's network hook covers only TCP, so an app with no network at all
/// could still send UDP (DNS tunnelling / one-way exfil). This seccomp filter
/// refuses `socket(AF_INET|AF_INET6, SOCK_DGRAM|SOCK_RAW, …)` with EACCES,
/// closing that for apps that have no network grant. It leaves AF_UNIX
/// (Wayland/D-Bus/PipeWire) and TCP-socket creation alone, so it only ever
/// stops datagram/raw INET sockets. `no_new_privs` (set in `restrict` before
/// this runs) is required for an unprivileged seccomp filter.
#[cfg(target_arch = "x86_64")]
fn block_inet_datagram_sockets() -> io::Result<()> {
    use libc::{sock_filter, sock_fprog};
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    const NR_SOCKET: u32 = 41; // __NR_socket on x86_64
    const SECCOMP_MODE_FILTER: libc::c_ulong = 2;
    const RET_ALLOW: u32 = 0x7fff_0000;
    const RET_ERRNO_EACCES: u32 = 0x0005_0000 | (libc::EACCES as u32 & 0xffff);
    // Offsets in `struct seccomp_data` (little-endian low word of each u64 arg).
    const OFF_NR: u32 = 0;
    const OFF_ARCH: u32 = 4;
    const OFF_DOMAIN: u32 = 16; // args[0]
    const OFF_TYPE: u32 = 24; // args[1]
    let ld = |k: u32| sock_filter { code: 0x20, jt: 0, jf: 0, k }; // BPF_LD|W|ABS
    let jeq = |k: u32, jt: u8, jf: u8| sock_filter { code: 0x15, jt, jf, k }; // BPF_JMP|JEQ|K
    let and = |k: u32| sock_filter { code: 0x54, jt: 0, jf: 0, k }; // BPF_ALU|AND|K
    let ret = |k: u32| sock_filter { code: 0x06, jt: 0, jf: 0, k }; // BPF_RET|K
    // Jump targets: instruction 11 = ALLOW, 12 = DENY (offsets are relative).
    let prog = [
        /* 0 */ ld(OFF_ARCH),
        /* 1 */ jeq(AUDIT_ARCH_X86_64, 0, 9), // other arch -> ALLOW
        /* 2 */ ld(OFF_NR),
        /* 3 */ jeq(NR_SOCKET, 0, 7), // not socket() -> ALLOW
        /* 4 */ ld(OFF_DOMAIN),
        /* 5 */ jeq(libc::AF_INET as u32, 1, 0), // AF_INET -> check type (7)
        /* 6 */ jeq(libc::AF_INET6 as u32, 0, 4), // not AF_INET6 -> ALLOW
        /* 7 */ ld(OFF_TYPE),
        /* 8 */ and(0xff), // strip SOCK_NONBLOCK/SOCK_CLOEXEC
        /* 9 */ jeq(libc::SOCK_DGRAM as u32, 2, 0), // UDP -> DENY
        /* 10 */ jeq(libc::SOCK_RAW as u32, 1, 0), // RAW -> DENY, else ALLOW
        /* 11 */ ret(RET_ALLOW),
        /* 12 */ ret(RET_ERRNO_EACCES),
    ];
    let fprog = sock_fprog { len: prog.len() as u16, filter: prog.as_ptr() as *mut sock_filter };
    // SAFETY: no_new_privs is set; fprog points to a valid classic-BPF program.
    let rc = unsafe {
        libc::prctl(libc::PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &fprog as *const sock_fprog as libc::c_ulong, 0, 0)
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_arch = "x86_64"))]
fn block_inet_datagram_sockets() -> io::Result<()> {
    // The BPF program above is x86_64-specific; NexaOS ships x86_64 only.
    Ok(())
}

/// Apply the rules to this process and its future children. Returns the
/// ABI used; Ok(0) means the kernel has no Landlock (logged, not fatal).
pub fn restrict(rules: &Rules) -> io::Result<i64> {
    let abi = abi();
    if abi < 1 {
        return Ok(0);
    }
    let fs_mask = handled_fs(abi);
    let attr = RulesetAttr {
        handled_access_fs: fs_mask,
        handled_access_net: if abi >= 4 && !rules.any_network { NET_BIND_TCP | NET_CONNECT_TCP } else { 0 },
        // Signal scoping applies to every confined process, network or not.
        scoped: if abi >= 6 { SCOPE_SIGNAL } else { 0 },
    };
    // Tell the kernel only the fields its ABI knows: fs (8), +net (16), +scoped (24).
    let attr_size = if abi >= 6 {
        std::mem::size_of::<RulesetAttr>()
    } else if abi >= 4 {
        16
    } else {
        8
    };
    // SAFETY: attr is a valid landlock_ruleset_attr of attr_size bytes.
    let ruleset = unsafe { libc::syscall(SYS_LANDLOCK_CREATE_RULESET, &attr as *const RulesetAttr, attr_size, 0u32) };
    if ruleset < 0 {
        return Err(io::Error::last_os_error());
    }
    let ruleset = ruleset as i32;
    let result = (|| {
        for (path, access) in &rules.fs {
            if !Path::new(path).exists() {
                continue;
            }
            let c = CString::new(path.as_str()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            // SAFETY: c is a valid C string; O_PATH opens without reading.
            let fd = unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // Directories may carry every right; files only file rights.
            let is_dir = Path::new(path).is_dir();
            let mut allowed = access & fs_mask;
            if !is_dir {
                allowed &= EXECUTE | WRITE_FILE | READ_FILE | TRUNCATE;
            }
            let rule = PathBeneathAttr { allowed_access: allowed, parent_fd: fd };
            // SAFETY: rule is a valid landlock_path_beneath_attr.
            let rc = unsafe { libc::syscall(SYS_LANDLOCK_ADD_RULE, ruleset, RULE_PATH_BENEATH, &rule as *const PathBeneathAttr, 0u32) };
            // SAFETY: fd was opened above.
            unsafe { libc::close(fd) };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        if abi >= 4 && !rules.any_network {
            for port in &rules.connect_ports {
                let rule = NetPortAttr { allowed_access: NET_CONNECT_TCP, port: *port as u64 };
                // SAFETY: rule is a valid landlock_net_port_attr.
                let rc = unsafe { libc::syscall(SYS_LANDLOCK_ADD_RULE, ruleset, RULE_NET_PORT, &rule as *const NetPortAttr, 0u32) };
                if rc < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        // SAFETY: plain prctl; required before an unprivileged restrict_self.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // No network at all (no `network` grant AND no allowed TCP port): also
        // deny UDP/raw INET sockets, which Landlock can't. Services that need
        // DNS for their HTTPS (nexa-mindd, nexa-store: connect_ports set) are
        // excluded, so name resolution keeps working for them.
        if !rules.any_network && rules.connect_ports.is_empty() {
            block_inet_datagram_sockets()?;
        }
        // SAFETY: ruleset is a valid Landlock ruleset fd.
        if unsafe { libc::syscall(SYS_LANDLOCK_RESTRICT_SELF, ruleset, 0u32) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(abi)
    })();
    // SAFETY: ruleset fd is ours.
    unsafe { libc::close(ruleset) };
    result
}

/// The rules for nexa-mindd: system read-only, its own folders writable,
/// outbound HTTPS only (model downloads).
pub fn service_rules(state_dir: &str, run_dir: &str) -> Rules {
    let mut fs: Vec<(String, u64)> = ["/usr", "/lib", "/bin", "/sbin"].iter().map(|p| (p.to_string(), READ_EXEC)).collect();
    for p in ["/etc", "/proc", "/sys", "/dev/urandom", "/dev/random"] {
        fs.push((p.into(), READ));
    }
    fs.push(("/dev/null".into(), READ | WRITE_FILE));
    fs.push((state_dir.into(), READ_WRITE));
    fs.push((run_dir.into(), READ_WRITE));
    Rules { fs, connect_ports: vec![443], any_network: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs in a child process (restrictions are irreversible): only the
    /// allowed folder is writable afterwards.
    #[test]
    fn confines_a_child_process() {
        if abi() < 1 {
            eprintln!("kernel without Landlock: skipped");
            return;
        }
        let base = std::env::temp_dir().join(format!("nexa-ll-{}", std::process::id()));
        let allowed = base.join("allowed");
        let denied = base.join("denied");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&denied).unwrap();
        // SAFETY: fork in a test; the child only does simple syscalls then exits.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            let rules = Rules { fs: vec![(allowed.to_string_lossy().into(), READ_WRITE)], connect_ports: vec![], any_network: false };
            let ok = restrict(&rules).is_ok()
                && std::fs::write(allowed.join("f"), b"x").is_ok()
                && std::fs::write(denied.join("f"), b"x").is_err()
                && std::fs::read_to_string("/etc/hostname").is_err();
            // SAFETY: leave the forked child without running test harness code.
            unsafe { libc::_exit(if ok { 0 } else { 1 }) };
        }
        let mut status = 0;
        // SAFETY: waiting for our own child.
        unsafe { libc::waitpid(pid, &mut status, 0) };
        std::fs::remove_dir_all(&base).ok();
        assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "child could escape the sandbox");
    }

    /// The UDP/exfil fix: a process with no network at all can't make a UDP (or
    /// raw) INET socket, but AF_UNIX and TCP-socket creation still work; an app
    /// with network, or a port-scoped service, keeps UDP so DNS resolves.
    #[test]
    fn seccomp_blocks_udp_only_when_there_is_no_network() {
        if abi() < 1 {
            eprintln!("no Landlock: skipped");
            return;
        }
        // In a child (restrictions are irreversible): returns
        // (udp_inet_blocked, af_unix_ok, tcp_socket_ok) seen after restrict().
        fn after_restrict(any_network: bool, ports: Vec<u16>) -> (bool, bool, bool) {
            // SAFETY: fork in a test; the child only makes sockets then exits.
            let pid = unsafe { libc::fork() };
            if pid == 0 {
                let rules = Rules { fs: vec![("/".into(), READ_EXEC)], connect_ports: ports, any_network };
                let _ = restrict(&rules);
                let made = |d: i32, t: i32| {
                    // SAFETY: a plain socket() call; closed if it opened.
                    let fd = unsafe { libc::socket(d, t, 0) };
                    if fd >= 0 { unsafe { libc::close(fd) }; true } else { false }
                };
                let blocked = |d: i32, t: i32| {
                    // SAFETY: a plain socket() call; closed if it opened.
                    let fd = unsafe { libc::socket(d, t, 0) };
                    if fd >= 0 { unsafe { libc::close(fd) }; false }
                    else { io::Error::last_os_error().raw_os_error() == Some(libc::EACCES) }
                };
                let code = (blocked(libc::AF_INET, libc::SOCK_DGRAM) as i32)
                    | ((made(libc::AF_UNIX, libc::SOCK_STREAM) as i32) << 1)
                    | ((made(libc::AF_INET, libc::SOCK_STREAM) as i32) << 2);
                // SAFETY: leave the forked child without running harness code.
                unsafe { libc::_exit(code) };
            }
            let mut st = 0;
            // SAFETY: waiting for our own child.
            unsafe { libc::waitpid(pid, &mut st, 0) };
            let c = libc::WEXITSTATUS(st);
            ((c & 1) != 0, (c & 2) != 0, (c & 4) != 0)
        }

        let (udp_blocked, unix_ok, tcp_ok) = after_restrict(false, vec![]);
        assert!(udp_blocked, "a no-network app must not be able to open a UDP INET socket");
        assert!(unix_ok, "AF_UNIX (Wayland/D-Bus/PipeWire) must still work");
        assert!(tcp_ok, "creating a TCP socket stays fine (Landlock blocks the connect)");
        assert!(!after_restrict(true, vec![]).0, "a network app keeps UDP for DNS");
        assert!(!after_restrict(false, vec![443]).0, "a port-scoped service (mindd/store) keeps UDP for DNS");
    }
}
