use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, Local, TimeDelta};
use croner::Cron;
use serde_json::Value;

const CRON_LIFETIME: TimeDelta = TimeDelta::days(7);

struct CronRequest {
    cron: Option<String>,
    recurring: bool,
}

/// True if the session has a live cron job or its last tool call scheduled a /loop wakeup.
///
/// Fails only when the transcript cannot be opened or read; lines that are not usable are skipped.
pub fn has_pending_schedule(transcript: &Path, now: DateTime<Local>) -> io::Result<bool> {
    let mut cron_requests: HashMap<String, CronRequest> = HashMap::new();
    // Job ID -> when it stops being live; `None` when the creation time is unknown.
    let mut jobs: HashMap<String, Option<DateTime<Local>>> = HashMap::new();
    let mut last_tool_name: Option<String> = None;

    for line in BufReader::new(File::open(transcript)?).split(b'\n') {
        let line = line?;
        if !line.windows(6).any(|window| window == b"\"tool_") {
            continue;
        }
        let Ok(entry) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let Some(blocks) = entry.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let (Some(id), Some(name)) =
                        (string_field(block, "id"), string_field(block, "name"))
                    else {
                        continue;
                    };
                    last_tool_name = Some(name.to_owned());
                    let input = &block["input"];
                    match name {
                        "CronCreate" => {
                            cron_requests.insert(
                                id.to_owned(),
                                CronRequest {
                                    cron: string_field(input, "cron").map(str::to_owned),
                                    recurring: input["recurring"].as_bool().unwrap_or(true),
                                },
                            );
                        }
                        "CronDelete" => {
                            if let Some(job_id) = string_field(input, "id") {
                                jobs.remove(job_id);
                            }
                        }
                        _ => {}
                    }
                }
                Some("tool_result") => {
                    let Some(request) = string_field(block, "tool_use_id")
                        .and_then(|call_id| cron_requests.get(call_id))
                    else {
                        continue;
                    };
                    let Some(job_id) = job_id(&entry, block) else {
                        continue;
                    };
                    let created = entry["timestamp"]
                        .as_str()
                        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                        .map(|created| created.with_timezone(&Local));
                    jobs.insert(
                        job_id,
                        created.and_then(|created| expires_at(created, request)),
                    );
                }
                _ => {}
            }
        }
    }

    let has_live_job = jobs
        .values()
        .any(|expires_at| expires_at.is_none_or(|expires_at| now < expires_at));
    Ok(has_live_job || last_tool_name.as_deref() == Some("ScheduleWakeup"))
}

fn string_field<'value>(value: &'value Value, key: &str) -> Option<&'value str> {
    value.get(key).and_then(Value::as_str)
}

/// A recurring job lasts seven days. A one-shot lasts until its next cron match, or seven days when
/// the expression is missing or cannot be evaluated. `None` means it never expires on its own.
fn expires_at(created: DateTime<Local>, request: &CronRequest) -> Option<DateTime<Local>> {
    let fires_at = Some(request)
        .filter(|request| !request.recurring)
        .and_then(|request| request.cron.as_deref())
        .and_then(|expression| Cron::from_str(expression).ok())
        .and_then(|cron| cron.find_next_occurrence(&created, false).ok());
    fires_at.or_else(|| created.checked_add_signed(CRON_LIFETIME))
}

/// The job ID from the entry's `toolUseResult`, else from the result text ("task 0a1b2c3d").
fn job_id(entry: &Value, block: &Value) -> Option<String> {
    if let Some(id) = entry.pointer("/toolUseResult/id").and_then(Value::as_str) {
        return Some(id.to_owned());
    }
    task_id_in(&block["content"].to_string()).map(str::to_owned)
}

