#![forbid(unsafe_code)]

#[cfg(windows)]
fn fake_engine() -> std::io::Result<()> {
    use std::{fs, io::BufRead, time::Duration};
    let dir = std::env::current_dir()?;
    let owner: serde_json::Value = serde_json::from_slice(&fs::read(dir.join("owner.json"))?)?;
    assert_eq!(owner["fixture"], "windows-signal-lifecycle");
    let mode = fs::read_to_string(dir.join("mode.txt"))?;
    assert!(matches!(mode.as_str(), "graceful" | "stubborn"));
    let token = fs::read_to_string(dir.join("probe.txt"))?;
    assert!(token.starts_with("fixture-ready-"));
    fs::write(
        dir.join("engine.pid.pending"),
        std::process::id().to_string(),
    )?;
    fs::rename(dir.join("engine.pid.pending"), dir.join("engine.pid"))?;
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line == token {
            fs::write(dir.join("ready.txt"), &token)?;
        } else if line == "stop" {
            fs::write(dir.join("stop.txt"), "stop acknowledged")?;
            if mode == "graceful" {
                return Ok(());
            }
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "unexpected fake-engine command",
            ));
        }
    }
    // A stubborn fake never turns EOF into a false successful forced-shutdown result.
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(windows)]
fn main() -> std::io::Result<()> {
    use std::{os::windows::process::CommandExt, process::Command};
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--help"] {
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--resources") {
        assert_eq!(args.len(), 6);
        assert_eq!(args[2], "--control-socket");
        assert_eq!(&args[4..], ["--dedicated", "--nogui"]);
        return fake_engine();
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let python = std::env::var_os("HORIZON_TEST_PYTHON").unwrap_or_else(|| "python".into());
    let status = Command::new(python)
        .arg("-B")
        .arg(root.join("tools/windows_signal_lifecycle.py"))
        .arg("--server")
        .arg(env!("CARGO_BIN_EXE_horizon-server"))
        .arg("--fake")
        .arg(std::env::current_exe()?)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW; broker creates its own hidden console.
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "native Windows lifecycle controller failed: {status}"
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    println!("NOT RUN: actual Windows console lifecycle requires Windows");
}
