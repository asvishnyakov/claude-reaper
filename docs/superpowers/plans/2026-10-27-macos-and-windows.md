# macOS and Windows Process and Stop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build order step 3 of the spec: `run` verifies identity, discovers config directories and stops sessions gracefully on macOS and Windows as it already does on Linux, implemented from probes P2, P3b, P4, P5 and P7, with the CI integration check on all three systems.

**Architecture:** `process` gains `macos.rs` (identity from `ps`) and `windows.rs` (identity from `GetProcessTimes`), plus `environment.rs`, shared by both, which reads other processes' environments and owners through `sysinfo`. `stop/linux.rs` becomes `stop/unix.rs` for Linux and macOS (`SIGINT`); `stop/windows.rs` runs the five-call Ctrl+C sequence in a child, the reaper re-executed with a hidden `ctrl-c <pid>` subcommand. The wait after a stop request moves to `stop.rs` and is shared.

**Tech Stack:** Rust 1.97 (edition 2024), `rustix` 1, `sysinfo` 0.39 (`default-features = false`, `features = ["system"]`), `windows-sys` 0.61; Python via `uv` with `psutil`; GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-09-claude-reaper-design.md` ("Config directories", "Stopping a session", "Windows mechanics", "Process identity", "Testing"). Evidence and decisions: `docs/probes/README.md`.

**Scope:** `run` only. Not in this plan: `daemon`, `install`/`uninstall` (P6), release, README. Each gets its own plan after this one.

## Global Constraints

- No hard kill anywhere: no `SIGKILL`, no `TerminateProcess`, no flag for it. Test cleanup of test-spawned children with `Child::kill` is allowed.
- Output wording, exact: `stopped {pid} ({name}, {config dir name}, idle {n} min)`, `would stop …`, `skip {pid} ({name}): {reason}`. New reason: `no console to deliver Ctrl+C`.
- Stop request: `SIGINT` on Linux and macOS; Ctrl+C through the target's console on Windows. Grace period `10` seconds; no escalation.
- A record without `procStart` is accepted on Linux only; on macOS and Windows it is `process identity unverified`.
- Only processes of the user running the reaper are ever stopped or trusted for discovery, on every OS.
- New crates, and only these: `sysinfo = { version = "0.39", default-features = false, features = ["system"] }` for macOS and Windows; `windows-sys = { version = "0.61", features = ["Win32_Foundation", "Win32_System_Threading", "Win32_System_Console"] }` for Windows. Both were checked to build with `cargo clippy --target x86_64-apple-darwin` and `--target x86_64-pc-windows-msvc` on Linux.
- Claude Code version for integration stays `2.1.310`, written only in `.github/actions/install-claude-code/action.yml`.
- Workflows that read `CLAUDE_CODE_OAUTH_TOKEN` trigger only on `push` and `workflow_dispatch`. The integration check refuses to run unless `GITHUB_ACTIONS` is `true`.
- Automated Rust tests call `sweep` only with `dry_run = true`, and stop only children the test spawned.
- Nothing committed or printed to a job log may contain the account's email, organisation or display name.
- Pushing is the repository owner's step: ask before each push (after Task 3, and in Task 4). Work on a branch, not `main`.
- Run Cargo and `rustup` outside the sandbox. Once, before Task 1: `rustup target add x86_64-apple-darwin x86_64-pc-windows-msvc`.
- Every task's local check is `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`, plus `cargo clippy --all-targets --target x86_64-apple-darwin -- -D warnings` and the same with `x86_64-pc-windows-msvc`. macOS and Windows tests run only in CI.
- Closures use full parameter names (`process`, `variable`, `line`).

## Review Focus

1. macOS, a day of the month from 1 to 9: `ps` pads it (`Oct  5`), and a record may not. Whitespace runs are collapsed on both sides before comparing, so the session is `Same`, not `PID reused`. Tests in Task 1.
2. A record whose `procStart` is a JSON number rather than a string: read as its decimal text, not dropped silently. Test in Task 3.
3. Windows, a process that has exited while some handle keeps its object alive: `Gone`, so a stop reports `stopped` at once instead of waiting out the grace period. Test in Task 3.
4. Windows, a session with no console: the helper exits `2`, the line is `skip … : no console to deliver Ctrl+C`, and the process keeps running. Test in Task 3.
5. Windows, a session with Ctrl+C disabled (started with `CREATE_NEW_PROCESS_GROUP`): the event is generated but ignored; the process keeps running and the line is `did not exit after graceful stop`. Test in Task 3.

---

### Task 1: macOS identity and SIGINT stop

**Files:**
- Modify: `Cargo.toml` (`rustix` target becomes `cfg(any(target_os = "linux", target_os = "macos"))`), `src/process.rs`, `src/process/linux.rs`, `src/process/unverified.rs` (cfg), `src/stop.rs`, `src/stop/unsupported.rs` (cfg), `src/cli.rs` (tests)
- Create: `src/process/macos.rs`
- Rename: `git mv src/stop/linux.rs src/stop/unix.rs`, compiled for Linux and macOS

**Interfaces:**
- Produces:

```rust
// process/macos.rs
pub fn identity(pid: u32, proc_start: Option<&str>) -> Identity;
/// One line of `ps -o uid=,stat=,lstart=`: uid, first character of the state, and the start time
/// with every run of whitespace collapsed to one space.
pub(crate) fn parse_ps_line(line: &str) -> Option<(u32, char, String)>;

