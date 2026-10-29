//! Beyond-owned exact logical allocation accounting. See FORK.md.
#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

/// First accounting failure; reporting remains failed for this allocator's life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountingError {
    /// A checked counter exceeded its representable range.
    Overflow,
    /// A removal exceeded recorded live state.
    Underflow,
}

impl std::fmt::Display for AccountingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Overflow => "allocation accounting overflow",
            Self::Underflow => "allocation accounting underflow",
        })
    }
}

impl std::error::Error for AccountingError {}

/// One serialized view of logical requested heap activity, not RSS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Successful allocations, zeroed allocations and reallocations.
    pub alloc_count: u64,
    /// Allocated bytes plus positive realloc growth, since construction.
    pub total_bytes: u64,
    /// Highest logical requested bytes in this window, including its baseline.
    pub peak_bytes: u64,
    /// Outstanding requested bytes.
    pub current_bytes: u64,
    /// Outstanding allocation count.
    pub live_count: u64,
    /// Highest outstanding count in this window.
    pub peak_live_count: u64,
}

/// Const-initialized serialized bookkeeping behind the frozen upstream adapter.
///
/// State and allocator-hook work are allocation-free and bounded. Lock wait time
/// is not bounded: preemption/starvation and asynchronous same-thread allocator
/// reentry are unsupported. No backend call, TLS, callback or formatting occurs
/// under the lock. See FORK.md for the logical linearization semantics.
pub struct ModAlloc {
    locked: AtomicBool,
    error: AtomicU8,
    alloc_count: AtomicU64,
    total_bytes: AtomicU64,
    peak_bytes: AtomicU64,
    current_bytes: AtomicU64,
    live_count: AtomicU64,
    peak_live_count: AtomicU64,
}

struct Lock<'a>(&'a AtomicBool);

impl Drop for Lock<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

enum Event {
    Alloc(u64),
    Dealloc(u64),
    Realloc(u64, u64),
}

impl ModAlloc {
    /// Start at zero before the first allocator operation, including before main.
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
            error: AtomicU8::new(0),
            alloc_count: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
            peak_bytes: AtomicU64::new(0),
            current_bytes: AtomicU64::new(0),
            live_count: AtomicU64::new(0),
            peak_live_count: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> Lock<'_> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
        Lock(&self.locked)
    }

    // Caller holds the sole lock; no accessor exposes the relaxed fields.
    fn read_locked(&self) -> Snapshot {
        Snapshot {
            alloc_count: self.alloc_count.load(Ordering::Relaxed),
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            peak_bytes: self.peak_bytes.load(Ordering::Relaxed),
            current_bytes: self.current_bytes.load(Ordering::Relaxed),
            live_count: self.live_count.load(Ordering::Relaxed),
            peak_live_count: self.peak_live_count.load(Ordering::Relaxed),
        }
    }

    fn status_locked(&self) -> Result<(), AccountingError> {
        match self.error.load(Ordering::Relaxed) {
            0 => Ok(()),
            1 => Err(AccountingError::Overflow),
            _ => Err(AccountingError::Underflow),
        }
    }

    /// Read every counter at one logical record point, or the sticky first error.
    pub fn try_snapshot(&self) -> Result<Snapshot, AccountingError> {
        let _lock = self.lock();
        self.status_locked()?;
        Ok(self.read_locked())
    }

    /// Capture the completed window and begin the next at the SAME record point.
    /// Current and cumulative counters are never cleared. Peaks start at current.
    pub fn finish_window(&self) -> Result<Snapshot, AccountingError> {
        let _lock = self.lock();
        self.status_locked()?;
        let captured = self.read_locked();
        self.peak_bytes
            .store(captured.current_bytes, Ordering::Relaxed);
        self.peak_live_count
            .store(captured.live_count, Ordering::Relaxed);
        Ok(captured)
    }

    fn record(&self, event: Event) {
        let _lock = self.lock();
        if self.status_locked().is_err() {
            return;
        }
        // Compute the WHOLE next state before committing any field. On error the
        // first failure latches; later operations never modify counters again.
        match self.read_locked().next(event) {
            Ok(s) => {
                self.alloc_count.store(s.alloc_count, Ordering::Relaxed);
                self.total_bytes.store(s.total_bytes, Ordering::Relaxed);
                self.peak_bytes.store(s.peak_bytes, Ordering::Relaxed);
                self.current_bytes.store(s.current_bytes, Ordering::Relaxed);
                self.live_count.store(s.live_count, Ordering::Relaxed);
                self.peak_live_count
                    .store(s.peak_live_count, Ordering::Relaxed);
            }
            Err(e) => self.error.store(
                match e {
                    AccountingError::Overflow => 1,
                    AccountingError::Underflow => 2,
                },
                Ordering::Relaxed,
            ),
        }
    }

    pub(crate) fn record_alloc(&self, size: u64) {
        self.record(Event::Alloc(size));
    }
    pub(crate) fn record_dealloc(&self, size: u64) {
        self.record(Event::Dealloc(size));
    }
    pub(crate) fn record_realloc(&self, old: u64, new: u64) {
        self.record(Event::Realloc(old, new));
    }
    pub(crate) fn register_self(&self) { /* Exact reporting never publishes a raw handle. */
    }
}

