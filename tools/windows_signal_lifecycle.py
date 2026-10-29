"""Dev-only actual Windows console-signal acceptance; never imports production code."""
from __future__ import annotations
from collections.abc import Callable
import time


def validate_group(server_pid: int, broker_pid: int, console_pids: set[int]) -> None:
    """Refuse broadcasts, self-targets, and any unexpected console member."""
    if (server_pid <= 0 or broker_pid <= 0 or server_pid == broker_pid
            or console_pids != {broker_pid, server_pid}):
        raise ValueError('signal group is not the exact owned server console group')


def validate_outcome(mode: str, outcome: dict[str, object]) -> None:
    """Accept only observations completed before cleanup, never cleanup-made exits."""
    if mode not in {'graceful', 'stubborn'}:
        raise ValueError('unknown fixture mode')
    if not (outcome.get('ready') is True and outcome.get('stop_ack') is True
            and outcome.get('server_exit') == 0 and outcome.get('cleanup_used') is False):
        raise ValueError('missing ready/stop/clean supervisor evidence or cleanup used')
    if mode == 'graceful':
        if (outcome.get('engine_exit') != 0 or outcome.get('forced') is not False
                or outcome.get('second_sent') is not False):
            raise ValueError('graceful child did not exit cleanly after the first event')
    else:
        elapsed = outcome.get('elapsed_after_second')
        if (outcome.get('engine_exit') in (None, 0) or outcome.get('forced') is not True
                or outcome.get('first_alive') is not True or outcome.get('second_sent') is not True
                or not isinstance(elapsed, (float, int)) or not 0 <= elapsed < 120):
            raise ValueError('second-event forced lifecycle not established')


def wait_until(predicate: Callable[[], bool], timeout: float, label: str) -> None:
    """Bound every wait using a monotonic deadline."""
    deadline = time.monotonic() + timeout
    while not predicate():
        if time.monotonic() >= deadline:
            raise TimeoutError(label)
        time.sleep(0.01)

# Native calls below are dev-test orchestration only. No production handler is moved here.
import argparse
import ctypes as C
from ctypes import wintypes as W
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid
from typing import Protocol


class CleanupSystem(Protocol):
    def exit_code(self, handle: int) -> int | None: ...
    def active(self, job: int) -> int: ...
    def terminate_process(self, handle: int) -> None: ...
    def terminate_job(self, job: int) -> None: ...
    def close(self, handle: int) -> None: ...


def cleanup_owned(n: CleanupSystem, process: int | None, thread: int | None,
                  job: int, assigned: bool) -> dict[str, object]:
    """A failed query cannot skip handle-owned termination; never claim uncertain cleanup."""
    errors: list[str] = []
    result: dict[str, object] = {'cleanup_used': False, 'process_reaped': process is None,
                                'descendants_stopped': not assigned}
    if process is not None:
        if not assigned:
            code = None
            try:
                code = n.exit_code(process)
            except Exception as error:
                errors.append(str(error))
            if code is None:
                result['cleanup_used'] = True
                try:
                    n.terminate_process(process)
                except Exception as error:
                    errors.append(str(error))
        else:
            active = None
            try:
                active = n.active(job)
            except Exception as error:
                errors.append(str(error))
            if active != 0:
                result['cleanup_used'] = True
                try:
                    n.terminate_job(job)
                except Exception as error:
                    errors.append(str(error))
            try:
                wait_until(lambda: n.active(job) == 0, 5, 'owned job teardown')
                result['descendants_stopped'] = True
            except Exception as error:
                errors.append(str(error))
        try:
            wait_until(lambda: n.exit_code(process) is not None, 5, 'owned broker reap')
            result['process_reaped'] = True
        except Exception as error:
            errors.append(str(error))
    closed: dict[str, bool] = {}
    for name, handle in [('thread', thread), ('process', process), ('job', job)]:
        if handle is not None:
            try:
                n.close(handle)
                closed[name] = True
            except Exception as error:
                closed[name] = False
                errors.append(str(error))
    result.update(handle_close=closed, job_handle_released=closed.get('job') is True,
                  errors=errors, teardown_confirmed=not errors)
    return result


