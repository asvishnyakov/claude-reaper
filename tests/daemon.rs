//! The `daemon` command, run as a child process.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

const PID: u32 = 4_000_000;

/// A record of an idle interactive `claude-vscode` session that has been idle for 20 minutes, in
/// `config_dir`. No process has this PID.
fn write_record(config_dir: &Path) {
    let idle_since = chrono::Local::now().timestamp_millis() - 20 * 60_000;
    let record = serde_json::json!({
        "pid": PID,
        "sessionId": "s1",
        "name": "fixture",
        "kind": "interactive",
        "entrypoint": "claude-vscode",
        "status": "idle",
        "statusUpdatedAt": idle_since,
    });
    let path = config_dir.join(format!("sessions/{PID}.json"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, record.to_string()).unwrap();
}

/// The daemon on `config_dir` and the fixture's entrypoint only, with `home` as the home directory
/// so that no real session is ever selected. The caller sets the standard streams.
fn daemon_command(home: &Path, config_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_claude-reaper"));
    command
        .arg("daemon")
        .arg("--config-dir")
        .arg(config_dir)
        .args(["--entrypoint", "reaper-test-only"])
        .env("HOME", home)
        .env("USERPROFILE", home);
    command
}

/// The lines of `source`, sent as they arrive until it closes.
fn read_lines(source: impl Read + Send + 'static) -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(source).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

/// The lines received until `expected` arrives, the sender closes or `timeout` passes. The last
/// one is `expected` if it arrived.
fn lines_until(lines: &Receiver<String>, expected: &str, timeout: Duration) -> Vec<String> {
    let deadline = Instant::now() + timeout;
    let mut received = Vec::new();
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match lines.recv_timeout(remaining) {
            Ok(line) => {
                let arrived = line == expected;
                received.push(line);
                if arrived {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
        }
    }
    received
}

#[test]
fn daemon_sweeps_at_start() {
    let home = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    write_record(config_dir.path());
    let mut daemon = daemon_command(home.path(), config_dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let lines = read_lines(daemon.stdout.take().unwrap());
    let expected = format!("skip {PID} (fixture): entrypoint claude-vscode not selected");

    let received = lines_until(&lines, &expected, Duration::from_secs(10));

    daemon.kill().unwrap();
    daemon.wait().unwrap();
    assert_eq!(received.last(), Some(&expected), "{received:?}");
}

/// A daemon alone on its console, as Task Scheduler starts it, frees the console so that its window
/// closes: no console is left for the Ctrl+C helper to attach to.
#[cfg(windows)]
#[test]
fn daemon_alone_on_its_console_detaches_from_it() {
    use std::os::windows::process::CommandExt;
    use std::thread::sleep;

    use windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE;

    let home = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    write_record(config_dir.path());
    let mut daemon = daemon_command(home.path(), config_dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .unwrap();
    sleep(Duration::from_secs(2));

    let helper = Command::new(env!("CARGO_BIN_EXE_claude-reaper"))
        .args(["ctrl-c", &daemon.id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();

    // Exit code 2 also means that the process is gone, so the daemon must still be running.
    let running = daemon.try_wait().unwrap().is_none();
    if running {
        daemon.kill().unwrap();
    }
    daemon.wait().unwrap();
    assert!(running, "the daemon exited");
    assert_eq!(helper.code(), Some(2), "{helper}");
}
