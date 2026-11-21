use crate::chat::backend::ChatSession;
use crate::chat::memory::usage_path;
use crate::ledger;
use crate::subprocess::{recorded, StreamingSubprocessResult};
use crate::warn::warn_if_err;
use crate::watches::locked;
use chrono::{DateTime, Utc};
use ralph_core::chat_backend::{cache_ttl_secs, Caps};
use ralph_core::chat_keepalive::{
    keepalive_action, keeps_after, keeps_for_turn, ping_follow_up, Keepalive, KeepaliveInput,
    PingFollowUp, KEEPALIVE_PING_PROMPT,
};
use ralph_core::chat_memory::{subagent_records, usage_record, CallSite, LedgerRow};
use ralph_core::stream::IterationMetadata;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CHECK_EVERY: Duration = Duration::from_secs(30);
pub const PING_TIMEOUT: Duration = Duration::from_secs(60);

pub use crate::chat::turn::Stamp;

#[derive(Default)]
pub struct Keeps {
    counted_turn_end: Option<DateTime<Utc>>,
    count: u32,
}

pub struct KeepaliveWatch {
    pub limit_hours: u64,
    pub idle: Arc<AtomicBool>,
    pub session: Arc<Mutex<Option<Box<dyn ChatSession>>>>,
    pub live: Arc<AtomicBool>,
    pub last_turn_end: Stamp,
    pub last_ping: Stamp,
    pub last_attempt: Stamp,
    pub keeps: Mutex<Keeps>,
    pub identity: Arc<Mutex<IterationMetadata>>,
    pub run: String,
    pub store: String,
    pub caps: Caps,
}

impl KeepaliveWatch {
    pub fn run(self) -> ! {
        loop {
            std::thread::sleep(CHECK_EVERY);
            self.tick();
        }
    }

    fn tick(&self) {
        self.ping_once();
    }

    fn ping_once(&self) -> Option<()> {
        let turn_end = (*locked(&self.last_turn_end))?;
        let now = Utc::now();
        let last_activity = (*locked(&self.last_ping)).max(Some(turn_end));
        let keeps = {
            let state = locked(&self.keeps);
            keeps_for_turn(state.counted_turn_end, turn_end, state.count)
        };
        let mut slot = locked(&self.session);
        let input = KeepaliveInput {
            limit_hours: self.limit_hours,
            idle_secs: (now - turn_end).num_seconds(),
            since_activity_secs: last_activity.map_or(0, |at| (now - at).num_seconds()),
            since_attempt_secs: (*locked(&self.last_attempt)).map(|at| (now - at).num_seconds()),
            session_live: slot.as_mut().is_some_and(|session| session.is_alive()),
            turn_running: !self.idle.load(Ordering::SeqCst),
            keeps,
            cache_ttl_secs: cache_ttl_secs(&self.caps),
        };
        if keepalive_action(&input) != Keepalive::Ping {
            return None;
        }
        let session = slot.as_mut()?;
        let result = session.ping(KEEPALIVE_PING_PROMPT, PING_TIMEOUT);
        let follow_up = ping_follow_up(
            result.as_ref().ok().map(|r| r.exit_code),
            session.is_alive(),
        );
        *locked(&self.keeps) = Keeps {
            counted_turn_end: Some(turn_end),
            count: keeps_after(keeps, follow_up),
        };
        match follow_up {
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
        self.record(recorded(&result)?);
        Some(())
    }

    fn record(&self, result: &StreamingSubprocessResult) {
        let identity = locked(&self.identity).clone();
        let at = Utc::now().to_rfc3339();
        let Some(record) = ping_row(at.clone(), result, &identity) else {
            return;
        };
        let record = record
            .with_run(Some(self.run.clone()))
            .with_store(Some(self.store.clone()));
        warn_if_err(
            ledger::record(&usage_path(), &record),
            "Failed to record keepalive ping usage",
        );
        let subagents = subagent_records(&at, &result.stream_result.costs, &identity)
            .into_iter()
            .map(|row| {
                row.with_run(Some(self.run.clone()))
                    .with_store(Some(self.store.clone()))
            });
        warn_if_err(
            ledger::record_all(&usage_path(), subagents),
            "Failed to record keepalive subagent usage",
        );
    }
}

fn ping_row(
    at: String,
    result: &StreamingSubprocessResult,
    identity: &IterationMetadata,
) -> Option<LedgerRow> {
    usage_record(CallSite::Ping, at, &result.stream_result.costs, identity).map(|record| {
        record
            .with_turn_cost(result.logged_before)
            .with_request_at(result.request_at.map(|at| at.to_rfc3339()))
    })
}

#[cfg(test)]
mod ping_row_tests;
