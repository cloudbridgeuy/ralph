use crate::chat::thread;
use crate::config::{load_for_chat, ChatLoad};
use crate::signal;
use ralph_core::chat::{CustomPrompt, ReplyImages};
use ralph_core::chat_process::{run_id, SessionKey};
use ralph_core::chat_tools::chat_tool_policy;
use ralph_core::claude_leftovers::claude_dir;
use ralph_core::claude_md::{claude_md_paths, git_root, inline_claude_md, Layout, Loaded};
use ralph_core::snapshot::snapshots_enabled;
use ralph_core::turn_event::TurnSink;
use ralph_core::worker::ATTACHMENTS_DIR;
use ralph_server::chat::agy::{acp_servers, AgySlot};
use ralph_server::chat::backend::{ChatBackend, ClaudeSlot};
use ralph_server::chat::fork::Fork;
use ralph_server::chat::keepalive::Stamp;
use ralph_server::chat::memory::Store;
use ralph_server::chat::send_it::SendIt;
use ralph_server::chat::session::{
    open_core, serve, tuning, CoreParts, Emit, Session, Tuning, Wiring, WiringConfig,
};
use ralph_server::chat::snapshots::TurnSnapshots;
use ralph_server::chat::socket::serve::SocketGuard;
use ralph_server::chat::suggestion::SuggestionCell;
use ralph_server::chat::turn::Turn;
use ralph_server::config::{ignored_extras, resolve_chat_model, ChatModel};
use ralph_server::invoke::Isolation;
use ralph_server::paths::{absolute_from, config_path};
use ralph_server::subprocess::OutputGate;
use ralph_server::watches::CloseGuard;
use ralph_server::workers::{running_workers, Workers};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

pub(crate) struct Settings {
    pub exe: PathBuf,
    pub cwd: PathBuf,
    pub global_config: PathBuf,
    pub store_config: PathBuf,
    pub loaded: ChatLoad,
    pub permission_mode: String,
    pub tuning: Tuning,
}

pub(crate) fn load_settings(
    store_dir: &Path,
    permission_mode: Option<&str>,
    recent_history_tokens: Option<usize>,
) -> Result<Settings, Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let cwd = std::env::current_dir()?;
    let global_config = absolute_from(&config_path(), &cwd);
    let store_config = thread::store_config_path(store_dir);
    let loaded = load_for_chat(&global_config, &store_config);
    let permission_mode = loaded.config.chat_permission_mode(
        permission_mode,
        std::env::var(ralph_server::invoke::CHAT_PERMISSION_MODE_ENV)
            .ok()
            .as_deref(),
    );
    let tuning = tuning(&loaded.config.chat, recent_history_tokens)?;
    Ok(Settings {
        exe,
        cwd,
        global_config,
        store_config,
        loaded,
        permission_mode,
        tuning,
    })
}

pub(crate) struct TurnOutput {
    pub gate: Arc<OutputGate>,
    pub sink: Arc<dyn TurnSink>,
    pub reply_images: ReplyImages,
    pub show_tools: bool,
}

pub(crate) struct AssembleOptions<'a> {
    pub store_dir: &'a Path,
    pub store_root: PathBuf,
    pub settings: &'a Settings,
    pub fork: Option<Fork>,
    pub model_args: Vec<String>,
    pub skills: &'a [PathBuf],
    pub worktree_dir: Option<String>,
    pub custom_prompt: CustomPrompt,
    pub output: TurnOutput,
    pub events: Emit,
}

pub(crate) struct Assembled {
    pub session: Session,
    pub guard: SocketGuard,
    pub skill_dirs: Vec<PathBuf>,
    pub start_warnings: Vec<String>,
    pub model_args: Vec<String>,
    pub close: CloseGuard,
}

