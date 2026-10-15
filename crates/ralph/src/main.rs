// Deny .unwrap() and .expect() in non-test code to ensure proper error handling.
// Test code may still use them for brevity.
// Any intentional uses must be documented with #[allow(...)] and comments.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
// Functions should have at most 5 arguments. Use config/options structs for more.
// Threshold configured in clippy.toml (too-many-arguments-threshold = 5).
#![cfg_attr(not(test), deny(clippy::too_many_arguments))]

pub mod ansi;
mod chat;
mod chat_cell;
mod chat_check;
mod chat_inbox;
mod chat_mcp;
mod chat_memory;
mod chat_route;
mod chat_watch;
mod cli;
mod clipboard;
pub mod config;
pub mod diff_highlight;
pub mod formatting;
mod frame;
pub mod highlight;
mod inbox_alert;
mod invoke;
pub mod markdown;
mod mermaid_guard;
pub mod paths;
pub mod prompt_source;
pub mod render;
pub mod signal;
pub mod spinner;
pub mod startup;
pub mod stream_processor;
pub mod subprocess;
mod version;
pub mod warn;
mod watch_requests;
mod watch_store;
mod watches;
mod worker_mcp;
mod workers;

use clap::Parser;
use cli::{
    AskArgs, ChatArgs, ChatMcpArgs, ChatNapArgs, Cli, Commands, MermaidArgs, WorkerMcpArgs,
    WorkerStopHookArgs,
};
use invoke::InvocationConfig;
use std::process::ExitCode;
use stream_processor::VerboseToolsConfig;
use warn::{warn, warn_if_err};

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command.unwrap_or(Commands::Chat(ChatArgs {
        store: None,
        timeout: cli::CHAT_TIMEOUT_SECS,
        frozen_window: None,
        wrap_width: None,
        skills: Vec::new(),
        permission_mode: None,
        worktree_dir: None,
        no_inbox_sound: false,
        no_inbox_notification: false,
    })) {
        Commands::Themes => execute_themes(),
        Commands::Ask(args) => execute_ask(args),
        Commands::Chat(args) => chat::run(args),
        Commands::ChatMcp(args) => execute_chat_mcp(args),
        Commands::ChatNap(args) => execute_chat_nap(args),
        Commands::Mermaid(args) => execute_mermaid(args),
        Commands::WorkerMcp(args) => execute_worker_mcp(args),
        Commands::WorkerStopHook(args) => execute_worker_stop_hook(args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
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
    use std::io::Read;
    let mut source = String::new();
    std::io::stdin().read_to_string(&mut source)?;
    println!(
        "{}",
        mermaid_text::render_with_width(&source, Some(args.width))?
    );
    Ok(())
}

fn execute_ask(args: AskArgs) -> Result<(), Box<dyn std::error::Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");

    let config = build_invocation_config(&args)?;

    if !args.no_prompt {
        let prompt_display = startup::PromptDisplay::from_prompt(&config.prompt);
        startup::display_prompt(&prompt_display);
    }

    let result = invoke::invoke(config)?;
    let costs = &result.stream_result.costs;
    startup::display_ask_summary(&startup::AskSummary {
        success: result.exit_code == 0,
        cost_usd: costs.cost_usd,
        duration_ms: costs.duration_ms,
        input_tokens: costs.usage.as_ref().map(|u| u.input_tokens),
        output_tokens: costs.usage.as_ref().map(|u| u.output_tokens),
    });

    if result.exit_code != 0 {
        return Err(format!("LLM subprocess exited with code {}", result.exit_code).into());
    }

    Ok(())
}

fn build_invocation_config(args: &AskArgs) -> Result<InvocationConfig, Box<dyn std::error::Error>> {
    let theme_config = highlight::ThemeConfig::from_config_and_env()
        .merge_cli(args.theme.as_deref(), args.no_background);
    let verbose_tools = VerboseToolsConfig::from_arg(args.verbose_tools.as_deref());
    for warning in verbose_tools.warnings() {
        warn(warning);
    }
    Ok(InvocationConfig {
        prompt: resolve_ask_prompt(args.prompt.as_deref())?,
        timeout_secs: args.timeout,
        theme_config,
        verbose_tools,
        permission_mode: config::AppConfig::load()
            .unwrap_or_default()
            .ask_permission_mode(
                args.permission_mode.as_deref(),
                std::env::var(invoke::ASK_PERMISSION_MODE_ENV)
                    .ok()
                    .as_deref(),
            ),
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
    let store = chat_memory::Store::new(args.store.clone());
    let workers = workers::Workers::new(
        args.store,
        std::env::current_exe()?,
        args.permission_mode,
        args.worktree_dir,
    );
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    chat_mcp::run_stdio_server(&store, &workers, stdin.lock(), stdout.lock())?;
    Ok(())
}

fn execute_worker_mcp(args: WorkerMcpArgs) -> Result<(), Box<dyn std::error::Error>> {
    let inbox = worker_mcp::Inbox::new(args.inbox);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    worker_mcp::run_stdio_server(&args.id, &inbox, stdin.lock(), stdout.lock())?;
    Ok(())
}

fn execute_worker_stop_hook(args: WorkerStopHookArgs) -> Result<(), Box<dyn std::error::Error>> {
    worker_mcp::run_stop_hook(
        &args.reported,
        &args.unreported,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )?;
    Ok(())
}

fn execute_chat_nap(args: ChatNapArgs) -> Result<(), Box<dyn std::error::Error>> {
    let store = chat_memory::Store::new(args.store);
    let theme_config = highlight::ThemeConfig::from_config_and_env();
    store.nap(|prompt| {
        let command = invoke::build_memory_command(prompt, dirs::home_dir().as_deref());
        let result = subprocess::invoke_subprocess_with_spinner_config(
            &subprocess::SpinnerSubprocessConfig {
                command,
                timeout_secs: args.timeout,
                theme_config: theme_config.clone(),
                session_elapsed_ms: 0,
                verbose_tools: stream_processor::VerboseToolsConfig::new(),
                session_info: spinner::SpinnerSessionInfo {
                    persona: Some("chat memory".into()),
                },
                pause: None,
                cancel: None,
            },
        )
        .map_err(|e| e.to_string())?;
        if let Some(record) = ralph_core::chat_memory::usage_record(
            ralph_core::chat_memory::CallKind::Nap,
            chrono::Utc::now().to_rfc3339(),
            &result.stream_result.costs,
        ) {
            warn_if_err(store.append_usage(&record), "Failed to record chat usage");
        }
        let response = stream_processor::extract_response_text(&result.stream_result.output_blocks);
        ralph_core::chat_memory::nap_summary(result.exit_code, response.as_deref())
    })?;
    Ok(())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
