use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// One observer's generations, not a count of individual delivered callbacks.
pub(super) struct Pending {
    flag: Arc<AtomicBool>,
    generation: u32,
}

impl Pending {
    pub(super) fn new(flag: Arc<AtomicBool>) -> Self {
        Self {
            flag,
            generation: 0,
        }
    }

    pub(super) fn observe(&mut self) -> u32 {
        if self.flag.swap(false, Ordering::SeqCst) {
            // Retain the original private counter's defined wrapping behavior.
            self.generation = self.generation.wrapping_add(1);
        }
        self.generation
    }
}

/// The caller owns a blocked-mask guard captured by `restore`. Unwinding drops
/// that guard; ordinary errors explicitly restore before returning any owner.
pub(super) fn publish(
    signals: &[i32],
    mut register: impl FnMut(i32, Arc<AtomicBool>) -> Result<(), String>,
    restore: impl FnOnce() -> Result<(), String>,
) -> Result<Pending, String> {
    let flag = Arc::new(AtomicBool::new(false));
    let registration = signals
        .iter()
        .try_for_each(|signal| register(*signal, Arc::clone(&flag)));
    let restoration = restore();
    match (registration, restoration) {
        (Ok(()), Ok(())) => Ok(Pending::new(flag)),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(registration), Err(restoration)) => {
            Err(format!("{registration}; additionally, {restoration}"))
        }
    }
}

#[cfg(test)]
#[test]
fn generation_wrap_is_defined_like_the_original_private_counter() {
    let flag = Arc::new(AtomicBool::new(true));
    let mut requests = Pending::new(flag);
    requests.generation = u32::MAX;
    assert_eq!(requests.observe(), 0);
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
