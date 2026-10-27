use horizon_launcher::process;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub fn console_probe() -> &'static Path {
    static PROBE: OnceLock<PathBuf> = OnceLock::new();
    PROBE.get_or_init(|| {
        build_probe()
            .unwrap_or_else(|error| panic!("automatic console fixture build failed: {error}"))
    })
}

fn fresh_root() -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..16 {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "horizon-console-probe-{}-{epoch}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&root) {
            Ok(()) => return Ok(root),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other(
        "could not create a fresh console fixture directory",
    ))
}

fn build_probe() -> io::Result<PathBuf> {
    let root = fresh_root()?;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/support/console-probe")
        .canonicalize()?;
    let manifest = fixture.join("Cargo.toml").canonicalize()?;
    let source = fixture.join("../console_probe.rs").canonicalize()?;
    let host = env!("HORIZON_BUILD_HOST");
    if host != env!("HORIZON_BUILD_TARGET") || !host.contains("windows") {
        return Err(io::Error::other(
            "console fixture requires native Windows tests (Cargo host must equal target)",
        ));
    }
    // Only the child receives overrides. In particular, do not inherit a parent build-dir
    // or cross-target and then wait on the parent's currently active compiler output lock.
    let cargo = || {
        let mut command = process::command(env!("CARGO"));
        command
            .current_dir(&fixture)
            .env("CARGO_BUILD_BUILD_DIR", root.join("build"))
            .env("CARGO_TARGET_DIR", root.join("target"))
            .env("CARGO_BUILD_TARGET", host)
            .env("CARGO_NET_OFFLINE", "true");
        command
    };
    let metadata_root = root.join("metadata");
    fs::create_dir(&metadata_root)?;
    let metadata = run_checked(
        cargo()
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--locked",
                "--offline",
                "--manifest-path",
            ])
            .arg(&manifest),
        &metadata_root,
    )?;
    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| io::Error::other("fixture metadata has no packages"))?;
    if packages.len() != 1
        || packages[0]["name"] != "horizon-console-probe"
        || packages[0]["version"] != "0.0.0"
        || Path::new(metadata["workspace_root"].as_str().unwrap_or("")).canonicalize()? != fixture
        || Path::new(packages[0]["manifest_path"].as_str().unwrap_or("")).canonicalize()?
            != manifest
    {
        return Err(io::Error::other(
            "unexpected fixture package/workspace/manifest",
        ));
    }
    let package = packages[0]["id"]
        .as_str()
        .ok_or_else(|| io::Error::other("fixture metadata has no package id"))?;
    let output = run_checked(
        cargo()
            .args(["build", "--manifest-path"])
            .arg(&manifest)
            .args([
                "--bin",
                "console-probe",
                "--locked",
                "--offline",
                "--jobs",
                "1",
                "--target",
                host,
                "--target-dir",
            ])
            .arg(root.join("target"))
            .arg("--message-format=json"),
        &root,
    )?;
    let artifact = checked_artifact(&output, &root, package, &manifest, &source)?;
    let executable = root.join("console-probe.exe");
    fs::copy(&artifact, &executable)?;
    fs::write(
        root.join("artifact.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "package_id":package, "manifest":manifest, "source":source, "native_host":host,
            "cargo_executable":artifact, "copied_executable":executable,
            "target_dir":root.join("target"), "build_dir":root.join("build")
        }))?,
    )?;
    eprintln!(
        "console fixture artifact: {}",
        root.join("artifact.json").display()
    );
    Ok(executable)
}

fn checked_artifact(
    output: &str,
    root: &Path,
    package: &str,
    manifest: &Path,
    source: &Path,
) -> io::Result<PathBuf> {
    let root = root.canonicalize()?;
    let manifest = manifest.canonicalize()?;
    let source = source.canonicalize()?;
    let mut artifacts = Vec::new();
    for line in output.lines() {
        let message: serde_json::Value = serde_json::from_str(line)?;
        if message["reason"] != "compiler-artifact"
            || message["package_id"] != package
            || message["target"]["name"] != "console-probe"
            || message["target"]["kind"] != serde_json::json!(["bin"])
            || message["manifest_path"]
                .as_str()
                .map(Path::new)
                .map(Path::canonicalize)
                .transpose()?
                .as_deref()
                != Some(manifest.as_path())
            || message["target"]["src_path"]
                .as_str()
                .map(Path::new)
                .map(Path::canonicalize)
                .transpose()?
                .as_deref()
                != Some(source.as_path())
        {
            continue;
        }
        let executable = Path::new(
            message["executable"]
                .as_str()
                .ok_or_else(|| io::Error::other("fixture artifact has no executable"))?,
        );
        if !executable.is_absolute() {
            return Err(io::Error::other(
                "fixture artifact executable is not absolute",
            ));
        }
        let executable = executable.canonicalize()?;
        if !executable.starts_with(&root) || !executable.is_file() {
            return Err(io::Error::other(
                "fixture executable is outside its owned output root or missing",
            ));
        }
        artifacts.push(executable);
    }
    if artifacts.len() != 1 {
        return Err(io::Error::other(format!(
            "expected exactly one matching fixture executable, found {}",
            artifacts.len()
        )));
    }
    Ok(artifacts.remove(0))
}

