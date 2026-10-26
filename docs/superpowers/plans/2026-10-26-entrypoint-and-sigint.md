# Entrypoint Selection and SIGINT Stop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the spec's 2026-10-25 amendment: `run` stops only sessions of selected entrypoints (default `claude-vscode`), Linux stops with `SIGINT` so clients see exit code 0, probe P3b records `SIGINT` behaviour on Linux and macOS, and a CI integration check proves on Linux that a real session stopped by `run` exits with code 0.

**Architecture:** The entrypoint rule joins the pure decision in `reaper`, carried with the idle threshold in one `Rules` value that `cli` builds from arguments. The Linux stop changes only its signal. P3b and the integration check reuse the existing probe harness (`start_session`, `stop_probe`, `report`); the Claude Code version both use is pinned in one composite action.

**Tech Stack:** Rust 1.97 (edition 2024), `clap` 4, `rustix` 1; Python via `uv` with `psutil`; GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-09-claude-reaper-design.md` (sections "Entrypoints", "Stopping a session", "Verification probes", "Testing"). Evidence: `docs/probes/stop-signal-local.md`.

**Scope:** `run` only. Not in this plan: macOS and Windows `process`/`stop` (they stay `Unverified`/`Unsupported` whatever P3b shows), `daemon`, `install`/`uninstall`, integration checks on macOS and Windows, release.

## Global Constraints

- No hard kill anywhere: no `SIGKILL`, no `TerminateProcess`, no flag for it. Test cleanup of test-spawned children with `Child::kill` is allowed.
- Output wording, exact: `stopped {pid} ({name}, {config dir name}, idle {n} min)`, `would stop {pid} ({name}, {config dir name}, idle {n} min)`, `skip {pid} ({name}): {reason}`. New reason: `entrypoint {value} not selected`, where a missing value prints as `unknown`.
- `--entrypoint <name>` is repeatable and replaces the default rather than adding to it. Default: `claude-vscode` only.
- Linux stop request: `SIGINT`. Grace period stays `10` seconds; no escalation.
- Claude Code version for probes and integration: `2.1.310`, written in exactly one place (the composite action's input default).
- Workflows that read `CLAUDE_CODE_OAUTH_TOKEN` trigger only on `push` and `workflow_dispatch`. No `pull_request` trigger.
- The integration check refuses to run unless `GITHUB_ACTIONS` is `true`: it runs `claude-reaper run --idle-minutes 0`, which would stop every idle session of the developer's own running profiles.
- Automated Rust tests call `sweep` only with `dry_run = true`, and `stop::stop` only on children the test spawned.
- Nothing committed under `docs/probes/` and nothing printed to a job log may contain the account's email, organisation or display name.
- Pushing to GitHub is the repository owner's step: ask before each push (Tasks 3 and 4), or hand the command over.
- Run Cargo outside the sandbox. Closures use full parameter names (`record`, `entrypoint`).

## Review Focus

1. A record whose `entrypoint` is missing or an empty string: skipped with `entrypoint unknown not selected`, never stopped. Test in Task 1.
2. An idle session of an unselected entrypoint that is still under the threshold, or busy: no line at all, so terminal users don't get a new skip line on every run. Test in Task 1.
3. `--entrypoint ""`: rejected when arguments are parsed, instead of selecting records with an empty value. Test in Task 1.
4. An unselected session whose transcript is unreadable: reported with the entrypoint reason; the transcript is not read. Test in Task 1.
5. A process that ignores `SIGINT`: `did not exit after graceful stop`, the process keeps running, nothing escalates. Test in Task 2.

---

### Task 1: Entrypoint selection

**Files:**
- Modify: `src/sessions.rs`, `src/reaper.rs`, `src/cli.rs` (tests inline in each)

**Interfaces:**
- Produces:

```rust
// sessions.rs — new field, read from the record's "entrypoint" key
pub struct SessionRecord { /* … */ pub entrypoint: Option<String>, /* … */ }

// reaper.rs
pub const DEFAULT_ENTRYPOINT: &str = "claude-vscode";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    pub idle_minutes: u64,
    pub entrypoints: Vec<String>,
}
pub fn decide<'records>(records: &'records [SessionRecord], now: DateTime<Local>, rules: &Rules) -> Vec<Decision<'records>>;
pub fn still_stoppable(record: &SessionRecord, now: DateTime<Local>, rules: &Rules) -> bool;

// cli.rs
pub struct RunArguments { /* … */ pub entrypoint: Vec<String> }
pub fn sweep(config_dirs: &[ConfigDir], now: DateTime<Local>, rules: &Rules, dry_run: bool, output: &mut impl Write) -> io::Result<()>;
```

Rule order in `decide_one`: kind and status (no decision), idle threshold (no decision), **entrypoint** (`Skip` with `entrypoint {value} not selected`, where `None` or `""` prints `unknown`), then the transcript. An empty string never matches a selected entrypoint.

`RunArguments::entrypoint`: `#[arg(long, default_value = DEFAULT_ENTRYPOINT, value_parser = clap::builder::NonEmptyStringValueParser::new())]`, help text `Client whose sessions may be stopped, as recorded in the session's entrypoint; may be repeated, and replaces the default`.

