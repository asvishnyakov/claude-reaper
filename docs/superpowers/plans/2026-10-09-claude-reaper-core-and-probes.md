# claude-reaper Core and Probes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust binary whose `run` command reproduces the Python script's reap rules (with the spec's two rule changes) and works fully on Linux, plus recorded GitHub Actions evidence (P1–P7) for every macOS and Windows unknown.

**Architecture:** One binary crate. `sessions`, `transcript` and `reaper` are pure file-reading logic tested with fixtures. `process` and `stop` each expose one interface with a Linux implementation and a "not verified" fallback for other systems, so macOS and Windows compile and report skips instead of acting on guesses. Probes are a separate Python script run by a workflow; they share no code with the crate.

**Tech Stack:** Rust 1.97 (edition 2024), `clap` 4, `serde` + `serde_json` 1, `croner` 4.0, `chrono` 0.4, `rustix` 1 (Linux only), `tempfile` (dev). Probes: Python via `uv` with `psutil` and `pywinpty`, Node.js for the P5 launcher, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-09-claude-reaper-design.md`

**Scope:** Spec build-order stages 1 and 2, plus the Linux half of stage 3 (`process` and `stop`), because Linux behaviour is already known and it makes this plan deliver a working replacement for the script. Not in this plan, and planned after Task 9's results exist: macOS and Windows `process`/`stop`, `daemon`, `install`/`uninstall`, cargo-dist, README, real-session integration tests.

## Global Constraints

- No hard kill anywhere, including tests of the tool's own behaviour: never `SIGKILL` or `TerminateProcess` from product code, no flag for it. Test cleanup of test-spawned children with `Child::kill` is allowed.
- No guessed platform behaviour: on macOS and Windows, identity is `Unverified` and stop is `Unsupported` until a later plan implements them from probe results.
- Tests never stop a process they did not spawn. Automated tests call the sweep only with `dry_run = true`; `stop::stop` is called only on children the test started. Reason: config-directory discovery finds the developer's real sessions.
- Tests that go through discovery assert on lines for their own fixture PID only (`contains`), never on the whole output.
- Output wording, exact: `stopped {pid} ({name}, {config dir name}, idle {n} min)`, `would stop {pid} ({name}, {config dir name}, idle {n} min)`, `skip {pid} ({name}): {reason}`. A missing name prints as `unnamed`.
- Idle threshold default `15` minutes. Stop grace period `10` seconds. Recurring cron lifetime `7` days.
- Lambdas and closures use full parameter names (`record`, `entry`), not single letters.
- Workflows that read `CLAUDE_CODE_OAUTH_TOKEN` trigger only on `push` and `workflow_dispatch`. No `pull_request` trigger.
- Nothing committed under `docs/probes/` and nothing printed to a job log may contain the account's email, organisation or display name. Screen text is printed only when a probe fails, after replacing anything matching an email address and the line containing "Welcome" with `<redacted>`.
- Pushing to GitHub is outward-facing: Task 6 Step 1 gets the repository owner's go-ahead once, for the probe branch.
- Run Cargo outside sandbox.

## Review Focus

1. A transcript that exists but cannot be read (permissions, directory in its place): the session is skipped with `transcript unreadable`, not reaped. Test in Task 4.
2. A `CronCreate` result whose entry has a missing or unparseable `timestamp`: the job counts as live until a `CronDelete` names it. Test in Task 3.
3. Files in `sessions/` that are not usable records (`<pid>.<hash>.key` files, JSON with `pid` missing or a string, an empty file): skipped silently, the rest of the directory is still read. Test in Task 2.
4. The same config directory reached twice (`~/.claude` and `CLAUDE_CONFIG_DIR=~/.claude/`, a symlink, a repeated `--config-dir`): each session appears once. Test in Task 2.
5. `statusUpdatedAt` missing or later than now (clock change): the session is not a candidate. Test in Task 4.

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml`, `.gitignore` | Crate manifest; ignore `target/` and `results/` |
| `src/main.rs` | Declare modules, call `cli::main` |
| `src/cli.rs` | Arguments, config-directory assembly, the sweep, output lines |
| `src/sessions.rs` | Filter config directories, read records, locate transcripts |
| `src/transcript.rs` | Pending-schedule decision for one transcript |
| `src/reaper.rs` | Pure decision over records |
| `src/process.rs`, `src/process/linux.rs`, `src/process/unverified.rs` | Identity and running config directories |
| `src/stop.rs`, `src/stop/linux.rs`, `src/stop/unsupported.rs` | Graceful stop and wait |
| `.github/workflows/ci.yml` | fmt, clippy, tests on three systems |
| `.github/workflows/probes.yml` | Install Claude Code, run probes, upload results |
| `probes/probe.py`, `probes/ctrl_c.py`, `probes/spawn-piped.mjs` | Probe harness, Windows Ctrl+C helper, extension-style launcher |
| `docs/probes/{linux,macos,windows}.md`, `docs/probes/README.md` | Recorded evidence and the decisions it supports |

