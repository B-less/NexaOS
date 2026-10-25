//! The encrypted channel between two Nexa devices: TCP carrying Noise
//! (XX: each side proves its long-term key; forward secrecy, so a key
//! stolen later can't open old traffic). For pairing, the one-time secret
//! from the code is mixed in (XXpsk3): without it the handshake fails.
//!
//! The other side's key is checked the moment it's known, before any
//! message from it is read: a device that isn't allowed gets nothing.

use crate::proto::{self, MSG_DATA, MSG_JSON};
use serde_json::Value;
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub struct Channel {
    stream: TcpStream,
    noise: snow::TransportState,
    pub remote_public: Vec<u8>,
    pub handshake_hash: Vec<u8>,
}

fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

fn denied() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "not a device this one is paired with")
}

fn builder<'a>(psk: Option<&'a [u8; 32]>, secret: &'a [u8]) -> io::Result<snow::Builder<'a>> {
    let params = (if psk.is_some() { proto::NOISE_PAIR } else { proto::NOISE }).parse().map_err(err)?;
    let mut b = snow::Builder::new(params).local_private_key(secret);
    if let Some(k) = psk {
        b = b.psk(3, k);
    }
    Ok(b)
}

/// Connect and shake hands. `allowed` sees the other side's public key
/// before this side proves itself or sends anything.
pub fn connect(addr: SocketAddr, secret: &[u8], psk: Option<&[u8; 32]>, allowed: impl Fn(&[u8]) -> bool) -> io::Result<Channel> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_nodelay(true).ok();
    let mut hs = builder(psk, secret)?.build_initiator().map_err(err)?;
    let mut buf = vec![0u8; 65535];
    let mut payload = vec![0u8; 65535];
    let n = hs.write_message(&[], &mut buf).map_err(err)?;
    proto::write_frame(&mut stream, &buf[..n])?;
    let msg = proto::read_frame(&mut stream)?;
    hs.read_message(&msg, &mut payload).map_err(err)?;
    let remote = hs.get_remote_static().ok_or_else(|| err("no remote key"))?.to_vec();
    if !allowed(&remote) {
        return Err(denied());
    }
    let n = hs.write_message(&[], &mut buf).map_err(err)?;
    proto::write_frame(&mut stream, &buf[..n])?;
    let handshake_hash = hs.get_handshake_hash().to_vec();
    let noise = hs.into_transport_mode().map_err(err)?;
    Ok(Channel { stream, noise, remote_public: remote, handshake_hash })
}

/// Shake hands on an accepted connection. A device that `allowed` refuses
/// is dropped right after the handshake's last message, unread.
pub fn accept(mut stream: TcpStream, secret: &[u8], psk: Option<&[u8; 32]>, allowed: impl Fn(&[u8]) -> bool) -> io::Result<Channel> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_nodelay(true).ok();
    let mut hs = builder(psk, secret)?.build_responder().map_err(err)?;
    let mut buf = vec![0u8; 65535];
    let mut payload = vec![0u8; 65535];
    let msg = proto::read_frame(&mut stream)?;
    hs.read_message(&msg, &mut payload).map_err(err)?;
    let n = hs.write_message(&[], &mut buf).map_err(err)?;
    proto::write_frame(&mut stream, &buf[..n])?;
    let msg = proto::read_frame(&mut stream)?;
    hs.read_message(&msg, &mut payload).map_err(err)?;
    let remote = hs.get_remote_static().ok_or_else(|| err("no remote key"))?.to_vec();
    if !allowed(&remote) {
        stream.shutdown(std::net::Shutdown::Both).ok();
        return Err(denied());
    }
    let handshake_hash = hs.get_handshake_hash().to_vec();
    let noise = hs.into_transport_mode().map_err(err)?;
    Ok(Channel { stream, noise, remote_public: remote, handshake_hash })
}

impl Channel {
    pub fn set_timeout(&self, secs: u64) {
        self.stream.set_read_timeout(Some(Duration::from_secs(secs))).ok();
    }

    fn send(&mut self, kind: u8, data: &[u8]) -> io::Result<()> {
        let mut plain = Vec::with_capacity(data.len() + 1);
        plain.push(kind);
        plain.extend_from_slice(data);
        let mut out = vec![0u8; plain.len() + 16];
        let n = self.noise.write_message(&plain, &mut out).map_err(err)?;
        proto::write_frame(&mut self.stream, &out[..n])
    }

    pub fn send_json(&mut self, v: &Value) -> io::Result<()> {
        self.send(MSG_JSON, v.to_string().as_bytes())
    }

    pub fn send_data(&mut self, data: &[u8]) -> io::Result<()> {
        self.send(MSG_DATA, data)
    }

    pub fn recv(&mut self) -> io::Result<(u8, Vec<u8>)> {
        let frame = proto::read_frame(&mut self.stream)?;
        let mut out = vec![0u8; frame.len()];
        let n = self.noise.read_message(&frame, &mut out).map_err(err)?;
        out.truncate(n);
        if out.is_empty() {
            return Err(err("empty message"));
        }
        let kind = out.remove(0);
        Ok((kind, out))
    }

    pub fn recv_json(&mut self) -> io::Result<Value> {
        match self.recv()? {
            (MSG_JSON, body) => serde_json::from_slice(&body).map_err(err),
            _ => Err(err("expected a message, got data")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn keypair() -> snow::Keypair {
        snow::Builder::new(proto::NOISE.parse().unwrap()).generate_keypair().unwrap()
    }

    #[test]
    fn two_devices_talk_and_a_stranger_is_dropped() {
        let pc = keypair();
        let phone = keypair();
        let stranger = keypair();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let phone_pub = phone.public.clone();
        let pc_secret = pc.private.clone();
        let server = std::thread::spawn(move || {
            // First the paired phone, then the stranger.
            let (s, _) = listener.accept().unwrap();
            let mut ch = accept(s, &pc_secret, None, |k| k == phone_pub.as_slice()).unwrap();
            let hello = ch.recv_json().unwrap();
            ch.send_json(&serde_json::json!({"t": "pong", "got": hello["t"]})).unwrap();
            let (s, _) = listener.accept().unwrap();
            accept(s, &pc_secret, None, |k| k == phone_pub.as_slice()).err().map(|e| e.kind())
        });
        let pc_pub = pc.public.clone();
        let mut ch = connect(addr, &phone.private, None, |k| k == pc_pub.as_slice()).unwrap();
        ch.send_json(&serde_json::json!({"t": "ping"})).unwrap();
        assert_eq!(ch.recv_json().unwrap()["got"], "ping");
        // The stranger completes the handshake, but nothing it sends is read.
        let _ = connect(addr, &stranger.private, None, |_| true);
        assert_eq!(server.join().unwrap(), Some(io::ErrorKind::PermissionDenied));
    }

    #[test]
    fn pairing_needs_the_code_secret() {
        let a = keypair();
        let b = keypair();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let right = [5u8; 32];
        let a_secret = a.private.clone();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            accept(s, &a_secret, Some(&right), |_| true).map(|c| c.handshake_hash)
        });
        let wrong = [6u8; 32];
        // The secret is checked in the last handshake message, so it's the
        // device showing the code that refuses; nothing gets through.
        let client = connect(addr, &b.private, Some(&wrong), |_| true);
        assert!(server.join().unwrap().is_err(), "a wrong code can't pair");
        if let Ok(mut ch) = client {
            assert!(ch.recv_json().is_err());
        }
    }
}
