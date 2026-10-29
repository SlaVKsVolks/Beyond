//! Test-only published Chaos backend INSIDE the frozen adapter.
#![forbid(unsafe_code)]

use crate::{ModAlloc, Snapshot};
use alloc_chaos::{AllocOp, Check};
use std::cell::Cell;

#[global_allocator]
static ALLOC: ModAlloc = ModAlloc::new();

#[test]
fn genuine_null_alloc_and_zeroed_do_not_change_any_counter() {
    for zeroed in [false, true] {
        let failed = Cell::new(false);
        let unchanged = Cell::new(false);
        let report = Check::new()
            .only_failure(0)
            .try_run(|| {
                let before = ALLOC.try_snapshot().unwrap();
                let error = if zeroed {
                    match bytemuck::allocation::try_zeroed_box::<[u8; 64]>() {
                        Ok(b) => {
                            assert_eq!(*b, [0; 64]);
                            drop(b);
                            false
                        }
                        Err(()) => true,
                    }
                } else {
                    let mut v = Vec::<u8>::new();
                    let e = v.try_reserve_exact(64).is_err();
                    drop(v);
                    e
                };
                if error {
                    failed.set(true);
                    unchanged.set(ALLOC.try_snapshot().unwrap() == before);
                }
            })
            .unwrap();
        report.assert_success();
        assert_eq!(report.baseline_allocations(), 1);
        assert!(failed.get() && unchanged.get());
        let operation = report.attempts()[0].injected_allocation().unwrap();
        assert_eq!(
            operation.operation(),
            if zeroed {
                AllocOp::AllocZeroed
            } else {
                AllocOp::Alloc
            }
        );
        assert_eq!((operation.size(), operation.align()), (64, 1));
        assert_eq!(operation.new_size(), None);
    }
}

#[test]
fn genuine_null_realloc_preserves_old_data_pointer_layout_and_accounting() {
    let failed = Cell::new(false);
    let intact = Cell::new(false);
    let unchanged = Cell::new(false);
    let layout = Cell::new((0, 0));
    let report = Check::new()
        .only_failure(1)
        .try_run(|| {
            let mut v = Vec::<u8>::new();
            v.try_reserve_exact(64).unwrap();
            v.resize(64, 91);
            let before: Snapshot = ALLOC.try_snapshot().unwrap();
            let old_pointer = v.as_ptr();
            let old_capacity = v.capacity();
            let new_capacity = v.len() + 128;
            layout.set((old_capacity, new_capacity));
            if v.try_reserve_exact(128).is_err() {
                failed.set(true);
                intact.set(
                    v.as_ptr() == old_pointer
                        && v.capacity() == old_capacity
                        && v.len() == 64
                        && v.iter().all(|&b| b == 91),
                );
                unchanged.set(ALLOC.try_snapshot().unwrap() == before);
            }
            drop(v);
        })
        .unwrap();
    // Deliberately selected index 1 of 2: not an exhaustive failure campaign.
    assert_eq!(report.baseline_allocations(), 2);
    assert!(report.is_truncated());
    assert_eq!(report.attempts().len(), 1);
    let attempt = &report.attempts()[0];
    assert!(attempt.is_success() && attempt.injected());
    assert!(failed.get() && intact.get() && unchanged.get());
    let operation = attempt.injected_allocation().unwrap();
    assert_eq!(operation.operation(), AllocOp::Realloc);
    assert_eq!((operation.size(), operation.align()), (layout.get().0, 1));
    assert_eq!(operation.new_size(), Some(layout.get().1));
}
