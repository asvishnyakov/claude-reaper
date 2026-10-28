# Daemon and Service Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build order step 4 of the spec: `daemon` repeats the sweep in the foreground, and `install` / `uninstall` register it with the OS service manager (systemd user unit, launchd user agent, Task Scheduler task), exercised in CI on all three systems.

**Architecture:** `cli` gains `SweepArguments`, shared by `run`, `daemon` and `install`, and `sweep_once`, which `run` and `daemon` both call. `daemon.rs` holds the loop. `service.rs` exposes `install` / `uninstall` with one implementation per OS (`service/systemd.rs`, `service/launchd.rs`, `service/task_scheduler.rs`, `service/unsupported.rs`), each a pure renderer of the registration file plus the service-manager commands. The CI integration check gains a `service` case that installs, lets the daemon stop a real session, kills the daemon to see it restarted, and uninstalls.

**Tech Stack:** Rust 1.97 (edition 2024); existing crates only (`rustix`, `windows-sys` with `Win32_System_Console`); Python via `uv` with `psutil`; GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-09-claude-reaper-design.md` ("Commands", "Error handling", "Testing"). Evidence: P6 in `docs/probes/README.md`; the working registrations are `p6_macos` and `p6_windows` in `probes/probe.py`.

**Scope:** `daemon`, `install`, `uninstall`. Not in this plan: cargo-dist release and README (build order step 5). The spec is not changed: every name and setting below is how this plan meets "Commands" as written.

## Global Constraints

- No hard kill of a session anywhere. The tool never calls `SIGKILL` or `TerminateProcess`; the service manager ending the daemon on `uninstall` (`systemctl`, `launchctl bootout`, `schtasks /end`) is not a stop of a session.
- Output wording, exact: `installed {description}`, `uninstalled {description}`, `not installed`, `error: {message}`. The daemon prints `run`'s lines unchanged, every sweep, as the spec says.
- Registration names, exact: systemd `claude-reaper.service` in `$XDG_CONFIG_HOME/systemd/user/` (default `~/.config/systemd/user/`); launchd label `io.github.asvishnyakov.claude-reaper`, file `~/Library/LaunchAgents/io.github.asvishnyakov.claude-reaper.plist`, domain `gui/<uid>`, log `~/Library/Logs/claude-reaper.log`; Task Scheduler task `claude-reaper`.
- `--interval-minutes` defaults to `5` and is at least `1`.
- No new crates (spec: "only when a probe shows they are needed"). XML and plist text are written by hand with escaping.
- Automated Rust tests never run `install`, `uninstall`, `systemctl`, `launchctl` or `schtasks`; they test the renderers. Only the CI integration check registers a service.
- A test that runs `daemon` passes `--entrypoint reaper-test-only` and sets `HOME` and `USERPROFILE` to a temporary directory, so no real session is ever selected. Test cleanup may `Child::kill` the daemon it spawned.
- The integration check refuses to run unless `GITHUB_ACTIONS` is `true` (as now). Workflows that read `CLAUDE_CODE_OAUTH_TOKEN` trigger only on `push` and `workflow_dispatch`.
- Nothing committed or printed to a job log may contain the account's email, organisation or display name.
- Pushing and opening PRs is the owner's step; the agent never pushes. Work on branch `daemon-and-service`.
- Run Cargo outside the sandbox. Every task's local check: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`, plus `cargo clippy --all-targets --target x86_64-apple-darwin -- -D warnings` and the same with `x86_64-pc-windows-msvc`. macOS and Windows tests run only in CI.
- Closures use full parameter names.

## Review Focus

1. `install --config-dir ./profile` run from some directory: the service starts elsewhere, so the registered path must be absolute. Test in Task 2.
2. A binary or config-dir path with a space, `%`, `$`, `"`, `&` or `<`: each renderer quotes or escapes it so the daemon receives it byte for byte. Tests in Tasks 2, 3 and 4.
3. `install` run a second time to change `--idle-minutes`: one registration, one daemon, running with the new options. On macOS a `bootstrap` right after `bootout` can fail while the old job is still unloading, so install waits for it to unload. Check in Task 5.
4. `uninstall` when nothing is installed: `not installed`, exit 0. Check in Task 5.
5. `daemon --interval-minutes 0`: rejected by argument parsing, not a busy loop. Test in Task 1.

