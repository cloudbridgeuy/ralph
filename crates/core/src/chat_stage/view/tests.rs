use super::*;
use crate::chat_local::turn_texts;
use crate::chat_stage::{render, Kind};
use crate::chat_status::WorkerActivity;
use crate::worker::{ReportKind, ReportStatus};
use crate::workers_view::{lines_text, Span, Tone};
use unicode_width::UnicodeWidthStr;

const MODEL_TEXT: &str = "Stage:
thread stage-output: unresolved, end with /thread merge or drop
s1 decision GUZ-1915: pick the /stage mockup to build
s2 owed send the resize redraw note to the reviewer and link the before and after screenshots in the ticket
s3 followup check the decay report after the next nap
needs you: GUZ-1915 question, GUZ-1748 permission
workers: GUZ-1912, GUZ-1890
watches: w1, w2
schedule: sc1 09:00
inbox: 2 unread";

const LOG_LEN: usize = 42;

fn item(id: &str, kind: Kind, turn: usize, text: &str) -> Item {
    Item {
        id: id.to_string(),
        kind,
        reference: None,
        text: text.to_string(),
        created: "2026-11-26T10:00:00+00:00".to_string(),
        turn,
    }
}

fn sample_items() -> Vec<Item> {
    let mut decision = item("s1", Kind::Decision, 4, "pick the /stage mockup to build");
    decision.reference = Some("GUZ-1915".to_string());
    vec![
        decision,
        item(
            "s2",
            Kind::Owed,
            9,
            "send the resize redraw note to the reviewer and link the before and after screenshots in the ticket",
        ),
        item("s3", Kind::Followup, 14, "check the decay report after the next nap"),
    ]
}

fn worker(id: &str, state: WorkerActivity) -> WorkerState {
    WorkerState {
        id: id.to_string(),
        state,
    }
}

fn sample_status() -> Status {
    Status {
        unread: 2,
        workers: vec![
            worker(
                "GUZ-1915",
                WorkerActivity::Reported(ReportKind::Worker(ReportStatus::Question)),
            ),
            worker("GUZ-1748", WorkerActivity::Permission),
            worker("GUZ-1912", WorkerActivity::Busy),
            worker("GUZ-1890", WorkerActivity::Idle),
        ],
        watches: vec![
            WatchState {
                id: "w1".to_string(),
                left: Some("12m".to_string()),
            },
            WatchState {
                id: "w2".to_string(),
                left: None,
            },
        ],
        schedule: vec![ScheduleState {
            id: "sc1".to_string(),
            time: "09:00".to_string(),
        }],
        ..Status::default()
    }
}

fn sample_thread() -> ThreadName {
    ThreadName::parse("stage-output").unwrap()
}

fn sample_view() -> StageView {
    let (items, status, thread) = (sample_items(), sample_status(), sample_thread());
    stage_view(&StageInput {
        items: &items,
        status: &status,
        thread: Some(&thread),
        log_len: LOG_LEN,
    })
    .unwrap()
}

fn view_of(items: &[Item], status: &Status) -> Option<StageView> {
    stage_view(&StageInput {
        items,
        status,
        thread: None,
        log_len: LOG_LEN,
    })
}

#[test]
fn the_model_text_is_unchanged_by_the_view() {
    let (items, status, thread) = (sample_items(), sample_status(), sample_thread());
    assert_eq!(render(&items, &status, Some(&thread)), MODEL_TEXT);
    sample_view();
    assert_eq!(render(&items, &status, Some(&thread)), MODEL_TEXT);
}

#[test]
fn turn_texts_carry_the_stage_text_not_the_view() {
    let (items, status, thread) = (sample_items(), sample_status(), sample_thread());
    let rendered = render(&items, &status, Some(&thread));
    let live = turn_texts(&[], &rendered, "hello", &[]).live;
    assert_eq!(live, format!("{MODEL_TEXT}\n\nNew message:\nhello"));
    assert!(!live.contains(['◆', '▲', '●', '○', '\u{1b}']), "{live}");
}

#[test]
fn a_decision_and_needing_workers_go_under_needs_you() {
    let needs: Vec<String> = sample_view()
        .needs
        .iter()
        .map(|need| match need {
            Need::Decision(row) => format!("decision {}", row.id),
            Need::Worker(worker) => format!("worker {} {}", worker.id, worker.state),
        })
        .collect();
    assert_eq!(
        needs,
        [
            "decision s1",
            "worker GUZ-1915 question",
            "worker GUZ-1748 permission"
        ]
    );
}

