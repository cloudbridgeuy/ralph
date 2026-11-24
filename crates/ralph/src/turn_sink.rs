use crate::highlight::{ThemeConfig, ThemeError};
use crate::render::wrap_width;
use crate::spinner::{Spinner, SpinnerContext, SpinnerSessionInfo};
use crate::stream_processor::{BlockRenderer, OutputBlock};
use crate::ui_theme::UiColors;
use crate::warn::warning_line;
use ralph_core::chunk::ChunkType;
use ralph_core::turn_event::{TurnEvent, TurnSink, Wait};
use ralph_server::subprocess::{write_to, OutputGate, Stream};
use ralph_server::watches::locked;
use std::io::IsTerminal;
use std::sync::{Arc, Mutex};
use termimad::MadSkin;

fn rendered_width() -> usize {
    wrap_width(termimad::terminal_size().0.into())
}

pub(crate) fn rule(skin: &MadSkin, width: usize) -> String {
    skin.text("---", Some(width)).to_string()
}

pub(crate) fn rule_skin(ui: &UiColors) -> MadSkin {
    let mut skin = MadSkin::default();
    skin.horizontal_rule.set_fg(ui.rule);
    skin
}

pub(crate) fn written(
    event: &TurnEvent,
    ui: &UiColors,
    width: usize,
    renderer: &mut BlockRenderer,
) -> Option<(Stream, String)> {
    match event {
        TurnEvent::Rule => Some((Stream::Stdout, rule(&rule_skin(ui), width))),
        TurnEvent::Warning(message) => Some((Stream::Stderr, warning_line(message))),
        TurnEvent::Blocks(blocks) => Some((Stream::Stdout, renderer.render(blocks))),
        TurnEvent::Stderr(text) => Some((Stream::Stderr, text.clone())),
        TurnEvent::Busy(_) | TurnEvent::Resume(_) | TurnEvent::Idle => None,
    }
}

pub(crate) trait ReplyWidth: Send + Sync {
    fn begin(&self);
    fn rendered(&self, cols: usize);
}

