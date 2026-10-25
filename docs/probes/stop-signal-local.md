# Local findings: stop signal and clients (2026-10-25)

Observed on the developer machine (Ubuntu 24.04, Linux), Claude Code CLI and VS Code extension **2.1.310**, VS Code 1.143.1, after Task 5 Step 8 of the core plan. These are local checks with throwaway Haiku sessions, not CI probes; macOS has not been checked.

## Why a stopped VS Code session showed an error

`claude-reaper run --idle-minutes 1` stopped the session open in the VS Code sidebar. The panel showed an error until another session was opened and then this one again, at which point the extension resumed it under a new PID.

The cause is the exit code, not which session was stopped. The extension (`extension.js`, `getProcessExitError`) reports any non-zero exit code as `Claude Code process exited with code N`, and death by a signal as `terminated by signal`. Only exit code 0 is treated as a normal close. The incident's channels exited with code 143.

## Exit code of an idle session per signal

| Signal | SDK mode (`--input-format stream-json`, as VS Code runs it) | Terminal session (pty) |
|---|---|---|
| `SIGTERM` | 143, record removed, ~1 s | 143, record removed, ~1 s |
| `SIGINT` | **0**, record removed, ~1 s | **0**, record removed, ~1 s |
| `SIGHUP` | 129, record removed | not checked |
| `SIGQUIT`, `SIGUSR1`, `SIGUSR2` | killed outright, **record left behind** | not checked |

Windows already stops with the Ctrl+C sequence, which P4 and P5 recorded as exit code 0, including a pipes-only session like VS Code's.

## VS Code after a `SIGINT` stop

`SIGINT` was sent to idle sessions in a throwaway folder after checking each record's `procStart` and the process's `cwd`:

- Three sessions at once: a background sidebar session, the sidebar session on screen, and an editor tab. All exited cleanly (`Closing Claude on channel`, no exit error in the extension log). Each resumed on the next message with no error in the UI.
- A session with an unsent draft in its prompt box. The draft was still there after the stop, and sending it continued the same conversation.

So with `SIGINT`, the reaper does not need to know which session a VS Code window shows.

## Memory freed by stopping idle VS Code sessions

Four real VS Code sessions idle 96 to 262 minutes (one in a work profile running the Serena MCP server with about ten language servers) were selected with `run --idle-minutes 60 --dry-run` and stopped with `SIGINT` after checking `status` and `procStart`.

| Measure | Before | After |
|---|---|---|
| RAM in use (`free`) | 14854 MiB | 13809 MiB |
| Swap in use | 6281 MiB | 4345 MiB |
| Session processes and their descendants, resident plus swapped | 73 processes, 6.49 GiB | 24 processes, 1.89 GiB |
| VS Code's own processes, resident plus swapped | 6.40 GiB | 6.42 GiB |

The system freed 2.91 GiB. The per-process sum fell by more (4.60 GiB) because shared pages are counted once per process. No Serena or language-server process outlived its session. VS Code's renderers and extension hosts did not shrink, so that part of its memory is out of the reaper's reach.

## Terminal sessions

A terminal session also exits cleanly on `SIGINT` (a real one in a terminal: ~0.75 s, record removed), but the user is returned to the shell and has to run `claude --resume`. Stopping one disrupts the user, so it suits automated runs only.

One terminal process shows one session at a time. Two cases look like more:

- `/resume`, `/clear` and similar switch the conversation inside the same process. The PID stays the same; the record is expected to follow the switch (inferred from the code, not checked).
- `claude agents` shows background sessions, but each is its own process under a supervisor, with record `kind` `bg`, `daemon` or `daemon-worker` (the CLI accepts `interactive`, `bg`, `daemon`, `daemon-worker`). The reaper only acts on `interactive`. According to the agent view docs, the supervisor already stops sessions that are idle and unattached for about an hour.

## Entrypoints seen

Records carry `entrypoint`. Values in this machine's transcripts: `claude-vscode`, `cli`, `sdk-py`, `claude-desktop`. The CLI also knows `sdk-ts`, `sdk-cli`, `claude-desktop-3p`, `local-agent`, `remote`, `ssh-remote` and others. JetBrains IDEs run the CLI in their terminal and have no entrypoint of their own.

## `CLAUDE_CODE_EXIT_AFTER_STOP_DELAY` is not a replacement

The CLI has a documented idle exit for SDK mode ("useful for automated workflows and scripts using SDK mode", since v2.0.35). With it set, VS Code sessions exit cleanly after N ms of idle time, including the one on screen. But it also ends sessions with a pending session cron or `/loop`, which are lost (checked in SDK mode and in VS Code), and terminal sessions never exit with it. The reaper's rules keep sessions with a pending schedule, so the variable is not used.

## Claude Desktop

Out of scope. Desktop (3.6556.2, read from its bundle and logs, not run) stops hidden idle Code sessions itself after 900 s and writes `entrypoint` `claude-desktop` or `claude-desktop-3p`.

## Proposed changes (spec amendment of 2026-10-25)

1. Stop with `SIGINT` instead of `SIGTERM` on Linux and macOS, so that clients see exit code 0. macOS needs a P3 variant for `SIGINT` before relying on it.
2. A repeatable `--entrypoint <name>` option, default `claude-vscode`. Sessions with another or no entrypoint are skipped with `skip {pid} ({name}): entrypoint {value} not selected`. Terminal users opt in with `--entrypoint cli`.
3. Idle threshold, schedule protection and identity checks unchanged.
