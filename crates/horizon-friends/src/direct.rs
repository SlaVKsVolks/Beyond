//! Direct connections between a host and a guest: the addresses each end
//! offers, and the race that dials all of the other's at once from the port
//! it also listens on (so two NATs that map a port the same way for every
//! destination let the two SYNs cross: TCP simultaneous open).
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::{
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
};

/// How long the ends try to reach each other directly.
pub const RACE: Duration = Duration::from_millis(3000);
const RETRY: Duration = Duration::from_millis(250);
const MAGIC: &[u8; 4] = b"HZF1";
/// A guest dialling an endpoint from an invite code (no relay involved).
pub const CODE_MAGIC: &[u8; 4] = b"HZF2";

fn socket_for(addr: &SocketAddr) -> io::Result<Socket> {
    let s = Socket::new(
        Domain::for_address(*addr),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    s.set_reuse_port(true)?;
    if addr.is_ipv6() {
        s.set_only_v6(true)?;
    }
    Ok(s)
}

fn unspecified(v6: bool, port: u16) -> SocketAddr {
    if v6 {
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port)
    } else {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)
    }
}

/// The direct socket's listeners: IPv4 on `port` (0 picks one), and IPv6 on
/// the same port when the system has it.
pub fn listen(port: u16) -> io::Result<(Vec<TcpListener>, u16)> {
    listen_with_scope(port, NetworkScope::AllInterfaces)
}

/// The network scope for direct connections and discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkScope {
    /// Accept on all local interfaces, as the normal host and guest do.
    AllInterfaces,
    /// Restrict listeners and connection candidates to this machine; do not discover external addresses.
    Loopback,
}

impl NetworkScope {
    pub(crate) fn allows(self, address: SocketAddr) -> bool {
        self == Self::AllInterfaces || address.ip().is_loopback()
    }

    pub(crate) fn resolve_relay(self, url: &str) -> io::Result<Vec<SocketAddr>> {
        if self == Self::AllInterfaces {
            return crate::resolve_relay(url);
        }
        let address = url
            .trim()
            .trim_start_matches("tcp://")
            .trim_end_matches('/');
        let address: SocketAddr = address.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "loopback relay must be a literal socket address",
            )
        })?;
        if !self.allows(address) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "relay is not loopback",
            ));
        }
        Ok(vec![address])
    }
}

/// Bind the direct listeners to the selected interfaces.
pub fn listen_with_scope(port: u16, mode: NetworkScope) -> io::Result<(Vec<TcpListener>, u16)> {
    let (ipv4, ipv6) = match mode {
        NetworkScope::AllInterfaces => (Ipv4Addr::UNSPECIFIED, Ipv6Addr::UNSPECIFIED),
        NetworkScope::Loopback => (Ipv4Addr::LOCALHOST, Ipv6Addr::LOCALHOST),
    };
    let bind = |addr: SocketAddr| -> io::Result<TcpListener> {
        let s = socket_for(&addr)?;
        s.bind(&SockAddr::from(addr))?;
        s.listen(16)?;
        let l: TcpListener = s.into();
        l.set_nonblocking(true)?;
        Ok(l)
    };
    let v4 = bind(SocketAddr::new(IpAddr::V4(ipv4), port))?;
    let port = v4.local_addr()?.port();
    let mut out = vec![v4];
    if let Ok(v6) = bind(SocketAddr::new(IpAddr::V6(ipv6), port)) {
        out.push(v6);
    }
    if mode == NetworkScope::Loopback {
        for listener in &out {
            if !listener.local_addr()?.ip().is_loopback() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "loopback listener bound outside loopback",
                ));
            }
        }
    }
    Ok((out, port))
}

