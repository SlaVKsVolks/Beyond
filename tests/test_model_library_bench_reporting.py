"""Exercise real benchmark reporting; only the external process/wait boundary is replaced."""
from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import subprocess
import sys
from types import SimpleNamespace, TracebackType
from typing import TextIO
import unittest
from unittest.mock import patch

from tools import model_library_bench


class _CompletedProcess:
    pid = 123
    returncode: int | None = None

    def __enter__(self) -> '_CompletedProcess':
        return self

    def __exit__(self, kind: type[BaseException] | None,
                 error: BaseException | None, traceback: TracebackType | None) -> None:
        return None


class ModelLibraryBenchReportingTests(unittest.TestCase):
    def _run_driver(self, status: int = 0) -> str:
        def launch(command: list[str], *, stdout: TextIO, stderr: int) -> _CompletedProcess:
            self.assertEqual(stderr, subprocess.STDOUT)
            self.assertEqual(len(command), 2)
            label = Path(command[0]).name
            self.assertIn(label, {'baseline', 'candidate'})
            stdout.write(
                'block_ms=1.500 item_ms=2.000 total_ms=3.500 '
                + ('retained_bytes=-64 block_retained_bytes=-32\n' if label == 'baseline'
                   else 'retained_bytes=0 block_retained_bytes=+16\n'))
            stdout.flush()
            return _CompletedProcess()

        # Mirror all sixteen Unix struct_rusage fields; the parser must retain RSS
        # separately from the controlled requested-live fields.
        fields = ('ru_utime ru_stime ru_maxrss ru_ixrss ru_idrss ru_isrss ru_minflt '
                  'ru_majflt ru_nswap ru_inblock ru_oublock ru_msgsnd ru_msgrcv '
                  'ru_nsignals ru_nvcsw ru_nivcsw').split()
        usage = SimpleNamespace(**{name: 4096 if name == 'ru_maxrss' else 0 for name in fields})
        output = io.StringIO()
        with patch.object(sys, 'argv', ['model_library_bench', 'baseline', 'candidate', 'resources']), \
                patch.object(model_library_bench.subprocess, 'Popen', side_effect=launch), \
                patch.object(model_library_bench.os, 'wait4',
                             return_value=(123, status << 8, usage), create=True), \
                redirect_stdout(output):
            model_library_bench.main()
        return output.getvalue()

    def test_signed_retained_fields_survive_rows_and_ten_sample_summary(self) -> None:
        lines = self._run_driver().splitlines()
        rows = [json.loads(line) for line in lines if line.startswith('{')]
        self.assertEqual(len(rows), 24)
        for row in rows:
            self.assertIn('retained_bytes', row)
            self.assertIn('block_retained_bytes', row)
            self.assertEqual(row['peak_rss_kib'], 4096)
            self.assertEqual(row['retained_bytes'], -64 if row['label'] == 'baseline' else 0)
            self.assertEqual(row['block_retained_bytes'], -32 if row['label'] == 'baseline' else 16)
        summaries = {line.split(' ', 1)[0]: json.loads(line.split(' ', 1)[1])
                     for line in lines if line.startswith(('baseline ', 'candidate '))}
        self.assertEqual(summaries['baseline']['retained_bytes'],
                         {'median': -64, 'samples': [-64] * 10})
        self.assertEqual(summaries['candidate']['block_retained_bytes'],
                         {'median': 16, 'samples': [16] * 10})

    def test_nonzero_child_exit_is_not_reported_as_benchmark_success(self) -> None:
        with self.assertRaises(subprocess.CalledProcessError) as caught:
            self._run_driver(status=7)
        self.assertEqual(caught.exception.returncode, 7)


if __name__ == '__main__':
    unittest.main()
