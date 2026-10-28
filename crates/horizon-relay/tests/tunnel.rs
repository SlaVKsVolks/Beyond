//! Hosts in front of an echo server standing in for the integrated server,
//! guests, and (for the relay cases) a self-hosted relay, all on loopback
//! in one process. Port mapping and STUN are off here: they have their own
//! tests against mock routers and servers in horizon-friends.
#![forbid(unsafe_code)]

use horizon_friends::{
    Code, Host, HostConfig, HostEvent, JoinError, PathMode,
    guest::{self, GuestConfig},
    invite::{self, Invite, Parsed},
    reach,
};
use horizon_relay::{Config, Relay, limits::Rates};
use std::{
    cell::RefCell,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    time::{Duration, Instant},
};

const PROTOCOL: i32 = 777;

/// Echoes every connection back until it closes.
fn echo_server() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            std::thread::spawn(move || {
                let mut out = s.try_clone().unwrap();
                let _ = std::io::copy(&mut s, &mut out);
            });
        }
    });
    a
}

fn relay(config: Config) -> (Relay, String) {
    let (relay, address, _) = Relay::spawn_local(config).unwrap();
    (relay, address.to_string())
}

fn generous() -> Config {
    Config {
        rates: Rates {
            join_per_min: 600.0,
            join_burst: 100.0,
            ..Rates::default()
        },
        ..Config::default()
    }
}

fn start(relay: Option<&str>, path: PathMode) -> Host {
    Host::start(HostConfig {
        lan: echo_server(),
        protocol: PROTOCOL,
        game: "26.3".into(),
        relay: relay.map(str::to_owned),
        path,
        port: 0,
        reach: reach::Options {
            map_port: false,
            stun: Vec::new(),
            loopback: true,
        },
    })
    .unwrap()
}

thread_local! {
    /// Events polled but not yet waited for.
    static BACKLOG: RefCell<Vec<HostEvent>> = const { RefCell::new(Vec::new()) };
}

/// The first event (in order) that `f` picks; the others stay queued.
fn wait_event<T>(host: &Host, mut f: impl FnMut(&HostEvent) -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let found = BACKLOG.with_borrow_mut(|backlog| {
            backlog.extend(host.poll());
            let i = backlog.iter().position(|e| f(e).is_some())?;
            f(&backlog.remove(i))
        });
        if let Some(t) = found {
            return t;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no such host event");
}

/// The invite once ready (and, with a relay, once it holds the short code).
fn invite_of(host: &Host, with_relay: bool) -> (String, Option<Code>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = host.state();
        if let Some(code) = s.invite
            && (!with_relay || s.short.is_some())
        {
            return (code, s.short);
        }
        assert!(Instant::now() < deadline, "no code");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn join(code: &str, relay: Option<&str>, path: PathMode) -> Result<guest::Tunnel, JoinError> {
    guest::join(&GuestConfig {
        code: code.to_owned(),
        relay: relay.map(str::to_owned),
        protocol: PROTOCOL,
        game: "26.3".into(),
        path,
    })
}

/// What the game's client does: connect to the loopback port, send, read
/// the echo back.
fn round_trip(tunnel: guest::Tunnel, payload: &[u8]) {
    let (port, done) = tunnel.serve_loopback().unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut w = s.try_clone().unwrap();
    let send = payload.to_vec();
    let writer = std::thread::spawn(move || w.write_all(&send).unwrap());
    let mut got = vec![0u8; payload.len()];
    s.read_exact(&mut got).unwrap();
    writer.join().unwrap();
    assert!(got == payload, "echo differs");
    drop(s);
    done.join().unwrap();
}

fn payload(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 + i / 251) as u8).collect()
}

