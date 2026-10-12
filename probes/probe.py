# /// script
# requires-python = ">=3.12"
# dependencies = ["psutil", "pywinpty; sys_platform == 'win32'"]
# ///
"""Probes that record how Claude Code behaves on each operating system.

Results are appended to results/<os>.md. No prompt is ever sent to Claude Code. Screen text is
printed only when a probe fails, and always through redact(), so that no account email, organisation
or display name reaches a log or a committed file.
"""

import argparse
import json
import os
import platform
import queue
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import warnings
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

import psutil

RESULTS_DIR = Path("results")
RECORD_TIMEOUT_SECONDS = 60
IDLE_TIMEOUT_SECONDS = 60
SCREEN_COLUMNS = 160
SCREEN_ROWS = 50
SCREEN_TAIL_LINES = 60
CLEANUP_GRACE_SECONDS = 5

TERMINAL_CODE_PATTERN = re.compile(
    r"\x1b\[(?P<forward>\d*)C"  # cursor forward: stands in for spaces
    r"|\x1b\[(?P<column>\d*)G"  # cursor to column: pads the line
    r"|\x1b\[(?P<row>\d*);?(?P<at_column>\d*)[Hf]"  # cursor to row and column: starts a new line
    r"|\x1b\[[0-?]*[ -/]*[@-~]"  # other CSI sequences
    r"|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)"  # OSC sequences
    r"|\x1b[@-Z\\-_]"  # two-character sequences
)
EMAIL_PATTERN = re.compile(r"[\w.+-]+@[\w-]+(?:\.[\w-]+)+")
IDENTITY_LINE_PATTERN = re.compile(r"welcome|organi[sz]ation", re.IGNORECASE)
# Wording of first-run dialogs and error screens. Strict mode keeps only lines that use it, because
# the main screen shows account details that no list of patterns can be sure to catch.
DIALOG_LINE_PATTERN = re.compile(
    r"\b(?:trust|folder|theme|style|log ?in|sign in|onboarding|press|continue|select|choose|"
    r"connect|network|proxy|error|failed|unable|update|terminal|permission|api key|dialog)\b",
    re.IGNORECASE,
)
ACCOUNT_SEPARATOR_PATTERN = re.compile(r"[@·•|]")

Mode = Literal["pty", "console", "piped"]

results_label = "unknown"
seed_state = True
# Filled in by start_session when no record appears: what the process was doing instead.
last_start_failure: dict[str, object] = {}


def _pad_to(line: str, column: str) -> str:
    return line + " " * max(0, int(column or 1) - 1 - len(line))


def _plain_lines(text: str) -> list[str]:
    """Approximates the screen: cursor movements become spaces and line breaks, colours vanish."""
    lines: list[str] = []
    for raw_line in text.replace("\r", "\n").split("\n"):
        line = ""
        last = 0
        for match in TERMINAL_CODE_PATTERN.finditer(raw_line):
            line += raw_line[last : match.start()]
            last = match.end()
            if match.group("forward") is not None:
                line += " " * int(match.group("forward") or 1)
            elif match.group("column") is not None:
                line = _pad_to(line, match.group("column"))
            elif match.group("row") is not None:
                lines.append(line)
                line = _pad_to("", match.group("at_column"))
        lines.append(line + raw_line[last:])
    return lines


def redact(text: str, strict: bool | None = None) -> str:
    """Terminal output as plain lines, with identifying lines replaced by <redacted>.

    Strict mode, the default while a token is in use, keeps only lines worded like a dialog or an error.
    """
    if strict is None:
        strict = bool(os.environ.get("CLAUDE_CODE_OAUTH_TOKEN"))
    lines: list[str] = []
    for line in _plain_lines(text):
        line = line.strip()
        if not line:
            continue
        looks_like_dialog = DIALOG_LINE_PATTERN.search(line) and not ACCOUNT_SEPARATOR_PATTERN.search(line)
        if IDENTITY_LINE_PATTERN.search(line) or (strict and not looks_like_dialog):
            line = "<redacted>"
        else:
            line = EMAIL_PATTERN.sub("<redacted>", line)
        if not lines or lines[-1] != line:
            lines.append(line)
    return "\n".join(lines)


