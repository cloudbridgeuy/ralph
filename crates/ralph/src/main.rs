#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::too_many_arguments))]

pub mod ansi;
mod backoff;
mod chat;
mod cli;
mod clipboard;
pub mod config;
pub mod diff_highlight;
mod diff_theme;
pub mod formatting;
mod frame;
pub mod highlight;
mod hook_bridge;
mod inbox_alert;
mod inbox_post;
mod input_theme;
mod invoke;
mod licenses;
pub mod markdown;
mod mermaid_guard;
pub mod prompt_source;
pub mod render;
mod schedule_store;
pub mod signal;
pub mod spinner;
pub mod startup;
mod status_theme;
mod theme_slots;
mod tool_theme;
mod ui_theme;
mod upgrade;

pub mod stream_processor;
pub mod subprocess;
mod version;
pub mod warn;
mod watch_requests;
mod wire;

use clap::Parser;
use cli::{
    AskArgs, ChatArgs, ChatMcpArgs, ChatNapArgs, ChatTagArgs, Cli, Commands, MermaidArgs,
    VerifyAgent, WorkerArgs, WorkerCommand, WorkerMcpArgs, WorkerStopHookArgs,
};
use invoke::InvocationConfig;
use ralph_core::worker::HarnessId;
use ralph_server::version_drift::DriftRun;
use ralph_server::warn::{warn, warn_if_err};
use std::process::ExitCode;
use stream_processor::VerboseToolsConfig;

fn main() -> ExitCode {
    ralph_server::warn::register_sink(warn::print_warning);
    ralph_server::warn::register_notice_sink(|m| warn::notice(m));
    let cli = Cli::parse();

    let result = match cli.command.unwrap_or(Commands::Chat(ChatArgs {
        command: None,
        store: None,
        name: None,
        thread: None,
        recent_history_tokens: None,
        wrap_width: None,
        skills: Vec::new(),
        permission_mode: None,
        worktree_dir: None,
        no_inbox_sound: false,
        no_inbox_notification: false,
        skip_summary: false,
    })) {
        Commands::Themes => execute_themes(),
        Commands::Licenses => licenses::run(),
        Commands::Ask(args) => execute_ask(args),
        Commands::Chat(args) => execute_chat(args),
        Commands::Wire(args) => wire::run(args),
        Commands::Thread(args) => chat::thread::run(args),
        Commands::Inbox(args) => inbox_post::run(args),
        Commands::Config(args) => execute_config(args),
        Commands::Worker(args) => execute_worker(args),
        Commands::Upgrade(args) => upgrade::run(args),
        Commands::ChatMcp(args) => execute_chat_mcp(args),
        Commands::ChatNap(args) => execute_chat_nap(args),
        Commands::ChatTag(args) => execute_chat_tag(args),
        Commands::Mermaid(args) => execute_mermaid(args),
        Commands::WorkerMcp(args) => execute_worker_mcp(args),
        Commands::WorkerStopHook(args) => execute_worker_stop_hook(args),
        Commands::HookBridge(args) => hook_bridge::execute_hook_bridge(args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn execute_chat(args: ChatArgs) -> Result<(), Box<dyn std::error::Error>> {
    match chat::run(args)? {
        chat::ChatExit::Quit => Ok(()),
        chat::ChatExit::Switch(target, opts) => {
            use std::os::unix::process::CommandExt;
            let exe = std::env::current_exe()?;
            let failed = std::process::Command::new(exe)
                .args(ralph_core::chat_switch::switch_argv(&opts, &target))
                .exec();
            Err(format!("could not start the new chat: {failed}").into())
        }
    }
}

fn execute_worker(args: WorkerArgs) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        WorkerCommand::Verify {
            agent,
            live,
            drift,
            inbox,
        } => ralph_server::worker_verify::run(
            match agent {
                VerifyAgent::Cursor => HarnessId::Cursor,
                VerifyAgent::Codex => HarnessId::Codex,
                VerifyAgent::ClaudeHeadless => HarnessId::ClaudeHeadless,
                VerifyAgent::Opencode => HarnessId::Opencode,
                VerifyAgent::Claude => HarnessId::Claude,
            },
            live,
            drift
                .zip(inbox)
                .map(|(version, inbox)| DriftRun::new(version, inbox)),
        ),
        WorkerCommand::DriftCheck { inbox } => {
            ralph_server::version_drift::run_daily(&inbox);
            Ok(())
        }
    }
}

fn execute_config(args: cli::ConfigArgs) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        cli::ConfigCommand::Check { path, store } => config::run_check(path, store),
        cli::ConfigCommand::Show { store } => config::run_show(store),
    }
}

