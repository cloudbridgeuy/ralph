//! Edit a session's conversation history in $EDITOR (Imperative Shell + Functional Core).
//!
//! This module implements the `ralph edit` subcommand. It projects a session's
//! conversation into a TOML file and opens it in the user's editor. After
//! editing, changes are diffed and applied back to iteration files.
//!
//! If the edited TOML fails to parse, the user is prompted to retry (reopen
//! the editor with the broken content preserved) or abort without losing work.

mod core;
mod types;

pub use core::{
    iterations_to_messages, messages_to_edit_toml, pair_messages_to_iterations, parse_edit_toml,
    plan_iteration_updates,
};
use types::{EditError, EditSummary, IterationUpdate};

use std::io::Write;
use std::path::Path;

use chrono::Utc;

use crate::cli::EditArgs;
use crate::iteration::{load_session_iterations, write_iteration_log, IterationLog};
use crate::session;

#[cfg(test)]
mod tests;

/// Resolve which editor to use.
///
/// Checks `$VISUAL`, then `$EDITOR`, falling back to `"vi"`.
fn resolve_editor() -> String {
    for var in &["VISUAL", "EDITOR"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return val;
            }
        }
    }
    "vi".to_string()
}

/// Prompt user to retry or abort after a parse error.
///
/// Writes the prompt to stderr so it shows even when stdout is redirected.
/// Returns `true` for retry, `false` for abort.
fn prompt_retry() -> Result<bool, EditError> {
    eprint!("[r]etry or [a]bort? ");
    let _ = std::io::stderr().flush();

    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(EditError::PromptInput)?;

    let choice = input.trim().to_lowercase();
    Ok(choice.starts_with('r'))
}

/// Spawn the editor and wait for it to exit.
fn spawn_editor(editor: &str, temp_path: &Path) -> Result<(), EditError> {
    let status = std::process::Command::new(editor)
        .arg(temp_path)
        .status()
        .map_err(|source| EditError::SpawnEditor {
            editor: editor.to_string(),
            source,
        })?;

    if status.success() {
        return Ok(());
    }

    match status.code() {
        Some(code) => Err(EditError::EditorFailed {
            editor: editor.to_string(),
            code,
        }),
        None => Err(EditError::EditorSignaled {
            editor: editor.to_string(),
        }),
    }
}

/// Apply iteration updates to disk.
///
/// For each update: rewrites, deletes, or creates iteration files. After all
/// operations, renumbers remaining files to maintain contiguous sequence.
fn execute_updates(
    session_dir: &Path,
    logs: &[IterationLog],
    plan: &[IterationUpdate],
) -> Result<EditSummary, EditError> {
    let mut edited = 0;
    let mut deleted = 0;
    let mut added = 0;

    // Build a map of sequence -> log for quick lookup
    let log_by_seq: std::collections::HashMap<u32, &IterationLog> =
        logs.iter().map(|l| (l.sequence, l)).collect();

    for update in plan {
        match update {
            IterationUpdate::Rewrite {
                sequence,
                prompt,
                response,
            } => {
                let existing = log_by_seq.get(sequence).ok_or_else(|| {
                    EditError::WriteIterationFile(std::io::Error::other(format!(
                        "iteration {sequence} not found in session logs"
                    )))
                })?;
                let mut updated_log = (*existing).clone();
                updated_log.prompt = prompt.clone();
                updated_log.response = response.clone();
                write_iteration_log(session_dir, &updated_log)
                    .map_err(|e| EditError::WriteIterationFile(std::io::Error::other(e)))?;
                edited += 1;
            }
            IterationUpdate::Delete { sequence } => {
                let path = session_dir.join(format!("iteration-{sequence}.toml"));
                if path.exists() {
                    std::fs::remove_file(&path).map_err(EditError::DeleteIterationFile)?;
                    deleted += 1;
                }
            }
            IterationUpdate::Create {
                sequence,
                prompt,
                response,
            } => {
                let now = Utc::now();
                let new_log = IterationLog {
                    sequence: *sequence,
                    started_at: now,
                    completed_at: now,
                    exit_code: 0,
                    pending_before: 0,
                    pending_after: 0,
                    prompt: prompt.clone(),
                    response: response.clone(),
                    metadata: None,
                    tool_calls: vec![],
                    chunks: vec![],
                    output_blocks: vec![],
                    strategy_name: None,
                    persona: None,
                };
                write_iteration_log(session_dir, &new_log)
                    .map_err(|e| EditError::WriteIterationFile(std::io::Error::other(e)))?;
                added += 1;
            }
        }
    }

    // Renumber remaining files to maintain contiguous sequence
    if deleted > 0 {
        renumber_iterations(session_dir)?;
    }

    Ok(EditSummary {
        edited,
        deleted,
        added,
    })
}

