#![forbid(unsafe_code)]

use alloc_metrics::{Metrics, global_metrics};
use std::{
    hint::black_box,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[path = "../examples/support/model_bench_allocator.rs"]
mod model_bench_allocator;

const TRACK: bool = option_env!("HORIZON_BENCH_COUNT_ALLOCATIONS").is_some();

#[path = "support/model_allocator_normal.rs"]
mod model_allocator_normal;

fn expect_delta(base: Metrics, bytes: usize, allocations: isize) {
    let expected = if TRACK {
        Metrics {
            allocated_bytes: base.allocated_bytes + isize::try_from(bytes).unwrap(),
            allocations: base.allocations + allocations,
        }
    } else {
        base
    };
    assert_eq!(global_metrics(), expected);
    assert_eq!(
        model_bench_allocator::live_bytes(),
        if TRACK {
            usize::try_from(expected.allocated_bytes).unwrap()
        } else {
            0
        }
    );
}

fn wait_for(state: &AtomicU8, expected: u8) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while state.load(Ordering::Acquire) != expected {
        assert!(
            Instant::now() < deadline,
            "bounded synchronization timed out"
        );
        thread::yield_now();
    }
}

fn main() {
    model_allocator_normal::exercise(TRACK);
    let initial = global_metrics();
    let initial_live = model_bench_allocator::live_bytes();
    assert_eq!(
        initial_live,
        if TRACK {
            usize::try_from(initial.allocated_bytes).unwrap()
        } else {
            0
        }
    );
    let retained_data = vec![5u8; 41];
    expect_delta(initial, retained_data.capacity(), 1);
    let retained =
        model_bench_allocator::retained_delta(model_bench_allocator::live_bytes(), initial_live);
    let block_retained = retained;
    assert_eq!(retained, if TRACK { 41 } else { 0 });
    black_box(&retained_data);
    let before_drop = model_bench_allocator::live_bytes();
    drop(retained_data);
    assert_eq!(
        model_bench_allocator::retained_delta(model_bench_allocator::live_bytes(), before_drop),
        if TRACK { -41 } else { 0 }
    );
    let deltas: [(usize, usize, i128); 5] = [
        (0, 0, 0),
        (41, 0, 41),
        (0, 41, -41),
        (usize::MAX, usize::MAX, 0),
        (usize::MAX - 1, usize::MAX, -1),
    ];
    for (current, initial, expected) in deltas {
        assert_eq!(
            model_bench_allocator::retained_delta(current, initial),
            expected
        );
    }
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(
            model_bench_allocator::retained_delta(usize::MAX, 0),
            18_446_744_073_709_551_615i128
        );
        assert_eq!(
            model_bench_allocator::retained_delta(0, usize::MAX),
            -18_446_744_073_709_551_615i128
        );
    }
    #[cfg(target_pointer_width = "32")]
    {
        assert_eq!(
            model_bench_allocator::retained_delta(usize::MAX, 0),
            4_294_967_295i128
        );
        assert_eq!(
            model_bench_allocator::retained_delta(0, usize::MAX),
            -4_294_967_295i128
        );
    }
    assert_eq!(global_metrics(), initial);
    assert_eq!(model_bench_allocator::live_bytes(), initial_live);
    // Thread/Arc/slot initialization and TLS setup precede the measurement.
    let state = Arc::new(AtomicU8::new(0));
    let slot = Arc::new(Mutex::new(None::<Vec<u8>>));
    let worker_state = Arc::clone(&state);
    let worker_slot = Arc::clone(&slot);
    black_box(thread::current());
    let worker = thread::spawn(move || {
        black_box(thread::current());
        worker_state.store(1, Ordering::Release);
        wait_for(&worker_state, 2);
        let data = vec![19u8; 113];
        *worker_slot.lock().unwrap() = Some(data);
        worker_state.store(3, Ordering::Release);
        wait_for(&worker_state, 4);
        let data = worker_slot.lock().unwrap().take().unwrap();
        assert_eq!(data.as_slice(), &[23; 79]);
        drop(data);
        worker_state.store(5, Ordering::Release);
        // Stay alive with every setup owner retained through final baseline.
        wait_for(&worker_state, 6);
        drop((worker_slot, worker_state));
    });
    wait_for(&state, 1);
    let thread_baseline = global_metrics();
    state.store(2, Ordering::Release);
    wait_for(&state, 3);
    let from_worker = slot.lock().unwrap().take().unwrap();
    expect_delta(thread_baseline, from_worker.capacity(), 1);
    assert_eq!(from_worker.as_slice(), &[19; 113]);
    black_box(&from_worker);
    drop(from_worker);
    assert_eq!(global_metrics(), thread_baseline);
    let from_main = vec![23u8; 79];
    expect_delta(thread_baseline, from_main.capacity(), 1);
    *slot.lock().unwrap() = Some(from_main);
    state.store(4, Ordering::Release);
    wait_for(&state, 5);
    assert_eq!(global_metrics(), thread_baseline);
    state.store(6, Ordering::Release);
    worker.join().unwrap();
    drop((slot, state));
    // An independent external expectation catches build-cfg/env propagation errors.
    if let Some(expected) = std::env::args().nth(1) {
        assert!(matches!(expected.as_str(), "present" | "absent"));
        assert_eq!(TRACK, expected == "present");
    }
    println!(
        "mode PASS compile_env={:?} tracking={} retained_bytes={retained} block_retained_bytes={block_retained}",
        option_env!("HORIZON_BENCH_COUNT_ALLOCATIONS"),
        TRACK
    );
    println!(
        "system PASS initial_bytes={} initial_allocations={} thread_baseline_bytes={} thread_baseline_allocations={} first_controlled=verified_for_mode cross_thread=both_directions final_baseline=worker_alive",
        initial.allocated_bytes,
        initial.allocations,
        thread_baseline.allocated_bytes,
        thread_baseline.allocations
    );
}
