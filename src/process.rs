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
#[cfg(target_os = "linux")]
pub use linux::{identity, names_config_dir, running_config_dirs};
#[cfg(all(test, target_os = "linux"))]
pub(crate) use linux::{proc_start_of, wait_for_environment};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::identity;
#[cfg(all(test, target_os = "macos"))]
pub(crate) use macos::proc_start_of;

// macOS takes the config directories of running processes from here until it has its own.
#[cfg(not(target_os = "linux"))]
mod unverified;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use unverified::identity;
#[cfg(not(target_os = "linux"))]
pub use unverified::{names_config_dir, running_config_dirs};
