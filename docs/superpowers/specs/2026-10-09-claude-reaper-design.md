# claude-reaper design

Date: 2026-10-09, amended 2026-10-25
Status: approved (amendment approved 2026-10-25)

## Goal

A single native binary that gracefully stops Claude Code sessions that have been idle longer than a threshold. It runs on Linux, macOS and Windows, needs no Python or Node.js, and installs with one command.

The problem it solves is memory held by stale sessions. The VS Code extension keeps every session the user has switched away from running until the window closes, and each one keeps its MCP servers running too. Observed on Linux (2.1.310): four VS Code sessions idle between 96 and 262 minutes, one of them running the Serena MCP server with about ten language servers, were stopped with `SIGINT`. System memory in use dropped by 2.91 GiB (1.02 GiB RAM, 1.89 GiB swap), no child process was left behind, and VS Code's own processes did not shrink. A stop must not disrupt the user: a session stopped in VS Code resumes on the next message with no error, and an unsent draft in its prompt box is kept (see [stop-signal-local.md](../../probes/stop-signal-local.md)).

Terminal sessions are not stopped by default. One terminal process runs one session at a time, closing the terminal ends it, and stopping it returns the user to the shell, so the reaper only helps where a terminal session outlives its user, such as a detached tmux or screen session. Those users opt in with `--entrypoint cli`.

It replaces a Linux-only Python script. The reap rules of that script are kept, except where marked as changed; the platform-specific parts are redesigned.

## Non-goals

- No interactive picker and no AI-generated session summaries.
- No hard kill. The tool never calls `SIGKILL` or `TerminateProcess`, and has no flag to enable it.
- No self-daemonizing. The daemon is a foreground loop; the OS service manager keeps it alive.
- No detection of which session a client is showing. The stop request is chosen so that clients resume the session silently instead.
- Claude Desktop is out of scope. It stops its own hidden idle Code sessions.

## Principle: no guessed platform behaviour

Linux behaviour is known from the existing script. Everything about macOS and Windows is treated as unknown until a GitHub Actions job has observed it against a real Claude Code process on that OS. Where a fact is still unknown, the tool takes the safe branch: it leaves the session alone and reports why.

## Which sessions are reaped

A session is stopped only when all of these hold. Rules 1 to 4 port the Python script; rule 5 is new.

1. Its record in `<config dir>/sessions/*.json` has `kind == "interactive"` and `status == "idle"`.
2. `now - statusUpdatedAt` is at least the idle threshold (default 15 minutes).
3. Its transcript (`<config dir>/projects/*/<sessionId>.jsonl`) shows no pending schedule:
   - no live cron job (see "Live cron jobs"), and
   - the last tool call is not `ScheduleWakeup`.
4. The process with the recorded PID is verifiably the same process that wrote the record (see "Process identity").
5. The record's `entrypoint` is one of the selected entrypoints (see "Entrypoints").

Session records are read from every config directory found as described under "Config directories".

### Entrypoints

Claude Code writes the client that started a session into the record's `entrypoint`, from the `CLAUDE_CODE_ENTRYPOINT` environment variable. Values seen: `claude-vscode` (the VS Code extension), `cli` (a terminal, including VS Code's integrated terminal and JetBrains IDEs), `sdk-py`, `sdk-ts` and `sdk-cli` (Agent SDK programs), `claude-desktop` and `claude-desktop-3p`.

`--entrypoint <name>` selects the entrypoints whose sessions may be stopped. It is repeatable, and replaces the default rather than adding to it. The default is `claude-vscode` only, the one client known to resume a stopped session silently. A session whose `entrypoint` is not selected, or is missing, is skipped with `skip {pid} ({name}): entrypoint {value} not selected`, where a missing value prints as `unknown`.

Agent SDK sessions are not selected by default because the program that started them owns the process and may not expect it to end.

The variable is inherited by child processes, so a `claude` started from inside a VS Code session's own shell is also labelled `claude-vscode`. This is accepted as rare.

`kind` values other than `interactive` (`bg`, `daemon`, `daemon-worker`) belong to background sessions run by Claude Code's own supervisor, which stops idle unattached ones itself after about an hour. Rule 1 already leaves them alone.

This changes the Python script's behaviour, which stopped sessions from every client.

Unreadable or malformed session records are skipped silently, as today. A malformed transcript line is skipped; it does not abort the sweep.

