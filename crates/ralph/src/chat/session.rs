use crate::chat::prompt::ChatPrompt;
use crate::chat::queue::exit_notice;
use crate::chat::thread::{self, Memory};
use crate::cli::HistoryArgs;
use crate::config::PromptFiles;
use crate::warn::warn_if_err;
use ralph_core::chat::{prompt_text, CustomPrompt};
use ralph_core::chat_hold::hold_targets;
use ralph_core::chat_memory::history_tool;
use ralph_core::chat_status::{route_input, Route};
use ralph_core::chat_thread::{in_scope, ThreadName};
use reedline::{HistoryItem, Reedline};
use std::io;
use std::path::{Path, PathBuf};

use super::STORE_ENV;
pub(super) fn resolve_store(
    flag: Option<PathBuf>,
    env: Option<PathBuf>,
    cwd: &Path,
    home: Option<&Path>,
) -> io::Result<PathBuf> {
    let env = env.filter(|dir| !dir.as_os_str().is_empty());
    match flag.or(env) {
        Some(dir) => Ok(cwd.join(dir)),
        None => home
            .map(|home| home.join(".ralph").join("chat"))
            .ok_or_else(|| io::Error::other("home directory not found")),
    }
}

pub(super) fn store_dir(flag: Option<PathBuf>) -> io::Result<PathBuf> {
    resolve_store(
        flag,
        std::env::var_os(STORE_ENV).map(PathBuf::from),
        &std::env::current_dir()?,
        dirs::home_dir().as_deref(),
    )
}

pub(super) fn chat_dir(flag: Option<PathBuf>, thread: Option<&ThreadName>) -> io::Result<PathBuf> {
    let main = store_dir(flag)?;
    Ok(match thread {
        Some(name) => thread::thread_dir(&main, name),
        None => main,
    })
}

pub(super) fn print_history(
    store_dir: &Path,
    history: &HistoryArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let text = history_tool(
        history.tool.name(),
        &history.words.join(" "),
        history.cursor.as_deref(),
    )
    .and_then(|call| match &history.scope {
        Some(scope) => in_scope(call, scope),
        None => Ok(call),
    })
    .and_then(|call| Memory::open(store_dir)?.call(&call))?;
    println!("{text}");
    Ok(())
}

fn read_prompt_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|e| format!("Skipping prompt file {}: {e}", path.display()))
}

pub(super) fn load_custom_prompt(files: &PromptFiles) -> (CustomPrompt, Vec<String>) {
    let (behaviour, behaviour_warning) = match files.behaviour.as_deref() {
        Some(path) => match read_prompt_file(path) {
            Ok(text) => match prompt_text(text) {
                Some(text) => (Some(text), None),
                None => (
                    None,
                    Some(format!(
                        "Prompt file {} is empty; using the default reply rules",
                        path.display()
                    )),
                ),
            },
            Err(warning) => (None, Some(warning)),
        },
        None => (None, None),
    };
    let (appends, append_warnings): (Vec<_>, Vec<_>) = files
        .appends
        .iter()
        .map(|path| read_prompt_file(path))
        .partition(Result::is_ok);
    (
        CustomPrompt {
            behaviour,
            appends: appends.into_iter().flatten().collect(),
        },
        behaviour_warning
            .into_iter()
            .chain(append_warnings.into_iter().filter_map(Result::err))
            .collect(),
    )
}

pub(super) fn route_queued(text: String, editor: &mut Reedline, prompt: &ChatPrompt) -> Route {
    print!("{}", prompt.echo(&text));
    warn_if_err(
        editor
            .history_mut()
            .save(HistoryItem::from_command_line(text.clone())),
        "Failed to save chat history",
    );
    let status = prompt.monitor.status();
    route_input(
        &text,
        status.unread,
        &prompt.empty_submit,
        &hold_targets(&status),
    )
}

pub(super) fn save_unsent(editor: &mut Reedline, lost: &[String]) -> bool {
    let mut saved = true;
    for text in lost {
        let result = editor
            .history_mut()
            .save(HistoryItem::from_command_line(text.clone()));
        saved &= result.is_ok();
        warn_if_err(result, "Failed to save chat history");
    }
    if let Some(notice) = exit_notice(lost.len()) {
        eprintln!("{notice}");
    }
    saved
}
