#!/usr/bin/env python3
"""Fail-closed Cargo lint audit. Python 3.10 needs requirements-dev.txt installed.

Run `python tools/check_rust_safety.py`; no parser is installed automatically.
This is manifest coverage, not compiler or runtime proof.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import stat
import sys

try:
    import tomllib as toml_parser
except ModuleNotFoundError:
    try:
        import tomli as toml_parser
    except ModuleNotFoundError:
        toml_parser = None

ROOT = Path(__file__).resolve().parent.parent
WORKSPACES = {Path('Cargo.toml'), Path('launcher/Cargo.toml'),
              Path('launcher/tests/support/console-probe/Cargo.toml'),
              Path('crates/horizon-telemetry-server/Cargo.toml'),
              Path('crates/horizon-mod-registry/Cargo.toml'), Path('third_party/mod-alloc/Cargo.toml'), Path('third_party/thread-priority/Cargo.toml')}
SKIP_DIRS = {Path('.git'), Path('.local'), Path('.superpowers'), Path('target'),
             Path('third_party/wgpu-hal'), Path('launcher/target'),
             Path('crates/horizon-telemetry-server/target'), Path('crates/horizon-mod-registry/target')}


def reject_links(path: Path) -> None:
    info = path.lstat()
    if stat.S_ISLNK(info.st_mode):
        raise ValueError(f'symlink requires manual scope review: {path}')
    if getattr(info, 'st_file_attributes', 0) & getattr(stat, 'FILE_ATTRIBUTE_REPARSE_POINT', 0x400):
        raise ValueError(f'reparse point requires manual scope review: {path}')


def contained_path(root: Path, value: object, label: str) -> None:
    if not isinstance(value, str) or not value:
        raise ValueError(f'{label}: missing or invalid path')
    candidate = Path(value)
    if not candidate.is_absolute():
        raise ValueError(f'{label}: Cargo returned a relative path: {candidate}')
    try:
        parts = candidate.relative_to(root).parts
    except ValueError as error:
        raise ValueError(f'{label}: outside project root: {candidate}') from error
    current = root
    for part in parts:
        if part == '..':
            current = current.parent
            try:
                current.relative_to(root)
            except ValueError as error:
                raise ValueError(f'{label}: outside project root: {candidate}') from error
        elif part != '.':
            current = current / part
            reject_links(current)
    try:
        current.resolve(strict=True).relative_to(root)
    except ValueError as error:
        raise ValueError(f'{label}: outside project root: {candidate}') from error


def manifests(root: Path) -> list[Path]:
    found: list[Path] = []
    def fail_walk(error: OSError) -> None:
        raise error

    for directory, dirs, files in os.walk(root, followlinks=False, onerror=fail_walk):
        for name in dirs + files:
            reject_links(Path(directory) / name)
        dirs[:] = [name for name in dirs if (Path(directory) / name).relative_to(root) not in SKIP_DIRS]
        if 'Cargo.toml' in files:
            found.append((Path(directory) / 'Cargo.toml').relative_to(root))
    return sorted(found)


def read_manifest(path: Path) -> dict[str, object]:
    if toml_parser is None:
        raise RuntimeError('TOML parser unavailable; install tomli via requirements-dev.txt for Python 3.10')
    with path.open('rb') as source:
        return toml_parser.load(source)


def table(value: object) -> dict[str, object]:
    if not isinstance(value, dict):
        return {}
    return value


def forbid(value: object) -> bool:
    return value == 'forbid' or (isinstance(value, dict) and value.get('level') == 'forbid')



def lint_policy_drift(workspace: dict[str, object], package: dict[str, object]) -> list[str]:
    """Compare a non-inheriting package against shared lints without adding allowances."""
    problems: list[str] = []
    for group in sorted(set(workspace) | set(package)):
        shared = table(workspace.get(group))
        local = table(package.get(group))
        for name in sorted(set(shared) | set(local)):
            if group == 'rust' and name == 'unsafe_code':
                continue
            if name not in local:
                level = shared[name]
                if level == 'allow' or (isinstance(level, dict) and level.get('level') == 'allow'):
                    continue
            if name not in shared or name not in local or shared[name] != local[name]:
                problems.append(f'workspace lint policy differs: {group}.{name}')
    return problems


def audit(root: Path, expected: set[Path]) -> list[str]:
    if toml_parser is None:
        return ['TOML parser unavailable; install tomli via requirements-dev.txt for Python 3.10']
    root = root.resolve()
    issues: list[str] = []
    try:
        paths = manifests(root)
        parsed = {path: read_manifest(root / path) for path in paths}
    except (OSError, ValueError) as error:
        return [f'manifest discovery/parsing failed: {error}']
    roots = {path for path, data in parsed.items() if 'workspace' in data}
    for path in sorted(roots - expected):
        issues.append(f'unexpected workspace requires review: {path}')
    for path in sorted(expected - roots):
        issues.append(f'expected workspace missing: {path}')
    packages = {path for path, data in parsed.items() if 'package' in data}
    covered: set[Path] = set()
    for workspace in sorted(roots & expected):
        try:
            result = subprocess.run(['cargo', 'metadata', '--offline', '--locked', '--no-deps',
                                     '--format-version', '1', '--manifest-path', str(root / workspace)],
                                    cwd=root, capture_output=True, text=True, check=True)
            metadata = json.loads(result.stdout)
            member_ids = set(metadata['workspace_members'])
            members = [p for p in metadata['packages'] if p['id'] in member_ids]
            if len(members) != len(member_ids):
                raise ValueError('metadata omitted workspace members')
            for package in members:
                path = Path(package['manifest_path']).resolve().relative_to(root)
                if path not in packages or path in covered:
                    raise ValueError(f'unknown or duplicate package: {path}')
                if not package['targets']:
                    raise ValueError(f'package has no Cargo targets: {path}')
                for target in package['targets']:
                    contained_path(root, target['src_path'], f'{path}: target source {target["name"]}')
                for dependency in package['dependencies']:
                    dependency_path = dependency.get('path')
                    if dependency_path is not None:
                        contained_path(root, dependency_path, f'{path}: dependency {dependency["name"]}')
                    elif not isinstance(dependency.get('source'), str) or not dependency['source']:
                        raise ValueError(f'{path}: dependency {dependency["name"]} has neither local path nor valid source')
                covered.add(path)
                lints = table(parsed[path].get('lints'))
                workspace_lints = table(table(parsed[workspace].get('workspace')).get('lints'))
                if lints.get('workspace') is True:
                    lints = workspace_lints
                elif workspace_lints:
                    for problem in lint_policy_drift(workspace_lints, lints):
                        issues.append(f'{path}: {problem}')
                if path == Path('third_party/mod-alloc/Cargo.toml'):
                    if __package__:
                        from .mod_alloc_integrity import audit_mod_alloc
                    else:
                        from mod_alloc_integrity import audit_mod_alloc
                    issues.extend(audit_mod_alloc(root))
                    continue
                if path == Path('third_party/thread-priority/Cargo.toml'):
                    if __package__:
                        from .thread_priority_integrity import audit_thread_priority
                    else:
                        from thread_priority_integrity import audit_thread_priority
                    issues.extend(audit_thread_priority(root))
                    continue
                rust = table(lints.get('rust'))
                if not forbid(rust.get('unsafe_code')):
                    issues.append(f'{path}: package unsafe_code forbid missing; {len(package["targets"])} targets require audit')
        except (OSError, subprocess.CalledProcessError, json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
            detail = error.stderr.strip() if isinstance(error, subprocess.CalledProcessError) and error.stderr else str(error)
            issues.append(f'{workspace}: Cargo metadata failed: {detail}')
    for path in sorted(packages - covered):
        issues.append(f'{path}: first-party package is not covered by an audited workspace')
    return issues


def main() -> int:
    issues = audit(ROOT, WORKSPACES)
    try:
        if __package__:
            from .rust_direct_inputs import audit_direct_inputs
        else:
            from rust_direct_inputs import audit_direct_inputs
        issues.extend(audit_direct_inputs(ROOT))
    except (ImportError, OSError, ValueError) as error:
        issues.append(f'direct compiler audit unavailable: {error}')
    if issues:
        print('Rust safety assurance FAIL:')
        for issue in issues:
            print(f'  {issue}')
        return 1
    print('Rust safety manifest coverage PASS; compiler, platform and runtime checks remain separate.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
