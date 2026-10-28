"""Keep fork validation separate from upstream publishing and private runners."""
from pathlib import Path
import re
import unittest


WORKFLOWS = Path(__file__).resolve().parents[1] / ".github" / "workflows"
UPSTREAM_ONLY = "github.repository == 'ludovic111/horizon'"


def job_header(filename: str, job: str) -> str:
    text = (WORKFLOWS / filename).read_text(encoding="utf-8")
    match = re.search(rf"^  {re.escape(job)}:\n(.*?)(?=^    (?:runs-on|uses):)",
                      text, re.MULTILINE | re.DOTALL)
    if match is None:
        raise AssertionError(f"No job header for {filename}: {job}")
    return match.group(1)


class ForkValidationTests(unittest.TestCase):
    def test_publishing_and_issue_writers_are_upstream_only(self) -> None:
        for filename, job in (("release.yml", "decide"), ("launcher.yml", "decide"),
                              ("nightly.yml", "audit"), ("parity-main.yml", "parity-main")):
            with self.subTest(workflow=filename):
                self.assertIn(UPSTREAM_ONLY, job_header(filename, job))

    def test_private_runner_entry_points_are_upstream_only(self) -> None:
        for filename, job in (("parity.yml", "parity"), ("run-parity.yml", "parity"),
                              ("ci.yml", "parity-scope"), ("ci.yml", "vanilla-parity"),
                              ("ci.yml", "jfr-analysis")):
            with self.subTest(workflow=filename, job=job):
                self.assertIn(UPSTREAM_ONLY, job_header(filename, job))

    def test_integration_branch_runs_real_verify(self) -> None:
        text = (WORKFLOWS / "ci.yml").read_text(encoding="utf-8")
        self.assertIn("branches: [main, 'integration/**']", text)
        self.assertNotRegex(job_header("ci.yml", "verify"), r"\bif:")

    def test_rust_acceptance_commands_are_not_reduced(self) -> None:
        text = (WORKFLOWS / "ci.yml").read_text(encoding="utf-8")
        for command in (
            "cargo +1.99.1 fmt --all -- --check",
            "cargo +1.99.1 clippy --workspace --all-targets --features horizon-client/mods -- -D warnings",
            "cargo +1.99.1 clippy -p horizon-client --all-targets --no-default-features -- -D warnings",
            "cargo +1.99.1 test --workspace --lib --bins --tests --features horizon-client/mods,horizon-mods/pack",
            "cargo +1.99.1 test --workspace --doc --features horizon-client/mods,horizon-mods/pack",
        ):
            with self.subTest(command=command):
                self.assertIn(command, text)


if __name__ == "__main__":
    unittest.main()