- [ ] **Step 1: Write the failing tests**

`src/sessions.rs`: add `"entrypoint":"claude-vscode"` to `INTERACTIVE_RECORD`; `reads_interactive_record_fields` also expects `entrypoint: Some("claude-vscode")`.

`src/reaper.rs`: the `record()` fixture gets `entrypoint: Some("claude-vscode")`, `write_record_file` writes the same key, and existing calls pass `&rules(15)` from a helper `fn rules(idle_minutes: u64) -> Rules` with `entrypoints: vec![DEFAULT_ENTRYPOINT.to_string()]`. New tests:

| Test | Arrange | Assert |
|---|---|---|
| `unselected_entrypoints_are_skipped` | for each `(entrypoint, shown)` in `(Some("cli"), "cli")`, `(Some("sdk-py"), "sdk-py")`, `(Some("something-new"), "something-new")`, `(None, "unknown")`, `(Some(""), "unknown")`: one record, 20 min, `rules(15)` | `[Skip { reason: format!("entrypoint {shown} not selected") }]` |
| `selected_entrypoints_replace_the_default` | pid 100 `cli`, pid 101 `claude-vscode`, both 20 min; rules `entrypoints: ["cli"]` | `[Stop { pid 100, idle_minutes: 20 }, Skip { pid 101, "entrypoint claude-vscode not selected" }]` |
| `unselected_entrypoint_under_threshold_is_ignored` | `cli`, 14 min | `[]` |
| `unselected_entrypoint_is_skipped_before_reading_the_transcript` | `cli`, 20 min, `projects/p/s1.jsonl` created as a directory | reason == `entrypoint cli not selected` |
| `record_whose_entrypoint_changed_is_not_stoppable` | record file on disk written with entrypoint `cli`; `still_stoppable(&record(dir, 20), now(), &rules(15))` | `false` |

`src/cli.rs`: the fixture record gets `"entrypoint": "claude-vscode"`; `dry_run_output_for` passes default rules. New tests:

| Test | Arrange | Assert |
|---|---|---|
| `entrypoint_defaults_to_claude_vscode` | parse `["claude-reaper", "run"]` | `run.entrypoint == ["claude-vscode"]` |
| `entrypoint_values_replace_the_default` | `--entrypoint cli --entrypoint sdk-py` | `["cli", "sdk-py"]` |
| `empty_entrypoint_is_rejected` | `--entrypoint ""` | `try_parse_from` is `Err` |
| `dry_run_reports_an_unselected_entrypoint` | fixture with entrypoint `cli` | output contains `skip {pid} (fixture): entrypoint cli not selected` |

- [ ] **Step 2: Run** `cargo test` — expected: compile errors (`entrypoint`, `Rules`, `DEFAULT_ENTRYPOINT` not found).
- [ ] **Step 3: Implement** the field, `Rules`, the rule in `decide_one`, the argument, and `cli::main` building `Rules { idle_minutes: run.idle_minutes, entrypoints: run.entrypoint }`.
- [ ] **Step 4: Run** `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` — expected: all pass, no warnings.
- [ ] **Step 5: Manual dry run on this machine:** `cargo run -- run --dry-run`, then `cargo run -- run --dry-run --entrypoint cli`. Expected: in the first, terminal sessions past the threshold appear only as `skip … : entrypoint cli not selected`; in the second, VS Code sessions appear as `skip … : entrypoint claude-vscode not selected`. Nothing is stopped.
- [ ] **Step 6: Commit** — `git add src && git commit -m "feat: stop only sessions of selected entrypoints, claude-vscode by default"`

---

### Task 2: Linux stop with SIGINT

**Files:**
- Modify: `src/stop/linux.rs`

**Interfaces:**
- Consumes/produces: `stop::stop(pid, proc_start, grace) -> io::Result<StopOutcome>`, unchanged.

`still_stoppable` must stay the last check before `stop::stop` in `cli::stop_session`: `SIGINT` to a session that has turned busy cancels its turn instead of ending it.

- [ ] **Step 1: Change the tests first**
  - `terminates_a_child_and_reports_stopped` → rename to `interrupts_a_child_and_reports_stopped`; assert `child.wait().unwrap().signal() == Some(2)`.
  - `reports_did_not_exit_and_leaves_the_process_running`: the child traps `INT` instead of `TERM` (`trap "" INT; echo ready; exec sleep 60`).
