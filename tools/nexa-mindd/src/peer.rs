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

/// The socket is mode 0660, group nexa-mind, so only members get here at all.
/// On top of that: people (uid >= 1000), root, and the service's own user;
/// other system accounts (daemons) are refused, so a compromised service
/// can't use the model.
pub fn allowed(peer: &Peer, own_uid: u32) -> bool {
    peer.uid == 0 || peer.uid == own_uid || (peer.uid >= 1000 && peer.uid < 60000)
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

    #[test]
    fn who_may_ask() {
        let me = 105;
        let peer = |uid| Peer { pid: 1, uid, gid: uid };
        assert!(allowed(&peer(1000), me) && allowed(&peer(0), me) && allowed(&peer(me), me));
        assert!(!allowed(&peer(81), me), "messagebus or another daemon");
        assert!(!allowed(&peer(65534), me), "nobody");
    }
}