def publish(path: Path, value: dict[str, object]) -> None:
    """Publish complete private handshake documents; readers never see partial JSON."""
    staging = path.with_suffix('.pending')
    with staging.open('x', encoding='utf8') as output:
        json.dump(value, output)
    staging.replace(path)


class Native:
    """Typed Win32 boundary, constructed only by the explicit Windows fixture."""
    def __init__(self) -> None:
        if os.name != 'nt':
            raise RuntimeError('Windows native fixture is unavailable on this host')
        self.k = C.WinDLL('kernel32', use_last_error=True)
        self.u = C.WinDLL('user32', use_last_error=True)
        declarations = {
            'CreateJobObjectW': ([C.c_void_p, W.LPCWSTR], W.HANDLE),
            'SetInformationJobObject': ([W.HANDLE, C.c_int, C.c_void_p, W.DWORD], W.BOOL),
            'QueryInformationJobObject': ([W.HANDLE, C.c_int, C.c_void_p, W.DWORD, C.c_void_p], W.BOOL),
            'AssignProcessToJobObject': ([W.HANDLE, W.HANDLE], W.BOOL),
            'TerminateJobObject': ([W.HANDLE, W.UINT], W.BOOL),
            'TerminateProcess': ([W.HANDLE, W.UINT], W.BOOL),
            'ResumeThread': ([W.HANDLE], W.DWORD),
            'CloseHandle': ([W.HANDLE], W.BOOL),
            'WaitForSingleObject': ([W.HANDLE, W.DWORD], W.DWORD),
            'GetExitCodeProcess': ([W.HANDLE, C.POINTER(W.DWORD)], W.BOOL),
            'OpenProcess': ([W.DWORD, W.BOOL, W.DWORD], W.HANDLE),
            'QueryFullProcessImageNameW': ([W.HANDLE, W.DWORD, W.LPWSTR, C.POINTER(W.DWORD)], W.BOOL),
            'GetProcessTimes': ([W.HANDLE] + [C.POINTER(W.FILETIME)] * 4, W.BOOL),
            'IsProcessInJob': ([W.HANDLE, W.HANDLE, C.POINTER(W.BOOL)], W.BOOL),
            'GetConsoleWindow': ([], W.HWND),
            'GetConsoleProcessList': ([C.POINTER(W.DWORD), W.DWORD], W.DWORD),
            'GenerateConsoleCtrlEvent': ([W.DWORD, W.DWORD], W.BOOL),
        }
        for name, (args, result) in declarations.items():
            f = getattr(self.k, name)
            f.argtypes, f.restype = args, result
        self.u.IsWindowVisible.argtypes = [W.HWND]
        self.u.IsWindowVisible.restype = W.BOOL

    @staticmethod
    def check(value: object) -> None:
        if not value:
            raise C.WinError(C.get_last_error())

    def close(self, handle: int) -> None:
        self.check(self.k.CloseHandle(handle))

    def terminate_process(self, handle: int) -> None:
        self.check(self.k.TerminateProcess(handle, 91))

    def terminate_job(self, job: int) -> None:
        self.check(self.k.TerminateJobObject(job, 92))

    def require_exact_job(self, handle: int, job: int) -> None:
        if not job:
            raise ValueError('an exact root-owned job handle is required')
        member = W.BOOL()
        self.check(self.k.IsProcessInJob(handle, job, C.byref(member)))
        if not member.value:
            raise RuntimeError('process is not in this exact fixture job')

    def creation(self, handle: int) -> int:
        times = [W.FILETIME() for _ in range(4)]
        self.check(self.k.GetProcessTimes(handle, *(C.byref(t) for t in times)))
        return times[0].dwHighDateTime << 32 | times[0].dwLowDateTime

    def exit_code(self, handle: int) -> int | None:
        wait = self.k.WaitForSingleObject(handle, 0)
        if wait == 258:
            return None
        if wait != 0:
            raise C.WinError(C.get_last_error())
        value = W.DWORD()
        self.check(self.k.GetExitCodeProcess(handle, C.byref(value)))
        return value.value

    def active(self, job: int) -> int:
        # JOBOBJECT_BASIC_ACCOUNTING_INFORMATION: four LARGE_INTEGER + four DWORD.
        data = (C.c_byte * 48)()
        self.check(self.k.QueryInformationJobObject(job, 1, data, C.sizeof(data), None))
        return W.DWORD.from_buffer(data, 40).value

    def console_members(self, server_pid: int) -> None:
        hwnd = self.k.GetConsoleWindow()
        if not hwnd or self.u.IsWindowVisible(hwnd):
            raise RuntimeError('owned broker console is absent or visible; no event sent')
        ids = (W.DWORD * 16)()
        count = self.k.GetConsoleProcessList(ids, len(ids))
        if count == 0 or count > len(ids):
            raise RuntimeError('cannot establish bounded console ownership')
        validate_group(server_pid, os.getpid(), set(ids[:count]))

    def signal(self, server_pid: int) -> None:
        self.console_members(server_pid)
        self.check(self.k.GenerateConsoleCtrlEvent(1, server_pid))  # CTRL_BREAK, never group0.

    def engine_handle(self, pid: int, image: Path, after: int, job: int | None = None) -> tuple[int, dict[str, object]]:
        if pid <= 0 or pid == os.getpid():
            raise RuntimeError('invalid fake engine PID')
        handle = self.k.OpenProcess(0x1000 | 0x100000, False, pid)
        self.check(handle)
        try:
            name, length = C.create_unicode_buffer(32768), W.DWORD(32768)
            self.check(self.k.QueryFullProcessImageNameW(handle, 0, name, C.byref(length)))
            created = self.creation(handle)
            if Path(name.value).resolve() != image.resolve() or created < after:
                raise RuntimeError('fake engine image/creation ownership mismatch')
            if job is not None:
                self.require_exact_job(handle, job)
            if self.exit_code(handle) is not None:
                raise RuntimeError('fake engine exited before observation')
            return handle, {'pid': pid, 'image': name.value, 'created_filetime': created,
                            'exact_root_job_verified': job is not None}
        except BaseException:
            self.k.CloseHandle(handle)
            raise