/// Renumber iteration files to maintain contiguous 1-indexed sequence.
fn renumber_iterations(session_dir: &Path) -> Result<(), EditError> {
    let mut logs = load_session_iterations(session_dir)?;

    for (i, log) in logs.iter_mut().enumerate() {
        let expected_seq = (i + 1) as u32;
        if log.sequence != expected_seq {
            // Remove old file
            let old_path = session_dir.join(format!("iteration-{}.toml", log.sequence));
            if old_path.exists() {
                std::fs::remove_file(&old_path).map_err(EditError::DeleteIterationFile)?;
            }
            // Write with new sequence
            log.sequence = expected_seq;
            write_iteration_log(session_dir, log)
                .map_err(|e| EditError::WriteIterationFile(std::io::Error::other(e)))?;
        }
    }

    Ok(())
}

/// Execute the `ralph edit` command.
///
/// Resolves the session, loads its conversation, projects it as TOML into a
/// temporary file, opens it in the user's editor, then parses edits and applies
/// changes back to the iteration files.
pub fn execute_edit(args: EditArgs) -> Result<(), Box<dyn std::error::Error>> {
    let project_path = std::env::current_dir()?;

    // Resolve session
    let entry = match &args.slug {
        Some(slug) => session::find_session_by_slug(slug)?,
        None => session::find_most_recent_session(&project_path, None)?.ok_or_else(|| {
            format!(
                "No sessions found for project '{}'. Create a session first with 'ralph ask'.",
                project_path.display()
            )
        })?,
    };

    let slug = &entry.slug;
    println!("Editing session '{slug}'");

    // Load iterations and convert to messages
    let session_dir = session::session_dir(slug);
    let logs = load_session_iterations(&session_dir)?;
    let original_messages = iterations_to_messages(&logs);
    println!("{} messages loaded", original_messages.len());

    // Generate TOML and write to temp file
    let toml_content = messages_to_edit_toml(&original_messages, slug);
    let mut temp_file = tempfile::Builder::new()
        .prefix("ralph-edit-")
        .suffix(".toml")
        .tempfile()
        .map_err(EditError::WriteTempFile)?;

    temp_file
        .write_all(toml_content.as_bytes())
        .map_err(EditError::WriteTempFile)?;

    // Flush stdout so banners are visible before the editor takes over the terminal.
    let _ = std::io::stdout().flush();

    // Open editor
    let editor = resolve_editor();
    spawn_editor(&editor, temp_file.path())?;

    // Parse-retry loop: re-open editor on parse errors
    let edited_messages = loop {
        let edited_content =
            std::fs::read_to_string(temp_file.path()).map_err(EditError::ReadTempFile)?;

        match parse_edit_toml(&edited_content) {
            Ok(msgs) => break msgs,
            Err(e) => {
                eprintln!("Parse error: {e}");
                if !prompt_retry()? {
                    println!("Edit aborted.");
                    return Ok(());
                }
                // User chose retry -- reopen editor with broken content preserved
                spawn_editor(&editor, temp_file.path())?;
            }
        }
    };

    // Check for changes
    if original_messages == edited_messages {
        println!("No changes detected.");
        return Ok(());
    }

    // Re-pair edited messages into iterations and plan updates
    let new_iterations = pair_messages_to_iterations(&edited_messages);
    let plan = plan_iteration_updates(&logs, &new_iterations);

    // Apply changes
    let summary = execute_updates(&session_dir, &logs, &plan)?;
    println!(
        "Applied: {} edited, {} deleted, {} added",
        summary.edited, summary.deleted, summary.added
    );

    Ok(())
}
