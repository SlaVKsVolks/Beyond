from pathlib import Path
import shutil
import tempfile
import unittest

from tools.thread_priority_integrity import audit_thread_priority


class ThreadPriorityIntegrityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fork = self.root / "third_party/thread-priority"
        source = Path(__file__).resolve().parents[1] / "third_party/thread-priority"
        shutil.copytree(source, self.fork)

    def test_registered_boundary_is_accepted(self) -> None:
        self.assertEqual(audit_thread_priority(self.root), [])

    def test_missing_fork_fails_closed(self) -> None:
        shutil.rmtree(self.fork)
        self.assertTrue(audit_thread_priority(self.root))

    def test_missing_provenance_fails_closed(self) -> None:
        (self.fork / "FORK.json").unlink()
        self.assertTrue(audit_thread_priority(self.root))

    def test_reviewed_registry_drift_is_rejected(self) -> None:
        path = self.fork / "FORK.json"
        path.write_bytes(path.read_bytes() + b" ")
        self.assertTrue(audit_thread_priority(self.root))

    def test_inactive_windows_source_drift_is_rejected(self) -> None:
        path = self.fork / "src/windows.rs"
        path.write_bytes(path.read_bytes() + b"\n// drift\n")
        self.assertTrue(audit_thread_priority(self.root))

    def test_original_unix_envelope_drift_is_rejected(self) -> None:
        path = self.fork / "src/unix.rs"
        path.write_bytes(path.read_bytes().replace(b"set_errno(0);", b"set_errno(1);", 1))
        self.assertTrue(audit_thread_priority(self.root))

    def test_removing_inserted_forbid_is_rejected(self) -> None:
        path = self.fork / "src/unix.rs"
        path.write_bytes(path.read_bytes().replace(b"#[forbid(unsafe_code)]", b"// removed forbid", 1))
        self.assertTrue(audit_thread_priority(self.root))

    def test_safe_but_unregistered_source_is_rejected(self) -> None:
        (self.fork / "src/unknown.rs").write_text("pub fn value() -> u8 { 1 }\n", encoding="utf8")
        self.assertTrue(audit_thread_priority(self.root))

    def test_new_build_driver_is_rejected(self) -> None:
        (self.fork / "build.rs").write_text("fn main() {}\n", encoding="utf8")
        self.assertTrue(audit_thread_priority(self.root))

    def test_dependency_manifest_drift_is_rejected(self) -> None:
        path = self.fork / "Cargo.toml"
        path.write_bytes(path.read_bytes() + b"\n# unreviewed envelope\n")
        self.assertTrue(audit_thread_priority(self.root))

    def test_nested_manifest_is_rejected(self) -> None:
        path = self.fork / "other/Cargo.toml"
        path.parent.mkdir()
        path.write_text('[package]\nname="unknown"\nversion="0.1.0"\n', encoding="utf8")
        self.assertTrue(audit_thread_priority(self.root))

    def test_license_and_legacy_test_drift_are_rejected(self) -> None:
        for name in ["LICENSE", "tests/unix.rs"]:
            with self.subTest(name=name):
                path = self.fork / name
                original = path.read_bytes()
                path.write_bytes(original + b"\n// drift\n")
                self.assertTrue(audit_thread_priority(self.root))
                path.write_bytes(original)


if __name__ == "__main__":
    unittest.main()