- [ ] **Step 2: Run** `cargo test stop` — expected: 2 failures (signal 15 seen; child that ignores `INT` killed by `TERM`, so `Stopped` instead of `DidNotExit`).
- [ ] **Step 3: Implement:** `Signal::INT`; the doc comment says `SIGINT`, which Claude Code answers with its shutdown routine and exit code 0.
- [ ] **Step 4: Run** `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` — expected: all pass. Run tests in the foreground: a shell that starts them with `SIGINT` ignored (`nohup`, a background `&` job) passes that to `sleep`, and the first test then fails.
- [ ] **Step 5: Manual check, with the owner present.** The owner opens a throwaway VS Code session in a throwaway folder, sends one message, switches to another session, waits 2 minutes, runs `cargo run -- run --idle-minutes 1 --dry-run` and confirms only expected sessions are listed, then runs it without `--dry-run`. Expected: `stopped …` for it; returning to it and sending a message resumes it with no error in the panel. This step stops every listed session; do not run it unattended.
- [ ] **Step 6: Commit** — `git add src && git commit -m "feat: stop Linux sessions with SIGINT so clients see exit code 0"`

---

### Task 3: Pinned Claude Code, probe P3b, and its results

**Files:**
- Create: `.github/actions/install-claude-code/action.yml`
- Modify: `.github/workflows/probes.yml`, `probes/probe.py`, `docs/probes/linux.md`, `docs/probes/macos.md`, `docs/probes/README.md`, `docs/superpowers/specs/2026-10-09-claude-reaper-design.md` ("Open items")

**Interfaces:**
- Produces: composite action `./.github/actions/install-claude-code`, input `version` (default `'2.1.310'`); afterwards `claude` is on `PATH` and `DISABLE_AUTOUPDATER=1` is in `GITHUB_ENV`.
- Produces in `probe.py`: `send_sigint(pid: int) -> dict[str, object]`; `stop_probe(name, question, mode, request_stop, draft: str | None = None)`.

