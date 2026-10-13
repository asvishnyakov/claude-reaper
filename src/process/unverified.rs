use std::path::{Path, PathBuf};

use super::Identity;

/// What `procStart` means on this system has not been observed yet, so identity cannot be confirmed.
pub fn identity(_pid: u32, _proc_start: Option<&str>) -> Identity {
    Identity::Unverified
}

/// Another process's environment is not read here, so nothing is ever confirmed.
pub fn names_config_dir(_pid: u32, _config_dir: &Path) -> bool {
    false
}

/// Only this process's own `CLAUDE_CONFIG_DIR`: reading other processes' environments is not verified here.
pub fn running_config_dirs() -> Vec<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .into_iter()
        .collect()
}
