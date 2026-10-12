use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::{DateTime, Local};
use clap::{Args, Parser, Subcommand};

use crate::process::{self, Identity};
use crate::reaper::{Decision, decide};
use crate::sessions::{SessionRecord, read_records, session_config_dirs};
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
    let result = sweep(
        &dirs,
        Local::now(),
        run.idle_minutes,
        run.dry_run,
        &mut io::stdout(),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The config directories to sweep: `~/.claude`, those of running processes, then those given with
/// `--config-dir`; each only if it holds `sessions/`, and once.
pub fn config_dirs(
    home: &Path,
    discovered: Vec<PathBuf>,
    flagged: &[PathBuf],
    warnings: &mut impl Write,
) -> Vec<PathBuf> {
    let mut candidates = vec![home.join(".claude")];
    candidates.extend(discovered);
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
    session_config_dirs(candidates)
}

/// Stops (or, with `dry_run`, reports) every session of `config_dirs` that `decide` selects,
/// one line per session.
pub fn sweep(
    config_dirs: &[PathBuf],
    now: DateTime<Local>,
    idle_minutes: u64,
    dry_run: bool,
    output: &mut impl Write,
) -> io::Result<()> {
    for config_dir in config_dirs {
        let records = read_records(config_dir);
        for decision in decide(&records, now, idle_minutes) {
            match decision {
                Decision::Skip { record, reason } => {
                    writeln!(output, "skip {} ({}): {reason}", record.pid, name(record))?;
                }
                Decision::Stop {
                    record,
                    idle_minutes,
                } => stop_session(record, idle_minutes, dry_run, output)?,
            }
        }
    }
    Ok(())
}

fn stop_session(
    record: &SessionRecord,
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

    const SCHEDULE_WAKEUP_CALL: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"ScheduleWakeup","input":{}}]}}"#;

    /// A config directory holding one record for this test process: interactive, idle 20 minutes.
    fn fixture(name: Option<&str>, proc_start: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut record = serde_json::json!({
            "pid": std::process::id(),
            "sessionId": "s1",
            "kind": "interactive",
            "status": "idle",
            "statusUpdatedAt": Local::now().timestamp_millis() - 20 * 60_000,
        });
        if let Some(name) = name {
            record["name"] = serde_json::json!(name);
        }
        if let Some(proc_start) = proc_start {
            record["procStart"] = serde_json::json!(proc_start);
        }
        let path = dir
            .path()
            .join(format!("sessions/{}.json", std::process::id()));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, record.to_string()).unwrap();
        dir
    }

    fn dry_run_output(config_dir: &Path) -> String {
        let mut output = Vec::new();
        sweep(
            &[config_dir.to_path_buf()],
            Local::now(),
            15,
            true,
            &mut output,
        )
        .unwrap();
        String::from_utf8(output).unwrap()
    }

    fn dir_name(dir: &Path) -> String {
        dir.file_name().unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn home_claude_directory_is_used() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".claude/sessions")).unwrap();

        let dirs = config_dirs(home.path(), vec![], &[], &mut io::sink());

        assert_eq!(dirs, [home.path().join(".claude")]);
    }

    #[test]
    fn unreferenced_profile_under_home_is_not_guessed() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".claude-other/sessions")).unwrap();

        let dirs = config_dirs(home.path(), vec![], &[], &mut io::sink());

        assert_eq!(dirs, Vec::<PathBuf>::new());
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

        assert_eq!(dirs, [profile.path().to_path_buf()]);
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

        assert_eq!(dirs, [home.path().join(".claude")]);
        assert_eq!(
            String::from_utf8(warnings).unwrap(),
            format!(
                "warning: --config-dir {} is not a directory\n",
                missing.display()
            )
        );
    }

    #[test]
    fn dry_run_reports_the_fixture_session() {
        let dir = fixture(Some("fixture"), None);
        let pid = std::process::id();

        let output = dry_run_output(dir.path());

        #[cfg(target_os = "linux")]
        let expected = format!(
            "would stop {pid} (fixture, {}, idle 20 min)",
            dir_name(dir.path())
        );
        #[cfg(not(target_os = "linux"))]
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

    #[cfg(target_os = "linux")]
    #[test]
    fn dry_run_reports_a_reused_pid() {
        let dir = fixture(Some("fixture"), Some("1"));
        let pid = std::process::id();

        let output = dry_run_output(dir.path());

        let expected = format!("skip {pid} (fixture): PID reused by another process");
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
