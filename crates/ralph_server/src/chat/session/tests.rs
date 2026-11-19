use super::*;
use crate::config::ChatSection;
use std::time::Duration;

#[test]
fn tuning_takes_the_flag_over_the_config_for_recent_history() {
    let chat = ChatSection {
        recent_history_tokens: Some(9),
        ..ChatSection::default()
    };
    assert_eq!(
        tuning(&chat, Some(7)).map(|t| t.recent_history_tokens),
        Ok(7)
    );
    assert_eq!(tuning(&chat, None).map(|t| t.recent_history_tokens), Ok(9));
}

#[test]
fn tuning_turns_a_zero_check_interval_off() {
    let chat = ChatSection {
        worker_check_secs: Some(0),
        ..ChatSection::default()
    };
    assert_eq!(tuning(&chat, None).map(|t| t.worker_check_secs), Ok(None));
}

#[test]
fn tuning_refuses_a_zero_watch_timeout() {
    let zero = ChatSection {
        watch_timeout_secs: Some(0),
        ..ChatSection::default()
    };
    assert!(tuning(&zero, None).is_err());
    let set = ChatSection {
        watch_timeout_secs: Some(5),
        ..ChatSection::default()
    };
    assert_eq!(
        tuning(&set, None).map(|t| t.watch_timeout),
        Ok(Duration::from_secs(5))
    );
}

#[test]
fn tuning_keeps_the_quiet_and_reset_warnings() {
    let chat = ChatSection {
        worker_quiet_secs: Some(toml::Value::Integer(-1)),
        reset_turns: Some(toml::Value::Integer(-1)),
        ..ChatSection::default()
    };
    let tuned = tuning(&chat, None);
    assert!(tuned.as_ref().is_ok_and(|t| t.quiet_warning.is_some()));
    assert!(tuned.is_ok_and(|t| !t.reset_warnings.is_empty()));
}
