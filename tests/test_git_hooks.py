"""Exercise Git hooks through their declared POSIX shell without real pushes."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def posix_shell() -> str:
    shell = shutil.which("sh")
    if shell:
        return shell
    git = shutil.which("git")
    if git:
        shell_path = Path(git).parent.parent / "bin" / "sh.exe"
        if shell_path.is_file():
            return str(shell_path)
    raise RuntimeError("Git hook tests require a POSIX shell (Git for Windows includes one)")


class GitHookTests(unittest.TestCase):
    def test_hooks_choose_the_platform_interpreter_and_preserve_failure_and_stdin(self) -> None:
        for platform in ("MINGW64_NT-10.0", "MSYS_NT-10.0", "Linux", "Darwin"):
            for hook, extra in (("pre-commit", []), ("pre-push", ["--pre-push"])):
                with self.subTest(platform=platform, hook=hook), tempfile.TemporaryDirectory() as tmp:
                    directory = Path(tmp)
                    selected = "python" if platform.startswith(("MINGW", "MSYS")) else "python3"
                    programs = {
                        "uname": f"#!/bin/sh\nprintf '%s\\n' '{platform}'\n",
                        selected: "#!/bin/sh\nprintf '%s\\n' \"$@\"\n"
                                  "IFS= read -r line\nprintf '%s\\n' \"$line\"\nexit 37\n",
                        "python3" if selected == "python" else "python": "#!/bin/sh\nexit 91\n",
                    }
                    for name, source in programs.items():
                        path = directory / name
                        path.write_text(source, encoding="utf-8", newline="\n")
                        path.chmod(0o755)
                    result = subprocess.run(
                        # Git for Windows prepends its tools during shell startup. Set the
                        # fixture PATH afterward so uname is simulated on that host too.
                        [posix_shell(), "-c",
                         'if command -v cygpath >/dev/null 2>&1; then PATH=$(cygpath -u "$1"); '
                         'else PATH=$1; fi; export PATH; . "$2"', "hook-test",
                         directory.as_posix(), (ROOT / ".githooks" / hook).as_posix()], cwd=ROOT,
                        env=dict(os.environ), input="ref evidence\n",
                        text=True, capture_output=True, check=False,
                    )
                    self.assertEqual(result.returncode, 37, result.stderr)
                    self.assertEqual(result.stdout.splitlines(),
                                     ["tools/check_public_tree.py", *extra, "ref evidence"])


if __name__ == "__main__":
    unittest.main()
