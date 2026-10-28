use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use super::{Registration, Uninstalled, run_tool, tool_succeeds};

pub const LABEL: &str = "io.github.asvishnyakov.claude-reaper";

const LOG_NAME: &str = "claude-reaper.log";
/// `launchctl bootout` returns before the job is gone, and `bootstrap` fails while it still is.
const UNLOAD_TIMEOUT: Duration = Duration::from_secs(10);
const UNLOAD_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Writes the agent's property list, replaces any loaded copy of it and has launchd load it in the
/// user's GUI domain. Returns what was registered.
pub fn install(registration: &Registration) -> io::Result<String> {
    let home = home_directory()?;
    let agents = home.join("Library/LaunchAgents");
    let logs = home.join("Library/Logs");
    fs::create_dir_all(&agents)?;
    fs::create_dir_all(&logs)?;
    let path = agents.join(format!("{LABEL}.plist"));
    fs::write(
        &path,
        property_list(registration, utf8(&logs.join(LOG_NAME))?),
    )?;
    let domain = domain();
    let target = format!("{domain}/{LABEL}");
    // Fails when the agent is not loaded, which is the common case.
    let _ = launchctl(&["bootout", &target]);
    wait_until_unloaded(&target)?;
    launchctl(&["bootstrap", &domain, utf8(&path)?])?;
    Ok(describe(&path))
}

/// Unloads the agent, then removes its property list.
pub fn uninstall() -> io::Result<Uninstalled> {
    let path = home_directory()?
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"));
    if !path.try_exists()? {
        return Ok(Uninstalled::NotInstalled);
    }
    // Fails when the agent is not loaded, which must not keep its file from being removed.
    let _ = launchctl(&["bootout", &format!("{}/{LABEL}", domain())]);
    fs::remove_file(&path)?;
    Ok(Uninstalled::Removed(describe(&path)))
}

fn describe(path: &Path) -> String {
    format!("launchd user agent {}", path.display())
}

fn launchctl(arguments: &[&str]) -> io::Result<()> {
    run_tool("launchctl", arguments)
}

/// Waits for `launchctl print` to stop finding `target`, up to `UNLOAD_TIMEOUT`; a copy that is
/// still loaded then makes the following `bootstrap` fail with launchd's own message.
fn wait_until_unloaded(target: &str) -> io::Result<()> {
    let deadline = Instant::now() + UNLOAD_TIMEOUT;
    while tool_succeeds("launchctl", &["print", target])? && Instant::now() < deadline {
        thread::sleep(UNLOAD_POLL_INTERVAL);
    }
    Ok(())
}

/// The launchd domain of the user's login session.
fn domain() -> String {
    format!("gui/{}", rustix::process::getuid().as_raw())
}

fn home_directory() -> io::Result<PathBuf> {
    std::env::home_dir().ok_or_else(|| io::Error::other("cannot determine the home directory"))
}

fn utf8(path: &Path) -> io::Result<&str> {
    path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path is not valid UTF-8: {}", path.display()),
        )
    })
}

/// The property list of an agent that runs `registration` when loaded and again after it fails,
/// with its output in `log`. Rendered from `launchd.plist`.
pub fn property_list(registration: &Registration, log: &str) -> String {
    let program_arguments = std::iter::once(&registration.program)
        .chain(&registration.arguments)
        .map(|word| format!("        <string>{}</string>", escape(word)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        include_str!("launchd.plist"),
        label = LABEL,
        program_arguments = program_arguments,
        log = escape(log),
    )
}

/// `text` as XML character data.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(program: &str, arguments: &[&str]) -> Registration {
        Registration {
            program: program.to_string(),
            arguments: arguments
                .iter()
                .map(|argument| argument.to_string())
                .collect(),
        }
    }

    #[test]
    fn property_list_runs_the_daemon_at_load_and_after_a_failure() {
        let text = property_list(
            &registration("/opt/claude-reaper", &["daemon"]),
            "/Users/u/Library/Logs/claude-reaper.log",
        );

        assert_eq!(
            text,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>io.github.asvishnyakov.claude-reaper</string>
    <key>ProgramArguments</key>
    <array>
        <string>/opt/claude-reaper</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>/Users/u/Library/Logs/claude-reaper.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/u/Library/Logs/claude-reaper.log</string>
</dict>
</plist>
"#
        );
    }

    #[test]
    fn property_list_escapes_xml_characters() {
        let text = property_list(
            &registration("/opt/claude-reaper", &["a&b<c>"]),
            "/Users/u/Library/Logs/claude-reaper.log",
        );

        assert!(text.contains("<string>a&amp;b&lt;c&gt;</string>"), "{text}");
    }
}
