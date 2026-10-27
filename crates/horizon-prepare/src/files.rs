//! Independent copies, using filesystem cloning when available. Never hard links: exporters
//! rewrite prepared resources in place. Checked imports publish only after verifying the copy.

use crate::{digest, hex};
use sha2::Digest;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn clone_file(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let source = fs::File::open(from)?;
        let target = fs::File::create(to)?;
        rustix::fs::ioctl_ficlone(&target, &source).map_err(Into::into)
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let path = |p: &Path| {
            CString::new(p.as_os_str().as_bytes())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
        };
        let (source, target) = (path(from)?, path(to)?);
        // SAFETY: both paths are valid NUL-terminated strings for the duration of the call.
        if unsafe { libc::clonefile(source.as_ptr(), target.as_ptr(), 0) } == 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (from, to);
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Copies an independent file, attempting a reflink first. The destination may be replaced.
pub fn clone_or_copy(from: &Path, to: &Path) -> io::Result<()> {
    clone_file(from, to).or_else(|_| fs::copy(from, to).map(|_| ()))
}

fn copy_hash<D: Digest>(from: &Path, to: &Path) -> io::Result<String> {
    let mut source = fs::File::open(from)?;
    let mut target = fs::File::create(to)?;
    let mut hash = D::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let n = source.read(&mut buffer)?;
        if n == 0 {
            return Ok(hex(&hash.finalize()));
        }
        target.write_all(&buffer[..n])?;
        hash.update(&buffer[..n]);
    }
}

/// A verified replacement staged beside its destination. Dropping an unfinished set cleans
/// every temporary file, so a corrupt late panorama face cannot replace earlier placeholders.
pub(crate) struct Import {
    temporary: Option<PathBuf>,
    destination: PathBuf,
}

impl Import {
    pub(crate) fn stage(
        source: &Path,
        destination: &Path,
        checksum: &str,
        algorithm: &str,
        verified: bool,
    ) -> Result<Self, String> {
        let mut import = Self {
            temporary: None,
            destination: destination.into(),
        };
        if verified || (destination.is_file() && digest(destination, algorithm)? == checksum) {
            return Ok(import);
        }
        fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
        static COPIES: AtomicUsize = AtomicUsize::new(0);
        let temporary = destination.with_file_name(format!(
            ".{}.partial-{}-{}",
            destination.file_name().unwrap().to_string_lossy(),
            std::process::id(),
            COPIES.fetch_add(1, Ordering::Relaxed)
        ));
        import.temporary = Some(temporary.clone());
        let actual = if clone_file(source, &temporary).is_ok() {
            digest(&temporary, algorithm)?
        } else {
            match algorithm {
                "sha1" => copy_hash::<sha1::Sha1>(source, &temporary),
                _ => copy_hash::<sha2::Sha256>(source, &temporary),
            }
            .map_err(|e| format!("{}: {e}", source.display()))?
        };
        if actual != checksum {
            return Err(format!(
                "{} is not the pinned file: expected {algorithm} {checksum}, got {actual}",
                source.display()
            ));
        }
        Ok(import)
    }

    pub(crate) fn commit(mut self) -> Result<PathBuf, String> {
        if let Some(temporary) = &self.temporary {
            fs::rename(temporary, &self.destination)
                .map_err(|e| format!("{}: {e}", self.destination.display()))?;
        }
        self.temporary = None;
        Ok(self.destination.clone())
    }
}

