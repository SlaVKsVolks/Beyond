"""Behavioral tests for fail-closed native signal fixture validation."""
import importlib.util
from pathlib import Path
import time
import os
import tempfile
import unittest

PATH = Path(__file__).resolve().parents[1] / 'tools/windows_signal_lifecycle.py'

class LifecycleTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('windows_signal_lifecycle', PATH)
        cls.tool = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.tool)

    def test_only_owned_nonzero_group_is_accepted(self):
        self.tool.validate_group(42, 31, {31, 42})
        for server, broker, members in [(0,31,{31}), (31,31,{31}), (42,31,{31}),
                                         (42,31,{31,42,99}), (-1,31,{31,-1})]:
            with self.subTest(server=server, members=members):
                with self.assertRaises(ValueError):
                    self.tool.validate_group(server, broker, members)

    def test_graceful_requires_observed_clean_exit_without_cleanup(self):
        row = dict(ready=True, stop_ack=True, engine_exit=0, server_exit=0,
                   forced=False, cleanup_used=False, first_alive=True,
                   second_sent=False, elapsed_after_second=None)
        self.tool.validate_outcome('graceful', row)
        for field, value in [('ready',False), ('stop_ack',False), ('engine_exit',1),
                             ('server_exit',1), ('forced',True), ('cleanup_used',True),
                             ('second_sent',True)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.tool.validate_outcome('graceful', row | {field:value})
        with self.assertRaises(ValueError):
            self.tool.validate_outcome('graceful', {})

    def test_forced_requires_live_child_after_first_and_exit_before_timeout(self):
        row = dict(ready=True, stop_ack=True, engine_exit=1, server_exit=0,
                   forced=True, cleanup_used=False, first_alive=True,
                   second_sent=True, elapsed_after_second=0.5)
        self.tool.validate_outcome('stubborn', row)
        for field, value in [('ready',False), ('stop_ack',False), ('engine_exit',0),
                             ('server_exit',1), ('forced',False), ('cleanup_used',True),
                             ('first_alive',False), ('second_sent',False),
                             ('elapsed_after_second',120), ('elapsed_after_second',None)]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.tool.validate_outcome('stubborn', row | {field:value})
        with self.assertRaises(ValueError):
            self.tool.validate_outcome('unknown', row)

    def test_wait_success_does_not_require_sleep(self):
        self.tool.wait_until(lambda: True, 0, 'ready')

    def test_wait_timeout_is_bounded(self):
        start = time.monotonic()
        with self.assertRaises(TimeoutError):
            self.tool.wait_until(lambda: False, 0.02, 'never ready')
        self.assertLess(time.monotonic() - start, 1)

    def test_fixture_environment_uses_only_owned_paths_without_parent_mutation(self):
        before = dict(os.environ)
        with tempfile.TemporaryDirectory() as home:
            case = Path(home)
            result = self.tool.fixture_env(case)
            for key, leaf in [('HORIZON_HOME','game'), ('HORIZON_LAUNCHER_HOME','launcher'),
                              ('XDG_DATA_HOME','data'), ('XDG_CONFIG_HOME','config'),
                              ('TEMP','tmp'), ('TMP','tmp')]:
                self.assertEqual(result[key], str(case/leaf))
            self.assertEqual(result['HORIZON_NO_AUDIO'], '1')
            self.assertEqual(result['HORIZON_LAUNCHER_NO_SELF_UPDATE'], '1')
            self.assertEqual(result['HORIZON_NO_UPDATE_CHECK'], '1')
            self.assertEqual(result['HORIZON_NO_DISCORD'], '1')
        self.assertEqual(dict(os.environ), before)

    def test_foreign_job_membership_does_not_establish_fixture_ownership(self):
        class Kernel:
            def IsProcessInJob(self, handle, job, result):
                result._obj.value = job is None or job == 77
                return 1
        native = object.__new__(self.tool.Native)
        native.k = Kernel()
        with self.assertRaises(RuntimeError):
            native.require_exact_job(42, 55)
        native.require_exact_job(42, 77)

    def test_failed_unassigned_query_still_terminates_owned_creation_handle(self):
        class System:
            terminated = False
            def exit_code(self, handle):
                if not self.terminated:
                    raise OSError('status query failed')
                return 91
            def terminate_process(self, handle):
                self.terminated = True
            def close(self, handle):
                pass
        system = System()
        result = self.tool.cleanup_owned(system, 42, 43, 55, False)
        self.assertTrue(system.terminated)
        self.assertTrue(result['process_reaped'])
        self.assertTrue(result['job_handle_released'])
        self.assertFalse(result['teardown_confirmed'])
        self.assertIn('status query failed', str(result['errors']))

    def test_job_close_failure_is_not_reported_as_released(self):
        class System:
            def exit_code(self, handle):
                return 0
            def active(self, job):
                return 0
            def close(self, handle):
                if handle == 55:
                    raise OSError('job close failed')
        result = self.tool.cleanup_owned(System(), 42, None, 55, True)
        self.assertFalse(result['job_handle_released'])
        self.assertFalse(result['teardown_confirmed'])
        self.assertEqual(result['handle_close']['process'], True)

    def test_failed_job_query_still_attempts_owned_job_termination(self):
        class System:
            terminated = False
            def active(self, job):
                if not self.terminated:
                    raise OSError('accounting query failed')
                return 0
            def terminate_job(self, job):
                self.terminated = True
            def exit_code(self, handle):
                return 92
            def close(self, handle):
                pass
        system = System()
        result = self.tool.cleanup_owned(system, 42, None, 55, True)
        self.assertTrue(system.terminated)
        self.assertTrue(result['descendants_stopped'])
        self.assertFalse(result['teardown_confirmed'])

if __name__ == '__main__':
    unittest.main()