### Config directories

Claude Code keeps its state in `~/.claude` (`%USERPROFILE%\.claude` on Windows). `CLAUDE_CONFIG_DIR` moves it anywhere, and is how people run several accounts side by side. The variable is set where `claude` is launched, so a daemon started by the OS service manager does not see it, and a profile directory can have any name and location.

The reaper looks in these places, and uses each one that contains `sessions/`, once:

1. `~/.claude`.
2. Every distinct `CLAUDE_CONFIG_DIR` value in the environments of the user's own running processes, the reaper's own included. A value that is empty or not an absolute path is ignored.
3. Every directory given with `--config-dir <path>`, which is repeatable. A path that is not a directory is reported as a warning on stderr and does not fail the sweep.

Step 2 needs no configuration and finds exactly the profiles that have something running. Observed on Linux (2.1.294): each of three running sessions carried the variable in `/proc/<pid>/environ`, and it named the directory that held its session record. The session process itself is read, not its children, because `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1` removes the variable from child processes.

Step 2 is implemented on Linux. How to read another process's environment on macOS and Windows is not known; probe P7 determines it, and until then those systems use steps 1 and 3 only, plus the reaper's own environment.

Step 3 covers what step 2 cannot: systems where environments cannot be read, and a `CLAUDE_CONFIG_DIR` set in a user or managed settings file. In that case the variable is expected to be missing from the process's startup environment; this has not been tested.

This changes the Python script's behaviour, which searched `~/.claude*`. That pattern is one user's naming convention, not something Claude Code defines.

### Live cron jobs

Cron jobs exist only in the session's memory, so they are reconstructed from the transcript. A job starts at a `CronCreate` call and ends at a `CronDelete` call with its ID. Claude Code also removes jobs on its own, without writing a `CronDelete`, and the rule differs by job type. The type and schedule are read from the `CronCreate` input (`recurring`, default `true`, and `cron`).

| Job type | Claude Code behaviour | Live until |
|---|---|---|
| Recurring | Auto-expires 7 days after creation | creation time + 7 days |
| One-shot (`recurring: false`) | Fires once at the next cron match, then deletes itself | the first cron match after creation time |

Cron expressions are evaluated in the machine's local time zone, as Claude Code does.

This changes the Python script's behaviour, which applied the 7-day rule to both types. That kept a session unreapable for up to 7 days after a one-shot had fired, and allowed reaping a session whose one-shot was scheduled more than 7 days ahead.

If a one-shot's cron expression cannot be parsed, the job is treated as recurring (live for 7 days), so the session is protected for a bounded time.

## Commands

```
claude-reaper run       [--idle-minutes 15] [--config-dir <path>]... [--entrypoint <name>]... [--dry-run]
claude-reaper daemon    [--idle-minutes 15] [--config-dir <path>]... [--entrypoint <name>]... [--interval-minutes 5]
claude-reaper install   [--idle-minutes 15] [--config-dir <path>]... [--entrypoint <name>]... [--interval-minutes 5]
claude-reaper uninstall
```

- `--config-dir` adds a config directory to look in (see "Config directories"). `install` passes its values on to the registered `daemon`.
- `--entrypoint` selects the clients whose sessions may be stopped (see "Entrypoints"). `install` passes its values on to the registered `daemon`.
- `daemon` finds config directories again on every sweep, so a profile started after the daemon is picked up without a restart.

- `run` performs one sweep and exits. `--dry-run` prints what would be stopped and stops nothing.
- `daemon` repeats the sweep every interval, in the foreground, logging to stdout.
- `install` registers `daemon` to start at login and restart on failure:
  - Linux: a systemd user unit.
  - macOS: a launchd user agent.
  - Windows: a Task Scheduler task that runs at logon in the user's session.
- `uninstall` removes that registration.

Output is one line per session, in the script's current wording: `stopped …`, `would stop …`, `skip … : <reason>`.

## Stopping a session

Stopping is always a request for Claude Code's own shutdown routine, followed by a wait.

| OS | Request | Notes |
|---|---|---|
| Linux, macOS | `SIGINT` to the PID | Claude Code handles `SIGINT` by running its shutdown routine and exits with code 0. |
| Windows | Ctrl+C delivered through the target's console | Claude Code handles it the same way and exits with code 0 (P4, P5). |