/// Execute the themes command.
///
/// Lists all available syntax highlighting themes.
fn execute_themes() -> Result<(), Box<dyn std::error::Error>> {
    let themes = highlight::Highlighter::available_themes();

    println!("Available syntax highlighting themes:\n");
    for theme in &themes {
        if *theme == highlight::DEFAULT_THEME {
            println!("  {} (default)", theme);
        } else {
            println!("  {}", theme);
        }
    }

    println!();
    println!("Use --theme <NAME> to select a theme for 'ralph ask'.");
    println!("Or set the RALPH_THEME environment variable.");
    println!();
    println!("You can also load custom .tmTheme files by specifying a file path.");

    Ok(())
}

fn execute_mermaid(args: MermaidArgs) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{Read, Write};
    if args.picture {
        let request = serde_json::from_reader(std::io::stdin())?;
        let mut fonts = resvg::usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        let png = render::mermaid_picture::rasterize(&request, fonts)?;
        std::io::stdout().write_all(&png)?;
        return Ok(());
    }
    let mut source = String::new();
    std::io::stdin().read_to_string(&mut source)?;
    println!("{}", mermaid_text::render_with_width(&source, args.width)?);
    Ok(())
}

fn execute_ask(args: AskArgs) -> Result<(), Box<dyn std::error::Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");

    let config = build_invocation_config(&args)?;
    let ui = *config.theme_config.ui();

    if !args.no_prompt {
        let prompt_display = startup::PromptDisplay::from_prompt(&config.prompt);
        startup::display_prompt(&prompt_display, &ui);
    }

    let outcome = invoke::invoke(config);
    if let Some(done) = subprocess::recorded(&outcome) {
        record_ask(done);
    }
    let result = outcome?;
    let costs = &result.stream_result.costs;
    startup::display_ask_summary(
        &startup::AskSummary {
            success: result.exit_code == 0,
            cost_usd: costs.cost_usd,
            duration_ms: costs.duration_ms,
            input_tokens: costs.usage.as_ref().map(|u| u.input_tokens),
            output_tokens: costs.usage.as_ref().map(|u| u.output_tokens),
        },
        &ui,
    );

    if result.exit_code != 0 {
        return Err(format!("LLM subprocess exited with code {}", result.exit_code).into());
    }

    Ok(())
}

fn record_ask(result: &subprocess::StreamingSubprocessResult) {
    let costs = &result.stream_result.costs;
    let at = chrono::Utc::now().to_rfc3339();
    if let Some(record) = ralph_core::chat_memory::usage_record(
        ralph_core::chat_memory::CallSite::Ask,
        at.clone(),
        costs,
        &result.stream_result.metadata,
    ) {
        warn_if_err(
            ralph_server::ledger::record(
                &ralph_server::chat::memory::usage_path(),
                &record.with_request_at(result.request_at.map(|at| at.to_rfc3339())),
            ),
            "Failed to record ask usage",
        );
    }
    warn_if_err(
        ralph_server::ledger::record_all(
            &ralph_server::chat::memory::usage_path(),
            ralph_core::chat_memory::subagent_records(&at, costs, &result.stream_result.metadata),
        ),
        "Failed to record ask subagent usage",
    );
}

fn build_invocation_config(args: &AskArgs) -> Result<InvocationConfig, Box<dyn std::error::Error>> {
    let theme_config = highlight::ThemeConfig::from_config_and_env()
        .merge_cli(args.theme.as_deref(), args.no_background);
    let verbose_tools = VerboseToolsConfig::from_arg(args.verbose_tools.as_deref());
    for warning in verbose_tools.warnings() {
        warn(warning);
    }
    let app_config = ralph_server::config::AppConfig::load().unwrap_or_default();
    warn::set_warning_color(theme_config.ui().warning);
    Ok(InvocationConfig {
        prompt: resolve_ask_prompt(args.prompt.as_deref())?,
        timeout_secs: args.timeout,
        theme_config,
        verbose_tools,
        permission_mode: app_config.ask_permission_mode(
            args.permission_mode.as_deref(),
            std::env::var(invoke::ASK_PERMISSION_MODE_ENV)
                .ok()
                .as_deref(),
        ),
        config_dir: app_config.claude.config_dir,
    })
}

// Re-export for local use and tests
use prompt_source::{classify_prompt_source, read_from_source, PromptSource};

