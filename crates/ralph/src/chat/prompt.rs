use crate::ansi::{DIM, RESET};
use crate::chat::editor::{edit_draft, OPEN_EDITOR};
use crate::chat::monitor::Monitor;
use crate::chat::statusbar::StatusBar;
use crate::clipboard::{paste_image, PASTE_IMAGE};
use crate::render::wrap_width;
use ralph_core::chat_edit_mode::{prompt_indicator, PromptMode};
use ralph_core::chat_status::{footer, route_input, EmptySubmit, Route};
use ralph_core::worker::ATTACHMENTS_DIR;
use reedline::{
    Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, PromptViMode, Reedline,
    Signal, DEFAULT_MULTILINE_INDICATOR,
};
use std::borrow::Cow;
use std::cell::RefCell;
use std::io::{self, ErrorKind};
use std::sync::Arc;

const DEFAULT_TERMINAL_WIDTH: usize = 80;

pub(super) struct ChatPrompt {
    pub(super) monitor: Arc<Monitor>,
    pub(super) color: bool,
    pub(super) bar: StatusBar,
    pub(super) notice: RefCell<Option<String>>,
    pub(super) editor: Vec<String>,
    pub(super) empty_submit: EmptySubmit,
}

impl ChatPrompt {
    fn dim(&self, line: &str) -> String {
        if self.color {
            format!("{DIM}{line}{RESET}\n")
        } else {
            format!("{line}\n")
        }
    }
}

pub(super) fn screen_columns() -> usize {
    wrap_width(
        crossterm::terminal::size()
            .map(|(w, _)| usize::from(w))
            .unwrap_or(DEFAULT_TERMINAL_WIDTH),
    )
}

impl Prompt for ChatPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Owned(self.bar.line(&self.monitor.status(), screen_columns()))
    }

    fn render_prompt_footer(&self) -> Cow<'_, str> {
        if let Some(notice) = self.notice.borrow().as_deref() {
            return Cow::Owned(self.dim(notice));
        }
        let hints = footer(self.monitor.status().unread, screen_columns());
        Cow::Owned(self.dim(hints.trim_end_matches('\n')))
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, edit_mode: PromptEditMode) -> Cow<'_, str> {
        let mode = match edit_mode {
            PromptEditMode::Vi(PromptViMode::Insert) => PromptMode::ViInsert,
            PromptEditMode::Vi(PromptViMode::Normal) => PromptMode::ViNormal,
            _ => PromptMode::Emacs,
        };
        Cow::Borrowed(prompt_indicator(mode))
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed(DEFAULT_MULTILINE_INDICATOR)
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({prefix}reverse-search: {}) ",
            history_search.term
        ))
    }
}

pub(super) fn next_route(editor: &mut Reedline, prompt: &ChatPrompt) -> io::Result<Option<Route>> {
    let attachments = prompt.monitor.store_dir.join(ATTACHMENTS_DIR);
    loop {
        match retry_interrupted(|| editor.read_line(prompt))? {
            Signal::Success(line) => {
                prompt.notice.take();
                return Ok(Some(route_input(
                    &line,
                    prompt.monitor.status().unread,
                    &prompt.empty_submit,
                )));
            }
            Signal::HostCommand(command) if command == PASTE_IMAGE => {
                *prompt.notice.borrow_mut() = paste_image(editor, &attachments)
            }
            Signal::HostCommand(command) if command == OPEN_EDITOR => {
                *prompt.notice.borrow_mut() =
                    edit_draft(editor, &prompt.monitor.store_dir, &prompt.editor)
                        .err()
                        .map(|error| format!("Editor failed: {error}"))
            }
            _ => return Ok(None),
        }
    }
}

/// Re-runs `read` while it fails with `Interrupted`, which a stop/`fg` resume raises.
fn retry_interrupted<T>(mut read: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    loop {
        match read() {
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            outcome => return outcome,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_interrupted_until_success() {
        let mut calls = 0;
        let outcome = retry_interrupted(|| {
            calls += 1;
            if calls < 3 {
                Err(io::Error::from(ErrorKind::Interrupted))
            } else {
                Ok(calls)
            }
        });
        assert_eq!(outcome.unwrap(), 3);
    }

    #[test]
    fn returns_other_errors_immediately() {
        let mut calls = 0;
        let outcome: io::Result<()> = retry_interrupted(|| {
            calls += 1;
            Err(io::Error::from(ErrorKind::NotFound))
        });
        assert_eq!(outcome.unwrap_err().kind(), ErrorKind::NotFound);
        assert_eq!(calls, 1);
    }
}
