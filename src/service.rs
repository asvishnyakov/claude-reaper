#[cfg(target_os = "linux")]
use std::io;

/// What the service manager runs: `program` with `arguments`.
#[derive(Debug, PartialEq, Eq)]
pub struct Registration {
    pub program: String,
    pub arguments: Vec<String>,
}

// Each OS implementation constructs only some of these variants; the unsupported one, none.
#[allow(dead_code)]
pub enum Uninstalled {
    /// What was removed.
    Removed(String),
    NotInstalled,
}

#[cfg(target_os = "linux")]
mod systemd;
#[cfg(target_os = "linux")]
pub use systemd::{install, uninstall};

#[cfg(not(target_os = "linux"))]
mod unsupported;
#[cfg(not(target_os = "linux"))]
pub use unsupported::{install, uninstall};

/// Runs a service-manager command; Err naming the command, its exit status and stderr when it fails.
#[cfg(target_os = "linux")]
pub(crate) fn run_tool(program: &str, arguments: &[&str]) -> io::Result<()> {
    use std::process::Command;

    let command = std::iter::once(program)
        .chain(arguments.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("cannot run {command}: {error}")))?;
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

#[cfg(all(test, target_os = "linux"))]
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
}