/// Resolve prompt for the ask command.
///
/// # Prompt Resolution
/// 1. If argument is "-", read from stdin
/// 2. If argument is a file path, read from file
/// 3. If argument is any other string, use as inline prompt
/// 4. If argument is None and stdin is piped (non-terminal), read from stdin
/// 5. If argument is None and stdin is a terminal, error (no prompt provided)
///
/// After resolution, validates that the prompt is not empty.
fn resolve_ask_prompt(prompt_arg: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    use std::io::IsTerminal;

    let prompt = match prompt_arg {
        // Explicit argument provided - use classify_prompt_source
        Some(_) => {
            let source = classify_prompt_source(prompt_arg);
            read_from_source(source, None)?
        }
        // No argument - check for piped stdin
        None => {
            if std::io::stdin().is_terminal() {
                // Interactive terminal with no prompt - error
                return Err("No prompt provided. Usage: ralph ask 'your prompt' or echo 'your prompt' | ralph ask".into());
            }
            // Non-interactive (piped) - read from stdin
            read_from_source(PromptSource::Stdin, None)?
        }
    };

    // Validate non-empty
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Err("Prompt cannot be empty. Provide a prompt as an argument or via stdin.".into());
    }

    Ok(prompt)
}

fn execute_chat_mcp(args: ChatMcpArgs) -> Result<(), Box<dyn std::error::Error>> {
    ralph_server::config::set_store_layer(&args.store);
    let memory = chat::thread::Memory::open(&args.store)?;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    chat::mcp::run_stdio_server(&memory, &args.socket, stdin.lock(), stdout.lock())?;
    Ok(())
}

fn execute_worker_mcp(args: WorkerMcpArgs) -> Result<(), Box<dyn std::error::Error>> {
    let site = args
        .worktree
        .zip(args.inbox.parent())
        .map(|(worktree, store)| ralph_core::snapshot::WorkerSite {
            store: store.to_path_buf(),
            id: args.id.clone(),
            worktree,
        });
    let changes = site
        .as_ref()
        .and_then(ralph_server::worker_changes::WorkerChanges::open);
    let inbox = ralph_server::worker_mcp::Inbox::new(args.inbox).requiring_owner(args.owner);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    ralph_server::worker_mcp::run_stdio_server(
        &args.id,
        &inbox,
        changes.as_ref(),
        stdin.lock(),
        stdout.lock(),
    )?;
    Ok(())
}

fn execute_worker_stop_hook(args: WorkerStopHookArgs) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(store) = args.layer_store() {
        ralph_server::config::set_store_layer(store);
    }
    let store = args.store.clone();
    let usage_file = args.usage;
    let usage = args.worker.zip(args.usage_seen).map(|(worker, seen)| {
        ralph_server::worker_usage::UsageTarget {
            worker,
            seen,
            store,
            usage: usage_file.unwrap_or_else(ralph_server::chat::memory::usage_path),
        }
    });
    let changes = args
        .store
        .zip(args.id)
        .zip(args.worktree)
        .map(|((store, id), worktree)| ralph_core::snapshot::WorkerSite {
            store,
            id,
            worktree,
        })
        .as_ref()
        .and_then(ralph_server::worker_changes::WorkerChanges::open);
    ralph_server::worker_mcp::run_stop_hook(
        &ralph_server::worker_mcp::StopHook {
            reported: &args.reported,
            unreported: &args.unreported,
            activity: args.activity.as_deref(),
            changes: changes.as_ref(),
            usage: usage.as_ref(),
        },
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )?;
    Ok(())
}

fn execute_chat_nap(args: ChatNapArgs) -> Result<(), Box<dyn std::error::Error>> {
    ralph_server::config::set_store_layer(&args.store);
    let store = ralph_server::chat::memory::Store::new(args.store.clone());
    store.nap(|prompt| {
        run_curator(
            prompt,
            &Curator {
                kind: ralph_core::chat_memory::CallSite::Nap,
                store: &args.store,
                run: args.run.clone(),
                timeout: args.timeout,
            },
        )
    })?;
    Ok(())
}

fn execute_chat_tag(args: ChatTagArgs) -> Result<(), Box<dyn std::error::Error>> {
    ralph_server::config::set_store_layer(&args.store);
    let store = chat::tag_store::TagStore::new(args.store.clone());
    let now = || chrono::Utc::now().to_rfc3339();
    let (loaded, notices) = ralph_server::config::AppConfig::load_with_notices();
    let store_failed = notices
        .iter()
        .any(ralph_server::config::Notice::is_store_fallback);
    match ralph_core::chat_memory::tag_plan(loaded.map(|c| c.chat.tag_entries), store_failed) {
        ralph_core::chat_memory::TagPlan::Run => {}
        ralph_core::chat_memory::TagPlan::SkipAhead => {
            store.skip_ahead(&now)?;
            return Ok(());
        }
        ralph_core::chat_memory::TagPlan::Skip => return Ok(()),
    }
    let curator = Curator {
        kind: ralph_core::chat_memory::CallSite::Tag,
        store: &args.store,
        run: args.run.clone(),
        timeout: args.timeout,
    };
    let command = tag_command();
    let pass = store.run_pass(&now, &mut |batch| {
        let run = run_memory_command(&command, batch, &curator)?;
        ralph_core::chat_memory::tag_reply(
            run.failure,
            signal::is_terminated(),
            run.code,
            run.text.as_deref(),
        )
    })?;
    if let chat::tag_store::Pass::Done(report) = pass {
        if report.failed > 0 {
            ralph_server::warn::warn(format!(
                "chat tagging could not tag {} entries this pass",
                report.failed
            ));
        }
    }
    Ok(())
}