---

### Task 1: `daemon`

**Files:**
- Modify: `src/cli.rs`, `src/main.rs` (`mod daemon;`)
- Create: `src/daemon.rs`, `tests/daemon.rs`

**Interfaces:**
- Produces:

```rust
// cli.rs — the three options move out of RunArguments, attributes and doc comments unchanged
#[derive(Debug, Args)]
pub struct SweepArguments { pub idle_minutes: u64, pub config_dir: Vec<PathBuf>, pub entrypoint: Vec<String> }
pub struct RunArguments { #[command(flatten)] pub sweep: SweepArguments, pub dry_run: bool }
#[derive(Debug, Args)]
pub struct DaemonArguments {
    #[command(flatten)] pub sweep: SweepArguments,
    /// Minutes between the end of one sweep and the start of the next
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..))]
    pub interval_minutes: u64,
}
Command::Daemon(DaemonArguments)   // "Repeat the sweep every interval, in the foreground"
/// Finds the config directories and sweeps them once. Err only when the sweep cannot run
/// ("cannot determine the home directory") or writing fails.
pub fn sweep_once(arguments: &SweepArguments, dry_run: bool, output: &mut impl Write, warnings: &mut impl Write) -> io::Result<()>;

// daemon.rs
pub fn run(arguments: &DaemonArguments) -> ExitCode;   // loops forever
```

`run` loops: sweep at start, then sleep `interval_minutes * 60` s after each sweep ends. Each sweep calls `sweep_once(&arguments.sweep, false, &mut lines, &mut warnings)` into two `Vec<u8>` buffers, then writes them to stdout and stderr with `let _ =`. A failed write must not abort a sweep half-way or end the loop; on Windows the daemon may have no console (Task 4). On `Err`, `error: sweep failed: {error}` goes to stderr and the loop continues ("Error handling").

- [ ] **Step 1: Write the failing tests.**
  - `src/cli.rs`: existing `run` tests read through `run.sweep`; add `daemon_defaults_to_five_interval_minutes`, `zero_interval_is_rejected`, `daemon_has_no_dry_run` (`daemon --dry-run` fails to parse).
  - `tests/daemon.rs`: `daemon_sweeps_at_start`. Make a fixture config dir with one record: pid `4_000_000`, `name` `fixture`, `entrypoint` `claude-vscode`, idle 20 min, like `cli::tests::fixture_for`. Run `env!("CARGO_BIN_EXE_claude-reaper") daemon --config-dir <fixture> --entrypoint reaper-test-only` with temp `HOME`/`USERPROFILE` and piped stdout. A reader thread sends lines over a channel. Assert that within 10 s the line `skip 4000000 (fixture): entrypoint claude-vscode not selected` arrives, then kill and wait the child.
- [ ] **Step 2: Run** `cargo test` — expected: compile errors, no `daemon` module and no `Daemon` variant.
- [ ] **Step 3: Implement** `SweepArguments`, `sweep_once` (`run_sweep` becomes `sweep_once` plus exit-code mapping, same messages), the `Daemon` variant and `daemon.rs`.
- [ ] **Step 4: Run** the local check — expected: all pass.
- [ ] **Step 5: Commit** — `git add src tests && git commit -m "feat: daemon repeats the sweep in the foreground"`

---

### Task 2: `install` / `uninstall` and the systemd user unit

**Files:**
- Modify: `src/cli.rs`, `src/main.rs` (`mod service;`)
- Create: `src/service.rs`, `src/service/systemd.rs`, `src/service/unsupported.rs`

**Interfaces:**
- Consumes: `DaemonArguments` (Task 1).
- Produces:

```rust
// service.rs
#[derive(Debug, PartialEq, Eq)]
pub struct Registration { pub program: String, pub arguments: Vec<String> }
pub enum Uninstalled { Removed(String), NotInstalled }
/// Registers and starts the daemon, replacing an earlier registration; returns what was registered.
pub fn install(registration: &Registration) -> io::Result<String>;
pub fn uninstall() -> io::Result<Uninstalled>;
/// Runs a service-manager command; Err naming the command, its exit status and stderr when it fails.
pub(crate) fn run_tool(program: &str, arguments: &[&str]) -> io::Result<()>;

// service/systemd.rs, cfg(target_os = "linux")
pub fn unit_file(registration: &Registration) -> String;

// cli.rs
Command::Install(DaemonArguments)   // "Register the daemon to start at login and restart on failure"
Command::Uninstall                  // "Remove the daemon's registration"
/// `program` and `daemon` with every option written out; relative --config-dir joined to `current_dir`.
/// InvalidInput when a path is not valid UTF-8.
pub fn registration(program: &Path, arguments: &DaemonArguments, current_dir: &Path) -> io::Result<Registration>;
```

Argument order: `daemon --idle-minutes N --interval-minutes M`, then `--config-dir P` for each, then `--entrypoint E` for each. `install` takes `program` from `std::env::current_exe()` and the directory from `std::env::current_dir()`.

`unit_file` exactly:

```ini
[Unit]
Description=Stop idle Claude Code sessions (claude-reaper)

[Service]
ExecStart={every word quoted}
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target
```

Each word is `"…"` with `\` → `\\`, `"` → `\"`, `%` → `%%`, `$` → `$$`.

Linux install: write the unit (creating directories), then `systemctl --user daemon-reload`, `enable claude-reaper.service`, `restart claude-reaper.service`; returns `systemd user unit {path}`. Uninstall: no unit file → `NotInstalled`; else `disable --now claude-reaper.service`, remove the file, `daemon-reload`. `unsupported.rs` (macOS and Windows until Tasks 3 and 4, and every other system) returns `Err(io::Error::other("install is not supported on this system"))`.

- [ ] **Step 1: Write the failing tests.**
  - `src/cli.rs`: `install_with_defaults_registers_the_default_options` (→ `["daemon", "--idle-minutes", "15", "--interval-minutes", "5", "--entrypoint", "claude-vscode"]`, `program` as given); `install_registers_every_option` (`--idle-minutes 30 --interval-minutes 2 --config-dir /a --entrypoint cli --entrypoint sdk-py`); `relative_config_dir_is_registered_as_absolute` (`--config-dir profile` with `current_dir` a temp dir → `current_dir.join("profile")` as a string); `uninstall_takes_no_options`.
  - `src/service/systemd.rs`: `unit_file_runs_the_daemon_and_restarts_it_on_failure` (whole text for program `/opt/claude-reaper`, arguments `["daemon"]`); `unit_file_quotes_spaces_percent_dollar_and_quotes` (program `/opt/my tools/claude-reaper`, argument `/p/100%$HOME"x\y` → `ExecStart="/opt/my tools/claude-reaper" "/p/100%%$$HOME\"x\\y"`).
- [ ] **Step 2: Run** `cargo test` — expected: compile errors, no `service` module.
- [ ] **Step 3: Implement** the variants (printing `installed {description}` / `uninstalled {description}` / `not installed`, or `error: {error}` and exit 1), `registration`, `service.rs`, `systemd.rs` and `unsupported.rs`.
- [ ] **Step 4: Run** the local check — expected: all pass.
- [ ] **Step 5: Commit** — `git add src && git commit -m "feat: install and uninstall a systemd user unit on Linux"`

---

### Task 3: launchd user agent on macOS

**Files:**
- Modify: `src/service.rs` (cfg wiring)
- Create: `src/service/launchd.rs`

**Interfaces:**
- Consumes: `Registration`, `Uninstalled`, `run_tool` (Task 2).
- Produces:

```rust
// service/launchd.rs, cfg(target_os = "macos")
pub const LABEL: &str = "io.github.asvishnyakov.claude-reaper";
pub fn property_list(registration: &Registration, log: &str) -> String;
```

