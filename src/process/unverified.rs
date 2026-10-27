use super::Identity;

/// What `procStart` means on this system has not been observed yet, so identity cannot be confirmed.
pub fn identity(_pid: u32, _proc_start: Option<&str>) -> Identity {
    Identity::Unverified
}

/// Another process's environment is not read here, so nothing is ever confirmed.
#[cfg(not(windows))]
pub fn names_config_dir(_pid: u32, _config_dir: &std::path::Path) -> bool {
    false
}

/// Only this process's own `CLAUDE_CONFIG_DIR`: reading other processes' environments is not verified here.
#[cfg(not(windows))]
pub fn running_config_dirs() -> Vec<std::path::PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .into_iter()
        .collect()
}