/// Connect to `to` from local `port` (shared with the listener), or from any
/// port when that one can't be used (the same pair already connected).
pub fn connect_from(port: Option<u16>, to: SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
    if let Some(port) = port {
        let s = socket_for(&to)?;
        if s.bind(&SockAddr::from(unspecified(to.is_ipv6(), port)))
            .is_ok()
        {
            match s.connect_timeout(&SockAddr::from(to), timeout) {
                Ok(()) => return Ok(s.into()),
                Err(e)
                    if e.kind() != io::ErrorKind::AddrInUse
                        && e.kind() != io::ErrorKind::AddrNotAvailable =>
                {
                    return Err(e);
                }
                Err(_) => {}
            }
        }
    }
    TcpStream::connect_timeout(&to, timeout)
}

/// This end's candidates for a relayed session: its interface addresses
/// with the direct port (loopback when the relay is local), then the address
/// the relay saw (when it is not one of them).
pub fn candidates(
    port: u16,
    relay: Option<SocketAddr>,
    observed: Option<SocketAddr>,
) -> Vec<SocketAddr> {
    let loopback = relay.is_some_and(|r| r.ip().is_loopback());
    let mut out: Vec<SocketAddr> = crate::reach::interface_addresses(loopback)
        .into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect();
    if let Some(o) = observed
        && !o.ip().is_unspecified()
        && !out.contains(&o)
    {
        out.push(o);
    }
    out.truncate(crate::invite::MAX_ENDPOINTS);
    out
}

pub(crate) fn scoped_candidates(
    scope: NetworkScope,
    port: u16,
    relay: Option<SocketAddr>,
    observed: Option<SocketAddr>,
) -> Vec<SocketAddr> {
    candidates_with_scope(scope, port, || candidates(port, relay, observed))
}

fn candidates_with_scope(
    scope: NetworkScope,
    port: u16,
    discover: impl FnOnce() -> Vec<SocketAddr>,
) -> Vec<SocketAddr> {
    match scope {
        NetworkScope::AllInterfaces => discover(),
        NetworkScope::Loopback => vec![SocketAddr::from((Ipv4Addr::LOCALHOST, port))],
    }
}

/// Both ends open every direct stream the same way, whoever dialled: the
/// magic and the session, then the other's must match.
pub fn introduce(stream: &mut TcpStream, session: u64) -> io::Result<()> {
    let mut hello = [0u8; 12];
    hello[..4].copy_from_slice(MAGIC);
    hello[4..].copy_from_slice(&session.to_be_bytes());
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(&hello)?;
    let mut theirs = [0u8; 12];
    stream.read_exact(&mut theirs)?;
    if theirs != hello {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not this session",
        ));
    }
    Ok(())
}

/// The session an inbound direct stream belongs to (the host's listener
/// serves every session): reads the other end's introduction.
pub fn read_session(stream: &mut TcpStream) -> io::Result<u64> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut theirs = [0u8; 12];
    stream.read_exact(&mut theirs)?;
    if &theirs[..4] != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a direct stream",
        ));
    }
    Ok(u64::from_be_bytes(theirs[4..].try_into().unwrap()))
}
/// What an inbound stream on the host's direct port is.
pub enum Hello {
    /// A relayed session's direct race.
    Relayed(u64),
    /// A guest with the invite code.
    Code(u64),
}
pub fn read_hello(stream: &mut TcpStream) -> io::Result<Hello> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    read_hello_from(stream)
}
pub(crate) fn read_hello_from(stream: &mut impl Read) -> io::Result<Hello> {
    let mut theirs = [0u8; 12];
    stream.read_exact(&mut theirs)?;
    let session = u64::from_be_bytes(theirs[4..].try_into().unwrap());
    match &theirs[..4] {
        m if m == MAGIC => Ok(Hello::Relayed(session)),
        m if m == CODE_MAGIC => Ok(Hello::Code(session)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Horizon stream",
        )),
    }
}
/// The guest's side of a code session's opening: send the hello, wait for
/// its echo (proof a Horizon host listens there).
pub fn code_hello(stream: &mut TcpStream, session: u64) -> io::Result<()> {
    let mut hello = [0u8; 12];
    hello[..4].copy_from_slice(CODE_MAGIC);
    hello[4..].copy_from_slice(&session.to_be_bytes());
    stream.set_read_timeout(Some(Duration::from_secs(4)))?;
    stream.write_all(&hello)?;
    let mut theirs = [0u8; 12];
    stream.read_exact(&mut theirs)?;
    if theirs != hello {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a Horizon host",
        ));
    }
    Ok(())
}
/// The host echoes a code session's hello.
pub fn answer_code(stream: &mut impl Write, session: u64) -> io::Result<()> {
    let mut hello = [0u8; 12];
    hello[..4].copy_from_slice(CODE_MAGIC);
    hello[4..].copy_from_slice(&session.to_be_bytes());
    stream.write_all(&hello)
}
/// Answer an inbound stream's introduction for `session`.
pub fn answer(stream: &mut impl Write, session: u64) -> io::Result<()> {
    let mut hello = [0u8; 12];
    hello[..4].copy_from_slice(MAGIC);
    hello[4..].copy_from_slice(&session.to_be_bytes());
    stream.write_all(&hello)
}