class _UnixPty:
    """The session runs on a pseudo-terminal that this process reads."""

    def __init__(self, argv: list[str], env: dict[str, str], cwd: Path) -> None:
        import fcntl
        import pty
        import struct
        import termios

        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            self.pid, self._master = pty.fork()
        if self.pid == 0:
            try:
                os.chdir(cwd)
                os.execve(argv[0], argv, env)
            finally:
                os._exit(127)
        fcntl.ioctl(self._master, termios.TIOCSWINSZ, struct.pack("HHHH", SCREEN_ROWS, SCREEN_COLUMNS, 0, 0))
        self.output = bytearray()
        self._exit_code: int | None = None
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self) -> None:
        while True:
            try:
                chunk = os.read(self._master, 65536)
            except OSError:
                return
            if not chunk:
                return
            self.output.extend(chunk)

    def write(self, text: str) -> None:
        os.write(self._master, text.encode())

    def poll(self) -> int | None:
        """Exit code once the process has exited (negative: killed by that signal)."""
        if self._exit_code is None:
            try:
                reaped, status = os.waitpid(self.pid, os.WNOHANG)
            except ChildProcessError:
                return None
            if reaped:
                self._exit_code = os.waitstatus_to_exitcode(status)
        return self._exit_code

    def force_close(self) -> None:
        """Test cleanup only: ask, wait, then force."""
        if self.poll() is None:
            os.kill(self.pid, signal.SIGTERM)
            deadline = time.monotonic() + CLEANUP_GRACE_SECONDS
            while self.poll() is None and time.monotonic() < deadline:
                time.sleep(0.1)
            if self.poll() is None:
                os.kill(self.pid, signal.SIGKILL)
                self.poll()
        try:
            os.close(self._master)
        except OSError:
            pass


class _WindowsPty:
    """The session runs on a ConPTY that this process reads."""

    def __init__(self, argv: list[str], env: dict[str, str], cwd: Path) -> None:
        import winpty  # type: ignore[import-not-found]

        self._process = winpty.PtyProcess.spawn(argv, cwd=str(cwd), env=env, dimensions=(SCREEN_ROWS, SCREEN_COLUMNS))
        self.pid: int = self._process.pid
        self.output = bytearray()
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self) -> None:
        while True:
            try:
                chunk = self._process.read(65536)
            except (EOFError, OSError):
                return
            self.output.extend(chunk.encode("utf-8", "replace"))

    def write(self, text: str) -> None:
        self._process.write(text)

    def poll(self) -> int | None:
        return None if self._process.isalive() else self._process.exitstatus

    def force_close(self) -> None:
        """Test cleanup only."""
        if self._process.isalive():
            self._process.terminate(force=True)


class _WindowsConsole:
    """The session owns a console window of its own; there is no terminal to read."""

    def __init__(self, argv: list[str], env: dict[str, str], cwd: Path) -> None:
        new_console = subprocess.CREATE_NEW_CONSOLE  # type: ignore[attr-defined]
        self._process = subprocess.Popen(argv, env=env, cwd=cwd, creationflags=new_console)
        self.pid: int = self._process.pid
        self.output = bytearray()

    def write(self, _text: str) -> None:
        pass

    def poll(self) -> int | None:
        return self._process.poll()

    def force_close(self) -> None:
        """Test cleanup only."""
        if self._process.poll() is None:
            self._process.kill()
            self._process.wait()


