//! A real Windows console program used to check hidden children and their streams.
#![forbid(unsafe_code)]

#[cfg(not(windows))]
compile_error!("the console fixture requires a native Windows host");
use std::io;

fn hardware_probe(visible: bool) -> bool {
    let Some(path) = std::env::var_os("HORIZON_STARTUP_PROBE_LOG") else {
        return false;
    };
    use std::io::Write;
    let arguments = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let program = std::env::current_exe().unwrap();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(
        log,
        "{timestamp}|{visible}|{}",
        program.file_name().unwrap().to_string_lossy()
    )
    .unwrap();
    // Slow probes make a synchronous startup regression visible without relying on WMI speed.
    std::thread::sleep(std::time::Duration::from_millis(100));
    let responses = [
        ("TotalPhysicalMemory", "17179869184"),
        ("NumberOfCores", "8"),
        ("MaxClockSpeed", "3600"),
        ("BatteryStatus", "2"),
        ("Win32_Battery", "0"),
    ];
    let response = responses
        .iter()
        .find_map(|&(query, response)| arguments.contains(query).then_some(response))
        .unwrap_or("Microsoft Windows [Version 10.0.26100]");
    println!("{response}");
    true
}

// Only the explicitly selected owned lifecycle test modes enter this protocol.
// Every dummy exits by itself within two seconds, including a RED run without job ownership.
fn lifecycle_probe() -> bool {
    use std::{
        fs,
        os::windows::process::CommandExt,
        path::PathBuf,
        process::Command,
        time::{Duration, Instant},
    };
    let mut args = std::env::args_os().skip(1);
    let Some(mode) = args.next() else {
        return false;
    };
    if mode != "--lifecycle-parent" && mode != "--lifecycle-grandchild" {
        return false;
    }
    let root = PathBuf::from(args.next().expect("owned fixture directory"));
    let behavior = args.next().unwrap_or_default();
    let end = Instant::now() + Duration::from_secs(2);
    if mode == "--lifecycle-grandchild" {
        fs::write(
            root.join("grandchild.ready"),
            std::process::id().to_string(),
        )
        .unwrap();
        while Instant::now() < end {
            fs::write(root.join("heartbeat"), format!("{:?}", Instant::now())).unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        fs::write(root.join("grandchild.finished"), "self bounded").unwrap();
        return true;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--lifecycle-grandchild"])
        .arg(&root)
        .creation_flags(0x0800_0000);
    let mut child = command.spawn().unwrap();
    while !root.join("grandchild.ready").is_file() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(root.join("grandchild.ready").is_file());
    fs::write(root.join("parent.ready"), std::process::id().to_string()).unwrap();
    if behavior == "parent-exits-first" {
        std::thread::sleep(Duration::from_millis(150));
        // Deliberately release only the parent-side Windows process/thread handles.
        // This fixture must exit before its self-bounded, job-owned child; waiting
        // here would make the root-first ownership regression impossible to test.
        drop(child);
        return true;
    }
    while Instant::now() < end {
        if behavior == "overflow" {
            println!("{}", "x".repeat(4096));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.wait().unwrap();
    true
}

fn main() {
    if lifecycle_probe() {
        return;
    }
    // A console connection can exist without a terminal window, including in ConPTY.
    // These are direct OS queries; this fixture never attaches/detaches concurrently.
    let visible = win32console::console::WinConsole::get_window().is_some()
        && winconsole::window::is_visible();
    if hardware_probe(visible) {
        return;
    }
    println!("console_window_visible={visible}");
    println!("stdout is captured");
    eprintln!("stderr is captured");
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    println!("stdin={}", input.trim());
    if std::env::args().any(|argument| argument == "--fail") {
        std::process::exit(7);
    }
}
