//! Who is on the other end of the Unix socket, straight from the kernel
//! (SO_PEERCRED): the uid can't be faked by the client.

use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peer {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

pub fn of(stream: &UnixStream) -> std::io::Result<Peer> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: getsockopt writes at most `len` bytes into `cred`, a ucred.
    let rc = unsafe {
        libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED,
                         &mut cred as *mut libc::ucred as *mut libc::c_void, &mut len)
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Peer { pid: cred.pid, uid: cred.uid, gid: cred.gid })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_peer_credentials() {
        let (a, _b) = UnixStream::pair().unwrap();
        let p = of(&a).unwrap();
        // SAFETY: getuid/getpid can't fail.
        assert_eq!((p.uid, p.pid), unsafe { (libc::getuid(), libc::getpid()) });
    }
}