# What the VS Code extension passes (2.1.294), without the values that belong to one machine or
# one conversation: --resume, and the debug flags.
PIPED_FLAGS = [
    "--output-format", "stream-json",
    "--input-format", "stream-json",
    "--verbose",
    "--include-partial-messages",
    "--replay-user-messages",
    "--enable-auth-status",
    "--no-chrome",
    "--max-thinking-tokens=31999",
    "--thinking-display=summarized",
    "--permission-prompt-tool=stdio",
    "--setting-sources=user,project,local",
]  # fmt: skip
PIPED_LAUNCHER = Path(__file__).with_name("spawn-piped.mjs")
PIPED_LAUNCHER_TIMEOUT_SECONDS = 30


class _Piped:
    """Imitates the VS Code extension: piped stdio and no terminal. A Node launcher owns the child and the pipes."""

    def __init__(self, argv: list[str], env: dict[str, str], cwd: Path) -> None:
        node = shutil.which("node")
        if node is None:
            raise RuntimeError("node is not on PATH")
        self._launcher = subprocess.Popen(
            [node, str(PIPED_LAUNCHER), *argv],
            env=env,
            cwd=cwd,
            stdout=subprocess.PIPE,
            text=True,
        )
        self.output = bytearray()
        self._exit_code: int | None = None
        self._first_line: queue.Queue[str] = queue.Queue()
        threading.Thread(target=self._read_launcher, daemon=True).start()
        try:
            first = self._first_line.get(timeout=PIPED_LAUNCHER_TIMEOUT_SECONDS)
        except queue.Empty:
            self._launcher.kill()
            raise RuntimeError("the launcher did not report a PID") from None
        if not first.isdigit():
            self._launcher.kill()
            raise RuntimeError(f"the launcher failed: {first}")
        self.pid = int(first)

    def _read_launcher(self) -> None:
        assert self._launcher.stdout is not None
        for index, line in enumerate(self._launcher.stdout):
            if index == 0:
                self._first_line.put(line.strip())
                continue
            parts = line.split()
            if parts[:1] == ["exit"]:
                code_or_signal = parts[1] if len(parts) > 1 else ""
                if code_or_signal.isdigit():
                    self._exit_code = int(code_or_signal)
                else:
                    self._exit_code = -int(getattr(signal.Signals, code_or_signal, 1))

    def write(self, _text: str) -> None:
        pass

    def poll(self) -> int | None:
        return self._exit_code

    def force_close(self) -> None:
        """Test cleanup only."""
        if psutil.pid_exists(self.pid):
            try:
                psutil.Process(self.pid).kill()
            except psutil.Error:
                pass
        self._launcher.kill()
        self._launcher.wait()


Launched = _UnixPty | _WindowsPty | _WindowsConsole | _Piped


