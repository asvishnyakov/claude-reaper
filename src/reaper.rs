use chrono::{DateTime, Local};

use crate::sessions::{SessionRecord, find_transcript, read_records};
use crate::transcript::has_pending_schedule;

const MINUTE_MS: i64 = 60_000;

/// The one client known to resume a stopped session silently: the VS Code extension.
pub const DEFAULT_ENTRYPOINT: &str = "claude-vscode";

/// What a session must satisfy, beyond being interactive and idle, to be stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    pub idle_minutes: u64,
    /// Clients whose sessions may be stopped, matched against the record's `entrypoint`.
    pub entrypoints: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum Decision<'records> {
    Stop {
        record: &'records SessionRecord,
        idle_minutes: i64,
    },
    Skip {
        record: &'records SessionRecord,
        reason: String,
    },
}

/// Records that are not interactive, not idle, or not idle long enough produce no decision.
pub fn decide<'records>(
    records: &'records [SessionRecord],
    now: DateTime<Local>,
    rules: &Rules,
) -> Vec<Decision<'records>> {
    let threshold_ms = i64::try_from(rules.idle_minutes)
        .ok()
        .and_then(|minutes| minutes.checked_mul(MINUTE_MS))
        .unwrap_or(i64::MAX);
    records
        .iter()
        .filter_map(|record| decide_one(record, now, threshold_ms, &rules.entrypoints))
        .collect()
}

/// Decides again from the record file as it is now. A sweep decides for every session first and then
/// stops them one at a time, so a decision can be seconds old when its turn comes: the user may have
/// come back to that session in between.
pub fn still_stoppable(record: &SessionRecord, now: DateTime<Local>, rules: &Rules) -> bool {
    let current = read_records(&record.config_dir)
        .into_iter()
        .find(|candidate| candidate.pid == record.pid);
    current.is_some_and(|current| {
        let current = [current];
        matches!(
            decide(&current, now, rules).as_slice(),
            [Decision::Stop { .. }]
        )
    })
}