/// Dial each target from `port` until one connects, the deadline passes or
/// `stop` is set; streams that connect are introduced and sent on.
pub fn dial(
    port: Option<u16>,
    targets: Vec<SocketAddr>,
    session: u64,
    deadline: Instant,
    stop: Arc<AtomicBool>,
    found: Sender<TcpStream>,
) {
    for target in targets {
        let (stop, found) = (stop.clone(), found.clone());
        let _ = std::thread::Builder::new()
            .name("horizon-friends-dial".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    if now >= deadline {
                        return;
                    }
                    let tried = Instant::now();
                    let wait = (deadline - now).min(Duration::from_millis(600));
                    if let Ok(mut s) = connect_from(port, target, wait)
                        && introduce(&mut s, session).is_ok()
                    {
                        let _ = found.send(s);
                        return;
                    }
                    let spent = tried.elapsed();
                    if spent < RETRY {
                        std::thread::sleep(RETRY - spent);
                    }
                }
            });
    }
}

/// Accept on the listeners until the deadline or `stop`; inbound streams for
/// `session` are answered and sent on.
pub fn accept(
    listeners: Vec<TcpListener>,
    session: u64,
    deadline: Instant,
    stop: Arc<AtomicBool>,
    found: Sender<TcpStream>,
) {
    let _ = std::thread::Builder::new()
        .name("horizon-friends-accept".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                let mut idle = true;
                for l in &listeners {
                    if let Ok((mut s, _)) = l.accept() {
                        idle = false;
                        let _ = s.set_nonblocking(false);
                        if read_session(&mut s).ok() == Some(session)
                            && answer(&mut s, session).is_ok()
                        {
                            let _ = found.send(s);
                        }
                    }
                }
                if idle {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn loopback_scope_rejects_remote_and_hostname_relays() {
        let scope = NetworkScope::Loopback;
        assert!(scope.allows(SocketAddr::from(([127, 0, 0, 1], 42))));
        assert!(!scope.allows(SocketAddr::from(([192, 0, 2, 1], 42))));
        assert_eq!(
            scope.resolve_relay("tcp://127.0.0.1:42/").unwrap(),
            vec![SocketAddr::from(([127, 0, 0, 1], 42))]
        );
        assert!(scope.resolve_relay("192.0.2.1:42").is_err());
        assert!(scope.resolve_relay("localhost:42").is_err());
        assert!(NetworkScope::AllInterfaces.allows(SocketAddr::from(([192, 0, 2, 1], 42))));
    }

    #[test]
    fn loopback_candidates_do_not_enumerate_other_interfaces() {
        let found = candidates_with_scope(NetworkScope::Loopback, 1234, || {
            panic!("loopback candidates must not enumerate interfaces")
        });
        assert_eq!(found, vec![SocketAddr::from(([127, 0, 0, 1], 1234))]);
    }

    #[test]
    fn repeated_connections_to_same_relay_fall_back_to_a_free_port() {
        let (_source, source_port) = listen_loopback().unwrap();
        let (_relay, relay_port) = listen_loopback().unwrap();
        let relay = SocketAddr::from(([127, 0, 0, 1], relay_port));
        let first = connect_from(Some(source_port), relay, Duration::from_secs(1)).unwrap();
        let second = connect_from(Some(source_port), relay, Duration::from_secs(1)).unwrap();
        assert_ne!(
            first.local_addr().unwrap().port(),
            second.local_addr().unwrap().port()
        );
    }

    fn listen_loopback() -> io::Result<(Vec<TcpListener>, u16)> {
        let result = listen_with_scope(0, NetworkScope::Loopback)?;
        for listener in &result.0 {
            assert!(listener.local_addr()?.ip().is_loopback());
        }
        Ok(result)
    }

    #[test]
    fn production_listeners_bind_unspecified_addresses() {
        let (listeners, port) = listen(0).unwrap();
        assert!(!listeners.is_empty());
        for listener in listeners {
            let addr = listener.local_addr().unwrap();
            assert!(addr.ip().is_unspecified());
            assert_eq!(addr.port(), port);
        }
    }

    #[test]
    fn candidates_have_the_port_and_the_observed_address() {
        let relay = SocketAddr::from(([127, 0, 0, 1], 1));
        let observed = SocketAddr::from(([203, 0, 113, 5], 40000));
        let c = candidates(5555, Some(relay), Some(observed));
        assert!(c.contains(&SocketAddr::from(([127, 0, 0, 1], 5555))));
        assert_eq!(c.last(), Some(&observed));
        let far = candidates(5555, None, None);
        assert!(
            far.iter()
                .all(|a| !a.ip().is_loopback() && a.port() == 5555)
        );
    }

    /// Two direct sockets on loopback: each listens on its port and dials
    /// the other's from it; the session's streams meet and wrong sessions
    /// don't.
    #[test]
    fn the_race_connects_and_checks_the_session() {
        let (la, pa) = listen_loopback().unwrap();
        let (lb, pb) = listen_loopback().unwrap();
        for listener in la.iter().chain(&lb) {
            assert!(listener.local_addr().unwrap().ip().is_loopback());
        }
        assert_ne!(pa, pb);
        let stop = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now() + Duration::from_secs(3);
        let (ta, ra) = channel();
        let (tb, rb) = channel();
        accept(la, 42, deadline, stop.clone(), ta.clone());
        accept(lb, 42, deadline, stop.clone(), tb.clone());
        dial(
            Some(pa),
            vec![SocketAddr::from(([127, 0, 0, 1], pb))],
            42,
            deadline,
            stop.clone(),
            ta,
        );
        let mut a = ra.recv_timeout(Duration::from_secs(3)).unwrap();
        let mut b = rb.recv_timeout(Duration::from_secs(3)).unwrap();
        a.write_all(b"ping").unwrap();
        let mut got = [0u8; 4];
        b.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"ping");
        stop.store(true, Ordering::Relaxed);

        let (lc, pc) = listen_loopback().unwrap();
        let (tc, rc) = channel();
        let deadline = Instant::now() + Duration::from_millis(800);
        let stop = Arc::new(AtomicBool::new(false));
        accept(lc, 7, deadline, stop.clone(), tc.clone());
        let (td, rd) = channel();
        dial(
            None,
            vec![SocketAddr::from(([127, 0, 0, 1], pc))],
            8,
            deadline,
            stop,
            td,
        );
        assert!(rc.recv_timeout(Duration::from_secs(2)).is_err());
        assert!(rd.recv_timeout(Duration::from_millis(100)).is_err());
    }
}
