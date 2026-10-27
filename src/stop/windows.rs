use std::env;
use std::io;
use std::process::{Command, Stdio};
use std::time::Duration;

use windows_sys::Win32::System::Console::{
    AttachConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler,
};

use super::{StopOutcome, wait_for_exit};

/// Sends Ctrl+C through the target's console, which Claude Code answers as it does `SIGINT`, then
/// waits up to `grace` for the process to exit. Never escalates. The sequence runs in a child
/// (`<current exe> ctrl-c <pid>`) because it detaches the caller from its own console.
pub fn stop(pid: u32, proc_start: Option<&str>, grace: Duration) -> io::Result<StopOutcome> {
    let status = Command::new(env::current_exe()?)
        .args(["ctrl-c", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    match status.code() {
        Some(0) => Ok(wait_for_exit(pid, proc_start, grace)),
        Some(2) => Ok(StopOutcome::NoConsole),
        _ => Err(io::Error::other(format!(
            "the Ctrl+C helper for {pid} failed: {status}"
        ))),
    }
}

/// Generates Ctrl+C in the console of `pid`, which reaches every process attached to it, and
/// returns the exit code for `ctrl-c`: 0 sent, 2 `AttachConsole` failed (no console to attach to),
/// 3 `GenerateConsoleCtrlEvent` failed, as in probes/ctrl_c.py (P4, P5). Detaches this process
/// from its own console.
pub fn send_ctrl_c(pid: u32) -> u8 {
    // SAFETY: none of these calls takes a pointer, and the handler is null. Each one acts on this
    // process's own console attachment and owns no handle that needs closing.
    unsafe {
        // A process can be attached to only one console.
        FreeConsole();
        if AttachConsole(pid) == 0 {
            return 2;
        }
        // This process ignores its own event.
        SetConsoleCtrlHandler(None, 1);
        let generated = GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) != 0;
        FreeConsole();
        if generated { 0 } else { 3 }
    }
}
