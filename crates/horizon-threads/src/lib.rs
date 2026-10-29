#![cfg_attr(windows, forbid(unsafe_code))]

//! Scheduling hints for the game's threads. Windows, Linux and Android use
//! published safe priority APIs, whose native internals remain dependencies.

use std::sync::atomic::{AtomicBool, Ordering};

/// Ask the OS to run the calling thread ahead of the process's other
/// threads, as vanilla's `MinecraftServer.spin` does for its server thread
/// (`Thread.setPriority(8)` when there are more than four processors), so
/// world generation and chunk encoding on every other core don't hold a tick
/// back (the client asks the same for its render thread, so they don't hold
/// a frame back either): above-normal priority on Windows. Nothing elsewhere: Linux lets an
/// unprivileged thread lower its priority but not raise it (and the JVM
/// leaves it there too). Whether the OS took it.
#[cfg(not(windows))]
pub fn favour_current() -> bool {
    if std::thread::available_parallelism().map_or(1, |n| n.get()) <= 4 {
        return false;
    }
    favour()
}

/// Ask for above-normal calling-thread priority when more than four processors
/// are available. Returns false on an unavailable count or an unsuccessful OS call.
#[cfg(windows)]
pub fn favour_current() -> bool {
    favour_with_parallelism(std::thread::available_parallelism())
}

#[cfg(windows)]
fn favour_with_parallelism(parallelism: std::io::Result<std::num::NonZeroUsize>) -> bool {
    if parallelism.map_or(1, |n| n.get()) <= 4 {
        return false;
    }
    favour()
}

#[cfg(windows)]
fn favour() -> bool {
    set_windows_priority(thread_priority::WinAPIThreadPriority::AboveNormal)
}

#[cfg(windows)]
fn set_windows_priority(priority: thread_priority::WinAPIThreadPriority) -> bool {
    thread_priority::set_winapi_thread_priority(thread_priority::thread_native_id(), priority)
        .is_ok()
}

#[cfg(not(windows))]
fn favour() -> bool {
    false
}

/// Set once as the game starts, before its helper threads do: they then go
/// behind the server thread and the render thread (`behind_server`).
static HELPERS_BEHIND: AtomicBool = AtomicBool::new(false);

/// How many nice levels a helper thread goes below the process on Linux and Android.
#[cfg(any(target_os = "linux", target_os = "android"))]
const BEHIND: i32 = 10;

/// From now on, the helper threads that call `behind_server` as they start
/// go behind the server thread (a dedicated server's or single player's)
/// and the render thread. Call it before they start.
pub fn put_helpers_behind() {
    HELPERS_BEHIND.store(true, Ordering::Relaxed);
}

