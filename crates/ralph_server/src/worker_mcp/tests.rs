use super::*;

fn stop_hook(reported: &Path, payload: &str) -> String {
    let unreported = reported.with_file_name("unreported");
    let mut out = Vec::new();
    let hook = StopHook {
        reported,
        unreported: &unreported,
        activity: None,
        changes: None,
        usage: None,
        verify: false,
    };
    run_stop_hook(&hook, payload.as_bytes(), &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "").unwrap();
}

fn activity_after(payload: &str, reported_exists: bool) -> String {
    let dir = tempfile::tempdir().unwrap();
    let reported = dir.path().join("reported");
    if reported_exists {
        touch(&reported);
    }
    let unreported = dir.path().join("unreported");
    let activity = dir.path().join("activity").join("A");
    let hook = StopHook {
        reported: &reported,
        unreported: &unreported,
        activity: Some(&activity),
        changes: None,
        usage: None,
        verify: false,
    };
    run_stop_hook(&hook, payload.as_bytes(), Vec::new()).unwrap();
    fs::read_to_string(activity).unwrap()
}

fn stop_row(verify: bool) -> serde_json::Value {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s1.jsonl");
    fs::write(
        &transcript,
        r#"{"type":"assistant","message":{"id":"m1","model":"opus","stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":2}}}"#,
    )
    .unwrap();
    let target = UsageTarget {
        worker: "verify".into(),
        seen: dir.path().join("verify.usage-seen"),
        store: None,
        usage: dir.path().join("usage.jsonl"),
    };
    let unreported = dir.path().join("unreported");
    let hook = StopHook {
        reported: &dir.path().join("reported"),
        unreported: &unreported,
        activity: None,
        changes: None,
        usage: Some(&target),
        verify,
    };
    let payload = format!(
        r#"{{"session_id":"s1","transcript_path":"{}"}}"#,
        transcript.display()
    );
    run_stop_hook(&hook, payload.as_bytes(), Vec::new()).unwrap();
    let written = fs::read_to_string(&target.usage).unwrap();
    assert_eq!(written.lines().count(), 1);
    serde_json::from_str(written.trim()).unwrap()
}

#[test]
fn a_verify_stop_hook_appends_one_verify_row_priced_like_a_worker_row() {
    let worker = stop_row(false);
    assert_eq!(worker["kind"], "worker");
    let row = stop_row(true);
    assert_eq!(row["kind"], "verify");
    assert_eq!(row["role"], "verify");
    assert_eq!(row["harness"], "claude-tmux");
    assert_eq!(row["cost_usd"], worker["cost_usd"]);
    if worker["cost_usd"].is_null() {
        assert!(row["cost_usd"].is_null());
    }
}

#[test]
fn drop_worker_removes_its_queued_reports_and_logs_each() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    for (worker, seq) in [("A", 1), ("B", 1), ("A", 2)] {
        let report = Report::from_worker(
            worker,
            &format!("2026-11-10T13:00:0{seq}Z"),
            ReportStatus::Done,
            "t".to_string(),
        );
        inbox.post(&report, seq).unwrap();
    }
    assert_eq!(inbox.drop_worker("A").unwrap(), 2);
    let left: Vec<String> = inbox
        .peek()
        .unwrap()
        .into_iter()
        .map(|r| r.worker)
        .collect();
    assert_eq!(left, ["B"]);
    let log = fs::read_to_string(dir.path().join(DROP_LOG)).unwrap();
    assert_eq!(log.lines().count(), 2);
    assert_eq!(inbox.drop_worker("A").unwrap(), 0);
}

#[test]
fn stop_hook_writes_background_while_tasks_run_and_idle_otherwise() {
    let busy = r#"{"stop_hook_active":false,"background_tasks":[{"id":"x"}]}"#;
    assert!(activity_after(busy, false).ends_with(" background"));
    let plain = r#"{"stop_hook_active":false}"#;
    assert!(activity_after(plain, true).ends_with(" idle"));
    assert!(activity_after("not json", false).ends_with(" idle"));
}