/// First `task ` followed by eight lowercase hex digits, both ends on a word boundary.
fn task_id_in(text: &str) -> Option<&str> {
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    text.match_indices("task ").find_map(|(start, _)| {
        let id = text.get(start + 5..start + 13)?;
        let starts_word = text[..start].chars().next_back().is_some_and(is_word);
        let ends_word = text[start + 13..].chars().next().is_some_and(is_word);
        let is_hex = id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        (is_hex && !starts_word && !ends_word).then_some(id)
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    const CREATED: &str = "2026-10-09T17:10:44.828Z";

    fn at(text: &str) -> DateTime<Local> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Local)
    }

    fn tool_use(call_id: &str, name: &str, input: Value) -> String {
        json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{"type": "tool_use", "id": call_id, "name": name, "input": input}],
            },
        })
        .to_string()
    }

    fn cron_create(call_id: &str, cron: &str, recurring: Option<bool>) -> String {
        let mut input = json!({"cron": cron, "prompt": "check"});
        if let Some(recurring) = recurring {
            input["recurring"] = json!(recurring);
        }
        tool_use(call_id, "CronCreate", input)
    }

    fn cron_created(call_id: &str, job_id: &str, timestamp: Option<&str>) -> String {
        let mut entry = json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": format!("Scheduled task {job_id} (*/5 * * * *). Auto-expires."),
                }],
            },
            "toolUseResult": {"id": job_id, "recurring": true, "durable": false},
        });
        if let Some(timestamp) = timestamp {
            entry["timestamp"] = json!(timestamp);
        }
        entry.to_string()
    }

    fn cron_delete(call_id: &str, job_id: &str) -> String {
        tool_use(call_id, "CronDelete", json!({"id": job_id}))
    }

    fn write_transcript(lines: &[String]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s1.jsonl");
        fs::write(&path, lines.join("\n") + "\n").unwrap();
        (dir, path)
    }

    fn pending(lines: &[String], now: &str) -> bool {
        let (_dir, path) = write_transcript(lines);
        has_pending_schedule(&path, at(now)).unwrap()
    }

    #[test]
    fn no_tool_calls_means_nothing_pending() {
        let lines = [json!({"type": "user", "message": {"content": "hello"}}).to_string()];

        assert!(!pending(&lines, "2026-10-09T00:00:00Z"));
    }

    #[test]
    fn recurring_job_is_live_within_seven_days() {
        let lines = [
            cron_create("toolu_1", "*/5 * * * *", None),
            cron_created("toolu_1", "8251faac", Some("2026-10-01T00:00:00Z")),
        ];

        assert!(pending(&lines, "2026-10-07T00:00:00Z"));
    }

    #[test]
    fn recurring_job_expires_after_seven_days() {
        let lines = [
            cron_create("toolu_1", "*/5 * * * *", None),
            cron_created("toolu_1", "8251faac", Some("2026-10-01T00:00:00Z")),
        ];

        assert!(!pending(&lines, "2026-10-09T00:00:00Z"));
    }

    #[test]
    fn deleted_job_is_not_live() {
        let lines = [
            cron_create("toolu_1", "*/5 * * * *", None),
            cron_created("toolu_1", "8251faac", Some("2026-10-01T00:00:00Z")),
            cron_delete("toolu_2", "8251faac"),
        ];

        assert!(!pending(&lines, "2026-10-02T00:00:00Z"));
    }

    #[test]
    fn one_shot_is_live_before_it_fires() {
        let lines = [
            cron_create("toolu_1", "7 3 1 1 *", Some(false)),
            cron_created("toolu_1", "8251faac", Some(CREATED)),
        ];

        assert!(pending(&lines, "2026-12-30T12:00:00Z"));
    }

    #[test]
    fn one_shot_is_gone_after_it_fires() {
        let lines = [
            cron_create("toolu_1", "7 3 1 1 *", Some(false)),
            cron_created("toolu_1", "8251faac", Some(CREATED)),
        ];

        assert!(!pending(&lines, "2027-01-03T12:00:00Z"));
    }

    #[test]
    fn fired_one_shot_does_not_protect_for_seven_days() {
        let lines = [
            cron_create("toolu_1", "*/5 * * * *", Some(false)),
            cron_created("toolu_1", "8251faac", Some(CREATED)),
        ];

        assert!(!pending(&lines, "2026-10-10T17:10:44Z"));
    }

    #[test]
    fn unparseable_one_shot_uses_the_seven_day_rule() {
        let lines = [
            cron_create("toolu_1", "not a cron", Some(false)),
            cron_created("toolu_1", "8251faac", Some("2026-10-01T00:00:00Z")),
        ];

        assert!(pending(&lines, "2026-10-07T00:00:00Z"));
        assert!(!pending(&lines, "2026-10-09T00:00:00Z"));
    }

    #[test]
    fn job_id_falls_back_to_result_text() {
        let result_without_id = json!({
            "type": "user",
            "timestamp": "2026-10-01T00:00:00Z",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_1",
                    "content": "Scheduled recurring task 0a1b2c3d (*/5 * * * *). Auto-expires.",
                }],
            },
        })
        .to_string();
        let created = [
            cron_create("toolu_1", "*/5 * * * *", None),
            result_without_id,
        ];
        let mut deleted = created.to_vec();
        deleted.push(cron_delete("toolu_2", "0a1b2c3d"));

        assert!(pending(&created, "2026-10-02T00:00:00Z"));
        assert!(!pending(&deleted, "2026-10-02T00:00:00Z"));
    }

    #[test]
    fn job_without_timestamp_stays_live() {
        let lines = [
            cron_create("toolu_1", "*/5 * * * *", None),
            cron_created("toolu_1", "8251faac", None),
        ];

        assert!(pending(&lines, "2030-01-01T00:00:00Z"));
    }

    #[test]
    fn job_with_unparseable_timestamp_stays_live() {
        let lines = [
            cron_create("toolu_1", "*/5 * * * *", None),
            cron_created("toolu_1", "8251faac", Some("yesterday-ish")),
        ];

        assert!(pending(&lines, "2030-01-01T00:00:00Z"));
    }

    #[test]
    fn schedule_wakeup_as_last_tool_call_is_pending() {
        let lines = [
            tool_use("toolu_1", "Read", json!({})),
            tool_use("toolu_2", "ScheduleWakeup", json!({"delaySeconds": 600})),
        ];

        assert!(pending(&lines, "2026-10-09T00:00:00Z"));
    }

    #[test]
    fn schedule_wakeup_followed_by_another_tool_is_not_pending() {
        let lines = [
            tool_use("toolu_1", "ScheduleWakeup", json!({"delaySeconds": 600})),
            tool_use("toolu_2", "Read", json!({})),
        ];

        assert!(!pending(&lines, "2026-10-09T00:00:00Z"));
    }

    #[test]
    fn malformed_line_is_skipped() {
        let lines = [
            cron_create("toolu_1", "*/5 * * * *", None),
            cron_created("toolu_1", "8251faac", Some("2026-10-01T00:00:00Z")),
            r#"{"tool_ broken"#.to_string(),
            r#"{"message":{"content":"tool_ text"}}"#.to_string(),
            r#"{"type":"tool_result","message":{"content":[1,null,{"type":"tool_use"},{"type":"tool_result"}]}}"#
                .to_string(),
        ];
        let (_dir, path) = write_transcript(&lines);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"\xff \"tool_ not utf-8\n").unwrap();

        assert!(has_pending_schedule(&path, at("2026-10-02T00:00:00Z")).unwrap());
    }

    #[test]
    fn missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();

        let result = has_pending_schedule(&dir.path().join("missing.jsonl"), at(CREATED));

        assert!(result.is_err());
    }
}
