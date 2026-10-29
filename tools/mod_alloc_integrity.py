"""Registered mod-alloc boundary; source audit is not compiler/runtime proof."""
import hashlib
import json
import os
from pathlib import Path
import stat

FORK = Path('third_party/mod-alloc')
MANIFEST = FORK / 'Cargo.toml'
# Any change to this reviewed registry needs a new review, not a folder exclusion.
REGISTRY_SHA256 = '0f11f0e5ba3e998bed4e8f781845fbac6577e59374fea5c40b55ab713000306a'
BAN = '#![forbid(unsafe_code)]'


def _digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _root_forbid(path: Path) -> bool:
    # Require an unconditional inner attribute before any other token, not a
    # substring in a block comment, cfg_attr, example or inactive module.
    for line in path.read_text(encoding='utf-8').splitlines():
        line = line.strip()
        if line and not line.startswith('//!'):
            return line == BAN
    return False


def _item(source: str, token: str) -> str:
    return source[source.index(token):].split('\n}\n', 1)[0] + '\n}\n'


def audit_mod_alloc(root: Path) -> list[str]:
    """Return failures in the exact registered fork, including inactive code."""
    fork = root / FORK
    issues: list[str] = []
    try:
        # Inventory before reading, including inactive source; never follow links.
        rust_files: set[str] = set()
        for path in [root, root / 'third_party', fork]:
            info = path.lstat()
            if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
                raise ValueError(f'link/reparse boundary: {path}')
        def failed(error: OSError) -> None:
            raise error
        for directory, dirs, files in os.walk(fork, followlinks=False, onerror=failed):
            for name in dirs + files:
                path = Path(directory) / name
                info = path.lstat()
                if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & 0x400:
                    raise ValueError(f'link/reparse boundary: {path}')
                if path.suffix == '.rs':
                    rust_files.add(path.relative_to(fork).as_posix())
            if 'Cargo.toml' in files and Path(directory) != fork:
                raise ValueError('unregistered nested Cargo package')
        registry_path = fork / 'FORK.json'
        if _digest(registry_path) != REGISTRY_SHA256:
            raise ValueError('fork registry differs from reviewed binding')
        registry = json.loads(registry_path.read_text(encoding='utf-8'))
        if rust_files != set(registry['rust_files']):
            issues.append('unregistered or missing Rust source')
        if _digest(fork / 'Cargo.toml') != registry['manifest_sha256']:
            issues.append('fork manifest/feature/target/dependency envelope changed')
        if _digest(fork / 'src/lib.rs') != registry['frozen_lib_envelope_sha256']:
            issues.append('lib.rs frozen unsafe/safe-bridge envelope changed')
        source = (fork / 'src/lib.rs').read_text(encoding='utf-8')
        for token, digest in registry['frozen_lib_items'].items():
            if hashlib.sha256(_item(source, token).encode()).hexdigest() != digest:
                issues.append(f'upstream unsafe item changed: {token}')
        for group in ['unchanged_source', 'legacy_targets']:
            for name, digest in registry[group].items():
                if _digest(fork / name) != digest:
                    issues.append(f'frozen source changed: {name}')
        for name in registry['owned_roots']:
            if not _root_forbid(fork / name):
                issues.append(f'owned root lacks effective unconditional forbid: {name}')
        for name in registry['legacy_targets']:
            if not _root_forbid(fork / name):
                issues.append(f'legacy target root lacks forbid: {name}')
        # Only the crate's upstream build-script body may follow its new forbid.
        if _digest(fork / 'build.rs') != registry.get('build_sha256'):
            issues.append('registered build script changed')
    except (OSError, UnicodeError, ValueError, KeyError, TypeError) as error:
        issues.append(f'cannot verify registered fork: {error}')
    return [f'{FORK}: {issue}' for issue in issues]
