# Probe results: linux

Runner: Linux-6.17.0-1022-azure-x86_64-with-glibc2.39, Python 3.12.3

## P1: Does an interactive session start unattended with pre-seeded onboarding state, and write a session record?

- Claude Code version: 2.1.296 (Claude Code)
- token provided: true
- record appeared: yes, after 1.0 s
- record keys: ["cwd", "entrypoint", "kind", "messagingSocketPath", "name", "nameSince", "nameSource", "peerFeatures", "peerProtocol", "pid", "pidDomain", "procStart", "sessionId", "startedAt", "status", "statusUpdatedAt", "updatedAt", "version"]
- kind: interactive
- status: idle
- entrypoint: cli
- procStart: 11534
- procStartFt: <absent>
- status reached idle: yes, after 0.0 s

## P1 (repeated without the token): Does an interactive session start unattended with pre-seeded onboarding state, and write a session record?

- Claude Code version: 2.1.296 (Claude Code)
- token provided: false
- record appeared: yes, after 0.5 s
- record keys: ["cwd", "entrypoint", "kind", "messagingSocketPath", "name", "nameSince", "nameSource", "peerFeatures", "peerProtocol", "pid", "pidDomain", "procStart", "sessionId", "startedAt", "status", "statusUpdatedAt", "updatedAt", "version"]
- kind: interactive
- status: idle
- entrypoint: cli
- procStart: 11698
- procStartFt: <absent>
- status reached idle: yes, after 0.0 s

## P2 (pty): What do procStart and procStartFt contain, and which OS value does each equal?

- procStart: 11789
- procStartFt: <absent>
- field 22 of /proc/<pid>/stat: 11789; equal as a string to: procStart
- psutil create_time(): 1791849339.89; equal as a string to: neither

## P7: Can the reaper read CLAUDE_CONFIG_DIR from the environment of a session started by the same user?

- underlying OS call (via psutil.Process.environ): read /proc/<pid>/environ
- variable present: true
- equals the config dir given at launch: true
- equals it after resolving both paths: true

## P3: Does SIGTERM end the session cleanly (process exits, record removed)?

- launch mode: pty
- session record: appeared after 1.0 s
- kind: interactive
- entrypoint: cli
- exited within 10 s: yes, after 0.7 s
- exit code: 143
- session record removed: true

## P5: Does a session spawned over pipes with no terminal, as the VS Code extension does, have a console, and does the stop request end it cleanly?

- launch mode: piped
- session record: appeared after 1.0 s
- kind: interactive
- entrypoint: claude-vscode
- flags passed: ["--output-format", "stream-json", "--input-format", "stream-json", "--verbose", "--include-partial-messages", "--replay-user-messages", "--enable-auth-status", "--no-chrome", "--max-thinking-tokens=31999", "--thinking-display=summarized", "--permission-prompt-tool=stdio", "--setting-sources=user,project,local"]
- environment imitated: CLAUDE_CODE_ENTRYPOINT=claude-vscode
- exited within 10 s: yes, after 0.4 s
- exit code: 143
- session record removed: true

## Added later: P3b

Added from workflow run 40354834423 (commit `7ce88063cffc96b4181253c10899cda11bf9b1f0`, Claude Code 2.1.310).

### P3b (pty): Does SIGINT end an idle session with exit code 0 and remove its record?

- Claude Code version: 2.1.310 (Claude Code)
- launch mode: pty
- session record: appeared after 1.0 s
- kind: interactive
- entrypoint: cli
- exited within 10 s: yes, after 0.7 s
- exit code: 0
- session record removed: true

### P3b (pty, unsent text): Does SIGINT end an idle session with exit code 0 and remove its record?

- Claude Code version: 2.1.310 (Claude Code)
- launch mode: pty
- session record: appeared after 1.0 s
- kind: interactive
- entrypoint: cli
- unsent text typed: yes
- unsent text shown in the prompt: true
- exited within 10 s: yes, after 0.7 s
- exit code: 0
- session record removed: true

### P3b (piped): Does SIGINT end an idle session with exit code 0 and remove its record?

- Claude Code version: 2.1.310 (Claude Code)
- launch mode: piped
- session record: appeared after 1.0 s
- kind: interactive
- entrypoint: claude-vscode
- flags passed: ["--output-format", "stream-json", "--input-format", "stream-json", "--verbose", "--include-partial-messages", "--replay-user-messages", "--enable-auth-status", "--no-chrome", "--max-thinking-tokens=31999", "--thinking-display=summarized", "--permission-prompt-tool=stdio", "--setting-sources=user,project,local"]
- environment imitated: CLAUDE_CODE_ENTRYPOINT=claude-vscode
- exited within 10 s: yes, after 0.6 s
- exit code: 0
- session record removed: true

