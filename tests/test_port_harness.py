from pathlib import Path
import os
import stat
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from tools import port_harness as harness

JAVAC = """/x/mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java:45: error: cannot find symbol
  symbol:   variable WORLD
  location: class RegistryLayer
1 error
"""


def state(**statuses):
    return {"harnesses": {name.replace("_1", "#1"): {"status": status} for name, status in statuses.items()}}


def make_reports_alias(alias: Path, target: Path) -> None:
    if os.name != "nt":
        alias.symlink_to(target, target_is_directory=True)
        return
    # cmd's mklink is a built-in. Reject expansion and shell syntax before quoting
    # the two owned absolute paths; list2cmdline does not escape cmd metacharacters.
    cmd = Path(os.environ["SystemRoot"]) / "System32" / "cmd.exe"
    for path in (cmd, alias, target):
        text = str(path)
        if not path.is_absolute() or any(ord(c) < 32 or c in '"&|<>^()%!' for c in text):
            raise ValueError("unsafe junction fixture path")
    command = f'"{cmd}" /d /s /c "mklink /J "{alias}" "{target}""'
    result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, timeout=10,
                            creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode:
        raise RuntimeError(f"junction creation failed: {result.stdout!r} {result.stderr!r}")


def remove_fixture_alias(alias: Path) -> None:
    try:
        if os.name == "nt":
            alias.rmdir()
        else:
            alias.unlink()
    except FileNotFoundError:
        pass


class ClassifyTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.logs = Path(self.directory.name)
        (self.logs / "food.log").write_text(JAVAC)
        (self.logs / "food_extra.log").write_text("not food's log")
        (self.logs / "light.log").write_text("first difference at (3, 64, 2)")

    def tearDown(self):
        self.directory.cleanup()

    def test_pass_oracle_fail_and_new(self):
        now = state(food="fail", light="fail", mob_ai="fail", move="pass", move_1="fail")
        baseline = state(food="pass", light="pass", mob_ai="fail", move="pass")
        found = harness.classify(now, self.logs, baseline)
        self.assertEqual(found["food"], {"status": "oracle", "baseline": "pass", "new": True, "compile_errors": [
            "ParityData.java:45: cannot find symbol: variable WORLD (class RegistryLayer)"]})
        self.assertEqual(found["light"], {"status": "fail", "baseline": "pass", "new": True})
        self.assertEqual(found["mob_ai"], {"status": "fail", "baseline": "fail", "new": False})
        self.assertEqual(found["move"]["status"], "fail")  # one shard failed
        self.assertEqual(harness.summary(found), {"pass": 0, "oracle": 1, "fail": 3, "new": 3, "known": 1})

    def test_without_a_baseline_every_failure_is_new(self):
        found = harness.classify(state(light="fail", food="pass"), self.logs)
        self.assertTrue(found["light"]["new"])
        self.assertFalse(found["food"]["new"])

    def test_named_harnesses_only(self):
        self.assertEqual(list(harness.classify(state(light="fail", food="pass"), self.logs, names=["food"])), ["food"])

    def test_logs_of_one_harness(self):
        (self.logs / "food#2.log").write_text("")
        self.assertEqual([p.name for p in harness.harness_logs(self.logs, "food")], ["food#2.log", "food.log"])


