//! Deadline-bound ownership of a handshake socket.
use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    sync::{
        Arc, Mutex,
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Active,
    Expired,
    Failed,
    Released,
}

struct Control {
    end: Instant,
    state: Mutex<State>,
    cancel: Mutex<Option<Sender<()>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
}

fn poisoned() -> io::Error {
    io::Error::other("deadline state poisoned")
}
fn expired() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "handshake deadline expired")
}

fn socket_timeout(remaining: Duration, baseline: Option<Duration>) -> Duration {
    // A wakeup cushion, not extra opening budget: remaining()/release() still
    // check the original end. Never extend an existing shorter socket policy.
    // Windows' native timeout resolution is separate from Instant resolution.
    #[cfg(windows)]
    let remaining = remaining.saturating_add(Duration::from_millis(1));
    baseline.map_or(remaining, |prior| prior.min(remaining))
}
impl Control {
    fn remaining(&self) -> io::Result<Duration> {
        let state = self.state.lock().map_err(|_| poisoned())?;
        if *state == State::Failed {
            return Err(io::Error::other("deadline socket has failed"));
        }
        if *state != State::Active {
            return Err(expired());
        }
        self.end
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(expired)
    }

    fn fail(&self, stream: &TcpStream) -> io::Result<()> {
        {
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            if *state != State::Expired {
                *state = State::Failed;
            }
        }
        let _ = stream.shutdown(Shutdown::Both);
        Ok(())
    }

    fn stop_worker(&self) -> io::Result<()> {
        if let Some(tx) = self.cancel.lock().map_err(|_| poisoned())?.take() {
            let _ = tx.send(());
        }
        let worker = self.worker.lock().map_err(|_| poisoned())?.take();
        if let Some(worker) = worker {
            worker
                .join()
                .map_err(|_| io::Error::other("deadline worker panicked"))?;
        }
        Ok(())
    }

    fn release(&self, stream: &TcpStream) -> io::Result<()> {
        self.release_with(stream, |socket, read, write| {
            socket.set_read_timeout(read)?;
            socket.set_write_timeout(write)
        })
    }

    fn release_with(
        &self,
        stream: &TcpStream,
        restore: impl FnOnce(&TcpStream, Option<Duration>, Option<Duration>) -> io::Result<()>,
    ) -> io::Result<()> {
        let released = {
            // This transition is the cancellation/expiry linearization point.
            // Once Released wins, joining/restoring may finish after `end`.
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            if *state != State::Active || Instant::now() >= self.end {
                if *state == State::Active {
                    *state = State::Expired;
                }
                false
            } else {
                *state = State::Released;
                true
            }
        };
        if !released {
            let _ = stream.shutdown(Shutdown::Both);
            return Err(expired());
        }
        let result = self
            .stop_worker()
            .and_then(|()| restore(stream, self.read_timeout, self.write_timeout));
        if result.is_err() {
            self.fail(stream)?;
        }
        result
    }

    fn expire(&self, stream: &TcpStream) -> io::Result<()> {
        {
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            if *state == State::Active {
                *state = State::Expired;
            }
        }
        let _ = stream.shutdown(Shutdown::Both);
        Ok(())
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        if let Some(tx) = self.cancel.get_mut().ok().and_then(Option::take) {
            let _ = tx.send(());
        }
        if let Some(worker) = self
            .worker
            .get_mut()
            .ok()
            .and_then(Option::take)
            .filter(|worker| worker.thread().id() != thread::current().id())
        {
            let _ = worker.join();
        }
    }
}

/// An exclusively owned stream bounded by one absolute handshake deadline.
/// Call `cancel` after a successful handshake to regain the stream.
/// Dropping it closes the owned socket; cancellation consumes it and returns
/// the live socket only when release wins before expiry. The timer and release
/// arbitrate one terminal state; successful release joins the timer and restores
/// both original timeouts fallibly. It cannot reopen an expired socket or cancel
/// arbitrary pending raw reads. The socket must be blocking, and callers must
/// not retain or use clones or change its shared socket options while it is guarded. Socket timeouts
/// are terminal and prevent the stream from being released.
pub struct Deadline {
    stream: Option<TcpStream>,
    control: Arc<Control>,
    split: bool,
}

