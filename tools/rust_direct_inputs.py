"""Conservative checks for Rust programs compiled outside audited Cargo targets."""
from __future__ import annotations

import ast
import os
from pathlib import Path
import re
import stat

JAVA = Path('crates/horizon-prepare/tests/support/java_probe.rs')
CONSOLE = Path('launcher/tests/support/console_probe.rs')
RUST_DRIVERS = {
    Path('launcher/src/backend/java_tests.rs'): 'java_probe.rs',
    Path('crates/horizon-prepare/tests/java_selection.rs'): 'java_probe.rs',
    Path('launcher/tests/support/mod.rs'): 'console_probe.rs',
}
GENERATORS = {Path('tests/test_world_input_capture.py'), Path('tests/test_remote_mining.py')}
SPRITE = Path('tools/bench_sprite_mips.py')
# These are exact repository output/vendor locations, not filename globs.
SKIP = {Path('.git'), Path('.local'), Path('.superpowers'), Path('target'),
        Path('third_party/wgpu-hal'), Path('launcher/target'),
        Path('crates/horizon-telemetry-server/target'), Path('crates/horizon-mod-registry/target')}
SELF = {Path('tools/rust_direct_inputs.py'), Path('tests/test_rust_direct_inputs.py')}
BAN = '#![forbid(unsafe_code)]'


def _python_commands(tree: ast.AST, *, require_subprocess_run: bool = False) -> list[ast.List]:
    result: list[ast.List] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call) or not node.args or not isinstance(node.args[0], ast.List):
            continue
        if require_subprocess_run and not (isinstance(node.func, ast.Attribute) and
                node.func.attr == 'run' and isinstance(node.func.value, ast.Name) and
                node.func.value.id == 'subprocess'):
            continue
        command = node.args[0]
        if command.elts and isinstance(command.elts[0], ast.Constant) and command.elts[0].value == 'rustc':
            result.append(command)
    return result


def _strings(node: ast.AST) -> list[str]:
    return [n.value for n in ast.walk(node) if isinstance(n, ast.Constant) and isinstance(n.value, str)]


def _first_part(node: ast.AST) -> str:
    if (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
            and node.func.attr == 'replace' and node.args and
            isinstance(node.args[0], ast.Constant) and node.args[0].value in {'METHOD', 'METHODS'}):
        return _first_part(node.func.value)
    while isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
        node = node.left
    return node.value if isinstance(node, ast.Constant) and isinstance(node.value, str) else ''


def _safe_fixture_update(node: ast.AST) -> bool:
    return (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and
            node.func.attr == 'replace' and isinstance(node.func.value, ast.Name) and
            node.func.value.id == 'fixture' and bool(node.args) and
            isinstance(node.args[0], ast.Constant) and node.args[0].value in {'METHOD', 'METHODS'})


