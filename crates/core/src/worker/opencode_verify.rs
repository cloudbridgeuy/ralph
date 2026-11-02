use super::{
    create_session_body, opencode_blocked_content, parse_event, parse_interrupted,
    session_permissions, session_update, turn_result, turn_stop, ApiData, CheckResult,
    PermissionDecision, SessionMessage, SessionUpdate, TurnSummary, VersionProbe,
    VERSION_PROBE_LIMIT,
};
use serde_json::{json, Value};

pub const OPENCODE_COMMAND: &str = "opencode";
pub const OPENCODE_REQUIRED_CHECKS: &[&str] = &[
    "version",
    "spawn",
    "events",
    "session/create",
    "session/read",
    "interrupt/idle",
    "session/delete",
];
pub const OPENCODE_ENDED: &str = "end_turn";
pub const OPENCODE_INTERRUPTED: &str = "cancelled";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpencodePlan {
    Plain,
    Allow,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpencodeAction {
    Answer {
        request_id: String,
        decision: PermissionDecision,
    },
    Interrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpencodeFold {
    pub summary: TurnSummary,
    pub ended: bool,
}

impl Default for OpencodeFold {
    fn default() -> Self {
        Self {
            summary: TurnSummary {
                stop: Err("the turn did not finish".to_string()),
                replies: 0,
                permissions: Vec::new(),
                cancel_sent: false,
            },
            ended: false,
        }
    }
}

pub fn fold_opencode_event(
    fold: &mut OpencodeFold,
    session_id: &str,
    line: &str,
    plan: OpencodePlan,
) -> Vec<OpencodeAction> {
    let Some(event) = parse_event(line).filter(|event| event.session_id() == Some(session_id))
    else {
        return Vec::new();
    };
    match session_update(&event) {
        Some(SessionUpdate::Blocked(asked)) => {
            let decision = match plan {
                OpencodePlan::Allow => PermissionDecision::Once,
                _ => PermissionDecision::Reject,
            };
            let action = OpencodeAction::Answer {
                request_id: asked.id.clone(),
                decision,
            };
            fold.summary
                .permissions
                .push(opencode_blocked_content(&asked));
            vec![action]
        }
        Some(SessionUpdate::TurnEnded(end)) => {
            fold.ended = true;
            fold.summary.stop = turn_stop(&end).map(str::to_string).map_err(str::to_string);
            Vec::new()
        }
        _ if plan == OpencodePlan::Cancel
            && !fold.summary.cancel_sent
            && event.kind.starts_with("session.step.") =>
        {
            fold.summary.cancel_sent = true;
            vec![OpencodeAction::Interrupt]
        }
        _ => Vec::new(),
    }
}

pub fn opencode_version(output: &str) -> Option<&str> {
    output
        .split_whitespace()
        .map(|word| word.strip_prefix('v').unwrap_or(word))
        .find(|word| word.starts_with(|first: char| first.is_ascii_digit()))
}

pub fn opencode_probed_version(probe: &VersionProbe) -> Result<String, String> {
    let unreadable = || format!("could not read the version from `{OPENCODE_COMMAND} --version`");
    match probe {
        VersionProbe::Exited(output) => opencode_version(output)
            .map(str::to_string)
            .ok_or_else(unreadable),
        VersionProbe::ExitedWithError(_) => Err(format!(
            "could not run `{OPENCODE_COMMAND} --version`: it exited with an error status"
        )),
        VersionProbe::Unreadable(_) => Err(unreadable()),
        VersionProbe::TimedOut => Err(format!(
            "`{OPENCODE_COMMAND} --version` did not answer within {}s",
            VERSION_PROBE_LIMIT.as_secs()
        )),
        VersionProbe::Failed(error) => Err(format!(
            "could not run `{OPENCODE_COMMAND} --version`: {error}"
        )),
    }
}

pub fn verify_session_permissions() -> Value {
    let mut rules = session_permissions();
    if let Some(list) = rules.as_array_mut() {
        list.push(json!({ "action": "shell", "resource": "*", "effect": "ask" }));
    }
    rules
}

pub fn verify_session_body(
    session_id: &str,
    directory: &str,
    model: &str,
    ask: bool,
) -> Result<Value, String> {
    let mut body = create_session_body(session_id, directory, model)?;
    if ask {
        body["permissions"] = verify_session_permissions();
    }
    Ok(body)
}

pub fn reply_count(messages_body: &str) -> usize {
    serde_json::from_str::<ApiData<Vec<SessionMessage>>>(messages_body)
        .ok()
        .and_then(|messages| turn_result(&messages.data, false))
        .map_or(0, |result| usize::from(!result.text.is_empty()))
}

fn session_reply_id(body: &str) -> Option<String> {
    let reply: Value = serde_json::from_str(body).ok()?;
    let id = reply.get("id").or_else(|| reply.get("data")?.get("id"))?;
    id.as_str().map(str::to_string)
}

pub fn session_check(name: &'static str, body: &str, id: &str) -> CheckResult {
    if session_reply_id(body).as_deref() == Some(id) {
        CheckResult::pass(name, format!("the server answered with session {id}"))
    } else {
        CheckResult::fail(
            name,
            format!("the answer did not name session {id}: {body}"),
        )
    }
}

pub fn messages_check(body: &str) -> CheckResult {
    match serde_json::from_str::<ApiData<Vec<SessionMessage>>>(body) {
        Ok(messages) => CheckResult::pass(
            "session/read",
            format!(
                "the session and {} messages were readable",
                messages.data.len()
            ),
        ),
        Err(error) => CheckResult::fail(
            "session/read",
            format!("unreadable message list ({error}): {body}"),
        ),
    }
}

pub fn interrupt_idle_check(body: &str) -> CheckResult {
    match parse_interrupted(body) {
        Some(false) => CheckResult::pass(
            "interrupt/idle",
            "interrupt on an idle session answered interrupted=false",
        ),
        Some(true) => CheckResult::fail(
            "interrupt/idle",
            "interrupt on an idle session answered interrupted=true",
        ),
        None => CheckResult::fail(
            "interrupt/idle",
            format!("unreadable interrupt reply: {body}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{permission_check, prompt_check, PermissionSetup, Verdict};

    const SESSION: &str = "ses_ralphverify";

    fn event(kind: &str, data: Value) -> String {
        format!("data: {}", json!({"id":"evt_1","type":kind,"data":data}))
    }

    fn started(session: &str) -> String {
        event("session.execution.started", json!({"sessionID": session}))
    }

    fn succeeded(session: &str) -> String {
        event("session.execution.succeeded", json!({"sessionID": session}))
    }

    fn asked() -> String {
        event(
            "permission.asked",
            json!({"id":"per_1","sessionID":SESSION,"action":"shell","resources":["date +%Y"],"save":["date *"]}),
        )
    }

    fn feed(fold: &mut OpencodeFold, plan: OpencodePlan, lines: &[String]) -> Vec<OpencodeAction> {
        lines
            .iter()
            .flat_map(|line| fold_opencode_event(fold, SESSION, line, plan))
            .collect()
    }

    #[test]
    fn an_html_body_that_mentions_the_id_does_not_pass_the_session_check() {
        let body = format!("<html><body>not found: {SESSION}</body></html>");
        let check = session_check("session/create", &body, SESSION);
        assert_eq!(check.verdict, Verdict::Fail, "{}", check.detail);
    }

    #[test]
    fn the_session_check_reads_the_id_field_at_top_level_or_under_data() {
        for body in [
            json!({"id": SESSION}),
            json!({"data": {"id": SESSION, "title": "x"}}),
        ] {
            let check = session_check("session/create", &body.to_string(), SESSION);
            assert_eq!(check.verdict, Verdict::Pass, "{}", check.detail);
        }
    }

    #[test]
    fn the_session_check_fails_on_another_id_or_a_mention_inside_a_field() {
        for body in [
            json!({"id": "ses_other"}),
            json!({"data": {"id": "ses_other"}}),
            json!({"title": SESSION}),
            json!({"data": {"id": 7}}),
        ] {
            let check = session_check("session/create", &body.to_string(), SESSION);
            assert_eq!(check.verdict, Verdict::Fail, "{body}");
        }
    }

    #[test]
    fn opencode_version_reads_the_real_version_output() {
        assert_eq!(opencode_version("opencode v2.2.5\n"), Some("2.2.5"));
        assert_eq!(opencode_version("2.2.5"), Some("2.2.5"));
    }

    #[test]
    fn opencode_version_parses_the_v_prefix() {
        assert_eq!(opencode_version("v2.2.5\n"), Some("2.2.5"));
        assert_eq!(opencode_version("2.2.5"), Some("2.2.5"));
        assert_eq!(opencode_version(""), None);
        assert_eq!(opencode_version("opencode"), None);
    }

    #[test]
    fn sse_execution_events_fold_into_a_turn_summary() {
        let mut fold = OpencodeFold::default();
        let sent = feed(
            &mut fold,
            OpencodePlan::Plain,
            &[started(SESSION), succeeded("ses_other"), succeeded(SESSION)],
        );
        assert!(sent.is_empty());
        assert!(fold.ended);
        assert_eq!(fold.summary.stop, Ok(OPENCODE_ENDED.to_string()));
        fold.summary.replies = reply_count(
            r#"{"data":[{"type":"idle","outcome":"succeeded"},{"type":"assistant","content":[{"type":"text","text":"ok"}]},{"type":"user"}]}"#,
        );
        assert_eq!(fold.summary.replies, 1);
        assert_eq!(
            prompt_check(&fold.summary, OPENCODE_ENDED).verdict,
            Verdict::Pass
        );
    }

    #[test]
    fn permission_asked_folds_into_an_allow_once_permission() {
        let mut fold = OpencodeFold::default();
        let sent = feed(
            &mut fold,
            OpencodePlan::Allow,
            &[asked(), succeeded(SESSION)],
        );
        assert_eq!(
            sent,
            vec![OpencodeAction::Answer {
                request_id: "per_1".to_string(),
                decision: PermissionDecision::Once
            }]
        );
        let check = permission_check(&fold.summary, OPENCODE_ENDED, &PermissionSetup::opencode());
        assert_eq!(check.verdict, Verdict::Pass, "{}", check.detail);
        let mut plain = OpencodeFold::default();
        let denied = feed(&mut plain, OpencodePlan::Plain, &[asked()]);
        assert_eq!(
            denied,
            vec![OpencodeAction::Answer {
                request_id: "per_1".to_string(),
                decision: PermissionDecision::Reject
            }]
        );
    }

    #[test]
    fn verify_session_asks_before_bash() {
        let rules = verify_session_permissions();
        let rules = rules.as_array().unwrap();
        let last = rules
            .iter()
            .rposition(|rule| rule["action"] == "shell")
            .unwrap();
        assert_eq!(rules[last]["effect"], "ask");
        assert!(rules.iter().position(|rule| rule["action"] == "*").unwrap() < last);
        let ask = verify_session_body(SESSION, "/tmp/w", "a/b", true).unwrap();
        let plain = verify_session_body(SESSION, "/tmp/w", "a/b", false).unwrap();
        assert_eq!(ask["permissions"], verify_session_permissions());
        assert_ne!(plain["permissions"], ask["permissions"]);
        assert_eq!(ask["id"], SESSION);
    }
}
