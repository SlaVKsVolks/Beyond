from pathlib import Path
import shutil
import tempfile
import unittest

from tools.mod_alloc_integrity import audit_mod_alloc


class ModAllocIntegrityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fork = self.root / 'third_party/mod-alloc'
        source = Path(__file__).resolve().parents[1] / 'third_party/mod-alloc'
        shutil.copytree(source, self.fork)

    def test_registered_fork_is_accepted(self) -> None:
        self.assertEqual(audit_mod_alloc(self.root), [])

    def test_frozen_adapter_comment_drift_fails_without_unsafe_fixture(self) -> None:
        path = self.fork / 'src/lib.rs'
        path.write_bytes(path.read_bytes().replace(b'let new_ptr = unsafe', b'let new_ptr = /* drift */ unsafe'))
        self.assertTrue(audit_mod_alloc(self.root))

    def test_inactive_upstream_source_drift_fails(self) -> None:
        path = self.fork / 'src/backtrace/capture.rs'
        path.write_bytes(path.read_bytes() + b'\n// drift\n')
        self.assertTrue(audit_mod_alloc(self.root))

    def test_owned_module_missing_forbid_fails(self) -> None:
        path = self.fork / 'src/exact.rs'
        path.write_bytes(path.read_bytes().replace(b'#![forbid(unsafe_code)]', b'// removed forbid'))
        self.assertTrue(audit_mod_alloc(self.root))

    def test_unknown_safe_source_is_not_blanket_excluded(self) -> None:
        (self.fork / 'src/unregistered.rs').write_text('pub fn value() -> u8 { 1 }\n', encoding='utf-8')
        self.assertTrue(audit_mod_alloc(self.root))

    def test_modified_safe_bridge_envelope_fails(self) -> None:
        path = self.fork / 'src/lib.rs'
        path.write_bytes(path.read_bytes().replace(b'use exact::ReentryGuard;', b'use exact::ReentryGuard;\n// drift'))
        self.assertTrue(audit_mod_alloc(self.root))


if __name__ == '__main__':
    unittest.main()