The exit code matters to the client that started the session. The VS Code extension reports any non-zero exit code, or death by a signal, as an error in its panel, and only resumes silently after code 0. Observed on Linux (2.1.310) for an idle session, in both the extension's `stream-json` mode and a terminal: `SIGINT` exits with 0, `SIGTERM` with 143 (which showed the error), `SIGHUP` with 129, all removing the record; `SIGQUIT`, `SIGUSR1` and `SIGUSR2` end the process without removing the record. A `SIGINT` sent to an idle terminal session with an empty prompt exits at once; it does not need the second press that a typed Ctrl+C needs. A terminal session whose prompt shows unsent text also exits with 0 on `SIGINT` (P3b, Linux and macOS, 2.1.310); whether the text survives was not checked.

This changes the Python script's behaviour, which sent `SIGTERM`.

After the request, the reaper waits up to a grace period (10 seconds) for the process to exit, then reports either `stopped` or `skip … : did not exit after graceful stop`. It does not escalate.

### Windows mechanics

Windows cannot signal a single process. The only externally generated event that Claude Code handles is Ctrl+C, and it has to be sent through the target's console:

1. `FreeConsole` — a process can be attached to only one console.
2. `AttachConsole(pid)`.
3. `SetConsoleCtrlHandler(NULL, TRUE)` so the sender ignores its own event.
4. `GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)`.
5. `FreeConsole`.

Because step 1 would cost the reaper its own console and output, the sequence runs in a short-lived child: the reaper re-executes itself with a hidden `ctrl-c <pid>` subcommand and reads the child's exit code.

Two consequences are accepted and documented in the README:

- Ctrl+C reaches every process attached to that console, including the shell that launched Claude Code. This is the same as the user pressing Ctrl+C in that terminal.
- A session whose process has no console cannot be stopped gracefully. It is skipped with `skip … : no console to deliver Ctrl+C`. This is expected to be rare: it needs a launcher that creates the process detached. The VS Code extension (2.1.294) spawns the binary with piped stdio and `windowsHide: true`, not detached, which should give it a hidden console of its own; probe P5 confirms or refutes that.

## Process identity

The recorded PID may have been reused by an unrelated process. Before stopping anything, the reaper confirms identity.

- Linux: field 22 (start time in clock ticks) of `/proc/<pid>/stat` must equal the record's `procStart`. This is the script's current check.
- macOS and Windows: the meaning of `procStart` is not known. Probe P2 below determines it, and the check is implemented from what the probe shows. Until then, identity counts as unverified and the session is skipped.

If the record has no `procStart`, the Linux script currently accepts the process. That is kept on Linux only.

## Verification probes (GitHub Actions)

These run before the platform code they inform is written, on `ubuntu-latest`, `macos-latest` and `windows-latest`. Each installs Claude Code with the official installer and starts a real session. Their output is recorded in the repository as the evidence for each platform decision.

| Probe | Question | Decides |
|---|---|---|
| P1 | With a subscription token and pre-seeded onboarding state, does an interactive session start unattended and write a session record? | Whether the other probes can run at all |
| P2 | What does `procStart` contain on macOS and Windows, and which OS value does it equal? | The identity check per OS |
| P3 | macOS: does `SIGTERM` end the session cleanly (process exits, record removed)? | macOS stop (superseded by P3b) |
| P3b | Linux and macOS: does `SIGINT` end an idle session with exit code 0 and remove its record, for a terminal session and for a `stream-json` session started over pipes? | macOS stop, and the Linux stop in CI |
| P4 | Windows, session in a console: does the Ctrl+C sequence end it cleanly? | Windows stop |
| P5 | Windows, session spawned over pipes with no terminal (as the VS Code extension does): does it have a console, and does Ctrl+C end it cleanly? | Whether VS Code sessions on Windows are stoppable or always skipped |
| P6 | Can the daemon, started by launchd or Task Scheduler, see and stop a session in the user's login session? | The `install` design per OS |
| P7 | macOS and Windows: can the reaper read `CLAUDE_CONFIG_DIR` from the environment of a session started by the same user, and with which API? | Whether config directories are discovered there or must be passed with `--config-dir` |

A credential is required. Observed locally on Linux (2.1.280): started with an empty config directory and no credential, Claude Code stops at first-run onboarding and writes no session record. CI therefore authenticates with the repository owner's Claude subscription: `claude setup-token` produces a one-year token that is stored as the `CLAUDE_CODE_OAUTH_TOKEN` repository secret. The probes send no prompts. Workflows that use the secret run only on pushes and manual dispatch in this repository, never on pull requests from forks.

