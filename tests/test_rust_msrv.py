"""Catch first-party MSRV promises below required local package minimums."""

import json
from pathlib import Path
import subprocess
import unittest

from tools.check_rust_safety import WORKSPACES


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
        for name, minimum, dependencies in packages.values():
            if minimum is None:
                continue  # No declared promise; this test does not invent one.
            # Cargo accepts both 1.85 and 1.85.0 as the same Rust floor.
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
