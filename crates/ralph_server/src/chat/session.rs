use crate::chat::check::CheckState;
use crate::chat::keepalive::Stamp;
use crate::chat::memory::{self, Store};
use crate::chat::monitor::{Monitor, ProcessMemo};
use crate::chat::schedule::restore_schedule;
use crate::chat::schedule_worker::alive_worker_ids;
use crate::chat::send_it::SendIt;
use crate::chat::skills::{
    env_skills, link_skills, read_claude_config, usable_skills, write_mcp_config,
    write_skill_plugin,
};
use crate::chat::socket::serve::{ChatSocket, Shared, SocketGuard};
use crate::config::{AppConfig, ChatSection};
use crate::opencode_worker::sweep_stale_server_records;
use crate::schedule_store::ScheduleStore;
use crate::watches::Watches;
use crate::wire::store::current_uid;
use crate::worker_mcp::Inbox;
use crate::workers::Workers;
use ralph_core::chat::{
    resolve_recent_history_tokens, resolve_watch_timeout, resolve_worker_check_secs,
    select_servers, ResetLimits,
};
use ralph_core::chat_keepalive::resolve_keepalive_hours;
use ralph_core::chat_process::UsageTail;
use ralph_core::chat_skills::{build_plugin_args, resolve_skills, ChatSkill, SkillSources};
use ralph_core::chat_socket::{runtime_root, system_prompt_path};
use ralph_core::chat_status::Status;
use ralph_core::program_status::Phase;
use ralph_core::schedule_request::SCHEDULE_DIR;
use ralph_core::worker::INBOX_DIR;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
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

pub struct WiringConfig<'a> {
    pub store_dir: &'a Path,
    pub exe: &'a Path,
    pub config: &'a AppConfig,
    pub cli_skills: &'a [PathBuf],
    pub home: Option<&'a Path>,
}

pub struct Wiring {
    pub socket: ChatSocket,
    pub guard: SocketGuard,
    pub mcp_args: Vec<String>,
    pub system_prompt_file: PathBuf,
    pub skill_dirs: Vec<PathBuf>,
    pub skills: Vec<ChatSkill>,
    pub plugin: Option<tempfile::TempDir>,
    pub plugin_args: Vec<String>,
}

impl Wiring {
    pub fn open(wiring: &WiringConfig) -> io::Result<Self> {
        sweep_stale_server_records();
        let chat = &wiring.config.chat;
        let socket = ChatSocket::bind(wiring.store_dir, &runtime_root(current_uid()))?;
        let guard = SocketGuard(socket.files().clone());
        let imported = select_servers(
            &read_claude_config(wiring.config.claude.config_dir.as_deref()),
            &chat.mcp_servers,
        );
        let mcp_args = write_mcp_config(wiring.store_dir, socket.path(), wiring.exe, &imported)?;
        let system_prompt_file = system_prompt_path(socket.path());
        let skill_dirs = resolve_skills(
            &SkillSources {
                cli: wiring.cli_skills,
                env: &env_skills(),
                config: &chat.skills,
            },
            wiring.home,
        );
        let skills = link_skills(usable_skills(skill_dirs.clone()));
        let plugin = write_skill_plugin(&skills)?;
        let plugin_args = plugin
            .as_ref()
            .map(|plugin| build_plugin_args(plugin.path()))
            .unwrap_or_default();
        Ok(Self {
            socket,
            guard,
            mcp_args,
            system_prompt_file,
            skill_dirs,
            skills,
            plugin,
            plugin_args,
        })
    }
}

pub struct CoreParts<'a> {
    pub store_dir: &'a Path,
    pub store_root: PathBuf,
    pub chat: &'a ChatSection,
    pub tuning: &'a Tuning,
    pub workers: Workers,
    pub live: Arc<AtomicBool>,
    pub last_ping: Stamp,
    pub run: String,
}

pub fn open_core(parts: CoreParts) -> Monitor {
    let inbox = Arc::new(Inbox::new(parts.store_dir.join(INBOX_DIR)));
    let known = inbox.peek().unwrap_or_default();
    let usage_stamp = memory::usage_stamp(&memory::usage_path()).unwrap_or_default();
    let store = Store::new(parts.store_dir.to_path_buf());
    Monitor {
        watches: Watches::new(
            Arc::clone(&inbox),
            parts.store_dir,
            parts.tuning.watch_timeout,
        ),
        schedule: ScheduleStore::new(parts.store_dir.join(SCHEDULE_DIR)),
        inbox,
        check: Mutex::new(CheckState::seeded(&store)),
        store,
        store_dir: parts.store_root,
        status: Mutex::new(Status::default()),
        seen: Mutex::new(HashMap::new()),
        active: Mutex::new(None),
        recent_history_tokens: parts.tuning.recent_history_tokens,
        worker_check_secs: parts.tuning.worker_check_secs,
        worker_quiet_secs: parts.tuning.worker_quiet_secs,
        auto_continue: parts.chat.auto_continue,
        hold_progress: parts.chat.hold_progress,
        workers: parts.workers,
        known: Mutex::new(known),
        fresh: Mutex::new((Vec::new(), 0)),
        starting: Arc::default(),
        scheduled_reports: Mutex::new(Vec::new()),
        warned: Mutex::new(HashSet::new()),
        unrecorded: Mutex::new(HashSet::new()),
        live: parts.live,
        last_ping: parts.last_ping,
        run: parts.run,
        reset_limits: parts.tuning.reset_limits,
        process: Mutex::new(ProcessMemo::default()),
        usage_tail: Mutex::new(UsageTail::starting_at(&usage_stamp)),
        phase: Mutex::new(Phase::Prompt),
        hold: Mutex::new(None),
    }
}

pub fn serve(
    core: &Monitor,
    socket: ChatSocket,
    send_it: &SendIt,
    roster: impl Fn() -> String + Send + Sync + 'static,
) {
    socket.serve(
        Shared {
            store: Store::new(core.store.dir().to_path_buf()),
            workers: core.workers.clone(),
            send_it: send_it.clone(),
            watches: core.watches.clone(),
            inbox: Arc::clone(&core.inbox),
        },
        roster,
    );
    restore_schedule(&core.schedule, &alive_worker_ids(&core.store_dir));
}

#[cfg(test)]
mod tests;
