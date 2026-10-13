# Probe results: macos

Runner: macOS-26.6.2-arm64-arm-64bit-Mach-O, Python 3.14.7

## P1: Does an interactive session start unattended with pre-seeded onboarding state, and write a session record?

- Claude Code version: 2.1.296 (Claude Code)
- token provided: true
- record appeared: yes, after 1.2 s
- record keys: ["cwd", "entrypoint", "kind", "messagingSocketPath", "name", "nameSince", "nameSource", "peerFeatures", "peerProtocol", "pid", "pidDomain", "procStart", "sessionId", "startedAt", "status", "statusUpdatedAt", "updatedAt", "version"]
- kind: interactive
- status: idle
- entrypoint: cli
- procStart: Mon Oct 12 23:55:43 2026
- procStartFt: <absent>
- status reached idle: yes, after 0.0 s

## P1 (repeated without the token): Does an interactive session start unattended with pre-seeded onboarding state, and write a session record?

- Claude Code version: 2.1.296 (Claude Code)
- token provided: false
- record appeared: yes, after 0.6 s
- record keys: ["cwd", "entrypoint", "kind", "messagingSocketPath", "name", "nameSince", "nameSource", "peerFeatures", "peerProtocol", "pid", "pidDomain", "procStart", "sessionId", "startedAt", "status", "statusUpdatedAt", "updatedAt", "version"]
- kind: interactive
- status: idle
- entrypoint: cli
- procStart: Mon Oct 12 23:55:45 2026
- procStartFt: <absent>
- status reached idle: yes, after 0.0 s

## P2 (pty): What do procStart and procStartFt contain, and which OS value does each equal?

- procStart: Mon Oct 12 23:55:46 2026
- procStartFt: <absent>
- ps -o lstart= with LC_ALL=C TZ=UTC: Mon Oct 12 23:55:46 2026; equal as a string to: procStart
- ps -o lstart= with the session's own environment: Mon Oct 12 23:55:46 2026; equal as a string to: procStart
- psutil create_time(): 1791849346.803968; equal as a string to: neither

## P2 (pty, TZ=Pacific/Kiritimati): What do procStart and procStartFt contain, and which OS value does each equal?

- procStart: Mon Oct 12 23:55:48 2026
- procStartFt: <absent>
- ps -o lstart= with LC_ALL=C TZ=UTC: Mon Oct 12 23:55:48 2026; equal as a string to: procStart
- ps -o lstart= with the session's own environment: Tue Oct 13 13:55:48 2026; equal as a string to: neither
- psutil create_time(): 1791849348.368393; equal as a string to: neither

## P7: Can the reaper read CLAUDE_CONFIG_DIR from the environment of a session started by the same user?

- underlying OS call (via psutil.Process.environ): sysctl KERN_PROCARGS2
- variable present: true
- equals the config dir given at launch: true
- equals it after resolving both paths: true

## P3: Does SIGTERM end the session cleanly (process exits, record removed)?

- launch mode: pty
- session record: appeared after 0.6 s
- kind: interactive
- entrypoint: cli
- exited within 10 s: yes, after 0.8 s
- exit code: 143
- session record removed: true

## P5: Does a session spawned over pipes with no terminal, as the VS Code extension does, have a console, and does the stop request end it cleanly?

- launch mode: piped
- session record: appeared after 1.1 s
- kind: interactive
- entrypoint: claude-vscode
- flags passed: ["--output-format", "stream-json", "--input-format", "stream-json", "--verbose", "--include-partial-messages", "--replay-user-messages", "--enable-auth-status", "--no-chrome", "--max-thinking-tokens=31999", "--thinking-display=summarized", "--permission-prompt-tool=stdio", "--setting-sources=user,project,local"]
- environment imitated: CLAUDE_CODE_ENTRYPOINT=claude-vscode
- exited within 10 s: yes, after 0.4 s
- exit code: 143
- session record removed: true

## P6: Can a daemon started by the service manager see and stop a session in the user's login session?

- launch mode: pty
- service manager: launchd user agent
- launchctl bootstrap gui/<uid>: exit code 0: <no output>
- launchctl bootout: exit code 0: <no output>
- service stdout and stderr: <empty>
- service-check output:

```text
- session record readable: yes
- target process visible: true
- exited within 10 s: yes, after 0.7 s
- exit code: <not observable: not a child process>
- session record removed: true
```
- session exit code seen by the probe afterwards: 143

