#![forbid(unsafe_code)]

use alloc_metrics::{MetricAlloc, global_metrics};
use allocator_api2::boxed::Box as FallibleBox;
use std::{alloc::System, hint::black_box};

#[global_allocator]
static ALLOCATOR: MetricAlloc<cap::Cap<System>> =
    MetricAlloc::new(cap::Cap::new(System, 16 * 1024 * 1024));

#[path = "support/model_allocator_normal.rs"]
mod model_allocator_normal;

fn main() {
    model_allocator_normal::exercise(true);
    let initial = global_metrics();
    let mut empty = Vec::<u8>::new();
    assert!(empty.try_reserve_exact(32 * 1024 * 1024).is_err());
    assert!(empty.is_empty());
    assert_eq!(empty.capacity(), 0);
    assert_eq!(global_metrics(), initial);

    let mut retained = vec![11u8; 53];
    let pointer = retained.as_ptr();
    let capacity = retained.capacity();
    let before_failure = global_metrics();
    assert!(
        retained
            .try_reserve_exact(32 * 1024 * 1024 - retained.len())
            .is_err()
    );
    assert_eq!(retained.as_ptr(), pointer);
    assert_eq!(retained.len(), 53);
    assert_eq!(retained.capacity(), capacity);
    assert_eq!(retained.as_slice(), &[11; 53]);
    assert_eq!(global_metrics(), before_failure);
    black_box(&retained);
    drop(retained);
    assert_eq!(global_metrics(), initial);

    // No 32 MiB stack value is constructed; Cap rejects before System is called.
    assert!(FallibleBox::<[u8; 33554432]>::try_new_zeroed().is_err());
    assert_eq!(global_metrics(), initial);
    println!(
        "failure PASS initial_bytes={} initial_allocations={} normal=grow,shrink,zeroed,aligned,drop failure=alloc,realloc,zeroed counters=bytes,allocations",
        initial.allocated_bytes, initial.allocations
    );
}
