"""Enforce Beyond's compiler floor and local dependency compatibility."""

import json
from pathlib import Path
import subprocess
import unittest

from tools.check_rust_safety import WORKSPACES
from tools.thread_priority_integrity import audit_thread_priority


class RepositoryRustMsrvTests(unittest.TestCase):
    def test_declared_minimum_covers_local_dependencies(self) -> None:
        root = Path(__file__).resolve().parents[1]
        packages: dict[Path, tuple[str, str | None, list[Path]]] = {}
        for manifest in sorted(WORKSPACES):
            result = subprocess.run(
                ['cargo', 'metadata', '--offline', '--locked', '--no-deps',
                 '--format-version', '1', '--manifest-path', str(root / manifest)],
                cwd=root, check=True, capture_output=True, text=True,
            )
            metadata = json.loads(result.stdout)
            for package in metadata['packages']:
                packages[Path(package['manifest_path']).resolve().parent] = (
                    package['name'], package['rust_version'],
                    [Path(dependency['path']).resolve()
                     for dependency in package['dependencies']
                     if dependency.get('path') is not None],
                )

        checked = 0
        for package_path, (name, minimum, dependencies) in packages.items():
            expected = '1.99.1'
            if package_path == (root / 'third_party/thread-priority').resolve():
                # Only this integrity-bound published dependency retains its original floor.
                self.assertEqual(audit_thread_priority(root), [])
                expected = '1.85'
            with self.subTest(package=name):
                self.assertEqual(minimum, expected,
                                 f'{name} must retain its reviewed Rust {expected} minimum')
            if minimum is None:
                continue  # The policy assertion above already reports this invalid package.
            # Cargo normalizes two- and three-component Rust floors.
            version = tuple(map(int, (minimum.split('.') + ['0', '0'])[:3]))
            for path in dependencies:
                local = packages.get(path)
                if local is None or local[1] is None:
                    continue
                dependency_name, required, _ = local
                floor = tuple(map(int, (required.split('.') + ['0', '0'])[:3]))
                checked += 1
                with self.subTest(package=name, dependency=dependency_name):
                    self.assertGreaterEqual(
                        version, floor,
                        f"{name} declares Rust {minimum}, but local dependency "
                        f"{dependency_name} requires Rust {required}",
                    )
        self.assertGreater(checked, 0, 'No declared local dependency floors were checked')