@dataclass
class Session:
    pid: int
    config_dir: Path
    record_path: Path  # config_dir / "sessions" / f"{pid}.json"
    seconds_to_record: float = 0.0
    _launched: Launched | None = field(default=None, repr=False)
    _directories: list[Path] = field(default_factory=list, repr=False)

    def record(self) -> dict | None:
        try:
            return json.loads(self.record_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return None

    def screen_text(self) -> str:
        """Redacted tail of what the session printed; empty when the launch mode has no terminal."""
        if self._launched is None:
            return ""
        text = redact(bytes(self._launched.output).decode("utf-8", "replace"))
        return "\n".join(text.split("\n")[-SCREEN_TAIL_LINES:])

    def send(self, text: str) -> None:
        """Types into the session's terminal, if it has one."""
        if self._launched is not None:
            self._launched.write(text)

    def exit_code(self) -> int | None:
        return None if self._launched is None else self._launched.poll()

    def close(self) -> None:
        """Test cleanup only."""
        if self._launched is not None:
            self._launched.force_close()
        for directory in self._directories:
            shutil.rmtree(directory, ignore_errors=True)


def claude_path() -> str:
    path = shutil.which("claude")
    if path is None:
        raise RuntimeError("claude is not on PATH")
    return path


def claude_version() -> str:
    output = subprocess.run([claude_path(), "--version"], capture_output=True, text=True, check=True).stdout
    return output.strip()


def seed_config(config_dir: Path, workdir: Path, version: str) -> None:
    """Pre-answers first-run onboarding and the folder-trust dialog. This is the hypothesis P1 tests."""
    paths = {str(workdir), workdir.as_posix(), str(workdir.resolve()), workdir.resolve().as_posix()}
    state = {
        "hasCompletedOnboarding": True,
        "lastOnboardingVersion": version.split()[0],
        "projects": {path: {"hasTrustDialogAccepted": True} for path in sorted(paths)},
    }
    (config_dir / ".claude.json").write_text(json.dumps(state), encoding="utf-8")


def session_env(config_dir: Path) -> dict[str, str]:
    """A clean environment: nothing of the developer's own Claude or Anthropic settings leaks in."""
    env = {key: value for key, value in os.environ.items() if not key.upper().startswith(("CLAUDE", "ANTHROPIC"))}
    env["CLAUDE_CONFIG_DIR"] = str(config_dir)
    env["TERM"] = "xterm-256color"
    env["DISABLE_AUTOUPDATER"] = "1"
    token = os.environ.get("CLAUDE_CODE_OAUTH_TOKEN")
    if token:
        env["CLAUDE_CODE_OAUTH_TOKEN"] = token
    return env


def launch(mode: Mode, argv: list[str], env: dict[str, str], cwd: Path) -> Launched:
    if mode == "pty":
        return _WindowsPty(argv, env, cwd) if sys.platform == "win32" else _UnixPty(argv, env, cwd)
    if mode == "console" and sys.platform == "win32":
        return _WindowsConsole(argv, env, cwd)
    if mode == "piped":
        return _Piped(argv, env, cwd)
    raise ValueError(f"launch mode {mode!r} is not available on {sys.platform}")


def start_session(mode: Mode) -> Session | None:
    """Fresh config dir, seeded state, real `claude`; waits up to 60 s for the record. None if it never appears."""
    last_start_failure.clear()
    version = claude_version()
    config_dir = Path(tempfile.mkdtemp(prefix="reaper-probe-config-"))
    workdir = Path(tempfile.mkdtemp(prefix="reaper-probe-cwd-")).resolve()
    if seed_state:
        seed_config(config_dir, workdir, version)
    env = session_env(config_dir)
    argv = [claude_path()]
    if mode == "piped":
        env["CLAUDE_CODE_ENTRYPOINT"] = "claude-vscode"
        argv += PIPED_FLAGS
    launched = launch(mode, argv, env, workdir)
    session = Session(
        pid=launched.pid,
        config_dir=config_dir,
        record_path=config_dir / "sessions" / f"{launched.pid}.json",
        _launched=launched,
        _directories=[config_dir, workdir],
    )
    started = time.monotonic()
    while time.monotonic() - started < RECORD_TIMEOUT_SECONDS:
        if session.record() is not None:
            session.seconds_to_record = time.monotonic() - started
            return session
        if launched.poll() is not None:
            break
        time.sleep(0.5)
    last_start_failure.update(
        waited_seconds=round(time.monotonic() - started, 1),
        exit_code=launched.poll(),
        screen=session.screen_text(),
    )
    session.close()
    return None


def wait_for_status(session: Session, status: str, timeout_seconds: float) -> float | None:
    """Seconds until the record shows `status`, or None if it does not within the timeout."""
    started = time.monotonic()
    while time.monotonic() - started < timeout_seconds:
        record = session.record()
        if record is not None and record.get("status") == status:
            return time.monotonic() - started
        time.sleep(0.5)
    return None


def format_observation(key: str, value: object) -> str:
    text = value if isinstance(value, str) else json.dumps(value, default=str)
    if "\n" in text:
        return f"- {key}:\n\n```text\n{text}\n```"
    return f"- {key}: {text}"


def report(probe: str, question: str, observations: dict[str, object]) -> None:
    """Appends one section to results/<os>.md and prints it."""
    section = "\n".join([f"## {probe}: {question}", "", *(format_observation(key, value) for key, value in observations.items())])
    RESULTS_DIR.mkdir(exist_ok=True)
    with (RESULTS_DIR / f"{results_label}.md").open("a", encoding="utf-8") as results:
        results.write(section + "\n\n")
    print(section + "\n", flush=True)


PROBES: list[Callable[[], bool | None]] = []


def probe(function: Callable[[], bool | None]) -> Callable[[], bool | None]:
    PROBES.append(function)
    return function


@probe
def p1() -> bool:
    question = "Does an interactive session start unattended with a token and pre-seeded onboarding state, and write a session record?"
    observations: dict[str, object] = {
        "Claude Code version": claude_version(),
        "token provided": bool(os.environ.get("CLAUDE_CODE_OAUTH_TOKEN")),
    }
    session = start_session("pty")
    if session is None:
        observations["record appeared"] = f"no, within {RECORD_TIMEOUT_SECONDS} s"
        observations["process exit code"] = last_start_failure.get("exit_code")
        observations["screen text (redacted)"] = last_start_failure.get("screen") or "<empty>"
        report("P1", question, observations)
        return False
    try:
        record = session.record() or {}
        observations["record appeared"] = f"yes, after {session.seconds_to_record:.1f} s"
        observations["record keys"] = sorted(record)
        for key in ("kind", "status", "entrypoint", "procStart", "procStartFt"):
            observations[key] = record.get(key, "<absent>")
        idle_after = wait_for_status(session, "idle", IDLE_TIMEOUT_SECONDS)
        observations["status reached idle"] = (
            f"yes, after {idle_after:.1f} s"
            if idle_after is not None
            else f"no, last status {(session.record() or {}).get('status')!r}"
        )
    finally:
        session.close()
    report("P1", question, observations)
    return True


def proc_stat_start_ticks(pid: int) -> str:
    stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8")
    return stat[stat.rindex(")") + 2 :].split()[19]


def ps_lstart(pid: int, extra_env: dict[str, str]) -> str:
    result = subprocess.run(
        ["ps", "-o", "lstart=", "-p", str(pid)],
        capture_output=True,
        text=True,
        check=True,
        env={**os.environ, **extra_env},
    )
    return result.stdout.strip()


def creation_filetime(pid: int) -> str:
    """Creation time from GetProcessTimes, as the decimal 64-bit FILETIME."""
    import ctypes
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)  # type: ignore[attr-defined]
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.GetProcessTimes.argtypes = [wintypes.HANDLE, *[ctypes.POINTER(wintypes.FILETIME)] * 4]
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    process_query_limited_information = 0x1000
    handle = kernel32.OpenProcess(process_query_limited_information, False, pid)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())  # type: ignore[attr-defined]
    try:
        creation, exited, kernel, user = (wintypes.FILETIME() for _ in range(4))
        if not kernel32.GetProcessTimes(handle, *(ctypes.byref(times) for times in (creation, exited, kernel, user))):
            raise ctypes.WinError(ctypes.get_last_error())  # type: ignore[attr-defined]
        return str((creation.dwHighDateTime << 32) | creation.dwLowDateTime)
    finally:
        kernel32.CloseHandle(handle)


