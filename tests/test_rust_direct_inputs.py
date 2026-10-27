"""Direct compiler coverage audit tests."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import rust_direct_inputs


class DirectInputsTests(unittest.TestCase):
    def test_unknown_compiler_driver_is_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "new_driver.py").write_text('import subprocess\nsubprocess.run(["rustc", "x.rs"])\n')
            issues = rust_direct_inputs.audit_direct_inputs(root)
            self.assertTrue(any("new_driver.py" in issue for issue in issues), issues)

    def test_missing_evidence_is_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            issues = rust_direct_inputs.audit_direct_inputs(Path(directory))
            self.assertTrue(any("java_probe.rs" in issue for issue in issues), issues)

    def test_literal_shell_compiler_is_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "driver.sh").write_text('rustc source.rs -o probe\n')
            issues = rust_direct_inputs.audit_direct_inputs(root)
            self.assertTrue(any("driver.sh" in issue for issue in issues), issues)

    def test_conditional_flag_does_not_count_as_enforcement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_known_inputs(root)
            path = root / "tests/test_remote_mining.py"
            text = path.read_text().replace('"-Funsafe_code"', '"-Funsafe_code" if False else ""')
            path.write_text(text)
            issues = rust_direct_inputs.audit_direct_inputs(root)
            self.assertTrue(any("forbid flag missing" in issue for issue in issues), issues)

    def test_conditional_root_ban_does_not_count_as_enforcement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_known_inputs(root)
            path = root / "tests/test_remote_mining.py"
            text = path.read_text().replace('fixture = "#![forbid(unsafe_code)]\\nfn main() {}"',
                                            'fixture = "#![forbid(unsafe_code)]" if False else "fn main() {}"')
            path.write_text(text)
            issues = rust_direct_inputs.audit_direct_inputs(root)
            self.assertTrue(any("crate-root unsafe forbid missing" in issue for issue in issues), issues)

    def test_later_fixture_replacement_cannot_remove_root_ban(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_known_inputs(root)
            path = root / "tests/test_remote_mining.py"
            text = path.read_text().replace('p.write_text(fixture)',
                'fixture = fixture.replace("#![forbid(unsafe_code)]", "")\np.write_text(fixture)')
            path.write_text(text)
            self.assertTrue(any("crate-root unsafe forbid missing" in issue for issue in rust_direct_inputs.audit_direct_inputs(root)))

    def test_conditional_manifest_does_not_count_as_enforcement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_known_inputs(root)
            path = root / "tools/bench_sprite_mips.py"
            text = path.read_text().replace('"[lints.rust]\\nunsafe_code = \\\"forbid\\\"\\n"',
                '("[lints.rust]\\nunsafe_code = \\\"forbid\\\"\\n" if False else "[package]\\n")')
            path.write_text(text)
            self.assertTrue(any("generated package" in issue for issue in rust_direct_inputs.audit_direct_inputs(root)))

    def test_covered_programs_do_not_add_false_gaps(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_known_inputs(root)
            issues = rust_direct_inputs.audit_direct_inputs(root)
            self.assertEqual(len(issues), 1, issues)
            self.assertTrue(any("console_probe.rs" in issue for issue in issues), issues)

    def test_missing_generator_flag_and_root_ban_are_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.write_known_inputs(root)
            path = root / "tests/test_remote_mining.py"
            path.write_text('fixture = "fn main() {}"\nrun(["rustc", "probe.rs"])\n')
            issues = rust_direct_inputs.audit_direct_inputs(root)
            self.assertTrue(any("forbid flag missing" in s for s in issues), issues)
            self.assertTrue(any("crate-root unsafe forbid missing" in s for s in issues), issues)

    def write_known_inputs(self, root):
        paths = {
            "crates/horizon-prepare/tests/support/java_probe.rs": "#![forbid(unsafe_code)]\nfn main() {}\n",
            "launcher/tests/support/console_probe.rs": "fn main() {}\n",
            "launcher/src/backend/java_tests.rs": 'let source = Path::new("x").join("java_probe.rs"); command("rustc").arg(source).arg("-Funsafe_code").output();',
            "crates/horizon-prepare/tests/java_selection.rs": 'command("rustc").arg(Path::new("x").join("java_probe.rs")).arg("-Funsafe_code").output();',
            "launcher/tests/support/mod.rs": 'let source = Path::new("x").join("console_probe.rs"); command("rustc").arg(source).output();',
            "tests/test_world_input_capture.py": 'fixture = "#![forbid(unsafe_code)]\\nfn main() {}"\np.write_text(fixture)\nsubprocess.run(["rustc", "-Funsafe_code", str(p)])\n',
            "tests/test_remote_mining.py": 'fixture = "#![forbid(unsafe_code)]\\nfn main() {}"\np.write_text(fixture)\nsubprocess.run(["rustc", "-Funsafe_code", str(p)])\n',
            "tools/bench_sprite_mips.py": '(build / "main.rs").write_text("#![forbid(unsafe_code)]\\n" + code)\n(build / "Cargo.toml").write_text("[lints.rust]\\nunsafe_code = \\\"forbid\\\"\\n")\n',
        }
        for path, contents in paths.items():
            file = root / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(contents, encoding="utf-8")
