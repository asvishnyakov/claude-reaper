use std::io::{self, Write};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use crate::cli::{DaemonArguments, SweepArguments, sweep_once};

/// Sweeps, sleeps for the interval after the sweep ends, and does so again, forever.
pub fn run(arguments: &DaemonArguments) -> ExitCode {
    #[cfg(windows)]
    release_own_console();
    let interval = Duration::from_secs(arguments.interval_minutes.saturating_mul(60));
    loop {
        log_sweep(&arguments.sweep);
        sleep(interval);
    }
}

/// One sweep, its lines to stdout and its warnings and failure to stderr. The sweep writes to
/// buffers so that a failed write to either stream, which happens when a Windows daemon has no
/// console, can neither stop it half-way nor end the loop.
fn log_sweep(arguments: &SweepArguments) {
    let mut lines = Vec::new();
    let mut warnings = Vec::new();
    let result = sweep_once(arguments, false, &mut lines, &mut warnings);
    let _ = io::stdout().write_all(&lines);
    let _ = io::stderr().write_all(&warnings);
    if let Err(error) = result {
        let _ = writeln!(io::stderr(), "error: sweep failed: {error}");
    }
}

/// Frees a console this process is alone on (`GetConsoleProcessList` count 1), which closes the
/// window Task Scheduler opened. A console shared with a shell is kept.
#[cfg(windows)]
fn release_own_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};

    let mut processes = [0_u32; 1];
    // SAFETY: the pointer is valid for the one element whose count is passed. When more processes
    // share the console the call writes nothing and returns how many there are; 0 means this
    // process has no console. `FreeConsole` takes no argument.
    unsafe {
        if GetConsoleProcessList(processes.as_mut_ptr(), processes.len() as u32) == 1 {
            FreeConsole();
        }
    }
}
