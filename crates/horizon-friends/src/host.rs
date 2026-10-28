//! The host's end. It listens on its direct port, works out how it can be
//! reached (interfaces, a router mapping, STUN) and puts that with a fresh
//! secret into the invite code: guests with the code dial it directly, no
//! server involved. When a relay is configured (the player's own, or any
//! they trust), it also registers there for a short code and serves relayed
//! sessions (with their own direct race, then the relay as fallback).
//! Every session ends at the integrated server's port. Everything runs on
//! its own threads; the game polls [`Host::poll`] and [`Host::state`].
use crate::{
    Code, PathMode,
    crypto::{self, Role},
    direct,
    invite::{Invite, RelayRef},
    reach::{self, Reach, Verdict},
    resolve_relay, session,
    wire::{self, FromRelay, ToRelay},
};
use std::{
    collections::HashMap,
    io::{self, BufReader},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Wrong short codes tried at the relay before the host retires it.
pub const MAX_WRONG: u32 = 5;
/// Sessions open at once (IntegratedServer.MAX_PLAYERS).
pub const MAX_SESSIONS: usize = 8;
/// The direct port asked for first, so a port forwarded by hand keeps
/// working from one session to the next (`HORIZON_FRIENDS_PORT`).
pub const DEFAULT_PORT: u16 = 25580;
/// Code handshakes in progress at once (scanners can't pile them up).
const MAX_HANDSHAKES: usize = 8;
/// The most an inbound stream's opening (hello, key exchange, confirmation)
/// may take, however slowly its bytes come.
const HANDSHAKE: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct HostConfig {
    /// The integrated server's port (Open to LAN's).
    pub lan: SocketAddr,
    pub protocol: i32,
    pub game: String,
    /// A relay to register with as well (short code, relay fallback).
    pub relay: Option<String>,
    pub path: PathMode,
    /// The direct port to ask for (0: any).
    pub port: u16,
    pub reach: reach::Options,
}

/// What the host's game shows.
#[derive(Clone, Debug, Default)]
pub struct HostState {
    /// The invite code; None while the host is still finding its addresses.
    pub invite: Option<String>,
    /// The relay's short code, when registered with one.
    pub short: Option<Code>,
    /// Who can join, and what to do when friends on the internet can't.
    pub summary: String,
    pub verdict: Option<Verdict>,
    /// Why the relay isn't serving, when one is configured.
    pub relay_problem: Option<String>,
    pub port: u16,
}

#[derive(Debug)]
pub enum HostEvent {
    /// A new invite code (the first, or after the relay's short code changed).
    Code(String),
    /// A guest's session is carried directly or through the relay.
    Connected {
        session: u64,
        direct: bool,
    },
    Disconnected {
        session: u64,
    },
    /// Someone tried a wrong code.
    WrongCode {
        count: u32,
    },
}

struct Shared {
    config: HostConfig,
    stop: AtomicBool,
    state: Mutex<HostState>,
    events: Mutex<Sender<HostEvent>>,
    /// The invite (endpoints and secret), rebuilt when the relay part changes.
    invite: Mutex<Option<Invite>>,
    /// The current relay control connection (shut down to stop).
    control: Mutex<Option<TcpStream>>,
    /// Inbound direct streams waiting for their relayed session.
    routes: Mutex<HashMap<u64, (Sender<TcpStream>, Instant)>>,
    sessions: AtomicUsize,
    handshakes: AtomicUsize,
    #[cfg(test)]
    opening_observer: Mutex<Option<Sender<OpeningEvent>>>,
    wrong: AtomicU32,
    relay_wrong: AtomicU32,
    reach: Mutex<Option<Reach>>,
}
#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
enum OpeningEvent {
    Reserved(bool),
    Rejected(bool),
    Released(bool),
}
impl Shared {
    #[cfg(test)]
    fn observe_opening(&self, event: OpeningEvent) {
        let observer = self.opening_observer.lock().unwrap().clone();
        if let Some(observer) = observer {
            let _ = observer.send(event);
        }
    }
    fn emit(&self, e: HostEvent) {
        let _ = self.events.lock().unwrap().send(e);
    }
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    /// Rebuild the code from the invite and the relay's short code.
    fn publish(&self, short: Option<Code>) {
        let text = {
            let mut invite = self.invite.lock().unwrap();
            let Some(inv) = invite.as_mut() else { return };
            inv.relay = match (&short, &self.config.relay) {
                (Some(code), Some(address)) => Some(RelayRef {
                    address: address.clone(),
                    code: code.clone(),
                }),
                _ => None,
            };
            inv.encode()
        };
        {
            let mut s = self.state.lock().unwrap();
            if s.invite.as_deref() == Some(&text) && s.short == short {
                return;
            }
            s.invite = Some(text.clone());
            s.short = short;
        }
        self.emit(HostEvent::Code(text));
    }
}

pub struct Host {
    shared: Arc<Shared>,
    events: Receiver<HostEvent>,
    threads: Vec<JoinHandle<()>>,
}

impl Host {
    /// Start sharing in the background: the code arrives as an event once
    /// the host knows its addresses (a few seconds).
    pub fn start(config: HostConfig) -> io::Result<Self> {
        let (tx, events) = channel();
        let (listeners, port) = direct::listen(config.port).or_else(|_| direct::listen(0))?;
        let shared = Arc::new(Shared {
            config,
            stop: AtomicBool::new(false),
            state: Mutex::new(HostState {
                port,
                ..HostState::default()
            }),
            events: Mutex::new(tx),
            invite: Mutex::new(None),
            control: Mutex::new(None),
            routes: Mutex::new(HashMap::new()),
            sessions: AtomicUsize::new(0),
            handshakes: AtomicUsize::new(0),
            #[cfg(test)]
            opening_observer: Mutex::new(None),
            wrong: AtomicU32::new(0),
            relay_wrong: AtomicU32::new(0),
            reach: Mutex::new(None),
        });
        let mut threads = Vec::new();
        let s = shared.clone();
        threads.push(
            std::thread::Builder::new()
                .name("horizon-friends-listen".into())
                .spawn(move || route_inbound(listeners, s))?,
        );
        let s = shared.clone();
        threads.push(
            std::thread::Builder::new()
                .name("horizon-friends-reach".into())
                .spawn(move || share(s, port))?,
        );
        Ok(Self {
            shared,
            events,
            threads,
        })
    }
    pub fn state(&self) -> HostState {
        self.shared.state.lock().unwrap().clone()
    }
    pub fn poll(&self) -> Vec<HostEvent> {
        self.events.try_iter().collect()
    }
    /// Guests whose sessions are open (connecting or playing).
    pub fn sessions(&self) -> usize {
        self.shared.sessions.load(Ordering::Relaxed)
    }
    /// End the code: close the direct port, delete the router mapping, leave
    /// the relay. Sessions already playing go on until the integrated server
    /// closes them.
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(c) = self.shared.control.lock().unwrap().take() {
            let _ = c.shutdown(Shutdown::Both);
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        if let Some(r) = self.shared.reach.lock().unwrap().as_mut() {
            r.release();
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Find the addresses, publish the code, keep the router mapping alive,
/// serve the relay when there is one.
fn share(shared: Arc<Shared>, port: u16) {
    let config = &shared.config;
    let reach = if config.path == PathMode::RelayOnly {
        Reach {
            port,
            ..Reach::default()
        }
    } else {
        Reach::discover(port, &config.reach)
    };
    let endpoints = if config.path == PathMode::RelayOnly {
        Vec::new()
    } else {
        reach.endpoints()
    };
    let invite = match Invite::new(port, endpoints, None) {
        Ok(i) => i,
        Err(_) => return,
    };
    {
        let mut s = shared.state.lock().unwrap();
        s.summary = if config.path == PathMode::RelayOnly {
            "Friends join through the relay only.".into()
        } else {
            reach.summary()
        };
        s.verdict = Some(reach.verdict());
    }
    *shared.invite.lock().unwrap() = Some(invite);
    *shared.reach.lock().unwrap() = Some(reach);
    if shared.stopped() {
        return;
    }
    shared.publish(None);
    let relay = match &config.relay {
        Some(r) => {
            let s = shared.clone();
            let r = r.clone();
            std::thread::Builder::new()
                .name("horizon-friends-relay".into())
                .spawn(move || run_relay(s, r))
                .ok()
        }
        None => None,
    };
    // Renew the router mapping at half-lease until stopped.
    while !shared.stopped() {
        std::thread::sleep(Duration::from_millis(100));
        let mut reach = shared.reach.lock().unwrap();
        if let Some(m) = reach.as_mut().and_then(|r| r.mapping.as_mut())
            && m.due()
            && m.renew().is_err()
        {
            // Try again in a minute rather than hammering the router.
            m.renewed = Instant::now() - m.lifetime / 2 + Duration::from_secs(60);
        }
    }
    if let Some(t) = relay {
        let _ = t.join();
    }
}

/// Inbound streams on the direct port: a relayed session's direct race, or
/// a guest with the code.
fn route_inbound(listeners: Vec<TcpListener>, shared: Arc<Shared>) {
    route_until(listeners, shared, None);
}

/// The same admission and absolute opening guard serve both the long-lived
/// direct port and a relayed session's short-lived rendezvous port.
fn route_until(
    listeners: Vec<TcpListener>,
    shared: Arc<Shared>,
    race: Option<(u64, Instant, Arc<AtomicBool>)>,
) {
    while !shared.stopped()
        && race
            .as_ref()
            .is_none_or(|(_, until, stop)| Instant::now() < *until && !stop.load(Ordering::Relaxed))
    {
        let mut idle = true;
        for l in &listeners {
            let Ok((s, _)) = l.accept() else { continue };
            idle = false;
            if shared.handshakes.fetch_add(1, Ordering::Relaxed) >= MAX_HANDSHAKES {
                shared.handshakes.fetch_sub(1, Ordering::Relaxed);
                #[cfg(test)]
                shared.observe_opening(OpeningEvent::Rejected(race.is_some()));
                continue;
            }
            #[cfg(test)]
            shared.observe_opening(OpeningEvent::Reserved(race.is_some()));
            let worker = shared.clone();
            let only_session = race.as_ref().map(|(session, _, _)| *session);
            let spawned = std::thread::Builder::new()
                .name("horizon-friends-inbound".into())
                .spawn(move || {
                    let shared = worker;
                    let _ = s.set_nonblocking(false);
                    let Ok(mut deadline) = s
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .and_then(|()| crate::deadline(s, HANDSHAKE))
                    else {
                        shared.handshakes.fetch_sub(1, Ordering::Relaxed);
                        #[cfg(test)]
                        shared.observe_opening(OpeningEvent::Released(only_session.is_some()));
                        return;
                    };
                    match direct::read_hello_from(&mut deadline) {
                        Ok(direct::Hello::Relayed(session)) => {
                            shared.handshakes.fetch_sub(1, Ordering::Relaxed);
                            #[cfg(test)]
                            shared.observe_opening(OpeningEvent::Released(only_session.is_some()));
                            if only_session.is_some_and(|only| only != session) {
                                return;
                            }
                            let route = {
                                let mut routes = shared.routes.lock().unwrap();
                                routes.retain(|_, (_, until)| *until > Instant::now());
                                routes.get(&session).map(|(tx, _)| tx.clone())
                            };
                            let Some(tx) = route else { return };
                            if direct::answer(&mut deadline, session).is_err() {
                                return;
                            }
                            if let Ok(s) = deadline.cancel() {
                                let _ = tx.send(s);
                            }
                        }
                        Ok(direct::Hello::Code(session)) if only_session.is_none() => {
                            serve_code(&shared, deadline, session)
                        }
                        _ => {
                            shared.handshakes.fetch_sub(1, Ordering::Relaxed);
                            #[cfg(test)]
                            shared.observe_opening(OpeningEvent::Released(only_session.is_some()));
                        }
                    }
                });
            if spawned.is_err() {
                shared.handshakes.fetch_sub(1, Ordering::Relaxed);
                #[cfg(test)]
                shared.observe_opening(OpeningEvent::Released(race.is_some()));
            }
        }
        if idle {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A guest who dialled an endpoint from the code: agree on the key from
/// the code's secret, then carry the session (`deadline` bounds the
/// handshake, not the game).
fn serve_code(shared: &Arc<Shared>, mut s: crate::Deadline, session: u64) {
    let secret = shared
        .invite
        .lock()
        .unwrap()
        .as_ref()
        .map(Invite::secret_text);
    let secured = (|| -> io::Result<session::Secure> {
        let secret = secret.ok_or_else(|| io::Error::other("not ready"))?;
        direct::answer_code(&mut s, session)?;
        let key = crypto::exchange(&mut s, &secret, "direct", session)?;
        session::secure_deadline(s, &key, Role::Host)
    })();
    shared.handshakes.fetch_sub(1, Ordering::Relaxed);
    #[cfg(test)]
    shared.observe_opening(OpeningEvent::Released(false));
    match secured {
        Ok(secure) => carry(shared, session, secure, true),
        Err(e) if crypto::is_wrong_code(&e) => {
            let count = shared.wrong.fetch_add(1, Ordering::Relaxed) + 1;
            shared.emit(HostEvent::WrongCode { count });
        }
        Err(_) => {}
    }
}

/// Connect a settled session to the integrated server and carry it.
fn carry(shared: &Arc<Shared>, session: u64, (r, w): session::Secure, direct: bool) {
    if shared.sessions.fetch_add(1, Ordering::Relaxed) >= MAX_SESSIONS {
        shared.sessions.fetch_sub(1, Ordering::Relaxed);
        let _ = r.get_ref().shutdown(Shutdown::Both);
        return;
    }
    let Ok(server) = TcpStream::connect_timeout(&shared.config.lan, Duration::from_secs(5)) else {
        let _ = r.get_ref().shutdown(Shutdown::Both);
        shared.sessions.fetch_sub(1, Ordering::Relaxed);
        return;
    };
    shared.emit(HostEvent::Connected { session, direct });
    if let Ok(t) = crate::pump(server, r, w) {
        let _ = t.join();
    }
    shared.sessions.fetch_sub(1, Ordering::Relaxed);
    shared.emit(HostEvent::Disconnected { session });
}

// ----- The relay (optional) ---------------------------------------------------------------------

fn connect_relay(relay: &str, port: Option<u16>) -> io::Result<(TcpStream, SocketAddr)> {
    let mut last = io::Error::other("no address");
    for address in resolve_relay(relay)? {
        match direct::connect_from(port, address, Duration::from_secs(5)) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok((s, address));
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Register, serve the control connection, register again when it drops
/// (a new short code) until stopped.
fn run_relay(shared: Arc<Shared>, relay: String) {
    let mut backoff = Duration::from_secs(1);
    while !shared.stopped() {
        match register(&shared, &relay) {
            Ok(()) => {
                backoff = Duration::from_secs(1);
                shared.state.lock().unwrap().relay_problem = None;
            }
            Err(e) => {
                if shared.stopped() {
                    break;
                }
                shared.state.lock().unwrap().relay_problem =
                    Some(format!("Couldn't reach the relay {relay}: {e}"));
                shared.publish(None);
                let until = Instant::now() + backoff;
                while Instant::now() < until && !shared.stopped() {
                    std::thread::sleep(Duration::from_millis(50));
                }
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

fn register(shared: &Arc<Shared>, relay_url: &str) -> io::Result<()> {
    let config = &shared.config;
    // Control registration does not occupy a session's rendezvous tuple.
    let (mut control, _) = connect_relay(relay_url, None)?;
    control.set_read_timeout(Some(Duration::from_secs(70)))?;
    wire::send(
        &mut control,
        &ToRelay::Host {
            v: wire::VERSION,
            protocol: config.protocol,
            game: config.game.clone(),
        },
    )?;
    let mut reader = BufReader::new(control.try_clone()?);
    let (nameplate, token) = match wire::recv_buffered::<_, FromRelay>(&mut reader)? {
        FromRelay::Registered {
            nameplate, token, ..
        } => (nameplate, token),
        FromRelay::Error { code, .. } => {
            return Err(io::Error::other(format!("it refused ({})", code.as_str())));
        }
        other => return Err(io::Error::other(format!("unexpected answer {other:?}"))),
    };
    let code = Code::new(&nameplate)?;
    *shared.control.lock().unwrap() = Some(control.try_clone()?);
    if shared.stopped() {
        let _ = control.shutdown(Shutdown::Both);
        return Ok(());
    }
    shared.relay_wrong.store(0, Ordering::Relaxed);
    shared.publish(Some(code.clone()));
    loop {
        let message = match wire::recv_buffered::<_, FromRelay>(&mut reader) {
            Ok(m) => m,
            Err(e) => {
                shared.control.lock().unwrap().take();
                return if shared.stopped()
                    || shared.relay_wrong.load(Ordering::Relaxed) >= MAX_WRONG
                {
                    Ok(())
                } else {
                    Err(e)
                };
            }
        };
        match message {
            FromRelay::Ping => wire::send(&mut control, &ToRelay::Pong)?,
            FromRelay::Join { session } => {
                if shared.sessions.load(Ordering::Relaxed) >= MAX_SESSIONS {
                    continue;
                }
                let (shared, code, token, control, relay_url) = (
                    shared.clone(),
                    code.clone(),
                    token.clone(),
                    control.try_clone()?,
                    relay_url.to_owned(),
                );
                let _ = std::thread::Builder::new()
                    .name("horizon-friends-session".into())
                    .spawn(move || {
                        serve_relayed(&shared, &relay_url, &code, &token, session, &control)
                    });
            }
            _ => {}
        }
    }
}

/// One guest through the relay: attach, agree on the key from the short
/// code, settle on a direct or relayed stream, carry it.
fn serve_relayed(
    shared: &Arc<Shared>,
    relay_url: &str,
    code: &Code,
    token: &str,
    session: u64,
    control: &TcpStream,
) {
    let config = &shared.config;
    // A completed relay transfer can leave its TCP tuple unusable on Windows.
    // Own a fresh port for this attachment AND its direct simultaneous-open
    // race, rather than spending the attachment budget on the old tuple.
    let (listeners, port) = rendezvous(direct::listen(0));
    let Ok((mut stream, relay)) = connect_relay(relay_url, port) else {
        return;
    };
    let attachment_port = stream.local_addr().ok().map(|a| a.port());
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let attach = ToRelay::Attach {
        v: wire::VERSION,
        session,
        token: token.to_owned(),
    };
    if wire::send(&mut stream, &attach).is_err() {
        return;
    }
    let observed = match wire::recv::<_, FromRelay>(&mut stream) {
        Ok(FromRelay::Bridged { observed, .. }) => {
            observed.and_then(|o| o.parse::<SocketAddr>().ok())
        }
        _ => return,
    };
    let Ok(key) = crypto::exchange(&mut stream, &code.secret, &code.nameplate, session) else {
        return;
    };
    let relayed = match session::secure(stream, &key, Role::Host) {
        Ok(s) => s,
        Err(e) => {
            if crypto::is_wrong_code(&e) {
                let count = shared.relay_wrong.fetch_add(1, Ordering::Relaxed) + 1;
                shared.emit(HostEvent::WrongCode { count });
                if count >= MAX_WRONG {
                    // Retire the short code: dropping the control connection
                    // ends it at the relay, and `run_relay` registers anew.
                    let _ = control.shutdown(Shutdown::Both);
                }
            }
            return;
        }
    };
    let candidates = port.map_or_else(Vec::new, |p| {
        let global = shared
            .reach
            .lock()
            .unwrap()
            .as_ref()
            .map(|reach| shared_candidates(reach, observed))
            .unwrap_or_default();
        relay_candidates(
            direct::candidates(p, Some(relay), None),
            global,
            reflected_candidate(observed, port, attachment_port),
        )
    });
    let routes = shared.clone();
    let mut race = InboundRace::default();
    let settled = session::settle(
        relayed,
        &key,
        Role::Host,
        config.path,
        session,
        port,
        candidates,
        Box::new(|tx, deadline, stop| {
            routes
                .routes
                .lock()
                .unwrap()
                .insert(session, (tx, deadline + Duration::from_millis(500)));
            race.stop = Some(stop.clone());
            race.worker = std::thread::Builder::new()
                .name("horizon-friends-rendezvous".into())
                .spawn(move || route_until(listeners, routes, Some((session, deadline, stop))))
                .ok();
        }),
    );
    race.finish(shared, session);
    if let Ok(settled) = settled {
        carry(shared, session, settled.secure, settled.direct);
    }
}

/// Allocation failure degrades like the guest: relay still works, without
/// claiming a reflexive address for a port that we cannot listen/race on.
fn rendezvous(result: io::Result<(Vec<TcpListener>, u16)>) -> (Vec<TcpListener>, Option<u16>) {
    match result {
        Ok((listeners, port)) => (listeners, Some(port)),
        Err(_) => (Vec::new(), None),
    }
}

/// A bind fallback may use another port. Its observed mapping cannot describe
/// our listener, but the listener's interface/global routes are still useful.
fn reflected_candidate(
    observed: Option<SocketAddr>,
    port: Option<u16>,
    attachment_port: Option<u16>,
) -> Option<SocketAddr> {
    observed.filter(|_| port.is_some() && port == attachment_port)
}

fn shared_candidates(reach: &Reach, observed: Option<SocketAddr>) -> Vec<SocketAddr> {
    // Only the IP is evidence for the shared port. This is an inferred
    // manual-forward/mapping candidate, NOT the fresh reflexive tuple.
    // A local clone preserves discovery state and performs no network work.
    let mut inferred = reach.clone();
    if inferred.public_ip.is_none() {
        inferred.public_ip = observed
            .filter(|address| !address.ip().is_unspecified())
            .map(|address| address.ip());
    }
    inferred.endpoints()
}

/// Reserve the observed path, best shared (mapped/forwarded) path and fresh
/// interface path before filling the bounded, deduplicated candidate list.
fn relay_candidates(
    local: Vec<SocketAddr>,
    global: Vec<SocketAddr>,
    observed: Option<SocketAddr>,
) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    for address in observed
        .into_iter()
        .chain(global.first().copied())
        .chain(local.first().copied())
        .chain(global)
        .chain(local)
    {
        if !address.ip().is_unspecified() && !out.contains(&address) {
            out.push(address);
            if out.len() == crate::invite::MAX_ENDPOINTS {
                break;
            }
        }
    }
    out
}

#[derive(Default)]
struct InboundRace {
    stop: Option<Arc<AtomicBool>>,
    worker: Option<JoinHandle<()>>,
}
impl InboundRace {
    fn finish(self, shared: &Shared, session: u64) {
        drop(self);
        shared.routes.lock().unwrap().remove(&session);
    }
}
impl Drop for InboundRace {
    fn drop(&mut self) {
        if let Some(stop) = &self.stop {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod opening_tests {
    use super::*;
    use std::io::{Read, Write};

    fn shared(lan: SocketAddr) -> Arc<Shared> {
        let (tx, _events) = channel();
        Arc::new(Shared {
            config: HostConfig {
                lan,
                protocol: 777,
                game: "test".into(),
                relay: None,
                path: PathMode::DirectOnly,
                port: 0,
                reach: reach::Options {
                    map_port: false,
                    stun: vec![],
                    loopback: true,
                },
            },
            stop: AtomicBool::new(false),
            state: Mutex::new(HostState::default()),
            events: Mutex::new(tx),
            invite: Mutex::new(Some(Invite {
                secret: [7; 10],
                port: 0,
                endpoints: vec![],
                relay: None,
            })),
            control: Mutex::new(None),
            routes: Mutex::new(HashMap::new()),
            sessions: AtomicUsize::new(0),
            handshakes: AtomicUsize::new(0),
            opening_observer: Mutex::new(None),
            wrong: AtomicU32::new(0),
            relay_wrong: AtomicU32::new(0),
            reach: Mutex::new(None),
        })
    }

    struct Router {
        shared: Arc<Shared>,
        thread: Option<JoinHandle<()>>,
        address: SocketAddr,
    }
    impl Router {
        fn start(shared: Arc<Shared>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            listener.set_nonblocking(true).unwrap();
            let worker = shared.clone();
            let thread = Some(std::thread::spawn(move || {
                route_inbound(vec![listener], worker)
            }));
            Self {
                shared,
                thread,
                address,
            }
        }
    }
    impl Drop for Router {
        fn drop(&mut self) {
            self.shared.stop.store(true, Ordering::Relaxed);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    #[test]
    fn relayed_handoff_restores_hello_baseline_and_keeps_payload() {
        let shared = shared("127.0.0.1:9".parse().unwrap());
        let (tx, rx) = channel();
        shared
            .routes
            .lock()
            .unwrap()
            .insert(17, (tx, Instant::now() + Duration::from_secs(20)));
        let router = Router::start(shared.clone());
        let mut peer = TcpStream::connect(router.address).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let started = Instant::now();
        direct::answer(&mut peer, 17).unwrap();
        let mut echo = [0; 12];
        peer.read_exact(&mut echo).unwrap();
        let mut routed = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(routed.read_timeout().unwrap(), Some(Duration::from_secs(5)));
        assert_eq!(routed.write_timeout().unwrap(), None);
        assert_eq!(shared.handshakes.load(Ordering::Relaxed), 0);
        std::thread::sleep(
            (HANDSHAKE + Duration::from_millis(100)).saturating_sub(started.elapsed()),
        );
        peer.write_all(b"payload").unwrap();
        let mut data = [0; 7];
        routed.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"payload");
        routed.write_all(b"reply").unwrap();
        let mut reply = [0; 5];
        peer.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"reply");
    }

    fn wait_for(mut predicate: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(3);
        while !predicate() {
            assert!(Instant::now() < until, "opening cleanup did not complete");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn code_handoff_keeps_confirmed_bidirectional_payload_after_deadline() {
        let lan = TcpListener::bind("127.0.0.1:0").unwrap();
        lan.set_nonblocking(true).unwrap();
        let shared = shared(lan.local_addr().unwrap());
        let secret = shared
            .invite
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .secret_text();
        let router = Router::start(shared.clone());
        let mut peer = TcpStream::connect(router.address).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let started = Instant::now();
        direct::code_hello(&mut peer, 23).unwrap();
        let key = crypto::exchange(&mut peer, &secret, "direct", 23).unwrap();
        let (mut reader, mut writer) = session::secure(peer, &key, Role::Guest).unwrap();
        let mut server = None;
        wait_for(|| {
            server = lan.accept().ok().map(|(socket, _)| socket);
            server.is_some()
        });
        let mut server = server.unwrap();
        server.set_nonblocking(false).unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        server
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        assert_eq!(shared.handshakes.load(Ordering::Relaxed), 0);
        std::thread::sleep(
            (HANDSHAKE + Duration::from_millis(100)).saturating_sub(started.elapsed()),
        );
        writer.write_data(b"guest payload").unwrap();
        let mut data = [0; 13];
        server.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"guest payload");
        server.write_all(b"host payload").unwrap();
        assert!(
            matches!(reader.read_frame().unwrap(), Some(crypto::Frame::Data(data)) if data == b"host payload")
        );
        let _ = reader.get_ref().shutdown(Shutdown::Both);
        let _ = server.shutdown(Shutdown::Both);
        wait_for(|| shared.sessions.load(Ordering::Relaxed) == 0);
    }

    #[test]
    fn wrong_code_and_invalid_hello_release_opening_slots() {
        let shared = shared("127.0.0.1:9".parse().unwrap());
        let router = Router::start(shared.clone());
        let mut peer = TcpStream::connect(router.address).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        direct::code_hello(&mut peer, 29).unwrap();
        let key = crypto::exchange(&mut peer, "wrong secret", "direct", 29).unwrap();
        assert!(session::secure(peer, &key, Role::Guest).is_err());
        wait_for(|| shared.wrong.load(Ordering::Relaxed) == 1);
        assert_eq!(shared.handshakes.load(Ordering::Relaxed), 0);
        assert_eq!(shared.sessions.load(Ordering::Relaxed), 0);
        let mut peer = TcpStream::connect(router.address).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        peer.write_all(b"bad hello!!!").unwrap();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        wait_for(|| shared.handshakes.load(Ordering::Relaxed) == 0);
        assert_eq!(shared.wrong.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn early_closed_global_opening_releases_admission() {
        let shared = shared("127.0.0.1:9".parse().unwrap());
        let router = Router::start(shared.clone());
        let peer = TcpStream::connect(router.address).unwrap();
        peer.shutdown(Shutdown::Both).unwrap();
        drop(peer);
        let mut peer = TcpStream::connect(router.address).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        peer.write_all(b"bad hello!!!").unwrap();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        wait_for(|| shared.handshakes.load(Ordering::Relaxed) == 0);
    }

    #[test]
    fn relayed_candidates_keep_observed_forwarded_and_fresh_paths_at_capacity() {
        let observed = "203.0.113.8:40000".parse().unwrap();
        let forwarded = "203.0.113.8:25580".parse().unwrap();
        let fresh = "192.168.1.2:40000".parse().unwrap();
        let mut global = vec![forwarded, forwarded];
        global.extend((1..=8).map(|n| SocketAddr::from(([10, 0, 0, n], 25580))));
        let candidates = relay_candidates(vec![fresh, observed], global, Some(observed));
        assert_eq!(&candidates[..3], &[observed, forwarded, fresh]);
        assert_eq!(candidates.len(), crate::invite::MAX_ENDPOINTS);
        assert_eq!(candidates.iter().filter(|&&a| a == observed).count(), 1);
        assert_eq!(candidates.iter().filter(|&&a| a == forwarded).count(), 1);
    }

    #[test]
    fn relay_ip_preserves_shared_public_paths_without_discovery() {
        let observed = "203.0.113.8:40000".parse().unwrap();
        let reach = Reach {
            port: 25580,
            local: vec!["192.168.1.2".parse().unwrap()],
            ..Reach::default()
        };
        let fresh = "192.168.1.2:40000".parse().unwrap();
        let candidates = relay_candidates(
            vec![fresh],
            shared_candidates(&reach, Some(observed)),
            reflected_candidate(Some(observed), Some(40000), Some(40000)),
        );
        assert!(candidates.contains(&"203.0.113.8:25580".parse().unwrap()));
        assert_eq!(candidates[0], observed);
        assert!(candidates.contains(&fresh));
    }

    #[test]
    fn relay_ip_inference_respects_existing_mapping_discovery_and_port_mismatch() {
        use crate::portmap::{Mapping, Method};
        let observed = "203.0.113.8:40001".parse().unwrap();
        let mut reach = Reach {
            port: 25580,
            local: vec!["192.168.1.2".parse().unwrap()],
            mapping: Some(Mapping::granted(
                Method::NatPmp,
                "0.0.0.0:40123".parse().unwrap(),
            )),
            ..Reach::default()
        };
        let fresh = "192.168.1.2:40000".parse().unwrap();
        let assembled = relay_candidates(
            vec![fresh],
            shared_candidates(&reach, Some(observed)),
            reflected_candidate(Some(observed), Some(40000), Some(40001)),
        );
        assert_eq!(assembled[0], "203.0.113.8:40123".parse().unwrap());
        assert!(assembled.contains(&"203.0.113.8:25580".parse().unwrap()));
        assert!(assembled.contains(&fresh));
        assert!(!assembled.contains(&observed));
        assert_eq!(reach.public_ip, None);
        assert_eq!(
            reach.mapping.as_ref().unwrap().external,
            "0.0.0.0:40123".parse().unwrap()
        );
        reach.public_ip = Some("198.51.100.9".parse().unwrap());
        let known = shared_candidates(&reach, Some(observed));
        assert_eq!(known[0], "198.51.100.9:40123".parse().unwrap());
        assert!(known.contains(&"198.51.100.9:25580".parse().unwrap()));
        assert!(!known.contains(&"203.0.113.8:25580".parse().unwrap()));
        reach.local = (1..=12)
            .map(|n| format!("10.0.0.{n}").parse().unwrap())
            .collect();
        let capped = relay_candidates(
            vec![fresh],
            shared_candidates(&reach, Some(observed)),
            Some(observed),
        );
        assert_eq!(capped.len(), crate::invite::MAX_ENDPOINTS);
        assert_eq!(
            &capped[..3],
            &[observed, "198.51.100.9:40123".parse().unwrap(), fresh]
        );
    }

    #[test]
    fn missing_rendezvous_listener_keeps_relay_only_capability() {
        let (listeners, port) = rendezvous(Err(io::ErrorKind::AddrNotAvailable.into()));
        assert!(listeners.is_empty());
        assert_eq!(port, None);
    }

    #[test]
    fn mismatched_attachment_omits_only_the_false_reflexive_candidate() {
        let observed = "203.0.113.8:40000".parse().unwrap();
        let forwarded = "203.0.113.8:25580".parse().unwrap();
        let fresh = "192.168.1.2:40000".parse().unwrap();
        let selected = reflected_candidate(Some(observed), Some(40000), Some(40001));
        assert_eq!(
            relay_candidates(vec![fresh], vec![forwarded], selected),
            vec![forwarded, fresh]
        );
        assert_eq!(
            reflected_candidate(Some(observed), Some(40000), Some(40000)),
            Some(observed)
        );
        assert_eq!(reflected_candidate(Some(observed), None, Some(40001)), None);
    }

    /// The new port must use the SAME global route/admission machinery,
    /// reject other sessions and release its listener when its owner drops.
    #[test]
    fn fresh_and_global_listeners_share_routes_and_admission() {
        let shared = shared("127.0.0.1:9".parse().unwrap());
        let router = Router::start(shared.clone());
        let (listeners, port) = direct::listen(0).unwrap();
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = shared.clone();
        let flag = stop.clone();
        let race = InboundRace {
            stop: Some(stop),
            worker: Some(std::thread::spawn(move || {
                route_until(
                    listeners,
                    worker,
                    Some((71, Instant::now() + direct::RACE, flag)),
                );
            })),
        };
        let (tx, rx) = channel();
        shared
            .routes
            .lock()
            .unwrap()
            .insert(71, (tx, Instant::now() + direct::RACE));

        // Already occupied admission slots reject fresh AND shared traffic.
        shared.handshakes.store(MAX_HANDSHAKES, Ordering::Relaxed);
        for to in [address, router.address] {
            let mut peer = TcpStream::connect(to).unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        }
        assert_eq!(shared.handshakes.load(Ordering::Relaxed), MAX_HANDSHAKES);
        shared.handshakes.store(0, Ordering::Relaxed);
        for to in [address, router.address] {
            let mut peer = TcpStream::connect(to).unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            peer.set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            direct::answer(&mut peer, 71).unwrap();
            let mut hello = [0; 12];
            peer.read_exact(&mut hello).unwrap();
            let mut routed = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            peer.write_all(b"handoff").unwrap();
            let mut data = [0; 7];
            routed.read_exact(&mut data).unwrap();
            assert_eq!(&data, b"handoff");
        }
        let mut wrong = TcpStream::connect(address).unwrap();
        wrong
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        direct::answer(&mut wrong, 72).unwrap();
        assert_eq!(wrong.read(&mut [0]).unwrap(), 0);
        wait_for(|| shared.handshakes.load(Ordering::Relaxed) == 0);
        // Drop joins the finite accept loop, including an error-path stop.
        drop(race);
        assert!(TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err());
    }

    fn owned_test_race(
        shared: &Arc<Shared>,
        session: u64,
    ) -> (InboundRace, SocketAddr, Receiver<TcpStream>) {
        let (listeners, port) = direct::listen(0).unwrap();
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = shared.clone();
        let (tx, rx) = channel();
        let until = Instant::now() + direct::RACE;
        shared.routes.lock().unwrap().insert(session, (tx, until));
        let race = InboundRace {
            stop: Some(stop),
            worker: Some(std::thread::spawn(move || {
                route_until(listeners, worker, Some((session, until, flag)));
            })),
        };
        (race, address, rx)
    }

    fn opening_event(rx: &Receiver<OpeningEvent>) -> OpeningEvent {
        rx.recv_timeout(Duration::from_secs(2)).unwrap()
    }

    fn opening_peer(address: SocketAddr) -> TcpStream {
        let peer = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        peer
    }

    #[test]
    fn overlapping_session_owners_isolate_shared_and_fresh_handoffs() {
        std::thread::scope(|scope| {
            let shared = shared("127.0.0.1:9".parse().unwrap());
            let router = Router::start(shared.clone());
            let (events, observed) = channel();
            *shared.opening_observer.lock().unwrap() = Some(events);
            let (a, aa, arx) = owned_test_race(&shared, 91);
            let (b, ba, brx) = owned_test_race(&shared, 92);
            let mut workers = Vec::new();
            let mut releases = Vec::new();
            for (address, session, data) in [
                (aa, 91u64, *b"a-fresh!"),
                (router.address, 91, *b"a-shared"),
                (ba, 92, *b"b-fresh!"),
                (router.address, 92, *b"b-shared"),
            ] {
                let mut peer = opening_peer(address);
                let mut hello = *b"HZF1\0\0\0\0\0\0\0\0";
                hello[4..].copy_from_slice(&session.to_be_bytes());
                peer.write_all(&hello[..11]).unwrap();
                let (go, ready) = channel();
                releases.push(go);
                workers.push(scope.spawn(move || {
                    ready.recv_timeout(Duration::from_secs(2)).unwrap();
                    peer.write_all(&hello[11..]).unwrap();
                    let mut echo = [0; 12];
                    peer.read_exact(&mut echo).unwrap();
                    assert_eq!(echo, hello);
                    peer.write_all(&data).unwrap();
                    let mut reply = [0; 8];
                    peer.read_exact(&mut reply).unwrap();
                    assert_eq!(reply, data);
                }));
            }
            // All four actual openings remain held BEFORE any hello completes.
            let mut fresh = 0;
            for _ in 0..4 {
                match opening_event(&observed) {
                    OpeningEvent::Reserved(is_fresh) => fresh += usize::from(is_fresh),
                    event => panic!("unexpected opening transition: {event:?}"),
                }
            }
            assert_eq!(fresh, 2);
            assert_eq!(shared.handshakes.load(Ordering::Relaxed), 4);
            assert_eq!(shared.routes.lock().unwrap().len(), 2);
            assert!(!a.worker.as_ref().unwrap().is_finished());
            assert!(!b.worker.as_ref().unwrap().is_finished());
            for go in releases {
                go.send(()).unwrap();
            }
            for (rx, expected) in [
                (&arx, [*b"a-fresh!", *b"a-shared"]),
                (&brx, [*b"b-fresh!", *b"b-shared"]),
            ] {
                let mut seen = Vec::new();
                for _ in 0..2 {
                    let mut stream = rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut data = [0; 8];
                    stream.read_exact(&mut data).unwrap();
                    assert!(expected.contains(&data));
                    assert!(!seen.contains(&data));
                    seen.push(data);
                    stream.write_all(&data).unwrap();
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
            for _ in 0..4 {
                assert!(matches!(
                    opening_event(&observed),
                    OpeningEvent::Released(_)
                ));
            }
            assert_eq!(shared.handshakes.load(Ordering::Relaxed), 0);
            // Early failures on BOTH fresh owners release actual slots too.
            for address in [aa, ba] {
                let mut peer = opening_peer(address);
                peer.write_all(b"H").unwrap();
                assert_eq!(opening_event(&observed), OpeningEvent::Reserved(true));
                peer.shutdown(Shutdown::Both).unwrap();
                drop(peer);
                assert_eq!(opening_event(&observed), OpeningEvent::Released(true));
            }
            assert_eq!(shared.handshakes.load(Ordering::Relaxed), 0);
            a.finish(&shared, 91);
            b.finish(&shared, 92);
            assert!(shared.routes.lock().unwrap().is_empty());
            for address in [aa, ba] {
                assert!(TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err());
            }
        });
    }

    #[test]
    fn competing_shared_and_fresh_openings_reserve_only_the_last_slot() {
        use std::sync::Barrier;
        std::thread::scope(|scope| {
            let shared = shared("127.0.0.1:9".parse().unwrap());
            let router = Router::start(shared.clone());
            let (events, observed) = channel();
            *shared.opening_observer.lock().unwrap() = Some(events);
            let (race, address, rx) = owned_test_race(&shared, 93);
            let mut held = Vec::new();
            // Real Code openings wait for key exchange while retaining slots.
            for session in 0..MAX_HANDSHAKES - 1 {
                let mut peer = opening_peer(router.address);
                direct::code_hello(&mut peer, session as u64).unwrap();
                assert_eq!(opening_event(&observed), OpeningEvent::Reserved(false));
                held.push(peer);
            }
            assert_eq!(
                shared.handshakes.load(Ordering::Relaxed),
                MAX_HANDSHAKES - 1
            );
            let start = Arc::new(Barrier::new(3));
            let global_start = start.clone();
            let global_address = router.address;
            let (result, outcome) = channel();
            let (release, until_release) = channel();
            let global = scope.spawn(move || {
                global_start.wait();
                let mut peer = opening_peer(global_address);
                let accepted = direct::code_hello(&mut peer, 94).is_ok();
                result.send(accepted).unwrap();
                until_release.recv_timeout(Duration::from_secs(2)).unwrap();
                let _ = peer.shutdown(Shutdown::Both);
            });
            let fresh_start = start.clone();
            let (complete, until_complete) = channel();
            let fresh = scope.spawn(move || {
                fresh_start.wait();
                let mut peer = opening_peer(address);
                let mut hello = *b"HZF1\0\0\0\0\0\0\0\0";
                hello[4..].copy_from_slice(&93u64.to_be_bytes());
                let wrote = peer.write_all(&hello[..11]).is_ok();
                until_complete.recv_timeout(Duration::from_secs(2)).unwrap();
                if !wrote || peer.write_all(&hello[11..]).is_err() {
                    return false;
                }
                let mut echo = [0; 12];
                peer.read_exact(&mut echo).is_ok() && echo == hello
            });
            start.wait();
            let first = opening_event(&observed);
            let second = opening_event(&observed);
            let fresh_won = match (first, second) {
                (OpeningEvent::Reserved(a), OpeningEvent::Rejected(b))
                | (OpeningEvent::Rejected(b), OpeningEvent::Reserved(a)) => {
                    assert_ne!(a, b);
                    a
                }
                events => panic!("more than one winner or missing rejection: {events:?}"),
            };
            assert_eq!(shared.handshakes.load(Ordering::Relaxed), MAX_HANDSHAKES);
            let global_won = outcome.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_ne!(fresh_won, global_won);
            complete.send(()).unwrap();
            assert_eq!(fresh.join().unwrap(), fresh_won);
            if fresh_won {
                drop(rx.recv_timeout(Duration::from_secs(2)).unwrap());
            }
            release.send(()).unwrap();
            global.join().unwrap();
            for peer in held {
                peer.shutdown(Shutdown::Both).unwrap();
            }
            for _ in 0..MAX_HANDSHAKES {
                assert!(matches!(
                    opening_event(&observed),
                    OpeningEvent::Released(_)
                ));
            }
            assert_eq!(shared.handshakes.load(Ordering::Relaxed), 0);
            race.finish(&shared, 93);
            assert!(shared.routes.lock().unwrap().is_empty());
            assert!(TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err());
        });
    }
}
