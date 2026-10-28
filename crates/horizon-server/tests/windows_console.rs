#![cfg_attr(windows, forbid(unsafe_code))]

#[cfg(windows)]
#[path = "../src/process.rs"]
mod process;
#[cfg(windows)]
#[path = "../src/signals.rs"]
mod signals;

#[cfg(windows)]
fn main() {
    use std::{
        io::{Read, Write},
        process::{Child, Stdio},
        time::{Duration, Instant},
    };
    use win32console::console::WinConsole;

    struct OwnedChild(Option<Child>);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if let Some(child) = self.0.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    if std::env::args().nth(1).as_deref() == Some("--horizon-console-child") {
        signals::install();
        assert_eq!(signals::requests(), 0);
        for (index, event) in [0, 1, 2, 5, 6, u32::MAX, 0].into_iter().enumerate() {
            assert_eq!(signals::handle(event), 1);
            assert_eq!(signals::requests(), index as u32 + 1);
        }
        let present = WinConsole::get_window().is_some();
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).unwrap();
        assert_eq!(input, "owned-child-input\n");
        println!("console={present};stdout-roundtrip:{input}");
        eprintln!("stderr-roundtrip");
        std::process::exit(7);
    }

    let present = WinConsole::get_window().is_some();
    let child = process::command(std::env::current_exe().unwrap())
        .arg("--horizon-console-child")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn owned console probe");
    let mut owned = OwnedChild(Some(child));
    let child = owned.0.as_mut().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"owned-child-input\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "owned console probe timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = owned.0.take().unwrap().wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(7));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stdout.starts_with(&format!(
            "console={present};stdout-roundtrip:owned-child-input\n"
        )),
        "{stdout}"
    );
    assert_eq!(stderr.trim_end(), "stderr-roundtrip");
    println!("owned child console presence, streams and exit status: PASS");
}

#[cfg(not(windows))]
fn main() {}