`property_list`: an XML plist (`<?xml version="1.0" encoding="UTF-8"?>`, the Apple `PropertyList-1.0.dtd` doctype, `<plist version="1.0">`) with keys `Label`, `ProgramArguments` (program then arguments), `RunAtLoad` `true`, `KeepAlive` → `<dict><key>SuccessfulExit</key><false/></dict>`, `StandardOutPath` and `StandardErrorPath` both `log`. Strings escape `&`, `<`, `>`. The keys are those of `p6_macos`, plus `KeepAlive`.

Install: write the plist, create `~/Library/Logs`, `launchctl bootout gui/<uid>/<LABEL>` with its failure ignored (not loaded). Then poll `launchctl print gui/<uid>/<LABEL>` every 100 ms until it fails, for at most 10 s, and run `launchctl bootstrap gui/<uid> <plist>`. `uid` comes from `rustix::process::getuid()`. Returns `launchd user agent {plist path}`. Uninstall: no plist → `NotInstalled`; else `bootout`, failure ignored, then remove the plist.

- [ ] **Step 1: Write the failing tests** in `src/service/launchd.rs`: `property_list_runs_the_daemon_at_load_and_after_a_failure` (whole text for program `/opt/claude-reaper`, arguments `["daemon"]`, log `/Users/u/Library/Logs/claude-reaper.log`); `property_list_escapes_xml_characters` (argument `a&b<c>` → `<string>a&amp;b&lt;c&gt;</string>`).
- [ ] **Step 2: Run** `cargo clippy --all-targets --target x86_64-apple-darwin -- -D warnings` — expected: errors, no `launchd` module.
- [ ] **Step 3: Implement** `launchd.rs` and the cfg wiring (`unsupported.rs` no longer serves macOS).
- [ ] **Step 4: Run** the local check — expected: all pass.
- [ ] **Step 5: Commit** — `git add src && git commit -m "feat: install and uninstall a launchd user agent on macOS"`

---

### Task 4: Task Scheduler task on Windows, and the daemon's console

**Files:**
- Modify: `src/service.rs` (cfg wiring), `src/daemon.rs`, `src/stop/windows.rs`, `tests/daemon.rs`
- Create: `src/service/task_scheduler.rs`

**Interfaces:**
- Consumes: `Registration`, `Uninstalled`, `run_tool` (Task 2); `daemon::run` (Task 1).
- Produces:

```rust
// service/task_scheduler.rs, cfg(windows)
pub const TASK_NAME: &str = "claude-reaper";
/// Arguments joined as the MSVC runtime parses them back: quoted when empty or holding a space, tab or `"`;
/// backslashes doubled before a `"` and before the closing quote.
pub fn command_line_arguments(arguments: &[String]) -> String;
pub fn task_definition(registration: &Registration, user: &str) -> String;

// daemon.rs, cfg(windows), called first in `run`
/// Frees a console this process is alone on (`GetConsoleProcessList` count 1), which closes the
/// window Task Scheduler opened. A console shared with a shell is kept.
fn release_own_console();
```

`task_definition`: `<?xml version="1.0" encoding="UTF-16"?>`, `<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">` with:
- `LogonTrigger`, `Enabled` `true`, `UserId` `user`.
- `Principal id="Author"`: `UserId` `user`, `LogonType` `InteractiveToken`, `RunLevel` `LeastPrivilege`.
- `Settings`: `MultipleInstancesPolicy` `IgnoreNew`, `DisallowStartIfOnBatteries` `false`, `StopIfGoingOnBatteries` `false`, `ExecutionTimeLimit` `PT0S`, `RestartOnFailure` (`Interval` `PT1M`, `Count` `255`), `Enabled` `true`.
- `Actions Context="Author"`: `Exec` with `Command` the program and `Arguments` from `command_line_arguments`.
Text is XML-escaped (`&`, `<`, `>`). `user` is `%USERDOMAIN%\%USERNAME%`; if either variable is missing, install fails with `error: …`.