impl Default for ModAlloc {
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot {
    fn next(mut self, event: Event) -> Result<Self, AccountingError> {
        use AccountingError::{Overflow, Underflow};
        match event {
            Event::Alloc(size) => {
                self.alloc_count = self.alloc_count.checked_add(1).ok_or(Overflow)?;
                self.total_bytes = self.total_bytes.checked_add(size).ok_or(Overflow)?;
                self.current_bytes = self.current_bytes.checked_add(size).ok_or(Overflow)?;
                self.live_count = self.live_count.checked_add(1).ok_or(Overflow)?;
            }
            Event::Dealloc(size) => {
                self.current_bytes = self.current_bytes.checked_sub(size).ok_or(Underflow)?;
                self.live_count = self.live_count.checked_sub(1).ok_or(Underflow)?;
            }
            Event::Realloc(old, new) => {
                if self.live_count == 0 || old > self.current_bytes {
                    return Err(Underflow);
                }
                self.alloc_count = self.alloc_count.checked_add(1).ok_or(Overflow)?;
                self.current_bytes = self
                    .current_bytes
                    .checked_sub(old)
                    .ok_or(Underflow)?
                    .checked_add(new)
                    .ok_or(Overflow)?;
                if new > old {
                    self.total_bytes = self.total_bytes.checked_add(new - old).ok_or(Overflow)?;
                }
            }
        }
        self.peak_bytes = self.peak_bytes.max(self.current_bytes);
        self.peak_live_count = self.peak_live_count.max(self.live_count);
        Ok(self)
    }
}

// The frozen unsafe adapter expects this guard. Exact hooks cannot allocate or
// reenter themselves, so no event is omitted for TLS initialization/destruction.
pub(crate) struct ReentryGuard;

impl ReentryGuard {
    pub(crate) fn enter() -> Option<Self> {
        Some(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Missing success accounting, destructive reset, unsigned delta and
    // old-plus-new realloc peaks each violate these literal expectations.
    #[test]
    fn retained_baseline_negative_delta_and_lower_next_peak() {
        let a = ModAlloc::new();
        a.record_alloc(100);
        assert_eq!(a.finish_window().unwrap().current_bytes, 100);
        a.record_alloc(200);
        a.record_dealloc(200);
        let first = a.finish_window().unwrap();
        assert_eq!(first.peak_bytes, 300);
        assert_eq!(first.current_bytes, 100);
        a.record_dealloc(100);
        let second = a.finish_window().unwrap();
        assert_eq!(second.peak_bytes, 100);
        assert_eq!(
            i128::from(second.current_bytes) - i128::from(first.current_bytes),
            -100
        );
        assert_eq!(second.alloc_count, 2);
        assert_eq!(second.total_bytes, 300);
        assert_eq!(second.live_count, 0);
        assert_eq!(second.peak_live_count, 1);
    }

    #[test]
    fn logical_realloc_is_one_event_and_never_old_plus_new_peak() {
        let a = ModAlloc::new();
        a.record_alloc(64);
        a.record_realloc(64, 128);
        a.record_realloc(128, 32);
        a.record_realloc(32, 32);
        assert_eq!(
            a.try_snapshot().unwrap(),
            Snapshot {
                alloc_count: 4,
                total_bytes: 128,
                peak_bytes: 128,
                current_bytes: 32,
                live_count: 1,
                peak_live_count: 1,
            }
        );
        a.record_dealloc(32);
        assert_eq!(a.try_snapshot().unwrap().current_bytes, 0);
    }

    #[test]
    fn overflow_and_underflow_are_sticky_not_successful_stale_reports() {
        let a = ModAlloc::new();
        a.record_alloc(u64::MAX);
        a.record_alloc(1);
        assert_eq!(a.try_snapshot(), Err(AccountingError::Overflow));
        a.record_dealloc(u64::MAX);
        assert_eq!(a.finish_window(), Err(AccountingError::Overflow));
        let b = ModAlloc::new();
        b.record_dealloc(1);
        assert_eq!(b.try_snapshot(), Err(AccountingError::Underflow));
        b.record_alloc(1);
        assert_eq!(b.finish_window(), Err(AccountingError::Underflow));
    }

    #[test]
    fn failed_transaction_never_commits_even_earlier_checked_fields() {
        let a = ModAlloc::new();
        a.record_alloc(u64::MAX);
        let before = a.read_locked(); // Local allocator is exclusively test-owned.
        a.record_realloc(u64::MAX, u64::MAX);
        let before_overflow = a.read_locked();
        assert_eq!(before_overflow.alloc_count, before.alloc_count + 1);
        a.record_alloc(1); // alloc_count could advance, total_bytes cannot.
        assert_eq!(a.read_locked(), before_overflow);
        assert_eq!(a.try_snapshot(), Err(AccountingError::Overflow));
        a.record_dealloc(u64::MAX);
        assert_eq!(a.read_locked(), before_overflow);

        let b = ModAlloc::new();
        b.record_alloc(8);
        let before = b.read_locked();
        b.record_realloc(9, 1);
        assert_eq!(b.try_snapshot(), Err(AccountingError::Underflow));
        assert_eq!(b.read_locked(), before);
    }

    #[test]
    fn concurrent_snapshots_and_finish_are_coherent_with_all_event_records() {
        let a = ModAlloc::new();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..2000 {
                        a.record_alloc(8);
                        a.record_dealloc(8);
                    }
                });
            }
            for _ in 0..2 {
                scope.spawn(|| {
                    for _ in 0..2000 {
                        let s = a.finish_window().unwrap();
                        assert_eq!(s.current_bytes, s.live_count * 8);
                        assert_eq!(s.total_bytes, s.alloc_count * 8);
                        assert!(s.peak_bytes >= s.current_bytes);
                        assert!(s.peak_live_count >= s.live_count);
                        let s = a.try_snapshot().unwrap();
                        assert_eq!(s.current_bytes, s.live_count * 8);
                        assert_eq!(s.total_bytes, s.alloc_count * 8);
                    }
                });
            }
        });
        let s = a.try_snapshot().unwrap();
        assert_eq!(
            (s.alloc_count, s.total_bytes, s.current_bytes, s.live_count),
            (8000, 64000, 0, 0)
        );
    }

    #[test]
    fn every_two_pair_interleaving_preserves_window_baseline() {
        // All six total orders preserving each pair's alloc-before-free order.
        for schedule in [
            [0, 1, 2, 3],
            [0, 2, 1, 3],
            [0, 2, 3, 1],
            [2, 0, 1, 3],
            [2, 0, 3, 1],
            [2, 3, 0, 1],
        ] {
            let a = ModAlloc::new();
            for event in schedule {
                match event {
                    0 => a.record_alloc(8),
                    1 => a.record_dealloc(8),
                    2 => a.record_alloc(8),
                    _ => a.record_dealloc(8),
                }
                let s = a.finish_window().unwrap();
                let next = a.try_snapshot().unwrap();
                assert_eq!(next.peak_bytes, s.current_bytes);
                assert_eq!(next.peak_live_count, s.live_count);
                assert_eq!(next.current_bytes, s.current_bytes);
                assert!(s.peak_bytes >= s.current_bytes);
            }
            let s = a.try_snapshot().unwrap();
            assert_eq!(
                (s.alloc_count, s.total_bytes, s.current_bytes, s.live_count),
                (2, 16, 0, 0)
            );
        }
    }
}
