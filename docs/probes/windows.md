# Probe results: windows

Runner: Windows-2025Server-10.0.26100-SP0, Python 3.12.10

## P1: Does an interactive session start unattended with pre-seeded onboarding state, and write a session record?

- Claude Code version: 2.1.296 (Claude Code)
- token provided: true
- record appeared: yes, after 4.5 s
- record keys: ["cwd", "entrypoint", "kind", "messagingSocketPath", "name", "nameSince", "nameSource", "peerFeatures", "peerProtocol", "pid", "pidDomain", "procStart", "sessionId", "startedAt", "version"]
- kind: interactive
- status: <absent>
- entrypoint: cli
- procStart: 134363229627676503
- procStartFt: <absent>
- status reached idle: yes, after 0.5 s

## P1 (repeated without the token): Does an interactive session start unattended with pre-seeded onboarding state, and write a session record?

- Claude Code version: 2.1.296 (Claude Code)
- token provided: false
- record appeared: yes, after 4.5 s
- record keys: ["cwd", "entrypoint", "kind", "messagingSocketPath", "name", "nameSince", "nameSource", "peerFeatures", "peerProtocol", "pid", "pidDomain", "procStart", "sessionId", "startedAt", "status", "statusUpdatedAt", "updatedAt", "version"]
- kind: interactive
- status: idle
- entrypoint: cli
- procStart: 134363229679536913
- procStartFt: <absent>
- status reached idle: yes, after 0.0 s

## P2 (pty): What do procStart and procStartFt contain, and which OS value does each equal?

- procStart: 134363229726002154
- procStartFt: <absent>
- GetProcessTimes creation FILETIME: 134363229726002154; equal as a string to: procStart
- Win32_Process.CreationDate: 2026-10-12T23:56:12.6002150+00:00; equal as a string to: neither
- psutil create_time(): 1791849372.6002154; equal as a string to: neither

## P2 (console): What do procStart and procStartFt contain, and which OS value does each equal?

- procStart: 134363229820990655
- procStartFt: <absent>
- GetProcessTimes creation FILETIME: 134363229820990655; equal as a string to: procStart
- Win32_Process.CreationDate: 2026-10-12T23:56:22.0990650+00:00; equal as a string to: neither
- psutil create_time(): 1791849382.0990655; equal as a string to: neither

## P7: Can the reaper read CLAUDE_CONFIG_DIR from the environment of a session started by the same user?

- underlying OS call (via psutil.Process.environ): reading the process environment block with ReadProcessMemory
- variable present: true
- equals the config dir given at launch: true
- equals it after resolving both paths: true

## P4 (console): Does the Ctrl+C sequence (FreeConsole, AttachConsole, SetConsoleCtrlHandler, GenerateConsoleCtrlEvent) end the session cleanly?

- launch mode: console
- session record: appeared after 1.5 s
- kind: interactive
- entrypoint: cli
- ctrl_c.py exit code: 0
- exited within 10 s: yes, after 1.0 s
- exit code: 0
- session record removed: true
- other processes on its console: 0 before the request
- of those, still alive afterwards: none

## P4 (pty): Does the Ctrl+C sequence (FreeConsole, AttachConsole, SetConsoleCtrlHandler, GenerateConsoleCtrlEvent) end the session cleanly?

- launch mode: pty
- session record: appeared after 4.5 s
- kind: interactive
- entrypoint: cli
- ctrl_c.py exit code: 0
- exited within 10 s: yes, after 1.0 s
- exit code: 0
- session record removed: true
- other processes on its console: 0 before the request
- of those, still alive afterwards: none

## P5: Does a session spawned over pipes with no terminal, as the VS Code extension does, have a console, and does the stop request end it cleanly?

- launch mode: piped
- session record: appeared after 1.5 s
- kind: interactive
- entrypoint: claude-vscode
- flags passed: ["--output-format", "stream-json", "--input-format", "stream-json", "--verbose", "--include-partial-messages", "--replay-user-messages", "--enable-auth-status", "--no-chrome", "--max-thinking-tokens=31999", "--thinking-display=summarized", "--permission-prompt-tool=stdio", "--setting-sources=user,project,local"]
- environment imitated: CLAUDE_CODE_ENTRYPOINT=claude-vscode
- ctrl_c.py exit code: 0
- exited within 10 s: yes, after 0.6 s
- exit code: 0
- session record removed: true
- other processes on its console: 0 before the request
- of those, still alive afterwards: none

## P6: Can a daemon started by the service manager see and stop a session in the user's login session?

- launch mode: console
- service manager: Task Scheduler task, trigger at logon, interactive only (/it)
- caveat: a logon cannot be triggered in CI; schtasks /run stands in for the logon trigger
- command file: a .cmd wrapper holding the --service-check command, because /tr is limited to 261 characters
- schtasks /create: exit code 0: SUCCESS: The scheduled task "claude-reaper-probe" has successfully been created.
- schtasks /run: exit code 0: SUCCESS: Attempted to run the scheduled task "claude-reaper-probe".
- schtasks /query: Status:                               Ready; Logon Mode:                           Interactive only; Last Result:                          0
- schtasks /delete: exit code 0: SUCCESS: The scheduled task "claude-reaper-probe" was successfully deleted.
- service-check output:

```text
- session record readable: yes
- target process visible: true
- ctrl_c.py exit code: 0
- exited within 10 s: yes, after 0.9 s
- exit code: <not observable: not a child process>
- session record removed: true
- other processes on its console: 0 before the request
- of those, still alive afterwards: none
```
- session exit code seen by the probe afterwards: 0

