use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::{DateTime, Local};
use clap::{Args, Parser, Subcommand};

use crate::process::{self, Identity};
use crate::reaper::{DEFAULT_ENTRYPOINT, Decision, Rules, decide, still_stoppable};
use crate::sessions::{SessionRecord, canonical, read_records, session_config_dirs};
use crate::stop::{self, StopOutcome};

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Arguments {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Perform one sweep and exit
    Run(RunArguments),
}

#[derive(Debug, Args)]
pub struct RunArguments {
    /// Minutes a session must have been idle before it is stopped
    #[arg(long, default_value_t = 15)]
    pub idle_minutes: u64,
    /// Extra config directory to look in; may be repeated
    #[arg(long)]
    pub config_dir: Vec<PathBuf>,
    /// Client whose sessions may be stopped, as recorded in the session's entrypoint; may be
    /// repeated, and replaces the default
    #[arg(long, default_value = DEFAULT_ENTRYPOINT, value_parser = clap::builder::NonEmptyStringValueParser::new())]
    pub entrypoint: Vec<String>,
    /// Print what would be stopped and stop nothing
    #[arg(long)]
    pub dry_run: bool,
}

pub fn main() -> ExitCode {
    let Command::Run(run) = Arguments::parse().command;
    let Some(home) = std::env::home_dir() else {
        eprintln!("error: cannot determine the home directory");
        return ExitCode::FAILURE;
    };
    let dirs = config_dirs(
        &home,
        process::running_config_dirs(),
        &run.config_dir,
        &mut io::stderr(),
    );
    let rules = Rules {
        idle_minutes: run.idle_minutes,
        entrypoints: run.entrypoint,
    };
    let result = sweep(&dirs, Local::now(), &rules, run.dry_run, &mut io::stdout());
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Where a config directory came from, which decides how far its records are trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// `~/.claude` or a `--config-dir`: the user named it.
    Named,
    /// Found in another process's environment, which anything running as this user could have set.
    Discovered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDir {
    pub path: PathBuf,
    pub origin: Origin,
}

/// The config directories to sweep, each only if it holds `sessions/`, and once: `~/.claude` and
/// those given with `--config-dir` (named), then those of running processes that are not also named.
pub fn config_dirs(
    home: &Path,
    discovered: Vec<PathBuf>,
    flagged: &[PathBuf],
    warnings: &mut impl Write,
) -> Vec<ConfigDir> {
    let mut candidates = vec![home.join(".claude")];
    for path in flagged {
        if path.is_dir() {
            candidates.push(path.clone());
        } else {
            let _ = writeln!(
                warnings,
                "warning: --config-dir {} is not a directory",
                path.display()
            );
        }
    }
    let named = session_config_dirs(candidates);
    let named_identities: HashSet<PathBuf> = named.iter().map(|path| canonical(path)).collect();
    let discovered = session_config_dirs(discovered)
        .into_iter()
        .filter(|path| !named_identities.contains(&canonical(path)));
    let with_origin = |origin| move |path| ConfigDir { path, origin };
    named
        .into_iter()
        .map(with_origin(Origin::Named))
        .chain(discovered.map(with_origin(Origin::Discovered)))
        .collect()
}

/// Stops (or, with `dry_run`, reports) every session of `config_dirs` that `decide` selects,
/// one line per session.
pub fn sweep(
    config_dirs: &[ConfigDir],
    now: DateTime<Local>,
    rules: &Rules,
    dry_run: bool,
    output: &mut impl Write,
) -> io::Result<()> {
    for config_dir in config_dirs {
        let records = read_records(&config_dir.path);
        for decision in decide(&records, now, rules) {
            match decision {
                Decision::Skip { record, reason } => {
                    writeln!(output, "skip {} ({}): {reason}", record.pid, name(record))?;
                }
                Decision::Stop {
                    record,
                    idle_minutes: idle_for_minutes,
                } if still_stoppable(record, now, rules) => {
                    stop_session(record, config_dir.origin, idle_for_minutes, dry_run, output)?;
                }
                Decision::Stop { record, .. } => {
                    writeln!(
                        output,
                        "skip {} ({}): changed since it was checked",
                        record.pid,
                        name(record)
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn stop_session(
    record: &SessionRecord,
    origin: Origin,
    idle_minutes: i64,
    dry_run: bool,
    output: &mut impl Write,
) -> io::Result<()> {
    let pid = record.pid;
    let name = name(record);
    let proc_start = record.proc_start.as_deref();
    let skip = |reason: &str| format!("skip {pid} ({name}): {reason}");
    let dir_name = record
        .config_dir
        .file_name()
        .unwrap_or(record.config_dir.as_os_str())
        .to_string_lossy();
    let label = format!("{pid} ({name}, {dir_name}, idle {idle_minutes} min)");
    let line = match process::identity(pid, proc_start) {
        Identity::Gone => skip("process already gone"),
        Identity::Different => skip("PID reused by another process"),
        Identity::Unverified => skip("process identity unverified"),
        // A directory found through another process's environment may have been planted, with
        // records naming any PID. Only a process that itself names the directory is acted on.
        Identity::Same
            if origin == Origin::Discovered
                && !process::names_config_dir(pid, &record.config_dir) =>
        {
            skip("process does not name this config directory")
        }
        Identity::Same if dry_run => format!("would stop {label}"),
        Identity::Same => match stop::stop(pid, proc_start, stop::GRACE) {
            Ok(StopOutcome::Stopped) => format!("stopped {label}"),
            Ok(StopOutcome::DidNotExit) => skip("did not exit after graceful stop"),
            Ok(StopOutcome::Unsupported) => skip("graceful stop not verified on this OS"),
            Err(error) => {
                eprintln!("error stopping {pid} ({name}): {error}");
                return Ok(());
            }
        },
    };
    writeln!(output, "{line}")
}

fn name(record: &SessionRecord) -> &str {
    record.name.as_deref().unwrap_or("unnamed")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn run_defaults_to_fifteen_idle_minutes_and_no_dry_run() {
        let arguments = Arguments::try_parse_from(["claude-reaper", "run"]).unwrap();
        let Command::Run(run) = arguments.command;
        assert_eq!(run.idle_minutes, 15);
        assert!(!run.dry_run);
        assert!(run.config_dir.is_empty());
    }

    #[test]
    fn config_dir_is_repeatable() {
        let arguments = Arguments::try_parse_from([
            "claude-reaper",
            "run",
            "--config-dir",
            "/a",
            "--config-dir",
            "/b",
        ])
        .unwrap();
        let Command::Run(run) = arguments.command;
        assert_eq!(run.config_dir, [PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn entrypoint_defaults_to_claude_vscode() {
        let arguments = Arguments::try_parse_from(["claude-reaper", "run"]).unwrap();
        let Command::Run(run) = arguments.command;
        assert_eq!(run.entrypoint, ["claude-vscode"]);
    }

    #[test]
    fn entrypoint_values_replace_the_default() {
        let arguments = Arguments::try_parse_from([
            "claude-reaper",
            "run",
            "--entrypoint",
            "cli",
            "--entrypoint",
            "sdk-py",
        ])
        .unwrap();
        let Command::Run(run) = arguments.command;
        assert_eq!(run.entrypoint, ["cli", "sdk-py"]);
    }

    #[test]
    fn empty_entrypoint_is_rejected() {
        let parsed = Arguments::try_parse_from(["claude-reaper", "run", "--entrypoint", ""]);
        assert!(parsed.is_err());
    }

    #[test]
    fn dry_run_reports_an_unselected_entrypoint() {
        let pid = std::process::id();
        let dir = fixture_for(pid, Some("fixture"), None, "cli");

        let output = dry_run_output(dir.path());

        let expected = format!("skip {pid} (fixture): entrypoint cli not selected");
        assert!(output.contains(&expected), "{output}");
    }

    const SCHEDULE_WAKEUP_CALL: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"ScheduleWakeup","input":{}}]}}"#;

    /// A config directory holding one record for this test process: interactive, idle 20 minutes.
    fn fixture(name: Option<&str>, proc_start: Option<&str>) -> tempfile::TempDir {
        fixture_for(std::process::id(), name, proc_start, DEFAULT_ENTRYPOINT)
    }

    fn fixture_for(
        pid: u32,
        name: Option<&str>,
        proc_start: Option<&str>,
        entrypoint: &str,
    ) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut record = serde_json::json!({
            "pid": pid,
            "sessionId": "s1",
            "kind": "interactive",
            "entrypoint": entrypoint,
            "status": "idle",
            "statusUpdatedAt": Local::now().timestamp_millis() - 20 * 60_000,
        });
        if let Some(name) = name {
            record["name"] = serde_json::json!(name);
        }
        if let Some(proc_start) = proc_start {
            record["procStart"] = serde_json::json!(proc_start);
        }
        let path = dir.path().join(format!("sessions/{pid}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, record.to_string()).unwrap();
        dir
    }

    fn named(path: &Path) -> ConfigDir {
        ConfigDir {
            path: path.to_path_buf(),
            origin: Origin::Named,
        }
    }

    fn paths(dirs: &[ConfigDir]) -> Vec<PathBuf> {
        dirs.iter().map(|dir| dir.path.clone()).collect()
    }

    fn dry_run_output_for(dirs: &[ConfigDir]) -> String {
        let mut output = Vec::new();
        let rules = Rules {
            idle_minutes: 15,
            entrypoints: vec![DEFAULT_ENTRYPOINT.to_string()],
        };
        sweep(dirs, Local::now(), &rules, true, &mut output).unwrap();
        String::from_utf8(output).unwrap()
    }

    fn dry_run_output(config_dir: &Path) -> String {
        dry_run_output_for(&[named(config_dir)])
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn dir_name(dir: &Path) -> String {
        dir.file_name().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn home_claude_directory_is_used() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".claude/sessions")).unwrap();

        let dirs = config_dirs(home.path(), vec![], &[], &mut io::sink());

        assert_eq!(dirs, [named(&home.path().join(".claude"))]);
    }

    #[test]
    fn unreferenced_profile_under_home_is_not_guessed() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".claude-other/sessions")).unwrap();

        let dirs = config_dirs(home.path(), vec![], &[], &mut io::sink());

        assert!(dirs.is_empty());
    }

    #[test]
    fn discovered_and_flagged_directories_are_added_once() {
        let home = tempfile::tempdir().unwrap();
        let profile = tempfile::tempdir().unwrap();
        fs::create_dir_all(profile.path().join("sessions")).unwrap();

        let dirs = config_dirs(
            home.path(),
            vec![profile.path().to_path_buf()],
            &[profile.path().to_path_buf()],
            &mut io::sink(),
        );

        assert_eq!(dirs, [named(profile.path())]);
    }

    #[test]
    fn directory_found_only_through_discovery_is_marked_discovered() {
        let home = tempfile::tempdir().unwrap();
        let profile = tempfile::tempdir().unwrap();
        fs::create_dir_all(profile.path().join("sessions")).unwrap();

        let dirs = config_dirs(
            home.path(),
            vec![profile.path().to_path_buf()],
            &[],
            &mut io::sink(),
        );

        assert_eq!(
            dirs,
            [ConfigDir {
                path: profile.path().to_path_buf(),
                origin: Origin::Discovered
            }]
        );
    }

    #[test]
    fn flag_that_is_not_a_directory_warns_and_continues() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".claude/sessions")).unwrap();
        let missing = home.path().join("missing");
        let mut warnings = Vec::new();

        let dirs = config_dirs(
            home.path(),
            vec![],
            std::slice::from_ref(&missing),
            &mut warnings,
        );

        assert_eq!(paths(&dirs), [home.path().join(".claude")]);
        assert_eq!(
            String::from_utf8(warnings).unwrap(),
            format!(
                "warning: --config-dir {} is not a directory\n",
                missing.display()
            )
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn dry_run_reports_the_fixture_session() {
        let pid = std::process::id();
        let dir = fixture(Some("fixture"), Some(&process::proc_start_of(pid)));

        let output = dry_run_output(dir.path());

        let expected = format!(
            "would stop {pid} (fixture, {}, idle 20 min)",
            dir_name(dir.path())
        );
        assert!(output.contains(&expected), "{output}");
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn record_without_proc_start_is_unverified_off_linux() {
        let dir = fixture(Some("fixture"), None);
        let pid = std::process::id();

        let output = dry_run_output(dir.path());

        let expected = format!("skip {pid} (fixture): process identity unverified");
        assert!(output.contains(&expected), "{output}");
    }

    #[test]
    fn dry_run_reports_a_pending_schedule() {
        let dir = fixture(Some("fixture"), None);
        let transcript = dir.path().join("projects/p/s1.jsonl");
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        fs::write(transcript, SCHEDULE_WAKEUP_CALL).unwrap();
        let pid = std::process::id();

        let output = dry_run_output(dir.path());

        let expected = format!("skip {pid} (fixture): pending cron or /loop wakeup");
        assert!(output.contains(&expected), "{output}");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn dry_run_reports_a_reused_pid() {
        let dir = fixture(Some("fixture"), Some("1"));
        let pid = std::process::id();

        let output = dry_run_output(dir.path());

        let expected = format!("skip {pid} (fixture): PID reused by another process");
        assert!(output.contains(&expected), "{output}");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn discovered_directory_is_skipped_when_the_session_process_does_not_name_it() {
        let pid = std::process::id();
        let dir = fixture(Some("fixture"), Some(&process::proc_start_of(pid)));
        let discovered = ConfigDir {
            path: dir.path().to_path_buf(),
            origin: Origin::Discovered,
        };

        let output = dry_run_output_for(&[discovered]);

        let expected = format!("skip {pid} (fixture): process does not name this config directory");
        assert!(output.contains(&expected), "{output}");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn discovered_directory_is_acted_on_when_the_session_process_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = process::spawn_with_config_dir(Some(dir.path()));
        let session = fixture_for(
            child.id(),
            Some("fixture"),
            Some(&process::proc_start_of(child.id())),
            DEFAULT_ENTRYPOINT,
        );
        // The record lives in the directory the process names.
        fs::rename(session.path().join("sessions"), dir.path().join("sessions")).unwrap();
        let discovered = ConfigDir {
            path: dir.path().to_path_buf(),
            origin: Origin::Discovered,
        };

        let output = dry_run_output_for(&[discovered]);

        child.kill().unwrap();
        child.wait().unwrap();
        let expected = format!(
            "would stop {} (fixture, {}, idle 20 min)",
            child.id(),
            dir_name(dir.path())
        );
        assert!(output.contains(&expected), "{output}");
    }

    #[test]
    fn session_without_a_name_prints_unnamed() {
        let dir = fixture(None, None);
        let pid = std::process::id();

        let output = dry_run_output(dir.path());

        assert!(output.contains(&format!("{pid} (unnamed")), "{output}");
    }
}
