use std::io;
use std::thread::sleep;
use std::time::{Duration, Instant};

use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process};

use super::StopOutcome;
use crate::process::{Identity, identity};

const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Sends `SIGTERM`, which Claude Code answers with its own shutdown routine, then waits up to
/// `grace` for the process to exit. Never escalates.
pub fn stop(pid: u32, proc_start: Option<&str>, grace: Duration) -> io::Result<StopOutcome> {
    let target = i32::try_from(pid)
        .ok()
        .and_then(Pid::from_raw)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("invalid pid {pid}")))?;
    match kill_process(target, Signal::TERM) {
        Ok(()) => {}
        Err(Errno::SRCH) => return Ok(StopOutcome::Stopped),
        Err(error) => return Err(error.into()),
    }
    let deadline = Instant::now() + grace;
    while identity(pid, proc_start) == Identity::Same {
        if Instant::now() >= deadline {
            return Ok(StopOutcome::DidNotExit);
        }
        sleep(POLL_INTERVAL);
    }
    Ok(StopOutcome::Stopped)
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};

    use super::*;

    #[test]
    fn terminates_a_child_and_reports_stopped() {
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();

        let outcome = stop(child.id(), None, Duration::from_secs(5));

        assert_eq!(outcome.unwrap(), StopOutcome::Stopped);
        assert_eq!(child.wait().unwrap().signal(), Some(15));
    }

    #[test]
    fn refuses_pid_zero_instead_of_signalling_a_process_group() {
        let outcome = stop(0, None, Duration::from_millis(10));

        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn reports_did_not_exit_and_leaves_the_process_running() {
        let mut child = Command::new("sh")
            .args(["-c", r#"trap "" TERM; echo ready; exec sleep 60"#])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();

        let outcome = stop(child.id(), None, Duration::from_millis(300));

        let still_running = child.try_wait().unwrap().is_none();
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(outcome.unwrap(), StopOutcome::DidNotExit);
        assert!(still_running);
    }
}