#[test]
fn stop_hook_blocks_once_then_a_report_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let reported = dir.path().join("reported").join("A");
    let first = stop_hook(&reported, r#"{"stop_hook_active":false}"#);
    assert_eq!(
        serde_json::from_str::<Value>(&first).unwrap(),
        stop_block_output()
    );
    assert_eq!(stop_hook(&reported, r#"{"stop_hook_active":true}"#), "");

    touch(&reported);
    assert_eq!(stop_hook(&reported, r#"{"stop_hook_active":false}"#), "");
    assert!(!reported.exists());
}

#[test]
fn stop_hook_give_up_writes_unreported_and_a_report_removes_both() {
    let dir = tempfile::tempdir().unwrap();
    let reported = dir.path().join("reported").join("A");
    let unreported = reported.with_file_name("unreported");
    assert_eq!(stop_hook(&reported, r#"{"stop_hook_active":true}"#), "");
    assert!(unreported.exists());

    touch(&reported);
    assert_eq!(stop_hook(&reported, r#"{"stop_hook_active":true}"#), "");
    assert!(!reported.exists() && !unreported.exists());
}

#[test]
fn stop_hook_with_background_tasks_leaves_markers_alone() {
    let dir = tempfile::tempdir().unwrap();
    let reported = dir.path().join("reported").join("A");
    let unreported = reported.with_file_name("unreported");
    touch(&reported);
    touch(&unreported);
    let busy = r#"{"stop_hook_active":false,"background_tasks":[{"id":"x"}]}"#;
    assert_eq!(stop_hook(&reported, busy), "");
    assert!(reported.exists() && unreported.exists());
}

#[test]
fn stop_hook_lets_malformed_input_stop() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(stop_hook(&dir.path().join("A"), "not json"), "");
}

#[test]
fn report_lands_in_inbox_and_drains_once() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"report","arguments":{"status":"done","text":"hello"}}}"#;
    let mut out = Vec::new();
    run_stdio_server(
        "GUZ-1",
        &inbox,
        None,
        format!("{call}\n").as_bytes(),
        &mut out,
    )
    .unwrap();
    let reply: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["result"]["isError"], false);
    assert!(dir.path().join("reported").join("GUZ-1").exists());

    assert_eq!(inbox.peek().unwrap().len(), 1);
    let reports = inbox.drain(Selection::All).unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].worker, "GUZ-1");
    assert_eq!(reports[0].text, "hello");
    assert!(inbox.peek().unwrap().is_empty());
}

fn report_call(id: u32, text: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"report","arguments":{{"status":"done","text":"{text}"}}}}}}"#
    )
}

fn replies(inbox: &Inbox, calls: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    let input = format!("{}\n", calls.join("\n"));
    run_stdio_server("GUZ-1", inbox, None, input.as_bytes(), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_second_report_in_the_same_turn_is_refused_and_posts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox")).once_per_turn(true);
    let out = replies(&inbox, &[report_call(1, "first"), report_call(2, "second")]);
    assert_eq!(out[0]["result"]["isError"], false);
    assert_eq!(out[1]["result"]["isError"], true);
    assert!(out[1]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Already reported this turn"));
    let reports = inbox.drain(Selection::All).unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].text, "first");
}

#[test]
fn a_report_after_the_turn_consumed_the_marker_is_delivered() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox")).once_per_turn(true);
    replies(&inbox, &[report_call(1, "first")]);
    fs::remove_file(dir.path().join("reported").join("GUZ-1")).unwrap();
    let out = replies(&inbox, &[report_call(2, "next turn")]);
    assert_eq!(out[0]["result"]["isError"], false);
    assert_eq!(inbox.peek().unwrap().len(), 2);
}

#[test]
fn without_the_once_per_turn_flag_repeat_reports_still_post() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    let out = replies(&inbox, &[report_call(1, "a"), report_call(2, "b")]);
    assert_eq!(out[1]["result"]["isError"], false);
    assert_eq!(inbox.peek().unwrap().len(), 2);
}

fn status_call(id: u32, text: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"set_status","arguments":{{"text":"{text}"}}}}}}"#
    )
}

#[test]
fn set_status_appends_to_the_file_without_touching_the_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    let input = format!(
        "{}\n{}\n",
        status_call(1, "first"),
        status_call(2, "second")
    );
    let mut out = Vec::new();
    run_stdio_server("GUZ-1", &inbox, None, input.as_bytes(), &mut out).unwrap();
    let replies: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(replies.iter().all(|r| r["result"]["isError"] == false));
    assert_eq!(replies[1]["result"]["content"][0]["text"], "Status set.");

    let status = fs::read_to_string(status_path(dir.path(), "GUZ-1")).unwrap();
    let texts: Vec<_> = decode_statuses(&status)
        .into_iter()
        .map(|status| status.text)
        .collect();
    assert_eq!(texts, ["first", "second"]);
    let names: Vec<_> = fs::read_dir(dir.path().join("status"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["GUZ-1"]);
    assert!(inbox.peek().unwrap().is_empty());
    assert!(!dir.path().join("inbox").exists());
    assert!(!dir.path().join("reported").exists());
}

#[test]
fn set_status_keeps_only_the_last_five() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    let input: String = (1..=7)
        .map(|n| format!("{}\n", status_call(n, &format!("step {n}"))))
        .collect();
    let mut out = Vec::new();
    run_stdio_server("GUZ-1", &inbox, None, input.as_bytes(), &mut out).unwrap();
    let status = fs::read_to_string(status_path(dir.path(), "GUZ-1")).unwrap();
    let texts: Vec<_> = decode_statuses(&status)
        .into_iter()
        .map(|status| status.text)
        .collect();
    assert_eq!(texts, ["step 3", "step 4", "step 5", "step 6", "step 7"]);
    assert_eq!(fs::read_dir(dir.path().join("status")).unwrap().count(), 1);
}

