#[test]
fn empty_bursts_persistence_and_a_later_request() {
    use super::Pending;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let flag = Arc::new(AtomicBool::new(false));
    let mut requests = Pending::new(Arc::clone(&flag));
    assert_eq!(requests.observe(), 0);
    for _ in 0..3 {
        flag.store(true, Ordering::SeqCst);
    }
    assert_eq!(requests.observe(), 1);
    assert_eq!(requests.observe(), 1);
    flag.store(true, Ordering::SeqCst);
    assert_eq!(requests.observe(), 2);
    assert_eq!(requests.observe(), 2);
}

#[test]
fn production_observer_matches_bounded_original_consumer_traces() {
    use super::Pending;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    // All 4^8 traces: handler, persistent query, initial snapshot, poll.
    for trace in 0u32..65_536 {
        let flag = Arc::new(AtomicBool::new(false));
        let mut requests = Pending::new(Arc::clone(&flag));
        let (mut original, mut old_handled, mut new_handled) = (0u32, 0u32, 0u32);
        for index in 0..8 {
            match (trace >> (2 * index)) & 3 {
                0 => {
                    original += 1;
                    flag.store(true, Ordering::SeqCst);
                }
                1 => assert_eq!(requests.observe() > 0, original > 0),
                2 => {
                    old_handled = original;
                    new_handled = requests.observe();
                }
                3 => {
                    let observed = requests.observe();
                    assert_eq!(observed > new_handled, original > old_handled);
                    old_handled = original;
                    new_handled = observed;
                }
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn partial_registration_error_still_restores_and_stops_publication() {
    use super::publish;
    use std::cell::Cell;

    let calls = Cell::new(0);
    let restores = Cell::new(0);
    let result = publish(
        &[1, 2, 3],
        |_, _| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                Err("registration failed".into())
            } else {
                Ok(())
            }
        },
        || {
            restores.set(restores.get() + 1);
            Ok(())
        },
    );
    assert_eq!(result.err().as_deref(), Some("registration failed"));
    assert_eq!(calls.get(), 2);
    assert_eq!(restores.get(), 1);
}

#[test]
fn restoration_failure_cannot_return_an_observer() {
    use super::publish;

    let result = publish(&[1, 2, 3], |_, _| Ok(()), || Err("restore failed".into()));
    assert_eq!(result.err().as_deref(), Some("restore failed"));
}

#[test]
fn both_registration_and_restoration_errors_survive() {
    use super::publish;

    let result = publish(
        &[1, 2, 3],
        |_, _| Err("registration failed".into()),
        || Err("restore failed".into()),
    );
    let error = result.err().expect("both failures must be returned");
    assert!(error.contains("registration failed"));
    assert!(error.contains("restore failed"));
}

#[test]
fn publication_uses_one_flag_and_observes_delivery_during_restoration() {
    use super::publish;
    use std::sync::{Arc, atomic::Ordering};

    let flags = std::cell::RefCell::new(Vec::new());
    let mut requests = publish(
        &[1, 2, 3],
        |_, flag| {
            flags.borrow_mut().push(flag);
            Ok(())
        },
        || {
            let flags = flags.borrow();
            assert_eq!(flags.len(), 3);
            assert!(flags.iter().all(|flag| Arc::ptr_eq(flag, &flags[0])));
            flags[0].store(true, Ordering::SeqCst);
            Ok(())
        },
    )
    .expect("successful publication and restoration");
    assert_eq!(requests.observe(), 1);
    assert_eq!(requests.observe(), 1);
}
