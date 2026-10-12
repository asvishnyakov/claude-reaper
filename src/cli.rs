use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

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
// The sweep reads these fields; it is wired in with the `run` command.
#[allow(dead_code)]
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
    let Command::Run(_run) = Arguments::parse().command;
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
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
}