fn decide_one<'record>(
    record: &'record SessionRecord,
    now: DateTime<Local>,
    threshold_ms: i64,
    entrypoints: &[String],
) -> Option<Decision<'record>> {
    if record.kind.as_deref() != Some("interactive") || record.status.as_deref() != Some("idle") {
        return None;
    }
    let idle_ms = now
        .timestamp_millis()
        .checked_sub(record.status_updated_at?)?;
    if idle_ms < threshold_ms {
        return None;
    }
    let entrypoint = record
        .entrypoint
        .as_deref()
        .filter(|entrypoint| !entrypoint.is_empty());
    if !entrypoint
        .is_some_and(|entrypoint| entrypoints.iter().any(|selected| selected == entrypoint))
    {
        return Some(Decision::Skip {
            record,
            reason: format!(
                "entrypoint {} not selected",
                entrypoint.unwrap_or("unknown")
            ),
        });
    }
    let schedule = record
        .session_id
        .as_deref()
        .and_then(|session_id| find_transcript(&record.config_dir, session_id))
        .map(|transcript| has_pending_schedule(&transcript, now));
    Some(match schedule {
        Some(Ok(true)) => Decision::Skip {
            record,
            reason: "pending cron or /loop wakeup".to_string(),
        },
        Some(Err(error)) => Decision::Skip {
            record,
            reason: format!("transcript unreadable: {error}"),
        },
        Some(Ok(false)) | None => Decision::Stop {
            record,
            idle_minutes: idle_ms / MINUTE_MS,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    const READ_CALL: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{}}]}}"#;
    const SCHEDULE_WAKEUP_CALL: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"ScheduleWakeup","input":{}}]}}"#;

    fn now() -> DateTime<Local> {
        DateTime::parse_from_rfc3339("2026-10-09T12:00:00Z")
            .unwrap()
            .with_timezone(&Local)
    }

    fn record(config_dir: &Path, idle_minutes: i64) -> SessionRecord {
        SessionRecord {
            pid: 100,
            session_id: Some("s1".to_string()),
            name: Some("fixture".to_string()),
            kind: Some("interactive".to_string()),
            entrypoint: Some(DEFAULT_ENTRYPOINT.to_string()),
            status: Some("idle".to_string()),
            status_updated_at: Some(now().timestamp_millis() - idle_minutes * 60_000),
            proc_start: None,
            config_dir: config_dir.to_path_buf(),
        }
    }

    fn rules(idle_minutes: u64) -> Rules {
        Rules {
            idle_minutes,
            entrypoints: vec![DEFAULT_ENTRYPOINT.to_string()],
        }
    }

    fn with_entrypoint(record: SessionRecord, entrypoint: Option<&str>) -> SessionRecord {
        SessionRecord {
            entrypoint: entrypoint.map(str::to_string),
            ..record
        }
    }

    fn write_transcript(config_dir: &Path, contents: &str) {
        let path = config_dir.join("projects/p/s1.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn idle_interactive_session_past_threshold_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let records = [record(dir.path(), 20)];

        assert_eq!(
            decide(&records, now(), &rules(15)),
            [Decision::Stop {
                record: &records[0],
                idle_minutes: 20
            }]
        );
    }

    #[test]
    fn session_exactly_at_threshold_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let records = [record(dir.path(), 15)];

        assert_eq!(
            decide(&records, now(), &rules(15)),
            [Decision::Stop {
                record: &records[0],
                idle_minutes: 15
            }]
        );
    }

    #[test]
    fn session_under_threshold_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let records = [record(dir.path(), 14)];

        assert_eq!(decide(&records, now(), &rules(15)), []);
    }

    #[test]
    fn busy_session_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let records = [SessionRecord {
            status: Some("busy".to_string()),
            ..record(dir.path(), 60)
        }];

        assert_eq!(decide(&records, now(), &rules(15)), []);
    }

    #[test]
    fn non_interactive_session_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let records = [SessionRecord {
            kind: Some("bg".to_string()),
            ..record(dir.path(), 60)
        }];

        assert_eq!(decide(&records, now(), &rules(15)), []);
    }

    #[test]
    fn missing_status_timestamp_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let records = [SessionRecord {
            status_updated_at: None,
            ..record(dir.path(), 60)
        }];

        assert_eq!(decide(&records, now(), &rules(15)), []);
    }

    #[test]
    fn status_timestamp_in_the_future_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let records = [record(dir.path(), -60)];

        assert_eq!(decide(&records, now(), &rules(15)), []);
    }

    #[test]
    fn pending_schedule_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_transcript(dir.path(), SCHEDULE_WAKEUP_CALL);
        let records = [record(dir.path(), 20)];

        assert_eq!(
            decide(&records, now(), &rules(15)),
            [Decision::Skip {
                record: &records[0],
                reason: "pending cron or /loop wakeup".to_string()
            }]
        );
    }

    #[test]
    fn transcript_without_schedule_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        write_transcript(dir.path(), READ_CALL);
        let records = [record(dir.path(), 20)];

        assert_eq!(
            decide(&records, now(), &rules(15)),
            [Decision::Stop {
                record: &records[0],
                idle_minutes: 20
            }]
        );
    }

    #[test]
    fn unreadable_transcript_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("projects/p/s1.jsonl")).unwrap();
        let records = [record(dir.path(), 20)];

        let decisions = decide(&records, now(), &rules(15));

        let [Decision::Skip { reason, .. }] = decisions.as_slice() else {
            panic!("expected one skip, got {decisions:?}");
        };
        assert!(reason.starts_with("transcript unreadable: "), "{reason}");
    }

    /// The record file as it is on disk now, for a session `decide` saw earlier as `record(dir, 20)`.
    fn write_record_file(config_dir: &Path, status: &str, entrypoint: &str, idle_minutes: i64) {
        let path = config_dir.join("sessions/100.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let record = serde_json::json!({
            "pid": 100,
            "sessionId": "s1",
            "kind": "interactive",
            "entrypoint": entrypoint,
            "status": status,
            "statusUpdatedAt": now().timestamp_millis() - idle_minutes * 60_000,
        });
        fs::write(path, record.to_string()).unwrap();
    }

    #[test]
    fn unchanged_record_is_still_stoppable() {
        let dir = tempfile::tempdir().unwrap();
        write_record_file(dir.path(), "idle", DEFAULT_ENTRYPOINT, 20);

        assert!(still_stoppable(&record(dir.path(), 20), now(), &rules(15)));
    }

    #[test]
    fn record_that_became_busy_is_not_stoppable() {
        let dir = tempfile::tempdir().unwrap();
        write_record_file(dir.path(), "busy", DEFAULT_ENTRYPOINT, 20);

        assert!(!still_stoppable(&record(dir.path(), 20), now(), &rules(15)));
    }

    #[test]
    fn record_updated_since_is_not_stoppable() {
        let dir = tempfile::tempdir().unwrap();
        write_record_file(dir.path(), "idle", DEFAULT_ENTRYPOINT, 0);

        assert!(!still_stoppable(&record(dir.path(), 20), now(), &rules(15)));
    }

    #[test]
    fn record_whose_entrypoint_changed_is_not_stoppable() {
        let dir = tempfile::tempdir().unwrap();
        write_record_file(dir.path(), "idle", "cli", 20);

        assert!(!still_stoppable(&record(dir.path(), 20), now(), &rules(15)));
    }

    #[test]
    fn unselected_entrypoints_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            (Some("cli"), "cli"),
            (Some("sdk-py"), "sdk-py"),
            (Some("something-new"), "something-new"),
            (None, "unknown"),
            (Some(""), "unknown"),
        ];
        for (entrypoint, shown) in cases {
            let records = [with_entrypoint(record(dir.path(), 20), entrypoint)];

            assert_eq!(
                decide(&records, now(), &rules(15)),
                [Decision::Skip {
                    record: &records[0],
                    reason: format!("entrypoint {shown} not selected")
                }],
                "entrypoint {entrypoint:?}"
            );
        }
    }

    #[test]
    fn selected_entrypoints_replace_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let records = [
            with_entrypoint(record(dir.path(), 20), Some("cli")),
            SessionRecord {
                pid: 101,
                ..record(dir.path(), 20)
            },
        ];
        let rules = Rules {
            idle_minutes: 15,
            entrypoints: vec!["cli".to_string()],
        };

        assert_eq!(
            decide(&records, now(), &rules),
            [
                Decision::Stop {
                    record: &records[0],
                    idle_minutes: 20
                },
                Decision::Skip {
                    record: &records[1],
                    reason: "entrypoint claude-vscode not selected".to_string()
                }
            ]
        );
    }

    #[test]
    fn unselected_entrypoint_under_threshold_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let records = [with_entrypoint(record(dir.path(), 14), Some("cli"))];

        assert_eq!(decide(&records, now(), &rules(15)), []);
    }

    #[test]
    fn unselected_entrypoint_is_skipped_before_reading_the_transcript() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("projects/p/s1.jsonl")).unwrap();
        let records = [with_entrypoint(record(dir.path(), 20), Some("cli"))];

        assert_eq!(
            decide(&records, now(), &rules(15)),
            [Decision::Skip {
                record: &records[0],
                reason: "entrypoint cli not selected".to_string()
            }]
        );
    }

    #[test]
    fn record_that_vanished_is_not_stoppable() {
        let dir = tempfile::tempdir().unwrap();

        assert!(!still_stoppable(&record(dir.path(), 20), now(), &rules(15)));
    }

    #[test]
    fn record_without_session_id_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let records = [SessionRecord {
            session_id: None,
            ..record(dir.path(), 20)
        }];

        assert_eq!(
            decide(&records, now(), &rules(15)),
            [Decision::Stop {
                record: &records[0],
                idle_minutes: 20
            }]
        );
    }
}