P5 imitates the extension's launch from a small script; it is not the extension itself. The README states that limitation.

If a probe shows that graceful stop is impossible in some situation, that situation becomes a documented skip. It is never worked around with a hard kill.

## Architecture

One Rust crate, one binary.

| Module | Responsibility | Depends on |
|---|---|---|
| `cli` | Argument parsing, command dispatch | all below |
| `sessions` | Keep the config directories that hold sessions, parse session records | filesystem |
| `transcript` | Decide whether a transcript has a pending schedule | filesystem |
| `reaper` | Pure decision: given records, clock and threshold, return what to stop or skip and why | `sessions`, `transcript` |
| `process` | Identity check and config directories of running processes, per OS | OS APIs |
| `stop` | Graceful stop request and wait, per OS | OS APIs |
| `service` | `install` / `uninstall`, per OS | OS service manager |

`reaper` takes the current time as an argument and performs no I/O beyond reading files it is handed, so its rules are testable with fixtures alone. `process`, `stop` and `service` each expose one interface with one implementation per OS, selected at compile time.

Crates: `clap` (arguments), `serde` + `serde_json` (records and transcripts), `croner` + `chrono` (cron expressions in local time), `rustix` (Unix signals), `windows-sys` (console API). Further crates are added only when a probe shows they are needed.

## Error handling

- Expected outcomes (record unreadable, process already gone, identity unverified, no console, did not exit) are reported as `skip` lines and never fail the sweep.
- Unexpected errors in one session are logged and the sweep continues with the next session.
- `run` exits non-zero only when the sweep itself could not run (for example, no home directory). `daemon` logs a failed sweep and continues.

## Testing

- Config directories: a session started with `CLAUDE_CONFIG_DIR` pointing outside the home directory is found with no flag (Linux, by starting a real process with that variable); a directory under home that nothing points to is not guessed; `--config-dir` adds directories.
- Unit tests: `reaper` decisions and `transcript` parsing, against fixture files in temporary directories. Cases mirror each rule above, including `ScheduleWakeup` as the last tool call and, for cron jobs: recurring created, deleted and expired after 7 days; one-shot before and after its fire time; one-shot scheduled more than 7 days ahead; one-shot with an unparseable expression.
- Entrypoints: with no `--entrypoint`, only a `claude-vscode` record is a candidate; a `cli`, an SDK, an unknown and a missing entrypoint are skipped with the entrypoint reason; repeated `--entrypoint` values replace the default.
- Integration tests in CI, on all three operating systems, against a real Claude Code session: `run --dry-run` reports it, `run` stops it, the process exits with code 0, and the process and its session record are gone afterwards.
- Windows integration covers both the console session (P4) and the pipe-spawned session (P5).
- `install` / `uninstall` are exercised in CI on each OS (P6).

## Distribution

cargo-dist builds release binaries for Linux, macOS and Windows and publishes them to GitHub Releases, with generated installers:

```
curl -LsSf https://github.com/asvishnyakov/claude-reaper/releases/latest/download/claude-reaper-installer.sh | sh
irm https://github.com/asvishnyakov/claude-reaper/releases/latest/download/claude-reaper-installer.ps1 | iex
```

## Build order

1. Crate skeleton, `sessions`, `transcript`, `reaper` with unit tests; `run --dry-run` works on all three OSes.
2. Probe workflow (P1–P7); results committed.
3. `process` and `stop` per OS, implemented from the probe results, with the CI integration tests.
4. `daemon`, then `install` / `uninstall`.
5. cargo-dist release configuration and README.

## Open items

- Running the probes requires pushing a branch to GitHub.
- Done 2026-10-09, as reported by the repository owner: `claude setup-token` was run and the result added as the `CLAUDE_CODE_OAUTH_TOKEN` repository secret.
- How to get past first-run onboarding (theme, folder trust) unattended is not yet known; P1 establishes it.
- Done 2026-10-26: P3b confirmed on Linux and macOS (2.1.310) that `SIGINT` ends an idle session with exit code 0 and removes its record, for terminal and pipe-started sessions (see the [P3b row](../../probes/README.md)). macOS stop with `SIGINT` is implemented in the next plan; until then it stays unsupported.