#[test]
fn a_decision_row_keeps_its_ref_and_text() {
    let view = sample_view();
    let Some(Need::Decision(row)) = view.needs.first() else {
        panic!("{:?}", view.needs);
    };
    assert_eq!(
        row.reference.as_ref().map(TicketRef::to_string),
        Some("GUZ-1915".to_string())
    );
    assert_eq!(row.text, "pick the /stage mockup to build");
}

#[test]
fn owed_and_followup_go_under_open_in_stage_order() {
    let open: Vec<(OpenKind, String, usize)> = sample_view()
        .open
        .iter()
        .map(|open| (open.kind, open.row.id.to_string(), open.turns_left))
        .collect();
    assert_eq!(
        open,
        [
            (OpenKind::Owed, "s2".to_string(), 14),
            (OpenKind::Followup, "s3".to_string(), 16)
        ]
    );
}

#[test]
fn turns_left_counts_half_the_log_rows_to_the_expiry_rounded_up() {
    let added = item("s1", Kind::Owed, 10, "x");
    assert_eq!(turns_left(&added, 10), 30);
    assert_eq!(turns_left(&added, 11), 30);
    assert_eq!(turns_left(&added, 12), 29);
    assert_eq!(turns_left(&added, 69), 1);
    assert_eq!(turns_left(&added, 70), 0);
    assert_eq!(turns_left(&added, 500), 0);
    assert_eq!(turns_left(&added, 3), 30);
}

#[test]
fn running_facts_keep_the_model_order_and_leave_out_empty_lines() {
    assert_eq!(
        sample_view().running,
        [
            Fact::Workers(vec![
                worker("GUZ-1912", WorkerActivity::Busy),
                worker("GUZ-1890", WorkerActivity::Idle)
            ]),
            Fact::Watches(sample_status().watches),
            Fact::Schedule(sample_status().schedule),
            Fact::Inbox(2)
        ]
    );
    let only_inbox = Status {
        unread: 1,
        ..Status::default()
    };
    assert_eq!(view_of(&[], &only_inbox).unwrap().running, [Fact::Inbox(1)]);
    let only_needing = Status {
        workers: vec![worker("a", WorkerActivity::Permission)],
        ..Status::default()
    };
    assert_eq!(view_of(&[], &only_needing).unwrap().running, []);
}

#[test]
fn an_empty_stage_has_no_view() {
    assert_eq!(view_of(&[], &Status::default()), None);
}

#[test]
fn a_thread_view_opens_with_the_thread_even_when_otherwise_empty() {
    let thread = sample_thread();
    let view = stage_view(&StageInput {
        items: &[],
        status: &Status::default(),
        thread: Some(&thread),
        log_len: 0,
    })
    .unwrap();
    assert_eq!(
        view,
        StageView {
            thread: Some(thread),
            needs: Vec::new(),
            open: Vec::new(),
            running: Vec::new(),
        }
    );
}

const NARROW: &str = "Stage · thread stage-output · unresolved, end with /thread merge or drop

Needs you
  ◆ s1  decision  GUZ-1915: pick the /stage mockup to build
  ▲ GUZ-1915  question
  ▲ GUZ-1748  permission

Open
  ● s2  owed      send the resize redraw note to the reviewer and link the
                  before and after screenshots in the ticket · ~14 turns left
  ○ s3  followup  check the decay report after the next nap · ~16 turns left

Running
  workers   GUZ-1912 busy · GUZ-1890 idle
  watches   w1 12m · w2
  schedule  sc1 09:00
  inbox     2 unread";

fn text_at(view: &StageView, width: usize) -> String {
    lines_text(&view_lines(view, width))
}

fn tone_of(lines: &[Vec<Span>], text: &str) -> Tone {
    lines
        .iter()
        .flatten()
        .find(|span| span.text.trim_end() == text)
        .unwrap_or_else(|| panic!("no span {text:?} in {lines:?}"))
        .tone
}

#[test]
fn view_lines_text_matches_the_mockup() {
    assert_eq!(text_at(&sample_view(), 80), NARROW);
}

