use std::io::{self, Write};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use crate::cli::{DaemonArguments, SweepArguments, sweep_once};

/// Sweeps, sleeps for the interval after the sweep ends, and does so again, forever.
pub fn run(arguments: &DaemonArguments) -> ExitCode {
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
