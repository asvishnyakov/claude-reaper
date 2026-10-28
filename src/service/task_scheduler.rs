use std::fs;
use std::io;
use std::path::Path;

use super::{Registration, Uninstalled, escape_xml, run_tool, tool_succeeds, utf8};

pub const TASK_NAME: &str = "claude-reaper";

/// The file in `%TEMP%` that holds the task definition while `schtasks` reads it.
const DEFINITION_FILE: &str = "claude-reaper-task.xml";

/// Writes the task definition, replaces any earlier task with it and starts it. Returns what was
/// registered.
pub fn install(registration: &Registration) -> io::Result<String> {
    let definition = task_definition(registration, &user()?);
    let path = std::env::temp_dir().join(DEFINITION_FILE);
    let registered =
        fs::write(&path, utf16_le_with_bom(&definition)).and_then(|()| create_and_run(&path));
    // The task keeps its own copy, so a file that cannot be removed is no reason to fail.
    let _ = fs::remove_file(&path);
    registered?;
    Ok(describe())
}

/// Ends the task, then deletes it.
pub fn uninstall() -> io::Result<Uninstalled> {
    if !tool_succeeds("schtasks", &["/query", "/tn", TASK_NAME])? {
        return Ok(Uninstalled::NotInstalled);
    }
    // Fails when the task is not running, which must not keep it from being deleted.
    let _ = schtasks(&["/end", "/tn", TASK_NAME]);
    schtasks(&["/delete", "/tn", TASK_NAME, "/f"])?;
    Ok(Uninstalled::Removed(describe()))
}

fn describe() -> String {
    format!("Task Scheduler task {TASK_NAME}")
}

fn schtasks(arguments: &[&str]) -> io::Result<()> {
    run_tool("schtasks", arguments)
}

fn create_and_run(definition: &Path) -> io::Result<()> {
    // Fails when the task does not exist or is not running, which is the common case. A running
    // copy would otherwise make `/run` start nothing, because new instances are ignored.
    let _ = schtasks(&["/end", "/tn", TASK_NAME]);
    schtasks(&["/create", "/tn", TASK_NAME, "/xml", utf8(definition)?, "/f"])?;
    schtasks(&["/run", "/tn", TASK_NAME])
}

/// `%USERDOMAIN%\%USERNAME%`, the account whose logon starts the task.
fn user() -> io::Result<String> {
    Ok(format!(
        r"{}\{}",
        variable("USERDOMAIN")?,
        variable("USERNAME")?
    ))
}

fn variable(name: &str) -> io::Result<String> {
    std::env::var(name).map_err(|error| io::Error::other(format!("cannot read %{name}%: {error}")))
}

/// `arguments` as one command line that the MSVC runtime parses back into the same arguments: an
/// argument is quoted when it is empty or holds a space, tab or `"`; inside quotes, backslashes
/// before a `"` and before the closing quote are doubled and each `"` is escaped.
pub fn command_line_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_string();
    }
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// The task that runs `registration` at the logon of `user` (`DOMAIN\name`), restarts it when it
/// fails and never ends it for running long. Rendered from `task_scheduler.xml`.
pub fn task_definition(registration: &Registration, user: &str) -> String {
    format!(
        include_str!("task_scheduler.xml"),
        user = escape_xml(user),
        command = escape_xml(&registration.program),
        arguments = escape_xml(&command_line_arguments(&registration.arguments)),
    )
}

/// `text` as `schtasks /xml` reads a file whose declaration says UTF-16: little-endian code units
/// after a byte order mark.
fn utf16_le_with_bom(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    bytes
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

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn arguments_with_spaces_quotes_and_trailing_backslashes_are_quoted() {
        let line = command_line_arguments(&words(&["daemon", r"C:\my dir\", "a\"b", ""]));

        assert_eq!(line, r#"daemon "C:\my dir\\" "a\"b" """#);
    }

    #[test]
    fn arguments_without_special_characters_are_left_alone() {
        let line = command_line_arguments(&words(&["daemon", r"C:\profile\", "--idle-minutes=5"]));

        assert_eq!(line, r"daemon C:\profile\ --idle-minutes=5");
    }

    #[test]
    fn backslashes_before_a_quote_are_doubled_and_the_quote_escaped() {
        let line = command_line_arguments(&words(&[r#"a\\"b"#, "tab\there"]));

        assert_eq!(line, "\"a\\\\\\\\\\\"b\" \"tab\there\"");
    }

    #[test]
    fn task_definition_starts_at_logon_of_the_user_and_never_times_out() {
        let text = task_definition(
            &registration(r"C:\tools\claude-reaper.exe", &["daemon"]),
            r"DOMAIN\user",
        );

        assert!(
            text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-16\"?>"),
            "{text}"
        );
        assert!(text.contains("<LogonTrigger>"), "{text}");
        assert_eq!(
            text.matches(r"<UserId>DOMAIN\user</UserId>").count(),
            2,
            "{text}"
        );
        for expected in [
            "<LogonType>InteractiveToken</LogonType>",
            "<RunLevel>LeastPrivilege</RunLevel>",
            "<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>",
            "<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>",
            "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>",
            "<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>",
            "<RestartOnFailure><Interval>PT1M</Interval><Count>255</Count></RestartOnFailure>",
            r"<Command>C:\tools\claude-reaper.exe</Command>",
            "<Arguments>daemon</Arguments>",
        ] {
            assert!(text.contains(expected), "{expected} missing from {text}");
        }
    }

    #[test]
    fn task_definition_escapes_xml_characters() {
        let text = task_definition(
            &registration(r"C:\tools\a&b.exe", &["a>b<c"]),
            r"DOMAIN\u&<ser>",
        );

        assert!(
            text.contains(r"<UserId>DOMAIN\u&amp;&lt;ser&gt;</UserId>"),
            "{text}"
        );
        assert!(
            text.contains(r"<Command>C:\tools\a&amp;b.exe</Command>"),
            "{text}"
        );
        assert!(
            text.contains("<Arguments>a&gt;b&lt;c</Arguments>"),
            "{text}"
        );
    }

    #[test]
    fn definition_file_is_utf16_little_endian_with_a_byte_order_mark() {
        let bytes = utf16_le_with_bom("a\u{20ac}\u{1f600}");

        assert_eq!(
            bytes,
            [0xFF, 0xFE, 0x61, 0x00, 0xAC, 0x20, 0x3D, 0xD8, 0x00, 0xDE]
        );
    }
}