class AssetTests(unittest.TestCase):
    def test_javac_errors_from_the_log_horizon_prepare_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "export.log"
            log.write_text("Oracle.java:33: error: cannot find symbol\n")
            output = f"error: javac failed (exit status: 1) while exporting block states; see {log}\n"
            self.assertIn("Oracle.java:33: error", harness.named_logs(output))
            self.assertEqual(harness.named_logs("error: see /nonexistent/x.log"), "")

    def test_the_reports_link_goes_before_horizon_prepare(self):
        with tempfile.TemporaryDirectory(prefix="reports fixture ") as tmp:
            root, store = Path(tmp) / "candidate", Path(tmp) / "store-reports"
            store.mkdir()
            marker = store / "marker"
            marker.write_bytes(b"store contents")
            reports = root / ".local" / "datagen" / "out" / "reports"
            reports.parent.mkdir(parents=True)
            try:
                make_reports_alias(reports, store)
                def stop_at_build(args, cwd, **kwargs):
                    self.assertEqual(args[:4], ["cargo", "build", "--release", "-p"])
                    self.assertEqual(cwd, root)
                    self.assertFalse(os.path.lexists(reports))
                    self.assertEqual(marker.read_bytes(), b"store contents")
                    raise RuntimeError("stopped before external build")

                with mock.patch.object(harness.port_regen, "run", side_effect=stop_at_build):
                    result = harness.prepare_resources(root, "fixture", store)
                self.assertEqual(result["status"], "failed")
                self.assertIn("stopped before external build", result["error"])
            finally:
                remove_fixture_alias(reports)

    def test_owned_directory_and_missing_reports_are_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            reports = root / ".local" / "datagen" / "out" / "reports"
            harness.own_reports(root)
            reports.mkdir(parents=True)
            marker = reports / "marker"
            marker.write_bytes(b"candidate contents")
            harness.own_reports(root)
            self.assertEqual(marker.read_bytes(), b"candidate contents")
            marker.unlink()
            reports.rmdir()
            reports.write_bytes(b"candidate file")
            harness.own_reports(root)
            self.assertEqual(reports.read_bytes(), b"candidate file")

    def test_broken_reports_alias_is_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, target = Path(tmp) / "candidate", Path(tmp) / "target"
            target.mkdir()
            reports = root / ".local" / "datagen" / "out" / "reports"
            reports.parent.mkdir(parents=True)
            try:
                make_reports_alias(reports, target)
                target.rmdir()
                harness.own_reports(root)
                self.assertFalse(os.path.lexists(reports))
            finally:
                remove_fixture_alias(reports)

    def test_unrecognized_reparse_points_and_volume_mounts_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cases = [(0xA0000003, "\\\\?\\Volume{01234567-89ab-cdef-0123-456789abcdef}\\"),
                     (0xA0000003, r"\Device\HarddiskVolume1\other"),
                     (0xA0000042, r"C:\target")]
            for tag, target in cases:
                with self.subTest(tag=tag, target=target):
                    info = SimpleNamespace(st_mode=stat.S_IFDIR, st_file_attributes=0x400, st_reparse_tag=tag)
                    with mock.patch.object(Path, "lstat", return_value=info), \
                         mock.patch.object(Path, "readlink", return_value=Path(target)), \
                         mock.patch.object(Path, "rmdir", side_effect=AssertionError("removed")), \
                         mock.patch.object(Path, "unlink", side_effect=AssertionError("removed")):
                        with self.assertRaises(OSError):
                            harness.own_reports(root)

    def test_reports_metadata_io_error_propagates(self):
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.object(Path, "lstat", side_effect=PermissionError("denied")):
                with self.assertRaises(PermissionError):
                    harness.own_reports(Path(tmp))

    def test_windows_symlink_removal_uses_the_matching_node_operation(self):
        with tempfile.TemporaryDirectory() as tmp:
            for attributes, operation in ((0x410, "rmdir"), (0x400, "unlink")):
                with self.subTest(operation=operation):
                    info = SimpleNamespace(st_mode=stat.S_IFLNK, st_file_attributes=attributes,
                                           st_reparse_tag=0xA000000C)
                    with mock.patch.object(Path, "lstat", return_value=info), \
                         mock.patch.object(Path, "rmdir") as rmdir, mock.patch.object(Path, "unlink") as unlink:
                        harness.own_reports(Path(tmp))
                    self.assertEqual(rmdir.call_count, int(operation == "rmdir"))
                    self.assertEqual(unlink.call_count, int(operation == "unlink"))

    def test_sounds_and_panorama_only(self):
        self.assertTrue(harness.wanted_asset("minecraft/sounds.json"))
        self.assertTrue(harness.wanted_asset("minecraft/sounds/mob/cow/say1.ogg"))
        self.assertTrue(harness.wanted_asset("minecraft/textures/gui/title/background/panorama_overlay.png"))
        self.assertFalse(harness.wanted_asset("minecraft/lang/fr_fr.json"))


