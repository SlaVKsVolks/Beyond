//! Stop requests from outside: Ctrl-C in the terminal, SIGTERM from systemd or launchd, SIGHUP when the
//! terminal closes, the console window closing on Windows. The supervisor observes requests and stops
//! the game cleanly; a later observed request stops it at once. Unix bursts between observations
//! coalesce, as they already do in the supervisor's stop policy; Windows callbacks retain their count.

use std::{marker::PhantomData, rc::Rc};

#[cfg(not(unix))]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(not(unix))]
static REQUESTS: AtomicU32 = AtomicU32::new(0);

#[cfg(unix)]
#[path = "signals/pending.rs"]
mod pending;

#[cfg(unix)]
#[path = "signals/unix.rs"]
mod unix;

/// The sole main-thread observer. It cannot be cloned, sent or shared across threads.
pub struct StopRequests {
    #[cfg(unix)]
    pending: pending::Pending,
    _thread: PhantomData<Rc<()>>,
}

impl StopRequests {
    pub fn requests(&mut self) -> u32 {
        #[cfg(unix)]
        {
            self.pending.observe()
        }
        #[cfg(not(unix))]
        {
            requests()
        }
    }
}

/// Install before spawning workers or children. An installation error stops startup.
pub fn observer() -> Result<StopRequests, String> {
    #[cfg(unix)]
    {
        observer_with(|signal, flag| {
            signal_hook::flag::register(signal, flag)
                .map(|_| ())
                .map_err(|error| format!("could not register stop signal {signal}: {error}"))
        })
    }
    #[cfg(not(unix))]
    {
        install();
        Ok(StopRequests {
            _thread: PhantomData,
        })
    }
}

// The registrar boundary also permits owned native tests to deliver a targeted
// signal while the actual production mask guard is held, never to the host.
#[cfg(unix)]
pub(crate) fn observer_with(
    register: impl FnMut(i32, std::sync::Arc<std::sync::atomic::AtomicBool>) -> Result<(), String>,
) -> Result<StopRequests, String> {
    Ok(StopRequests {
        pending: unix::install(register)?,
        _thread: PhantomData,
    })
}

/// How many stop requests arrived.
#[cfg(not(unix))]
pub fn requests() -> u32 {
    REQUESTS.load(Ordering::SeqCst)
}

#[cfg(windows)]
pub(super) extern "system" fn handle(_: u32) -> i32 {
    REQUESTS.fetch_add(1, Ordering::SeqCst);
    1
}

#[cfg(windows)]
pub fn install() {
    // The published safe API owns the native registration boundary internally.
    let _ = winconsole::console::set_ctrl_handler(Some(handle), true);
}

#[cfg(not(any(unix, windows)))]
pub fn install() {}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn every_callback_counts_synchronously_and_reports_handled() {
        let before = super::requests();
        for (index, event) in [0, 1, 2, 5, 6, u32::MAX, 0].into_iter().enumerate() {
            assert_eq!(super::handle(event), 1);
            assert_eq!(super::requests(), before + index as u32 + 1);
        }
    }
}
