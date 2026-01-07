mod cli;
mod git;
mod init;
mod iteration;
mod run;
mod session;
mod subprocess;

use clap::Parser;
use cli::{Cli, Commands, RunArgs};
use ralph_core::context::{defaults, substitute_template_placeholders, ContextPaths};
use run::{run, RunConfig};
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run(args) => execute_run(args),
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
fn execute_run(args: RunArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Resolve context file paths
    let project_root = std::env::current_dir()?;
    let context_paths = ContextPaths::new(
        &project_root,
        args.design.as_deref(),
        args.prd.as_deref(),
        args.progress.as_deref(),
    );

    // Determine command template
    let command_template = args
        .command
        .unwrap_or_else(|| defaults::COMMAND_TEMPLATE.to_string());

    // Determine completion marker
    let completion_marker = args
        .completion_marker
        .unwrap_or_else(|| defaults::COMPLETION_MARKER.to_string());

    // Resolve prompt template and substitute placeholders
    let prompt = resolve_prompt(args.prompt.as_deref(), &context_paths, &completion_marker)?;

    // Substitute {prompt} in command template
    let command = substitute_prompt_in_command(&command_template, &prompt);

    // Build run config
    let config = RunConfig {
        max_iterations: args.iterations,
        slug: args.slug,
        command,
        completion_marker,
        context_paths,
    };

    // Execute the run loop
    let result = run(config)?;

    // Print summary
    println!("Session: {}", result.slug);
    println!("Iterations completed: {}", result.iterations_completed);
    if let Some(reason) = result.completion_reason {
        println!("Completion reason: {:?}", reason);
    }

    Ok(())
}

/// Resolve the prompt from various sources.
///
/// Loads the prompt template from one of three sources:
/// - A file path (if the argument is a path to an existing file)
/// - Stdin (if the argument is "-")
/// - An inline string (if the argument doesn't match a file)
/// - The default template (if no argument is provided)
///
/// After loading the template, placeholders are substituted with actual values:
/// - `{design_file}` - Path to the design document
/// - `{prd_file}` - Path to the PRD file
/// - `{progress_file}` - Path to the progress notes file
/// - `{completion_marker}` - The completion marker string
fn resolve_prompt(
    prompt_arg: Option<&str>,
    context_paths: &ContextPaths,
    completion_marker: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let template = match prompt_arg {
        Some("-") => {
            // Read from stdin
            use std::io::Read;
            let mut prompt = String::new();
            std::io::stdin().read_to_string(&mut prompt)?;
            prompt
        }
        Some(value) => {
            // Check if it's a file path
            let path = Path::new(value);
            if path.exists() && path.is_file() {
                std::fs::read_to_string(path)?
            } else {
                // Treat as inline string
                value.to_string()
            }
        }
        None => {
            // Use built-in default prompt template
            defaults::PROMPT_TEMPLATE.to_string()
        }
    };

    // Substitute placeholders in the template
    Ok(substitute_template_placeholders(
        &template,
        context_paths,
        completion_marker,
    ))
}

/// Substitute {prompt} placeholder in command template.
fn substitute_prompt_in_command(template: &str, prompt: &str) -> String {
    // Shell-escape the prompt for safe inclusion
    // For now, just wrap in single quotes and escape internal single quotes
    let escaped = prompt.replace('\'', "'\"'\"'");
    let quoted_prompt = format!("'{}'", escaped);
    template.replace("{prompt}", &quoted_prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substitute_prompt_simple() {
        let result = substitute_prompt_in_command("echo {prompt}", "hello");
        assert_eq!(result, "echo 'hello'");
    }

    #[test]
    fn test_substitute_prompt_with_quotes() {
        let result = substitute_prompt_in_command("echo {prompt}", "it's a test");
        assert_eq!(result, "echo 'it'\"'\"'s a test'");
    }

    #[test]
    fn test_substitute_prompt_claude_command() {
        let result = substitute_prompt_in_command(
            "claude --permission-mode acceptEdits --output-format stream-json -p {prompt}",
            "test prompt",
        );
        assert_eq!(
            result,
            "claude --permission-mode acceptEdits --output-format stream-json -p 'test prompt'"
        );
    }
}
