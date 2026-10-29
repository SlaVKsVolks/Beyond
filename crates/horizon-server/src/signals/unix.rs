use super::pending::{self, Pending};
use nix::sys::signal::{SigSet, SigmaskHow, Signal};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

static INSTALLATION_STARTED: AtomicBool = AtomicBool::new(false);
const SIGNALS: [Signal; 3] = [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP];

struct MaskGuard {
    previous: Option<SigSet>,
    _thread: PhantomData<Rc<()>>,
}

impl MaskGuard {
    fn block() -> Result<Self, String> {
        let mut blocked = SigSet::empty();
        for signal in SIGNALS {
            blocked.add(signal);
        }
        let previous = blocked
            .thread_swap_mask(SigmaskHow::SIG_BLOCK)
            .map_err(|error| {
                format!("could not block stop signals during registration: {error}")
            })?;
        Ok(Self {
            previous: Some(previous),
            _thread: PhantomData,
        })
    }

    fn restore(mut self) -> Result<(), String> {
        // Consume even on failure: Drop is not a hidden retry after an explicit error.
        let previous = self
            .previous
            .take()
            .ok_or("stop signal mask already restored")?;
        previous
            .thread_set_mask()
            .map_err(|error| format!("could not restore the previous signal mask: {error}"))
    }
}

impl Drop for MaskGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            // Unwinding fallback only; normal paths use fallible restore above.
            let _ = previous.thread_set_mask();
        }
    }
}

pub(super) fn install(
    register: impl FnMut(i32, Arc<AtomicBool>) -> Result<(), String>,
) -> Result<Pending, String> {
    // Partial registration is not guaranteed to roll back. Never retry or re-enter.
    if INSTALLATION_STARTED.swap(true, Ordering::SeqCst) {
        return Err("stop signal installation was already attempted".into());
    }
    let mask = MaskGuard::block()?;
    let signals = SIGNALS.map(|signal| signal as i32);
    pending::publish(&signals, register, move || mask.restore())
}
