use chrono::{DateTime, Utc};
use std::time::Duration;

pub const KEEPALIVE_HOURS: u64 = 4;
pub const KEEPALIVE_PING_SECS: u64 = 55 * 60;
pub const KEEPALIVE_RETRY_SECS: u64 = 5 * 60;
pub const KEEPALIVE_MAX_KEEPS: u32 = 3;
pub const KEEPALIVE_PING_PROMPT: &str = "PING. Reply with only PONG.";

pub fn resolve_keepalive_hours(config: Option<u64>) -> Option<u64> {
    match config.unwrap_or(KEEPALIVE_HOURS) {
        0 => None,
        hours => Some(hours),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keepalive {
    Ping,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PingFollowUp {
    Warm,
    Keep,
    Kill,
}

pub fn ping_follow_up(exit: Option<i32>, alive: bool) -> PingFollowUp {
    match exit {
        _ if !alive => PingFollowUp::Kill,
        Some(0) => PingFollowUp::Warm,
        Some(_) => PingFollowUp::Keep,
        None => PingFollowUp::Kill,
    }
}

pub fn keeps_after(keeps: u32, follow_up: PingFollowUp) -> u32 {
    match follow_up {
        PingFollowUp::Keep => keeps.saturating_add(1),
        PingFollowUp::Warm | PingFollowUp::Kill => 0,
    }
}

pub fn keeps_for_turn(
    counted_turn_end: Option<DateTime<Utc>>,
    turn_end: DateTime<Utc>,
    keeps: u32,
) -> u32 {
    if counted_turn_end == Some(turn_end) {
        keeps
    } else {
        0
    }
}

pub fn lock_wait_continues(waited: Duration, limit: Duration) -> bool {
    waited < limit
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepaliveInput {
    pub limit_hours: u64,
    pub idle_secs: i64,
    pub since_activity_secs: i64,
    pub since_attempt_secs: Option<i64>,
    pub session_live: bool,
    pub turn_running: bool,
    pub keeps: u32,
}

pub fn keepalive_action(input: &KeepaliveInput) -> Keepalive {
    let idle = u64::try_from(input.idle_secs).unwrap_or(0);
    let since_activity = u64::try_from(input.since_activity_secs).unwrap_or(0);
    let retry_pending = input
        .since_attempt_secs
        .is_some_and(|secs| u64::try_from(secs).unwrap_or(0) < KEEPALIVE_RETRY_SECS);
    if idle >= input.limit_hours.saturating_mul(3600)
        || input.turn_running
        || !input.session_live
        || retry_pending
        || input.keeps >= KEEPALIVE_MAX_KEEPS
    {
        Keepalive::Skip
    } else if since_activity >= KEEPALIVE_PING_SECS {
        Keepalive::Ping
    } else {
        Keepalive::Skip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> KeepaliveInput {
        KeepaliveInput {
            limit_hours: 18,
            idle_secs: 3600,
            since_activity_secs: 3600,
            since_attempt_secs: None,
            session_live: true,
            turn_running: false,
            keeps: 0,
        }
    }

    #[test]
    fn hours_default_to_four_and_zero_turns_it_off() {
        assert_eq!(resolve_keepalive_hours(None), Some(4));
        assert_eq!(resolve_keepalive_hours(Some(0)), None);
        assert_eq!(resolve_keepalive_hours(Some(2)), Some(2));
    }

    #[test]
    fn pings_at_fifty_five_minutes_not_before() {
        let at = |secs| KeepaliveInput {
            since_activity_secs: secs,
            ..input()
        };
        assert_eq!(keepalive_action(&at(3299)), Keepalive::Skip);
        assert_eq!(keepalive_action(&at(3300)), Keepalive::Ping);
    }

    #[test]
    fn repeats_from_the_last_ping_even_when_the_turn_was_hours_ago() {
        let at = |since| KeepaliveInput {
            idle_secs: 5 * 3600,
            since_activity_secs: since,
            ..input()
        };
        assert_eq!(keepalive_action(&at(54 * 60)), Keepalive::Skip);
        assert_eq!(keepalive_action(&at(55 * 60)), Keepalive::Ping);
    }

    #[test]
    fn skips_after_the_hour_limit_not_before() {
        let idle = |secs| KeepaliveInput {
            idle_secs: secs,
            ..input()
        };
        assert_eq!(keepalive_action(&idle(18 * 3600 - 1)), Keepalive::Ping);
        assert_eq!(keepalive_action(&idle(18 * 3600)), Keepalive::Skip);
    }

    #[test]
    fn skips_while_a_turn_runs_or_without_a_session() {
        let running = KeepaliveInput {
            turn_running: true,
            ..input()
        };
        let dead = KeepaliveInput {
            session_live: false,
            ..input()
        };
        assert_eq!(keepalive_action(&running), Keepalive::Skip);
        assert_eq!(keepalive_action(&dead), Keepalive::Skip);
    }

    #[test]
    fn negative_clocks_count_as_zero() {
        let skewed = KeepaliveInput {
            idle_secs: -5,
            since_activity_secs: -5,
            ..input()
        };
        assert_eq!(keepalive_action(&skewed), Keepalive::Skip);
    }

    #[test]
    fn follow_up_maps_the_exit_code() {
        assert_eq!(ping_follow_up(Some(0), true), PingFollowUp::Warm);
        assert_eq!(ping_follow_up(Some(1), true), PingFollowUp::Keep);
        assert_eq!(ping_follow_up(None, true), PingFollowUp::Kill);
    }

    #[test]
    fn a_dead_process_kills_whatever_the_exit_was() {
        assert_eq!(ping_follow_up(Some(0), false), PingFollowUp::Kill);
        assert_eq!(ping_follow_up(Some(1), false), PingFollowUp::Kill);
    }

    #[test]
    fn the_third_keep_still_pings_then_pings_stop() {
        let after = |keeps| KeepaliveInput { keeps, ..input() };
        assert_eq!(keepalive_action(&after(2)), Keepalive::Ping);
        assert_eq!(keepalive_action(&after(3)), Keepalive::Skip);
    }

    #[test]
    fn keeps_count_up_and_warm_or_kill_reset_them() {
        assert_eq!(keeps_after(2, PingFollowUp::Keep), 3);
        assert_eq!(keeps_after(2, PingFollowUp::Warm), 0);
        assert_eq!(keeps_after(2, PingFollowUp::Kill), 0);
    }

    #[test]
    fn a_new_turn_resets_the_keeps() {
        let at = |secs| DateTime::from_timestamp(secs, 0).unwrap();
        assert_eq!(keeps_for_turn(Some(at(10)), at(10), 3), 3);
        assert_eq!(keeps_for_turn(Some(at(10)), at(20), 3), 0);
        assert_eq!(keeps_for_turn(None, at(20), 3), 0);
    }

    #[test]
    fn the_lock_wait_ends_at_the_limit() {
        let limit = Duration::from_secs(60);
        assert!(lock_wait_continues(Duration::from_secs(59), limit));
        assert!(!lock_wait_continues(limit, limit));
    }

    #[test]
    fn a_failed_attempt_backs_off_for_five_minutes() {
        let after = |secs| KeepaliveInput {
            since_attempt_secs: Some(secs),
            ..input()
        };
        assert_eq!(keepalive_action(&after(299)), Keepalive::Skip);
        assert_eq!(keepalive_action(&after(300)), Keepalive::Ping);
    }
}
