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
            *state = State::Failed;
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
        {
            let mut state = self.state.lock().map_err(|_| poisoned())?;
            if *state != State::Active || Instant::now() >= self.end {
                *state = State::Expired;
                return Err(expired());
            }
            *state = State::Released;
        }
        self.stop_worker()?;
        stream.set_read_timeout(self.read_timeout)?;
        stream.set_write_timeout(self.write_timeout)?;
        if Instant::now() >= self.end {
            return Err(expired());
        }
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
/// The socket must be blocking, and callers must not retain or use clones
/// or change its shared socket options while it is guarded. Socket timeouts
/// are terminal and prevent the stream from being released.
pub struct Deadline {
    stream: Option<TcpStream>,
    control: Arc<Control>,
    read_limit: Option<Duration>,
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
        read_limit: read_timeout,
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
    pub(crate) fn limit_read_timeout(&mut self, limit: Duration) {
        self.read_limit = Some(self.read_limit.map_or(limit, |prior| prior.min(limit)));
    }
    pub(crate) fn split(mut self) -> io::Result<(Self, Self)> {
        if self.split {
            return Err(io::Error::other("deadline already split"));
        }
        let stream = self.stream.as_ref().expect("owned stream").try_clone()?;
        let other = Self {
            stream: Some(stream),
            control: self.control.clone(),
            read_limit: self.read_limit,
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
            Err(error) if error.kind() == io::ErrorKind::TimedOut => return Ok(0),
            Err(error) => return Err(error),
        };
        let deadline_limited = self.read_limit.is_none_or(|prior| remaining <= prior);
        let limit = self
            .read_limit
            .map_or(remaining, |prior| prior.min(remaining));
        let stream = self.stream.as_mut().expect("owned stream");
        stream.set_read_timeout(Some(limit))?;
        match operation(stream, buf) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                self.control.fail(stream)?;
                if deadline_limited && Instant::now() >= self.control.end {
                    Ok(0)
                } else {
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
        let limit = self
            .control
            .write_timeout
            .map_or(remaining, |prior| prior.min(remaining));
        let stream = self.stream.as_mut().expect("owned stream");
        stream.set_write_timeout(Some(limit))?;
        match operation(stream, buf) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                self.control.fail(stream)?;
                if Instant::now() >= self.control.end {
                    Err(expired())
                } else {
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
}