class NamedLogTests(unittest.TestCase):
    def test_native_path_with_spaces_and_multiple_deduplicated_logs(self):
        with tempfile.TemporaryDirectory(prefix="named logs ") as tmp:
            first = Path(tmp) / "first log.log"
            second = Path(tmp) / "second log.log"
            unmentioned = Path(tmp) / "unmentioned.log"
            first.write_text("Oracle.java:33: error: cannot find symbol\n", encoding="utf-8")
            second.write_text("second diagnostic\n", encoding="utf-8")
            unmentioned.write_text("must not appear\n", encoding="utf-8")
            output = (f"failed exporting; see {first}\nfailed again; see {first}\n"
                      f"failed generator; see {second}\n"
                      f"failed; see {Path(tmp) / 'missing.log'}\n")
            self.assertEqual(harness.named_logs(output),
                             "Oracle.java:33: error: cannot find symbol\nsecond diagnostic\n")

    def test_absolute_posix_and_windows_names_on_any_host(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "real.log"
            log.write_text("diagnostic\n", encoding="utf-8")
            names = ["/tmp/no-spaces.log", "/tmp/with spaces/export.log",
                     r"C:\Program Files\Horizon\export.log"]
            for name in names:
                with self.subTest(name=name), mock.patch.object(harness, "Path", side_effect={name: log}.__getitem__):
                    self.assertEqual(harness.named_logs(f"failed; see {name}\n"), "diagnostic\n")
            with mock.patch.object(harness, "Path", side_effect=lambda value: log):
                self.assertEqual(harness.named_logs("see /tmp/no-spaces.log\nfailed; see relative.log\n"), "")

    def test_log_named_directory_does_not_open_unrelated_prefix(self):
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp) / "work.log"
            prefix.write_text("unrelated prefix\n", encoding="utf-8")
            directory = Path(tmp) / "work.log backup"
            directory.mkdir()
            log = directory / "export.log"
            log.write_text("actual diagnostic\n", encoding="utf-8")
            self.assertEqual(harness.named_logs(f"failed; see {log}\n"), "actual diagnostic\n")

    def test_unc_relative_and_ambiguous_trailing_text_never_open(self):
        names = ["//server/share/export.log", r"/\server\share\export.log",
                 r"\\server\share\export.log", r"C:relative\export.log", "relative/export.log",
                 "/tmp/export.log; more details", "/tmp/export.log. Additional prose", "/tmp/export.log."]
        with mock.patch.object(harness, "Path", side_effect=AssertionError("unexpected open")):
            for name in names:
                with self.subTest(name=name):
                    self.assertEqual(harness.named_logs(f"failed; see {name}\n"), "")

    def test_large_log_reads_only_bounded_tail_with_invalid_utf8(self):
        with tempfile.TemporaryDirectory(prefix="named logs ") as tmp:
            log = Path(tmp) / "large.log"
            ending = b"\xffOracle.java:33: error\n"
            tail = b"\x82\xac" + b"x" * ((1 << 20) - 2 - len(ending)) + ending
            log.write_bytes(b"old diagnostic\n\xe2" + tail)
            original_open = Path.open

            class BoundedFile:
                def __init__(self, stream):
                    self.stream = stream

                def __enter__(self):
                    self.stream.__enter__()
                    return self

                def __exit__(self, *args):
                    return self.stream.__exit__(*args)

                def seek(self, *args):
                    return self.stream.seek(*args)

                def tell(self):
                    return self.stream.tell()

                def read(self, size=-1):
                    if size < 0 or size > (1 << 20):
                        raise AssertionError("unbounded log read")
                    return self.stream.read(size)

            def bounded_open(path, *args, **kwargs):
                return BoundedFile(original_open(path, *args, **kwargs))

            with mock.patch.object(Path, "open", bounded_open):
                result = harness.named_logs(f"failed; see {log}\n")
            self.assertTrue(result.startswith("\ufffd"))  # The byte limit cuts a UTF-8 character.
            self.assertIn("\ufffdOracle.java:33: error", result)
            self.assertNotIn("old diagnostic", result)

    def test_unexpected_io_error_is_not_swallowed(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp) / "directory.log"
            directory.mkdir()
            with self.assertRaises(OSError):
                harness.named_logs(f"failed; see {directory}\n")


if __name__ == "__main__":
    unittest.main()
