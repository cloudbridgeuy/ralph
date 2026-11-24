use crate::chat::prompt::{ChatPrompt, Submitted};
use crate::chat::queue::exit_notice;
use crate::chat::route::history_call;
use crate::chat::thread;
use crate::cli::{HistoryArgs, HistorySub};
use crate::render::pill::{emphasis_active, pill_prompt_rule};
use ralph_core::chat::{prompt_text, CustomPrompt};
use ralph_core::chat_hold::hold_targets;
use ralph_core::chat_memory::{history_tool, history_tool_named, CallSource};
use ralph_core::chat_status::route_input;
use ralph_core::chat_thread::{in_scope, ThreadName};
use ralph_server::config::PromptFiles;
use ralph_server::history_log;
use ralph_server::warn::warn_if_err;
use reedline::{HistoryItem, Reedline};
use std::io;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use super::STORE_ENV;

mod decay;
mod stats;

pub(super) fn resolve_store(
    flag: Option<PathBuf>,
    env: Option<PathBuf>,
    cwd: &Path,
    home: Option<&Path>,
) -> io::Result<PathBuf> {
    let env = env.filter(|dir| !dir.as_os_str().is_empty());
    match flag.or(env) {
        Some(dir) => Ok(cwd.join(dir)),
        None => {
            let store = cwd.join(".ralph");
            match home {
                Some(home) if store == home.join(".ralph") => Err(io::Error::other(format!(
                    "the default store {} is ralph's runtime directory; pass --store or set {STORE_ENV}",
                    store.display()
                ))),
                _ => Ok(store),
            }
        }
    }
}

pub(crate) fn store_dir(flag: Option<PathBuf>) -> io::Result<PathBuf> {
    let home = dirs::home_dir().map(|home| canonical_or_raw(&home));
    resolve_store(
        flag,
        std::env::var_os(STORE_ENV).map(PathBuf::from),
        &canonical_or_raw(&std::env::current_dir()?),
        home.as_deref(),
    )
}

fn canonical_or_raw(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
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
    let Some(tool) = history.tool else {
        return match &history.stats {
            Some(HistorySub::Stats(args)) => stats::print_stats(store_dir, args),
            Some(HistorySub::Decay(args)) => decay::print_decay(store_dir, args),
            None => Err("history needs a tool or a subcommand".into()),
        };
    };
    let parsed = history_tool(
        tool.name(),
        &history.words.join(" "),
        history.cursor.as_deref(),
        history.tag.as_deref(),
    )
    .and_then(|call| match &history.scope {
        Some(scope) => in_scope(call, scope),
        None => Ok(call),
    });
    let call = parsed.inspect_err(|message| {
        if let Some(tool) = history_tool_named(tool.name()) {
            history_log::record_refusal(store_dir, tool, message, CallSource::Cli);
        }
    })?;
    let text = history_call(store_dir, &call, CallSource::Cli)?.text;
    println!("{text}");
    Ok(())
}

fn read_prompt_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|e| format!("Skipping prompt file {}: {e}", path.display()))
}

fn with_pill_rule(mut prompt: CustomPrompt, emphasis: bool, color: bool) -> CustomPrompt {
    if emphasis_active(emphasis, color) {
        prompt.appends.push(pill_prompt_rule());
    }
    prompt
}

pub(crate) fn load_custom_prompt(files: &PromptFiles) -> (CustomPrompt, Vec<String>) {
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
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    (
        with_pill_rule(
            CustomPrompt {
                behaviour,
                appends: appends.into_iter().flatten().collect(),
            },
            files.emphasis,
            color,
        ),
        behaviour_warning
            .into_iter()
            .chain(append_warnings.into_iter().filter_map(Result::err))
            .collect(),
    )
}

pub(super) fn route_queued(text: String, editor: &mut Reedline, prompt: &ChatPrompt) -> Submitted {
    print!("{}", prompt.echo(&text));
    warn_if_err(
        editor
            .history_mut()
            .save(HistoryItem::from_command_line(text.clone())),
        "Failed to save chat history",
    );
    let status = prompt.monitor.status();
    let route = route_input(
        &text,
        status.unread,
        &prompt.empty_submit,
        &hold_targets(&status),
    );
    Submitted { line: text, route }
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
    saved
}

pub(super) fn print_exit_notice(count: usize, saved: bool) {
    if let Some(notice) = exit_notice(count, saved) {
        eprintln!("{notice}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> CustomPrompt {
        CustomPrompt {
            behaviour: Some("b".to_string()),
            appends: vec!["a".to_string()],
        }
    }

    #[test]
    fn flag_off_leaves_the_prompt_unchanged() {
        assert_eq!(with_pill_rule(base(), false, true), base());
        assert_eq!(with_pill_rule(base(), false, false), base());
    }

    #[test]
    fn flag_on_without_colour_leaves_the_prompt_unchanged() {
        assert_eq!(with_pill_rule(base(), true, false), base());
    }

    #[test]
    fn flag_on_with_colour_appends_the_rule_once_after_existing_appends() {
        let out = with_pill_rule(base(), true, true);
        assert_eq!(out.behaviour, base().behaviour);
        assert_eq!(out.appends, vec!["a".to_string(), pill_prompt_rule()]);
    }
}
