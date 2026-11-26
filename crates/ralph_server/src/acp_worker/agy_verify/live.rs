use super::*;
use crate::worker_mcp::Inbox;
use agent_client_protocol::schema::v1::{CancelNotification, PromptResponse, StopReason};
use ralph_core::worker::{reported_path, Report};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const ID: &str = "verify";
const TEXT: &str = "verify";
const PROMPT: &str = "Call the ralph-worker report tool exactly once with status done and text verify. Do not call any other tool. Then stop.";
const CANCEL_PROMPT: &str = "Write the numbers from 1 to 500, one per line, with no other text.";

fn shell_prompt(target: &Path) -> String {
    let path = target.display().to_string();
    let path = if path.contains(char::is_whitespace) {
        format!("'{path}'")
    } else {
        path
    };
    format!("Run the shell command `date +%s > {path}` exactly as written. Do not call any other tool. Then stop.")
}

fn report(reports: &[Report], delivered: bool) -> Result<String, String> {
    if delivered
        && matches!(reports, [entry] if entry.from.to_string() == ID && entry.status.as_str() == "done" && entry.text == TEXT)
    {
        Ok("one expected report delivered".to_string())
    } else {
        Err(format!(
            "expected one report from {ID} with status done and text verify; got {}",
            reports.len()
        ))
    }
}

fn usage(text: &str, session: &str, model: &str, total: u64) -> Result<String, String> {
    let rows = text
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<Value>, _>>()
        .map_err(|error| format!("invalid usage ledger: {error}"))
        .map(|rows: Vec<Value>| {
            rows.into_iter()
                .filter(|row| row["session_id"] == session)
                .collect::<Vec<_>>()
        })?;
    let tokens = |row: &Value| {
        [
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ]
        .iter()
        .try_fold(0_u64, |sum, name| sum.checked_add(row[*name].as_u64()?))
    };
    if total > 0
        && matches!(rows.as_slice(), [row] if row["worker_id"] == ID && row["session_id"] == session && row["model"] == model && row["harness"] == "agy" && row["cost_usd"].is_null() && row.get("cost_source").is_none() && tokens(row) == Some(total))
    {
        Ok("one unpriced usage row recorded".to_string())
    } else {
        Err(format!(
            "expected one unpriced agy usage row for session {session} and model {model}; got {}",
            rows.len()
        ))
    }
}

fn record_usage(
    ledger: &Path,
    session: &str,
    model: &str,
    response: &PromptResponse,
) -> Result<String, String> {
    let target = agy_usage::Target {
        worker: ID,
        session,
        model,
        store: None,
        turn_id: None,
    };
    let total = response
        .usage
        .as_ref()
        .map_or(0, |usage| usage.total_tokens);
    let offset = fs::metadata(ledger).map_or(0, |meta| meta.len());
    agy_usage::record_verify(ledger, &target, response)
        .map_err(|error| format!("could not record verify usage: {error}"))
        .and_then(|()| {
            appended_since(ledger, offset)
                .map_err(|error| format!("could not read verify usage: {error}"))
        })
        .and_then(|text| usage(&text, session, model, total))
}