pub(crate) fn assemble(options: AssembleOptions) -> Result<Assembled, Box<dyn std::error::Error>> {
    let (store_dir, settings) = (options.store_dir, options.settings);
    std::fs::create_dir_all(store_dir.join(ATTACHMENTS_DIR))?;
    let config = &settings.loaded.config;
    let chat_config = &config.chat;
    let mut wiring = Wiring::open(&WiringConfig {
        store_dir,
        exe: &settings.exe,
        config,
        cli_skills: options.skills,
        home: dirs::home_dir().as_deref(),
    })?;
    let isolation = Isolation::for_chat(
        chat_config,
        &settings.cwd,
        config.claude.config_dir.as_deref(),
    );
    let live = Arc::new(AtomicBool::new(false));
    let key = options.fork.as_ref().map_or(SessionKey::Main, |fork| {
        SessionKey::Thread(fork.name.clone())
    });
    let run = run_id(std::process::id(), &key, chrono::Utc::now());
    let last_ping: Stamp = Arc::default();
    let (backend, model_args): (Arc<dyn ChatBackend>, Vec<String>) =
        match resolve_chat_model(chat_config, options.model_args)? {
            ChatModel::Claude(model_args) => (
                Arc::new(ClaudeSlot {
                    system_prompt_file: wiring.system_prompt_file.clone(),
                    permission_mode: settings.permission_mode.clone(),
                    mcp_args: std::mem::take(&mut wiring.mcp_args),
                    plugin_args: std::mem::take(&mut wiring.plugin_args),
                    model_args: model_args.clone(),
                    isolation,
                }),
                model_args,
            ),
            ChatModel::Agy { model, effort } => {
                let mcp_config: serde_json::Value =
                    serde_json::from_slice(&wiring.runtime_files.mcp_config_bytes)?;
                let names = mcp_config["mcpServers"]
                    .as_object()
                    .into_iter()
                    .flat_map(|servers| servers.keys().map(String::as_str));
                (
                    Arc::new(AgySlot {
                        cwd: settings.cwd.clone(),
                        model,
                        effort,
                        servers: acp_servers(&mcp_config)?,
                        policy: chat_tool_policy(names),
                        claude_md: agy_claude_md(config, &settings.cwd),
                    }),
                    Vec::new(),
                )
            }
        };
    let caps = backend.capabilities();
    let snapshots = TurnSnapshots::open(
        snapshots_enabled(Some(config.changes.enabled)),
        store_dir,
        &settings.cwd,
    );
    let monitor = Arc::new(open_core(CoreParts {
        store_dir,
        store_root: options.store_root,
        chat: chat_config,
        tuning: &settings.tuning,
        workers: Workers::new(
            store_dir.to_path_buf(),
            settings.exe.clone(),
            settings.permission_mode.clone(),
            options.worktree_dir,
        ),
        live: Arc::clone(&live),
        last_ping,
        run: run.clone(),
        caps,
    }));
    let roster = {
        let (monitor, store) = (Arc::clone(&monitor), Store::new(store_dir.to_path_buf()));
        move || {
            crate::chat::workers::tool_report(
                &store,
                &running_workers(&monitor.store_dir),
                &monitor.status().workers,
                monitor.inbox.peek().ok().as_deref(),
                chrono::Utc::now(),
            )
        }
    };
    let (send_it_limits, send_it_warnings) = chat_config.send_it_setting();
    let start_warnings = send_it_warnings
        .into_iter()
        .chain(agy_ignored_warning(&ignored_extras(
            config,
            chat_config.harness_kind(),
        )))
        .collect();
    let send_it = SendIt::new(store_dir, send_it_limits, options.events)?;
    let close = monitor.watches.close_guard();
    monitor.watches.restore();
    let socket = serve(&monitor, wiring.socket, &send_it, roster);
    let turn = Turn {
        store: Store::new(store_dir.to_path_buf()),
        store_dir: store_dir.to_path_buf(),
        fork: options.fork,
        runtime_files: wiring.runtime_files,
        socket,
        skills: std::mem::take(&mut wiring.skills),
        custom_prompt: options.custom_prompt,
        reply_images: options.output.reply_images,
        backend,
        session: Arc::new(Mutex::new(None)),
        process_turns: AtomicU64::new(0),
        reset: Mutex::new(None),
        live,
        reset_limits: settings.tuning.reset_limits,
        recent_history_tokens: settings.tuning.recent_history_tokens,
        gate: options.output.gate,
        run,
        suggestions: SuggestionCell::default(),
        last_turn_end: Arc::default(),
        identity: Arc::default(),
        last_cost_usd: Mutex::new(None),
        timing: Mutex::default(),
        seen_len: Mutex::new(None),
        status: {
            let monitor = Arc::clone(&monitor);
            Box::new(move || monitor.status())
        },
        stage_mark: Mutex::new(None),
        sink: options.output.sink,
        show_tools: options.output.show_tools,
        terminated: signal::is_terminated,
        exe: Some(settings.exe.clone()),
    };
    let session = Session {
        turn,
        core: monitor,
        send_it,
        limits: send_it_limits,
        snapshots,
    };
    Ok(Assembled {
        session,
        guard: wiring.guard,
        skill_dirs: wiring.skill_dirs,
        start_warnings,
        model_args,
        close,
    })
}

fn agy_claude_md(config: &ralph_server::config::AppConfig, cwd: &Path) -> String {
    let home = dirs::home_dir();
    let user_dir = home
        .as_deref()
        .filter(|_| config.chat.user_claude_md)
        .map(|home| claude_dir(config.claude.config_dir.as_deref(), home));
    let root = git_root(cwd, &|dir| dir.join(".git").exists());
    let paths = claude_md_paths(&Layout {
        git_root: &root,
        cwd,
        user_dir: user_dir.as_deref(),
        project: config.chat.project_claude_md,
    });
    inline_claude_md(&paths, home.as_deref(), &|path| {
        let path = path.canonicalize().ok()?;
        let text = std::fs::read_to_string(&path).ok()?;
        Some(Loaded { path, text })
    })
}

fn agy_ignored_warning(extras: &[String]) -> Option<String> {
    (!extras.is_empty()).then(|| format!("chat on agy ignores: {}", extras.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_extras_means_no_warning() {
        assert_eq!(agy_ignored_warning(&[]), None);
    }

    #[test]
    fn the_extras_are_listed_in_one_warning() {
        let extras = vec!["a: x".to_string(), "b: y".to_string()];
        let warning = agy_ignored_warning(&extras).unwrap();
        assert!(warning.starts_with("chat on agy ignores"));
        assert!(warning.contains("a: x; b: y"));
    }
}
