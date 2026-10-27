use horizon_launcher::process;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs, io,
    io::{Read, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    os::windows::io::{AsHandle, AsRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    process::Command,
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
    let (executable, mut output) = prepare_probe()?;
    output.cleanup_outputs()?;
    Ok(executable)
}

fn prepare_probe() -> io::Result<(PathBuf, CheckedOutput)> {
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
    let metadata: serde_json::Value = serde_json::from_str(&metadata.stdout)?;
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
    let artifact = checked_artifact(&output.stdout, &root, package, &manifest, &source)?;
    let executable = root.join("console-probe.exe");
    fs::copy(&artifact, &executable)?;
    let artifact_hash = file_hash(&mut fs::File::open(&artifact)?)?;
    if artifact_hash != file_hash(&mut fs::File::open(&executable)?)? {
        return Err(io::Error::other(
            "copied fixture differs from current Cargo artifact",
        ));
    }
    fs::write(
        root.join("artifact.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "package_id":package, "manifest":manifest, "source":source, "native_host":host,
            "cargo_executable":artifact, "copied_executable":executable,
            "target_dir":root.join("target"), "build_dir":root.join("build"),
            "sha256":artifact_hash
        }))?,
    )?;
    eprintln!(
        "console fixture artifact: {}",
        root.join("artifact.json").display()
    );
    Ok((executable, output))
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

// Only these private, known Cargo command specs are converted. No raw_arg/env_clear or
// opaque extra creation flags occur at the callsites; CREATE_NO_WINDOW is kept explicitly.
#[derive(Clone, Copy)]
struct Limits {
    operation: Duration,
    log_bytes: u64,
    teardown: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            operation: Duration::from_secs(180),
            log_bytes: 32 * 1024 * 1024,
            teardown: Duration::from_secs(5),
        }
    }
}

#[derive(Debug)]
struct CheckedOutput {
    stdout: String,
    tree: ProcessTree,
    root: PathBuf,
}

fn file_hash(file: &mut fs::File) -> io::Result<String> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

// File guards deny writes but permit pathname unlink. Directory guards deny rename/
// deletion while descendants are inspected. This is deliberately not an atomic handle
// unlink: the unique owned tree must have no unrelated writer between ID check/unlink.
#[derive(Debug)]
struct OutputEntry {
    path: PathBuf,
    id: file_id::FileId,
    metadata: fs::Metadata,
    hash: Option<String>,
    allocated: u64,
    guard: Option<fs::File>,
    cargo_lock: bool,
}

fn checked_path(path: &Path, root: &Path) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_attributes() & 0x400 != 0
        || path.canonicalize()? != path
        || !path.starts_with(root)
    {
        return Err(io::Error::other(format!(
            "reparse or out-of-root output: {}",
            path.display()
        )));
    }
    Ok(metadata)
}

fn catalog_output(path: &Path, root: &Path, entries: &mut Vec<OutputEntry>) -> io::Result<()> {
    if entries.len() >= 16_384 {
        return Err(io::Error::other("unexpected compiler output catalog size"));
    }
    let metadata = checked_path(path, root)?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(io::Error::other("unknown compiler output kind"));
    }
    let cargo_lock = metadata.is_file()
        && matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some(".cargo-lock" | ".cargo-artifact-lock")
        );
    let mut options = fs::OpenOptions::new();
    if metadata.is_dir() {
        options
            .access_mode(0)
            .custom_flags(0x0200_0000)
            .share_mode(0x1 | 0x2);
    } else {
        options.read(true).write(cargo_lock).share_mode(0x1 | 0x4);
    }
    let mut guard = options.open(path).map_err(|error| {
        io::Error::other(format!(
            "compiler output guard unavailable {}: {error}",
            path.display()
        ))
    })?;
    if cargo_lock {
        fs2::FileExt::try_lock_exclusive(&guard).map_err(|error| {
            io::Error::other(format!(
                "Cargo output lock unavailable {}: {error}",
                path.display()
            ))
        })?;
    }
    let id = file_id::get_high_res_file_id(path)?;
    let hash = if metadata.is_file() {
        Some(file_hash(&mut guard)?)
    } else {
        None
    };
    let allocated = if metadata.is_file() {
        fs2::FileExt::allocated_size(&guard)?
    } else {
        0
    };
    let held = guard.metadata()?;
    if held.len() != metadata.len()
        || held.modified()? != metadata.modified()?
        || file_id::get_high_res_file_id(path)? != id
    {
        return Err(io::Error::other("output changed while acquiring guard"));
    }
    entries.push(OutputEntry {
        path: path.to_owned(),
        id,
        metadata,
        hash,
        allocated,
        guard: Some(guard),
        cargo_lock,
    });
    if entries.last().unwrap().metadata.is_dir() {
        let mut children = fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        children.sort();
        for child in children {
            catalog_output(&child, root, entries)?;
        }
    }
    Ok(())
}

