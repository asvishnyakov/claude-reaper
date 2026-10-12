use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

use super::Identity;

/// Process state and start time (field 22, in clock ticks) from the text of `/proc/<pid>/stat`.
/// Fields are counted after the last `)`, so a command name with spaces or `)` cannot shift them.
pub(crate) fn parse_start_ticks(stat: &str) -> Option<(char, &str)> {
    let after_name = &stat[stat.rfind(')')? + 1..];
    let mut fields = after_name.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let start_ticks = fields.nth(18)?;
    Some((state, start_ticks))
}

pub fn identity(pid: u32, proc_start: Option<&str>) -> Identity {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return Identity::Gone;
    };
    match parse_start_ticks(&stat) {
        None | Some(('Z', _)) => Identity::Gone,
        Some((_, start_ticks)) => {
            // A session record can name any PID; only this user's own processes are ever ours to stop.
            let own_uid = rustix::process::getuid().as_raw();
            let is_own = fs::metadata(format!("/proc/{pid}"))
                .is_ok_and(|metadata| metadata.uid() == own_uid);
            match proc_start {
                _ if !is_own => Identity::Different,
                Some(recorded) if recorded != start_ticks => Identity::Different,
                _ => Identity::Same,
            }
        }
    }
}

/// Distinct absolute `CLAUDE_CONFIG_DIR` values of this user's running processes, this one included.
pub fn running_config_dirs() -> Vec<PathBuf> {
    let uid = rustix::process::getuid().as_raw();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut dirs = BTreeSet::new();
    for entry in entries.filter_map(Result::ok) {
        let is_process = entry.file_name().as_bytes().iter().all(u8::is_ascii_digit);
        let is_own = entry.metadata().is_ok_and(|metadata| metadata.uid() == uid);
        if !is_process || !is_own {
            continue;
        }
        if let Ok(environ) = fs::read(entry.path().join("environ"))
            && let Some(dir) = config_dir_in(&environ)
        {
            dirs.insert(dir);
        }
    }
    dirs.into_iter().collect()
}

/// The absolute `CLAUDE_CONFIG_DIR` in a NUL-separated process environment, if there is one.
fn config_dir_in(environ: &[u8]) -> Option<PathBuf> {
    environ
        .split(|&byte| byte == 0)
        .find_map(|variable| variable.strip_prefix(b"CLAUDE_CONFIG_DIR="))
        .map(|value| PathBuf::from(OsStr::from_bytes(value)))
        .filter(|path| path.is_absolute())
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn start_ticks_survive_parentheses_in_the_command_name() {
        let stat =
            "1234 (a) b) S 1 1234 1234 0 -1 4194304 1 0 0 0 0 0 0 0 20 0 1 0 2202283 1000 100";

        assert_eq!(parse_start_ticks(stat), Some(('S', "2202283")));
    }

    #[test]
    fn own_process_without_recorded_start_is_same() {
        assert_eq!(identity(std::process::id(), None), Identity::Same);
    }

    #[test]
    fn own_process_with_a_wrong_start_is_different() {
        assert_eq!(identity(std::process::id(), Some("1")), Identity::Different);
    }

    /// A running process that belongs to another user, if this system has one.
    fn pid_of_another_user() -> Option<u32> {
        let own_uid = rustix::process::getuid().as_raw();
        fs::read_dir("/proc")
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .metadata()
                    .is_ok_and(|metadata| metadata.uid() != own_uid)
            })
            .find_map(|entry| entry.file_name().to_str()?.parse().ok())
    }

    #[test]
    fn process_of_another_user_is_different() {
        let Some(pid) = pid_of_another_user() else {
            return;
        };

        assert_eq!(identity(pid, None), Identity::Different);
    }

    #[test]
    fn exited_child_is_gone() {
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();

        assert_eq!(identity(child.id(), None), Identity::Gone);
    }

    #[test]
    fn finds_config_dir_of_a_running_process() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new("sleep")
            .arg("30")
            .env("CLAUDE_CONFIG_DIR", dir.path())
            .spawn()
            .unwrap();

        let found = running_config_dirs();

        child.kill().unwrap();
        child.wait().unwrap();
        assert!(found.contains(&dir.path().to_path_buf()), "{found:?}");
    }

    #[test]
    fn ignores_relative_and_empty_config_dir_values() {
        let mut relative = Command::new("sleep")
            .arg("30")
            .env("CLAUDE_CONFIG_DIR", "relative/unique-marker")
            .spawn()
            .unwrap();
        let mut empty = Command::new("sleep")
            .arg("30")
            .env("CLAUDE_CONFIG_DIR", "")
            .spawn()
            .unwrap();

        let found = running_config_dirs();

        for child in [&mut relative, &mut empty] {
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert!(
            found
                .iter()
                .all(|path| !path.ends_with("unique-marker") && !path.as_os_str().is_empty()),
            "{found:?}"
        );
    }
}
