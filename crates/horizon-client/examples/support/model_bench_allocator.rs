//! Compile-time optional requested-byte accounting for the model benchmark.
#![forbid(unsafe_code)]

use std::alloc::System;

#[cfg(horizon_bench_count_allocations)]
#[global_allocator]
static ALLOCATOR: alloc_metrics::MetricAlloc<System> = alloc_metrics::MetricAlloc::new(System);

#[cfg(not(horizon_bench_count_allocations))]
#[global_allocator]
static ALLOCATOR: System = System;

pub fn live_bytes() -> usize {
    #[cfg(horizon_bench_count_allocations)]
    {
        usize::try_from(alloc_metrics::global_metrics().allocated_bytes)
            .expect("global requested allocation bytes must be nonnegative")
    }
    #[cfg(not(horizon_bench_count_allocations))]
    {
        0
    }
}

/// Difference used by the model benchmark's retained-byte fields.
pub fn retained_delta(current: usize, initial: usize) -> i128 {
    current as i128 - initial as i128
}