/// Called by a helper thread as it starts (world generation, lighting and
/// meshing, a connection's packet writer): work no tick or frame waits for.
/// Once `put_helpers_behind` was called, on Linux and Android, where the server and
/// render threads can't be raised (`favour_current`), the helper lowers
/// itself instead, which an unprivileged thread may: 10 nice levels below
/// the process, or, where nice runs out (a game started under nice 10 or
/// more), the idle class, which runs the thread only when no ordinary thread
/// wants its core. A tick or a frame then gets a core when it starts instead
/// of waiting behind them. Nothing elsewhere. Whether the OS took it.
pub fn behind_server() -> bool {
    HELPERS_BEHIND.load(Ordering::Relaxed) && behind()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn behind() -> bool {
    let Ok(nice) = rustix::process::getpriority_process(None) else {
        return false;
    };
    if nice + BEHIND <= 19 {
        rustix::process::setpriority_process(None, nice + BEHIND).is_ok()
    } else {
        set_idle(nice)
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn idle_priority(nice: i32) -> Option<thread_priority::ThreadPriority> {
    use thread_priority::{
        NormalThreadSchedulePolicy, ThreadPriority, ThreadPriorityValue, ThreadSchedulePolicy,
    };
    let policy = ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Idle);
    (0..=100)
        .filter_map(|value| ThreadPriorityValue::try_from(value).ok())
        .map(ThreadPriority::Crossplatform)
        .find(|priority| priority.to_posix(policy) == Ok(nice))
}

// A false result can mean the OS changed the policy but a later operation or
// readback failed; the safe API is a best-effort hint, not an atomic transaction.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn idle_result(result: Result<(), thread_priority::Error>, nice: i32) -> bool {
    use thread_priority::{NormalThreadSchedulePolicy, ThreadSchedulePolicy};
    result.is_ok()
        && thread_priority::thread_schedule_policy()
            == Ok(ThreadSchedulePolicy::Normal(
                NormalThreadSchedulePolicy::Idle,
            ))
        && rustix::process::getpriority_process(None) == Ok(nice)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_idle(nice: i32) -> bool {
    use thread_priority::{NormalThreadSchedulePolicy, ThreadSchedulePolicy};
    let Some(priority) = idle_priority(nice) else {
        return false;
    };
    let policy = ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Idle);
    idle_result(
        thread_priority::set_thread_priority_and_policy(
            thread_priority::thread_native_id(),
            priority,
            policy,
        ),
        nice,
    )
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn behind() -> bool {
    false
}

/// Run the calling thread only on a core nothing else wants: work that may
/// take seconds and that nothing waits for yet (the stronghold rings while a
/// world opens). The idle class on Linux, idle priority on Windows. Whether the OS took it.
pub fn idle_current() -> bool {
    idle()
}

#[cfg(target_os = "linux")]
fn idle() -> bool {
    rustix::process::getpriority_process(None).is_ok_and(set_idle)
}

#[cfg(windows)]
fn idle() -> bool {
    set_windows_priority(thread_priority::WinAPIThreadPriority::Idle)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn idle() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn idle_selection_validates_each_nice_and_uses_static_priority_zero() {
        use thread_priority::{NormalThreadSchedulePolicy, ThreadSchedulePolicy};
        let policy = ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Idle);
        for nice in -20..=19 {
            assert_eq!(
                super::idle_priority(nice)
                    .unwrap()
                    .to_posix(policy)
                    .unwrap(),
                0
            );
        }
        assert!(super::idle_priority(-21).is_none());
        assert!(super::idle_priority(20).is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn failed_idle_setter_is_reported_as_false() {
        let error = thread_priority::Error::Priority("injected setter failure");
        assert!(!super::idle_result(Err(error), 0));
    }

    /// The hint holds on the platforms that take it.
    #[test]
    fn favouring_a_thread_takes_where_the_os_allows_it() {
        let took = std::thread::spawn(super::favour).join().unwrap();
        assert_eq!(took, cfg!(windows));
    }

    /// A helper goes behind only once a dedicated server asks for it, and
    /// then, on Linux and Android, 10 nice levels below the process or in the idle class.
    #[test]
    fn helpers_go_behind_only_when_asked() {
        assert!(!std::thread::spawn(super::behind_server).join().unwrap());
        super::put_helpers_behind();
        #[cfg(any(target_os = "linux", target_os = "android"))]
        std::thread::spawn(|| {
            let before = rustix::process::getpriority_process(None).unwrap();
            assert!(super::behind_server());
            let after = rustix::process::getpriority_process(None).unwrap();
            let policy = thread_priority::thread_schedule_policy().unwrap();
            if before + super::BEHIND <= 19 {
                assert_eq!(after, before + super::BEHIND);
            } else {
                assert_eq!(
                    policy,
                    thread_priority::ThreadSchedulePolicy::Normal(
                        thread_priority::NormalThreadSchedulePolicy::Idle
                    )
                );
            }
        })
        .join()
        .unwrap();
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        assert!(!std::thread::spawn(super::behind_server).join().unwrap());
    }

    /// The idle hint holds on the supported platforms.
    #[test]
    fn idling_a_thread_takes() {
        let took = std::thread::spawn(super::idle_current).join().unwrap();
        assert_eq!(took, cfg!(any(target_os = "linux", windows)));
        #[cfg(target_os = "linux")]
        std::thread::spawn(|| {
            assert!(super::idle_current());
            assert_eq!(
                thread_priority::thread_schedule_policy().unwrap(),
                thread_priority::ThreadSchedulePolicy::Normal(
                    thread_priority::NormalThreadSchedulePolicy::Idle
                )
            );
        })
        .join()
        .unwrap();
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::{io, num::NonZeroUsize, sync::mpsc};
    use thread_priority::{ThreadPriority, WinAPIThreadPriority, get_current_thread_priority};

    fn priority() -> ThreadPriority {
        get_current_thread_priority().unwrap()
    }

    fn expected(priority: WinAPIThreadPriority) -> ThreadPriority {
        ThreadPriority::Os(priority.into())
    }

    #[test]
    fn native_hints_set_exact_calling_thread_priorities() {
        std::thread::spawn(|| {
            assert!(favour());
            assert_eq!(priority(), expected(WinAPIThreadPriority::AboveNormal));
            assert!(idle_current());
            assert_eq!(priority(), expected(WinAPIThreadPriority::Idle));
        })
        .join()
        .unwrap();
    }

    #[test]
    fn parallelism_gate_preserves_priority_through_four_and_on_error() {
        std::thread::spawn(|| {
            for count in [1, 4] {
                let before = priority();
                assert!(!favour_with_parallelism(Ok(
                    NonZeroUsize::new(count).unwrap()
                )));
                assert_eq!(priority(), before);
            }
            let before = priority();
            assert!(!favour_with_parallelism(Err(io::Error::other(
                "unavailable"
            ))));
            assert_eq!(priority(), before);
            assert!(favour_with_parallelism(Ok(NonZeroUsize::new(5).unwrap())));
            assert_eq!(priority(), expected(WinAPIThreadPriority::AboveNormal));
        })
        .join()
        .unwrap();
    }

    #[test]
    fn public_favour_uses_the_available_parallelism_gate() {
        std::thread::spawn(|| {
            let before = priority();
            let enough = std::thread::available_parallelism().is_ok_and(|n| n.get() > 4);
            assert_eq!(favour_current(), enough);
            assert_eq!(
                priority(),
                if enough {
                    expected(WinAPIThreadPriority::AboveNormal)
                } else {
                    before
                }
            );
        })
        .join()
        .unwrap();
    }

    #[test]
    fn native_failure_returns_false_without_changing_priority() {
        std::thread::spawn(|| {
            let before = priority();
            // A fresh thread is not in background mode, so ending it must fail.
            assert!(!set_windows_priority(
                WinAPIThreadPriority::BackgroundModeEnd
            ));
            assert_eq!(priority(), before);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn hints_leave_siblings_and_child_creation_priority_unchanged() {
        std::thread::spawn(|| {
            let before = priority();
            let (ready_tx, ready_rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            let sibling = std::thread::spawn(move || {
                let before = priority();
                ready_tx.send(()).unwrap();
                done_rx.recv().unwrap();
                assert_eq!(priority(), before);
            });
            ready_rx.recv().unwrap();
            assert!(favour());
            let child_priority = std::thread::spawn(priority).join().unwrap();
            // Windows creates children at normal relative priority, not their parent's hint.
            assert_eq!(child_priority, expected(WinAPIThreadPriority::Normal));
            assert!(idle_current());
            assert!(!behind_server());
            put_helpers_behind();
            assert!(!behind_server());
            assert_eq!(priority(), expected(WinAPIThreadPriority::Idle));
            done_tx.send(()).unwrap();
            sibling.join().unwrap();
            assert_ne!(before, priority());
        })
        .join()
        .unwrap();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    #![forbid(unsafe_code)]

    use std::thread;
    use thread_priority::{NormalThreadSchedulePolicy, ThreadPriority, ThreadSchedulePolicy};

    fn snapshot() -> (i32, ThreadSchedulePolicy) {
        (
            rustix::process::getpriority_process(None).unwrap(),
            thread_priority::thread_schedule_policy().unwrap(),
        )
    }

    // A caller must not confuse Idle's static priority zero with Linux niceness.
    // Dropping set_idle or resetting niceness to zero must fail this native check.
    #[test]
    fn native_idle_preserves_zero_ten_and_nineteen_nice_and_parent() {
        let parent = snapshot();
        for nice in [0, 10, 19] {
            thread::spawn(move || {
                rustix::process::setpriority_process(None, nice).unwrap();
                assert_eq!(snapshot().0, nice);
                assert!(super::set_idle(nice), "idle hint rejected nice {nice}");
                let (policy, params) = thread_priority::thread_schedule_policy_param(
                    thread_priority::thread_native_id(),
                )
                .unwrap();
                assert_eq!(
                    policy,
                    ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Idle)
                );
                assert_eq!(params.sched_priority, 0);
                assert_eq!(snapshot().0, nice);
            })
            .join()
            .unwrap();
            assert_eq!(snapshot(), parent);
        }
    }

    #[test]
    fn native_helpers_increment_nice_or_enter_idle_without_touching_sibling() {
        let parent = snapshot();
        // The test thread stays live as an unaffected sibling throughout each call.
        for (nice, expected_nice, idle) in [(0, 10, false), (10, 10, true), (19, 19, true)] {
            thread::spawn(move || {
                rustix::process::setpriority_process(None, nice).unwrap();
                let before = snapshot();
                assert!(super::behind());
                assert_eq!(snapshot().0, expected_nice);
                assert_eq!(
                    snapshot().1,
                    if idle {
                        ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Idle)
                    } else {
                        before.1
                    }
                );
            })
            .join()
            .unwrap();
            assert_eq!(snapshot(), parent);
        }
    }

    #[test]
    fn invalid_idle_nice_and_priority_fail_without_native_changes() {
        thread::spawn(|| {
            let before = snapshot();
            for nice in [i32::MIN, -21, 20, i32::MAX] {
                assert!(!super::set_idle(nice));
                assert_eq!(snapshot(), before);
            }
            let invalid = ThreadPriority::Os(1.into());
            assert!(
                thread_priority::set_thread_priority_and_policy(
                    thread_priority::thread_native_id(),
                    invalid,
                    ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Idle),
                )
                .is_err()
            );
            assert_eq!(snapshot(), before);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn already_inherited_negative_nice_is_preserved_when_available() {
        let inherited = snapshot().0;
        if inherited >= 0 {
            eprintln!("native negative niceness NOT RUN: no legitimately inherited negative nice");
            return;
        }
        thread::spawn(move || {
            assert_eq!(snapshot().0, inherited);
            assert!(super::set_idle(inherited));
            assert_eq!(snapshot().0, inherited);
            assert_eq!(
                snapshot().1,
                ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Idle)
            );
        })
        .join()
        .unwrap();
    }
}
