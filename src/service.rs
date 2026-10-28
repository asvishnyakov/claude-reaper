#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use std::io;

/// What the service manager runs: `program` with `arguments`.
#[derive(Debug, PartialEq, Eq)]
pub struct Registration {
    pub program: String,
    pub arguments: Vec<String>,
}

// The systemd, launchd and Task Scheduler implementations construct these variants; the unsupported
// one (any other OS) constructs none.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos", windows)),
    allow(dead_code)
)]
pub enum Uninstalled {
    /// What was removed.
    Removed(String),
    NotInstalled,
}

#[cfg(target_os = "linux")]
mod systemd;
#[cfg(target_os = "linux")]
pub use systemd::{install, uninstall};

#[cfg(target_os = "macos")]
mod launchd;
#[cfg(target_os = "macos")]
pub use launchd::{install, uninstall};

#[cfg(windows)]
mod task_scheduler;
#[cfg(windows)]
pub use task_scheduler::{install, uninstall};

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub use unsupported::{install, uninstall};

/// Runs a service-manager command; Err naming the command, its exit status and stderr when it fails.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) fn run_tool(program: &str, arguments: &[&str]) -> io::Result<()> {
    use std::process::Command;

    let command = command_line(program, arguments);
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| cannot_run(&command, error))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    let detail = if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    };
    Err(io::Error::other(format!(
        "{command} failed ({status}){detail}",
        status = output.status
    )))
}

/// Whether a service-manager command exits with success, for probing state; Err only when it
/// cannot run, naming the command.
#[cfg(any(target_os = "macos", windows))]
pub(crate) fn tool_succeeds(program: &str, arguments: &[&str]) -> io::Result<bool> {
    use std::process::{Command, Stdio};

    Command::new(program)
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .map_err(|error| cannot_run(&command_line(program, arguments), error))
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn command_line(program: &str, arguments: &[&str]) -> String {
    std::iter::once(program)
        .chain(arguments.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn cannot_run(command: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("cannot run {command}: {error}"))
}

/// `path` as the text of a service-manager command line; InvalidInput when it is not valid UTF-8.
#[cfg(any(target_os = "macos", windows))]
fn utf8(path: &std::path::Path) -> io::Result<&str> {
    path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path is not valid UTF-8: {}", path.display()),
        )
    })
}

/// `text` as XML character data.
#[cfg(any(target_os = "macos", windows))]
fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn failing_tool_error_names_the_command_status_and_stderr() {
        let error = run_tool("sh", &["-c", "echo boom >&2; exit 3"]).unwrap_err();

        assert_eq!(
            error.to_string(),
            "sh -c echo boom >&2; exit 3 failed (exit status: 3): boom"
        );
    }

    #[test]
    fn missing_tool_error_names_the_command() {
        let error = run_tool("claude-reaper-no-such-tool", &["--user"]).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(
            error
                .to_string()
                .starts_with("cannot run claude-reaper-no-such-tool --user: "),
            "{error}"
        );
    }

    #[test]
    fn succeeding_tool_is_ok() {
        run_tool("sh", &["-c", "exit 0"]).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tool_succeeds_reports_the_exit_status_without_failing() {
        assert!(tool_succeeds("sh", &["-c", "exit 0"]).unwrap());
        assert!(!tool_succeeds("sh", &["-c", "exit 3"]).unwrap());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tool_succeeds_names_the_command_it_cannot_run() {
        let error = tool_succeeds("claude-reaper-no-such-tool", &["print"]).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(
            error
                .to_string()
                .starts_with("cannot run claude-reaper-no-such-tool print: "),
            "{error}"
        );
    }
}
