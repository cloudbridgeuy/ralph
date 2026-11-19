use super::*;
use crate::worker_mcp::Inbox;
use agent_client_protocol::schema::v1::StopReason;
use ralph_core::worker::{reported_path, Report};
use serde_json::Value;

const ID: &str = "verify";
const TEXT: &str = "verify";
const PROMPT: &str = "Call the ralph-worker report tool exactly once with status done and text verify. Do not call any other tool. Then stop.";
const SHELL_PROMPT: &str =
    "Run the shell command `date +%Y` and reply with its output. Do not call any other tool.";

fn report(reports: &[Report], delivered: bool) -> Result<String, String> {
    if delivered
        && matches!(reports, [entry] if entry.worker == ID && entry.status == "done" && entry.text == TEXT)
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
    let rows: Vec<Value> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|error| format!("invalid usage ledger: {error}"))?;
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

fn yolo_check(requests: usize, stop: &Result<String, String>) -> Result<String, String> {
    match (requests, stop.as_deref()) {
        (0, Ok("end_turn")) => Ok(
            "agy raised 0 permission requests for an MCP report and a shell command".to_string(),
        ),
        (count, Ok("end_turn")) => Err(format!(
            "agy raised {count} permission requests in yolo mode"
        )),
        (count, Ok(reason)) => Err(format!(
            "agy shell turn ended with `{reason}` after {count} permission requests"
        )),
        (_, Err(message)) => Err(format!("agy shell prompt failed: {message}")),
    }
}

async fn shell_turn(connection: &ConnectionTo<Agent>, id: &SessionId) -> Result<String, String> {
    connection
        .send_request(PromptRequest::new(id.clone(), vec![SHELL_PROMPT.into()]))
        .block_task()
        .await
        .map(|done| {
            serde_json::to_value(done.stop_reason)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default()
        })
        .map_err(|error| error.message)
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
}

pub(super) async fn run(connection: &ConnectionTo<Agent>, dir: &Path, checks: &Checks, live: Live) {
    let Live {
        mode,
        updates,
        allow_report,
        requests,
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
            let session = id.to_string();
            let path = dir.join("usage.jsonl");
            let target = agy_usage::Target {
                worker: ID,
                session: &session,
                model: &selected.model,
                store: Some(dir),
                turn_id: None,
            };
            let total = response
                .usage
                .as_ref()
                .map_or(0, |usage| usage.total_tokens);
            agy_usage::record(&path, &target, &response)
                .map_err(|error| format!("could not record verify usage: {error}"))
                .and_then(|()| {
                    fs::read_to_string(&path)
                        .map_err(|error| format!("could not read verify usage: {error}"))
                })
                .and_then(|text| usage(&text, &session, &selected.model, total))
        })
    } else {
        false
    };
    if passed && mode == PermissionMode::Yolo {
        let stop = shell_turn(connection, &id).await;
        step(
            checks,
            "live yolo",
            yolo_check(requests.load(Ordering::SeqCst), &stop),
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
    fn yolo_passes_only_with_no_requests_and_a_normal_end() {
        let ended = Ok("end_turn".to_string());
        assert!(yolo_check(0, &ended).is_ok());
        let many = yolo_check(2, &ended).err().unwrap_or_default();
        assert!(many.contains('2'), "{many}");
        let cancelled = yolo_check(0, &Ok("cancelled".to_string()))
            .err()
            .unwrap_or_default();
        assert!(cancelled.contains("cancelled"), "{cancelled}");
        assert!(yolo_check(0, &Err("boom".to_string())).is_err());
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
        let entry = Report {
            worker: ID.into(),
            at: "t".into(),
            status: "done".into(),
            text: TEXT.into(),
        };
        assert!(report(std::slice::from_ref(&entry), true).is_ok());
        assert!(report(&[], true).is_err());
        assert!(report(std::slice::from_ref(&entry), false).is_err());
        assert!(report(&[entry.clone(), entry.clone()], true).is_err());
        for bad in [
            Report {
                worker: "other".into(),
                ..entry.clone()
            },
            Report {
                status: "blocked".into(),
                ..entry.clone()
            },
            Report {
                text: "wrong".into(),
                ..entry.clone()
            },
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
    fn usage_requires_one_exact_unpriced_row() {
        let row = json!({"worker_id": ID, "session_id": "s", "model": "m", "harness": "agy", "cost_usd": null, "input_tokens": 8, "output_tokens": 2, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0});
        assert!(usage(&row.to_string(), "s", "m", 10).is_ok());
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
