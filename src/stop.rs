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

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::stop;

#[cfg(not(target_os = "linux"))]
mod unsupported;
#[cfg(not(target_os = "linux"))]
pub use unsupported::stop;
