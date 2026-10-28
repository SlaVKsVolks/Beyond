//! Play with friends: a host shares the world it opened to LAN behind an
//! invite code that holds its addresses and a secret; guests dial it
//! directly (the host opens its router port with PCP / NAT-PMP / UPnP and
//! learns its public address with STUN). A relay anyone can run
//! (`horizon-relay`) adds short codes and a fallback for networks that
//! can't be reached. Both ends carry the game's TCP stream
//! unchanged between loopback ports (the integrated server on the host, a
//! local port the guest's client connects to), end-to-end encrypted with a
//! key only the holders of the code can derive. The design, the protocol and
//! what the relay sees are in docs/play-with-friends.md.
#![forbid(unsafe_code)]

pub mod code;
pub mod crypto;
mod deadline;
pub mod direct;
pub mod guest;
pub mod host;
pub mod invite;
pub mod portmap;
pub mod reach;
mod session;
pub mod stun;
pub mod wire;

pub use code::Code;
pub use deadline::{Deadline, deadline};
pub use guest::{JoinError, Tunnel};
pub use host::{Host, HostConfig, HostEvent, HostState};

use std::{
    io,
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    thread::JoinHandle,
};

/// The game's server address for a friend's world: `friend:K7QM-3XPD`
/// (`horizon-client --server friend:…`, what the launcher's Join a friend
/// starts).
pub const ADDRESS_PREFIX: &str = "friend:";

/// Fill `buf` from the OS random generator.
pub(crate) fn fill_random(buf: &mut [u8]) -> io::Result<()> {
    getrandom::fill(buf).map_err(|e| io::Error::other(e.to_string()))
}

/// Which paths a session may take (`HORIZON_FRIENDS_PATH`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathMode {
    /// Direct when the ends reach each other, else the relay.
    Auto,
    /// Never connect directly: the other end doesn't learn this one's address.
    RelayOnly,
    /// Never fall back to the relay (tests).
    DirectOnly,
}
impl PathMode {
    pub fn from_env() -> Self {
        match std::env::var("HORIZON_FRIENDS_PATH").as_deref() {
            Ok("relay") => Self::RelayOnly,
            Ok("direct") => Self::DirectOnly,
            _ => Self::Auto,
        }
    }
}

/// A relay from `HORIZON_RELAY_URL` (tests, or players who set it in the
/// environment); the game adds its own setting. None by default: friends
/// join from the code alone.
pub fn relay_url() -> Option<String> {
    std::env::var("HORIZON_RELAY_URL")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// `host:port`, optionally written `tcp://host:port`.
pub fn resolve_relay(url: &str) -> io::Result<Vec<SocketAddr>> {
    let address = url
        .trim()
        .trim_start_matches("tcp://")
        .trim_end_matches('/');
    let found: Vec<SocketAddr> = address.to_socket_addrs()?.collect();
    if found.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the relay's name didn't resolve",
        ));
    }
    Ok(found)
}

