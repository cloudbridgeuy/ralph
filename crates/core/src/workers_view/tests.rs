use super::*;

const NOW: &str = "2026-01-01T01:00:00Z";

fn workers_view(rows: &[WorkerRow], now: DateTime<Utc>, width: usize) -> View {
    super::workers_view(rows, now, width, Layout::Table, false)
}

pub(super) fn now() -> DateTime<Utc> {
    instant(NOW).unwrap()
}

fn entry(role: Role, at: &str, text: &str) -> Entry {
    Entry {
        role,
        at: at.into(),
        text: text.into(),
        corrupt: false,
    }
}

pub(super) fn line(status: &str, at: &str, text: &str) -> ReportLine {
    ReportLine {
        status: status.into(),
        at: at.into(),
        text: text.into(),
    }
}

pub(super) fn row(id: &str) -> WorkerRow {
    WorkerRow {
        id: id.into(),
        harness: "claude".into(),
        state: Some("busy".into()),
        report: Lookup::Found(line("done", "2026-01-01T00:48:00Z", "")),
        status: None,
        head: Lookup::Found(Head {
            hash: "2f7f8c0".into(),
            subject: "fix(chat): exact /help, width-wrapped help and stricter help tests".into(),
        }),
        diff: Lookup::Found(Diff {
            added: 120,
            deleted: 40,
            files: 3,
            untracked: 2,
        }),
        behind: Lookup::Found(2),
        base: Some("main".into()),
        ticket: None,
        pr: Remote::None,
    }
}

#[test]
fn several_reports_in_one_entry_are_split_newest_first() {
    let text = "[worker A · done]\nfirst\nmore\n\n[worker B · blocked]\nstuck\n\n[worker A · question]\nwhy?";
    let entries = [entry(Role::Worker, "2026-01-01T00:10:00Z", text)];
    let grouped = group_reports(&entries, &[]);
    assert_eq!(
        grouped["A"],
        vec![
            line("question", "2026-01-01T00:10:00Z", "why?"),
            line("done", "2026-01-01T00:10:00Z", "first\nmore"),
        ]
    );
    assert_eq!(grouped["B"].len(), 1);
}

#[test]
fn non_report_statuses_and_other_roles_are_ignored() {
    let text = "[worker A · check]\nquiet\n\n[worker A · watch]\nexit 0";
    let entries = [
        entry(Role::Worker, "2026-01-01T00:10:00Z", text),
        entry(Role::User, "2026-01-01T00:11:00Z", "[worker A · done]\nx"),
    ];
    let unread = [Report {
        worker: "A".into(),
        at: "2026-01-01T00:12:00Z".into(),
        status: "check".into(),
        text: "quiet".into(),
    }];
    assert!(group_reports(&entries, &unread).is_empty());
}

#[test]
fn a_check_header_ends_the_previous_report_text() {
    let text = "[worker A · done]\nreal\n\n[worker A · check]\nnoise";
    let entries = [entry(Role::Worker, "2026-01-01T00:10:00Z", text)];
    assert_eq!(group_reports(&entries, &[])["A"][0].text, "real");
}

#[test]
fn unread_reports_sort_with_logged_ones_by_time() {
    let entries = [entry(
        Role::Worker,
        "2026-01-01T00:30:00Z",
        "[worker A · done]\nold",
    )];
    let unread = [Report {
        worker: "A".into(),
        at: "2026-01-01T00:40:00Z".into(),
        status: "blocked".into(),
        text: "new".into(),
    }];
    let statuses: Vec<_> = group_reports(&entries, &unread)["A"]
        .iter()
        .map(|r| r.status.clone())
        .collect();
    assert_eq!(statuses, ["blocked", "done"]);
}

#[test]
fn numstat_counts_lines_and_files_with_binary_as_zero() {
    let diff = parse_numstat("10\t2\ta.rs\n-\t-\timg.png\n5\t0\tdir/b c.rs\n");
    assert_eq!(
        diff,
        Diff {
            added: 15,
            deleted: 2,
            files: 3,
            untracked: 0
        }
    );
    assert_eq!(parse_numstat(""), Diff::default());
}

#[test]
fn untracked_counts_every_file_of_a_new_directory() {
    assert_eq!(count_untracked("?? d/a\n?? d/b\n?? d/c.rs\n M x"), 3);
}