// process/linux.rs and process/macos.rs, re-exported from process.rs
#[cfg(test)] pub(crate) fn proc_start_of(pid: u32) -> String; // what Claude Code would record for pid

// stop.rs
/// Polls `identity` every 100 ms until it is `Gone` or `Different` (Stopped), or `grace` passes (DidNotExit).
pub(crate) fn wait_for_exit(pid: u32, proc_start: Option<&str>, grace: Duration) -> StopOutcome;
```

`identity` on macOS runs `ps -o uid=,stat=,lstart= -p <pid>` with `LC_ALL=C` and `TZ=UTC` (P2: `procStart` is that text in UTC whatever the session's own zone). Order: `ps` cannot be run → `Unverified`; non-zero exit or no line → `Gone`; state `Z` → `Gone`; uid other than `rustix::process::getuid()` → `Different`; no `procStart` → `Unverified`; `procStart` with whitespace collapsed differs from the `lstart` field → `Different`; otherwise `Same`.

`stop/unix.rs` keeps its `SIGINT` and calls `wait_for_exit`. Waiting until `Gone` or `Different` instead of while `Same` matters on macOS, where a recordless child is `Unverified` while it runs.

- [ ] **Step 1: Write the failing tests** in `src/process/macos.rs` (all `#[cfg(test)]`, run on macOS in CI):

| Test | Assert |
|---|---|
| `ps_line_collapses_the_padded_day` | `parse_ps_line("  501 Ss   Mon Oct  5 09:01:02 2026") == Some((501, 'S', "Mon Oct 5 09:01:02 2026".into()))` |
| `own_process_with_its_own_start_is_same` | `identity(id, Some(&proc_start_of(id))) == Same` |
| `recorded_start_with_extra_spaces_is_same` | recorded `format!(" {}  ", proc_start_of(id).replace(' ', "  "))` → `Same` |
| `own_process_with_a_wrong_start_is_different` | `Some("Mon Jan 1 00:00:00 2001")` → `Different` |
| `own_process_without_recorded_start_is_unverified` | `None` → `Unverified` |
| `exited_child_is_gone` | `true` spawned and waited → `Gone` |
| `unreaped_child_is_gone` | `true` spawned, 300 ms sleep, not waited (a zombie) → `Gone`; then wait |
| `process_of_another_user_is_different` | pid `1` (launchd, root) → `Different` |

In `src/cli.rs`, `dry_run_reports_the_fixture_session` and `dry_run_reports_a_reused_pid` become `#[cfg(any(target_os = "linux", target_os = "macos"))]`; the first builds its fixture with `Some(&process::proc_start_of(pid))` and expects `would stop`. Add `record_without_proc_start_is_unverified_off_linux` (`#[cfg(not(target_os = "linux"))]`, fixture with `None` → `process identity unverified`). `stop/unix.rs` tests stay as they are and now also run on macOS.