/// Carry one connection both ways between a plain stream (the game's) and a
/// protected one, until either side ends; then both are closed.
pub fn pump(
    plain: TcpStream,
    mut reader: crypto::SecureReader<TcpStream>,
    mut writer: crypto::SecureWriter<TcpStream>,
) -> io::Result<JoinHandle<()>> {
    use io::{Read, Write};
    for s in [&plain, reader.get_ref(), writer.get_ref()] {
        s.set_read_timeout(None)?;
        let _ = s.set_nodelay(true);
    }
    let mut plain_in = plain.try_clone()?;
    let mut plain_out = plain;
    let outbound = std::thread::Builder::new()
        .name("horizon-friends-out".into())
        .spawn(move || {
            let mut buf = vec![0u8; crypto::MAX_PAYLOAD];
            loop {
                match plain_in.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if writer.write_data(&buf[..n]).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = writer.get_ref().shutdown(std::net::Shutdown::Both);
            let _ = plain_in.shutdown(std::net::Shutdown::Both);
        })?;
    std::thread::Builder::new()
        .name("horizon-friends-in".into())
        .spawn(move || {
            while let Ok(Some(frame)) = reader.read_frame() {
                if let crypto::Frame::Data(d) = frame
                    && plain_out.write_all(&d).is_err()
                {
                    break;
                }
            }
            let _ = plain_out.shutdown(std::net::Shutdown::Both);
            let _ = reader.get_ref().shutdown(std::net::Shutdown::Both);
            let _ = outbound.join();
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        net::TcpListener,
        time::{Duration, Instant},
    };

    #[test]
    fn a_deadline_ends_the_stream_unless_dropped() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let _kept = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (kept, _) = l.accept().unwrap();
        let _ended = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (ended, _) = l.accept().unwrap();
        let mut kept = deadline(kept, Duration::from_millis(100))
            .unwrap()
            .cancel()
            .unwrap();
        let mut ended = deadline(ended, Duration::from_millis(100)).unwrap();
        let started = Instant::now();
        assert_eq!(ended.read(&mut [0u8; 1]).unwrap(), 0);
        assert!(started.elapsed() < Duration::from_secs(2));
        kept.set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let e = kept.read(&mut [0u8; 1]).unwrap_err();
        assert!(matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
    }

    #[test]
    fn deadline_cancel_restores_prior_socket_timeouts() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        use std::io::Write;
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let read = Some(Duration::from_millis(500));
        let write = Some(Duration::from_millis(700));
        stream.set_read_timeout(read).unwrap();
        stream.set_write_timeout(write).unwrap();
        peer.write_all(&[3]).unwrap();
        let mut bounded = deadline(stream, Duration::from_millis(250)).unwrap();
        let mut byte = [0];
        bounded.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [3]);
        bounded.write_all(&[4]).unwrap();
        peer.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [4]);
        let stream = bounded.cancel().unwrap();
        assert_eq!(stream.read_timeout().unwrap(), read);
        assert_eq!(stream.write_timeout().unwrap(), write);
    }

    #[test]
    fn shorter_socket_timeout_is_preserved_and_terminal() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut bounded = deadline(stream, Duration::from_secs(2)).unwrap();
        let error = bounded.read(&mut [0; 1]).unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));
        assert!(bounded.cancel().is_err());
    }

    #[test]
    fn deadline_rejects_expired_release_and_overflow() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let bounded = deadline(stream, Duration::ZERO).unwrap();
        assert_eq!(
            bounded.cancel().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        assert!(
            matches!(deadline(stream, Duration::MAX), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn deadline_is_absolute_despite_trickled_bytes() {
        use std::io::Write;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let worker = std::thread::spawn(move || {
            for _ in 0..12 {
                if peer.write_all(&[1]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        });
        let mut bounded = deadline(stream, Duration::from_millis(120)).unwrap();
        let started = Instant::now();
        let mut count = 0;
        while bounded.read(&mut [0; 1]).unwrap() != 0 {
            count += 1;
        }
        assert!(count > 0);
        assert!(count < 12);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(bounded.cancel().is_err());
        worker.join().unwrap();
    }

    #[test]
    fn deadline_preserves_peer_eof_and_rejects_late_write() {
        use std::io::Write;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut bounded = deadline(stream, Duration::from_secs(2)).unwrap();
        drop(peer);
        assert_eq!(bounded.read(&mut [0; 1]).unwrap(), 0);
        assert!(bounded.cancel().is_ok());

        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut bounded = deadline(stream, Duration::ZERO).unwrap();
        assert_eq!(
            bounded.write(&[1]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn deadline_bounds_a_stalled_writer() {
        use std::io::Write;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut bounded = deadline(stream, Duration::from_millis(150)).unwrap();
        let started = Instant::now();
        let payload = [7u8; 65536];
        let error = loop {
            match bounded.write(&payload) {
                Ok(n) if n > 0 => {}
                Ok(_) => panic!("nonempty write made no progress"),
                Err(error) => break error,
            }
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(bounded.cancel().is_err());
    }
}