#[test]
fn untracked_counts_only_question_mark_lines() {
    assert_eq!(count_untracked("?? a\n M b\n?? c d\nA  e"), 2);
    assert_eq!(count_untracked(""), 0);
}

#[test]
fn head_splits_hash_from_subject() {
    assert_eq!(
        parse_head("abc1234 fix: it\n"),
        Some(Head {
            hash: "abc1234".into(),
            subject: "fix: it".into()
        })
    );
    assert_eq!(parse_head("abc1234").unwrap().subject, "");
    assert_eq!(parse_head("  "), None);
}

#[test]
fn cut_uses_display_width() {
    assert_eq!(cut("abcdef", 6), "abcdef");
    assert_eq!(cut("abcdef", 4), "abc…");
    assert_eq!(cut("漢字漢字", 5), "漢字…");
    assert_eq!(cut("abc", 0), "");
}

#[test]
fn empty_workers_print_a_plain_line() {
    for layout in [Layout::Table, Layout::List] {
        assert_eq!(
            super::workers_view(&[], now(), 80, layout, false),
            View::Text(vec![vec![plain(NO_WORKERS)]])
        );
    }
}

#[test]
fn layout_selects_a_table_or_blocks_on_wide_screens() {
    let rows = [row("GUZ-1")];
    assert!(matches!(
        super::workers_view(&rows, now(), 160, Layout::Table, false),
        View::Table(_)
    ));
    assert_eq!(
        list_text(super::workers_view(&rows, now(), 160, Layout::List, false)),
        "● GUZ-1 · claude · busy\n  report  done 12m ago\n  head    2f7f8c0 fix(chat): exact /help, width-wrapped help and stricter help tests\n  diff    +120 -40 in 3 files ?2 · 2 behind origin/main\n  ticket  -\n  pr      -"
    );
}

#[test]
fn table_fallback_matches_list_layout_on_narrow_screens() {
    let rows = [row("GUZ-1")];
    let list = super::workers_view(&rows, now(), 80, Layout::List, false);
    assert!(matches!(list, View::Text(_)));
    assert_eq!(
        super::workers_view(&rows, now(), 80, Layout::Table, false),
        list
    );
}

fn list_text(view: View) -> String {
    match view {
        View::Text(lines) => lines_text(&lines),
        other => panic!("not a list: {other:?}"),
    }
}

fn cell(cells: &[Vec<Span>; COLUMNS], index: usize) -> String {
    spans_text(&cells[index])
}

fn table_of(view: View) -> String {
    match view {
        View::Table(markdown) => markdown,
        other => panic!("not a table: {other:?}"),
    }
}

#[test]
fn wide_screens_get_a_table_with_the_full_subject() {
    let markdown = table_of(workers_view(&[row("GUZ-1")], now(), 160));
    assert_eq!(
        markdown,
        "| ID | HARNESS | STATE | REPORT | HEAD | +/- | BEHIND | TICKET | PR |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n| GUZ-1 | claude | busy | done 12m ago | 2f7f8c0 fix(chat): exact /help, width-wrapped help and stricter help tests | +120 -40 3f ?2 | 2 | `-` | `-` |"
    );
}

#[test]
fn the_subject_is_cut_before_the_table_gives_up() {
    let markdown = table_of(workers_view(&[row("GUZ-1")], now(), 100));
    assert!(markdown.contains("…"), "{markdown}");
    assert!(table_width(&[table_cells(&row("GUZ-1"), now(), 20)]) <= 100);
}

#[test]
fn narrow_screens_get_a_list_cut_to_width() {
    let text = list_text(workers_view(&[row("GUZ-1")], now(), 80));
    assert_eq!(
        text,
        "● GUZ-1 · claude · busy\n  report  done 12m ago\n  head    2f7f8c0 fix(chat): exact /help, width-wrapped help and stricter help …\n  diff    +120 -40 in 3 files ?2 · 2 behind origin/main\n  ticket  -\n  pr      -"
    );
    for width in [10, 30, 80] {
        let text = list_text(workers_view(&[row("GUZ-1")], now(), width));
        assert!(text.lines().all(|l| l.width() <= width), "{text}");
    }
}