## Observed Facts This Plan Relies On

Observed on Linux, Claude Code 2.1.296, 2026-10-09. Fixtures copy these shapes.

- Session record keys used: `pid` (number), `sessionId`, `name`, `kind`, `status`, `procStart` (all strings), `statusUpdatedAt` (number, ms). `sessions/` also holds `<pid>.<64 hex>.key` files.
- `CronCreate` call, in an `assistant` entry: `{"type":"tool_use","id":"toolu_…","name":"CronCreate","input":{"cron":"7 3 1 1 *","prompt":"…","recurring":false}}`.
- Its result, in a `user` entry with `"timestamp":"2026-10-09T17:10:44.828Z"`: block `{"type":"tool_result","tool_use_id":"toolu_…","content":"Scheduled one-shot task 8251faac (7 3 1 1 *). …"}` and a top-level `"toolUseResult":{"id":"8251faac","humanSchedule":"7 3 1 1 *","recurring":false,"durable":false}`.
- `CronDelete` call input: `{"id":"8251faac"}`.
- `croner` 4.0.1: `Cron::from_str("7 3 1 1 *")?.find_next_occurrence(&created_local, false)` returned `2027-01-01T03:07:00` local for a 2026-10-09 start; `Cron::from_str("not a cron")` is an error; six-field expressions also parse.
- In the 2.1.296 Linux binary, the non-Linux branch of the `procStart` function runs `ps -o lstart= -p <pid>` with `LC_ALL=C TZ=UTC`, and a second record field `procStartFt` exists behind a Windows check with a magnitude test at `300000000000000000`. These are hypotheses for P2, not facts about those systems.

---

### Task 1: Crate skeleton and CI

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `src/main.rs`, `src/cli.rs`, `.github/workflows/ci.yml`

**Interfaces:**
- Produces: `cli::main() -> std::process::ExitCode`; clap command `run` with `--idle-minutes <u64>` (default `15`), `--config-dir <PathBuf>` (repeatable), `--dry-run`.

- [ ] **Step 1: Create the crate**

`Cargo.toml`: package `claude-reaper`, edition `2024`, `rust-version = "1.97"`, license from `LICENSE`. Dependencies: `clap` (feature `derive`), `serde` (feature `derive`), `serde_json`, `croner = "4.0"`, `chrono = "0.4"`; under `[target.'cfg(target_os = "linux")'.dependencies]`: `rustix = { version = "1", features = ["process"] }`; dev: `tempfile`. `.gitignore`: `target/`, `results/`.

- [ ] **Step 2: Write the failing test** in `src/cli.rs`

```rust
#[test]
fn run_defaults_to_fifteen_idle_minutes_and_no_dry_run() {
    let arguments = Arguments::try_parse_from(["claude-reaper", "run"]).unwrap();
    let Command::Run(run) = arguments.command;
    assert_eq!(run.idle_minutes, 15);
    assert!(!run.dry_run);
    assert!(run.config_dir.is_empty());
}

#[test]
fn config_dir_is_repeatable() {
    let arguments = Arguments::try_parse_from(
        ["claude-reaper", "run", "--config-dir", "/a", "--config-dir", "/b"]).unwrap();
    let Command::Run(run) = arguments.command;
    assert_eq!(run.config_dir, [PathBuf::from("/a"), PathBuf::from("/b")]);
}
```

- [ ] **Step 3: Run** `cargo test` — expected: compile error, `Arguments` not found.

- [ ] **Step 4: Implement** `Arguments`, `Command::Run(RunArguments)` and `cli::main` (parses, prints nothing yet, returns `ExitCode::SUCCESS`); `src/main.rs` calls it.

- [ ] **Step 5: Run** `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` — expected: 2 passed, no warnings.

- [ ] **Step 6: Add `.github/workflows/ci.yml`**

Triggers `push` and `pull_request`. One job, matrix `os: [ubuntu-latest, macos-latest, windows-latest]`, `fail-fast: false`, `permissions: contents: read`. Steps: checkout, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. It uses no secrets.

- [ ] **Step 7: Commit** — `git add Cargo.toml Cargo.lock .gitignore src .github/workflows/ci.yml && git commit -m "feat: crate skeleton with run arguments and CI"`

---

### Task 2: `sessions`

