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

fn main() {
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
