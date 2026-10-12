// Deny .unwrap() and .expect() in non-test code to ensure proper error handling.
// Test code may still use them for brevity.
// Any intentional uses must be documented with #[allow(...)] and comments.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
// Functions should have at most 5 arguments. Use config/options structs for more.
// Threshold configured in clippy.toml (too-many-arguments-threshold = 5).
#![cfg_attr(not(test), deny(clippy::too_many_arguments))]

pub mod ansi;
mod ask;
mod chat;
mod chat_mcp;
mod chat_memory;
mod cli;
pub mod config;
pub mod diff_highlight;
pub mod formatting;
pub mod highlight;
mod invoke;
pub mod iteration;
pub mod markdown;
pub mod paths;
pub mod prompt_source;
pub mod render;
pub mod replay;
pub mod replay_countdown;
pub mod replay_renderer;
mod session;
pub mod sessions_display;
pub mod signal;
pub mod spinner;
pub mod startup;
pub mod stream_processor;
pub mod subprocess;
pub mod warn;
mod worker_mcp;
mod workers;

use clap::Parser;
use cli::{
    AskArgs, ChatArgs, ChatMcpArgs, ChatNapArgs, Cli, Commands, ReplayArgs, SessionsArgs,
    WorkerMcpArgs,
};
use invoke::InvocationConfig;
use iteration::{extract_conversation_messages, load_session_iterations};
use ralph_core::session::SessionOutcome;
use std::path::Path;
use std::process::ExitCode;
use stream_processor::VerboseToolsConfig;
use warn::{warn, warn_if_err};

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command.unwrap_or(Commands::Chat(ChatArgs {
        store: None,
        timeout: cli::CHAT_TIMEOUT_SECS,
        frozen_window: None,
        skills: Vec::new(),
    })) {
        Commands::Sessions(args) => execute_sessions(args),
        Commands::Replay(args) => execute_replay(args),
        Commands::Themes => execute_themes(),
        Commands::Ask(args) => execute_ask(args),
        Commands::Chat(args) => chat::run(args),
        Commands::ChatMcp(args) => execute_chat_mcp(args),
        Commands::ChatNap(args) => execute_chat_nap(args),
        Commands::WorkerMcp(args) => execute_worker_mcp(args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

/// Execute the sessions command.
///
/// Lists all sessions from the global sessions index with detailed metadata
/// (cost, duration, tokens), optionally filtered by project path or outcome.
fn execute_sessions(args: SessionsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let filter = sessions_display::SessionsFilter {
        project: args.project,
        outcome: args.outcome,
    };

    sessions_display::list_sessions(filter)?;

    Ok(())
}

/// Execute the replay command.
///
/// Replays a session's output with syntax highlighting. Reads iteration logs
/// from the session directory and re-renders the chunks.
fn execute_replay(args: ReplayArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Build theme configuration from config file, env vars, and CLI args
    let theme_config = Some(
        highlight::ThemeConfig::from_config_and_env()
            .merge_cli(args.theme.as_deref(), args.no_background),
    );

    let options = replay::ReplayOptions::new()
        .with_iteration(args.iteration)
        .with_theme(theme_config)
        .with_show_prompt(!args.no_prompt)
        .with_delay(args.delay);

    let result = replay::replay_session_with_options(&args.slug, options)?;

    // Print summary
    println!();
    println!("{}", "─".repeat(60));
    println!(
        "Replayed {} iteration(s) from session '{}'",
        result.iterations_replayed, result.slug
    );

    Ok(())
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

/// Execute the ask command.
///
/// Sends a single-shot prompt to the LLM and displays the response.
/// Creates a session for persistence, allowing replay with `ralph replay`.
/// When --continue is used, continues an existing session instead.
/// When --clone is used, creates a new session with history from an existing one.
/// When --history is used, displays the conversation history for the session.
fn execute_ask(args: AskArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize signal handler for graceful shutdown on Ctrl+C/SIGTERM
    warn_if_err(signal::init(), "Failed to initialize signal handler");

    // Get current working directory as project path
    let project_path = std::env::current_dir()?;

    // Handle --history flag: requires --session or --continue
    if args.history {
        return execute_ask_with_history(&args, &project_path);
    }

    execute_ask_core(&args, &project_path)
}

/// Core ask execution logic.
///
/// Validates arguments, builds configuration, displays prompt, and executes the ask command.
/// Assumes signal handler is already initialized by caller.
fn execute_ask_core(args: &AskArgs, project_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Validate --clone requires --session or --continue to specify source
    if args.clone_session && args.session.is_none() && !args.continue_session {
        return Err(
            "--clone requires --session or --continue to specify which session to clone from"
                .into(),
        );
    }

    // Build ask config from args
    let config = build_invocation_config(args, project_path)?;

    if !args.no_prompt && !config.prompt.is_empty() {
        let prompt_display = startup::PromptDisplay::from_prompt(&config.prompt);
        startup::display_prompt(&prompt_display);
    }

    execute_and_finalize(config)
}

fn build_invocation_config(
    args: &AskArgs,
    project_path: &Path,
) -> Result<InvocationConfig, Box<dyn std::error::Error>> {
    let clone_info = if args.clone_session {
        Some(resolve_clone_source(&args.session, project_path)?)
    } else {
        None
    };
    let continuation = if args.continue_session && !args.clone_session {
        Some(resolve_continuation(&args.session, project_path)?)
    } else {
        None
    };
    let theme_config = highlight::ThemeConfig::from_config_and_env()
        .merge_cli(args.theme.as_deref(), args.no_background);
    let verbose_tools = VerboseToolsConfig::from_arg(args.verbose_tools.as_deref());
    for warning in verbose_tools.warnings() {
        warn(warning);
    }
    let prompt = resolve_ask_prompt(args.prompt.as_deref())?;
    Ok(InvocationConfig {
        prompt,
        timeout_secs: args.timeout,
        theme_config,
        verbose_tools,
        project_path: project_path.to_path_buf(),
        slug: if args.continue_session || args.clone_session {
            None
        } else {
            args.session.clone()
        },
        continuation,
        clone: clone_info,
        permission_mode: ask::resolve_permission_mode(args.permission_mode.as_deref()),
    })
}

fn execute_and_finalize(config: InvocationConfig) -> Result<(), Box<dyn std::error::Error>> {
    finalize_invocation(invoke::invoke(config)?)
}

fn finalize_invocation(result: invoke::InvocationResult) -> Result<(), Box<dyn std::error::Error>> {
    let outcome = match result.exit_code {
        0 => SessionOutcome::Completed,
        _ => SessionOutcome::Failed,
    };
    warn_if_err(
        session::finalize_session(&result.slug, result.iteration_count, outcome),
        "Failed to finalize session",
    );

    let summary = startup::AskSummary {
        slug: result.slug.clone(),
        success: result.exit_code == 0,
        cost_usd: result.cost_usd,
        duration_ms: result.duration_ms,
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
    };
    startup::display_ask_summary(&summary);

    if result.exit_code != 0 {
        return Err(format!("LLM subprocess exited with code {}", result.exit_code).into());
    }

    Ok(())
}

/// Find a session entry by name or get the most recent for the project.
///
/// This is a shared helper for both continuation and clone operations.
///
/// # Logic
/// - If `session_name` is Some: find that specific session
/// - If `session_name` is None: find the most recent session for the project
///
/// # Errors
/// - Session not found (by name or no sessions for project)
fn find_session_entry(
    session_name: &Option<String>,
    project_path: &Path,
) -> Result<ralph_core::session::SessionEntry, Box<dyn std::error::Error>> {
    if let Some(name) = session_name {
        Ok(session::find_session_by_slug(name)?)
    } else {
        session::find_most_recent_session(project_path)?.ok_or_else(|| {
            format!(
                "No sessions found for project '{}'. Create a session first with 'ralph ask'.",
                project_path.display()
            )
            .into()
        })
    }
}

/// Resolve session continuation info based on --continue flag and optional --session.
///
/// # Errors
/// - Session not found (by name or no sessions for project)
/// - Failed to count existing iterations
fn resolve_continuation(
    session_name: &Option<String>,
    project_path: &Path,
) -> Result<invoke::ContinuationInfo, Box<dyn std::error::Error>> {
    use crate::iteration::count_iterations;

    let entry = find_session_entry(session_name, project_path)?;
    let session_dir = session::session_dir(&entry.slug);
    let existing_count = count_iterations(&session_dir)?;

    Ok(invoke::ContinuationInfo {
        slug: entry.slug,
        next_sequence: existing_count + 1,
        session_dir,
    })
}

fn resolve_clone_source(
    session_name: &Option<String>,
    project_path: &Path,
) -> Result<invoke::CloneInfo, Box<dyn std::error::Error>> {
    let entry = find_session_entry(session_name, project_path)?;
    let source_session_dir = session::session_dir(&entry.slug);

    Ok(invoke::CloneInfo {
        source_slug: entry.slug,
        source_session_dir,
    })
}

/// Execute ask command with --history flag.
///
/// Displays the conversation history for a session. If a prompt is also provided
/// with --continue, displays history then proceeds with the new prompt.
fn execute_ask_with_history(
    args: &AskArgs,
    project_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // --history requires --session or --continue to specify which session
    if args.session.is_none() && !args.continue_session {
        return Err(
            "--history requires --session or --continue to specify which session to display".into(),
        );
    }

    // Resolve the session to display history for
    let entry = find_session_entry(&args.session, project_path)?;

    // Load iteration logs and extract conversation history
    let session_dir = session::session_dir(&entry.slug);
    let logs = load_session_iterations(&session_dir)?;
    let messages = extract_conversation_messages(&logs);

    // Build and display conversation history
    let history = startup::ConversationHistory::from_messages(entry.slug.clone(), messages);
    startup::display_conversation_history(&history);

    // If a prompt was also provided with --continue, proceed with asking
    if args.continue_session && args.prompt.is_some() {
        return execute_ask_core(args, project_path);
    }

    Ok(())
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
    let workers = workers::Workers::new(args.store, std::env::current_exe()?);
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
                    slug: None,
                    current_iteration: None,
                    max_iterations: None,
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
        let response = iteration::extract_response_text(&result.stream_result.output_blocks);
        ralph_core::chat_memory::nap_summary(result.exit_code, response.as_deref())
    })?;
    Ok(())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