#[test]
fn missing_and_failed_values_show_dash_and_question_mark() {
    let mut gone = row("A");
    gone.state = None;
    gone.report = Lookup::Missing;
    gone.head = Lookup::Missing;
    gone.diff = Lookup::Failed;
    gone.behind = Lookup::Failed;
    let cells = table_cells(&gone, now(), 99);
    assert_eq!(cell(&cells, 2), "-");
    assert_eq!(cell(&cells, 3), "-");
    assert_eq!(cell(&cells, 4), "-");
    assert_eq!(cell(&cells, 5), "?");
    assert_eq!(cell(&cells, 6), "?");
    let text = list_text(workers_view(&[gone], now(), 20));
    assert!(text.contains("diff    ? · ?"), "{text}");
}

#[test]
fn behind_text_names_the_remote_tracking_ref() {
    assert_eq!(spans_text(&behind_text(&row("A"))), "2 behind origin/main");
    let mut bare = row("A");
    bare.base = None;
    assert_eq!(spans_text(&behind_text(&bare)), "2 behind");
    bare.behind = Lookup::Failed;
    assert_eq!(spans_text(&behind_text(&bare)), "?");
}

#[test]
fn dash_cells_are_escaped_in_the_markdown() {
    let mut gone = row("A");
    gone.report = Lookup::Missing;
    gone.behind = Lookup::Missing;
    let View::Table(markdown) = workers_view(&[gone], now(), 400) else {
        panic!("not a table");
    };
    assert!(markdown.contains("| `-` | 2f7f8c0"), "{markdown}");
    assert!(markdown.ends_with("| `-` |"), "{markdown}");
}

#[test]
fn untracked_count_is_omitted_at_zero() {
    let mut clean = row("A");
    clean.diff = Lookup::Found(Diff {
        added: 1,
        deleted: 0,
        files: 1,
        untracked: 0,
    });
    assert_eq!(cell(&table_cells(&clean, now(), 99), 5), "+1 -0 1f");
    assert!(spans_text(&row_lines(&clean, now())[2].1).starts_with("+1 -0 in 1 file ·"));
}

#[test]
fn pipes_in_a_subject_do_not_split_table_cells() {
    let mut piped = row("A");
    piped.head = Lookup::Found(Head {
        hash: "abc".into(),
        subject: "a | b".into(),
    });
    assert_eq!(cell(&table_cells(&piped, now(), 99), 4), "abc a ¦ b");
}

pub(super) fn with_status(mut row: WorkerRow) -> WorkerRow {
    row.status = Some(StatusLine {
        at: "2026-01-01T00:55:00Z".into(),
        text: "Reading the tests.".into(),
    });
    row
}

#[test]
fn a_status_replaces_the_report_cell_with_text_and_age() {
    let cells = table_cells(&with_status(row("A")), now(), 99);
    assert_eq!(cell(&cells, 3), "Reading the tests. · 5m ago");
    assert_eq!(
        spans_text(&row_lines(&with_status(row("A")), now())[0].1),
        "Reading the tests. · 5m ago"
    );
}

#[test]
fn a_row_without_status_falls_back_to_the_latest_report() {
    assert_eq!(cell(&table_cells(&row("A"), now(), 99), 3), "done 12m ago");
    assert_eq!(
        spans_text(&row_lines(&row("A"), now())[0].1),
        "done 12m ago"
    );
}

#[test]
fn a_report_newer_than_the_status_is_shown_instead() {
    let mut newer = with_status(row("A"));
    newer.report = Lookup::Found(line("done", "2026-01-01T00:58:00Z", ""));
    assert_eq!(cell(&table_cells(&newer, now(), 99), 3), "done 2m ago");
    assert_eq!(spans_text(&row_lines(&newer, now())[0].1), "done 2m ago");
}

#[test]
fn equal_times_and_unparsable_report_times_keep_the_report() {
    let mut same = with_status(row("A"));
    same.report = Lookup::Found(line("done", "2026-01-01T00:55:00Z", ""));
    assert_eq!(cell(&table_cells(&same, now(), 99), 3), "done 5m ago");
    same.report = Lookup::Found(line("done", "garbage", ""));
    assert!(!is_newer("2026-01-01T00:55:00Z", "garbage"));
    assert_eq!(shown_status(&same), None);
}

#[test]
fn times_compare_across_offsets() {
    assert!(is_newer(
        "2026-01-01T00:55:00Z",
        "2026-01-01T01:50:00+01:00"
    ));
    assert!(!is_newer(
        "2026-01-01T00:55:00Z",
        "2026-01-01T00:40:00-01:00"
    ));
}

