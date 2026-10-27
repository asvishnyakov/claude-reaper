#[derive(Debug, PartialEq, Eq)]
// Each OS implementation constructs only some of these variants.
#[allow(dead_code)]
pub enum Identity {
    Same,
    Different,
    Gone,
    Unverified,
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(test, target_os = "linux"))]
pub(crate) use linux::proc_start_of;
#[cfg(all(test, target_os = "linux"))]
use linux::wait_for_environment;
#[cfg(target_os = "linux")]
pub use linux::{identity, names_config_dir, running_config_dirs};

#[cfg(any(target_os = "macos", windows))]
mod environment;
#[cfg(all(test, any(target_os = "macos", windows)))]
use environment::wait_for_environment;
#[cfg(any(target_os = "macos", windows))]
pub use environment::{names_config_dir, running_config_dirs};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::identity;
#[cfg(all(test, target_os = "macos"))]
pub(crate) use macos::proc_start_of;

// Windows takes process identity from here until it has its own; any other OS takes everything.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod unverified;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use unverified::identity;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub use unverified::{names_config_dir, running_config_dirs};

/// A sleeping child (`sleep 30`; on Windows `ping -n 30 127.0.0.1` with output discarded) with
/// `CLAUDE_CONFIG_DIR` removed or set; returns once its environment is readable.
#[cfg(test)]
pub(crate) fn spawn_with_config_dir(config_dir: Option<&std::path::Path>) -> std::process::Child {
    use std::process::Command;

    #[cfg(windows)]
    let mut command = {
        use std::process::Stdio;

        let mut command = Command::new("ping");
        command
            .args(["-n", "30", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("sleep");
        command.arg("30");
        command
    };
    command.env_remove("CLAUDE_CONFIG_DIR");
    if let Some(config_dir) = config_dir {
        command.env("CLAUDE_CONFIG_DIR", config_dir);
    }
    let child = command.spawn().unwrap();
    wait_for_environment(child.id());
    child
}
