use super::*;

const VERIFY: &str = "cargo test -p core";

fn limits_with(mode: VerifyMode) -> Limits {
    Limits {
        verify: mode,
        ..limits()
    }
}

fn verifying_json() -> serde_json::Value {
    let mut json = plan_json();
    json["steps"][0]["criterion"]["verify"] = serde_json::json!(VERIFY);
    json
}

fn verifying_plan(mode: VerifyMode) -> RunPlan {
    parse_run(&verifying_json(), limits_with(mode)).unwrap()
}

fn stopped_at_first_step(mode: VerifyMode) -> RunFile {
    let plan = verifying_plan(mode);
    let hash = plan_hash(&plan);
    let stopped = stop_file(&approved(plan), &hash, Some(1)).unwrap();
    signed_go(&stopped, None, true).unwrap()
}

#[test]
fn the_written_run_file_carries_no_verify_mode() {
    for mode in [VerifyMode::Off, VerifyMode::Observe, VerifyMode::Enforce] {
        let text = write_run(&approved(verifying_plan(mode))).unwrap();
        let table: toml::Table = toml::from_str(&text).unwrap();
        assert!(!table.contains_key("verify"), "{text}");
        assert!(!table["caps"].as_table().unwrap().contains_key("verify"));
        assert!(
            !text.contains("observe") && !text.contains("enforce"),
            "{text}"
        );
    }
}

#[test]
fn the_plan_hash_ignores_the_verify_mode() {
    assert_eq!(
        plan_hash(&verifying_plan(VerifyMode::Off)),
        plan_hash(&verifying_plan(VerifyMode::Enforce))
    );
}

#[test]
fn a_stray_verify_key_in_the_run_file_loads_and_the_current_limits_win() {
    let text = write_run(&approved(verifying_plan(VerifyMode::Off))).unwrap();
    let mut table: toml::Table = toml::from_str(&text).unwrap();
    table.insert("verify".to_string(), "enforce".into());
    table
        .get_mut("caps")
        .and_then(toml::Value::as_table_mut)
        .unwrap()
        .insert("verify".to_string(), "enforce".into());
    let stray = toml::to_string(&table).unwrap();
    assert!(stray.contains("enforce"));
    for mode in [VerifyMode::Off, VerifyMode::Observe] {
        let file = read_run(&stray, limits_with(mode)).unwrap();
        assert_eq!(file.plan.limits.verify, mode);
        assert!(file.is_approved(TRUST));
    }
}

#[test]
fn a_resume_that_finds_the_worker_done_verifies_under_the_configured_mode() {
    for mode in [VerifyMode::Observe, VerifyMode::Enforce] {
        let ticked = start(&stopped_at_first_step(mode), &[("E1", "done")], &[]).unwrap();
        assert_eq!(ticked.next, Next::Check(VERIFY.to_string()));
        assert_eq!(ticked.driving.progress.verify, mode);
        assert!(ticked.driving.progress.verifying);
    }
}

#[test]
fn a_resume_that_finds_the_worker_done_skips_the_check_when_verify_is_off() {
    let ticked = start(
        &stopped_at_first_step(VerifyMode::Off),
        &[("E1", "done")],
        &[],
    )
    .unwrap();
    assert!(!matches!(ticked.next, Next::Check(command) if command == VERIFY));
    assert_eq!(ticked.driving.progress.verify, VerifyMode::Off);
    assert!(!ticked.driving.progress.verifying);
}

#[test]
fn a_fresh_start_keeps_the_configured_mode_for_later_reports() {
    let file = approved(verifying_plan(VerifyMode::Enforce));
    let ticked = start(&file, &[], &[]).unwrap();
    assert_eq!(ticked.driving.progress.verify, VerifyMode::Enforce);
}
