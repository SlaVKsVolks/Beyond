//! Native checks run only in fresh owned children, never against the test host.

#[cfg(unix)]
#[path = "../src/signals.rs"]
mod signals;

#[cfg(unix)]
fn child_case(case: &str) -> Result<(), Box<dyn std::error::Error>> {
    use nix::sys::signal::{SigSet, Signal, raise};
    use std::{cell::Cell, sync::atomic::Ordering};

    #[cfg(target_os = "linux")]
    assert_eq!(
        std::fs::read_dir("/proc/self/task")?.count(),
        1,
        "registration requires a single-threaded standalone startup"
    );

    let original = SigSet::thread_get_mask()?;
    let mut prior = original;
    for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP] {
        prior.remove(signal);
    }
    prior.add(Signal::SIGUSR1); // Unrelated blocked bits must survive installation.
    if case == "already-blocked" {
        prior.add(Signal::SIGINT);
    }
    prior.thread_set_mask()?;

    let signal = match case.rsplit('-').next().unwrap_or(case) {
        "term" => Signal::SIGTERM,
        "hup" => Signal::SIGHUP,
        _ => Signal::SIGINT,
    };
    if case == "registration-error" {
        let calls = Cell::new(0);
        let result = signals::observer_with(|number, flag| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                return Err("injected registrar failure".into());
            }
            signal_hook::flag::register(number, flag)
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
        assert!(
            result
                .err()
                .expect("registration must fail")
                .contains("injected registrar failure")
        );
        assert_eq!(calls.get(), 2);
        assert!(
            signals::observer().is_err(),
            "an uncertain partial install must not retry"
        );
        raise(Signal::SIGINT)?; // The retained partial action still owns its flag.
    } else if case == "registration-unwind" {
        let result = std::panic::catch_unwind(|| {
            signals::observer_with(|_, _| panic!("injected registrar unwind"))
        });
        assert!(result.is_err());
        assert!(signals::observer().is_err());
    } else {
        let mut requests = if case.starts_with("during-") {
            let calls = Cell::new(0);
            signals::observer_with(|number, flag| {
                let blocked = SigSet::thread_get_mask().map_err(|e| e.to_string())?;
                for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP] {
                    assert!(blocked.contains(signal));
                }
                if calls.get() == 0 {
                    // Targets this owned child thread before any action is published.
                    raise(signal).map_err(|e| e.to_string())?;
                }
                calls.set(calls.get() + 1);
                signal_hook::flag::register(number, std::sync::Arc::clone(&flag))
                    .map_err(|e| e.to_string())?;
                assert!(
                    !flag.load(Ordering::SeqCst),
                    "delivery must wait for restoration"
                );
                Ok(())
            })?
        } else {
            signals::observer()?
        };
        assert_eq!(
            SigSet::thread_get_mask()?.iter().collect::<Vec<_>>(),
            prior.iter().collect::<Vec<_>>()
        );
        if case.starts_with("during-") {
            assert!(
                requests.requests() > 0,
                "pending signal delivered after publication"
            );
        } else if case == "already-blocked" {
            raise(Signal::SIGINT)?;
            assert_eq!(requests.requests(), 0);
            let mut unblocked = prior;
            unblocked.remove(Signal::SIGINT);
            unblocked.thread_set_mask()?;
            assert!(requests.requests() > 0);
            prior.thread_set_mask()?;
        } else {
            assert_eq!(requests.requests(), 0);
            raise(signal)?;
            let first = requests.requests();
            assert!(first > 0);
            assert_eq!(requests.requests(), first, "stop remains observable");
            raise(signal)?;
            raise(signal)?;
            assert!(
                requests.requests() > first,
                "later request can force the stop"
            );
        }
        assert!(
            signals::observer().is_err(),
            "there is exactly one observer"
        );
    }
    assert_eq!(
        SigSet::thread_get_mask()?.iter().collect::<Vec<_>>(),
        prior.iter().collect::<Vec<_>>()
    );
    original.thread_set_mask()?;
    Ok(())
}

#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    if let Some(case) = std::env::args().nth(1) {
        return child_case(&case);
    }
    for case in [
        "int",
        "term",
        "hup",
        "during-int",
        "during-term",
        "during-hup",
        "already-blocked",
        "registration-error",
        "registration-unwind",
    ] {
        let mut child = Command::new(std::env::current_exe()?)
            .arg(case)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    let cleanup = terminate_owned(&mut child);
                    return Err(format!(
                        "owned signal case {case} wait failed: {error}; {cleanup}"
                    )
                    .into());
                }
            }
            if Instant::now() >= deadline {
                let cleanup = terminate_owned(&mut child);
                return Err(format!("owned signal case {case} timed out; {cleanup}").into());
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        if !status.success() {
            return Err(format!(
                "signal case {case}: {status}; child output inherited into test log"
            )
            .into());
        }
        println!("native signal case {case}: passed");
    }
    Ok(())
}

#[cfg(unix)]
fn terminate_owned(child: &mut std::process::Child) -> String {
    use std::time::{Duration, Instant};
    let pid = child.id();
    let kill = child.kill();
    let until = Instant::now() + Duration::from_secs(1);
    let mut wait_error = None;
    while Instant::now() < until {
        match child.try_wait() {
            Ok(Some(status)) => return format!("owned pid {pid}: kill={kill:?}, reaped={status}"),
            Ok(None) => {}
            Err(error) => wait_error = Some(error.to_string()),
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    format!("owned pid {pid} teardown unconfirmed; HOLD: kill={kill:?}, wait_error={wait_error:?}")
}

#[cfg(not(unix))]
fn main() {
    println!("Unix signal native checks NOT RUN: this is not a Unix target");
}