class Limit(C.Structure):
    _fields_ = [('per_process', C.c_int64), ('per_job', C.c_int64), ('flags', W.DWORD),
                ('min_working', C.c_size_t), ('max_working', C.c_size_t),
                ('active_limit', W.DWORD), ('affinity', C.c_size_t),
                ('priority', W.DWORD), ('scheduling', W.DWORD)]


class ExtendedLimit(C.Structure):
    _fields_ = [('basic', Limit), ('io', C.c_uint64 * 6), ('process_memory', C.c_size_t),
                ('job_memory', C.c_size_t), ('peak_process', C.c_size_t),
                ('peak_job', C.c_size_t)]


def broker(case: Path, server: Path, fake: Path, mode: str) -> None:
    """Own one server process group inside the root's hidden, containing job."""
    n = Native()
    output = case/'server.log'
    result: dict[str, object] = {'mode': mode, 'cleanup_used': False}
    engine = None
    with output.open('wb', buffering=0) as log:
        with subprocess.Popen([str(server), '--dir', str(case), 'run'], cwd=case,
                              stdin=subprocess.PIPE, stdout=log, stderr=log,
                              creationflags=subprocess.CREATE_NEW_PROCESS_GROUP) as child:
            try:
                created_after = int((time.time() + 11644473600) * 10_000_000) - 10_000_000
                wait_until(lambda: (case/'engine.pid').exists(), 15, 'fake child start')
                pid = int((case/'engine.pid').read_text())
                engine, identity = n.engine_handle(pid, fake, created_after)
                request = dict(identity, token=uuid.uuid4().hex)
                publish(case/'engine-verification-request.json', request)
                wait_until(lambda: (case/'engine-verification-ack.json').exists(), 15,
                           'root exact-job ownership verification')
                ack = json.loads((case/'engine-verification-ack.json').read_text())
                if ack != dict(request, exact_root_job_verified=True):
                    raise RuntimeError('root job-verification acknowledgement mismatch')
                result['engine_identity'] = ack
                result['server_pid'] = child.pid
                if child.stdin is None:
                    raise RuntimeError('no owned supervisor stdin')
                token = (case/'probe.txt').read_text()
                child.stdin.write((token + '\n').encode())
                child.stdin.flush()
                wait_until(lambda: (case/'ready.txt').exists()
                           and (case/'ready.txt').read_text() == token, 15, 'forwarded readiness')
                result['ready'] = True
                n.signal(child.pid)
                wait_until(lambda: (case/'stop.txt').exists(), 15, 'first stop acknowledgement')
                result['stop_ack'] = True
                result['first_alive'] = n.exit_code(engine) is None
                result['second_sent'] = False
                result['elapsed_after_second'] = None
                if mode == 'stubborn':
                    if not result['first_alive']:
                        raise RuntimeError('stubborn engine exited after first request')
                    second = time.monotonic()
                    n.signal(child.pid)
                    result['second_sent'] = True
                wait_until(lambda: n.exit_code(engine) is not None, 15, 'engine exit')
                result['engine_exit'] = n.exit_code(engine)
                wait_until(lambda: child.poll() is not None, 15, 'supervisor exit')
                result['server_exit'] = child.returncode
                if mode == 'stubborn':
                    result['elapsed_after_second'] = time.monotonic() - second
                result['forced'] = 'Stopping the game at once, without saving' in output.read_text()
                # Persist before any job termination or context-manager cleanup.
                validate_outcome(mode, result)
                result['observed_before_cleanup'] = True
                (case/'outcome.json').write_text(json.dumps(result, indent=2) + '\n')
            except BaseException as error:
                result.update(error=str(error), cleanup_used=True)
                (case/'outcome.json').write_text(json.dumps(result, indent=2) + '\n')
                # Broker never guesses child cleanup; root owns the whole job.
                # Avoid Popen.__exit__ waiting for a stubborn child: terminate own server handle.
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=5)
                raise
            finally:
                if engine is not None:
                    n.k.CloseHandle(engine)


