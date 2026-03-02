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
mod cli;
pub mod config;
pub mod diff_highlight;
pub mod formatting;
mod git;
pub mod highlight;
mod init;
mod invoke;
pub mod iteration;
pub mod iterations;
pub mod keyboard;
pub mod markdown;
mod orchestrator;
pub mod paths;
mod persona;
mod prompt;
pub mod prompt_source;
pub mod render;
pub mod replay;
pub mod replay_countdown;
pub mod replay_renderer;
mod run;
mod session;
pub mod sessions_display;
pub mod signal;
pub mod spinner;
pub mod startup;
pub mod stream_processor;
pub mod subprocess;
pub mod warn;

use clap::Parser;
use cli::{
    AskArgs, Cli, Commands, IterationsArgs, PersonaAction, PersonaArgs, PersonaInvokeArgs,
    ReplayArgs, RunArgs, SessionsArgs,
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

    let result = match cli.command {
        Commands::Run(args) => execute_run(args),
        Commands::Sessions(args) => execute_sessions(args),
        Commands::Iterations(args) => execute_iterations(args),
        Commands::Replay(args) => execute_replay(args),
        Commands::Themes => execute_themes(),
        Commands::Ask(args) => execute_ask(args),
        Commands::Persona(args) => execute_persona(args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::FAILURE
        }
    }
}