#[test]
fn set_status_starts_from_an_old_single_entry_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = status_path(dir.path(), "GUZ-1");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "2026-01-01T00:00:00+00:00\nold\n").unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    let input = format!("{}\n", status_call(1, "new"));
    let mut out = Vec::new();
    run_stdio_server("GUZ-1", &inbox, None, input.as_bytes(), &mut out).unwrap();
    let texts: Vec<_> = decode_statuses(&fs::read_to_string(&path).unwrap())
        .into_iter()
        .map(|status| status.text)
        .collect();
    assert_eq!(texts, ["old", "new"]);
}

#[test]
fn set_status_is_refused_after_close_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let inbox =
        Inbox::new(dir.path().join("inbox")).requiring_owner(Some(dir.path().join("owner-gone")));
    let mut out = Vec::new();
    let input = format!("{}\n", status_call(1, "late"));
    run_stdio_server("GUZ-1", &inbox, None, input.as_bytes(), &mut out).unwrap();
    let reply: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(
        reply["result"]["content"][0]["text"],
        "Could not set status: worker closed."
    );
    assert!(!dir.path().join("status").exists());
}

#[test]
fn set_status_works_while_the_owner_exists() {
    let dir = tempfile::tempdir().unwrap();
    let owner = dir.path().join("owner");
    fs::write(&owner, "").unwrap();
    let inbox = Inbox::new(dir.path().join("inbox")).requiring_owner(Some(owner));
    let mut out = Vec::new();
    let input = format!("{}\n", status_call(1, "ok"));
    run_stdio_server("GUZ-1", &inbox, None, input.as_bytes(), &mut out).unwrap();
    let reply: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["result"]["isError"], false);
    assert!(status_path(dir.path(), "GUZ-1").exists());
}

#[test]
fn drain_some_takes_only_that_position() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    for (seq, text) in [(1, "one"), (2, "two")] {
        let report = Report::from_worker(
            "A",
            &format!("2026-01-01T00:00:0{seq}Z"),
            ReportStatus::Done,
            text.to_string(),
        );
        inbox.post(&report, seq).unwrap();
    }
    let taken = inbox.drain(Selection::At("2".parse().unwrap())).unwrap();
    assert_eq!(taken[0].text, "two");
    assert_eq!(inbox.peek().unwrap().len(), 1);
}

#[test]
fn drained_reports_leave_the_ledger_and_the_queue_restarts_at_one() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    let post = |text: &str, seq| {
        let report = Report::from_worker(
            "A",
            &format!("2026-01-01T00:00:0{seq}Z"),
            ReportStatus::Done,
            text.to_string(),
        );
        inbox.post(&report, seq).unwrap();
    };
    post("one", 1);
    post("two", 2);
    assert_eq!(inbox.drain(Selection::All).unwrap().len(), 2);
    post("three", 3);

    let queued = inbox.peek().unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].text, "three");
    assert!(inbox.pending().unwrap().is_empty());
    let value: Value =
        serde_json::from_slice(&fs::read(dir.path().join("inbox.json")).unwrap()).unwrap();
    assert_eq!(value["entries"].as_array().unwrap().len(), 1);
}

fn same_name_report(text: &str) -> Report {
    Report::ralph_blocked("schedule:a", "2026-01-01T00:00:00Z", text.to_string())
}

fn queued_texts(inbox: &Inbox) -> Vec<String> {
    inbox.peek().unwrap().into_iter().map(|r| r.text).collect()
}

#[test]
fn two_posts_with_one_file_name_both_reach_the_queue() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    inbox.post(&same_name_report("one"), 0).unwrap();
    inbox.post(&same_name_report("two"), 0).unwrap();
    assert_eq!(queued_texts(&inbox), ["one", "two"]);
    assert_eq!(fs::read_dir(dir.path().join("inbox")).unwrap().count(), 0);
}

#[test]
fn a_post_that_reuses_a_queued_file_name_reaches_the_queue() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    inbox.post(&same_name_report("one"), 0).unwrap();
    assert_eq!(queued_texts(&inbox), ["one"]);
    inbox.post(&same_name_report("two"), 0).unwrap();
    assert_eq!(queued_texts(&inbox), ["one", "two"]);
}

#[test]
fn a_report_removes_the_unreported_marker() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    let unreported = unreported_path(dir.path(), "A");
    touch(&unreported);
    inbox.mark_reported("A").unwrap();
    assert!(!unreported.exists());
    assert!(reported_path(dir.path(), "A").exists());
}

#[test]
fn a_report_removes_the_sent_marker() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = Inbox::new(dir.path().join("inbox"));
    let sent = sent_path(dir.path(), "A");
    touch(&sent);
    inbox.mark_reported("A").unwrap();
    assert!(!sent.exists());
}
