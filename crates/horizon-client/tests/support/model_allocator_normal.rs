#![forbid(unsafe_code)]

use alloc_metrics::{Metrics, global_metrics};
use allocator_api2::boxed::Box as FallibleBox;
use std::hint::black_box;

#[repr(align(64))]
struct Aligned([u8; 64]);

fn expect_delta(base: Metrics, bytes: usize, allocations: isize, tracking: bool) {
    assert_eq!(
        global_metrics(),
        if tracking {
            Metrics {
                allocated_bytes: base.allocated_bytes + isize::try_from(bytes).unwrap(),
                allocations: base.allocations + allocations,
            }
        } else {
            base
        }
    );
}

pub fn exercise(tracking: bool) {
    let initial = global_metrics();
    let mut data = Vec::<u8>::new();
    data.try_reserve_exact(37).unwrap();
    data.extend_from_slice(&[7; 37]);
    expect_delta(initial, data.capacity(), 1, tracking);
    data.try_reserve_exact(4096 - data.len()).unwrap();
    expect_delta(initial, data.capacity(), 1, tracking);
    assert_eq!(data.as_slice(), &[7; 37]);
    data.truncate(17);
    data.shrink_to_fit();
    expect_delta(initial, data.capacity(), 1, tracking);
    assert_eq!(data.as_slice(), &[7; 17]);
    black_box(&data);
    drop(data);
    assert_eq!(global_metrics(), initial);

    let zeroed = vec![0u8; 257];
    expect_delta(initial, zeroed.capacity(), 1, tracking);
    assert!(zeroed.iter().all(|&byte| byte == 0));
    black_box(&zeroed);
    drop(zeroed);
    assert_eq!(global_metrics(), initial);

    let aligned = Box::new(Aligned([3; 64]));
    expect_delta(initial, 64, 1, tracking);
    assert_eq!(std::ptr::from_ref(aligned.as_ref()).addr() % 64, 0);
    assert_eq!(aligned.0, [3; 64]);
    black_box(&aligned);
    drop(aligned);
    assert_eq!(global_metrics(), initial);

    // Keep MaybeUninit opaque: this exercises the safe zeroed allocation API.
    let zeroed_box = FallibleBox::<[u8; 4096]>::try_new_zeroed().unwrap();
    expect_delta(initial, 4096, 1, tracking);
    black_box(&zeroed_box);
    drop(zeroed_box);
    assert_eq!(global_metrics(), initial);
}