**Files:**
- Create: `src/sessions.rs` (tests inline)

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    pub pid: u32,
    pub session_id: Option<String>,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub status_updated_at: Option<i64>, // ms since epoch
    pub proc_start: Option<String>,
    pub config_dir: PathBuf,            // not from JSON; set by read_records
}
/// Keeps directories that contain `sessions/`, once each, in first-seen order.
pub fn session_config_dirs(candidates: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf>;
/// Records from `<config_dir>/sessions/*.json`, sorted by file name. Unusable files are skipped silently.
pub fn read_records(config_dir: &Path) -> Vec<SessionRecord>;
/// First `<config_dir>/projects/*/<session_id>.jsonl`.
pub fn find_transcript(config_dir: &Path, session_id: &str) -> Option<PathBuf>;
```

- [ ] **Step 1: Write the failing tests**, each in a `tempfile::tempdir()`:

| Test | Arrange | Assert |
|---|---|---|
| `reads_interactive_record_fields` | `sessions/100.json` = `{"pid":100,"sessionId":"s1","name":"fixture","kind":"interactive","status":"idle","statusUpdatedAt":1791580419201,"procStart":"2202283","version":"2.1.296"}` | one record with exactly those values and `config_dir` = the tempdir |
| `skips_files_that_are_not_usable_records` | `100.json` valid; `101.json` = `{"pid":"101"}`; `102.json` = `{"kind":"interactive"}`; `103.json` empty; `104.json` = `not json`; `100.aaaa.key` = `x` | `read_records` returns only pid 100 |
| `records_are_sorted_by_file_name` | `200.json`, `100.json` | pids `[100, 200]` |
| `missing_sessions_directory_gives_no_records` | empty tempdir | empty vec |
| `keeps_only_directories_with_sessions` | `a/sessions/`, `b/` | `session_config_dirs([a, b])` == `[a]` |
| `same_directory_reached_twice_is_kept_once` | `a/sessions/`; candidates `a`, `a` with a trailing separator, and (on Unix) a symlink to `a` | one entry |
| `finds_transcript_in_any_project` | `projects/p1/other.jsonl`, `projects/p2/s1.jsonl` | `find_transcript(dir, "s1")` == the `p2` path; `"nope"` → `None` |

- [ ] **Step 2: Run** `cargo test sessions` — expected: compile error, items not found.
- [ ] **Step 3: Implement.** Deserialize with `#[serde(rename_all = "camelCase")]`; deduplicate on `std::fs::canonicalize` but return the path as given the first time. No glob crate: two `read_dir` calls are enough.
- [ ] **Step 4: Run** `cargo test sessions` — expected: 7 passed.
- [ ] **Step 5: Commit** — `git add src/sessions.rs src/main.rs && git commit -m "feat: read session records and locate transcripts"`

Tasks 2–4 add modules nothing calls yet, so `cargo clippy -- -D warnings` reports dead code until Task 5 wires them in; those tasks verify with `cargo test` only.

---

### Task 3: `transcript`

**Files:**
- Create: `src/transcript.rs` (tests inline)

**Interfaces:**
- Produces: `pub fn has_pending_schedule(transcript: &Path, now: DateTime<Local>) -> std::io::Result<bool>` — `Err` only when the file cannot be opened or read.

Rules, in transcript order (port of `has_pending_schedule` in `script.py`, with the spec's cron change):

- Lines without the substring `"tool_` are not parsed. A line that fails to parse, or whose `message.content` is not an array, or a block that lacks the fields needed, is skipped.
- `tool_use` block: remember `id → (name, input)`; it becomes the last tool call. If `name == "CronDelete"`, remove job `input.id`.
- `tool_result` block whose `tool_use_id` maps to a `CronCreate`: job ID is the entry's top-level `toolUseResult.id` when it is a string, otherwise the first match of `\btask ([0-9a-f]{8})\b` in the block's `content` serialized as JSON. No ID → no job. Record the job with the entry's `timestamp` (RFC 3339), and the call's `input.cron` and `input.recurring` (default `true`).
- A job is live when: timestamp missing or unparseable → live; recurring → `now - created < 7 days`; one-shot → `now <` first match of `cron` after `created`, evaluated in `Local`; one-shot whose `cron` is missing or does not parse → the recurring rule.
- Result: any live job, or last tool call is `ScheduleWakeup`.

- [ ] **Step 1: Write the failing tests.** Add a test helper that writes entries to a temp `.jsonl` using the shapes under "Observed Facts", and `fn at(text: &str) -> DateTime<Local>` parsing an RFC 3339 string. Offsets are a day or more so the tests pass in any time zone.

| Test | Transcript | `now` | Expect |
|---|---|---|---|
| `no_tool_calls_means_nothing_pending` | one plain user entry | any | `false` |
| `recurring_job_is_live_within_seven_days` | create `*/5 * * * *` (no `recurring` key) at `2026-10-01T00:00:00Z` | `2026-10-07T00:00:00Z` | `true` |
| `recurring_job_expires_after_seven_days` | same | `2026-10-09T00:00:00Z` | `false` |
| `deleted_job_is_not_live` | same, then `CronDelete {"id": <job id>}` | `2026-10-02T00:00:00Z` | `false` |
| `one_shot_is_live_before_it_fires` | create `7 3 1 1 *`, `recurring:false`, at `2026-10-09T17:10:44.828Z` | `2026-12-30T12:00:00Z` | `true` (82 days ahead: also the "more than 7 days" case) |
| `one_shot_is_gone_after_it_fires` | same | `2027-01-03T12:00:00Z` | `false` |
| `fired_one_shot_does_not_protect_for_seven_days` | create `*/5 * * * *`, `recurring:false`, at `2026-10-09T17:10:44.828Z` | `2026-10-10T17:10:44Z` | `false` |
| `unparseable_one_shot_uses_the_seven_day_rule` | create `not a cron`, `recurring:false`, at `2026-10-01T00:00:00Z` | `2026-10-07T00:00:00Z` → `true`; `2026-10-09T00:00:00Z` → `false` | |
| `job_id_falls_back_to_result_text` | result entry without `toolUseResult`, content `Scheduled recurring task 0a1b2c3d (…)`, then `CronDelete {"id":"0a1b2c3d"}` | 1 day later | `false` |
| `job_without_timestamp_stays_live` | create result entry with no `timestamp` | `2030-01-01T00:00:00Z` | `true` |
| `schedule_wakeup_as_last_tool_call_is_pending` | `Read`, then `ScheduleWakeup` | any | `true` |
| `schedule_wakeup_followed_by_another_tool_is_not_pending` | `ScheduleWakeup`, then `Read` | any | `false` |
| `malformed_line_is_skipped` | recurring create, a line `{"tool_ broken`, a line `{"message":{"content":"tool_ text"}}`, a line whose content is `[1, null]` | 1 day after create | `Ok(true)` |
| `missing_file_is_an_error` | path that does not exist | any | `Err` |

- [ ] **Step 2: Run** `cargo test transcript` — expected: compile error.
- [ ] **Step 3: Implement.** Parse lines as `serde_json::Value`; the ID pattern is a hand-written scan for `task ` followed by eight lowercase hex characters and a non-word boundary, so no regex crate is added.
- [ ] **Step 4: Run** `cargo test transcript` — expected: 14 passed.
- [ ] **Step 5: Commit** — `git add src/transcript.rs && git commit -m "feat: detect pending cron jobs and wakeups in transcripts"`

---

### Task 4: `reaper`

**Files:**
- Create: `src/reaper.rs` (tests inline)

**Interfaces:**
- Consumes: `sessions::{SessionRecord, find_transcript}`, `transcript::has_pending_schedule`.
- Produces:

```rust
#[derive(Debug, PartialEq)]
pub enum Decision<'records> {
    Stop { record: &'records SessionRecord, idle_minutes: i64 },
    Skip { record: &'records SessionRecord, reason: String },
}
/// Records that are not interactive, not idle, or not idle long enough produce no decision.
pub fn decide(records: &[SessionRecord], now: DateTime<Local>, idle_minutes: u64) -> Vec<Decision<'_>>;
```

Skip reasons, exact: `pending cron or /loop wakeup`; `transcript unreadable: {io error}`.

- [ ] **Step 1: Write the failing tests.** Fixture: a config dir in a tempdir, `now` fixed, records built in code with `status_updated_at = now_ms - minutes * 60_000`.

| Test | Record / files | Expect |
|---|---|---|
| `idle_interactive_session_past_threshold_is_stopped` | interactive, idle, 20 min, no transcript file | `[Stop { idle_minutes: 20 }]` |
| `session_exactly_at_threshold_is_stopped` | 15 min | `Stop { idle_minutes: 15 }` |
| `session_under_threshold_is_ignored` | 14 min | `[]` |
| `busy_session_is_ignored` | status `busy`, 60 min | `[]` |
| `non_interactive_session_is_ignored` | kind `bg`, idle, 60 min | `[]` |
| `missing_status_timestamp_is_ignored` | `status_updated_at: None` | `[]` |
| `status_timestamp_in_the_future_is_ignored` | `now_ms + 3_600_000` | `[]` |
| `pending_schedule_is_skipped` | transcript `projects/p/s1.jsonl` whose last tool call is `ScheduleWakeup` | `[Skip { reason: "pending cron or /loop wakeup" }]` |
| `transcript_without_schedule_is_stopped` | transcript with one `Read` call | `Stop` |
| `unreadable_transcript_is_skipped` | `projects/p/s1.jsonl` created as a directory | `Skip` whose reason starts with `transcript unreadable: ` |
| `record_without_session_id_is_stopped` | `session_id: None`, 20 min | `Stop` (the script's behaviour) |

- [ ] **Step 2: Run** `cargo test reaper` — expected: compile error.
- [ ] **Step 3: Implement** `decide`.
- [ ] **Step 4: Run** `cargo test reaper` — expected: 11 passed.
- [ ] **Step 5: Commit** — `git add src/reaper.rs && git commit -m "feat: decide which sessions to stop"`

---

### Task 5: `process`, `stop`, and the `run` sweep

**Files:**
- Create: `src/process.rs`, `src/process/linux.rs`, `src/process/unverified.rs`, `src/stop.rs`, `src/stop/linux.rs`, `src/stop/unsupported.rs`
- Modify: `src/cli.rs`, `src/main.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–4.
- Produces:

```rust
// process.rs — implementation chosen with #[cfg(target_os = "linux")] / #[cfg(not(...))]
#[derive(Debug, PartialEq, Eq)]
pub enum Identity { Same, Different, Gone, Unverified }
pub fn identity(pid: u32, proc_start: Option<&str>) -> Identity;
/// Distinct absolute CLAUDE_CONFIG_DIR values of the user's running processes, this one included.
pub fn running_config_dirs() -> Vec<PathBuf>;

// stop.rs
#[derive(Debug, PartialEq, Eq)]
pub enum StopOutcome { Stopped, DidNotExit, Unsupported }
pub const GRACE: Duration = Duration::from_secs(10);
pub fn stop(pid: u32, proc_start: Option<&str>, grace: Duration) -> std::io::Result<StopOutcome>;

// cli.rs
pub fn config_dirs(home: &Path, discovered: Vec<PathBuf>, flagged: &[PathBuf], warnings: &mut impl Write) -> Vec<PathBuf>;
pub fn sweep(config_dirs: &[PathBuf], now: DateTime<Local>, idle_minutes: u64, dry_run: bool, output: &mut impl Write) -> std::io::Result<()>;
```

Linux behaviour:
- `linux::parse_start_ticks(stat: &str) -> Option<(char, &str)>` (`pub(crate)`): process state and field 22, counted after the last `)` so a command name containing spaces or `)` cannot shift fields.
- `identity`: `/proc/<pid>/stat` unreadable, or state `Z` → `Gone`; `proc_start` is `None` → `Same`; equal → `Same`; else `Different`.
- `running_config_dirs`: for each numeric `/proc/<pid>` owned by the current uid, read `environ`, split on `\0`, take `CLAUDE_CONFIG_DIR=`; ignore empty and non-absolute values and unreadable files; deduplicate.
- `stop`: `rustix::process::kill_process(pid, Signal::TERM)` (rustix 1 spells it `Signal::TERM`); "no such process" counts as `Stopped`; then poll `identity` every 100 ms until it is not `Same` (`Stopped`) or `grace` has passed (`DidNotExit`). Other errors are returned.

Other systems: `identity` → `Unverified`; `running_config_dirs` → this process's own `CLAUDE_CONFIG_DIR` if absolute; `stop` → `Ok(Unsupported)`. There, `Identity` and `StopOutcome` have variants nothing constructs; put `#[cfg_attr(not(target_os = "linux"), allow(dead_code))]` on both enums so clippy passes on macOS and Windows.

`config_dirs`: candidates in order `home/.claude`, `discovered`, `flagged`; a flagged path that is not a directory writes `warning: --config-dir {path} is not a directory` to `warnings` and is dropped; the list then goes through `sessions::session_config_dirs`.

`sweep`, per decision from `reaper::decide` over each directory's records:

| Situation | Line |
|---|---|
| `Decision::Skip` | `skip {pid} ({name}): {reason}` |
| identity `Gone` | `skip {pid} ({name}): process already gone` |
| identity `Different` | `skip {pid} ({name}): PID reused by another process` |
| identity `Unverified` | `skip {pid} ({name}): process identity unverified` |
| `Same`, `dry_run` | `would stop {pid} ({name}, {dir name}, idle {n} min)` |
| `Same`, stop `Stopped` | `stopped {pid} ({name}, {dir name}, idle {n} min)` |
| `Same`, stop `DidNotExit` | `skip {pid} ({name}): did not exit after graceful stop` |
| `Same`, stop `Unsupported` | `skip {pid} ({name}): graceful stop not verified on this OS` |
| `Same`, stop `Err` | to stderr: `error stopping {pid} ({name}): {error}`; continue |

`cli::main` for `run`: `std::env::home_dir()` missing → message on stderr, non-zero exit. Otherwise `config_dirs(home, process::running_config_dirs(), flags, stderr)` then `sweep(..., Local::now(), ..., stdout)`.

- [ ] **Step 1: Write the failing tests**

`src/process/linux.rs` (all `#[cfg(target_os = "linux")]`):

| Test | Arrange | Assert |
|---|---|---|
| `start_ticks_survive_parentheses_in_the_command_name` | `"1234 (a) b) S 1 1234 1234 0 -1 4194304 1 0 0 0 0 0 0 0 20 0 1 0 2202283 1000 100"` | `Some(('S', "2202283"))` |
| `own_process_without_recorded_start_is_same` | `identity(std::process::id(), None)` | `Same` |
| `own_process_with_a_wrong_start_is_different` | `identity(std::process::id(), Some("1"))` | `Different` |
| `exited_child_is_gone` | spawn `true`, `wait()` it | `identity(child_pid, None)` == `Gone` |
| `finds_config_dir_of_a_running_process` | spawn `sleep 30` with `CLAUDE_CONFIG_DIR` = a tempdir (under `$TMPDIR`, outside home) | `running_config_dirs()` contains it; kill and wait the child |
| `ignores_relative_and_empty_config_dir_values` | spawn two `sleep 30` with `CLAUDE_CONFIG_DIR=relative/unique-marker` and `=` empty | no returned path ends with `unique-marker`; no empty path |

`src/stop/linux.rs`:

| Test | Arrange | Assert |
|---|---|---|
| `terminates_a_child_and_reports_stopped` | spawn `sleep 60` | `stop(pid, None, 5 s)` == `Ok(Stopped)`; `child.wait()` shows it died from signal 15 |
| `reports_did_not_exit_and_leaves_the_process_running` | spawn `sh -c 'trap "" TERM; echo ready; sleep 60'`, read the `ready` line first | `stop(pid, None, 300 ms)` == `Ok(DidNotExit)`; `child.try_wait()` is `Ok(None)`; then `child.kill()` and wait |

`src/cli.rs` (all systems; the record's `pid` is `std::process::id()` with no `procStart`, idle 20 min, name `fixture`):

| Test | Arrange | Assert |
|---|---|---|
| `home_claude_directory_is_used` | fake home with `.claude/sessions/` | `config_dirs(home, vec![], &[], sink)` == `[home/.claude]` |
| `unreferenced_profile_under_home_is_not_guessed` | fake home with `.claude-other/sessions/` only | `[]` |
| `discovered_and_flagged_directories_are_added_once` | `d/sessions/` passed as discovered and as flag | one entry |
| `flag_that_is_not_a_directory_warns_and_continues` | flag `home/missing` | result excludes it; warnings == `warning: --config-dir {path} is not a directory\n` |
| `dry_run_reports_the_fixture_session` | fixture config dir | Linux: output contains `would stop {pid} (fixture, {dir name}, idle 20 min)`; other systems: contains `skip {pid} (fixture): process identity unverified` |
| `dry_run_reports_a_pending_schedule` | fixture plus transcript ending in `ScheduleWakeup` | contains `skip {pid} (fixture): pending cron or /loop wakeup` |
| `dry_run_reports_a_reused_pid` (Linux) | fixture with `procStart: "1"` | contains `skip {pid} (fixture): PID reused by another process` |
| `session_without_a_name_prints_unnamed` | fixture without `name` | contains `{pid} (unnamed` |

- [ ] **Step 2: Run** `cargo test` — expected: compile errors for the new items.
- [ ] **Step 3: Implement** `process` (both variants) and `stop` (both variants).
- [ ] **Step 4: Run** `cargo test process stop` — expected on Linux: 8 passed.
- [ ] **Step 5: Implement** `config_dirs`, `sweep`, and wire `run` in `cli::main`.
- [ ] **Step 6: Run** `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` — expected: all pass, no warnings (in particular none about `std::env::home_dir`).
- [ ] **Step 7: Manual check on this machine, dry run only**

Run: `cargo run -- run --dry-run`
Expected: exit 0; every line starts with `would stop ` or `skip `; sessions from each running profile directory can appear, with that directory's name in the label. Compare with `python3` running the original script with `--dry-run` if it is installed; differences must be explained by the two documented rule changes (config directories, one-shot crons) or the new identity lines.

- [x] **Step 8: Manual check of a real stop, with the owner present**

The owner starts a throwaway `claude` session in a terminal, leaves it idle 1 minute, then runs `cargo run -- run --idle-minutes 1 --dry-run`, confirms only expected sessions are listed, and only then runs it without `--dry-run`. Expected: `stopped …` for the throwaway session, its process is gone, `sessions/<pid>.json` is removed. This step stops every listed session; do not run it unattended.

- [ ] **Step 9: Commit** — `git add src && git commit -m "feat: run sweep with Linux identity check and graceful stop"`; push the branch only as part of Task 6 Step 1.

---

### Task 6: Probe harness and P1 (a session starts unattended)

**Files:**
- Create: `probes/probe.py`, `.github/workflows/probes.yml`
- Create (from results): `docs/probes/linux.md`, `docs/probes/macos.md`, `docs/probes/windows.md`

**Interfaces:**
- Produces, in `probes/probe.py` (PEP 723 header: `requires-python = ">=3.12"`, dependencies `psutil`, `pywinpty; sys_platform == "win32"`):

```python
@dataclass
class Session:
    pid: int
    config_dir: Path
    record_path: Path            # config_dir / "sessions" / f"{pid}.json"
    def record(self) -> dict | None: ...
    def screen_text(self) -> str: ...     # redacted; "" when the launch mode has no terminal to read
    def close(self) -> None: ...          # test cleanup only

def start_session(mode: Literal["pty", "console", "piped"]) -> Session | None:
    """Fresh config dir, seeded state, real `claude`; waits up to 60 s for the record. None if it never appears."""

def report(probe: str, question: str, observations: dict[str, object]) -> None:
    """Appends one section to results/<os>.md and prints it."""
```

CLI: `uv run probes/probe.py --os <linux|macos|windows>` runs every probe function registered so far, each in its own `try` so one failure does not hide the others, and always exits 0 unless P1 fails.

Launch modes: `pty` = Unix `pty.fork`, Windows `winpty.PtyProcess` (ConPTY); `console` = Windows only, `subprocess.Popen(creationflags=CREATE_NEW_CONSOLE)`; `piped` = Task 8.

Seeding (the hypothesis P1 tests; key names seen in a real `.claude.json`): `<config dir>/.claude.json` = `{"hasCompletedOnboarding": true, "lastOnboardingVersion": "<claude --version>", "projects": {"<cwd>": {"hasTrustDialogAccepted": true}}}`; environment `CLAUDE_CONFIG_DIR=<config dir>`, `CLAUDE_CODE_OAUTH_TOKEN` passed through; cwd = a fresh empty directory. No prompt is ever sent.

- [ ] **Step 1: Get the go-ahead and check the secret**

Ask the repository owner to approve pushing branch `probes` to `origin`. Then run `gh secret list --repo asvishnyakov/claude-reaper` — expected: a `CLAUDE_CODE_OAUTH_TOKEN` row. Stop here if either is missing.

- [ ] **Step 2: Write `probes.yml`**

`on: workflow_dispatch` and `push` with `paths: ['probes/**', '.github/workflows/probes.yml']`. `permissions: contents: read`. Job matrix `os: [ubuntu-latest, macos-latest, windows-latest]`, `fail-fast: false`, `timeout-minutes: 15`, `env: CLAUDE_CODE_OAUTH_TOKEN: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}`. Steps: checkout; `astral-sh/setup-uv`; install Claude Code with the official installer (`curl -fsSL https://claude.ai/install.sh | bash` on Unix, `irm https://claude.ai/install.ps1 | iex` on Windows — confirm both against https://code.claude.com/docs/en/setup before the first push) and append its bin directory to `$GITHUB_PATH`; `claude --version`; `uv run probes/probe.py --os <label>`; `actions/upload-artifact` of `results/` as `probe-results-<label>` with `if: always()`.

- [ ] **Step 3: Write `probe.py` with `start_session("pty")` and probe `p1`**

`p1` reports: Claude Code version, whether the record appeared and after how many seconds, the record's key names and the values of `kind`, `status`, `entrypoint`, `procStart`, `procStartFt`, whether `status` reached `idle` within 60 s, and on failure the redacted screen text.

- [ ] **Step 4: Run locally on Linux as far as possible**

Run: `uv run probes/probe.py --os linux` without the token. Expected: the harness runs end to end and P1 reports "record did not appear" with a redacted onboarding screen — this reproduces the spec's 2.1.280 observation and proves the failure path prints usable evidence.

- [ ] **Step 5: Push and iterate until P1 passes on all three systems**

Run: `git push -u origin probes`, then `gh run watch`. If a system reports no record, read the redacted screen text in that job's results, change one thing (a seed key, or one keystroke sent to the terminal such as Enter), push again. Record each attempt and its outcome as a bullet in that system's P1 section. If a system cannot be made to start after the screen text no longer shows a dialog, record that as the P1 result for it and continue with the others.

- [ ] **Step 6: Commit the results**

Run: `gh run download <run id> --dir results`, copy each `results/<os>.md` to `docs/probes/<os>.md`, check the constraint on account identifiers by reading the three files, then `git add probes .github/workflows/probes.yml docs/probes && git commit -m "probe: P1 unattended session start"` and push.

---

### Task 7: P2 and P7 (read-only observations)

**Files:**
- Modify: `probes/probe.py`, `docs/probes/*.md`

**Interfaces:**
- Consumes: `start_session`, `report`.

- [ ] **Step 1: Add probe `p2`** — for one `pty` session per system, report the record's `procStart` and `procStartFt` next to each candidate and whether it is equal as a string:

| System | Candidates |
|---|---|
| Linux (control) | field 22 of `/proc/<pid>/stat` |
| macOS | output of `ps -o lstart= -p <pid>` with `LC_ALL=C TZ=UTC`, trimmed; the same without those variables; `psutil.Process(pid).create_time()` |
| Windows | creation `FILETIME` from `GetProcessTimes` as a decimal 64-bit integer; `(Get-CimInstance Win32_Process -Filter "ProcessId=<pid>").CreationDate`; `psutil.Process(pid).create_time()` |

On Windows run it for both `pty` and `console` sessions.

- [ ] **Step 2: Add probe `p7`** — start a session with `CLAUDE_CONFIG_DIR` set to its fresh directory and report whether `psutil.Process(pid).environ().get("CLAUDE_CONFIG_DIR")` equals that directory, or the exception type and message. Name the underlying OS call in the report text (macOS `sysctl` `KERN_PROCARGS2`; Windows reading the process environment block with `ReadProcessMemory`).

- [ ] **Step 3: Push, wait for the run, download** — expected: a P2 and a P7 section per system; the Linux P2 row is equal (control). If the Linux control is not equal, the probe is wrong: fix it before trusting the other rows.

- [ ] **Step 4: Commit** — copy results into `docs/probes/`, `git commit -m "probe: P2 process start identity, P7 environment reading"`, push.

---

### Task 8: P3, P4, P5 (graceful stop)

**Files:**
- Create: `probes/ctrl_c.py`, `probes/spawn-piped.mjs`
- Modify: `probes/probe.py`, `docs/probes/*.md`

**Interfaces:**
- Consumes: `start_session`, `report`.
- Produces: `probes/ctrl_c.py <pid>` — exits `0` when the event was generated, `2` when `AttachConsole` failed (prints `GetLastError`), `3` when `GenerateConsoleCtrlEvent` failed. `start_session("piped")`.

Every stop probe reports the same four observations: did the process exit within 10 s and after how long; its exit code; is `sessions/<pid>.json` gone; were any other processes that shared its console still alive afterwards (Windows).

- [ ] **Step 1: Add probe `p3`** (macOS; also Linux as control) — `os.kill(pid, signal.SIGTERM)` on a `pty` session.

- [ ] **Step 2: Write `ctrl_c.py`** — with `ctypes`, exactly the spec's sequence: `FreeConsole`, `AttachConsole(pid)`, `SetConsoleCtrlHandler(None, True)`, `GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)`, `FreeConsole`.

- [ ] **Step 3: Add probe `p4`** (Windows) — run `ctrl_c.py` as a child process against a `console` session and, separately, a `pty` (ConPTY) session; report the helper's exit code plus the four observations for each.

- [ ] **Step 4: Write `spawn-piped.mjs` and `start_session("piped")`**

The launcher calls `child_process.spawn(claude, args, { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true })`, prints the child PID as its first stdout line, and stays alive holding the pipes. For `args`, copy the flags of a live VS Code extension session on the developer machine (`tr '\0' '\n' < /proc/<pid>/cmdline` for a session whose record has `"entrypoint":"claude-vscode"`), leaving out anything that carries a path, port, token or session ID specific to that machine; list the flags used in the P5 report.

- [ ] **Step 5: Add probe `p5`** (Windows; Linux and macOS as controls using SIGTERM) — on a `piped` session report: did a session record appear and its `kind` and `entrypoint`; `ctrl_c.py` exit code and `GetLastError` if it failed; the four observations.

- [ ] **Step 6: Push, wait, download** — expected: P3 on macOS, P4 twice and P5 on Windows, each with all observations filled in. A "did not exit" or "no console" result is a valid finding, not a failed step.

- [ ] **Step 7: Commit** — `git commit -m "probe: P3-P5 graceful stop on macOS and Windows"`, push.

---

### Task 9: P6 (service manager) and the decision summary

**Files:**
- Modify: `probes/probe.py`, `docs/probes/*.md`
- Create: `docs/probes/README.md`

**Interfaces:**
- Consumes: `start_session`, `report`, `probes/ctrl_c.py`.
- Produces: `uv run probes/probe.py --service-check <config dir> <pid> <out file>` — run by the service manager, not by the workflow: writes whether `sessions/<pid>.json` under that directory is readable, sends the system's stop request, and writes the four stop observations.

- [ ] **Step 1: Add probe `p6`, macOS** — start a `pty` session; write a launchd agent plist whose `ProgramArguments` is the `--service-check` command; `launchctl bootstrap gui/$(id -u) <plist>`; wait up to 30 s for the out file; `launchctl bootout`. Report the `launchctl` exit codes and output, and the out file's contents or its absence.

- [ ] **Step 2: Add probe `p6`, Windows** — start a `console` session; `schtasks /create /tn claude-reaper-probe /sc onlogon /it /tr "<--service-check command>" /f`; `schtasks /run /tn claude-reaper-probe`; wait up to 30 s; `schtasks /delete /tn claude-reaper-probe /f`. Report the same. A logon cannot be triggered in CI; say so in the report, and that `/run` stands in for it.

- [ ] **Step 3: Push, wait, download, commit** — `git commit -m "probe: P6 stop from launchd and Task Scheduler"`.

- [ ] **Step 4: Write `docs/probes/README.md`**

One row per probe: question (from the spec's table), result per system in one sentence with a link to the section that shows it, and the decision it supports, worded as the spec's "Decides" column asks. Where a probe could not answer, the decision is the spec's safe branch (skip the session, or require `--config-dir`). State the limits: P5 imitates the extension's launch, P6 on Windows used `/run`, and the Claude Code version each result was observed on.

- [ ] **Step 5: Final check and commit**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`, and confirm the `ci.yml` run for the pushed branch is green on all three systems (`gh run list --workflow ci.yml --branch probes`). Then `git add docs/probes/README.md && git commit -m "docs: probe decisions for macOS and Windows"` and push.

The next plan (macOS and Windows `process`/`stop`, `daemon`, `install`/`uninstall`, release) is written from `docs/probes/README.md`.