Install: write the definition as UTF-16 LE with a BOM to `%TEMP%\claude-reaper-task.xml`. Run `schtasks /end /tn claude-reaper` with its failure ignored, then `schtasks /create /tn claude-reaper /xml <file> /f` and `schtasks /run /tn claude-reaper`. Remove the file on every path. Returns `Task Scheduler task claude-reaper`. Uninstall: `schtasks /query /tn claude-reaper` fails → `NotInstalled`; else `/end` with its failure ignored, then `/delete /tn claude-reaper /f`.

`stop/windows.rs` starts the helper with `CommandExt::creation_flags(CREATE_NO_WINDOW)`. A daemon without a console would otherwise flash a new window at every stop.

- [ ] **Step 1: Write the failing tests.**
  - `src/service/task_scheduler.rs`: `arguments_with_spaces_quotes_and_trailing_backslashes_are_quoted` (`["daemon", r"C:\my dir\", "a\"b", ""]` → `daemon "C:\my dir\\" "a\"b" ""`); `task_definition_starts_at_logon_of_the_user_and_never_times_out` (contains `<UserId>DOMAIN\user</UserId>` twice, `<LogonType>InteractiveToken</LogonType>`, `<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>`, `<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>`, `<RestartOnFailure><Interval>PT1M</Interval><Count>255</Count></RestartOnFailure>`, `<Command>C:\tools\claude-reaper.exe</Command>`); `task_definition_escapes_xml_characters`.
  - `tests/daemon.rs`: `#[cfg(windows)] daemon_alone_on_its_console_detaches_from_it`. Spawn the daemon as in Task 1, with `CREATE_NEW_CONSOLE` and without piped stdout; wait 2 s; `claude-reaper ctrl-c <pid>` exits `2`; then kill and wait the child.
- [ ] **Step 2: Run** cross-target clippy for `x86_64-pc-windows-msvc` — expected: errors, no `task_scheduler` module.
- [ ] **Step 3: Implement** `task_scheduler.rs`, the cfg wiring (`unsupported.rs` now only for systems that are none of the three), `release_own_console` and the helper's creation flag. Add `Win32_System_Threading`'s `CREATE_NO_WINDOW` from `windows-sys` (feature already enabled).
- [ ] **Step 4: Run** the local check — expected: all pass.
- [ ] **Step 5: Commit** — `git add src tests && git commit -m "feat: install and uninstall a Task Scheduler task on Windows; the daemon closes its own console window"`
- [ ] **Step 6: CI.** Ask the owner to push `daemon-and-service`, then `gh run watch` the `ci` run. Expected: `check` green on all three systems. A failure is a finding: read the log, fix, commit, ask for another push. Do not weaken or cfg away a test.

---

### Task 5: Service case in the integration check, and the record

**Files:**
- Modify: `probes/probe.py`, `probes/test_probe.py`, `.github/workflows/integration.yml`, `docs/probes/README.md`, `docs/superpowers/specs/2026-10-09-claude-reaper-design.md` ("Open items")

**Interfaces:**
- Consumes: `start_session`, `wait_for_status`, `session_ended_cleanly`, `Checks`, `report`, `is_running` (in `probe.py`); `install` / `uninstall` / `daemon` (Tasks 1–4).
- Produces:

```python
def is_daemon(command_line: list[str], reaper: Path) -> bool: ...   # argv[0] resolves to reaper and argv[1] == "daemon"
def registration_query(platform: str) -> list[str]: ...           # the command whose exit 0 means "registered"
def service_case(reaper: Path) -> bool: ...
def session_ended_cleanly(checks, session, timeout_seconds: float = STOP_TIMEOUT_SECONDS) -> None: ...
```

`registration_query`: `linux` → `["systemctl", "--user", "is-enabled", "claude-reaper.service"]`; `darwin` → `["launchctl", "print", f"gui/{os.getuid()}/io.github.asvishnyakov.claude-reaper"]`; `win32` → `["schtasks", "/query", "/tn", "claude-reaper"]`.

