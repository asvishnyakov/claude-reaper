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
pub(crate) use linux::wait_for_environment;
#[cfg(target_os = "linux")]
pub use linux::{identity, names_config_dir, running_config_dirs};

#[cfg(not(target_os = "linux"))]
mod unverified;
#[cfg(not(target_os = "linux"))]
pub use unverified::{identity, names_config_dir, running_config_dirs};