def cim_creation_date(pid: int) -> str:
    command = f'(Get-CimInstance Win32_Process -Filter "ProcessId={pid}").CreationDate.ToString("o")'
    result = subprocess.run(
        ["powershell", "-NoProfile", "-Command", command], capture_output=True, text=True, check=True
    )
    return result.stdout.strip()


def start_time_candidates(pid: int) -> dict[str, Callable[[], str]]:
    """The OS values that `procStart` might equal, per system."""
    candidates: dict[str, Callable[[], str]] = {}
    if sys.platform.startswith("linux"):
        candidates["field 22 of /proc/<pid>/stat"] = lambda: proc_stat_start_ticks(pid)
    elif sys.platform == "darwin":
        candidates["ps -o lstart= with LC_ALL=C TZ=UTC"] = lambda: ps_lstart(pid, {"LC_ALL": "C", "TZ": "UTC"})
        candidates["ps -o lstart= with the inherited environment"] = lambda: ps_lstart(pid, {})
    elif sys.platform == "win32":
        candidates["GetProcessTimes creation FILETIME"] = lambda: creation_filetime(pid)
        candidates["Win32_Process.CreationDate"] = lambda: cim_creation_date(pid)
    candidates["psutil create_time()"] = lambda: repr(psutil.Process(pid).create_time())
    return candidates