fn validate_entry(
    entry: &mut OutputEntry,
    root: &Path,
    check_directory_time: bool,
) -> io::Result<()> {
    let current = checked_path(&entry.path, root)?;
    if file_id::get_high_res_file_id(&entry.path)? != entry.id
        || current.is_dir() != entry.metadata.is_dir()
        || (current.is_file() && current.len() != entry.metadata.len())
        || ((current.is_file() || check_directory_time)
            && current.modified()? != entry.metadata.modified()?)
    {
        return Err(io::Error::other(format!(
            "compiler output identity/metadata changed: {}",
            entry.path.display()
        )));
    }
    if let Some(expected) = &entry.hash {
        let guard = entry
            .guard
            .as_mut()
            .ok_or_else(|| io::Error::other("missing output guard"))?;
        let held = guard.metadata()?;
        if held.len() != current.len()
            || held.modified()? != current.modified()?
            || file_hash(guard)? != *expected
        {
            return Err(io::Error::other("compiler output content changed"));
        }
    }
    Ok(())
}

impl CheckedOutput {
    fn cleanup_outputs(&mut self) -> io::Result<()> {
        self.cleanup_outputs_before_remove(|| {})
    }

    fn cleanup_outputs_before_remove(&mut self, before_remove: impl FnOnce()) -> io::Result<()> {
        let mut journal = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.root.join("cleanup-journal.jsonl"))?;
        let outcome = self.cleanup_qualified(&mut journal, before_remove);
        if let Err(error) = outcome {
            let retained = format!(
                "compiler cleanup incomplete: {error}; journal and remaining outputs at {}",
                self.root.display()
            );
            if let Err(journal_error) = writeln!(journal,"{}",serde_json::json!({"state":"error", "error":error.to_string(),"remaining_roots":[self.root.join("target"),self.root.join("build")]})).and_then(|()|journal.sync_all()) {
                return Err(io::Error::other(format!("{retained}; error journal failed: {journal_error}")));
            }
            return Err(io::Error::other(retained));
        }
        Ok(())
    }

    fn cleanup_qualified(
        &mut self,
        journal: &mut fs::File,
        before_remove: impl FnOnce(),
    ) -> io::Result<()> {
        // Only successful run_checked creates this owner; no caller can substitute a PID.
        if self.root.canonicalize()? != self.root
            || !self.tree.members()?.is_empty()
            || self.tree.root_process.wait(Some(Duration::ZERO))?
                != win_custody::WaitOutcome::Signaled
        {
            return Err(io::Error::other("compiler tree not authoritatively idle"));
        }
        let mut entries = Vec::new();
        let root_metadata = checked_path(&self.root, &self.root)?;
        let root_id = file_id::get_high_res_file_id(&self.root)?;
        let root_guard = fs::OpenOptions::new()
            .access_mode(0)
            .custom_flags(0x0200_0000)
            .share_mode(0x1 | 0x2)
            .open(&self.root)?;
        for name in ["target", "build"] {
            catalog_output(&self.root.join(name), &self.root, &mut entries)?;
        }
        // Cargo 1.99's current native debug invocation creates these four real locks;
        // build-dir output is covered by its target-dir locks, not a fabricated lock.
        for profile in [
            self.root.join("target/debug"),
            self.root
                .join("target")
                .join(env!("HORIZON_BUILD_HOST"))
                .join("debug"),
        ] {
            for name in [".cargo-lock", ".cargo-artifact-lock"] {
                if !entries
                    .iter()
                    .any(|entry| entry.cargo_lock && entry.path == profile.join(name))
                {
                    return Err(io::Error::other(format!(
                        "actual Cargo output lock missing: {}",
                        profile.join(name).display()
                    )));
                }
            }
        }
        let manifest = entries.iter().map(|entry| serde_json::json!({
            "path":entry.path, "identity":format!("{:?}",entry.id), "directory":entry.metadata.is_dir(),
            "bytes":entry.metadata.len(), "modified":format!("{:?}",entry.metadata.modified()),
            "sha256":entry.hash, "allocated":entry.allocated, "cargo_lock":entry.cargo_lock,
        })).collect::<Vec<_>>();
        fs::write(
            self.root.join("cleanup-manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        writeln!(
            journal,
            "{}",
            serde_json::json!({"state":"qualified", "root":self.root,"tree_empty":true,"entries":entries.len()})
        )?;
        journal.sync_all()?;
        before_remove();
        let outcome = (|| {
            if file_id::get_high_res_file_id(&self.root)? != root_id
                || root_guard.metadata()?.created()? != root_metadata.created()?
            {
                return Err(io::Error::other("owned root changed"));
            }
            // Revalidate the entire catalog before the first removal: a changed entry
            // does not cause unrelated earlier catalog entries to be removed.
            for entry in &mut entries {
                validate_entry(entry, &self.root, true)?;
            }
            // Cargo locks remain acquired until all other files have been unlinked.
            for locks in [false, true] {
                for entry in entries
                    .iter_mut()
                    .filter(|entry| entry.metadata.is_file() && entry.cargo_lock == locks)
                {
                    validate_entry(entry, &self.root, false)?;
                    fs::remove_file(&entry.path)?;
                    if entry.path.try_exists()? {
                        return Err(io::Error::other("removed output remains present"));
                    }
                    writeln!(
                        journal,
                        "{}",
                        serde_json::json!({"state":"removed", "path":entry.path,"allocated":entry.allocated})
                    )?;
                    journal.sync_all()?;
                }
            }
            let mut directories = entries
                .iter_mut()
                .filter(|entry| entry.metadata.is_dir())
                .collect::<Vec<_>>();
            directories.sort_by_key(|entry| std::cmp::Reverse(entry.path.components().count()));
            for entry in directories {
                validate_entry(entry, &self.root, false)?;
                if fs::read_dir(&entry.path)?.next().is_some() {
                    return Err(io::Error::other("compiler directory no longer empty"));
                }
                drop(entry.guard.take());
                // Directory mtime changes when its cataloged children are removed.
                validate_entry(entry, &self.root, false)?;
                fs::remove_dir(&entry.path)?;
                writeln!(
                    journal,
                    "{}",
                    serde_json::json!({"state":"removed-directory", "path":entry.path})
                )?;
                journal.sync_all()?;
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => {
                writeln!(journal, "{}", serde_json::json!({"state":"complete"}))?;
                journal.sync_all()?;
                Ok(())
            }
            Err(error) => {
                writeln!(
                    journal,
                    "{}",
                    serde_json::json!({"state":"error", "error":error.to_string(),"remaining":entries.iter().filter(|entry|entry.path.exists()).map(|entry|&entry.path).collect::<Vec<_>>()})
                )?;
                journal.sync_all()?;
                Err(io::Error::other(format!(
                    "compiler cleanup incomplete: {error}; journal and remaining outputs at {}",
                    self.root.display()
                )))
            }
        }
    }
}

#[derive(Debug)]
struct ProcessTree {
    root_process: win_custody::Process,
    wait_handle: OwnedHandle,
    container: win_custody::Job,
    monitor: win32job::Job,
    seen: BTreeSet<usize>,
}

#[derive(Debug, Default)]
struct Teardown {
    root_signaled: bool,
    empty: bool,
    errors: Vec<String>,
}

impl ProcessTree {
    fn spawn(
        command: &Command,
        stdout: &fs::File,
        stderr: &fs::File,
        limits: Limits,
    ) -> io::Result<Self> {
        let container =
            win_custody::Job::create(&win_custody::JobLimits::new().kill_on_close(true))?;
        let mut monitor_limits = win32job::ExtendedLimitInfo::new();
        monitor_limits.limit_kill_on_job_close();
        let monitor =
            win32job::Job::create_with_limit_info(&monitor_limits).map_err(io::Error::from)?;
        let nul = win_custody::null_device()?;
        let mut launch = win_custody::Launch::new(PathBuf::from(command.get_program()));
        launch
            .args(command.get_args())
            .stdin(&nul)
            .stdout(stdout)
            .stderr(stderr)
            .create_no_window(true);
        if let Some(directory) = command.get_current_dir() {
            launch.current_dir(directory);
        }
        for (name, value) in command.get_envs() {
            match value {
                Some(value) => {
                    launch.env(name, value);
                }
                None => {
                    launch.env_remove(name);
                }
            }
        }
        let suspended = launch.spawn_suspended()?;
        let wait_handle = match suspended.process().as_handle().try_clone_to_owned() {
            Ok(handle) => handle,
            Err(error) => {
                return Err(rollback_suspended(
                    suspended, &container, false, error, limits,
                ))
            }
        };
        if let Err(error) = container.assign(suspended.process()) {
            return Err(rollback_suspended(
                suspended, &container, false, error, limits,
            ));
        }
        // This is a borrowed value from the new live owned process, passed only to this
        // published safe call. No pointer is fabricated, dereferenced, owned or retained.
        if let Err(error) = monitor.assign_process(suspended.process().as_raw_handle() as isize) {
            return Err(rollback_suspended(
                suspended,
                &container,
                true,
                error.into(),
                limits,
            ));
        }
        let root_process = match suspended.resume() {
            Ok(process) => process,
            Err(error) => {
                let mut seen = BTreeSet::new();
                let stopped = terminate_parts(
                    &container,
                    &monitor,
                    &wait_handle,
                    &mut seen,
                    Instant::now() + limits.teardown,
                );
                return Err(io::Error::other(format!(
                    "resume failed: {error}; rollback {stopped:?}"
                )));
            }
        };
        Ok(Self {
            root_process,
            wait_handle,
            container,
            monitor,
            seen: BTreeSet::new(),
        })
    }

    fn members(&mut self) -> io::Result<Vec<usize>> {
        let members = self
            .monitor
            .query_process_id_list()
            .map_err(io::Error::from)?;
        self.seen.extend(members.iter().copied());
        Ok(members)
    }

    fn terminate(&mut self, deadline: Instant) -> Teardown {
        terminate_parts(
            &self.container,
            &self.monitor,
            &self.wait_handle,
            &mut self.seen,
            deadline,
        )
    }
}

// Called while the process is still suspended: it cannot have spawned descendants.
fn rollback_suspended(
    suspended: win_custody::SuspendedChild,
    container: &win_custody::Job,
    assigned: bool,
    primary: io::Error,
    limits: Limits,
) -> io::Error {
    let deadline = Instant::now() + limits.teardown;
    let mut errors = vec![format!("startup failed: {primary}")];
    if assigned {
        if let Err(error) = container.terminate(1) {
            errors.push(format!("job terminate failed: {error}"));
        }
    }
    if let Err(error) = suspended.process().terminate(1) {
        errors.push(format!("suspended root terminate failed: {error}"));
    }
    match suspended
        .process()
        .wait(Some(deadline.saturating_duration_since(Instant::now())))
    {
        Ok(win_custody::WaitOutcome::Signaled) => {}
        outcome => errors.push(format!(
            "suspended root completion unconfirmed: {outcome:?}"
        )),
    }
    // SuspendedChild/Job destructors only perform their nonblocking ownership fallback.
    io::Error::other(errors.join("; "))
}

fn terminate_parts(
    container: &win_custody::Job,
    monitor: &win32job::Job,
    wait_handle: &OwnedHandle,
    seen: &mut BTreeSet<usize>,
    deadline: Instant,
) -> Teardown {
    let mut result = Teardown::default();
    if let Err(error) = container.terminate(1) {
        result.errors.push(format!("job terminate failed: {error}"));
    }
    match win_custody::wait(
        wait_handle,
        Some(deadline.saturating_duration_since(Instant::now())),
    ) {
        Ok(win_custody::WaitOutcome::Signaled) => result.root_signaled = true,
        outcome => result
            .errors
            .push(format!("root completion unconfirmed: {outcome:?}")),
    }
    match poll_completion(deadline, seen, || {
        monitor.query_process_id_list().map_err(io::Error::from)
    }) {
        Ok(()) => result.empty = true,
        Err(error) => result.errors.push(error.to_string()),
    }
    result
}

fn poll_completion(
    deadline: Instant,
    seen: &mut BTreeSet<usize>,
    mut query: impl FnMut() -> io::Result<Vec<usize>>,
) -> io::Result<()> {
    loop {
        let members = query().map_err(|error| {
            io::Error::other(format!("monitor completion query failed: {error}"))
        })?;
        if members.is_empty() {
            return Ok(());
        }
        seen.extend(members);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::other(
                "descendant completion unconfirmed at teardown deadline",
            ));
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

fn tree_receipt(
    root: &Path,
    tree: &ProcessTree,
    completed: bool,
    stopped: &Teardown,
    reason: &str,
) -> io::Result<()> {
    fs::write(
        root.join("process-tree.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "completed":completed, "seen_pids":tree.seen, "empty":stopped.empty,
            "root_signaled":stopped.root_signaled, "teardown_errors":stopped.errors, "reason":reason,
        }))?,
    )
}

fn run_checked(command: &mut Command, root: &Path) -> io::Result<CheckedOutput> {
    run_checked_with_limits(command, root, Limits::default())
}

fn run_checked_with_limits(
    command: &Command,
    root: &Path,
    limits: Limits,
) -> io::Result<CheckedOutput> {
    let deadline = Instant::now() + limits.operation;
    let stdout_path = root.join("cargo.stdout.log");
    let stderr_path = root.join("cargo.stderr.log");
    let stdout = fs::File::create(&stdout_path)?;
    let stderr = fs::File::create(&stderr_path)?;
    let mut tree = ProcessTree::spawn(command, &stdout, &stderr, limits)?;
    let outcome = (|| {
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::other("fixture operational deadline exceeded"));
            }
            tree.members()?;
            if fs::metadata(&stdout_path)?.len() > limits.log_bytes
                || fs::metadata(&stderr_path)?.len() > limits.log_bytes
            {
                return Err(io::Error::other("fixture output bound exceeded"));
            }
            if tree.root_process.wait(Some(Duration::ZERO))? == win_custody::WaitOutcome::Signaled {
                let code = tree
                    .root_process
                    .exit_code()?
                    .ok_or_else(|| io::Error::other("root exit status unavailable"))?;
                if code != 0 {
                    return Err(io::Error::other(format!("fixture Cargo status {code}")));
                }
                // Root exit can precede a legitimate Cargo/rustc teardown. It is not
                // completion until the owned monitored tree is empty. Keep polling
                // under the original operation deadline, never a renewed allowance.
                if tree.members()?.is_empty() {
                    if Instant::now() >= deadline {
                        return Err(io::Error::other(
                            "fixture operational deadline exceeded before empty-tree confirmation",
                        ));
                    }
                    if fs::metadata(&stdout_path)?.len() > limits.log_bytes
                        || fs::metadata(&stderr_path)?.len() > limits.log_bytes
                    {
                        return Err(io::Error::other("fixture output bound exceeded after exit"));
                    }
                    return fs::read_to_string(&stdout_path);
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::other("fixture operational deadline exceeded"));
            }
            std::thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    })();
    match outcome {
        Ok(output) => {
            tree_receipt(
                root,
                &tree,
                true,
                &Teardown {
                    root_signaled: true,
                    empty: true,
                    errors: Vec::new(),
                },
                "successful root and empty monitored tree",
            )?;
            // The files must close before any compiler-output cleanup can begin.
            drop(stdout);
            drop(stderr);
            Ok(CheckedOutput {
                stdout: output,
                tree,
                root: root.canonicalize()?,
            })
        }
        Err(primary) => {
            let stopped = tree.terminate(Instant::now() + limits.teardown);
            let mut message = format!(
                "{primary}; teardown {stopped:?}; all outputs retained at {}",
                root.display()
            );
            if let Err(error) = tree_receipt(root, &tree, false, &stopped, &primary.to_string()) {
                message.push_str(&format!("; receipt failed: {error}"));
            }
            let mut diagnostics = Vec::new();
            match fs::File::open(&stderr_path)
                .and_then(|file| file.take(64 * 1024).read_to_end(&mut diagnostics))
            {
                Ok(_) => message.push_str(&format!("\n{}", String::from_utf8_lossy(&diagnostics))),
                Err(error) => message.push_str(&format!("; stderr read failed: {error}")),
            }
            Err(io::Error::other(message))
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
    #[test]
    fn parent_exit_before_live_descendant_is_not_success() {
        let root = fresh_root().unwrap();
        let mut command = process::command(console_probe());
        command
            .arg("--lifecycle-parent")
            .arg(&root)
            .arg("parent-exits-first");
        let start = Instant::now();
        let error = run_checked_with_limits(
            &command,
            &root,
            Limits {
                operation: Duration::from_millis(350),
                ..Limits::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(root.join("grandchild.ready").is_file());
        assert!(error.contains("operational deadline"), "{error}");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "child expired naturally"
        );
        assert!(!root.join("grandchild.finished").exists());
        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("process-tree.json")).unwrap()).unwrap();
        assert_eq!(receipt["completed"], false, "{receipt}");
        assert_eq!(receipt["empty"], true, "{receipt}");
        assert_eq!(receipt["root_signaled"], true, "{receipt}");
    }

    #[test]
    fn successful_automatic_probe_releases_only_its_compiler_outputs() {
        let executable = console_probe();
        let root = executable.parent().unwrap();
        assert!(executable.is_file());
        assert!(root.join("artifact.json").is_file());
        assert!(root.join("cargo.stdout.log").is_file());
        assert!(root.join("cleanup-manifest.json").is_file());
        assert!(fs::read_to_string(root.join("cleanup-journal.jsonl"))
            .unwrap()
            .contains("\"state\":\"complete\""));
        assert!(
            !root.join("target").exists(),
            "successful fixture retained target"
        );
        assert!(
            !root.join("build").exists(),
            "successful fixture retained build"
        );
    }

    #[test]
    fn cleanup_lock_contention_and_changed_outputs_retain_diagnostics() {
        for behavior in ["locked", "replaced", "missing-lock"] {
            // These are real fresh Cargo outputs and real Cargo locks, not invented locks.
            let (executable, mut output) = prepare_probe().unwrap();
            let lock_path = output.root.join("target/debug/.cargo-lock");
            let lock = if behavior == "locked" {
                let lock = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&lock_path)
                    .unwrap();
                fs2::FileExt::try_lock_exclusive(&lock).unwrap();
                Some(lock)
            } else {
                None
            };
            let root = output.root.clone();
            let changed = root.join("build/.rustc_info.json");
            if behavior == "missing-lock" {
                fs::rename(&lock_path, root.join("removed-lock")).unwrap();
            }
            let error = if behavior == "replaced" {
                output.cleanup_outputs_before_remove(|| {
                    // Delete-sharing retains the old deny-write handle but permits this
                    // controlled path replacement. High-res identity must reject it.
                    fs::rename(&changed, root.join("replaced-original.json")).unwrap();
                    fs::write(&changed, "controlled replacement").unwrap();
                })
            } else {
                output.cleanup_outputs()
            }
            .unwrap_err()
            .to_string();
            assert!(error.contains("cleanup incomplete"), "{error}");
            assert!(root.join("target").is_dir());
            assert!(root.join("build").is_dir());
            assert!(executable.is_file());
            assert!(root.join("artifact.json").is_file());
            let journal = fs::read_to_string(root.join("cleanup-journal.jsonl")).unwrap();
            assert!(journal.contains("\"state\":\"error\""), "{journal}");
            assert!(!journal.contains("\"state\":\"removed\""), "{journal}");
            drop(lock);
        }
    }

    #[test]
    fn output_catalog_rejects_reparse_outside_and_live_writes() {
        let raw_root = fresh_root().unwrap();
        let raw_outside = fresh_root().unwrap();
        let root = raw_root.canonicalize().unwrap();
        let outside = raw_outside.canonicalize().unwrap();
        assert!(catalog_output(&outside, &root, &mut Vec::new()).is_err());
        let file = root.join("output.bin");
        fs::write(&file, "owned test data").unwrap();
        let mut entries = Vec::new();
        catalog_output(&file, &root, &mut entries).unwrap();
        assert!(
            fs::OpenOptions::new().write(true).open(&file).is_err(),
            "deny-write guard not effective"
        );
        fs::rename(&file, root.join("renamed.bin")).unwrap();
        fs::write(&file, "owned test data").unwrap();
        assert!(validate_entry(&mut entries[0], &root, false).is_err());
        // Symlinks require a privilege not held by this host. A narrowly constrained,
        // job-owned PowerShell fixture creates a real junction without changing policy.
        let link = root.join("reparse");
        let raw_link = raw_root.join("reparse");
        for path in [&raw_link, &raw_outside] {
            let text = path.to_str().expect("test path must be Unicode");
            assert!(
                !text
                    .chars()
                    .any(|character| character.is_control() || "%!^&|<>\"[]*?".contains(character)),
                "provider wildcard or control form in test path rejected"
            );
        }
        let sentinel = outside.join("sentinel.txt");
        fs::write(&sentinel, "unrelated owned target must remain").unwrap();
        let mut command = process::command(
            PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        );
        command
            .current_dir(&raw_root)
            .args(["-NoProfile", "-NonInteractive", "-Command",
                "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:HORIZON_TEST_JUNCTION_LINK -Target $env:HORIZON_TEST_JUNCTION_TARGET | Out-Null"])
            .env("HORIZON_TEST_JUNCTION_LINK", &raw_link)
            .env("HORIZON_TEST_JUNCTION_TARGET", &raw_outside);
        eprintln!("owned junction fixture: {command:?}");
        run_checked_with_limits(
            &command,
            &root,
            Limits {
                operation: Duration::from_secs(2),
                ..Limits::default()
            },
        )
        .unwrap();
        let metadata = fs::symlink_metadata(&link).unwrap();
        assert_ne!(metadata.file_attributes() & 0x400, 0);
        let rejected = catalog_output(&link, &root, &mut Vec::new()).is_err();
        // Nonrecursive unlink of this exact junction never traverses its target.
        fs::remove_dir(&link).unwrap();
        assert!(rejected);
        assert_eq!(
            fs::read_to_string(&sentinel).unwrap(),
            "unrelated owned target must remain"
        );
    }

    #[test]
    fn active_owner_and_spawn_failure_never_authorize_cleanup() {
        let root = fresh_root().unwrap().canonicalize().unwrap();
        let stdout = fs::File::create(root.join("stdout.log")).unwrap();
        let stderr = fs::File::create(root.join("stderr.log")).unwrap();
        let mut command = process::command(console_probe());
        command.arg("--lifecycle-parent").arg(&root).arg("timeout");
        let tree = ProcessTree::spawn(&command, &stdout, &stderr, Limits::default()).unwrap();
        let mut output = CheckedOutput {
            stdout: String::new(),
            tree,
            root: root.clone(),
        };
        assert!(output
            .cleanup_outputs()
            .unwrap_err()
            .to_string()
            .contains("not authoritatively idle"));
        let stopped = output
            .tree
            .terminate(Instant::now() + Duration::from_secs(5));
        assert!(stopped.empty && stopped.root_signaled, "{stopped:?}");
        let other = fresh_root().unwrap();
        let missing = process::command(other.join("does-not-exist.exe"));
        assert!(run_checked_with_limits(&missing, &other, Limits::default()).is_err());
        assert!(other.join("cargo.stdout.log").is_file());
        assert!(other.join("cargo.stderr.log").is_file());
    }

    #[test]
    fn teardown_query_error_and_expiry_are_not_empty_success() {
        let mut seen = BTreeSet::new();
        let error = poll_completion(Instant::now(), &mut seen, || {
            Err(io::Error::other("injected query failure"))
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("injected query failure"), "{error}");
        let error = poll_completion(Instant::now(), &mut seen, || Ok(vec![123]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("unconfirmed at teardown deadline"),
            "{error}"
        );
        assert!(seen.contains(&123));
    }

    #[test]
    fn timeout_and_overflow_terminate_the_owned_parent_and_grandchild() {
        for behavior in ["timeout", "overflow"] {
            let root = fresh_root().unwrap();
            let mut command = process::command(console_probe());
            command.arg("--lifecycle-parent").arg(&root).arg(behavior);
            let start = Instant::now();
            let error = run_checked_with_limits(
                &command,
                &root,
                Limits {
                    operation: Duration::from_millis(350),
                    log_bytes: if behavior == "overflow" {
                        1024
                    } else {
                        32 * 1024 * 1024
                    },
                    teardown: Duration::from_secs(5),
                },
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains(if behavior == "overflow" {
                    "output bound"
                } else {
                    "deadline"
                }),
                "{error}"
            );
            assert!(
                start.elapsed() < Duration::from_secs(6),
                "unbounded teardown"
            );
            let receipt: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join("process-tree.json")).unwrap()).unwrap();
            for name in ["parent.ready", "grandchild.ready"] {
                let pid: usize = fs::read_to_string(root.join(name))
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!(
                    receipt["seen_pids"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(pid)),
                    "monitor never saw {name}: {receipt}"
                );
            }
            assert_eq!(receipt["empty"], true, "{receipt}");
            assert_eq!(receipt["root_signaled"], true, "{receipt}");
            assert!(
                !root.join("grandchild.finished").exists(),
                "dummy reached its natural lifetime instead of being terminated"
            );
            assert!(root.join("cargo.stderr.log").is_file());
        }
    }
}