#[test]
fn a_status_shows_when_there_is_no_report_to_compare() {
    for report in [Lookup::Missing, Lookup::Failed] {
        let mut r = with_status(row("A"));
        r.report = report;
        assert_eq!(
            cell(&table_cells(&r, now(), 99), 3),
            "Reading the tests. · 5m ago"
        );
    }
}

#[test]
fn a_long_status_is_cut_and_the_table_stays_a_table() {
    let mut long = with_status(row("GUZ-1"));
    let text = "s".repeat(120);
    long.status = Some(StatusLine {
        at: "2026-01-01T00:55:00Z".into(),
        text: text.clone(),
    });
    let markdown = table_of(workers_view(&[long.clone()], now(), 140));
    assert!(markdown.contains("…"), "{markdown}");
    assert!(!markdown.contains(&text));
    assert!(markdown.lines().all(|l| l.width() <= 140), "{markdown}");
    let list = list_text(workers_view(&[long], now(), 40));
    assert!(!list.contains(&text));
}

#[test]
fn a_wide_screen_keeps_a_long_status_whole() {
    let mut long = with_status(row("GUZ-1"));
    let text = "s".repeat(120);
    long.status = Some(StatusLine {
        at: "2026-01-01T00:55:00Z".into(),
        text: text.clone(),
    });
    assert!(table_of(workers_view(&[long], now(), 400)).contains(&text));
}

fn with_remote(mut row: WorkerRow) -> WorkerRow {
    row.ticket = Some(TicketRemote {
        key: row.id.clone(),
        state: Remote::Found("In Progress".into()),
    });
    row.pr = Remote::Found(Pr {
        number: 12,
        state: PrState::Open,
        draft: false,
        review: Review::Approved,
        url: "https://github.com/o/r/pull/12".into(),
    });
    row
}

#[test]
fn the_table_at_160_columns_has_the_ticket_and_pr_cells() {
    let markdown = table_of(workers_view(&[with_remote(row("GUZ-1"))], now(), 160));
    let last = markdown.lines().last().unwrap();
    assert!(
        last.ends_with("| 2 | In Progress | #12 approved |"),
        "{markdown}"
    );
    assert!(markdown.contains("stricter help tests"), "{markdown}");
}

#[test]
fn at_130_columns_the_subject_is_cut_before_the_new_columns() {
    let markdown = table_of(workers_view(&[with_remote(row("GUZ-1"))], now(), 130));
    assert!(markdown.contains('…'), "{markdown}");
    assert!(
        markdown.contains("| In Progress | #12 approved |"),
        "{markdown}"
    );
}

#[test]
fn the_list_at_80_columns_has_ticket_and_pr_lines_within_the_width() {
    let text = list_text(workers_view(&[with_remote(row("GUZ-1"))], now(), 80));
    assert!(text.contains("\n  ticket  In Progress\n"), "{text}");
    assert!(text.ends_with("\n  pr      #12 approved"), "{text}");
    assert!(text.lines().all(|l| l.width() <= 80), "{text}");
}

#[test]
fn pending_and_failed_remote_cells_show_in_the_row() {
    let mut waiting = row("GUZ-1");
    waiting.ticket = Some(TicketRemote {
        key: "GUZ-1".into(),
        state: Remote::Pending,
    });
    waiting.pr = Remote::Failed;
    let cells = table_cells(&waiting, now(), 99);
    assert_eq!(cell(&cells, 7), "…");
    assert_eq!(cell(&cells, 8), "?");
}

#[test]
fn not_live_names_the_id() {
    assert_eq!(not_live("X-1"), "No live worker X-1. /workers lists them.");
}

#[test]
fn status_tone_maps_each_state() {
    assert_eq!(status_tone("done"), Tone::Done);
    assert_eq!(status_tone("blocked"), Tone::Blocked);
    assert_eq!(status_tone("question"), Tone::Attention);
    assert_eq!(status_tone("permission"), Tone::Attention);
    assert_eq!(status_tone("running"), Tone::Active);
    assert_eq!(status_tone("busy"), Tone::Active);
    assert_eq!(status_tone("idle"), Tone::Dim);
    assert_eq!(status_tone("weird"), Tone::Plain);
}