- [ ] **Step 2: Run** `cargo clippy --all-targets --target x86_64-apple-darwin -- -D warnings` — expected: errors, `macos` module and `proc_start_of` not found.
- [ ] **Step 3: Implement** `process/macos.rs`, `proc_start_of` for Linux (field 22 via `parse_start_ticks`) and macOS (`parse_ps_line`'s third field), the cfg wiring in `process.rs` and `stop.rs` (`unverified.rs` and `unsupported.rs` now serve every system except Linux and macOS; Task 3 takes Windows out of that set), and `wait_for_exit`.
- [ ] **Step 4: Run** the local check from Global Constraints — expected: all pass, no warnings on all three targets.
- [ ] **Step 5: Commit** — `git add Cargo.toml src && git commit -m "feat: macOS identity from ps lstart and SIGINT stop"`

---

### Task 2: Config directories of running processes on macOS and Windows

**Files:**
- Modify: `Cargo.toml` (`sysinfo` under `cfg(any(target_os = "macos", windows))`), `src/process.rs`, `src/process/linux.rs` (tests use the shared spawn helper), `src/cli.rs` (tests)
- Create: `src/process/environment.rs`

**Interfaces:**
- Produces:

```rust
// process/environment.rs, cfg(any(target_os = "macos", windows)); re-exported from process.rs
pub fn running_config_dirs() -> Vec<PathBuf>;
pub fn names_config_dir(pid: u32, config_dir: &Path) -> bool;
/// Some(true) when pid runs as the same user as this process; None when either owner cannot be read.
pub fn owned_by_current_user(pid: u32) -> Option<bool>;
/// The absolute CLAUDE_CONFIG_DIR among `KEY=VALUE` entries; the name matches ignoring ASCII case on Windows.
fn config_dir_in(environ: &[OsString]) -> Option<PathBuf>;

// process.rs, all three systems
/// A sleeping child (`sleep 30`; on Windows `ping -n 30 127.0.0.1` with output discarded) with
/// CLAUDE_CONFIG_DIR removed or set; returns once its environment is readable, failing after 5 s.
#[cfg(test)] pub(crate) fn spawn_with_config_dir(config_dir: Option<&Path>) -> std::process::Child;
```

Reading uses `System::refresh_processes_specifics` with `ProcessRefreshKind::nothing().with_environ(UpdateKind::Always).with_user(UpdateKind::Always)` (`ProcessesToUpdate::All` for `running_config_dirs`, `Some(&[pid])` for the other two), and `Process::environ()` / `Process::user_id()`, compared with this process's own `user_id()`. A process whose environment cannot be read (P7 safe branch: access denied) contributes nothing. `running_config_dirs` keeps only processes with `owned_by_current_user == Some(true)`; `names_config_dir` canonicalises both paths with `sessions::canonical`, as Linux does. On Linux, `spawn_with_config_dir` replaces `sleeper_with_config_dir` and calls `wait_for_environment`; on macOS and Windows it polls the child's `sysinfo` environment every 5 ms until it is non-empty. Windows keeps `unverified::identity` until Task 3; gate `unverified.rs`'s other two functions so no target has dead code.

- [ ] **Step 1: Write the failing tests** in `src/process/environment.rs`, ported from `process/linux.rs`: `finds_config_dir_of_a_running_process`, `ignores_relative_and_empty_config_dir_values`, `process_names_the_config_dir_it_was_started_with`, `process_does_not_name_a_different_config_dir`, `process_without_the_variable_names_no_config_dir`, `another_spelling_of_the_same_directory_counts` (`#[cfg(target_os = "macos")]`, symlink), `own_process_is_owned_by_current_user` (`Some(true)`), and `#[cfg(windows)] variable_name_case_is_ignored_on_windows` (`config_dir_in(&["claude_config_dir=C:\\profile".into()]) == Some(PathBuf::from("C:\\profile"))`). In `src/cli.rs`, `discovered_directory_is_acted_on_when_the_session_process_names_it` uses `spawn_with_config_dir` and a fixture with `Some(&process::proc_start_of(child.id()))`, and both discovered-directory tests become `#[cfg(any(target_os = "linux", target_os = "macos"))]`.
- [ ] **Step 2: Run** cross-target clippy for both targets — expected: errors, `environment` module not found.
- [ ] **Step 3: Implement** the module, the re-exports (macOS and Windows take `running_config_dirs` and `names_config_dir` from `environment`), and `spawn_with_config_dir`.
- [ ] **Step 4: Run** the local check — expected: all pass.
- [ ] **Step 5: Commit** — `git add Cargo.toml Cargo.lock src && git commit -m "feat: discover config directories from running processes on macOS and Windows"`

---

### Task 3: Windows identity and Ctrl+C stop

**Files:**
- Modify: `Cargo.toml` (`windows-sys` under `cfg(windows)`), `src/process.rs`, `src/stop.rs` (new variant), `src/sessions.rs`, `src/cli.rs`
- Create: `src/process/windows.rs`, `src/stop/windows.rs`, `tests/ctrl_c.rs`

**Interfaces:**
- Consumes: `owned_by_current_user` (Task 2), `wait_for_exit` (Task 1).
- Produces:

```rust
// process/windows.rs
pub fn identity(pid: u32, proc_start: Option<&str>) -> Identity;
#[cfg(test)] pub(crate) fn proc_start_of(pid: u32) -> String; // creation FILETIME as decimal

// stop.rs
pub enum StopOutcome { Stopped, DidNotExit, NoConsole, Unsupported }

// stop/windows.rs
/// Spawns `<current exe> ctrl-c <pid>` with null stdio; exit 0 → wait_for_exit, 2 → NoConsole, anything else → Err.
pub fn stop(pid: u32, proc_start: Option<&str>, grace: Duration) -> io::Result<StopOutcome>;
/// The spec's five calls in this process: FreeConsole, AttachConsole(pid), SetConsoleCtrlHandler(None, TRUE),
/// GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0), FreeConsole. Returns 0 sent, 2 AttachConsole failed,
/// 3 GenerateConsoleCtrlEvent failed (the exit codes of probes/ctrl_c.py, which P4 and P5 used).
pub fn send_ctrl_c(pid: u32) -> u8;

// cli.rs
#[cfg(windows)] #[command(hide = true)] CtrlC { pid: u32 }   // new Command variant → ExitCode::from(stop::send_ctrl_c(pid))
```

`identity` on Windows: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` fails with `ERROR_INVALID_PARAMETER` → `Gone`, with any other error → `Different`; `GetExitCodeProcess` other than `STILL_ACTIVE` → `Gone`; `owned_by_current_user` `Some(false)` → `Different`, `None` → `Unverified`; no `procStart` → `Unverified`; the `GetProcessTimes` creation FILETIME, `(high << 32) | low` as decimal, differs from `procStart` → `Different`; otherwise `Same` (P2). The handle is closed on every path.

`sessions.rs` reads `procStart` given as a string or as a JSON number (decimal text either way). `cli::stop_session` maps `NoConsole` to `skip("no console to deliver Ctrl+C")`. `cli::main` dispatches with a `match` now that `Command` has two variants on Windows.

- [ ] **Step 1: Write the failing tests**
  - `src/process/windows.rs`: `own_process_with_its_own_start_is_same`, `own_process_with_a_wrong_start_is_different` (`Some("1")`), `own_process_without_recorded_start_is_unverified`, `exited_child_whose_handle_is_held_is_gone` (spawn `cmd /c exit 0`, poll `try_wait` until it returns `Some`, keep the `Child` alive → `Gone`), `system_process_is_not_same` (pid `4` → `!= Same`).
  - `src/sessions.rs`: `reads_numeric_proc_start_as_decimal_text` — record with `"procStart": 134363229627676503` → `proc_start == Some("134363229627676503".into())`.
  - `tests/ctrl_c.rs` (`#![cfg(windows)]`, runs `env!("CARGO_BIN_EXE_claude-reaper")` with `ctrl-c <pid>` against `ping -n 60 127.0.0.1` children):

| Test | Child creation flags | Assert |
|---|---|---|
| `ctrl_c_ends_a_process_in_its_own_console` | `CREATE_NEW_CONSOLE` | helper exit `0`; child exits within 10 s |
| `process_without_a_console_is_reported` | `DETACHED_PROCESS` | helper exit `2`; child still running after 1 s |
| `process_ignoring_ctrl_c_keeps_running` | `CREATE_NEW_CONSOLE \| CREATE_NEW_PROCESS_GROUP` | helper exit `0`; child still running after 2 s |

  - `src/cli.rs`: remove the cfg from `dry_run_reports_the_fixture_session`, `dry_run_reports_a_reused_pid` and both discovered-directory tests, so they run on all three systems; add `#[cfg(windows)] ctrl_c_subcommand_is_hidden` (`ctrl-c 1` parses; the rendered `--help` does not contain `ctrl-c`).
- [ ] **Step 2: Run** `cargo test sessions` and cross-target clippy for Windows — expected: the numeric test fails; Windows does not compile (`windows` modules missing).
- [ ] **Step 3: Implement** the modules, the variant, the subcommand, the `procStart` reader and the cfg wiring (`unverified.rs` and `unsupported.rs` now only for systems that are none of the three).
- [ ] **Step 4: Run** the local check — expected: all pass.
- [ ] **Step 5: Commit** — `git add Cargo.toml Cargo.lock src tests && git commit -m "feat: Windows identity from GetProcessTimes and Ctrl+C stop through the target's console"`
- [ ] **Step 6: Push and wait.** Ask the owner to push the branch, then `gh run watch` the `ci` run. Expected: `check` green on `ubuntu-latest`, `macos-latest` and `windows-latest`. A failure there is a finding: read the log, fix, commit, ask for another push. Do not weaken or cfg away a test to make it pass.

---

### Task 4: Integration check on all three systems

**Files:**
- Modify: `probes/probe.py`, `probes/test_probe.py`, `.github/workflows/integration.yml`, `docs/probes/README.md`, `docs/superpowers/specs/2026-10-09-claude-reaper-design.md` ("Open items")

**Interfaces:**
- Consumes: `integration`, `reaper_prints`, `run_integration_checks` (in `probe.py`); `run` from Tasks 1–3.
- Produces:

```python
IntegrationCase = tuple[str, Mode, list[str], str | None]  # name, launch mode, reaper flags, entrypoint unselected by default
def integration_cases(platform: str) -> list[IntegrationCase]: ...
def reaper_prints(checks, label, reaper, session, flags, prefix, suffix="", config_dir=True) -> None: ...
```

`integration_cases`: always `("VS Code style", "piped", [], None)`; then `("console", "console", ["--entrypoint", "cli"], "cli")` on `win32` (spec: Windows covers the console session of P4 and the pipe-spawned one of P5), otherwise `("terminal", "pty", ["--entrypoint", "cli"], "cli")`. `integration` runs `integration_cases(sys.platform)`.

`run_integration_checks` adds one check, before `dry run reports it`: `dry run finds it without --config-dir`, `reaper_prints(..., [*flags, "--dry-run"], f"would stop {pid} (", config_dir=False)`. It proves discovery through the session's environment (P7) and the `names_config_dir` check on each system.

- [ ] **Step 1: Write the failing tests** in `probes/test_probe.py`: `test_windows_integration_covers_console_and_piped_sessions` (`[case[1] for case in probe.integration_cases("win32")] == ["piped", "console"]`) and `test_unix_integration_covers_terminal_and_piped_sessions` (same for `"linux"` and `"darwin"` → `["piped", "pty"]`).
- [ ] **Step 2: Run** `uv run --no-project --with psutil python -B -m unittest discover -s probes` — expected: fails, no `integration_cases`.
- [ ] **Step 3: Implement** `integration_cases`, the `config_dir` parameter and the new check.
- [ ] **Step 4: Run** the probe tests — expected: all pass.
- [ ] **Step 5: Make `integration.yml` a matrix** like `probes.yml`: `include` of `{os: ubuntu-latest, label: linux, reaper: target/debug/claude-reaper}`, `{os: macos-latest, label: macos, reaper: target/debug/claude-reaper}`, `{os: windows-latest, label: windows, reaper: target/debug/claude-reaper.exe}`; `fail-fast: false`; `timeout-minutes: 20`; the run step uses `--os ${{ matrix.label }} --integration ${{ matrix.reaper }}`; the artifact is `integration-results-${{ matrix.label }}`. Triggers, permissions, pinned actions and the token's step-level `env` stay as they are.
- [ ] **Step 6: Commit** — `git add .github probes && git commit -m "test: integration check on Linux, macOS and Windows"`
- [ ] **Step 7: Push and wait.** Ask the owner to push, then `gh run watch` the `integration` run. Expected: all three jobs green; each artifact shows `yes` for every check of both cases. A `no` is a finding: read the artifact, fix the cause, ask for another push. Do not weaken a check.
- [ ] **Step 8: Record the outcome.**
  - `docs/probes/README.md`: the sentence "The next plan (…) is written from this table." becomes one saying macOS and Windows `process` and `stop` were implemented from this table, with the integration run ID, and that `daemon`, `install`/`uninstall` and release follow. Read the downloaded artifacts for account identifiers before quoting anything from them.
  - Spec "Open items": add `Done <date>: macOS and Windows identity, stop and config-directory discovery implemented from P2, P3b, P4, P5 and P7; the integration check passes on all three systems (workflow run <id>).`
- [ ] **Step 9: Commit** — `git add docs && git commit -m "docs: macOS and Windows stop implemented, integration on all three systems"`; the owner pushes.
- [ ] **Step 10: Final check.** `gh run list --branch <branch>` shows `ci` and `integration` green for the last commit.