/// Execute the run command.
///
/// Delegates to the `run::execute` module which handles all run execution logic
/// including failure recovery prompting, resume functionality, and hard stop handling.
fn execute_run(args: RunArgs) -> Result<(), Box<dyn std::error::Error>> {
    run::execute::execute_run(args)
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

/// Execute the iterations command.
///
/// Lists all iterations across all sessions, optionally filtered by
/// session, project, or outcome.
fn execute_iterations(args: IterationsArgs) -> Result<(), Box<dyn std::error::Error>> {
    let filter = iterations::IterationsFilter {
        session: args.session,
        project: args.project,
        outcome: args.outcome,
    };

    iterations::list_iterations(filter)?;

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
    println!("Use --theme <NAME> to select a theme for 'ralph run'.");
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

/// Shared parameters for building an `InvocationConfig`.
///
/// Captures the common fields between `AskArgs` and `PersonaArgs` so that
/// `build_shared_invocation_config` can handle the duplicated logic once.
struct InvocationConfigParams<'a> {
    session: &'a Option<String>,
    continue_session: bool,
    clone_session: bool,
    theme: Option<&'a str>,
    no_background: bool,
    verbose_tools: Option<&'a str>,
    prompt: Option<&'a str>,
    timeout: u64,
    persona: Option<&'a str>,
    permission_mode: String,
}

fn build_shared_invocation_config(
    params: &InvocationConfigParams,
    project_path: &Path,
) -> Result<InvocationConfig, Box<dyn std::error::Error>> {
    let clone_info = if params.clone_session {
        Some(resolve_clone_source(
            params.session,
            project_path,
            params.persona,
        )?)
    } else {
        None
    };
    let continuation = if params.continue_session && !params.clone_session {
        Some(resolve_continuation(
            params.session,
            project_path,
            params.persona,
        )?)
    } else {
        None
    };
    let theme_config =
        highlight::ThemeConfig::from_config_and_env().merge_cli(params.theme, params.no_background);
    let verbose_tools = VerboseToolsConfig::from_arg(params.verbose_tools);
    for warning in verbose_tools.warnings() {
        warn(warning);
    }
    let prompt = resolve_ask_prompt(params.prompt)?;
    Ok(InvocationConfig {
        prompt,
        timeout_secs: params.timeout,
        theme_config,
        verbose_tools,
        project_path: project_path.to_path_buf(),
        slug: if params.continue_session || params.clone_session {
            None
        } else {
            params.session.clone()
        },
        continuation,
        clone: clone_info,
        permission_mode: params.permission_mode.clone(),
        persona: params.persona.map(String::from),
    })
}

fn build_invocation_config(
    args: &AskArgs,
    project_path: &Path,
) -> Result<InvocationConfig, Box<dyn std::error::Error>> {
    build_shared_invocation_config(
        &InvocationConfigParams {
            session: &args.session,
            continue_session: args.continue_session,
            clone_session: args.clone_session,
            theme: args.theme.as_deref(),
            no_background: args.no_background,
            verbose_tools: args.verbose_tools.as_deref(),
            prompt: args.prompt.as_deref(),
            timeout: args.timeout,
            persona: None,
            permission_mode: ask::resolve_permission_mode(args.permission_mode.as_deref()),
        },
        project_path,
    )
}

fn execute_and_finalize(config: InvocationConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Extract fields needed for orchestration before invoke() takes ownership.
    let orch_config = orchestrator::OrchestrationConfig {
        project_path: config.project_path.clone(),
        timeout_secs: config.timeout_secs,
        theme_config: config.theme_config.clone(),
        verbose_tools: config.verbose_tools.clone(),
        budget: orchestrator::Budget::new(orchestrator::DEFAULT_BUDGET),
    };

    let result = invoke::invoke(config)?;

    // Scan for directives and orchestrate if found
    if let Some(directives) = orchestrator::scan_for_directives(&result) {
        if let Err(e) = orchestrator::orchestrate(&result, directives, &orch_config) {
            warn(format!("Orchestration failed: {e}"));
        }
    }

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
    persona: Option<&str>,
) -> Result<ralph_core::session::SessionEntry, Box<dyn std::error::Error>> {
    if let Some(name) = session_name {
        Ok(session::find_session_by_slug(name)?)
    } else {
        session::find_most_recent_session(project_path, persona)?.ok_or_else(|| {
            match persona {
                Some(p) => format!(
                    "No sessions found for persona '{}' in project '{}'. Start a conversation first.",
                    p, project_path.display()
                ),
                None => format!(
                    "No sessions found for project '{}'. Create a session first with 'ralph ask'.",
                    project_path.display()
                ),
            }
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
    persona: Option<&str>,
) -> Result<invoke::ContinuationInfo, Box<dyn std::error::Error>> {
    use crate::iteration::count_iterations;

    let entry = find_session_entry(session_name, project_path, persona)?;
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
    persona: Option<&str>,
) -> Result<invoke::CloneInfo, Box<dyn std::error::Error>> {
    let entry = find_session_entry(session_name, project_path, persona)?;
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
    let entry = find_session_entry(&args.session, project_path, None)?;

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

// =============================================================================
// Persona command
// =============================================================================

fn execute_persona(args: PersonaArgs) -> Result<(), Box<dyn std::error::Error>> {
    let action = args
        .into_action()
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

    match action {
        PersonaAction::List => execute_persona_list(),
        PersonaAction::Invoke(invoke_args) => {
            warn_if_err(signal::init(), "Failed to initialize signal handler");

            let project_path = std::env::current_dir()?;

            // Verify agent file exists before doing anything else
            persona::verify_agent_file(&invoke_args.persona, &project_path)?;

            if invoke_args.history {
                return execute_persona_with_history(&invoke_args, &project_path);
            }

            execute_persona_core(&invoke_args, &project_path)
        }
    }
}

fn execute_persona_list() -> Result<(), Box<dyn std::error::Error>> {
    let project_path = std::env::current_dir()?;
    let personas = persona::discover_personas(&project_path);
    let output = persona::format_persona_list(&personas);
    print!("{output}");
    Ok(())
}

fn execute_persona_core(
    args: &PersonaInvokeArgs,
    project_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if args.clone_session && args.session.is_none() && !args.continue_session {
        return Err(
            "--clone requires --session or --continue to specify which session to clone from"
                .into(),
        );
    }

    let config = build_persona_config(args, project_path)?;

    if !args.no_prompt && !config.prompt.is_empty() {
        let prompt_display = startup::PromptDisplay::from_prompt(&config.prompt);
        startup::display_prompt(&prompt_display);
    }

    execute_and_finalize(config)
}

fn build_persona_config(
    args: &PersonaInvokeArgs,
    project_path: &Path,
) -> Result<InvocationConfig, Box<dyn std::error::Error>> {
    build_shared_invocation_config(
        &InvocationConfigParams {
            session: &args.session,
            continue_session: args.continue_session,
            clone_session: args.clone_session,
            theme: args.theme.as_deref(),
            no_background: args.no_background,
            verbose_tools: args.verbose_tools.as_deref(),
            prompt: args.prompt.as_deref(),
            timeout: args.timeout,
            persona: Some(&args.persona),
            permission_mode: invoke::DEFAULT_PERMISSION_MODE.to_string(),
        },
        project_path,
    )
}

fn execute_persona_with_history(
    args: &PersonaInvokeArgs,
    project_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if args.session.is_none() && !args.continue_session {
        return Err(
            "--history requires --session or --continue to specify which session to display".into(),
        );
    }

    let entry = find_session_entry(&args.session, project_path, Some(&args.persona))?;
    let session_dir = session::session_dir(&entry.slug);
    let logs = load_session_iterations(&session_dir)?;
    let messages = extract_conversation_messages(&logs);

    let history = startup::ConversationHistory::from_messages(entry.slug.clone(), messages);
    startup::display_conversation_history(&history);

    if args.continue_session && args.prompt.is_some() {
        return execute_persona_core(args, project_path);
    }

    Ok(())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
