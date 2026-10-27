use crate::subprocess::Session;
use crate::watches::locked;
use chrono::{DateTime, Utc};
use ralph_core::chat_keepalive::{
    keepalive_action, ping_follow_up, Keepalive, KeepaliveInput, PingFollowUp,
    KEEPALIVE_PING_PROMPT,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CHECK_EVERY: Duration = Duration::from_secs(30);
const PING_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) type Stamp = Arc<Mutex<Option<DateTime<Utc>>>>;

pub(super) struct KeepaliveWatch {
    pub(super) limit_hours: u64,
    pub(super) at_prompt: Arc<AtomicBool>,
    pub(super) session: Arc<Mutex<Option<Session>>>,
    pub(super) live: Arc<AtomicBool>,
    pub(super) last_turn_end: Stamp,
    pub(super) last_ping: Stamp,
    pub(super) last_attempt: Stamp,
}

impl KeepaliveWatch {
    pub(super) fn spawn(self) {
        std::thread::spawn(move || loop {
            std::thread::sleep(CHECK_EVERY);
            self.tick();
        });
    }

    fn tick(&self) {
        let Some(turn_end) = *locked(&self.last_turn_end) else {
            return;
        };
        let now = Utc::now();
        let last_activity = (*locked(&self.last_ping)).max(Some(turn_end));
        let mut slot = locked(&self.session);
        let input = KeepaliveInput {
            limit_hours: self.limit_hours,
            idle_secs: (now - turn_end).num_seconds(),
            since_activity_secs: last_activity.map_or(0, |at| (now - at).num_seconds()),
            since_attempt_secs: (*locked(&self.last_attempt)).map(|at| (now - at).num_seconds()),
            session_live: slot.as_mut().is_some_and(Session::is_alive),
            turn_running: !self.at_prompt.load(Ordering::SeqCst),
        };
        if keepalive_action(&input) != Keepalive::Ping {
            return;
        }
        let Some(session) = slot.as_mut() else {
            return;
        };
        let result = session.ping(KEEPALIVE_PING_PROMPT, PING_TIMEOUT);
        match ping_follow_up(result.ok().map(|r| r.exit_code)) {
            PingFollowUp::Warm => {
                *locked(&self.last_ping) = Some(Utc::now());
                *locked(&self.last_attempt) = None;
            }
            PingFollowUp::Keep => *locked(&self.last_attempt) = Some(Utc::now()),
            PingFollowUp::Kill => {
                session.discard();
                self.live.store(false, Ordering::SeqCst);
            }
        }
    }
}
