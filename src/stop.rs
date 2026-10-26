use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
// Each OS implementation constructs only some of these variants.
#[allow(dead_code)]
pub enum StopOutcome {
    Stopped,
    DidNotExit,
    Unsupported,
}

pub const GRACE: Duration = Duration::from_secs(10);

/// Polls `identity` every 100 ms until it is `Gone` or `Different` (`Stopped`), or `grace` passes
/// (`DidNotExit`). `Unverified` counts as still running: on macOS a record without `procStart`
/// leaves the process unverified for as long as it lives.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn wait_for_exit(pid: u32, proc_start: Option<&str>, grace: Duration) -> StopOutcome {
    use std::thread::sleep;
    use std::time::Instant;

    use crate::process::{Identity, identity};

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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use unsupported::stop;
