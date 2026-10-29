"""Exact scoped dependency boundary, not dependency or native safety proof."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat

FORK = Path("third_party/thread-priority")
MANIFEST = FORK / "Cargo.toml"
# Review the registry, source diff and native evidence before changing this binding.
REGISTRY_SHA256 = "a24fb8716fcfd25912a4465b050ce9f7c45cac987311bf3fd8e771c82df6af5e"
INSERTION = b'                #[cfg(any(target_os = "linux", target_os = "android"))]\n                #[forbid(unsafe_code)]\n                {\n                    // Idle uses static priority zero, not a request to change niceness.\n                    if policy == ThreadSchedulePolicy::Normal(NormalThreadSchedulePolicy::Idle) {\n                        return Ok(());\n                    }\n                }\n\n'


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _reject_link(path: Path) -> None:
    info = path.lstat()
    if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
        raise ValueError(f"link/reparse boundary: {path}")


def audit_thread_priority(root: Path) -> list[str]:
    """Check every published file and the exact sole safe inserted block."""
    fork = root / FORK
    issues: list[str] = []
    try:
        for path in [root, root / "third_party", fork]:
            _reject_link(path)
        files: set[str] = set()

        def failed(error: OSError) -> None:
            raise error

        for directory, dirs, names in os.walk(fork, followlinks=False, onerror=failed):
            for name in dirs + names:
                _reject_link(Path(directory) / name)
            for name in names:
                files.add((Path(directory) / name).relative_to(fork).as_posix())
        registry_path = fork / "FORK.json"
        if _sha(registry_path.read_bytes()) != REGISTRY_SHA256:
            raise ValueError("fork registry differs from reviewed binding")
        registry = json.loads(registry_path.read_text(encoding="utf8"))
        if files != set(registry["files"]) | {"FORK.json"}:
            issues.append("unregistered or missing fork file (including inactive source/build inputs)")
        for name, expected in registry["files"].items():
            if _sha((fork / name).read_bytes()) != expected:
                issues.append(f"registered file changed: {name}")
        unix = (fork / "src/unix.rs").read_bytes()
        if unix.count(INSERTION) != 1:
            issues.append("exact forbid-protected safe Idle block missing or changed")
        elif _sha(unix.replace(INSERTION, b"", 1)) != registry["upstream_files"]["src/unix.rs"]:
            issues.append("upstream Unix envelope or unsafe body changed")
        manifest = (fork / "Cargo.toml").read_bytes()
        suffix = b"\n[workspace]\n"
        if not manifest.endswith(suffix) or _sha(manifest[:-len(suffix)]) != registry["upstream_files"]["Cargo.toml"]:
            issues.append("published manifest changed beyond standalone workspace marker")
        for name, expected in registry["upstream_files"].items():
            if name not in {"src/unix.rs", "Cargo.toml"} and _sha((fork / name).read_bytes()) != expected:
                issues.append(f"published upstream file changed: {name}")
    except (OSError, UnicodeError, ValueError, KeyError, TypeError) as error:
        issues.append(f"cannot verify registered fork: {error}")
    return [f"{FORK}: {issue}" for issue in issues]