fn long(code: &str) -> Invite {
    match invite::parse(code) {
        Some(Parsed::Long(i)) => i,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_code_alone_connects_without_any_server() {
    let host = start(None, PathMode::Auto);
    let (code, short) = invite_of(&host, false);
    assert!(short.is_none());
    let invite = long(&code);
    assert!(invite.relay.is_none());
    assert!(
        invite.endpoints.iter().any(|e| e.ip().is_loopback()),
        "{invite:?}"
    );
    let tunnel = join(&code, None, PathMode::Auto).unwrap();
    assert!(tunnel.direct);
    round_trip(tunnel, &payload(1 << 20));
    assert!(wait_event(&host, |e| match e {
        HostEvent::Connected { direct, .. } => Some(*direct),
        _ => None,
    }));
    wait_event(&host, |e| {
        matches!(e, HostEvent::Disconnected { .. }).then_some(())
    });
    assert_eq!(host.sessions(), 0);
    // Two guests at once.
    let a = join(&code, None, PathMode::Auto).unwrap();
    let b = join(&code, None, PathMode::Auto).unwrap();
    let ta = std::thread::spawn(move || round_trip(a, &payload(50_000)));
    round_trip(b, &payload(70_000));
    ta.join().unwrap();
    // Without a relay, a relay-only guest has nowhere to go.
    assert!(matches!(
        join(&code, None, PathMode::RelayOnly),
        Err(JoinError::NotReachable)
    ));
}

#[test]
fn a_short_code_through_a_self_hosted_relay() {
    let (relay, address) = relay(generous());
    let host = start(Some(&address), PathMode::Auto);
    let (code, short) = invite_of(&host, true);
    let short = short.unwrap().to_string();
    // The full code names the relay too.
    assert_eq!(long(&code).relay.unwrap().address, address);
    // A short code needs the relay's address.
    assert!(matches!(
        join(&short, None, PathMode::Auto),
        Err(JoinError::NeedsRelay)
    ));
    let tunnel = join(&short, Some(&address), PathMode::RelayOnly).unwrap();
    assert!(!tunnel.direct);
    let data = payload(1 << 20);
    round_trip(tunnel, &data);
    let relayed = relay.counts().relayed_bytes;
    assert!(relayed > 2 * data.len() as u64, "{relayed}");
    // The relay's own direct race (both ends on loopback): direct.
    let tunnel = join(&short, Some(&address), PathMode::Auto).unwrap();
    assert!(tunnel.direct);
    round_trip(tunnel, b"raced");
}

#[test]
fn unreachable_endpoints_fall_back_to_the_relay_in_the_code() {
    let (relay, address) = relay(generous());
    let host = start(Some(&address), PathMode::Auto);
    let (code, _) = invite_of(&host, true);
    // The same invite with an endpoint nobody answers on.
    let mut invite = long(&code);
    let dead = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    invite.endpoints = vec![dead];
    let started = Instant::now();
    assert!(matches!(
        join(&invite.encode(), None, PathMode::DirectOnly),
        Err(JoinError::NotReachable)
    ));
    assert!(started.elapsed() < Duration::from_secs(6));
    let tunnel = join(&invite.encode(), None, PathMode::Auto).unwrap();
    round_trip(tunnel, b"fallback");
    assert!(relay.counts().relayed_bytes > 0);
    // Without a relay in the code: the host's advice is the way out.
    invite.relay = None;
    assert!(matches!(
        join(&invite.encode(), None, PathMode::Auto),
        Err(JoinError::NotReachable)
    ));
}

#[test]
fn wrong_secrets_fail() {
    let (_relay, address) = relay(generous());
    let host = start(Some(&address), PathMode::Auto);
    let (code, short) = invite_of(&host, true);
    // A full code with another secret (re-encoded, so the checksum passes).
    let mut forged = long(&code);
    forged.secret[0] ^= 1;
    assert!(matches!(
        join(&forged.encode(), None, PathMode::DirectOnly),
        Err(JoinError::WrongCode)
    ));
    assert_eq!(
        wait_event(&host, |e| match e {
            HostEvent::WrongCode { count } => Some(*count),
            _ => None,
        }),
        1
    );
    // Short codes: five wrong secrets retire it for a new one.
    let short = short.unwrap();
    let wrong = Code {
        nameplate: short.nameplate.clone(),
        secret: if short.secret == "0000" {
            "0001".into()
        } else {
            "0000".into()
        },
    };
    for _ in 0..horizon_friends::host::MAX_WRONG {
        let e = join(&wrong.to_string(), Some(&address), PathMode::Auto)
            .err()
            .unwrap();
        assert!(matches!(e, JoinError::WrongCode), "{e}");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let fresh = loop {
        if let Some(s) = host.state().short
            && s.nameplate != short.nameplate
        {
            break s;
        }
        assert!(Instant::now() < deadline, "no new short code");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(matches!(
        join(&short.to_string(), Some(&address), PathMode::Auto),
        Err(JoinError::Ended)
    ));
    round_trip(
        join(&fresh.to_string(), Some(&address), PathMode::Auto).unwrap(),
        b"fresh",
    );
    // The full code follows the new short code; its secret stays.
    let (code2, _) = invite_of(&host, true);
    assert_eq!(long(&code2).relay.unwrap().code, fresh);
    assert_eq!(long(&code2).secret, long(&code).secret);
}

#[test]
fn bad_codes_unknown_codes_versions_and_ended_invites() {
    assert!(matches!(
        join("hello", None, PathMode::Auto),
        Err(JoinError::BadCode(_))
    ));
    let (relay, address) = relay(generous());
    let mut host = start(Some(&address), PathMode::RelayOnly);
    let (_, short) = invite_of(&host, true);
    let short = short.unwrap();
    let other = guest::join(&GuestConfig {
        code: short.to_string(),
        relay: Some(address.clone()),
        protocol: PROTOCOL - 1,
        game: "26.2".into(),
        path: PathMode::Auto,
    });
    match other {
        Err(e @ JoinError::VersionMismatch { .. }) => {
            assert_eq!(
                e.to_string(),
                "Your friend plays Minecraft 26.3; you have 26.2."
            )
        }
        other => panic!("{:?}", other.err()),
    }
    host.stop();
    assert!(matches!(
        join(&short.to_string(), Some(&address), PathMode::Auto),
        Err(JoinError::Ended)
    ));
    // A nameplate nobody holds; five of them block this address for a while.
    let free = (0..)
        .map(|i| horizon_friends::code::nameplate(i * 7919))
        .find(|n| *n != short.nameplate)
        .unwrap();
    let unknown = format!("{free}0000");
    for _ in 0..5 {
        assert!(matches!(
            join(&unknown, Some(&address), PathMode::Auto),
            Err(JoinError::UnknownCode)
        ));
    }
    assert!(matches!(
        join(&short.to_string(), Some(&address), PathMode::Auto),
        Err(JoinError::RateLimited)
    ));
    let counts = relay.counts();
    assert_eq!(counts.unknown, 5);
    assert!(counts.rate_limited >= 1);
    assert_eq!(counts.hosts, 0);
}

#[test]
fn a_host_that_never_attaches_is_unreachable() {
    use horizon_friends::wire::{self, FromRelay, ToRelay};
    let (_relay, address) = relay(Config {
        attach_wait: Duration::from_millis(500),
        ..generous()
    });
    let mut control = TcpStream::connect(&address).unwrap();
    wire::send(
        &mut control,
        &ToRelay::Host {
            v: wire::VERSION,
            protocol: PROTOCOL,
            game: "26.3".into(),
        },
    )
    .unwrap();
    let FromRelay::Registered {
        nameplate, token, ..
    } = wire::recv(&mut control).unwrap()
    else {
        panic!()
    };
    let code = format!("{nameplate}0000");
    assert!(matches!(
        join(&code, Some(&address), PathMode::Auto),
        Err(JoinError::HostUnreachable)
    ));
    // The relay announced the session; attaching late fails.
    let FromRelay::Join { session } = wire::recv(&mut control).unwrap() else {
        panic!()
    };
    let mut late = TcpStream::connect(&address).unwrap();
    wire::send(
        &mut late,
        &ToRelay::Attach {
            v: wire::VERSION,
            session,
            token,
        },
    )
    .unwrap();
    assert!(matches!(
        wire::recv::<_, FromRelay>(&mut late).unwrap(),
        FromRelay::Error { .. }
    ));
}

#[test]
fn relayed_bandwidth_is_capped() {
    let rate = 200_000.0;
    let (_relay, address) = relay(Config {
        session_rate: rate,
        ..generous()
    });
    let host = start(Some(&address), PathMode::RelayOnly);
    let (_, short) = invite_of(&host, true);
    let tunnel = join(
        &short.unwrap().to_string(),
        Some(&address),
        PathMode::RelayOnly,
    )
    .unwrap();
    let started = Instant::now();
    round_trip(tunnel, &payload(600_000));
    // The first 100 kB pass at once, the rest at 200 kB/s.
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn health_and_whoami_answer_http() {
    let (_relay, address) = relay(Config::default());
    let get = |path: &str| {
        let mut s = TcpStream::connect(&address).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: relay\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    };
    let health = get("/health");
    assert!(health.starts_with("HTTP/1.1 200 OK\r\n"), "{health}");
    assert!(health.ends_with("{\"ok\":true,\"hosts\":0,\"bridges\":0,\"registrations\":0,\"joins\":0,\"unknown\":0,\"rate_limited\":0,\"relayed_bytes\":0}"));
    assert!(get("/whoami").contains("\"address\":\"127.0.0.1:"));
    assert!(get("/").starts_with("HTTP/1.1 404"));
}

/// HTTP headers share the absolute first-line deadline.
#[test]
fn slow_http_headers_are_closed_within_the_first_line_deadline() {
    let (_relay, address) = relay(Config {
        first_line: Duration::from_secs(1),
        ..Config::default()
    });
    let mut stream = TcpStream::connect(&address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(b"GET /health HTTP/1.1\r\n").unwrap();
    let started = Instant::now();
    let mut write_closed = false;
    while started.elapsed() < Duration::from_secs(3) {
        if let Err(error) = stream.write_all(b"x") {
            assert!(matches!(
                error.kind(),
                io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
            ));
            write_closed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        write_closed,
        "relay did not close the incomplete HTTP request"
    );
    let mut response = [0u8; 1];
    match stream.read(&mut response) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
            ) => {}
        result => panic!("expected closed connection, got {result:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(3));
}

/// A first line trickled a byte at a time still has to arrive within
/// `first_line` in all; past `max_connections`, connections close unanswered.
#[test]
fn slow_first_lines_and_too_many_connections_are_closed() {
    let (_relay, address) = relay(Config {
        first_line: Duration::from_secs(1),
        max_connections: 2,
        ..Config::default()
    });
    let started = Instant::now();
    let mut slow = TcpStream::connect(&address).unwrap();
    let held = TcpStream::connect(&address).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let mut over = TcpStream::connect(&address).unwrap();
    over.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert_eq!(over.read(&mut [0u8; 1]).unwrap_or(0), 0);
    while slow.write_all(b"{").is_ok() && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(200));
    }
    let closed = started.elapsed();
    assert!(
        closed < Duration::from_secs(3),
        "the slow first line held on for {closed:?}"
    );
    drop(held);
}

#[test]
fn proxy_request_and_http_headers_share_one_opening_budget() {
    let (_relay, address) = relay(Config {
        proxy_protocol: true,
        first_line: Duration::from_millis(500),
        ..Config::default()
    });
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let started = Instant::now();
    stream
        .write_all(b"PROXY TCP4 192.0.2.7 192.0.2.8 1234 7000\r\n")
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    stream
        .write_all(b"GET /health HTTP/1.1\r\nX: a\r\n")
        .unwrap();
    // Complete lines do not restart the remaining 200ms budget.
    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    assert!(started.elapsed() < Duration::from_millis(750));
}

#[test]
fn proxy_and_incomplete_first_line_share_one_opening_budget() {
    let (_relay, address) = relay(Config {
        proxy_protocol: true,
        first_line: Duration::from_millis(500),
        ..Config::default()
    });
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let started = Instant::now();
    stream
        .write_all(b"PROXY TCP4 192.0.2.7 192.0.2.8 1234 7000\r\n")
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    stream.write_all(b"{").unwrap();
    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    assert!(started.elapsed() < Duration::from_millis(750));
}

#[test]
fn http_header_count_and_line_length_limits_are_unchanged() {
    let (_relay, address) = relay(Config::default());
    let connect = || {
        let stream = TcpStream::connect(&address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream
    };
    let mut stream = connect();
    stream.write_all(b"GET /health HTTP/1.1\r\n").unwrap();
    for _ in 0..65 {
        stream.write_all(b"X: a\r\n").unwrap();
    }
    // Existing policy answers after header65, without needing a blank line.
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    let mut stream = connect();
    stream.write_all(b"GET /health HTTP/1.1\r\n").unwrap();
    let mut line = vec![b'x'; horizon_friends::wire::MAX_LINE + 1];
    line.push(b'\n');
    stream.write_all(&line).unwrap();
    // Unread over-limit bytes permit a reset instead of graceful peer EOF.
    match stream.read(&mut [0]) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
            ) => {}
        result => panic!("over-limit line remained live: {result:?}"),
    }
}

/// Reusing the host's relay tuple after a large completed transfer used to
/// strand Attach on Windows; concurrent guests must have independent races.
#[test]
fn repeated_large_relay_transfers_keep_short_code_races_independent() {
    let (_relay, address) = relay(generous());
    let host = start(Some(&address), PathMode::Auto);
    let (_, short) = invite_of(&host, true);
    let short = short.unwrap().to_string();
    for _ in 0..2 {
        let tunnel = join(&short, Some(&address), PathMode::RelayOnly).unwrap();
        assert!(!tunnel.direct);
        round_trip(tunnel, &payload(1 << 20));
        let a = join(&short, Some(&address), PathMode::Auto).unwrap();
        let b = join(&short, Some(&address), PathMode::Auto).unwrap();
        assert!(a.direct && b.direct);
        let worker = std::thread::spawn(move || round_trip(a, b"first guest"));
        round_trip(b, b"second guest");
        worker.join().unwrap();
    }
}
