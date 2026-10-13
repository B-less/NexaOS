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
const MAKE_SYM: u64 = 1 << 12;
const REFER: u64 = 1 << 13; // ABI 2
const TRUNCATE: u64 = 1 << 14; // ABI 3
const NET_BIND_TCP: u64 = 1 << 0;
const NET_CONNECT_TCP: u64 = 1 << 1;

pub const READ: u64 = READ_FILE | READ_DIR;
pub const READ_EXEC: u64 = READ | EXECUTE;
pub const READ_WRITE: u64 = READ | WRITE_FILE | REMOVE_DIR | REMOVE_FILE | MAKE_DIR | MAKE_REG | MAKE_SOCK
    | REFER | TRUNCATE;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
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
    pub connect_ports: Vec<u16>,
}

/// What this ABI can control (older kernels know fewer rights).
fn handled_fs(abi: i64) -> u64 {
    let mut all = EXECUTE | WRITE_FILE | READ_FILE | READ_DIR | REMOVE_DIR | REMOVE_FILE | MAKE_CHAR | MAKE_DIR
        | MAKE_REG | MAKE_SOCK | MAKE_FIFO | MAKE_BLOCK | MAKE_SYM;
    if abi >= 2 { all |= REFER; }
    if abi >= 3 { all |= TRUNCATE; }
    all
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
        handled_access_net: if abi >= 4 { NET_BIND_TCP | NET_CONNECT_TCP } else { 0 },
    };
    let attr_size = if abi >= 4 { std::mem::size_of::<RulesetAttr>() } else { 8 };
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
        if abi >= 4 {
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
    Rules { fs, connect_ports: vec![443] }
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
            let rules = Rules { fs: vec![(allowed.to_string_lossy().into(), READ_WRITE)], connect_ports: vec![] };
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
}
