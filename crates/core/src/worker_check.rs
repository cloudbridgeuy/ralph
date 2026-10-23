use crate::chat_memory::{Entry, Role};
use crate::worker::{startup_state, HarnessId, Report, ReportStatus, Startup};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

pub const CHECK_STATUS: &str = "check";
pub const NUDGE_TEXT: &str =
    "You stopped without calling report. Call the report tool now: done, blocked or question.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerObservation {
    pub id: String,
    pub session: String,
    pub kind: HarnessId,
    pub ready: bool,
    pub blocked: bool,
    pub waiting: bool,
    pub unreported: bool,
    pub reported_since_nudge: bool,
    pub pane: Option<String>,
    pub pane_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    Working,
    Waiting,
    BlockedPermission,
    BlockedTrust,
    Unknown,
    Reported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckAction {
    Nudge(String),
    Inbox(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckMemory {
    pub workers: BTreeMap<String, WorkerMemory>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerMemory {
    pub state: WorkerState,
    pub nudged: bool,
    pub unknown_posted: bool,
}

pub fn last_reports(reports: &[Report]) -> HashMap<String, String> {
    reports
        .iter()
        .filter(|report| {
            ReportStatus::ALL
                .iter()
                .any(|status| status.as_str() == report.status)
        })
        .map(|report| (report.worker.clone(), report.at.clone()))
        .collect()
}

pub fn logged_reports(entries: &[Entry]) -> HashMap<String, String> {
    let mut reports = HashMap::new();
    for entry in entries.iter().filter(|entry| entry.role == Role::Worker) {
        for id in entry.text.lines().filter_map(report_header_id) {
            reports.insert(id.to_string(), entry.at.clone());
        }
    }
    reports
}

fn report_header_id(line: &str) -> Option<&str> {
    let inner = line.strip_prefix("[worker ")?.strip_suffix(']')?;
    let (id, status) = inner.split_once(" · ")?;
    ReportStatus::ALL
        .iter()
        .any(|known| known.as_str() == status)
        .then_some(id)
}

pub fn reported_since_nudge(last_report: Option<&String>, nudge: Option<&Option<String>>) -> bool {
    match nudge {
        None => last_report.is_some(),
        Some(before) => last_report.is_some() && last_report != before.as_ref(),
    }
}

pub fn check_every_polls(secs: u64, poll: Duration) -> u64 {
    let poll_ms = u64::try_from(poll.as_millis()).unwrap_or(u64::MAX).max(1);
    secs.saturating_mul(1000).div_ceil(poll_ms).max(1)
}

fn pane_footer(pane: &str) -> String {
    let lines: Vec<&str> = pane.lines().collect();
    match lines.iter().rposition(|l| l.starts_with('─')) {
        Some(rule) => lines[rule + 1..].join("\n"),
        None => lines
            .iter()
            .rfind(|l| !l.trim().is_empty())
            .map(|l| (*l).to_string())
            .unwrap_or_default(),
    }
}

fn counts_running_agents(footer: &str) -> bool {
    footer.split("← ").skip(1).any(|rest| {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let after = &rest[digits.len()..];
        digits.parse::<u64>().is_ok_and(|n| n >= 1) && after.starts_with(" agent")
    })
}

fn pane_busy(pane: &str) -> bool {
    let footer = pane_footer(pane);
    footer.contains("esc to interrupt") || counts_running_agents(&footer)
}

pub fn classify(obs: &WorkerObservation) -> WorkerState {
    if obs.blocked {
        return WorkerState::BlockedPermission;
    }
    if let Some(pane) = &obs.pane {
        if startup_state(pane, obs.ready) == Startup::TrustPrompt {
            return WorkerState::BlockedTrust;
        }
    }
    if obs.waiting {
        WorkerState::Waiting
    } else if obs.reported_since_nudge {
        WorkerState::Reported
    } else if !obs.pane_changed
        && !obs.unreported
        && obs.pane.as_deref().is_some_and(|pane| !pane_busy(pane))
    {
        WorkerState::Unknown
    } else {
        WorkerState::Working
    }
}

fn inbox_text(state: WorkerState, obs: &WorkerObservation) -> Option<String> {
    let id = &obs.id;
    let session = &obs.session;
    if matches!(
        obs.kind,
        HarnessId::Cursor | HarnessId::Opencode | HarnessId::Codex
    ) {
        return match state {
            WorkerState::Waiting => Some(format!("Worker {id} stopped without calling report.")),
            WorkerState::BlockedPermission if obs.kind == HarnessId::Opencode => Some(format!(
                "Worker {id} is waiting for a permission answer. Answer it with reply and an optionId."
            )),
            _ => None,
        };
    }
    match state {
        WorkerState::Waiting => Some(format!(
            "Worker {id} stopped without calling report. To look: tmux attach -t {session}"
        )),
        WorkerState::BlockedPermission => Some(format!(
            "Worker {id} is waiting at a permission prompt; ralph does not type into it. To answer: tmux attach -t {session}"
        )),
        WorkerState::BlockedTrust => Some(format!(
            "Worker {id} is waiting at the folder-trust dialog. To answer: tmux attach -t {session}"
        )),
        WorkerState::Unknown => Some(format!(
            "ralph can't tell what worker {id} is waiting on: its pane has not changed and it has not reported. To look: tmux attach -t {session}"
        )),
        WorkerState::Working | WorkerState::Reported => None,
    }
}

pub fn plan(
    prev: &CheckMemory,
    observations: &[WorkerObservation],
) -> (Vec<CheckAction>, CheckMemory) {
    let mut actions = Vec::new();
    let mut next = CheckMemory::default();
    for obs in observations {
        let state = classify(obs);
        let prev_entry = prev.workers.get(&obs.id);
        let mut nudged = prev_entry.is_some_and(|p| p.nudged) && !obs.reported_since_nudge;
        if state == WorkerState::Waiting && !nudged {
            actions.push(CheckAction::Nudge(obs.id.clone()));
            nudged = true;
        }
        let was_unknown_posted = prev_entry.is_some_and(|p| p.unknown_posted);
        let repeat_unknown = state == WorkerState::Unknown && was_unknown_posted;
        if prev_entry.map(|p| p.state) != Some(state) && !repeat_unknown {
            if let Some(text) = inbox_text(state, obs) {
                actions.push(CheckAction::Inbox(obs.id.clone(), text));
            }
        }
        let unknown_posted = match state {
            WorkerState::Unknown => true,
            WorkerState::Working => was_unknown_posted,
            _ => false,
        };
        next.workers.insert(
            obs.id.clone(),
            WorkerMemory {
                state,
                nudged,
                unknown_posted,
            },
        );
    }
    (actions, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs() -> WorkerObservation {
        WorkerObservation {
            id: "w1".to_string(),
            session: "sess".to_string(),
            kind: HarnessId::Claude,
            ready: true,
            blocked: false,
            waiting: false,
            unreported: false,
            reported_since_nudge: false,
            pane: None,
            pane_changed: true,
        }
    }

    fn waiting() -> WorkerObservation {
        WorkerObservation {
            waiting: true,
            ..obs()
        }
    }

    fn trust_pane() -> Option<String> {
        Some("Yes, I trust this folder".to_string())
    }

    const RULE: &str = "─────────────────────────";

    fn pane_with_footer(above: &str, footer: &str) -> String {
        format!("{above}\n\n{RULE}\n❯\u{a0}\n{RULE}\n{footer}\n")
    }

    const SPINNER_PANE: &str = "✽ Zigzagging… (43s · ↓ 2.9k tokens · thought for 1s)\n\n─────────────────────────\n❯\u{a0}\n─────────────────────────\n  ⏵⏵ auto mode on (shift+tab to cycle) · esc to interrupt · ← for agents\n";
    const IDLE_DONE_PANE: &str = "⏺ I sent the report and I am waiting for your answer.\n\n✻ Crunched for 16m 50s · done 12:36 PM\n\n─────────────────────────\n❯\u{a0}\n─────────────────────────\n  ⏵⏵ auto mode on (shift+tab to cycle) · PR #2772 · ← for agents\n";
    const ONE_AGENT_FOOTER: &str = "  ⏵⏵ auto mode on (shift+tab to cycle) · ← 1 agent";
    const TWO_AGENTS_FOOTER: &str = "  ⏵⏵ auto mode on (shift+tab to cycle) · ← 2 agents";
    const IDLE_FOOTER: &str = "  ⏵⏵ auto mode on (shift+tab to cycle) · ← for agents";
    const TRANSCRIPT_MARKERS: &str = "esc to interrupt\n  ← 3 agents\n⏺ Done.\n✻ Crunched for 1m";

    fn unchanged(pane: &str) -> WorkerObservation {
        WorkerObservation {
            pane: Some(pane.to_string()),
            pane_changed: false,
            ..obs()
        }
    }

    #[test]
    fn pane_busy_on_spinner() {
        assert!(pane_busy(SPINNER_PANE));
    }

    #[test]
    fn pane_busy_on_background_agents() {
        assert!(pane_busy(&pane_with_footer("⏺ ok", ONE_AGENT_FOOTER)));
        assert!(pane_busy(&pane_with_footer("⏺ ok", TWO_AGENTS_FOOTER)));
    }

    #[test]
    fn pane_not_busy_when_idle() {
        assert!(!pane_busy(&pane_with_footer("", IDLE_FOOTER)));
        assert!(!pane_busy(IDLE_DONE_PANE));
        assert!(!pane_busy(&pane_with_footer(
            "⏺ ok",
            "  ⏵⏵ auto mode on · ← 0 agents"
        )));
    }

    #[test]
    fn pane_busy_ignores_transcript_above_the_rule() {
        assert!(!pane_busy(&pane_with_footer(
            TRANSCRIPT_MARKERS,
            IDLE_FOOTER
        )));
    }

    #[test]
    fn pane_busy_without_rule_reads_last_non_blank_line() {
        assert!(pane_busy("old\n  ← 2 agents\n\n"));
        assert!(!pane_busy("esc to interrupt\nidle line\n\n"));
        assert!(!pane_busy(""));
    }

    #[test]
    fn classify_unchanged_busy_panes_are_working() {
        for pane in [
            SPINNER_PANE.to_string(),
            pane_with_footer("⏺ ok", ONE_AGENT_FOOTER),
            pane_with_footer("⏺ ok", TWO_AGENTS_FOOTER),
        ] {
            assert_eq!(classify(&unchanged(&pane)), WorkerState::Working);
        }
    }

    #[test]
    fn classify_unchanged_idle_pane_is_unknown() {
        assert_eq!(classify(&unchanged(IDLE_DONE_PANE)), WorkerState::Unknown);
    }

    #[test]
    fn idle_unchanged_pane_posts_once_and_busy_pane_never_posts() {
        let idle = unchanged(IDLE_DONE_PANE);
        let busy = unchanged(SPINNER_PANE);
        let mut memory = CheckMemory::default();
        let mut idle_posts = 0;
        for _ in 0..3 {
            let (actions, next) = plan(&memory, std::slice::from_ref(&idle));
            idle_posts += inboxes(&actions);
            memory = next;
        }
        assert_eq!(idle_posts, 1);
        let mut memory = CheckMemory::default();
        for _ in 0..3 {
            let (actions, next) = plan(&memory, std::slice::from_ref(&busy));
            assert!(actions.is_empty());
            memory = next;
        }
    }

    fn nudges(actions: &[CheckAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, CheckAction::Nudge(_)))
            .count()
    }

    fn inboxes(actions: &[CheckAction]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, CheckAction::Inbox(..)))
            .count()
    }

    #[test]
    fn classify_blocked_beats_waiting() {
        let o = WorkerObservation {
            blocked: true,
            waiting: true,
            ..obs()
        };
        assert_eq!(classify(&o), WorkerState::BlockedPermission);
    }

    #[test]
    fn classify_trust_only_when_not_ready() {
        let not_ready = WorkerObservation {
            ready: false,
            pane: trust_pane(),
            ..obs()
        };
        assert_eq!(classify(&not_ready), WorkerState::BlockedTrust);
        let ready = WorkerObservation {
            ready: true,
            ..not_ready
        };
        assert_ne!(classify(&ready), WorkerState::BlockedTrust);
    }

    #[test]
    fn classify_trust_beats_waiting() {
        let o = WorkerObservation {
            ready: false,
            waiting: true,
            pane: trust_pane(),
            ..obs()
        };
        assert_eq!(classify(&o), WorkerState::BlockedTrust);
    }

    #[test]
    fn classify_waiting_beats_reported() {
        let o = WorkerObservation {
            reported_since_nudge: true,
            ..waiting()
        };
        assert_eq!(classify(&o), WorkerState::Waiting);
    }

    #[test]
    fn classify_reported_beats_unknown() {
        let o = WorkerObservation {
            reported_since_nudge: true,
            pane: Some("x".to_string()),
            pane_changed: false,
            ..obs()
        };
        assert_eq!(classify(&o), WorkerState::Reported);
    }

    #[test]
    fn classify_unknown_needs_pane_and_no_change() {
        let unchanged = WorkerObservation {
            pane: Some("x".to_string()),
            pane_changed: false,
            ..obs()
        };
        assert_eq!(classify(&unchanged), WorkerState::Unknown);
        let no_pane = WorkerObservation {
            pane: None,
            pane_changed: false,
            ..obs()
        };
        assert_eq!(classify(&no_pane), WorkerState::Working);
        let changed = WorkerObservation {
            pane_changed: true,
            ..unchanged
        };
        assert_eq!(classify(&changed), WorkerState::Working);
    }

    #[test]
    fn classify_unchanged_pane_is_working_while_waiting_is_pending() {
        let o = WorkerObservation {
            unreported: true,
            pane: Some("x".to_string()),
            pane_changed: false,
            ..obs()
        };
        assert_eq!(classify(&o), WorkerState::Working);
        let (actions, _) = plan(&CheckMemory::default(), &[o]);
        assert!(actions.is_empty());
    }

    #[test]
    fn unknown_posts_once_until_the_wait_ends() {
        let unknown = WorkerObservation {
            pane: Some("x".to_string()),
            pane_changed: false,
            ..obs()
        };
        let reported = WorkerObservation {
            reported_since_nudge: true,
            ..obs()
        };
        let mut memory = CheckMemory::default();
        let mut posts = Vec::new();
        for o in [
            unknown.clone(),
            obs(),
            unknown.clone(),
            obs(),
            unknown.clone(),
            reported,
            unknown,
        ] {
            let (actions, next) = plan(&memory, &[o]);
            posts.push(inboxes(&actions));
            memory = next;
        }
        assert_eq!(posts, vec![1, 0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn first_waiting_nudges_then_informs() {
        let (actions, memory) = plan(&CheckMemory::default(), &[waiting()]);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0], CheckAction::Nudge("w1".to_string()));
        assert!(matches!(&actions[1], CheckAction::Inbox(id, _) if id == "w1"));
        assert!(memory.workers["w1"].nudged);
    }

    #[test]
    fn waiting_again_does_nothing() {
        let (_, memory) = plan(&CheckMemory::default(), &[waiting()]);
        let (actions, _) = plan(&memory, &[waiting()]);
        assert!(actions.is_empty());
    }

    #[test]
    fn waiting_working_waiting_informs_without_nudge() {
        let (_, m1) = plan(&CheckMemory::default(), &[waiting()]);
        let (_, m2) = plan(&m1, &[obs()]);
        let (actions, _) = plan(&m2, &[waiting()]);
        assert_eq!(nudges(&actions), 0);
        assert_eq!(inboxes(&actions), 1);
    }

    #[test]
    fn report_resets_nudge() {
        let (_, m1) = plan(&CheckMemory::default(), &[waiting()]);
        let reported = WorkerObservation {
            reported_since_nudge: true,
            ..obs()
        };
        let (_, m2) = plan(&m1, &[reported]);
        assert!(!m2.workers["w1"].nudged);
        let (actions, _) = plan(&m2, &[waiting()]);
        assert_eq!(nudges(&actions), 1);
    }

    #[test]
    fn lingering_waiting_marker_nudges_once_per_new_report() {
        let reported_waiting = WorkerObservation {
            reported_since_nudge: true,
            ..waiting()
        };
        let (first, mut memory) = plan(&CheckMemory::default(), &[waiting()]);
        let mut total = nudges(&first);
        for _ in 0..5 {
            for _ in 0..3 {
                let (actions, next) = plan(&memory, &[waiting()]);
                assert_eq!(nudges(&actions), 0);
                memory = next;
            }
            let (actions, next) = plan(&memory, std::slice::from_ref(&reported_waiting));
            total += nudges(&actions);
            memory = next;
        }
        assert_eq!(total, 1 + 5);
        assert!(memory.workers["w1"].nudged);
    }

    #[test]
    fn second_wait_after_a_report_nudges_without_a_check_between() {
        let (first, memory) = plan(&CheckMemory::default(), &[waiting()]);
        assert_eq!(nudges(&first), 1);
        let reported_waiting = WorkerObservation {
            reported_since_nudge: true,
            ..waiting()
        };
        let (actions, _) = plan(&memory, &[reported_waiting]);
        assert_eq!(nudges(&actions), 1);
    }

    #[test]
    fn cleared_marker_after_a_nudge_does_not_loop() {
        let reported = WorkerObservation {
            reported_since_nudge: true,
            ..obs()
        };
        let (first, mut memory) = plan(&CheckMemory::default(), &[waiting()]);
        let mut total = nudges(&first);
        for o in [
            obs(),
            reported.clone(),
            reported.clone(),
            reported.clone(),
            reported,
        ] {
            let (actions, next) = plan(&memory, &[o]);
            total += nudges(&actions);
            memory = next;
        }
        assert_eq!(total, 1);
        assert_eq!(memory.workers["w1"].state, WorkerState::Reported);

        let fresh_wait = WorkerObservation {
            reported_since_nudge: true,
            ..waiting()
        };
        let (actions, next) = plan(&memory, &[fresh_wait]);
        assert_eq!(nudges(&actions), 1);
        memory = next;
        for _ in 0..3 {
            let (actions, next) = plan(&memory, &[waiting()]);
            assert_eq!(nudges(&actions), 0);
            memory = next;
        }
    }

    #[test]
    fn blocked_and_unknown_never_nudge() {
        let blocked = WorkerObservation {
            blocked: true,
            ..obs()
        };
        let trust = WorkerObservation {
            ready: false,
            pane: trust_pane(),
            ..obs()
        };
        let unknown = WorkerObservation {
            pane: Some("x".to_string()),
            pane_changed: false,
            ..obs()
        };
        for o in [blocked, trust, unknown] {
            let (actions, _) = plan(&CheckMemory::default(), &[o]);
            assert_eq!(nudges(&actions), 0);
            assert_eq!(inboxes(&actions), 1);
        }
    }

    #[test]
    fn one_inbox_per_state_change() {
        let blocked = WorkerObservation {
            blocked: true,
            ..obs()
        };
        let (first, memory) = plan(&CheckMemory::default(), std::slice::from_ref(&blocked));
        let (second, _) = plan(&memory, &[blocked]);
        assert_eq!(inboxes(&first), 1);
        assert!(second.is_empty());
    }

    #[test]
    fn a_cursor_permission_block_leaves_the_alert_to_the_worker() {
        let blocked = WorkerObservation {
            blocked: true,
            kind: HarnessId::Cursor,
            ..obs()
        };
        let (actions, _) = plan(&CheckMemory::default(), &[blocked]);
        assert_eq!(inboxes(&actions), 0);
    }

    #[test]
    fn working_and_reported_emit_nothing() {
        let reported = WorkerObservation {
            reported_since_nudge: true,
            ..obs()
        };
        let (actions, _) = plan(&CheckMemory::default(), &[obs(), reported]);
        assert!(actions.is_empty());
    }

    #[test]
    fn inbox_texts_point_at_session() {
        let states = [
            waiting(),
            WorkerObservation {
                blocked: true,
                ..obs()
            },
            WorkerObservation {
                ready: false,
                pane: trust_pane(),
                ..obs()
            },
            WorkerObservation {
                pane: Some("x".to_string()),
                pane_changed: false,
                ..obs()
            },
        ];
        for o in states {
            let (actions, _) = plan(&CheckMemory::default(), &[o]);
            let text = actions
                .iter()
                .find_map(|a| match a {
                    CheckAction::Inbox(_, text) => Some(text.clone()),
                    CheckAction::Nudge(_) => None,
                })
                .unwrap();
            assert!(text.ends_with("tmux attach -t sess"));
        }
    }

    #[test]
    fn unobserved_workers_are_dropped() {
        let other = WorkerObservation {
            id: "w2".to_string(),
            ..obs()
        };
        let (_, memory) = plan(&CheckMemory::default(), &[obs(), other.clone()]);
        assert_eq!(memory.workers.len(), 2);
        let (_, memory) = plan(&memory, &[other]);
        assert_eq!(memory.workers.keys().collect::<Vec<_>>(), vec!["w2"]);
    }

    fn report(worker: &str, at: &str, status: &str) -> Report {
        Report {
            worker: worker.to_string(),
            at: at.to_string(),
            status: status.to_string(),
            text: String::new(),
        }
    }

    #[test]
    fn last_reports_ignore_check_posts() {
        let reports = [
            report("w1", "t1", "done"),
            report("w1", "t2", CHECK_STATUS),
            report("w2", "t3", CHECK_STATUS),
            report("w1", "t4", "question"),
        ];
        let last = last_reports(&reports);
        assert_eq!(last.len(), 1);
        assert_eq!(last["w1"], "t4");
    }

    fn worker_entry(at: &str, text: &str) -> Entry {
        Entry {
            role: Role::Worker,
            at: at.to_string(),
            text: text.to_string(),
            corrupt: false,
        }
    }

    #[test]
    fn logged_reports_maps_a_single_report() {
        let entries = [worker_entry("t1", "[worker w1 · done]\nall good")];
        let reports = logged_reports(&entries);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports["w1"], "t1");
    }

    #[test]
    fn logged_reports_reads_several_reports_in_one_entry() {
        let entries = [worker_entry(
            "t1",
            "[worker a · done]\none\n\n[worker b · question]\ntwo",
        )];
        let reports = logged_reports(&entries);
        assert_eq!(reports["a"], "t1");
        assert_eq!(reports["b"], "t1");
    }

    #[test]
    fn logged_reports_ignores_check_status() {
        let entries = [worker_entry("t1", "[worker w1 · check]\nstuck")];
        assert!(logged_reports(&entries).is_empty());
    }

    #[test]
    fn logged_reports_ignores_other_roles_and_partial_lines() {
        let user = Entry {
            role: Role::User,
            at: "t1".to_string(),
            text: "[worker w1 · done]".to_string(),
            corrupt: false,
        };
        let inline = worker_entry("t2", "see [worker w2 · done] above");
        assert!(logged_reports(&[user, inline]).is_empty());
    }

    #[test]
    fn logged_reports_later_entry_wins() {
        let entries = [
            worker_entry("t1", "[worker w1 · blocked]\nx"),
            worker_entry("t2", "[worker w1 · done]\ny"),
        ];
        assert_eq!(logged_reports(&entries)["w1"], "t2");
    }

    #[test]
    fn restart_with_logged_done_report_stays_reported_and_silent() {
        let reports = logged_reports(&[worker_entry("t1", "[worker w1 · done]\nfinished")]);
        let observation = WorkerObservation {
            reported_since_nudge: reported_since_nudge(reports.get("w1"), None),
            pane: Some("idle".to_string()),
            pane_changed: false,
            ..obs()
        };
        assert_eq!(classify(&observation), WorkerState::Reported);
        let (actions, _) = plan(&CheckMemory::default(), &[observation]);
        assert_eq!(inboxes(&actions), 0);
        assert_eq!(nudges(&actions), 0);
    }

    #[test]
    fn reported_since_nudge_needs_a_newer_report() {
        let t1 = "t1".to_string();
        let t2 = "t2".to_string();
        assert!(!reported_since_nudge(None, None));
        assert!(reported_since_nudge(Some(&t1), None));
        assert!(!reported_since_nudge(None, Some(&None)));
        assert!(reported_since_nudge(Some(&t1), Some(&None)));
        assert!(!reported_since_nudge(Some(&t1), Some(&Some(t1.clone()))));
        assert!(reported_since_nudge(Some(&t2), Some(&Some(t1))));
    }

    #[test]
    fn check_every_polls_rounds_up_to_at_least_one() {
        let poll = Duration::from_millis(500);
        assert_eq!(check_every_polls(60, poll), 120);
        assert_eq!(check_every_polls(1, Duration::from_millis(700)), 2);
        assert_eq!(check_every_polls(0, poll), 1);
    }

    #[test]
    fn headless_workers_without_a_pane_are_working_and_texts_skip_tmux() {
        for kind in [HarnessId::Cursor, HarnessId::Opencode] {
            let headless = WorkerObservation {
                kind,
                pane: None,
                ..obs()
            };
            assert_eq!(classify(&headless), WorkerState::Working);
            for observation in [
                WorkerObservation {
                    waiting: true,
                    ..headless.clone()
                },
                WorkerObservation {
                    blocked: true,
                    ..headless
                },
            ] {
                let state = classify(&observation);
                let text = inbox_text(state, &observation).unwrap_or_default();
                assert!(!text.contains("tmux"));
            }
        }
    }

    #[test]
    fn pane_footer_is_everything_below_the_last_rule() {
        let pane = pane_with_footer("work", "footer");
        assert_eq!(pane_footer(&pane), "footer");
    }

    #[test]
    fn headless_harnesses_get_short_inbox_texts() {
        let on = |kind| WorkerObservation { kind, ..obs() };
        assert_eq!(
            inbox_text(WorkerState::Waiting, &on(HarnessId::Cursor)),
            Some("Worker w1 stopped without calling report.".to_string())
        );
        assert_eq!(
            inbox_text(WorkerState::Waiting, &on(HarnessId::Opencode)),
            Some("Worker w1 stopped without calling report.".to_string())
        );
        assert_eq!(
            inbox_text(WorkerState::BlockedPermission, &on(HarnessId::Opencode)),
            Some(
                "Worker w1 is waiting for a permission answer. Answer it with reply and an optionId."
                    .to_string()
            )
        );
        assert_eq!(
            inbox_text(WorkerState::BlockedPermission, &on(HarnessId::Cursor)),
            None
        );
    }
}