impl Drop for Import {
    fn drop(&mut self) {
        if let Some(temporary) = &self.temporary {
            let _ = fs::remove_file(temporary);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "horizon-file-copy-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create fixture: {error}"),
                }
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove file-copy fixture");
        }
    }

    // A reversed descriptor pair, lost overwrite, or hard-link substitution breaks these
    // literal byte assertions. Unsupported reflinks are not evidence of clone success.
    #[cfg(target_os = "linux")]
    fn cloned_or_unsupported(result: io::Result<()>) -> bool {
        match result {
            Ok(()) => {
                writeln!(
                    io::stderr().lock(),
                    "DIRECT_FICLONE_SUPPORTED: real file clone succeeded"
                )
                .unwrap();
                true
            }
            Err(error) if matches!(error.raw_os_error(), Some(code) if code == rustix::io::Errno::OPNOTSUPP.raw_os_error() || code == rustix::io::Errno::NOTTY.raw_os_error()) =>
            {
                writeln!(io::stderr().lock(),
                    "DIRECT_FICLONE_UNSUPPORTED: errno={:?}, {error}; positive reflink proof unavailable",
                    error.raw_os_error()).unwrap();
                false
            }
            Err(error) => panic!("unexpected direct clone failure: {error}"),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn direct_clone_overwrites_without_linking_either_direction() {
        let fixture = Fixture::new();
        let source = fixture.path("source");
        let target = fixture.path("target");
        fs::write(&source, b"source bytes").unwrap();
        fs::write(&target, b"old destination longer than source").unwrap();
        if !cloned_or_unsupported(clone_file(&source, &target)) {
            assert_eq!(fs::read(&source).unwrap(), b"source bytes");
            assert_eq!(fs::read(&target).unwrap(), b"");
            return;
        }
        assert_eq!(fs::read(&target).unwrap(), b"source bytes");
        fs::write(&target, b"target rewrite").unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"source bytes");
        fs::write(&source, b"source rewrite").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"target rewrite");
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[test]
    fn direct_clone_explicitly_reports_unsupported_without_touching_files() {
        let fixture = Fixture::new();
        let source = fixture.path("source");
        let target = fixture.path("target");
        fs::write(&source, b"source bytes").unwrap();
        fs::write(&target, b"old destination").unwrap();
        assert_eq!(
            clone_file(&source, &target).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(fs::read(&source).unwrap(), b"source bytes");
        assert_eq!(fs::read(&target).unwrap(), b"old destination");
    }

    #[test]
    fn clone_or_copy_overwrites_and_keeps_writes_independent() {
        let fixture = Fixture::new();
        let source = fixture.path("source");
        let target = fixture.path("target");
        fs::write(&source, b"source bytes").unwrap();
        fs::write(&target, b"old destination longer than source").unwrap();
        clone_or_copy(&source, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"source bytes");
        fs::write(&target, b"target rewrite").unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"source bytes");
        fs::write(&source, b"source rewrite").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"target rewrite");
    }

    #[test]
    fn missing_source_does_not_clobber_existing_destination() {
        let fixture = Fixture::new();
        let source = fixture.path("missing");
        let target = fixture.path("target");
        fs::write(&target, b"old destination").unwrap();
        assert!(clone_file(&source, &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old destination");
        assert_eq!(
            clone_or_copy(&source, &target).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(fs::read(&target).unwrap(), b"old destination");
    }

    #[test]
    fn invalid_destination_reports_error_without_changing_source() {
        let fixture = Fixture::new();
        let source = fixture.path("source");
        let target = fixture.path("missing-parent/target");
        fs::write(&source, b"source bytes").unwrap();
        assert!(clone_file(&source, &target).is_err());
        assert_eq!(
            clone_or_copy(&source, &target).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(fs::read(&source).unwrap(), b"source bytes");
        assert!(!target.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn direct_clone_follows_symlinks_and_accepts_non_utf8_paths() {
        use std::ffi::OsStr;
        use std::os::unix::{
            ffi::OsStrExt,
            fs::{symlink, PermissionsExt},
        };

        let fixture = Fixture::new();
        let source = fixture.0.join(OsStr::from_bytes(b"source-\xff"));
        let target = fixture.0.join(OsStr::from_bytes(b"target-\xfe"));
        let source_link = fixture.path("source-link");
        let target_link = fixture.path("target-link");
        fs::write(&source, b"source bytes").unwrap();
        fs::write(&target, b"old destination").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&source, &source_link).unwrap();
        symlink(&target, &target_link).unwrap();
        let supported = cloned_or_unsupported(clone_file(&source_link, &target_link));
        assert!(fs::symlink_metadata(&source_link).unwrap().is_symlink());
        assert!(fs::symlink_metadata(&target_link).unwrap().is_symlink());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(fs::read(&source).unwrap(), b"source bytes");
        assert_eq!(
            fs::read(&target).unwrap(),
            if supported {
                &b"source bytes"[..]
            } else {
                &b""[..]
            }
        );
        // The same paths must still copy correctly when a reflink is unsupported.
        clone_or_copy(&source_link, &target_link).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"source bytes");
    }

    #[test]
    fn rejected_checksum_preserves_destination_and_removes_partial_file() {
        let fixture = Fixture::new();
        let source = fixture.path("source");
        let target = fixture.path("target");
        fs::write(&source, b"abc").unwrap();
        fs::write(&target, b"old destination").unwrap();
        for algorithm in ["sha1", "sha256"] {
            let result = Import::stage(&source, &target, "bad hash", algorithm, false);
            assert!(result.is_err());
            assert_eq!(fs::read(&target).unwrap(), b"old destination");
            assert_eq!(fs::read(&source).unwrap(), b"abc");
            assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 2);
        }
    }

    #[test]
    fn dropping_uncommitted_import_removes_only_staging_file() {
        let fixture = Fixture::new();
        let source = fixture.path("source");
        let target = fixture.path("target");
        fs::write(&source, b"abc").unwrap();
        fs::write(&target, b"old destination").unwrap();
        let staged = Import::stage(
            &source,
            &target,
            "a9993e364706816aba3e25717850c26c9cd0d89d",
            "sha1",
            false,
        )
        .unwrap();
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 3);
        assert_eq!(fs::read(&target).unwrap(), b"old destination");
        drop(staged);
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 2);
        assert_eq!(fs::read(&target).unwrap(), b"old destination");
        assert_eq!(fs::read(&source).unwrap(), b"abc");
    }

    #[test]
    fn copies_are_independent_and_checked_before_publication() {
        let fixture = Fixture::new();
        let dir = &fixture.0;
        let source = dir.join("source");
        let target = dir.join("target");
        let data = vec![42; 200_000]; // More than one streaming buffer.
        fs::write(&source, &data).unwrap();
        for algorithm in ["sha1", "sha256"] {
            let checksum = digest(&source, algorithm).unwrap();
            Import::stage(&source, &target, &checksum, algorithm, false)
                .unwrap()
                .commit()
                .unwrap();
            assert_eq!(digest(&target, algorithm).unwrap(), checksum);
            fs::write(&target, b"old destination").unwrap();
            assert_eq!(fs::read(&source).unwrap(), data);
            assert!(Import::stage(&source, &target, "bad hash", algorithm, false).is_err());
            assert_eq!(fs::read(&target).unwrap(), b"old destination");
            // Explicitly exercise ordinary streaming copies even on a reflink filesystem.
            let streamed = if algorithm == "sha1" {
                copy_hash::<sha1::Sha1>(&source, &target).unwrap()
            } else {
                copy_hash::<sha2::Sha256>(&source, &target).unwrap()
            };
            assert_eq!(streamed, checksum);
            assert_eq!(fs::read(&target).unwrap(), data);
        }
        clone_or_copy(&source, &target).unwrap();
        fs::write(&target, b"rewrite").unwrap();
        assert_eq!(fs::read(&source).unwrap(), data);
        let staged = Import::stage(
            &source,
            &target,
            &digest(&source, "sha1").unwrap(),
            "sha1",
            false,
        )
        .unwrap();
        assert_eq!(fs::read_dir(dir).unwrap().count(), 3);
        drop(staged);
        assert_eq!(fs::read_dir(dir).unwrap().count(), 2);
    }
}
