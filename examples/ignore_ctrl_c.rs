//! The test child for tests/ctrl_c.rs that ignores Ctrl+C: `GenerateConsoleCtrlEvent` skips the
//! handlers of a process that has called `SetConsoleCtrlHandler(NULL, TRUE)`, as the helper does.

#[cfg(windows)]
fn main() {
    use std::io::Write;
    use std::thread::sleep;
    use std::time::Duration;

    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

    // SAFETY: no pointer is passed; the handler is null.
    let ignoring = unsafe { SetConsoleCtrlHandler(None, 1) } != 0;
    assert!(ignoring, "SetConsoleCtrlHandler failed");
    let mut stdout = std::io::stdout();
    writeln!(stdout, "ready").unwrap();
    stdout.flush().unwrap();
    sleep(Duration::from_secs(60));
}

#[cfg(not(windows))]
fn main() {}
