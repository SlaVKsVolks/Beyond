//! Real System-backed allocator coverage; run alone in a fresh process.
#![forbid(unsafe_code)]

use mod_alloc::ModAlloc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[global_allocator]
static ALLOC: ModAlloc = ModAlloc::new();

static TLS_ALLOC_SEEN: AtomicBool = AtomicBool::new(false);
static TLS_FREE_SEEN: AtomicBool = AtomicBool::new(false);
static FIRST_DROPPED: AtomicBool = AtomicBool::new(false);
static THREAD_BASE: AtomicU64 = AtomicU64::new(0);

struct Late;
struct First;

thread_local! {
    static LATE: Late = const { Late };
    static FIRST: First = const { First };
}

impl Drop for First {
    fn drop(&mut self) {
        FIRST_DROPPED.store(true, Ordering::SeqCst);
    }
}

impl Drop for Late {
    fn drop(&mut self) {
        // This destructor executes after another TLS destructor has completed.
        let before = ALLOC.try_snapshot().unwrap();
        let b = std::hint::black_box(Box::new([7_u8; 8192]));
        let during = ALLOC.try_snapshot().unwrap();
        TLS_ALLOC_SEEN.store(
            FIRST_DROPPED.load(Ordering::SeqCst)
                && during.current_bytes == before.current_bytes + 8192
                && during.alloc_count == before.alloc_count + 1,
            Ordering::SeqCst,
        );
        std::hint::black_box(&b);
        drop(b);
        TLS_FREE_SEEN.store(
            ALLOC.try_snapshot().unwrap().current_bytes == before.current_bytes,
            Ordering::SeqCst,
        );
    }
}

#[test]
fn actual_system_success_cross_thread_and_tls_destructor_accounting() {
    let before = ALLOC.finish_window().unwrap();
    let mut v = Vec::<u8>::new();
    v.try_reserve_exact(64).unwrap();
    v.resize(64, 23);
    let a = ALLOC.try_snapshot().unwrap();
    assert_eq!(a.current_bytes - before.current_bytes, v.capacity() as u64);
    v.try_reserve_exact(128).unwrap();
    assert!(v[..64].iter().all(|&b| b == 23));
    let b = ALLOC.try_snapshot().unwrap();
    assert_eq!(b.current_bytes - before.current_bytes, v.capacity() as u64);
    assert_eq!(b.alloc_count, before.alloc_count + 2);
    assert_eq!(b.peak_bytes, b.current_bytes.max(before.current_bytes));
    v.truncate(16);
    v.shrink_to_fit();
    assert_eq!(v.as_slice(), &[23; 16]);
    assert_eq!(
        ALLOC.try_snapshot().unwrap().current_bytes - before.current_bytes,
        v.capacity() as u64
    );
    drop(v);
    assert_eq!(
        ALLOC.try_snapshot().unwrap().current_bytes,
        before.current_bytes
    );

    let before = ALLOC.try_snapshot().unwrap();
    let z = bytemuck::allocation::try_zeroed_box::<[u8; 128]>().unwrap();
    assert_eq!(*z, [0; 128]);
    assert_eq!(
        ALLOC.try_snapshot().unwrap().current_bytes,
        before.current_bytes + 128
    );
    drop(z);
    assert_eq!(
        ALLOC.try_snapshot().unwrap().current_bytes,
        before.current_bytes
    );

    // Spawn setup traffic is measured, not assumed zero; capture inside thread.
    let transferred = Box::new([5_u8; 4096]);
    std::thread::spawn(move || {
        let before = ALLOC.try_snapshot().unwrap();
        drop(transferred);
        assert_eq!(
            ALLOC.try_snapshot().unwrap().current_bytes,
            before.current_bytes - 4096
        );
        LATE.with(|_| {});
        FIRST.with(|_| {});
        THREAD_BASE.store(before.current_bytes, Ordering::SeqCst);
    })
    .join()
    .unwrap();
    assert!(THREAD_BASE.load(Ordering::SeqCst) > 0);
    assert!(TLS_ALLOC_SEEN.load(Ordering::SeqCst));
    assert!(TLS_FREE_SEEN.load(Ordering::SeqCst));

    // This is actual System traffic through the frozen adapter, not synthetic
    // record calls. Warm one identical round before measuring runtime overhead.
    let round = || {
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..2000 {
                        let b = std::hint::black_box(Box::new([11_u8; 64]));
                        std::hint::black_box(b.as_slice());
                        drop(b);
                    }
                });
            }
            scope.spawn(|| {
                for _ in 0..2000 {
                    let s = ALLOC.finish_window().unwrap();
                    assert!(s.peak_bytes >= s.current_bytes);
                    assert!(s.peak_live_count >= s.live_count);
                }
            });
        });
    };
    round();
    let before = ALLOC.try_snapshot().unwrap();
    round();
    let after = ALLOC.try_snapshot().unwrap();
    assert_eq!(after.current_bytes, before.current_bytes);
    assert_eq!(after.live_count, before.live_count);
    assert!(after.alloc_count >= before.alloc_count + 8000);
    assert!(after.total_bytes >= before.total_bytes + 512000);
}
