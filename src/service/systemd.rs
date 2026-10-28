use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{Registration, Uninstalled, run_tool};

const UNIT_NAME: &str = "claude-reaper.service";

/// Writes the user unit, then has systemd load it, enable it and (re)start it. Returns what was
/// registered.
pub fn install(registration: &Registration) -> io::Result<String> {
    let directory = unit_directory()?;
    fs::create_dir_all(&directory)?;
    let path = directory.join(UNIT_NAME);
    fs::write(&path, unit_file(registration))?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", UNIT_NAME])?;
    systemctl(&["restart", UNIT_NAME])?;
    Ok(describe(&path))
}

/// Stops and disables the unit, then removes its file.
pub fn uninstall() -> io::Result<Uninstalled> {
    let path = unit_directory()?.join(UNIT_NAME);
    if !path.try_exists()? {
        return Ok(Uninstalled::NotInstalled);
    }
    systemctl(&["disable", "--now", UNIT_NAME])?;
    fs::remove_file(&path)?;
    systemctl(&["daemon-reload"])?;
    Ok(Uninstalled::Removed(describe(&path)))
}

fn describe(path: &Path) -> String {
    format!("systemd user unit {}", path.display())
}

fn systemctl(arguments: &[&str]) -> io::Result<()> {
    let mut with_user_scope = vec!["--user"];
    with_user_scope.extend_from_slice(arguments);
    run_tool("systemctl", &with_user_scope)
}

fn unit_directory() -> io::Result<PathBuf> {
    unit_directory_in(std::env::var_os("XDG_CONFIG_HOME"), std::env::home_dir())
}

/// `systemd/user` under `$XDG_CONFIG_HOME` when that is set to an absolute path, else under
/// `~/.config`.
fn unit_directory_in(
    xdg_config_home: Option<OsString>,
    home: Option<PathBuf>,
) -> io::Result<PathBuf> {
    let config_home = match xdg_config_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        Some(path) => path,
        None => home
            .ok_or_else(|| io::Error::other("cannot determine the home directory"))?
            .join(".config"),
    };
    Ok(config_home.join("systemd/user"))
}

/// The user unit that runs `registration` and restarts it when it fails.
pub fn unit_file(registration: &Registration) -> String {
    let command = std::iter::once(&registration.program)
        .chain(&registration.arguments)
        .map(|word| quote(word))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "[Unit]\n\
         Description=Stop idle Claude Code sessions (claude-reaper)\n\
         \n\
         [Service]\n\
         ExecStart={command}\n\
         Restart=on-failure\n\
         RestartSec=10\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

/// `word` as one double-quoted `ExecStart` word. systemd expands `%` specifiers and `$`
/// variables even inside quotes, so those are doubled, and `\` and `"` are escaped.
fn quote(word: &str) -> String {
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('"');
    for character in word.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '%' => quoted.push_str("%%"),
            '$' => quoted.push_str("$$"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(program: &str, arguments: &[&str]) -> Registration {
        Registration {
            program: program.to_string(),
            arguments: arguments
                .iter()
                .map(|argument| argument.to_string())
                .collect(),
        }
    }

    #[test]
    fn unit_file_runs_the_daemon_and_restarts_it_on_failure() {
        let unit = unit_file(&registration("/opt/claude-reaper", &["daemon"]));

        assert_eq!(
            unit,
            "[Unit]\n\
             Description=Stop idle Claude Code sessions (claude-reaper)\n\
             \n\
             [Service]\n\
             ExecStart=\"/opt/claude-reaper\" \"daemon\"\n\
             Restart=on-failure\n\
             RestartSec=10\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n"
        );
    }

    #[test]
    fn unit_file_quotes_spaces_percent_dollar_and_quotes() {
        let unit = unit_file(&registration(
            "/opt/my tools/claude-reaper",
            &[r#"/p/100%$HOME"x\y"#],
        ));

        let expected = r#"ExecStart="/opt/my tools/claude-reaper" "/p/100%%$$HOME\"x\\y""#;
        assert!(unit.lines().any(|line| line == expected), "{unit}");
    }

    #[test]
    fn unit_directory_is_under_an_absolute_xdg_config_home() {
        let config_home = std::env::temp_dir().join("config-home");

        let directory = unit_directory_in(
            Some(config_home.clone().into_os_string()),
            Some(PathBuf::from("/home/user")),
        )
        .unwrap();

        assert_eq!(directory, config_home.join("systemd/user"));
    }

    #[test]
    fn unit_directory_ignores_an_xdg_config_home_that_is_empty_or_relative() {
        for xdg_config_home in ["", "relative/config"] {
            let directory = unit_directory_in(
                Some(OsString::from(xdg_config_home)),
                Some(PathBuf::from("/home/user")),
            )
            .unwrap();

            assert_eq!(directory, Path::new("/home/user/.config/systemd/user"));
        }
    }

    #[test]
    fn unit_directory_defaults_to_dot_config_in_the_home_directory() {
        let directory = unit_directory_in(None, Some(PathBuf::from("/home/user"))).unwrap();

        assert_eq!(directory, Path::new("/home/user/.config/systemd/user"));
    }

    #[test]
    fn unit_directory_without_xdg_config_home_or_home_is_an_error() {
        let error = unit_directory_in(None, None).unwrap_err();

        assert_eq!(error.to_string(), "cannot determine the home directory");
    }
}
