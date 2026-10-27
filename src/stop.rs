#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::thread::sleep;
use std::time::Duration;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::time::Instant;

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::process::{Identity, identity};

#[derive(Debug, PartialEq, Eq)]
// Each OS implementation constructs only some of these variants.
#[allow(dead_code)]
pub enum StopOutcome {
    Stopped,
    DidNotExit,
    /// Windows: the process has no console through which to deliver Ctrl+C.
    NoConsole,
    Unsupported,
}

pub const GRACE: Duration = Duration::from_secs(10);

/// Polls `identity` every 100 ms until it is `Gone` or `Different` (`Stopped`), or `grace` passes
/// (`DidNotExit`). `Unverified` counts as still running: on macOS a record without `procStart`
/// leaves the process unverified for as long as it lives.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) fn wait_for_exit(pid: u32, proc_start: Option<&str>, grace: Duration) -> StopOutcome {
    const POLL_INTERVAL: Duration = Duration::from_millis(100);

    let deadline = Instant::now() + grace;
    loop {
        if matches!(
            identity(pid, proc_start),
            Identity::Gone | Identity::Different
        ) {
            return StopOutcome::Stopped;
        }
        if Instant::now() >= deadline {
            return StopOutcome::DidNotExit;
        }
        sleep(POLL_INTERVAL);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix::stop;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{send_ctrl_c, stop};

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub use unsupported::stop;
