use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    pub pid: u32,
    pub session_id: Option<String>,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub status_updated_at: Option<i64>,
    pub proc_start: Option<String>,
    /// Not part of the JSON; set by `read_records`.
    pub config_dir: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordFile {
    pid: u32,
    session_id: Option<String>,
    name: Option<String>,
    kind: Option<String>,
    status: Option<String>,
    status_updated_at: Option<i64>,
    proc_start: Option<String>,
}

/// Keeps directories that contain `sessions/`, once each, in first-seen order.
pub fn session_config_dirs(candidates: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| candidate.join("sessions").is_dir())
        .filter(|candidate| {
            let identity = fs::canonicalize(candidate).unwrap_or_else(|_| candidate.clone());
            seen.insert(identity)
        })
        .collect()
}

/// Records from `<config_dir>/sessions/*.json`, sorted by file name.
/// Files that are not usable records are skipped silently.
pub fn read_records(config_dir: &Path) -> Vec<SessionRecord> {
    let Ok(entries) = fs::read_dir(config_dir.join("sessions")) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| read_record(path, config_dir))
        .collect()
}

fn read_record(path: &Path, config_dir: &Path) -> Option<SessionRecord> {
    let contents = fs::read(path).ok()?;
    let file: RecordFile = serde_json::from_slice(&contents).ok()?;
    Some(SessionRecord {
        pid: file.pid,
        session_id: file.session_id,
        name: file.name,
        kind: file.kind,
        status: file.status,
        status_updated_at: file.status_updated_at,
        proc_start: file.proc_start,
        config_dir: config_dir.to_path_buf(),
    })
}

/// First `<config_dir>/projects/*/<session_id>.jsonl`.
pub fn find_transcript(config_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let mut projects: Vec<PathBuf> = fs::read_dir(config_dir.join("projects"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    projects.sort();
    projects
        .into_iter()
        .map(|project| project.join(format!("{session_id}.jsonl")))
        .find(|transcript| transcript.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    const INTERACTIVE_RECORD: &str = r#"{"pid":100,"sessionId":"s1","name":"fixture","kind":"interactive","status":"idle","statusUpdatedAt":1791580419201,"procStart":"2202283","version":"2.1.296"}"#;

    #[test]
    fn reads_interactive_record_fields() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "sessions/100.json", INTERACTIVE_RECORD);

        let records = read_records(dir.path());

        assert_eq!(
            records,
            [SessionRecord {
                pid: 100,
                session_id: Some("s1".to_string()),
                name: Some("fixture".to_string()),
                kind: Some("interactive".to_string()),
                status: Some("idle".to_string()),
                status_updated_at: Some(1791580419201),
                proc_start: Some("2202283".to_string()),
                config_dir: dir.path().to_path_buf(),
            }]
        );
    }

    #[test]
    fn skips_files_that_are_not_usable_records() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "sessions/100.json", INTERACTIVE_RECORD);
        write_file(dir.path(), "sessions/101.json", r#"{"pid":"101"}"#);
        write_file(dir.path(), "sessions/102.json", r#"{"kind":"interactive"}"#);
        write_file(dir.path(), "sessions/103.json", "");
        write_file(dir.path(), "sessions/104.json", "not json");
        write_file(dir.path(), "sessions/100.aaaa.key", "x");

        let pids: Vec<u32> = read_records(dir.path())
            .iter()
            .map(|record| record.pid)
            .collect();

        assert_eq!(pids, [100]);
    }

    #[test]
    fn records_are_sorted_by_file_name() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "sessions/200.json", r#"{"pid":200}"#);
        write_file(dir.path(), "sessions/100.json", r#"{"pid":100}"#);

        let pids: Vec<u32> = read_records(dir.path())
            .iter()
            .map(|record| record.pid)
            .collect();

        assert_eq!(pids, [100, 200]);
    }

    #[test]
    fn missing_sessions_directory_gives_no_records() {
        let dir = tempfile::tempdir().unwrap();

        assert!(read_records(dir.path()).is_empty());
    }

    #[test]
    fn keeps_only_directories_with_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let with_sessions = dir.path().join("a");
        let without_sessions = dir.path().join("b");
        fs::create_dir_all(with_sessions.join("sessions")).unwrap();
        fs::create_dir_all(&without_sessions).unwrap();

        let kept = session_config_dirs([with_sessions.clone(), without_sessions]);

        assert_eq!(kept, [with_sessions]);
    }

    #[test]
    fn same_directory_reached_twice_is_kept_once() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join("a");
        fs::create_dir_all(config_dir.join("sessions")).unwrap();
        let mut candidates = vec![config_dir.clone(), config_dir.join("")];
        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&config_dir, &link).unwrap();
            candidates.push(link);
        }

        let kept = session_config_dirs(candidates);

        assert_eq!(kept, [config_dir]);
    }

    #[test]
    fn finds_transcript_in_any_project() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "projects/p1/other.jsonl", "");
        write_file(dir.path(), "projects/p2/s1.jsonl", "");

        assert_eq!(
            find_transcript(dir.path(), "s1"),
            Some(dir.path().join("projects/p2/s1.jsonl"))
        );
        assert_eq!(find_transcript(dir.path(), "nope"), None);
    }
}
