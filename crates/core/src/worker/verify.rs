use crate::cursor_leftovers::{cursor_slug, project_dirs};
use crate::worker::BlockedContent;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const VERIFIED_AGENTS_FILE: &str = "verified-agents.json";
pub const CURSOR_HOOKS_FILE: &str = ".cursor/hooks.json";
pub const CURSOR_VERIFY_HOOKS: &str = r#"{"version":1,"hooks":{"beforeShellExecution":[{"command":"echo '{\"permission\":\"ask\"}'"}]}}"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedAgent {
    pub agent: String,
    pub version: String,
    pub live: bool,
    pub verified_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedAgents {
    #[serde(default)]
    pub agents: Vec<VerifiedAgent>,
}

impl VerifiedAgents {
    pub fn is_verified(&self, agent: &str, version: &str) -> bool {
        self.agents
            .iter()
            .any(|entry| entry.agent == agent && entry.version == version)
    }

    pub fn record(mut self, entry: VerifiedAgent) -> Self {
        let same =
            |known: &VerifiedAgent| known.agent == entry.agent && known.version == entry.version;
        match self.agents.iter_mut().find(|known| same(known)) {
            Some(known) if known.live && !entry.live => {}
            Some(known) => *known = entry,
            None => self.agents.push(entry),
        }
        self
    }
}

pub fn merge_verified(
    existing: Option<&str>,
    entry: VerifiedAgent,
) -> Result<VerifiedAgents, String> {
    let known = match existing {
        Some(text) => serde_json::from_str::<VerifiedAgents>(text).map_err(|error| {
            format!("refusing to overwrite {VERIFIED_AGENTS_FILE}: it does not parse ({error}); fix or remove it")
        })?,
        None => VerifiedAgents::default(),
    };
    Ok(known.record(entry))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    Unknown,
}

impl Verdict {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub name: &'static str,
    pub verdict: Verdict,
    pub detail: String,
    pub blocking: bool,
}

impl CheckResult {
    pub fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self::blocking(name, Verdict::Pass, detail)
    }

    pub fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self::blocking(name, Verdict::Fail, detail)
    }

    pub fn advisory(name: &'static str, verdict: Verdict, detail: impl Into<String>) -> Self {
        Self {
            blocking: false,
            ..Self::blocking(name, verdict, detail)
        }
    }

    fn blocking(name: &'static str, verdict: Verdict, detail: impl Into<String>) -> Self {
        Self {
            name,
            verdict,
            detail: detail.into(),
            blocking: true,
        }
    }
}

pub const AGY_REQUIRED_CHECKS: &[&str] = &[
    "version",
    "initialize",
    "session/new",
    "settings/new",
    "mcp/new",
    "session/close",
    "session/load",
    "settings/load",
    "mcp/load",
    "session/close reload",
];
pub const CURSOR_REQUIRED_CHECKS: &[&str] = &["initialize", "session/new"];
const CURSOR_LIVE_REQUIRED_CHECKS: &[&str] =
    &["initialize", "session/new", "print prompt", "print usage"];

pub fn cursor_required_checks(live: bool) -> &'static [&'static str] {
    if live {
        CURSOR_LIVE_REQUIRED_CHECKS
    } else {
        CURSOR_REQUIRED_CHECKS
    }
}
pub const CODEX_REQUIRED_CHECKS: &[&str] = &["initialize", "initialized", "thread/start"];

pub const FREE_VERIFY_LIMIT: Duration = Duration::from_secs(90);
pub const LIVE_VERIFY_LIMIT: Duration = Duration::from_secs(540);

pub fn verify_limit(live: bool) -> Duration {
    if live {
        LIVE_VERIFY_LIMIT
    } else {
        FREE_VERIFY_LIMIT
    }
}

pub fn limit_failure(
    name: &'static str,
    remaining: Duration,
    limit: Duration,
) -> Option<CheckResult> {
    remaining.is_zero().then(|| {
        CheckResult::fail(
            name,
            format!(
                "the {}s verify limit passed before the {name} check ran",
                limit.as_secs()
            ),
        )
    })
}