fn has_prose(blocks: &[OutputBlock]) -> bool {
    blocks.iter().any(
        |block| matches!(block, OutputBlock::Text(text) if text.chunk.chunk_type == ChunkType::Prose),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Spin {
    Fresh(SpinnerContext),
    Restart(SpinnerContext),
    Stop,
}

pub(crate) fn spin(event: &TurnEvent) -> Option<Spin> {
    match event {
        TurnEvent::Busy(wait) => Some(Spin::Fresh(context(*wait))),
        TurnEvent::Resume(wait) => Some(Spin::Restart(context(*wait))),
        TurnEvent::Idle => Some(Spin::Stop),
        _ => None,
    }
}

pub(crate) struct TerminalSink {
    gate: Option<Arc<OutputGate>>,
    ui: UiColors,
    columns: fn() -> usize,
    session: SpinnerSessionInfo,
    spinner: Mutex<Option<Spinner>>,
    renderer: Mutex<BlockRenderer>,
    reply_width: Option<Arc<dyn ReplyWidth>>,
}

impl TerminalSink {
    pub(crate) fn new(
        gate: Option<Arc<OutputGate>>,
        theme: &ThemeConfig,
        columns: fn() -> usize,
    ) -> Result<Self, ThemeError> {
        Self::with_highlighting(gate, theme, columns, std::io::stdout().is_terminal())
    }

    pub(crate) fn with_highlighting(
        gate: Option<Arc<OutputGate>>,
        theme: &ThemeConfig,
        columns: fn() -> usize,
        highlighting: bool,
    ) -> Result<Self, ThemeError> {
        Ok(TerminalSink {
            gate,
            ui: *theme.ui(),
            columns,
            session: SpinnerSessionInfo::default(),
            spinner: Mutex::new(None),
            renderer: Mutex::new(BlockRenderer::new(Some(theme.clone()), highlighting)?),
            reply_width: None,
        })
    }

    pub(crate) fn plain(theme: &ThemeConfig) -> Result<Self, ThemeError> {
        TerminalSink::new(None, theme, rendered_width)
    }

    pub(crate) fn with_session(self, session: SpinnerSessionInfo) -> Self {
        TerminalSink { session, ..self }
    }

    pub(crate) fn with_reply_width(self, reply_width: Arc<dyn ReplyWidth>) -> Self {
        TerminalSink {
            reply_width: Some(reply_width),
            ..self
        }
    }

    fn track(&self, event: &TurnEvent) {
        let Some(reply_width) = &self.reply_width else {
            return;
        };
        match event {
            TurnEvent::Busy(Wait::Response) => reply_width.begin(),
            TurnEvent::Blocks(blocks) if has_prose(blocks) => {
                reply_width.rendered((self.columns)());
            }
            _ => {}
        }
    }

    fn spinner(&self) -> Spinner {
        Spinner::with_session_context(0, self.session.clone()).with_color(self.ui.spinner)
    }

    fn turn(&self, step: Spin) {
        let mut held = locked(&self.spinner);
        match step {
            Spin::Fresh(context) => {
                held.take();
                held.insert(self.spinner()).start_with_context(context);
            }
            Spin::Restart(context) => held
                .get_or_insert_with(|| self.spinner())
                .start_with_context(context),
            Spin::Stop => {
                if let Some(spinner) = held.as_mut() {
                    spinner.stop();
                }
            }
        }
    }
}

impl TurnSink for TerminalSink {
    fn emit(&self, event: TurnEvent) {
        self.track(&event);
        if let Some(step) = spin(&event) {
            self.turn(step);
            return;
        }
        let shown = written(
            &event,
            &self.ui,
            (self.columns)(),
            &mut locked(&self.renderer),
        );
        if let Some((stream, text)) = shown {
            write_to(self.gate.as_deref(), stream, &text);
        }
    }
}

pub(crate) fn context(wait: Wait) -> SpinnerContext {
    match wait {
        Wait::Response => SpinnerContext::WaitingForResponse,
        Wait::Memory => SpinnerContext::WaitingForMemory,
        Wait::Thinking => SpinnerContext::Thinking,
        Wait::Tool => SpinnerContext::WaitingForTool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_processor::OutputBlock;
    use ralph_core::chunk::{ChunkType, ParsedChunk};

    fn ui() -> UiColors {
        UiColors::default()
    }

    fn renderer() -> BlockRenderer {
        BlockRenderer::new(None, false).unwrap()
    }

    fn prose(content: &str) -> OutputBlock {
        OutputBlock::text(ParsedChunk {
            chunk_type: ChunkType::Prose,
            content: content.to_string(),
        })
    }

    #[test]
    fn a_rule_is_the_themed_rule_at_the_width_on_stdout() {
        let expected = rule(&rule_skin(&ui()), 20);
        assert_eq!(
            written(&TurnEvent::Rule, &ui(), 20, &mut renderer()),
            Some((Stream::Stdout, expected))
        );
    }

    #[test]
    fn a_warning_is_a_warning_line_on_stderr() {
        assert_eq!(
            written(
                &TurnEvent::Warning("Turn cancelled.".into()),
                &ui(),
                20,
                &mut renderer()
            ),
            Some((Stream::Stderr, warning_line("Turn cancelled.")))
        );
    }

    #[test]
    fn blocks_render_to_text_on_stdout_and_stderr_passes_through() {
        assert_eq!(
            written(
                &TurnEvent::Blocks(vec![prose("hello\n")]),
                &ui(),
                20,
                &mut renderer()
            ),
            Some((Stream::Stdout, "hello\n".to_string()))
        );
        assert_eq!(
            written(
                &TurnEvent::Stderr("oops\n".into()),
                &ui(),
                20,
                &mut renderer()
            ),
            Some((Stream::Stderr, "oops\n".to_string()))
        );
    }

    #[test]
    fn spinner_events_write_nothing() {
        for event in [
            TurnEvent::Busy(Wait::Response),
            TurnEvent::Resume(Wait::Thinking),
            TurnEvent::Idle,
        ] {
            assert_eq!(written(&event, &ui(), 20, &mut renderer()), None);
        }
    }

    #[test]
    fn busy_starts_a_fresh_spinner_clock() {
        assert_eq!(
            spin(&TurnEvent::Busy(Wait::Memory)),
            Some(Spin::Fresh(SpinnerContext::WaitingForMemory))
        );
    }

    #[test]
    fn resume_restarts_the_spinner_on_the_same_clock() {
        assert_eq!(
            spin(&TurnEvent::Resume(Wait::Tool)),
            Some(Spin::Restart(SpinnerContext::WaitingForTool))
        );
    }

    #[test]
    fn idle_stops_the_spinner() {
        assert_eq!(spin(&TurnEvent::Idle), Some(Spin::Stop));
    }

    #[test]
    fn written_events_leave_the_spinner_alone() {
        assert_eq!(spin(&TurnEvent::Rule), None);
        assert_eq!(spin(&TurnEvent::Blocks(vec![prose("x")])), None);
    }

    #[test]
    fn each_wait_shows_its_spinner_message() {
        assert_eq!(context(Wait::Response), SpinnerContext::WaitingForResponse);
        assert_eq!(context(Wait::Memory), SpinnerContext::WaitingForMemory);
        assert_eq!(context(Wait::Thinking), SpinnerContext::Thinking);
        assert_eq!(context(Wait::Tool), SpinnerContext::WaitingForTool);
    }
}
