//! Stop requests from outside: Ctrl-C in the terminal, SIGTERM from systemd or launchd, SIGHUP when the
//! terminal closes, the console window closing on Windows. The handler only counts them; the supervisor
//! sees the count and stops the game cleanly (a second request stops it at once).

use std::sync::atomic::{AtomicU32, Ordering};

static REQUESTS: AtomicU32 = AtomicU32::new(0);

/// How many stop requests arrived.
pub fn requests() -> u32 {
    REQUESTS.load(Ordering::SeqCst)
}

#[cfg(unix)]
pub fn install() {
    extern "C" fn handle(_: libc::c_int) {
        REQUESTS.fetch_add(1, Ordering::SeqCst);
    }
    let handler = handle as extern "C" fn(libc::c_int);
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: the handler only increments an atomic, which is async-signal-safe.
        unsafe {
            libc::signal(signal, handler as libc::sighandler_t);
        }
    }
}

#[cfg(windows)]
pub(super) extern "system" fn handle(_: u32) -> i32 {
    REQUESTS.fetch_add(1, Ordering::SeqCst);
    1
}

#[cfg(windows)]
pub fn install() {
    // The static callback counts every event synchronously, including unknown event codes.
    let _ = winconsole::console::set_ctrl_handler(Some(handle), true);
}

#[cfg(not(any(unix, windows)))]
pub fn install() {}