pub fn missing_checks<'a>(results: &[CheckResult], required: &[&'a str]) -> Vec<&'a str> {
    required
        .iter()
        .copied()
        .filter(|name| !results.iter().any(|result| result.name == *name))
        .collect()
}

pub fn required_failure(results: &[CheckResult], required: &[&str]) -> Option<CheckResult> {
    let missing = missing_checks(results, required);
    let earlier_failure = results
        .iter()
        .any(|result| result.blocking && result.verdict != Verdict::Pass);
    (!earlier_failure && !missing.is_empty()).then(|| {
        CheckResult::fail(
            "required",
            format!("these checks did not run: {}", missing.join(", ")),
        )
    })
}

pub fn spec_passed(results: &[CheckResult], version: Option<&str>, required: &[&str]) -> bool {
    version.is_some()
        && missing_checks(results, required).is_empty()
        && results
            .iter()
            .filter(|result| result.blocking)
            .all(|result| result.verdict == Verdict::Pass)
}

pub fn render_check(result: &CheckResult) -> String {
    let advisory = if result.blocking {
        ""
    } else {
        " (does not fail the spec)"
    };
    format!(
        "{:<8}{}{advisory}: {}",
        result.verdict.label(),
        result.name,
        result.detail
    )
}

pub fn render_summary(agent: &str, version: &str, live: bool, passed: bool) -> String {
    let scope = match (live, agent) {
        (true, _) => "free and live checks",
        (false, "cursor") => "free checks only; --live adds print prompt and print usage checks",
        (false, _) => "free checks only; --live adds prompt, permission and cancel checks",
    };
    let outcome = if passed { "PASS" } else { "FAIL" };
    format!("{outcome}: {agent} {version} ({scope})")
}

pub const STDERR_TAIL_CAP: usize = 2048;

pub fn stderr_tail(bytes: &[u8], cap: usize) -> String {
    let tail = &bytes[bytes.len().saturating_sub(cap)..];
    let start = tail
        .iter()
        .position(|byte| byte & 0xC0 != 0x80)
        .unwrap_or(tail.len());
    String::from_utf8_lossy(&tail[start..]).trim().to_string()
}

pub fn with_stderr_tail(result: CheckResult, tail: &str) -> CheckResult {
    if result.verdict != Verdict::Fail || tail.is_empty() {
        return result;
    }
    CheckResult {
        detail: format!("{}; agent stderr: {tail}", result.detail),
        ..result
    }
}

