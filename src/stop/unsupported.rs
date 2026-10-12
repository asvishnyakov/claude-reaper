use std::io;
use std::time::Duration;

use super::StopOutcome;

/// A graceful stop has not been observed to work on this system, so none is attempted.
pub fn stop(_pid: u32, _proc_start: Option<&str>, _grace: Duration) -> io::Result<StopOutcome> {
    Ok(StopOutcome::Unsupported)
}