#[test]
fn view_lines_fit_the_width_and_wrap_with_a_hanging_indent() {
    for width in [80, 120] {
        let text = text_at(&sample_view(), width);
        assert!(text.lines().all(|line| line.width() <= width), "{text}");
        assert!(!text.contains('…'), "{text}");
        assert!(text.contains("screenshots in the ticket"), "{text}");
    }
    let wide = text_at(&sample_view(), 120);
    assert!(
        wide.contains(&format!(
            "in the ticket\n{}~14 turns left\n",
            " ".repeat(18)
        )),
        "{wide}"
    );
}

#[test]
fn the_thread_tail_moves_to_its_own_line_when_the_title_is_too_wide() {
    let text = text_at(&sample_view(), 71);
    assert!(
        text.starts_with(
            "Stage · thread stage-output\n  unresolved, end with /thread merge or drop\n\nNeeds you\n"
        ),
        "{text}"
    );
    assert!(text_at(&sample_view(), 72).starts_with(NARROW.lines().next().unwrap()));
}

#[test]
fn a_main_stage_has_a_bare_title_and_only_the_sections_with_rows() {
    let status = Status {
        unread: 1,
        ..Status::default()
    };
    assert_eq!(
        text_at(&view_of(&[], &status).unwrap(), 80),
        "Stage\n\nRunning\n  inbox  1 unread"
    );
    let owed = [item("s7", Kind::Owed, LOG_LEN, "ask the reviewer")];
    assert_eq!(
        text_at(&view_of(&owed, &Status::default()).unwrap(), 80),
        "Stage\n\nOpen\n  ● s7  owed      ask the reviewer · ~30 turns left"
    );
}

#[test]
fn the_id_column_fits_the_longest_id() {
    let items = [
        item("s9", Kind::Decision, 0, "first"),
        item("s120", Kind::Followup, LOG_LEN, "second"),
    ];
    let text = text_at(&view_of(&items, &Status::default()).unwrap(), 80);
    assert!(text.contains("  ◆ s9    decision  first\n"), "{text}");
    assert!(text.contains("  ○ s120  followup  second · "), "{text}");
}

#[test]
fn the_tones_follow_the_workers_view() {
    let lines = view_lines(&sample_view(), 80);
    assert_eq!(tone_of(&lines, "Stage"), Tone::Name);
    assert_eq!(tone_of(&lines, "stage-output"), Tone::Name);
    assert_eq!(
        tone_of(&lines, "unresolved, end with /thread merge or drop"),
        Tone::Attention
    );
    assert_eq!(tone_of(&lines, "◆"), Tone::Attention);
    assert_eq!(tone_of(&lines, "decision"), Tone::Attention);
    assert_eq!(tone_of(&lines, "s1"), Tone::Name);
    assert_eq!(tone_of(&lines, "GUZ-1915"), Tone::Active);
    assert_eq!(tone_of(&lines, "▲"), Tone::Attention);
    assert_eq!(tone_of(&lines, "question"), Tone::Attention);
    assert_eq!(tone_of(&lines, "●"), Tone::Active);
    assert_eq!(tone_of(&lines, "owed"), Tone::Plain);
    assert_eq!(tone_of(&lines, "○"), Tone::Dim);
    assert_eq!(tone_of(&lines, "followup"), Tone::Dim);
    assert_eq!(tone_of(&lines, "~14 turns left"), Tone::Dim);
    assert_eq!(tone_of(&lines, "busy"), Tone::Active);
    assert_eq!(tone_of(&lines, "idle"), Tone::Dim);
    assert_eq!(tone_of(&lines, " 12m"), Tone::Dim);
}

#[test]
fn a_blocked_worker_and_a_near_expiry_stand_out() {
    let status = Status {
        workers: vec![worker(
            "w",
            WorkerActivity::Reported(ReportKind::Worker(ReportStatus::Blocked)),
        )],
        ..Status::default()
    };
    let items = [
        item("s1", Kind::Owed, 0, "soon"),
        item("s2", Kind::Owed, 2, "later"),
    ];
    let view = stage_view(&StageInput {
        items: &items,
        status: &status,
        thread: None,
        log_len: 50,
    })
    .unwrap();
    let lines = view_lines(&view, 80);
    assert_eq!(tone_of(&lines, "▲"), Tone::Blocked);
    assert_eq!(tone_of(&lines, "blocked"), Tone::Blocked);
    assert_eq!(tone_of(&lines, "~5 turns left"), Tone::Attention);
    assert_eq!(tone_of(&lines, "~6 turns left"), Tone::Dim);
}