`service_case` checks, in order, each through `Checks.add`, reported as `Integration (service)`:
1. `install --idle-minutes 600` exits 0 and prints `installed `.
2. `install --idle-minutes 0 --interval-minutes 1` exits 0 (reinstall).
3. Within 30 s, exactly one process passes `is_daemon` (psutil), and its command line contains `--idle-minutes 0`.
4. `registration_query` exits 0.
5. Windows only: `ctrl-c <daemon pid>` exits `2` (the daemon has no console).
6. A `piped` session started after install, once idle, ends with `session_ended_cleanly(…, timeout_seconds=150)`. Discovery without `--config-dir` on a later sweep is what finds it.
7. Linux: `journalctl --user -u claude-reaper.service -o cat` contains `stopped {pid} (`. macOS: `~/Library/Logs/claude-reaper.log` contains it. Windows: no check, Task Scheduler discards the daemon's output.
8. The test hard-kills the daemon (`psutil.Process.kill`). Within 90 s there is a daemon with another PID: restarted on failure.
9. `uninstall` exits 0 and prints `uninstalled `. Within 15 s no `is_daemon` process remains, and `registration_query` fails.
10. `uninstall` again prints `not installed` and exits 0.

A `finally` runs `uninstall`, so a failed case leaves nothing registered. `integration` runs `service_case` after the existing cases, and its result counts toward the exit code.

- [ ] **Step 1: Write the failing tests** in `probes/test_probe.py`: `test_daemon_is_recognised_by_its_command_line` (`[str(reaper), "daemon", "--idle-minutes", "0"]` → `True`; `[str(reaper), "run"]` → `False`; another program's `daemon` → `False`), `test_registration_is_queried_with_each_systems_service_manager` (first word per platform: `systemctl`, `launchctl`, `schtasks`).
- [ ] **Step 2: Run** `uv run --no-project --with psutil python -B -m unittest discover -s probes` — expected: fails, no `is_daemon`.
- [ ] **Step 3: Implement** the four functions and the call from `integration`.
- [ ] **Step 4: Run** the probe tests — expected: all pass.
- [ ] **Step 5: Workflow.** In `integration.yml`, before the run step, add a step `Start the systemd user manager` (`if: runner.os == 'Linux'`): `sudo loginctl enable-linger "$USER"`, wait up to 10 s for `/run/user/$(id -u)/bus` to exist, and append `XDG_RUNTIME_DIR=/run/user/$(id -u)` to `$GITHUB_ENV`. Whether the runner needs exactly this is not known; the first run decides it. Raise `timeout-minutes` to `30`.
- [ ] **Step 6: Commit** — `git add .github probes && git commit -m "test: install, restart and uninstall the daemon in the integration check"`
- [ ] **Step 7: Push and wait.** Ask the owner to push, then `gh run watch` the `integration` run. Expected: all three jobs green, and every check of every case reads `yes` in the artifact. A `no` is a finding: read the artifact, fix the cause, ask for another push. Do not weaken a check. If Task Scheduler does not restart a killed task, check 8 on Windows is the finding: report it to the owner before changing the design.
- [ ] **Step 8: Record the outcome.**
  - `docs/probes/README.md`: say that `daemon` and `install` / `uninstall` were implemented, with the integration run ID, and that the release follows. Under "Limits", add these: on Windows the daemon's output is discarded, so it keeps no log (for the README in the release plan); no real logon was triggered on any system; Windows ran as an administrator, so a standard user's `schtasks /create` was not observed; Linux needed (or did not need) `loginctl enable-linger` on the runner. Read the downloaded artifacts for account identifiers before quoting anything from them.
  - Spec "Open items": `Done <date>: daemon and install / uninstall implemented; the integration check installs, restarts and uninstalls the daemon on all three systems (workflow run <id>).`
- [ ] **Step 9: Commit** — `git add docs && git commit -m "docs: daemon and service implemented, integration on all three systems"`. The owner pushes and opens the PR.
- [ ] **Step 10: Final check.** `gh run list --branch daemon-and-service` shows `ci` and `integration` green for the last commit.