/// Bound all I/O on `stream` by the same monotonic deadline.
pub fn deadline(stream: TcpStream, limit: Duration) -> io::Result<Deadline> {
    let end = Instant::now()
        .checked_add(limit)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "deadline overflow"))?;
    let read_timeout = stream.read_timeout()?;
    let write_timeout = stream.write_timeout()?;
    let socket = stream.try_clone()?;
    let (tx, rx) = mpsc::channel();
    let control = Arc::new(Control {
        end,
        state: Mutex::new(State::Active),
        cancel: Mutex::new(Some(tx)),
        worker: Mutex::new(None),
        read_timeout,
        write_timeout,
    });
    let weak = Arc::downgrade(&control);
    let worker = thread::Builder::new()
        .name("horizon-friends-deadline".into())
        .spawn(move || {
            if rx.recv_timeout(end.saturating_duration_since(Instant::now()))
                != Err(mpsc::RecvTimeoutError::Timeout)
            {
                return;
            }
            let Some(control) = weak.upgrade() else {
                return;
            };
            let should_shutdown = if let Ok(mut state) = control.state.lock() {
                if *state == State::Active {
                    *state = State::Expired;
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if should_shutdown {
                let _ = socket.shutdown(Shutdown::Both);
            }
        })?;
    *control.worker.lock().map_err(|_| poisoned())? = Some(worker);
    Ok(Deadline {
        stream: Some(stream),
        control,
        split: false,
    })
}

impl Deadline {
    /// Cancel the deadline, wait for its worker and restore original options.
    pub fn cancel(mut self) -> io::Result<TcpStream> {
        if self.split {
            return Err(io::Error::other("cannot release one deadline half"));
        }
        self.control
            .release(self.stream.as_ref().expect("owned stream"))?;
        Ok(self.stream.take().expect("owned stream"))
    }
    // Only secure_deadline uses these halves, sequentially, never concurrently.
    // Socket timeout options are shared; this is not a public clone facility.
    pub(crate) fn split(mut self) -> io::Result<(Self, Self)> {
        if self.split {
            return Err(io::Error::other("deadline already split"));
        }
        let stream = self.stream.as_ref().expect("owned stream").try_clone()?;
        let other = Self {
            stream: Some(stream),
            control: self.control.clone(),
            split: true,
        };
        self.split = true;
        Ok((other, self))
    }
    pub(crate) fn reunite(
        mut reader: Self,
        mut writer: Self,
    ) -> io::Result<(TcpStream, TcpStream)> {
        if !reader.split || !writer.split || !Arc::ptr_eq(&reader.control, &writer.control) {
            return Err(io::Error::other("unrelated deadline streams"));
        }
        reader
            .control
            .release(reader.stream.as_ref().expect("owned stream"))?;
        Ok((
            reader.stream.take().expect("owned stream"),
            writer.stream.take().expect("owned stream"),
        ))
    }
}

impl Deadline {
    fn read_op(
        &mut self,
        buf: &mut [u8],
        operation: impl FnOnce(&mut TcpStream, &mut [u8]) -> io::Result<usize>,
    ) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let remaining = match self.control.remaining() {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                self.control
                    .expire(self.stream.as_ref().expect("owned stream"))?;
                return Ok(0);
            }
            Err(error) => return Err(error),
        };
        let deadline_limited = self
            .control
            .read_timeout
            .is_none_or(|prior| remaining <= prior);
        let limit = socket_timeout(remaining, self.control.read_timeout);
        let stream = self.stream.as_mut().expect("owned stream");
        stream.set_read_timeout(Some(limit))?;
        match operation(stream, buf) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                if deadline_limited && Instant::now() >= self.control.end {
                    self.control.expire(stream)?;
                    Ok(0)
                } else {
                    self.control.fail(stream)?;
                    Err(error)
                }
            }
            result => result,
        }
    }

    fn write_op(
        &mut self,
        buf: &[u8],
        operation: impl FnOnce(&mut TcpStream, &[u8]) -> io::Result<usize>,
    ) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let remaining = self.control.remaining()?;
        let limit = socket_timeout(remaining, self.control.write_timeout);
        let stream = self.stream.as_mut().expect("owned stream");
        stream.set_write_timeout(Some(limit))?;
        match operation(stream, buf) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                if Instant::now() >= self.control.end {
                    self.control.expire(stream)?;
                    Err(expired())
                } else {
                    self.control.fail(stream)?;
                    Err(error)
                }
            }
            result => result,
        }
    }
}
impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_op(buf, |stream, bytes| stream.read(bytes))
    }
}
impl Write for Deadline {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_op(buf, |stream, bytes| stream.write(bytes))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.control.remaining()?;
        self.stream.as_mut().expect("owned stream").flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn split_half_cannot_cancel_or_split_again() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let (read, write) = deadline(stream, Duration::from_secs(2))
            .unwrap()
            .split()
            .unwrap();
        assert!(read.cancel().is_err());
        assert!(write.split().is_err());
    }

    #[test]
    fn interrupted_and_partial_io_keep_the_original_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut bounded = deadline(stream, Duration::from_secs(2)).unwrap();
        let mut buf = [0u8; 4];
        let mut first = Duration::ZERO;
        let error = bounded
            .read_op(&mut buf, |socket, _| {
                first = socket.read_timeout()?.unwrap();
                Err(io::ErrorKind::Interrupted.into())
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        std::thread::sleep(Duration::from_millis(30));
        let mut second = Duration::ZERO;
        let count = bounded
            .read_op(&mut buf, |socket, bytes| {
                second = socket.read_timeout()?.unwrap();
                bytes[0] = 9;
                Ok(1)
            })
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(buf[0], 9);
        assert!(second < first);
        let count = bounded
            .write_op(&[1, 2, 3, 4], |_, bytes| Ok(bytes.len() / 2))
            .unwrap();
        assert_eq!(count, 2);
        let error = bounded
            .write_op(&[3, 4], |_, _| Err(io::ErrorKind::TimedOut.into()))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(bounded.cancel().is_err());
    }

    #[test]
    fn expiry_timeout_stays_eof_for_subsequent_reads() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut bounded = deadline(stream, Duration::from_millis(50)).unwrap();
        assert_eq!(
            bounded
                .read_op(&mut [0], |_, _| {
                    thread::sleep(Duration::from_millis(70));
                    Err(io::ErrorKind::TimedOut.into())
                })
                .unwrap(),
            0
        );
        assert_eq!(bounded.read(&mut [0]).unwrap(), 0);
        assert_eq!(
            bounded.read_exact(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn released_winner_may_finish_join_after_expiry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let bounded = deadline(stream, Duration::from_millis(100)).unwrap();
        // Deterministically delay joining, not cancellation arbitration.
        bounded.control.stop_worker().unwrap();
        *bounded.control.worker.lock().unwrap() =
            Some(thread::spawn(|| thread::sleep(Duration::from_millis(150))));
        let mut stream = bounded.cancel().unwrap();
        peer.write_all(b"late").unwrap();
        let mut data = [0; 4];
        stream.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"late");
    }
    #[test]
    fn restoration_error_fails_closed_and_keeps_original_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let bounded = deadline(stream, Duration::from_secs(2)).unwrap();
        let error = bounded
            .control
            .release_with(bounded.stream.as_ref().unwrap(), |socket, _, _| {
                socket.set_read_timeout(Some(Duration::from_secs(1)))?;
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "restore write option",
                ))
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
        assert!(bounded.cancel().is_err());
    }

    #[test]
    fn zero_deadline_empty_buffers_and_read_exact_preserve_trait_semantics() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut bounded = deadline(stream, Duration::ZERO).unwrap();
        assert_eq!(bounded.read(&mut []).unwrap(), 0);
        assert_eq!(bounded.write(&[]).unwrap(), 0);
        assert_eq!(
            bounded.read_exact(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(
            bounded.write_all(b"x").unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn partial_read_exact_expires_without_read_ahead() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let (stream, _) = listener.accept().unwrap();
        peer.write_all(b"x").unwrap();
        let mut bounded = deadline(stream, Duration::from_millis(100)).unwrap();
        let started = Instant::now();
        let mut data = [0; 2];
        assert_eq!(
            bounded.read_exact(&mut data).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(data[0], b'x');
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn genuine_errors_and_success_crossing_expiry_do_not_renew_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut bounded = deadline(stream, Duration::from_millis(100)).unwrap();
        assert_eq!(
            bounded
                .read_op(&mut [0], |_, _| Err(io::ErrorKind::ConnectionReset.into()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::ConnectionReset
        );
        assert_eq!(
            bounded
                .write_op(b"x", |_, _| Err(io::ErrorKind::Interrupted.into()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        assert_eq!(
            bounded
                .read_op(&mut [0], |_, bytes| {
                    thread::sleep(Duration::from_millis(120));
                    bytes[0] = 1;
                    Ok(1)
                })
                .unwrap(),
            1
        );
        assert_eq!(bounded.read(&mut [0]).unwrap(), 0);
        assert!(bounded.cancel().is_err());
    }
    #[test]
    fn native_timeout_readback_does_not_shorten_absolute_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut bounded = deadline(stream, Duration::from_secs(2)).unwrap();
        let end = bounded.control.end;
        bounded
            .read_op(&mut [0], |socket, _| {
                let configured = socket.read_timeout()?.unwrap();
                let remaining = end.saturating_duration_since(Instant::now());
                assert!(
                    configured >= remaining,
                    "native timeout {configured:?} shortened {remaining:?}"
                );
                Ok(0)
            })
            .unwrap();
    }
    #[test]
    fn line_adapter_does_not_read_ahead_into_handed_off_payload() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        peer.write_all(b"hello\npayload").unwrap();
        let mut bounded = deadline(stream, Duration::from_secs(2)).unwrap();
        assert_eq!(
            crate::wire::read_line(&mut bounded).unwrap().unwrap(),
            b"hello"
        );
        let mut stream = bounded.cancel().unwrap();
        let mut data = [0; 7];
        stream.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"payload");
    }
    #[cfg(windows)]
    #[test]
    fn windows_wakeup_cushion_is_bounded_and_does_not_extend_baseline() {
        let remaining = Duration::from_micros(10_500);
        assert_eq!(
            socket_timeout(remaining, None),
            Duration::from_micros(11_500)
        );
        assert_eq!(
            socket_timeout(remaining, Some(Duration::from_millis(5))),
            Duration::from_millis(5)
        );
        assert_eq!(
            socket_timeout(remaining, Some(Duration::from_millis(11))),
            Duration::from_millis(11)
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(socket_timeout(remaining, None)))
            .unwrap();
        // std rounds the native millisecond option upward too.
        assert_eq!(
            stream.read_timeout().unwrap(),
            Some(Duration::from_millis(12))
        );
        let mut bounded = deadline(stream, Duration::from_secs(2)).unwrap();
        let end = bounded.control.end;
        assert_eq!(
            bounded
                .read_op(&mut [0], |_, _| Err(io::ErrorKind::TimedOut.into()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert!(Instant::now() < end);
        assert!(bounded.cancel().is_err());
    }
}
