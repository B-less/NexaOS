//! Who may use Nexa Mind (the kernel-checked peer comes from nexa_sys::peer).

pub use nexa_sys::peer::{of, Peer};

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
    fn who_may_ask() {
        let me = 105;
        let peer = |uid| Peer { pid: 1, uid, gid: uid };
        assert!(allowed(&peer(1000), me) && allowed(&peer(0), me) && allowed(&peer(me), me));
        assert!(!allowed(&peer(81), me), "messagebus or another daemon");
        assert!(!allowed(&peer(65534), me), "nobody");
    }
}
