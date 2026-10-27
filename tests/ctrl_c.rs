//! The hidden `ctrl-c <pid>` helper, aimed at children started in different kinds of console.
#![cfg(windows)]

use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::Win32::System::Threading::{CREATE_NEW_CONSOLE, DETACHED_PROCESS};

/// `ping -n 60 127.0.0.1` created with `creation_flags`, its output discarded.
fn spawn_ping(creation_flags: u32) -> Child {
    Command::new("ping")
        .args(["-n", "60", "127.0.0.1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(creation_flags)
        .spawn()
        .unwrap()
}

/// The `ignore_ctrl_c` example in a console of its own, once it reports that it ignores Ctrl+C.
fn spawn_ignoring_ctrl_c() -> Child {
    let program = Path::new(env!("CARGO_BIN_EXE_claude-reaper"))
        .with_file_name("examples")
        .join("ignore_ctrl_c.exe");
    assert!(
        program.is_file(),
        "{} is missing; `cargo test` builds it",
        program.display()
    );
    let mut child = Command::new(&program)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .unwrap();
    let mut ready = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim_end(), "ready");
    child
}

/// The exit code and stderr of the reaper's helper sending Ctrl+C to `pid`, run as `stop` runs it
/// except that stderr is captured.
fn ctrl_c(pid: u32) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_claude-reaper"))
        .args(["ctrl-c", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Every attempt of the helper on `pid`, retried every 100 ms for up to 5 s while it exits 2: a
/// console created with the child may not exist yet when `spawn` returns. The last is the result.
fn ctrl_c_once_the_console_exists(pid: u32) -> Vec<(Option<i32>, String)> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut attempts = vec![ctrl_c(pid)];
    while attempts.last().unwrap().0 == Some(2) && Instant::now() < deadline {
        sleep(Duration::from_millis(100));
        attempts.push(ctrl_c(pid));
    }
    attempts
}

/// Whether `child` exits before `timeout` passes.
fn exits_within(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        sleep(Duration::from_millis(50));
    }
}

/// Ends a child the test spawned, whether or not it is still running.
fn clean_up(mut child: Child) {
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    child.wait().unwrap();
}

#[test]
fn ctrl_c_ends_a_process_in_its_own_console() {
    let mut child = spawn_ping(CREATE_NEW_CONSOLE);

    let attempts = ctrl_c_once_the_console_exists(child.id());

    let exited = exits_within(&mut child, Duration::from_secs(10));
    clean_up(child);
    assert_eq!(
        attempts.last().unwrap().0,
        Some(0),
        "{} attempts: {attempts:?}",
        attempts.len()
    );
    assert!(exited, "{} attempts: {attempts:?}", attempts.len());
}

#[test]
fn process_without_a_console_is_reported() {
    let mut child = spawn_ping(DETACHED_PROCESS);

    let (code, stderr) = ctrl_c(child.id());

    let exited = exits_within(&mut child, Duration::from_secs(1));
    clean_up(child);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(!exited, "{stderr}");
}

#[test]
fn process_ignoring_ctrl_c_keeps_running() {
    // `CREATE_NEW_PROCESS_GROUP`, which would disable Ctrl+C, is ignored with `CREATE_NEW_CONSOLE`.
    let mut child = spawn_ignoring_ctrl_c();

    let attempts = ctrl_c_once_the_console_exists(child.id());

    let exited = exits_within(&mut child, Duration::from_secs(2));
    clean_up(child);
    assert_eq!(
        attempts.last().unwrap().0,
        Some(0),
        "{} attempts: {attempts:?}",
        attempts.len()
    );
    assert!(!exited, "{} attempts: {attempts:?}", attempts.len());
}