// Cargo's small JSON protocol goes to owned files, not pipes that could deadlock when full.
// Bound time and log size. On every failure retain those files and the exact build directory.
fn run_checked(command: &mut Command, root: &Path) -> io::Result<String> {
    let stdout = root.join("cargo.stdout.log");
    let stderr = root.join("cargo.stderr.log");
    let mut child = OwnedChild(
        command
            .stdin(Stdio::null())
            .stdout(fs::File::create(&stdout)?)
            .stderr(fs::File::create(&stderr)?)
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline
            || fs::metadata(&stdout)?.len() > 32 * 1024 * 1024
            || fs::metadata(&stderr)?.len() > 32 * 1024 * 1024
        {
            return Err(io::Error::other(format!(
                "fixture Cargo exceeded 180s/32MiB bound; diagnostics retained at {}",
                root.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    // Check again after exit: a fast writer can finish between polling intervals.
    if fs::metadata(&stdout)?.len() > 32 * 1024 * 1024
        || fs::metadata(&stderr)?.len() > 32 * 1024 * 1024
    {
        return Err(io::Error::other(format!(
            "fixture Cargo exceeded 32MiB output bound; diagnostics retained at {}",
            root.display()
        )));
    }
    let output = fs::read_to_string(&stdout)?;
    if !status.success() {
        let diagnostics = fs::read_to_string(&stderr)?;
        return Err(io::Error::other(format!(
            "fixture Cargo status {status}; diagnostics at {}\n{}",
            root.display(),
            diagnostics.chars().take(64 * 1024).collect::<String>()
        )));
    }
    Ok(output)
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn artifact(root: &Path, manifest: &Path, source: &Path, executable: &Path) -> String {
        json!({"reason":"compiler-artifact", "package_id": format!("path+file:///{}#horizon-console-probe@0.0.0", root.display()), "manifest_path":manifest, "target":{"name":"console-probe","kind":["bin"],"src_path":source},"executable":executable}).to_string()
    }

    #[test]
    fn checked_artifact_rejects_missing_ambiguous_and_out_of_scope_results() {
        let root = fresh_root().unwrap();
        let manifest = root.join("Cargo.toml");
        let source = root.join("probe.rs");
        let executable = root.join("target/probe.exe");
        fs::create_dir(root.join("target")).unwrap();
        for path in [&manifest, &source, &executable] {
            fs::write(path, "fixture").unwrap();
        }
        let package = format!(
            "path+file:///{}#horizon-console-probe@0.0.0",
            root.display()
        );
        let valid = artifact(&root, &manifest, &source, &executable);
        assert_eq!(
            checked_artifact(&valid, &root, &package, &manifest, &source).unwrap(),
            executable.canonicalize().unwrap()
        );
        assert!(checked_artifact("", &root, &package, &manifest, &source).is_err());
        assert!(checked_artifact(
            &format!("{valid}\n{valid}"),
            &root,
            &package,
            &manifest,
            &source
        )
        .is_err());
        assert!(checked_artifact(&valid, &root, "unrelated-package", &manifest, &source).is_err());
        assert!(
            checked_artifact(&valid, &root, &package, &manifest, &root.join("other.rs")).is_err()
        );
        let other_root = fresh_root().unwrap();
        let outside = other_root.join("not-owned.exe");
        fs::write(&outside, "unrelated executable").unwrap();
        assert!(checked_artifact(
            &artifact(&root, &manifest, &source, &outside),
            &root,
            &package,
            &manifest,
            &source
        )
        .is_err());
        let mut wrong: serde_json::Value = serde_json::from_str(&valid).unwrap();
        wrong["target"]["name"] = json!("another-bin");
        assert!(checked_artifact(&wrong.to_string(), &root, &package, &manifest, &source).is_err());
        wrong["target"]["name"] = json!("console-probe");
        wrong["target"]["kind"] = json!(["lib"]);
        assert!(checked_artifact(&wrong.to_string(), &root, &package, &manifest, &source).is_err());
        assert!(checked_artifact("not JSON", &root, &package, &manifest, &source).is_err());
        fs::remove_file(executable).unwrap();
        assert!(checked_artifact(&valid, &root, &package, &manifest, &source).is_err());
    }

    #[test]
    fn failed_fixture_build_reports_status_and_retains_diagnostics() {
        let root = fresh_root().unwrap();
        let mut command = process::command(env!("CARGO"));
        command
            .args(["build", "--offline", "--locked", "--manifest-path"])
            .arg(root.join("missing/Cargo.toml"));
        let error = run_checked(&mut command, &root).unwrap_err().to_string();
        assert!(error.contains("status"), "{error}");
        assert!(error.contains("Cargo.toml"), "{error}");
        assert!(root.join("cargo.stderr.log").is_file());
    }
}