def fixture_env(case: Path) -> dict[str, str]:
    """Use the repository's canonical protected game environment, with owned paths."""
    try:
        from tools import lab
    except ModuleNotFoundError as error:
        if error.name != 'tools':
            raise
        import lab  # Direct execution from tools/, not a sibling checkout.
    return lab.game_env(
        os.environ, HORIZON_HOME=case/'game', HORIZON_LAUNCHER_HOME=case/'launcher',
        XDG_DATA_HOME=case/'data', XDG_CONFIG_HOME=case/'config',
        TEMP=case/'tmp', TMP=case/'tmp', HORIZON_NO_UPDATE_CHECK='1',
        HORIZON_NO_DISCORD='1', HORIZON_TELEMETRY_URL='off')


def run_case(root: Path, server: Path, fake: Path, mode: str) -> dict[str, object]:
    """Create suspended, assign to a non-inheritable no-breakaway job, then resume."""
    import _winapi
    case = root/mode
    case.mkdir()
    for name in ['tmp', 'game', 'launcher', 'data', 'config', 'resources']:
        (case/name).mkdir()
    (case/'owner.json').write_text(json.dumps({'fixture': 'windows-signal-lifecycle',
                                              'root_pid': os.getpid(), 'mode': mode}))
    (case/'mode.txt').write_text(mode)
    (case/'probe.txt').write_text('fixture-ready-' + uuid.uuid4().hex)
    (case/'resources/horizon-resources.json').write_text('{}')
    (case/'server.toml').write_text(
        '[engine]\npath = ' + json.dumps(str(fake)) + '\nresources = "resources"\n'
        'restart_on_crash = false\n[backups]\nevery = "0"\n[panel]\nbind = ""\n')
    env = fixture_env(case)
    n = Native()
    job = n.k.CreateJobObjectW(None, None)
    n.check(job)
    process = thread = verified_engine = None
    assigned = False
    broker_identity: dict[str, object] = {}
    try:
        limits = ExtendedLimit()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE; no breakaway.
        n.check(n.k.SetInformationJobObject(job, 9, C.byref(limits), C.sizeof(limits)))
        startup = subprocess.STARTUPINFO()
        startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = subprocess.SW_HIDE
        args = [sys.executable, '-B', str(Path(__file__).resolve()), '--broker',
                str(case), '--server', str(server), '--fake', str(fake), '--mode', mode]
        process, thread, pid, _ = _winapi.CreateProcess(
            sys.executable, subprocess.list2cmdline(args), None, None, False,
            subprocess.CREATE_NEW_CONSOLE | 4, env, str(case), startup)
        broker_identity = {'pid': pid, 'image': sys.executable,
                           'owned_creation_handle': True, 'created_filetime': None}
        broker_identity['created_filetime'] = n.creation(process)
        n.check(n.k.AssignProcessToJobObject(job, process))
        assigned = True
        if n.k.ResumeThread(thread) == 0xFFFFFFFF:
            raise C.WinError(C.get_last_error())
        n.check(n.k.CloseHandle(thread))
        thread = None
        def broker_finished() -> bool:
            nonlocal verified_engine
            request_path = case/'engine-verification-request.json'
            if verified_engine is None and request_path.exists():
                request = json.loads(request_path.read_text())
                verified_engine, identity = n.engine_handle(
                    request['pid'], fake, broker_identity['created_filetime'], job)
                if identity['created_filetime'] != request['created_filetime']:
                    raise RuntimeError('engine identity changed between broker and root handles')
                publish(case/'engine-verification-ack.json',
                        dict(request, exact_root_job_verified=True))
            return n.exit_code(process) is not None
        wait_until(broker_finished, 75, 'broker deadline')
        if n.exit_code(process) != 0:
            raise RuntimeError(f'broker failed: owned pid={pid}, exit={n.exit_code(process)}')
        wait_until(lambda: n.active(job) == 0, 5, 'natural descendant teardown')
        result = json.loads((case/'outcome.json').read_text())
        validate_outcome(mode, result)
        if result.get('observed_before_cleanup') is not True:
            raise RuntimeError('outcome was not observed before cleanup')
        if verified_engine is None or n.exit_code(verified_engine) is None:
            raise RuntimeError('root-held exact-job engine exit not confirmed')
        result.update(broker_identity=broker_identity, all_job_descendants_stopped=True)
        return result
    finally:
        primary = sys.exc_info()[1]
        cleanup = cleanup_owned(n, process, thread, job, assigned)
        if verified_engine is not None:
            try:
                n.close(verified_engine)
            except Exception as error:
                cleanup['errors'].append(str(error))
                cleanup['teardown_confirmed'] = False
        cleanup.update(broker_identity=broker_identity,
                       primary_error=str(primary) if primary is not None else None)
        (case/'cleanup.json').write_text(json.dumps(cleanup, indent=2) + '\n')
        if not cleanup['teardown_confirmed']:
            raise RuntimeError('owned fixture teardown not confirmed: ' + str(cleanup['errors'])) from primary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--server', type=Path, required=True)
    parser.add_argument('--fake', type=Path, required=True)
    parser.add_argument('--broker', type=Path)
    parser.add_argument('--mode', choices=['graceful', 'stubborn'])
    args = parser.parse_args()
    if os.name != 'nt':
        print('NOT RUN: actual Windows console lifecycle requires Windows')
        return 0
    server, fake = args.server.resolve(strict=True), args.fake.resolve(strict=True)
    if args.broker is not None:
        if args.mode is None:
            raise ValueError('broker mode required')
        case = args.broker.resolve(strict=True)
        try:
            broker(case, server, fake, args.mode)
        except Exception:
            import traceback
            (case/'broker-error.log').write_text(traceback.format_exc())
            raise
        return 0
    root = Path(tempfile.mkdtemp(prefix='horizon-windows-signal-lifecycle-'))
    print(f'Native lifecycle evidence retained at {root}', flush=True)
    (root/'owner.json').write_text(json.dumps({'purpose': 'native signal fixture evidence',
                                             'owner_pid': os.getpid(),
                                             'cleanup_condition': 'accepted evidence and no process references'}))
    result = {mode: run_case(root, server, fake, mode) for mode in ['graceful', 'stubborn']}
    (root/'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print('PASS: actual owned CTRL_BREAK first graceful / second forced server shutdown')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