fn tag_command() -> String {
    let config_dir = ralph_server::config::AppConfig::load()
        .unwrap_or_default()
        .claude
        .config_dir;
    invoke::build_tag_command(
        ralph_core::chat_memory::TAG_MODEL,
        ralph_core::chat_memory::TAG_PROMPT,
        dirs::home_dir().as_deref(),
        config_dir.as_deref(),
    )
}

pub(crate) struct Curator<'a> {
    pub kind: ralph_core::chat_memory::CallSite,
    pub store: &'a std::path::Path,
    pub run: Option<String>,
    pub timeout: u64,
}

struct CuratorRows {
    kind: ralph_core::chat_memory::CallSite,
    run: Option<String>,
    store: String,
    at: String,
}

fn curator_rows(
    rows: &CuratorRows,
    done: &subprocess::StreamingSubprocessResult,
) -> Vec<ralph_core::chat_memory::LedgerRow> {
    let costs = &done.stream_result.costs;
    let metadata = &done.stream_result.metadata;
    ralph_core::chat_memory::usage_record(rows.kind.clone(), rows.at.clone(), costs, metadata)
        .into_iter()
        .chain(ralph_core::chat_memory::subagent_records(
            &rows.at, costs, metadata,
        ))
        .map(|row| {
            row.with_run(rows.run.clone())
                .with_store(Some(rows.store.clone()))
        })
        .collect()
}

pub(crate) fn run_curator(prompt: &str, curator: &Curator) -> Result<String, String> {
    let config_dir = ralph_server::config::AppConfig::load()
        .unwrap_or_default()
        .claude
        .config_dir;
    let command = invoke::build_memory_command(dirs::home_dir().as_deref(), config_dir.as_deref());
    let run = run_memory_command(&command, prompt, curator)?;
    ralph_core::chat_memory::nap_summary(run.code, run.text.as_deref())
}

/// What a finished memory call left behind. `failure` is how the final
/// attempt was classified, so a caller can tell an outage that ran out of
/// retries from an ordinary failure.
struct MemoryRun {
    code: i32,
    text: Option<String>,
    failure: Option<ralph_core::retry::FailureKind>,
}

/// Runs one memory call. An error means the call never finished (timeout or a
/// spawn error); the exit code and reply text are left to the caller.
fn run_memory_command(command: &str, prompt: &str, curator: &Curator) -> Result<MemoryRun, String> {
    let theme_config = highlight::ThemeConfig::from_config_and_env();
    let stdin = invoke::prompt_stdin(prompt);
    let outcome = backoff::retry_on_outage(
        "chat memory curator",
        || {
            backoff::attempt_from(subprocess::invoke_subprocess_silent(
                &subprocess::SpinnerSubprocessConfig {
                    command: command.to_string(),
                    timeout_secs: Some(curator.timeout),
                    theme_config: theme_config.clone(),
                    session_elapsed_ms: 0,
                    verbose_tools: stream_processor::VerboseToolsConfig::new(),
                    session_info: spinner::SpinnerSessionInfo {
                        persona: Some("chat memory".into()),
                    },
                    gate: None,
                    cancel: None,
                    stdin: Some(stdin.clone()),
                },
            ))
        },
        |delay| backoff::wait_unless(signal::is_terminated, delay),
    );
    if let Some(done) = subprocess::recorded(&outcome) {
        let rows = CuratorRows {
            kind: curator.kind.clone(),
            run: curator.run.clone(),
            store: ralph_server::workers::usage_store(curator.store),
            at: chrono::Utc::now().to_rfc3339(),
        };
        warn_if_err(
            ralph_server::ledger::record_all(
                &ralph_server::chat::memory::usage_path(),
                curator_rows(&rows, done),
            ),
            "Failed to record chat usage",
        );
    }
    let failure = backoff::final_tag_failure_kind(&outcome);
    let result = outcome.map_err(|e| e.to_string())?;
    let response = stream_processor::extract_response_text(&result.stream_result.output_blocks);
    Ok(MemoryRun {
        code: result.exit_code,
        text: response,
        failure,
    })
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
