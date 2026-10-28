use std::io;
use std::process::Command;

use super::Identity;

/// One line of `ps -o uid=,stat=,lstart=`: uid, first character of the state, and the start time
/// with every run of whitespace collapsed to one space.
pub(crate) fn parse_ps_line(line: &str) -> Option<(u32, char, String)> {
    let mut fields = line.split_whitespace();
    let uid = fields.next()?.parse().ok()?;
    let state = fields.next()?.chars().next()?;
    let start: Vec<&str> = fields.collect();
    (!start.is_empty()).then(|| (uid, state, start.join(" ")))
}

/// What `ps` says about one process, with its start time as in a session record's `procStart`
/// (`LC_ALL=C` for English names, `TZ=UTC` whatever zone the session itself runs in): `Err` when
/// `ps` cannot be run, `None` when it names no such process.
fn ps_process(pid: u32) -> io::Result<Option<(u32, char, String)>> {
    let output = Command::new("ps")
        .args(["-o", "uid=,stat=,lstart=", "-p", &pid.to_string()])
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(parse_ps_line))
}

pub fn identity(pid: u32, proc_start: Option<&str>) -> Identity {
    let Ok(process) = ps_process(pid) else {
        return Identity::Unverified;
    };
    let Some((uid, state, start)) = process else {
        return Identity::Gone;
    };
    if state == 'Z' {
        return Identity::Gone;
    }
    // A session record can name any PID; only this user's own processes are ever ours to stop.
    if uid != rustix::process::getuid().as_raw() {
        return Identity::Different;
    }
    match proc_start {
        None => Identity::Unverified,
        // `ps` pads a one-digit day with a second space, which a record may not have.
        Some(recorded) if recorded.split_whitespace().eq(start.split_whitespace()) => {
            Identity::Same
        }
        Some(_) => Identity::Different,
    }
}

/// What Claude Code would record as `procStart` for `pid`.
#[cfg(test)]
pub(crate) fn proc_start_of(pid: u32) -> String {
    let (_, _, start) = ps_process(pid)
        .expect("ps runs")
        .expect("the process exists");
    start
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::thread::sleep;
    use std::time::Duration;

    use super::*;

    #[test]
    fn ps_line_collapses_the_padded_day() {
        let parsed = parse_ps_line("  501 Ss   Mon Oct  5 09:01:02 2026");

        assert_eq!(parsed, Some((501, 'S', "Mon Oct 5 09:01:02 2026".into())));
    }

    #[test]
    fn own_process_with_its_own_start_is_same() {
        let id = std::process::id();

        assert_eq!(identity(id, Some(&proc_start_of(id))), Identity::Same);
    }

    #[test]
    fn recorded_start_with_extra_spaces_is_same() {
        let id = std::process::id();
        let recorded = format!(" {}  ", proc_start_of(id).replace(' ', "  "));

        assert_eq!(identity(id, Some(&recorded)), Identity::Same);
    }

    #[test]
    fn own_process_with_a_wrong_start_is_different() {
        let id = std::process::id();

        assert_eq!(
            identity(id, Some("Mon Jan 1 00:00:00 2001")),
            Identity::Different
        );
    }

    #[test]
    fn own_process_without_recorded_start_is_unverified() {
        assert_eq!(identity(std::process::id(), None), Identity::Unverified);
    }

    #[test]
    fn exited_child_is_gone() {
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();

        assert_eq!(identity(child.id(), None), Identity::Gone);
    }

    #[test]
    fn unreaped_child_is_gone() {
        let mut child = Command::new("true").spawn().unwrap();
        sleep(Duration::from_millis(300));

        let found = identity(child.id(), None);

        child.wait().unwrap();
        assert_eq!(found, Identity::Gone);
    }

    #[test]
    fn process_of_another_user_is_different() {
        assert_eq!(identity(1, None), Identity::Different);
    }
}