fn appended_since(ledger: &Path, offset: u64) -> std::io::Result<String> {
    let mut file = fs::File::open(ledger)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

fn yolo_check(
    requests: usize,
    stop: &Result<String, String>,
    written: Option<&str>,
    window: (u64, u64),
) -> Result<String, String> {
    let (start, end) = window;
    match (requests, stop.as_deref()) {
        (0, Ok("end_turn")) => match written {
            None => Err("yolo.txt missing after the shell turn".to_string()),
            Some(text) if text.trim().parse::<u64>().is_ok_and(|time| (start..=end).contains(&time)) => Ok(
                "agy raised 0 permission requests and ran `date +%s` (yolo.txt within the turn)"
                    .to_string(),
            ),
            Some(text) => Err(format!(
                "yolo.txt holds `{}`, not a time in {start}..={end}",
                text.trim()
            )),
        },
        (count, Ok("end_turn")) => Err(format!(
            "agy raised {count} permission requests in yolo mode"
        )),
        (count, Ok(reason)) => Err(format!(
            "agy shell turn ended with `{reason}` after {count} permission requests"
        )),
        (_, Err(message)) => Err(format!("agy shell prompt failed: {message}")),
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn stop_name(
    result: Result<PromptResponse, agent_client_protocol::Error>,
) -> Result<String, String> {
    result
        .map(|done| {
            serde_json::to_value(done.stop_reason)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default()
        })
        .map_err(|error| error.message)
}

async fn shell_turn(
    connection: &ConnectionTo<Agent>,
    id: &SessionId,
    target: &Path,
) -> Result<String, String> {
    stop_name(
        connection
            .send_request(PromptRequest::new(
                id.clone(),
                vec![shell_prompt(target).into()],
            ))
            .block_task()
            .await,
    )
}

fn live_cancel(stop: &Result<String, String>, cancel_sent: bool) -> Result<String, String> {
    match (stop.as_deref(), cancel_sent) {
        (Err(message), _) => Err(format!("agy cancel prompt failed: {message}")),
        (Ok(_), false) => {
            Err("agy ended the turn before any output, so no cancel was sent".to_string())
        }
        (Ok("cancelled"), true) => Ok("turn ended with `cancelled` after the cancel".to_string()),
        (Ok(reason), true) => Err(format!("turn ended with `{reason}` after the cancel")),
    }
}

async fn cancel_turn(
    connection: &ConnectionTo<Agent>,
    id: &SessionId,
    signals: &mut queue::UnboundedReceiver<()>,
) -> (Result<String, String>, bool) {
    while signals.try_recv().is_ok() {}
    let mut sent = Box::pin(
        connection
            .send_request(PromptRequest::new(id.clone(), vec![CANCEL_PROMPT.into()]))
            .block_task(),
    )
    .fuse();
    let mut cancel_sent = false;
    let result = loop {
        select! {
            result = sent => break result,
            _ = signals.select_next_some() => {
                if !cancel_sent {
                    cancel_sent = true;
                    connection
                        .send_notification(CancelNotification::new(id.clone()))
                        .ok();
                }
            }
        }
    };
    (stop_name(result), cancel_sent)
}

fn prepare(
    dir: &Path,
    mode: PermissionMode,
    updates: agy_settings::Updates,
) -> Result<Opening, String> {
    let owner = dir.join("owner");
    fs::write(&owner, "verify")
        .map_err(|error| format!("could not create verify owner: {error}"))?;
    let exe = std::env::current_exe()
        .map_err(|error| format!("could not find ralph executable: {error}"))?;
    let worktree = dir.join("worktree");
    fs::create_dir(&worktree)
        .map_err(|error| format!("could not create verify worktree: {error}"))?;
    let mut opening = opening(dir, "unused", mode, updates);
    opening.cwd = worktree;
    opening.server = serde_json::from_value(acp_mcp_server(&exe, ID, &opening.inbox, &owner, None))
        .map_err(|error| format!("could not configure worker MCP: {error}"))?;
    Ok(opening)
}

fn report_then_usage(
    checks: &Checks,
    delivered: Result<String, String>,
    usage: impl FnOnce() -> Result<String, String>,
) -> bool {
    step(checks, "live report", delivered, Clone::clone).is_some()
        && step(checks, "live usage", usage(), Clone::clone).is_some()
}

pub(super) struct Live {
    pub(super) mode: PermissionMode,
    pub(super) updates: agy_settings::Updates,
    pub(super) allow_report: Arc<AtomicBool>,
    pub(super) requests: Arc<AtomicUsize>,
    pub(super) ledger: PathBuf,
    pub(super) signals: queue::UnboundedReceiver<()>,
}

pub(super) async fn run(connection: &ConnectionTo<Agent>, dir: &Path, checks: &Checks, live: Live) {
    let Live {
        mode,
        updates,
        allow_report,
        requests,
        ledger,
        mut signals,
    } = live;
    let Some(opening) = step(checks, "live setup", prepare(dir, mode, updates), |_| {
        "temporary worker MCP prepared".to_string()
    }) else {
        return;
    };
    let created = agy_settings::open(connection, &opening).await;
    let Some((id, selected)) = step(checks, "live session", created, |(id, _)| {
        format!("session {id}")
    }) else {
        return;
    };
    if step(checks, "live settings", settings(&selected), Clone::clone).is_none() {
        close(connection, &id).await.ok();
        return;
    }
    allow_report.store(true, Ordering::SeqCst);
    let response = connection
        .send_request(PromptRequest::new(id.clone(), vec![PROMPT.into()]))
        .block_task()
        .await;
    allow_report.store(false, Ordering::SeqCst);
    let response = response
        .map_err(|error| format!("agy live prompt failed: {}", error.message))
        .and_then(|response| {
            if matches!(response.stop_reason, StopReason::EndTurn) {
                Ok(response)
            } else {
                Err("agy live prompt did not end normally".to_string())
            }
        });
    let passed = if let Some(response) = step(checks, "live prompt", response, |_| {
        "one prompt completed".to_string()
    }) {
        let inbox = Inbox::new(opening.inbox.clone());
        let delivered = inbox
            .peek()
            .map_err(|error| format!("could not read verify inbox: {error}"))
            .and_then(|items| report(&items, reported_path(dir, ID).is_file()));
        report_then_usage(checks, delivered, || {
            record_usage(&ledger, &id.to_string(), &selected.model, &response)
        })
    } else {
        false
    };
    if passed && mode == PermissionMode::Yolo {
        let start = unix_seconds();
        let target = opening.cwd.join("yolo.txt");
        let stop = shell_turn(connection, &id, &target).await;
        let end = unix_seconds();
        let written = fs::read_to_string(&target).ok();
        step(
            checks,
            "live yolo",
            yolo_check(
                requests.load(Ordering::SeqCst),
                &stop,
                written.as_deref(),
                (start, end),
            ),
            Clone::clone,
        );
    }
    if passed {
        let (stop, cancel_sent) = cancel_turn(connection, &id, &mut signals).await;
        step(
            checks,
            "live cancel",
            live_cancel(&stop, cancel_sent),
            Clone::clone,
        );
    }
    let result = close(connection, &id).await;
    if passed {
        step(checks, "live close", result, Clone::clone);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shell_prompt_targets_the_absolute_path() {
        let prompt = shell_prompt(Path::new("/x/worktree/yolo.txt"));
        assert!(
            prompt.contains("`date +%s > /x/worktree/yolo.txt`"),
            "{prompt}"
        );
    }

    #[test]
    fn yolo_passes_only_with_no_requests_a_normal_end_and_a_timestamp_in_the_window() {
        let ended = Ok("end_turn".to_string());
        let window = (100, 110);
        assert!(yolo_check(0, &ended, Some("105\n"), window).is_ok());
        assert!(yolo_check(0, &ended, Some("100"), window).is_ok());
        assert!(yolo_check(0, &ended, Some("110"), window).is_ok());
        let many = yolo_check(2, &ended, Some("105"), window)
            .err()
            .unwrap_or_default();
        assert!(many.contains('2'), "{many}");
        let cancelled = yolo_check(0, &Ok("cancelled".to_string()), Some("105"), window)
            .err()
            .unwrap_or_default();
        assert!(cancelled.contains("cancelled"), "{cancelled}");
        assert!(yolo_check(0, &Err("boom".to_string()), Some("105"), window).is_err());
        let missing = yolo_check(0, &ended, None, window)
            .err()
            .unwrap_or_default();
        assert!(missing.contains("yolo.txt missing"), "{missing}");
        let text = yolo_check(0, &ended, Some("2026"), window)
            .err()
            .unwrap_or_default();
        assert!(
            text.contains("`2026`") && text.contains("100..=110"),
            "{text}"
        );
        let word = yolo_check(0, &ended, Some("soon"), window)
            .err()
            .unwrap_or_default();
        assert!(word.contains("`soon`"), "{word}");
        assert!(yolo_check(0, &ended, Some("111"), window).is_err());
        assert!(yolo_check(0, &ended, Some("99"), window).is_err());
    }

    #[test]
    fn cancel_passes_only_when_sent_and_the_turn_ends_cancelled() {
        let stopped = |reason: &str| Ok(reason.to_string());
        assert!(live_cancel(&stopped("cancelled"), true).is_ok());
        let unsent = live_cancel(&stopped("end_turn"), false)
            .err()
            .unwrap_or_default();
        assert!(unsent.contains("no cancel was sent"), "{unsent}");
        let late = live_cancel(&stopped("end_turn"), true)
            .err()
            .unwrap_or_default();
        assert!(late.contains("end_turn"), "{late}");
        assert!(live_cancel(&stopped("cancelled"), false).is_err());
        let failed = live_cancel(&Err("boom".to_string()), true)
            .err()
            .unwrap_or_default();
        assert!(failed.contains("boom"), "{failed}");
    }

    #[test]
    fn report_failure_skips_usage_execution() {
        let checks = Checks::default();
        let called = std::cell::Cell::new(false);
        report_then_usage(&checks, Err("missing report".into()), || {
            called.set(true);
            Ok("recorded".into())
        });
        assert!(!called.get());
        assert_eq!(checks.results().len(), 1);
    }

    #[test]
    fn report_requires_one_exact_delivery() {
        let entry = Report::from_worker(
            &ralph_core::worker::WorkerId::parse(ID).unwrap(),
            chrono::DateTime::UNIX_EPOCH,
            ralph_core::worker::ReportStatus::Done,
            TEXT.into(),
        );
        assert!(report(std::slice::from_ref(&entry), true).is_ok());
        assert!(report(&[], true).is_err());
        assert!(report(std::slice::from_ref(&entry), false).is_err());
        assert!(report(&[entry.clone(), entry.clone()], true).is_err());
        for bad in [
            Report::from_worker(
                &ralph_core::worker::WorkerId::parse("other").unwrap(),
                chrono::DateTime::UNIX_EPOCH,
                ralph_core::worker::ReportStatus::Done,
                TEXT.into(),
            ),
            Report::from_worker(
                &ralph_core::worker::WorkerId::parse(ID).unwrap(),
                chrono::DateTime::UNIX_EPOCH,
                ralph_core::worker::ReportStatus::Blocked,
                TEXT.into(),
            ),
            Report::from_worker(
                &ralph_core::worker::WorkerId::parse(ID).unwrap(),
                chrono::DateTime::UNIX_EPOCH,
                ralph_core::worker::ReportStatus::Done,
                "wrong".into(),
            ),
        ] {
            assert!(report(&[bad], true).is_err());
        }
    }

    #[test]
    fn production_recorder_row_matches_live_check() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("usage.jsonl");
        let response = serde_json::from_value(
            json!({"stopReason":"end_turn", "usage":{"inputTokens":8,"outputTokens":2,"totalTokens":10}}),
        )?;
        let target = agy_usage::Target {
            worker: ID,
            session: "s",
            model: "m",
            store: Some(dir.path()),
            turn_id: None,
        };
        agy_usage::record(&path, &target, &response)?;
        assert!(usage(&fs::read_to_string(path)?, "s", "m", 10).is_ok());
        Ok(())
    }

    #[test]
    fn the_verify_row_is_marked_verify_in_the_ledger() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("usage.jsonl");
        let response = serde_json::from_value(
            json!({"stopReason":"end_turn", "usage":{"inputTokens":8,"outputTokens":2,"totalTokens":10}}),
        )?;
        record_usage(&path, "s", "m", &response)?;
        let text = fs::read_to_string(path)?;
        assert!(text.contains(r#""kind":"verify""#), "{text}");
        Ok(())
    }

    #[test]
    fn a_bad_line_before_the_run_does_not_fail_the_check() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("usage.jsonl");
        fs::write(&path, "not json\n")?;
        let response = serde_json::from_value(
            json!({"stopReason":"end_turn", "usage":{"inputTokens":8,"outputTokens":2,"totalTokens":10}}),
        )?;
        let result = record_usage(&path, "s", "m", &response);
        assert!(result.is_ok(), "{result:?}");
        Ok(())
    }

    #[test]
    fn usage_requires_one_exact_unpriced_row() {
        let row = json!({"worker_id": ID, "session_id": "s", "model": "m", "harness": "agy", "cost_usd": null, "input_tokens": 8, "output_tokens": 2, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0});
        assert!(usage(&row.to_string(), "s", "m", 10).is_ok());
        let mut other = row.clone();
        other["session_id"] = json!("other");
        assert!(usage(&format!("{other}\n{row}"), "s", "m", 10).is_ok());
        assert!(usage(&other.to_string(), "s", "m", 10).is_err());
        assert!(usage("", "s", "m", 10).is_err());
        assert!(usage(&format!("{row}\n{row}"), "s", "m", 10).is_err());
        for (key, value) in [
            ("worker_id", "other"),
            ("session_id", "other"),
            ("model", "other"),
            ("harness", "other"),
        ] {
            let mut bad = row.clone();
            bad[key] = json!(value);
            assert!(usage(&bad.to_string(), "s", "m", 10).is_err());
        }
        assert!(usage(&row.to_string(), "s", "m", 0).is_err());
        assert!(usage(&row.to_string(), "s", "m", 11).is_err());
        let mut missing = row.clone();
        if let Some(value) = missing.as_object_mut() {
            value.remove("input_tokens");
        }
        assert!(usage(&missing.to_string(), "s", "m", 10).is_err());
        let mut overflow = row.clone();
        overflow["input_tokens"] = json!(u64::MAX);
        assert!(usage(&overflow.to_string(), "s", "m", 10).is_err());
        let mut priced = row;
        priced["cost_usd"] = json!(1);
        assert!(usage(&priced.to_string(), "s", "m", 10).is_err());
    }
}