pub fn verified_entry(agent: &str, version: &str, live: bool, today: &str) -> VerifiedAgent {
    VerifiedAgent {
        agent: agent.to_string(),
        version: version.to_string(),
        live,
        verified_at: today.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSummary {
    pub stop: Result<String, String>,
    pub replies: usize,
    pub permissions: Vec<BlockedContent>,
    pub cancel_sent: bool,
}

impl TurnSummary {
    fn stopped_with(&self, expected: &str) -> Result<(), String> {
        match &self.stop {
            Ok(reason) if reason == expected => Ok(()),
            Ok(reason) => Err(format!("turn ended with `{reason}`, expected `{expected}`")),
            Err(message) => Err(message.clone()),
        }
    }
}

pub fn prompt_check(turn: &TurnSummary, ended: &str) -> CheckResult {
    match (turn.stopped_with(ended), turn.replies) {
        (Err(message), _) => CheckResult::fail("prompt/reply", message),
        (Ok(()), 0) => CheckResult::fail("prompt/reply", "turn ended without any reply text"),
        (Ok(()), replies) => CheckResult::pass(
            "prompt/reply",
            format!("turn ended with `{ended}` after {replies} reply updates"),
        ),
    }
}

pub fn cursor_verify_project(home: &Path, workspace: &Path) -> PathBuf {
    home.join(".cursor/projects")
        .join(cursor_slug(&workspace.to_string_lossy()))
}

pub fn cursor_verify_leftovers(home: &Path, workspace: &Path, real: &Path) -> Vec<PathBuf> {
    let projects = home.join(".cursor/projects");
    let project = cursor_verify_project(home, workspace);
    let mut leftovers: Vec<PathBuf> = Vec::new();
    for path in [workspace, real] {
        let Some(path) = path.to_str() else { continue };
        for dir in project_dirs(&projects, path) {
            if dir != project && !leftovers.contains(&dir) {
                leftovers.push(dir);
            }
        }
    }
    leftovers
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionSetup {
    pub label: String,
    pub no_request: String,
}

impl PermissionSetup {
    pub fn claude() -> Self {
        let label =
            "setting-sources project,local with permission-mode default and prompt-tool stdio";
        Self {
            label: label.to_string(),
            no_request: format!(
                "claude sent no can_use_tool request although it runs {label} and allows no shell tool"
            ),
        }
    }

    pub fn opencode() -> Self {
        let label = "a session whose permission rules ask before shell commands";
        Self {
            label: label.to_string(),
            no_request: format!("opencode sent no permission.asked event although it runs {label}"),
        }
    }

    pub fn codex() -> Self {
        let label = "a read-only thread with approvalPolicy on-request";
        Self {
            label: label.to_string(),
            no_request: format!("codex sent no approval request although it runs {label}"),
        }
    }
}

pub fn permission_check(turn: &TurnSummary, ended: &str, setup: &PermissionSetup) -> CheckResult {
    let allowable = turn.permissions.first().map(|request| {
        request
            .options
            .iter()
            .any(|option| option.kind == "allow_once")
    });
    let mut result = match (allowable, turn.stopped_with(ended)) {
        (None, _) => CheckResult::fail("permission", setup.no_request.clone()),
        (Some(false), _) => CheckResult::fail("permission", "the request had no allow_once option"),
        (Some(true), Err(message)) => CheckResult::fail("permission", message),
        (Some(true), Ok(())) => CheckResult::pass(
            "permission",
            format!(
                "request from `{}` had an allow_once option; allowed; turn ended with `{ended}`",
                turn.permissions.first().map_or("", |request| &request.tool)
            ),
        ),
    };
    result.detail = format!("{} (setup: {})", result.detail, setup.label);
    result
}

pub fn cancel_check(turn: &TurnSummary, cancelled: &str) -> CheckResult {
    let advisory = |verdict, detail: String| CheckResult::advisory("cancel", verdict, detail);
    match (&turn.stop, turn.cancel_sent) {
        (Err(message), _) => advisory(Verdict::Fail, message.clone()),
        (Ok(_), false) => advisory(
            Verdict::Unknown,
            "the turn ended before any output, so no cancel was sent".to_string(),
        ),
        (Ok(reason), true) if reason == cancelled => advisory(
            Verdict::Pass,
            format!("turn ended with `{cancelled}` after the cancel"),
        ),
        (Ok(reason), true) => advisory(
            Verdict::Unknown,
            format!("turn ended with `{reason}`; the cancel may have arrived too late"),
        ),
    }
}

pub fn verify_hint(agent: &str) -> String {
    format!("run `ralph worker verify {agent}` to check this version.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(live: bool, at: &str) -> VerifiedAgent {
        verified_entry("cursor", "2026.10.22", live, at)
    }

    #[test]
    fn a_free_pass_never_downgrades_a_live_record() {
        let live = VerifiedAgents::default().record(entry(true, "2026-10-01"));
        assert_eq!(
            live.clone().record(entry(false, "2026-10-02")).agents,
            vec![entry(true, "2026-10-01")]
        );
        assert_eq!(
            live.record(entry(true, "2026-10-03")).agents,
            vec![entry(true, "2026-10-03")]
        );
    }

    #[test]
    fn a_live_pass_upgrades_a_free_record_and_agents_stay_apart() {
        let file = VerifiedAgents::default()
            .record(entry(false, "2026-10-01"))
            .record(entry(true, "2026-10-02"))
            .record(verified_entry("codex", "2026.10.22", false, "2026-10-02"));
        assert_eq!(file.agents.len(), 2);
        assert!(file.is_verified("cursor", "2026.10.22"));
        assert!(file.is_verified("codex", "2026.10.22"));
        assert!(!file.is_verified("cursor", "2026.10.23"));
    }

    #[test]
    fn the_file_round_trips_and_tolerates_an_empty_object() {
        let file = VerifiedAgents::default().record(entry(false, "2026-10-01"));
        let text = serde_json::to_string(&file).unwrap();
        assert_eq!(serde_json::from_str::<VerifiedAgents>(&text).unwrap(), file);
        assert_eq!(
            serde_json::from_str::<VerifiedAgents>("{}").unwrap(),
            VerifiedAgents::default()
        );
    }

    #[test]
    fn saving_refuses_a_record_file_it_cannot_parse() {
        let error = merge_verified(Some("{not json"), entry(false, "2026-10-01")).unwrap_err();
        assert!(error.starts_with("refusing to overwrite verified-agents.json"));
        assert!(merge_verified(Some(r#"{"agents":7}"#), entry(false, "2026-10-01")).is_err());
    }

    #[test]
    fn saving_starts_or_extends_the_record_file() {
        let first = merge_verified(None, entry(false, "2026-10-01")).unwrap();
        assert_eq!(first.agents, vec![entry(false, "2026-10-01")]);
        let text = serde_json::to_string(&first).unwrap();
        let codex = verified_entry("codex", "0.164.0", true, "2026-10-02");
        let second = merge_verified(Some(&text), codex.clone()).unwrap();
        assert_eq!(second.agents, vec![entry(false, "2026-10-01"), codex]);
    }

    #[test]
    fn advisory_checks_never_fail_the_spec_but_blocking_ones_do() {
        let cancel = CheckResult::advisory("cancel", Verdict::Fail, "x");
        let version = Some("1.0");
        assert!(spec_passed(
            &[CheckResult::pass("a", "ok"), cancel.clone()],
            version,
            &[]
        ));
        assert!(!spec_passed(
            &[CheckResult::fail("a", "no"), cancel],
            version,
            &[]
        ));
        let unknown = CheckResult {
            verdict: Verdict::Unknown,
            ..CheckResult::pass("b", "")
        };
        assert!(!spec_passed(&[unknown], version, &[]));
    }

    #[test]
    fn the_summary_names_the_outcome_and_the_check_scope() {
        assert_eq!(
            render_summary("codex", "0.164.0", false, true),
            "PASS: codex 0.164.0 (free checks only; --live adds prompt, permission and cancel checks)"
        );
        assert_eq!(
            render_summary("cursor", "unknown version", true, false),
            "FAIL: cursor unknown version (free and live checks)"
        );
    }

    #[test]
    fn the_cursor_free_summary_names_the_print_checks() {
        let summary = render_summary("cursor", "2026.10.22", false, true);
        assert_eq!(
            summary,
            "PASS: cursor 2026.10.22 (free checks only; --live adds print prompt and print usage checks)"
        );
        assert!(!summary.contains("permission") && !summary.contains("cancel"));
    }

    #[test]
    fn the_stderr_tail_keeps_the_last_bytes_and_trims() {
        assert_eq!(stderr_tail(b"abcdef\n", 4), "def");
        assert_eq!(stderr_tail(b"  short \n", 100), "short");
        assert_eq!(stderr_tail(b"", 4), "");
    }

    #[test]
    fn the_stderr_tail_never_splits_a_char() {
        assert_eq!(stderr_tail("a\u{e9}b".as_bytes(), 2), "b");
        assert_eq!(stderr_tail("a\u{e9}b".as_bytes(), 3), "\u{e9}b");
    }

    #[test]
    fn a_failed_row_gains_the_stderr_tail_and_other_rows_do_not() {
        let failed = CheckResult::fail("initialize", "no answer");
        assert_eq!(
            with_stderr_tail(failed.clone(), "boom").detail,
            "no answer; agent stderr: boom"
        );
        assert_eq!(with_stderr_tail(failed.clone(), ""), failed);
        let passed = CheckResult::pass("initialize", "ok");
        assert_eq!(with_stderr_tail(passed.clone(), "boom"), passed);
    }

    #[test]
    fn an_expired_deadline_fails_the_named_check() {
        assert_eq!(
            limit_failure("events", Duration::ZERO, FREE_VERIFY_LIMIT),
            Some(CheckResult::fail(
                "events",
                "the 90s verify limit passed before the events check ran"
            ))
        );
    }

    #[test]
    fn time_left_lets_the_check_run() {
        assert_eq!(
            limit_failure("spawn", Duration::from_millis(1), LIVE_VERIFY_LIMIT),
            None
        );
    }

    #[test]
    fn the_live_limit_is_named_in_the_failure() {
        let failure = limit_failure("session/read", Duration::ZERO, verify_limit(true)).unwrap();
        assert!(failure.detail.contains("540s"), "{}", failure.detail);
        assert_eq!(verify_limit(false), Duration::from_secs(90));
    }

    #[test]
    fn a_live_cursor_verify_requires_the_print_prompt_and_usage() {
        let live = cursor_required_checks(true);
        assert!(live.contains(&"print prompt"));
        assert!(live.contains(&"print usage"));
        assert!(!cursor_required_checks(false).contains(&"print prompt"));
    }

    #[test]
    fn an_unknown_version_fails_the_spec_even_when_every_check_passed() {
        let passed = [
            CheckResult::pass("spawn", "started"),
            CheckResult::pass("initialize", "answered"),
        ];
        assert!(spec_passed(&passed, Some("0.164.0"), &[]));
        assert!(!spec_passed(&passed, None, &[]));
    }

    #[test]
    fn the_required_line_shows_only_when_no_earlier_check_failed() {
        let passed = [CheckResult::pass("initialize", "answered")];
        let failure = required_failure(&passed, CURSOR_REQUIRED_CHECKS).unwrap();
        assert_eq!(failure.verdict, Verdict::Fail);
        assert_eq!(failure.detail, "these checks did not run: session/new");
        let failed = [CheckResult::fail("initialize", "no answer")];
        assert_eq!(required_failure(&failed, CURSOR_REQUIRED_CHECKS), None);
        let advisory = CheckResult::advisory("cancel", Verdict::Fail, "late");
        assert!(required_failure(&[passed[0].clone(), advisory], CURSOR_REQUIRED_CHECKS).is_some());
        assert!(!spec_passed(&failed, Some("1.0"), CURSOR_REQUIRED_CHECKS));
    }

    #[test]
    fn a_spec_fails_unless_its_required_checks_ran() {
        assert!(!spec_passed(&[], Some("1.0"), CURSOR_REQUIRED_CHECKS));
        assert!(!spec_passed(&[], Some("1.0"), CODEX_REQUIRED_CHECKS));
        let cursor = [
            CheckResult::pass("initialize", "answered"),
            CheckResult::pass("session/new", "session s"),
        ];
        assert!(spec_passed(&cursor, Some("1.0"), CURSOR_REQUIRED_CHECKS));
        assert!(!spec_passed(
            &cursor[..1],
            Some("1.0"),
            CURSOR_REQUIRED_CHECKS
        ));
        assert_eq!(
            missing_checks(&cursor, CODEX_REQUIRED_CHECKS),
            vec!["initialized", "thread/start"]
        );
    }

    fn turn(stop: Result<&str, &str>, replies: usize, kinds: &[&str]) -> TurnSummary {
        TurnSummary {
            stop: stop.map(str::to_string).map_err(str::to_string),
            replies,
            permissions: kinds
                .iter()
                .map(|kind| BlockedContent {
                    request_id: "r".into(),
                    tool: "edit".into(),
                    summary: "write".into(),
                    options: vec![crate::worker::PermissionOption {
                        option_id: "o".into(),
                        name: "O".into(),
                        kind: (*kind).into(),
                    }],
                    pending: Vec::new(),
                })
                .collect(),
            cancel_sent: replies > 0,
        }
    }

    #[test]
    fn live_checks_judge_the_turn_summary() {
        assert_eq!(
            prompt_check(&turn(Ok("end_turn"), 2, &[]), "end_turn").verdict,
            Verdict::Pass
        );
        assert_eq!(
            prompt_check(&turn(Ok("end_turn"), 0, &[]), "end_turn").verdict,
            Verdict::Fail
        );
        assert_eq!(
            prompt_check(&turn(Ok("cancelled"), 2, &[]), "end_turn").verdict,
            Verdict::Fail
        );
        let setup = PermissionSetup {
            label: "a hook".into(),
            no_request: "no request".into(),
        };
        let permission =
            |kinds| permission_check(&turn(Ok("end_turn"), 1, kinds), "end_turn", &setup);
        assert_eq!(permission(&["allow_once"]).verdict, Verdict::Pass);
        assert_eq!(permission(&["reject_once"]).verdict, Verdict::Fail);
        let missing = permission(&[]);
        assert_eq!(missing.verdict, Verdict::Fail);
        assert_eq!(missing.detail, "no request (setup: a hook)");
        assert!(permission(&["allow_once"])
            .detail
            .ends_with(" (setup: a hook)"));
    }

    #[test]
    fn the_cursor_hook_lives_in_the_project_folder_cursor_acp_reads() {
        let project = |cwd: &str| cursor_verify_project(Path::new("/h"), Path::new(cwd));
        assert_eq!(
            project("/tmp/acp-spike/ws"),
            Path::new("/h/.cursor/projects/tmp-acp-spike-ws")
        );
        assert_eq!(
            project("/private/tmp/smoke-568c/repo/.worktrees/smk2"),
            Path::new("/h/.cursor/projects/private-tmp-smoke-568c-repo-worktrees-smk2")
        );
        assert_eq!(
            project("/private/tmp/ralph-smoke-626/repo/.worktrees/echo-sleep"),
            Path::new("/h/.cursor/projects/private-tmp-ralph-smoke-626-repo-worktrees-echo-sleep")
        );
        assert_eq!(
            project("/var/folders/g9/x/T/.tmpiqjUi0"),
            Path::new("/h/.cursor/projects/var-folders-g9-x-T-tmpiqjUi0")
        );
    }

    #[test]
    fn cursor_verify_removes_the_hashed_folder_cursor_makes_for_the_real_path() {
        let home = Path::new("/Users/guzmanmonne");
        let dir = Path::new("/var/folders/g9/4b8tv1yj0tnfsp21spqtx5zr0000gp/T/.tmpX4HBM2");
        let real = Path::new("/private/var/folders/g9/4b8tv1yj0tnfsp21spqtx5zr0000gp/T/.tmpX4HBM2");
        assert_eq!(
            cursor_verify_leftovers(home, dir, real),
            vec![
                PathBuf::from("/Users/guzmanmonne/.cursor/projects/var-folders-g9-4b8tv1yj0tnfsp21spqtx5zr0000gp-T-60693f2"),
                PathBuf::from("/Users/guzmanmonne/.cursor/projects/private-var-folders-g9-4b8tv1yj0tnfsp21spqtx5zr0000gp-T-tmpX4HBM2"),
                PathBuf::from("/Users/guzmanmonne/.cursor/projects/private-var-folders-g9-4b8tv1yj0tnfsp21spqtx5zr-bc092f5"),
            ]
        );
        let short = Path::new("/tmp/x");
        assert_eq!(
            cursor_verify_leftovers(home, short, short),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn cursor_verify_removes_every_folder_the_print_turn_makes() {
        let home = Path::new("/Users/guzmanmonne");
        let dir = Path::new("/var/folders/g9/4b8tv1yj0tnfsp21spqtx5zr0000gp/T/.tmpdLJVd5");
        let real = Path::new("/private/var/folders/g9/4b8tv1yj0tnfsp21spqtx5zr0000gp/T/.tmpdLJVd5");
        let projects = "/Users/guzmanmonne/.cursor/projects";
        assert_eq!(
            cursor_verify_leftovers(home, dir, real),
            vec![
                PathBuf::from(format!(
                    "{projects}/var-folders-g9-4b8tv1yj0tnfsp21spqtx5zr0000gp-T-0323e50"
                )),
                PathBuf::from(format!(
                    "{projects}/private-var-folders-g9-4b8tv1yj0tnfsp21spqtx5zr0000gp-T-tmpdLJVd5"
                )),
                PathBuf::from(format!(
                    "{projects}/private-var-folders-g9-4b8tv1yj0tnfsp21spqtx5zr-6d07eb7"
                )),
            ]
        );
    }

    #[test]
    fn the_cursor_verify_hook_asks_before_every_shell_command() {
        let hooks: serde_json::Value = serde_json::from_str(CURSOR_VERIFY_HOOKS).unwrap();
        assert_eq!(hooks["version"], 1);
        assert_eq!(
            hooks["hooks"]["beforeShellExecution"][0]["command"],
            r#"echo '{"permission":"ask"}'"#
        );
    }

    #[test]
    fn cancel_is_advisory_and_unknown_when_it_cannot_be_judged() {
        let verdict = |turn| cancel_check(&turn, "cancelled");
        assert_eq!(
            verdict(turn(Ok("cancelled"), 1, &[])).verdict,
            Verdict::Pass
        );
        assert_eq!(
            verdict(turn(Ok("end_turn"), 1, &[])).verdict,
            Verdict::Unknown
        );
        assert_eq!(
            verdict(turn(Ok("end_turn"), 0, &[])).verdict,
            Verdict::Unknown
        );
        let failed = verdict(turn(Err("boom"), 1, &[]));
        assert_eq!(failed.verdict, Verdict::Fail);
        assert!(!failed.blocking);
    }

    #[test]
    fn check_lines_show_the_verdict_and_the_advisory_note() {
        assert_eq!(
            render_check(&CheckResult::pass("version", "2026.10.22")),
            "PASS    version: 2026.10.22"
        );
        assert_eq!(
            render_check(&CheckResult::advisory(
                "cancel",
                Verdict::Unknown,
                "no turn"
            )),
            "UNKNOWN cancel (does not fail the spec): no turn"
        );
    }
}
#[test]
fn agy_requires_discovery_and_reload() {
    let version = [CheckResult::pass("version", "1.5.12")];
    assert!(!spec_passed(&version, Some("1.5.12"), AGY_REQUIRED_CHECKS));
    let initial = [
        CheckResult::pass("version", "1.5.12"),
        CheckResult::pass("initialize", "ok"),
        CheckResult::pass("session/new", "ok"),
        CheckResult::pass("mcp/new", "ok"),
        CheckResult::pass("session/close", "ok"),
    ];
    assert!(!spec_passed(&initial, Some("1.5.12"), AGY_REQUIRED_CHECKS));
}
#[test]
fn agy_free_verify_requires_model_and_effort_readback() {
    let protocol = [
        "version",
        "initialize",
        "session/new",
        "mcp/new",
        "session/close",
        "session/load",
        "mcp/load",
        "session/close reload",
    ]
    .map(|name| CheckResult::pass(name, "ok"));
    assert!(!spec_passed(&protocol, Some("1.5.12"), AGY_REQUIRED_CHECKS));
}