def describe_error(error: Exception) -> str:
    return f"<{type(error).__name__}: {error}>"


@probe
def p2() -> None:
    question = "What do procStart and procStartFt contain, and which OS value does each equal?"
    modes: list[Mode] = ["pty", "console"] if sys.platform == "win32" else ["pty"]
    for mode in modes:
        session = start_session(mode)
        name = f"P2 ({mode})"
        if session is None:
            report(name, question, {"session": "did not start", "process exit code": last_start_failure.get("exit_code")})
            continue
        try:
            record = session.record() or {}
            observations: dict[str, object] = {
                "procStart": record.get("procStart", "<absent>"),
                "procStartFt": record.get("procStartFt", "<absent>"),
            }
            for label, candidate in start_time_candidates(session.pid).items():
                try:
                    value = candidate()
                except Exception as error:
                    observations[label] = describe_error(error)
                    continue
                equals = [key for key in ("procStart", "procStartFt") if str(record.get(key)) == value]
                observations[label] = f"{value}; equal as a string to: {', '.join(equals) or 'neither'}"
        finally:
            session.close()
        report(name, question, observations)


ENVIRONMENT_READ_CALL = {
    "linux": "read /proc/<pid>/environ",
    "darwin": "sysctl KERN_PROCARGS2",
    "win32": "reading the process environment block with ReadProcessMemory",
}


@probe
def p7() -> None:
    question = "Can the reaper read CLAUDE_CONFIG_DIR from the environment of a session started by the same user?"
    observations: dict[str, object] = {
        "underlying OS call (via psutil.Process.environ)": ENVIRONMENT_READ_CALL.get(sys.platform, sys.platform),
    }
    session = start_session("pty")
    if session is None:
        observations["session"] = "did not start"
        report("P7", question, observations)
        return
    try:
        try:
            value = psutil.Process(session.pid).environ().get("CLAUDE_CONFIG_DIR")
        except Exception as error:
            observations["result"] = describe_error(error)
        else:
            observations["variable present"] = value is not None
            observations["equals the config dir given at launch"] = value == str(session.config_dir)
            observations["equals it after resolving both paths"] = (
                value is not None and Path(value).resolve() == session.config_dir.resolve()
            )
    finally:
        session.close()
    report("P7", question, observations)


STOP_TIMEOUT_SECONDS = 10
RECORD_REMOVAL_SECONDS = 2
CTRL_C_HELPER = Path(__file__).with_name("ctrl_c.py")


def run_ctrl_c(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(CTRL_C_HELPER), *arguments], capture_output=True, text=True, timeout=30)


def console_sharers(pid: int) -> list[int] | str:
    """Windows: IDs of the processes attached to the session's console, or why they could not be listed."""
    result = run_ctrl_c("--list", str(pid))
    return json.loads(result.stdout) if result.returncode == 0 else f"could not attach: {result.stdout.strip()}"


def send_ctrl_c(session: Session) -> dict[str, object]:
    result = run_ctrl_c(str(session.pid))
    observations: dict[str, object] = {"ctrl_c.py exit code": result.returncode}
    if result.stdout.strip():
        observations["ctrl_c.py output"] = result.stdout.strip()
    return observations


def send_sigterm(session: Session) -> dict[str, object]:
    os.kill(session.pid, signal.SIGTERM)
    return {}


