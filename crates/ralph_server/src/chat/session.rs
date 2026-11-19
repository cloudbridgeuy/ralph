use crate::config::ChatSection;
use ralph_core::chat::{
    resolve_recent_history_tokens, resolve_watch_timeout, resolve_worker_check_secs, ResetLimits,
};
use ralph_core::chat_keepalive::resolve_keepalive_hours;
use std::time::Duration;

pub struct Tuning {
    pub recent_history_tokens: usize,
    pub worker_check_secs: Option<u64>,
    pub keepalive_hours: Option<u64>,
    pub worker_quiet_secs: u64,
    pub quiet_warning: Option<String>,
    pub reset_limits: ResetLimits,
    pub reset_warnings: Vec<String>,
    pub watch_timeout: Duration,
}

pub fn tuning(chat: &ChatSection, recent_history_flag: Option<usize>) -> Result<Tuning, String> {
    let (worker_quiet_secs, quiet_warning) = chat.worker_quiet_setting();
    let (reset_limits, reset_warnings) = chat.reset_limits_setting();
    Ok(Tuning {
        recent_history_tokens: resolve_recent_history_tokens(
            recent_history_flag,
            chat.recent_history_tokens,
        ),
        worker_check_secs: resolve_worker_check_secs(chat.worker_check_secs),
        keepalive_hours: resolve_keepalive_hours(chat.keepalive_hours),
        worker_quiet_secs,
        quiet_warning,
        reset_limits,
        reset_warnings,
        watch_timeout: resolve_watch_timeout(chat.watch_timeout_secs)?,
    })
}

#[cfg(test)]
mod tests;
