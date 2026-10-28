"""Keep explicit compiler selections on Beyond's supported toolchain."""

import argparse
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import gate, test_native_client


class ProjectToolchainTests(unittest.TestCase):
    def test_gate_keeps_all_rust_checks_on_project_compiler(self) -> None:
        expected = [
            ('rustfmt', ['cargo', '+1.99.1', 'fmt', '--all', '--', '--check']),
            ('clippy', ['cargo', '+1.99.1', 'clippy', '--locked', '--workspace',
                        '--all-targets', '--', '-D', 'warnings']),
            ('rust tests', ['cargo', '+1.99.1', 'test', '--locked', '--workspace',
                            '--lib', '--bins', '--tests']),
            ('doc tests', ['cargo', '+1.99.1', 'test', '--locked', '--workspace', '--doc']),
        ]
        for quick in (False, True):
            with self.subTest(quick=quick):
                args = argparse.Namespace(quick=quick, no_parity=False, jobs=None,
                                          flake_base='parity-base')
                commands = [(name, command) for name, command in gate.steps(args)
                            if command[0] == 'cargo']
                self.assertEqual(commands, expected[:2] if quick else expected)

    def test_native_build_overrides_inherited_compiler_not_other_environment(self) -> None:
        # Stop at the subprocess boundary: never build or launch the game.
        class BuildIntercepted(Exception):
            pass

        with tempfile.TemporaryDirectory() as temp:
            with (patch.object(test_native_client.lab, 'LOCAL', Path(temp)),
                  patch.object(test_native_client, 'resource_fingerprint', return_value={}),
                  patch.object(test_native_client.sys, 'argv', ['test_native_client.py']),
                  patch.dict(os.environ, {'RUSTUP_TOOLCHAIN': '1.85.0',
                                          'BEYOND_TEST_ENV': 'preserve-me'}),
                  patch.object(test_native_client.subprocess, 'run',
                               side_effect=BuildIntercepted) as run):
                with self.assertRaises(BuildIntercepted):
                    test_native_client.main()
                self.assertEqual(run.call_args.args[0],
                                 ['cargo', 'build', '--locked', '-p', 'horizon-client'])
                environment = run.call_args.kwargs['env']
                self.assertEqual(environment['RUSTUP_TOOLCHAIN'], '1.99.1')
                self.assertEqual(environment['BEYOND_TEST_ENV'], 'preserve-me')
                self.assertEqual(os.environ['RUSTUP_TOOLCHAIN'], '1.85.0')