def observe_stop(session: Session, request_stop: Callable[[Session], dict[str, object]]) -> dict[str, object]:
    """Sends the system's stop request and records the four observations every stop probe reports."""
    sharers: list[int] | str | None = console_sharers(session.pid) if sys.platform == "win32" else None
    started = time.monotonic()
    observations = request_stop(session)
    while session.exit_code() is None and time.monotonic() - started < STOP_TIMEOUT_SECONDS:
        time.sleep(0.1)
    exited = session.exit_code() is not None
    observations["exited within 10 s"] = f"yes, after {time.monotonic() - started:.1f} s" if exited else "no"
    observations["exit code"] = session.exit_code()
    removal_deadline = time.monotonic() + RECORD_REMOVAL_SECONDS
    while session.record_path.exists() and time.monotonic() < removal_deadline:
        time.sleep(0.1)
    observations["session record removed"] = not session.record_path.exists()
    if isinstance(sharers, str):
        observations["other processes on its console"] = sharers
    elif sharers is not None:
        others = [other for other in sharers if other != session.pid]
        alive = []
        for other in others:
            try:
                alive.append(f"{psutil.Process(other).name()} ({other})")
            except psutil.Error:
                pass
        observations["other processes on its console"] = f"{len(others)} before the request"
        observations["of those, still alive afterwards"] = alive or "none"
    return observations


def stop_probe(name: str, question: str, mode: Mode, request_stop: Callable[[Session], dict[str, object]]) -> None:
    session = start_session(mode)
    if session is None:
        report(name, question, {"session": "did not start", "process exit code": last_start_failure.get("exit_code")})
        return
    try:
        record = session.record() or {}
        observations: dict[str, object] = {
            "launch mode": mode,
            "session record": f"appeared after {session.seconds_to_record:.1f} s",
            "kind": record.get("kind", "<absent>"),
            "entrypoint": record.get("entrypoint", "<absent>"),
        }
        if mode == "piped":
            observations["flags passed"] = PIPED_FLAGS
            observations["environment imitated"] = "CLAUDE_CODE_ENTRYPOINT=claude-vscode"
        wait_for_status(session, "idle", IDLE_TIMEOUT_SECONDS)
        observations.update(observe_stop(session, request_stop))
    finally:
        session.close()
    report(name, question, observations)


@probe
def p3() -> None:
    if sys.platform == "win32":
        return
    stop_probe("P3", "Does SIGTERM end the session cleanly (process exits, record removed)?", "pty", send_sigterm)


@probe
def p4() -> None:
    if sys.platform != "win32":
        return
    for mode in ("console", "pty"):
        stop_probe(
            f"P4 ({mode})",
            "Does the Ctrl+C sequence (FreeConsole, AttachConsole, SetConsoleCtrlHandler, GenerateConsoleCtrlEvent) end the session cleanly?",
            mode,
            send_ctrl_c,
        )


@probe
def p5() -> None:
    stop_probe(
        "P5",
        "Does a session spawned over pipes with no terminal, as the VS Code extension does, have a console, and does the stop request end it cleanly?",
        "piped",
        send_ctrl_c if sys.platform == "win32" else send_sigterm,
    )


def main() -> int:
    global results_label, seed_state
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[attr-defined]
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--os", required=True, choices=["linux", "macos", "windows"], dest="os_label")
    parser.add_argument("--no-seed", action="store_true", help="diagnostic: start with an empty config dir, as a first run")
    arguments = parser.parse_args()
    results_label = arguments.os_label
    seed_state = not arguments.no_seed

    RESULTS_DIR.mkdir(exist_ok=True)
    (RESULTS_DIR / f"{results_label}.md").write_text(
        f"# Probe results: {results_label}\n\nRunner: {platform.platform()}, Python {platform.python_version()}\n\n",
        encoding="utf-8",
    )
    p1_failed = False
    for function in PROBES:
        try:
            succeeded = function() is not False
        except Exception:
            report(function.__name__.upper(), "probe raised an exception", {"traceback": redact(traceback.format_exc())})
            succeeded = False
        if function is p1 and not succeeded:
            p1_failed = True
    return 1 if p1_failed else 0


if __name__ == "__main__":
    sys.exit(main())
