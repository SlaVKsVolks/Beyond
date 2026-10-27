#![cfg_attr(windows, forbid(unsafe_code))]

//! Scheduling hints for the game's threads. Windows uses a safe native API;
//! the other supported platforms retain their platform-specific OS calls.

use std::sync::atomic::{AtomicBool, Ordering};

/// Ask the OS to run the calling thread ahead of the process's other
/// threads, as vanilla's `MinecraftServer.spin` does for its server thread
/// (`Thread.setPriority(8)` when there are more than four processors), so
/// world generation and chunk encoding on every other core don't hold a tick
/// back (the client asks the same for its render thread, so they don't hold
/// a frame back either): user-interactive quality of service on macOS,
/// above-normal priority on Windows. Nothing elsewhere: Linux lets an
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

#[cfg(target_vendor = "apple")]
fn favour() -> bool {
    // SAFETY: sets the calling thread's own QoS class; no pointers.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0) == 0
    }
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

#[cfg(not(any(target_vendor = "apple", windows)))]
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
    // SAFETY: these read and set the calling thread's own nice value or
    // policy (who 0, pid 0 is the calling thread on Linux and Android); `param` lives
    // across the call.
    unsafe {
        let nice = libc::getpriority(libc::PRIO_PROCESS as _, 0);
        if nice + BEHIND <= 19 {
            libc::setpriority(libc::PRIO_PROCESS as _, 0, nice + BEHIND) == 0
        } else {
            let param = libc::sched_param { sched_priority: 0 };
            libc::sched_setscheduler(0, libc::SCHED_IDLE, &param) == 0
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn behind() -> bool {
    false
}

/// Run the calling thread only on a core nothing else wants: work that may
/// take seconds and that nothing waits for yet (the stronghold rings while a
/// world opens). The idle class on Linux, background quality of service on
/// macOS, idle priority on Windows. Whether the OS took it.
pub fn idle_current() -> bool {
    idle()
}

#[cfg(target_os = "linux")]
fn idle() -> bool {
    let param = libc::sched_param { sched_priority: 0 };
    // SAFETY: sets the calling thread's own policy (pid 0); `param` lives
    // across the call.
    unsafe { libc::sched_setscheduler(0, libc::SCHED_IDLE, &param) == 0 }
}

#[cfg(target_vendor = "apple")]
fn idle() -> bool {
    // SAFETY: sets the calling thread's own QoS class; no pointers.
    unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0) == 0 }
}

#[cfg(windows)]
fn idle() -> bool {
    set_windows_priority(thread_priority::WinAPIThreadPriority::Idle)
}

#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
fn idle() -> bool {
    false
}

#[cfg(test)]
mod tests {
    /// The hint holds on the platforms that take it.
    #[test]
    fn favouring_a_thread_takes_where_the_os_allows_it() {
        let took = std::thread::spawn(super::favour).join().unwrap();
        assert_eq!(took, cfg!(any(target_vendor = "apple", windows)));
        #[cfg(target_vendor = "apple")]
        std::thread::spawn(|| {
            assert!(super::favour());
            let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
            let mut relative = 0;
            // SAFETY: reads the calling thread's own QoS class into locals.
            let read = unsafe {
                libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative)
            };
            assert_eq!(read, 0);
            assert!(matches!(
                class,
                libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE
            ));
        })
        .join()
        .unwrap();
    }

    /// A helper goes behind only once a dedicated server asks for it, and
    /// then, on Linux and Android, 10 nice levels below the process or in the idle class.
    #[test]
    fn helpers_go_behind_only_when_asked() {
        assert!(!std::thread::spawn(super::behind_server).join().unwrap());
        super::put_helpers_behind();
        #[cfg(any(target_os = "linux", target_os = "android"))]
        std::thread::spawn(|| {
            // SAFETY: reads the calling thread's own nice value.
            let before = unsafe { libc::getpriority(libc::PRIO_PROCESS as _, 0) };
            assert!(super::behind_server());
            // SAFETY: reads the calling thread's own nice value and policy.
            let (after, policy) = unsafe {
                (
                    libc::getpriority(libc::PRIO_PROCESS as _, 0),
                    libc::sched_getscheduler(0),
                )
            };
            if before + super::BEHIND <= 19 {
                assert_eq!(after, before + super::BEHIND);
            } else {
                assert_eq!(policy, libc::SCHED_IDLE);
            }
        })
        .join()
        .unwrap();
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        assert!(!std::thread::spawn(super::behind_server).join().unwrap());
    }

    /// The idle hint holds on the three platforms.
    #[test]
    fn idling_a_thread_takes() {
        let took = std::thread::spawn(super::idle_current).join().unwrap();
        assert_eq!(
            took,
            cfg!(any(target_os = "linux", target_vendor = "apple", windows))
        );
        #[cfg(target_os = "linux")]
        std::thread::spawn(|| {
            assert!(super::idle_current());
            // SAFETY: reads the calling thread's own policy.
            assert_eq!(unsafe { libc::sched_getscheduler(0) }, libc::SCHED_IDLE);
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