#[test]
fn behind_is_toned_only_above_zero() {
    let tone = |count| {
        behind_text(&WorkerRow {
            behind: Lookup::Found(count),
            ..row("A")
        })[0]
            .tone
    };
    assert_eq!(tone(0), Tone::Dim);
    assert_eq!(tone(3), Tone::Behind);
}

#[test]
fn styled_tables_mark_name_sha_and_dim_fields_without_changing_widths() {
    let rows = [row("GUZ-1")];
    let plain_md = table_of(super::workers_view(&rows, now(), 160, Layout::Table, false));
    let marked_md = table_of(super::workers_view(&rows, now(), 160, Layout::Table, true));
    assert!(
        marked_md.contains(&format!(
            "| **GUZ-1** | *claude* | {TONE_ACTIVE}busy{TONE_END} | {TONE_DONE}done{TONE_END} *12m ago* | `2f7f8c0` fix(chat)"
        )),
        "{marked_md}"
    );
    assert!(
        marked_md.ends_with(&format!("| {TONE_WAITING}2{TONE_END} | *-* | *-* |")),
        "{marked_md}"
    );
    let narrow = |styled| list_text(super::workers_view(&rows, now(), 80, Layout::Table, styled));
    assert_eq!(narrow(true), narrow(false));
    assert!(plain_md.contains("| GUZ-1 | claude |"));
}

#[test]
fn cut_spans_keep_the_cut_text_of_the_plain_line() {
    let spans = vec![plain("ab"), dim("cdef")];
    for width in [0, 1, 3, 6, 9] {
        assert_eq!(
            spans_text(&cut_spans(spans.clone(), width)),
            cut("abcdef", width)
        );
    }
}

fn tone_row(id: &str, state: &str, behind: usize) -> WorkerRow {
    WorkerRow {
        state: Some(state.into()),
        behind: Lookup::Found(behind),
        ..row(id)
    }
}

fn styled_table(rows: &[WorkerRow]) -> String {
    table_of(super::workers_view(rows, now(), 200, Layout::Table, true))
}

#[test]
fn styled_diff_cells_mark_added_and_removed_counts() {
    let markdown = styled_table(&[row("GUZ-1")]);
    assert!(
        markdown.contains(&format!(
            "{TONE_DONE}+120{TONE_END} {TONE_BLOCKED}-40{TONE_END} 3f"
        )),
        "{markdown}"
    );
}

#[test]
fn styled_behind_marks_waiting_above_zero_and_keeps_zero_dim() {
    let markdown = styled_table(&[tone_row("GUZ-1", "busy", 2), tone_row("GUZ-2", "busy", 0)]);
    let lines: Vec<&str> = markdown.lines().collect();
    assert!(
        lines[2].contains(&format!("| {TONE_WAITING}2{TONE_END} |")),
        "{markdown}"
    );
    assert!(lines[3].contains("| *0* |"), "{markdown}");
}

#[test]
fn styled_state_cells_mark_their_tones() {
    let states = [
        ("done", TONE_DONE),
        ("blocked", TONE_BLOCKED),
        ("question", TONE_WAITING),
        ("running", TONE_ACTIVE),
    ];
    for (state, marker) in states {
        let markdown = styled_table(&[tone_row("GUZ-1", state, 2)]);
        assert!(
            markdown.contains(&format!("| {marker}{state}{TONE_END} |")),
            "{state}: {markdown}"
        );
    }
}

#[test]
fn tone_markers_in_a_subject_are_stripped_from_the_table() {
    let mut worker = row("GUZ-1");
    worker.head = Lookup::Found(Head {
        hash: "2f7f8c0".into(),
        subject: format!("fix {TONE_DONE}evil{TONE_END} {TONE_BLOCKED}x"),
    });
    let markdown = styled_table(&[worker]);
    assert!(markdown.contains("fix evil x"), "{markdown}");
    let clean = styled_table(&[row("GUZ-1")]);
    let count = |text: &str| text.chars().filter(|ch| TONE_MARKERS.contains(ch)).count();
    assert_eq!(count(&markdown), count(&clean), "{markdown}");
}

#[test]
fn unstyled_tables_have_no_tone_markers() {
    let markdown = table_of(workers_view(&[row("GUZ-1")], now(), 200));
    assert!(
        !markdown
            .chars()
            .any(|ch| ('\u{206A}'..='\u{206E}').contains(&ch)),
        "{markdown}"
    );
}
