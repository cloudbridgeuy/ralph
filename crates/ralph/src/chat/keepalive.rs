use crate::chat::memory::{append_usage, usage_path};
use crate::subprocess::{Session, StreamingSubprocessResult};
use crate::watches::locked;
use chrono::{DateTime, Utc};
use ralph_core::chat_keepalive::{
    keepalive_action, ping_follow_up, Keepalive, KeepaliveInput, PingFollowUp,
    KEEPALIVE_PING_PROMPT,
};
use ralph_core::chat_memory::{usage_record, CallKind};
use ralph_core::stream::IterationMetadata;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CHECK_EVERY: Duration = Duration::from_secs(30);
const PING_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) type Stamp = Arc<Mutex<Option<DateTime<Utc>>>>;

pub(super) struct KeepaliveWatch {
    pub(super) limit_hours: u64,
    pub(super) idle: Arc<AtomicBool>,
    pub(super) session: Arc<Mutex<Option<Session>>>,
    pub(super) live: Arc<AtomicBool>,
    pub(super) last_turn_end: Stamp,
    pub(super) last_ping: Stamp,
    pub(super) last_attempt: Stamp,
    pub(super) run: String,
}

impl KeepaliveWatch {
    pub(super) fn spawn(self) {
        std::thread::spawn(move || loop {
            std::thread::sleep(CHECK_EVERY);
            self.tick();
        });
    }

    fn tick(&self) {
        if let Some(result) = self.ping_once() {
            self.record(&result);
        }
    }

    fn ping_once(&self) -> Option<StreamingSubprocessResult> {
        let turn_end = (*locked(&self.last_turn_end))?;
        let now = Utc::now();
        let last_activity = (*locked(&self.last_ping)).max(Some(turn_end));
        let mut slot = locked(&self.session);
        let input = KeepaliveInput {
            limit_hours: self.limit_hours,
            idle_secs: (now - turn_end).num_seconds(),
            since_activity_secs: last_activity.map_or(0, |at| (now - at).num_seconds()),
            since_attempt_secs: (*locked(&self.last_attempt)).map(|at| (now - at).num_seconds()),
            session_live: slot.as_mut().is_some_and(Session::is_alive),
            turn_running: !self.idle.load(Ordering::SeqCst),
        };
        if keepalive_action(&input) != Keepalive::Ping {
            return None;
        }
        let session = slot.as_mut()?;
        let result = session.ping(KEEPALIVE_PING_PROMPT, PING_TIMEOUT);
        match ping_follow_up(result.as_ref().ok().map(|r| r.exit_code)) {
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
        result.ok()
    }

    fn record(&self, result: &StreamingSubprocessResult) {
        let Some(mut record) = usage_record(
            CallKind::Ping,
            Utc::now().to_rfc3339(),
            &result.stream_result.costs,
            &IterationMetadata::new(),
        ) else {
            return;
        };
        record.request_at = result.request_at.map(|at| at.to_rfc3339());
        record.run = Some(self.run.clone());
        let _ = append_usage(&usage_path(), &record);
    }
}