def audit_direct_inputs(root: Path) -> list[str]:
    """Return coverage gaps; this lexical inventory cannot prove dynamic calls absent."""
    issues: list[str] = []
    texts: dict[Path, str] = {}
    known = {JAVA, CONSOLE, SPRITE, *RUST_DRIVERS, *GENERATORS}
    for path in sorted(known):
        try:
            candidate = root / path
            info = candidate.lstat()
            if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & getattr(stat, 'FILE_ATTRIBUTE_REPARSE_POINT', 0x400):
                raise ValueError('link or reparse point')
            texts[path] = candidate.read_text(encoding='utf-8')
        except (OSError, UnicodeError, ValueError) as error:
            issues.append(f'{path}: cannot audit: {error}')
    if JAVA in texts and BAN not in texts[JAVA].splitlines()[:4]:
        issues.append(f'{JAVA}: crate-root unsafe forbid missing')
    if CONSOLE in texts:
        issues.append(f'{CONSOLE}: native console visibility probe uses unsafe; direct compilation remains blocked')
    for path, source in RUST_DRIVERS.items():
        if path not in texts:
            continue
        text = texts[path]
        calls = re.findall(r'(?:command|new)\("rustc"\)(.*?\.output\(\))', text, re.S)
        binding = (re.search(r'let source\s*=.*?\.join\("[^"]*' + re.escape(source) + r'"\)\s*;', text, re.S)
                   and re.search(r'\.arg\(source\)', calls[0] if calls else ''))
        direct = calls and re.search(r'\.arg\([^;]*?\.join\("[^"]*' + re.escape(source) + r'"\)\)', calls[0], re.S)
        if len(calls) != 1 or not (binding or direct):
            issues.append(f'{path}: rustc invocation or source binding requires review')
        elif source != 'console_probe.rs' and not re.search(r'\.arg\("-Funsafe_code"\)', calls[0]):
            issues.append(f'{path}: rustc unsafe forbid flag missing')
    for path in sorted(GENERATORS):
        if path not in texts:
            continue
        try:
            tree = ast.parse(texts[path], filename=str(path))
        except SyntaxError as error:
            issues.append(f'{path}: cannot parse generator: {error}')
            continue
        commands = _python_commands(tree, require_subprocess_run=True)
        if len(commands) != 1 or not any(isinstance(e, ast.Constant) and e.value == '-Funsafe_code'
                                          for e in commands[0].elts):
            issues.append(f'{path}: rustc invocation or forbid flag missing')
        assignments = [n for n in ast.walk(tree) if isinstance(n, ast.Assign) and
                       any(isinstance(t, ast.Name) and t.id == 'fixture' for t in n.targets)]
        assignments.sort(key=lambda a: a.lineno)
        if (not assignments or not _first_part(assignments[0].value).lstrip().startswith(BAN)
                or any(not _safe_fixture_update(a.value) for a in assignments[1:])):
            issues.append(f'{path}: generated crate-root unsafe forbid missing')
        writes = [n for n in ast.walk(tree) if isinstance(n, ast.Call) and
                  isinstance(n.func, ast.Attribute) and n.func.attr == 'write_text' and n.args and
                  isinstance(n.args[0], ast.Name) and n.args[0].id == 'fixture']
        compiled = [arg.args[0] for cmd in commands for arg in cmd.elts
                    if isinstance(arg, ast.Call) and isinstance(arg.func, ast.Name) and
                    arg.func.id == 'str' and len(arg.args) == 1]
        if len(writes) != 1 or not any(ast.dump(writes[0].func.value) == ast.dump(arg) for arg in compiled):
            issues.append(f'{path}: generated fixture write and compiler input require review')
    if SPRITE in texts:
        try:
            tree = ast.parse(texts[SPRITE], filename=str(SPRITE))
            writes = [n for n in ast.walk(tree) if isinstance(n, ast.Call) and
                      isinstance(n.func, ast.Attribute) and n.func.attr == 'write_text' and n.args]
            main = [n for n in writes if 'main.rs' in _strings(n.func.value)]
            manifest = [n for n in writes if 'Cargo.toml' in _strings(n.func.value)]
            if len(main) != 1 or not _first_part(main[0].args[0]).startswith(BAN):
                issues.append(f'{SPRITE}: generated crate-root unsafe forbid missing')
            expression = manifest[0].args[0] if len(manifest) == 1 else None
            if (not isinstance(expression, (ast.Constant, ast.JoinedStr)) or
                    not any('[lints.rust]\nunsafe_code = "forbid"' in s for s in _strings(expression))):
                issues.append(f'{SPRITE}: generated package unsafe forbid missing')
        except SyntaxError as error:
            issues.append(f'{SPRITE}: cannot parse generator: {error}')
    try:
        def failed(error: OSError) -> None:
            raise error
        for directory, dirs, files in os.walk(root, followlinks=False, onerror=failed):
            relative = Path(directory).relative_to(root)
            for name in dirs:
                path = Path(directory) / name
                info = path.lstat()
                if stat.S_ISLNK(info.st_mode) or getattr(info, 'st_file_attributes', 0) & getattr(stat, 'FILE_ATTRIBUTE_REPARSE_POINT', 0x400):
                    raise ValueError(f'link or reparse point: {path}')
            dirs[:] = [name for name in dirs if relative / name not in SKIP]
            for name in files:
                path = relative / name
                if path in known or path in SELF or path.suffix not in {'.py', '.rs', '.sh', '.ps1'}:
                    continue
                full = root / path
                if full.is_symlink():
                    raise ValueError(f'link: {full}')
                text = full.read_text(encoding='utf-8')
                if path.suffix == '.py':
                    tree = ast.parse(text, filename=str(path))
                    calls = _python_commands(tree)
                    if any(not ({'--version', '-V'} & set(_strings(c))) for c in calls):
                        issues.append(f'{path}: unreviewed direct rustc invocation')
                elif (path.suffix == '.rs' and re.search(r'(?:command|new)\s*\(\s*["\']rustc["\']\s*\)', text)
                      or path.suffix in {'.sh', '.ps1'} and
                      re.search(r'(?m)^\s*(?:&\s*)?(?:["\']?rustc(?:\.exe)?["\']?)(?:\s|$)', text)):
                    issues.append(f'{path}: unreviewed direct rustc invocation')
    except (OSError, UnicodeError, ValueError, SyntaxError) as error:
        issues.append(f'direct compiler discovery failed: {error}')
    return issues