- [ ] **Step 1: Write the action.** Unix: `curl -fsSL https://claude.ai/install.sh | bash -s "$VERSION"`; Windows PowerShell: `& ([scriptblock]::Create((irm https://claude.ai/install.ps1))) $env:VERSION` (syntax from https://code.claude.com/docs/en/setup, "Install a specific version"). Pass the input through `env: VERSION: ${{ inputs.version }}`, never by interpolating it into the script. Append the bin directory to `GITHUB_PATH` as `probes.yml` does now, write `DISABLE_AUTOUPDATER=1` to `GITHUB_ENV`, and end with a step that fails unless `claude --version` contains `${VERSION} (Claude Code)`.
- [ ] **Step 2: Use it in `probes.yml`:** replace both install steps, `claude --version` and the job-level `DISABLE_AUTOUPDATER` with `- uses: ./.github/actions/install-claude-code` after checkout.
- [ ] **Step 3: Add P3b to `probe.py`**
  - `stop_probe` adds the observation `Claude Code version` (from `claude_version()`) for every stop probe. With `draft`, after the session is idle it calls `session.send(draft)` without Enter, waits 1 s, and adds `unsent text typed: yes`.
  - Probe `p3b`, skipped on Windows, question `Does SIGINT end an idle session with exit code 0 and remove its record?`, three runs: `P3b (pty)`; `P3b (pty, unsent text)` with draft `draft that was never sent`; `P3b (piped)`.
- [ ] **Step 4: Run** `uv run --no-project --with psutil python -B -m unittest discover -s probes` — expected: existing probe tests pass. Then, outside the sandbox, `uv run --locked --no-build probes/probe.py --os linux` — expected: `results/linux.md` has three P3b sections, `P3b (pty)` and `P3b (piped)` with exit code `0` and `session record removed: true`.
- [ ] **Step 5: Commit** — `git add .github probes && git commit -m "probe: P3b SIGINT on Linux and macOS, Claude Code pinned to 2.1.310"`
- [ ] **Step 6: Push and wait.** Ask the owner to run `git push origin probes`, then `gh run watch` the `probes` run. Expected: all three jobs succeed; Linux and macOS each have three P3b sections reporting version `2.1.310`.
- [ ] **Step 7: Record the results.** `gh run download <run id> --dir results`. Append only the three P3b sections to `docs/probes/linux.md` and `docs/probes/macos.md`, under the line `Added from workflow run <run id> (commit <sha>, Claude Code 2.1.310).`; the earlier sections stay as recorded on 2.1.296. Read the appended text for account identifiers.
- [ ] **Step 8: Update the decisions.**
  - `docs/probes/README.md`: a P3b row (question from the spec's table, one-sentence result per system with section links, decision). Decision when macOS `pty` and `piped` both exit `0` with the record removed: "macOS stop is `SIGINT` followed by a wait, implemented in the next plan"; otherwise macOS stop stays unsupported, and the row says what was seen instead. Mark the P3 decision as superseded by P3b. In "Limits", "One version" names 2.1.310 for P3b.
  - Whatever the unsent-text run shows (exit code, or no exit within 10 s) goes into the README's "Limits" as what `--entrypoint cli` does to a terminal prompt holding text. If the session did not exit or did not exit with `0`, tell the owner; the spec decision on terminal sessions is theirs.
  - Spec "Open items": replace the P3b line with its outcome and a link to the README row.
- [ ] **Step 9: Commit** — `git add docs && git commit -m "docs: P3b results and the macOS stop decision"`; the owner pushes.

---

### Task 4: Integration check on Linux

**Files:**
- Create: `.github/workflows/integration.yml`
- Modify: `probes/probe.py`, `probes/test_probe.py`

**Interfaces:**
- Consumes: `start_session`, `wait_for_status`, `is_running`, `report`, `STOP_TIMEOUT_SECONDS`, `IDLE_TIMEOUT_SECONDS` (all in `probe.py`); the action from Task 3; `run` with `--entrypoint` from Task 1.
- Produces: `def integration(reaper: Path) -> int` in `probe.py` — `0` when every check passed, `1` when any failed, `2` when it refused to run. CLI: `uv run probes/probe.py --os linux --integration <path to claude-reaper>`; results go to `results/integration-<os>.md`.

`integration` runs two cases. Each reaper call is `[reaper, "run", "--idle-minutes", "0", "--config-dir", <session config dir>, *case flags]`, with `--dry-run` where marked; a line "matches" when it starts with the given prefix for the session's PID. Every check is one observation, `yes` or `no: <what was seen>`, and every reaper exit status must be `0`.

| Case | Launch | Checks, in order |
|---|---|---|
| VS Code style | `start_session("piped")`, wait for `idle` | dry run, default entrypoints: `would stop {pid} (`; run: `stopped {pid} (`; session exit code `0` within `STOP_TIMEOUT_SECONDS`; record file gone; `is_running(pid)` false |
| Terminal | `start_session("pty")`, wait for `idle` | dry run, default entrypoints: a line starting `skip {pid} (` and ending `): entrypoint cli not selected`; dry run with `--entrypoint cli`: `would stop {pid} (`; run with `--entrypoint cli`: the same four stop checks |

A session that never starts or never reaches `idle` is a failed check, not a crash. Sessions are closed in `finally`.

- [ ] **Step 1: Write the failing test** in `probes/test_probe.py`:

```python
def test_integration_refuses_to_run_outside_github_actions(self):
    with unittest.mock.patch.dict(os.environ, {}, clear=True), \
         unittest.mock.patch.object(probe, "start_session") as start_session, \
         unittest.mock.patch.object(probe.subprocess, "run") as run:
        result = probe.integration(Path("claude-reaper"))
    self.assertEqual(result, 2)
    start_session.assert_not_called()
    run.assert_not_called()
```

- [ ] **Step 2: Run** `uv run --no-project --with psutil python -B -m unittest discover -s probes` — expected: fails, `probe` has no `integration`.
- [ ] **Step 3: Implement** `integration` and the `--integration` argument (requires `--os`; sets `results_label` to `integration-<os>`). On refusal it prints `refusing to run: the integration check stops sessions and runs only on GitHub Actions` to stderr.
- [ ] **Step 4: Run** the probe tests again — expected: all pass.
- [ ] **Step 5: Write `integration.yml`.** `on: workflow_dispatch` and `push` with `paths: ['src/**', 'Cargo.toml', 'Cargo.lock', 'probes/**', '.github/workflows/integration.yml', '.github/actions/**']`; `permissions: contents: read`; one job on `ubuntu-latest`, `timeout-minutes: 15`. Steps: checkout (same pinned SHA as `ci.yml`); install stable Rust as `ci.yml` does; `astral-sh/setup-uv` pinned as in `probes.yml`; `./.github/actions/install-claude-code`; `cargo build --locked`; `uv run --locked --no-build probes/probe.py --os linux --integration target/debug/claude-reaper` with `CLAUDE_CODE_OAUTH_TOKEN` from secrets in that step's `env`; `actions/upload-artifact` of `results/` as `integration-results-linux` with `if: always()`.
- [ ] **Step 6: Commit** — `git add .github probes && git commit -m "test: CI integration check that run stops a real session with exit code 0"`
- [ ] **Step 7: Push and wait.** Ask the owner to push, then `gh run watch` the `integration` run. Expected: green, and the artifact shows `yes` for every check of both cases. A `no` is a finding: read the artifact, fix the cause, push again. Do not weaken a check to make it pass.
- [ ] **Step 8: Final check.** `cargo test && cargo clippy --all-targets -- -D warnings`, and `gh run list --branch probes` shows `ci`, `probes` and `integration` green for the last commit.
