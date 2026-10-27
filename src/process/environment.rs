use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::sessions::canonical;

/// The processes asked for, with their environments and owners read. A process whose environment
/// cannot be read (access denied) has an empty one.
fn read_processes(processes: ProcessesToUpdate<'_>) -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        processes,
        true,
        ProcessRefreshKind::nothing()
            .with_environ(UpdateKind::Always)
            .with_user(UpdateKind::Always),
    );
    system
}

/// Whether `process` runs as the same user as this process, which `system` must have read as well.
fn same_user_as_this_process(system: &System, process: &Process) -> Option<bool> {
    let this_process = system.process(sysinfo::get_current_pid().ok()?)?;
    Some(process.user_id()? == this_process.user_id()?)
}

/// Distinct absolute `CLAUDE_CONFIG_DIR` values of this user's running processes, this one included.
pub fn running_config_dirs() -> Vec<PathBuf> {
    let system = read_processes(ProcessesToUpdate::All);
    system
        .processes()
        .values()
        .filter(|process| same_user_as_this_process(&system, process) == Some(true))
        .filter_map(|process| config_dir_in(process.environ()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// True when the process's own environment names `config_dir` as its `CLAUDE_CONFIG_DIR`, however
/// either is spelled. A session that Claude Code started under that directory always does.
pub fn names_config_dir(pid: u32, config_dir: &Path) -> bool {
    let pid = Pid::from_u32(pid);
    read_processes(ProcessesToUpdate::Some(&[pid]))
        .process(pid)
        .and_then(|process| config_dir_in(process.environ()))
        .is_some_and(|named| canonical(&named) == canonical(config_dir))
}

/// `Some(true)` when `pid` runs as the same user as this process; `None` when either owner cannot be read.
// Not called outside tests yet: process identity on Windows is to use it, while macOS reads the owner from `ps`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn owned_by_current_user(pid: u32) -> Option<bool> {
    let pid = Pid::from_u32(pid);
    let this_pid = sysinfo::get_current_pid().ok()?;
    // A pid listed twice is dropped from the refresh as if it had exited.
    let pids: &[Pid] = if pid == this_pid {
        &[pid]
    } else {
        &[pid, this_pid]
    };
    let system = read_processes(ProcessesToUpdate::Some(pids));
    same_user_as_this_process(&system, system.process(pid)?)
}

/// The absolute `CLAUDE_CONFIG_DIR` among `KEY=VALUE` entries, if there is one; the name matches
/// ignoring ASCII case on Windows. Entries that are not valid Unicode are skipped.
fn config_dir_in(environ: &[OsString]) -> Option<PathBuf> {
    const NAME: &str = "CLAUDE_CONFIG_DIR";
    environ
        .iter()
        .filter_map(|variable| variable.to_str()?.split_once('='))
        .find(|(name, _)| {
            if cfg!(windows) {
                name.eq_ignore_ascii_case(NAME)
            } else {
                *name == NAME
            }
        })
        .map(|(_, value)| PathBuf::from(value))
        .filter(|path| path.is_absolute())
}

/// Waits until a freshly spawned process has an environment to read, which it can lack for a moment.
#[cfg(test)]
pub(crate) fn wait_for_environment(pid: u32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let pid = Pid::from_u32(pid);
    while !read_processes(ProcessesToUpdate::Some(&[pid]))
        .process(pid)
        .is_some_and(|process| !process.environ().is_empty())
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the environment of {pid} never became visible"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::spawn_with_config_dir;

    fn names_config_dir_for(started_with: Option<&Path>, asked_about: &Path) -> bool {
        let mut child = spawn_with_config_dir(started_with);
        let names = names_config_dir(child.id(), asked_about);
        child.kill().unwrap();
        child.wait().unwrap();
        names
    }

    #[test]
    fn finds_config_dir_of_a_running_process() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = spawn_with_config_dir(Some(dir.path()));

        let found = running_config_dirs();

        child.kill().unwrap();
        child.wait().unwrap();
        assert!(found.contains(&dir.path().to_path_buf()), "{found:?}");
    }

    #[test]
    fn ignores_relative_and_empty_config_dir_values() {
        let mut relative = spawn_with_config_dir(Some(Path::new("relative/unique-marker")));
        let mut empty = spawn_with_config_dir(Some(Path::new("")));

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

    #[test]
    fn process_names_the_config_dir_it_was_started_with() {
        let dir = tempfile::tempdir().unwrap();

        assert!(names_config_dir_for(Some(dir.path()), dir.path()));
    }

    #[test]
    fn process_does_not_name_a_different_config_dir() {
        let started_with = tempfile::tempdir().unwrap();
        let asked_about = tempfile::tempdir().unwrap();

        assert!(!names_config_dir_for(
            Some(started_with.path()),
            asked_about.path()
        ));
    }

    #[test]
    fn process_without_the_variable_names_no_config_dir() {
        let dir = tempfile::tempdir().unwrap();

        assert!(!names_config_dir_for(None, dir.path()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn another_spelling_of_the_same_directory_counts() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path(), &link).unwrap();

        assert!(names_config_dir_for(Some(&link), dir.path()));
    }

    #[test]
    fn own_process_is_owned_by_current_user() {
        assert_eq!(owned_by_current_user(std::process::id()), Some(true));
    }

    #[cfg(windows)]
    #[test]
    fn variable_name_case_is_ignored_on_windows() {
        let found = config_dir_in(&["claude_config_dir=C:\\profile".into()]);

        assert_eq!(found, Some(PathBuf::from("C:\\profile")));
    }
}
