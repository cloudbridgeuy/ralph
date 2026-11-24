use crate::config::confirm_answer;
use ralph_core::worker::{
    answer_hint, blocked_notice, permission_response, Choice, Classes, ClaudePermission, Grants,
    ALLOW_ONCE,
};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Chunk {
    stream: Stream,
    text: String,
}

#[derive(Debug, Default)]
pub(super) struct Held {
    chunks: Vec<Chunk>,
}

impl Held {
    pub(super) fn push(&mut self, stream: Stream, text: &str) {
        self.chunks.push(Chunk {
            stream,
            text: text.to_string(),
        });
    }

    pub(super) fn take(&mut self) -> Vec<Chunk> {
        std::mem::take(&mut self.chunks)
    }
}

#[derive(Debug, Default)]
struct GateState {
    paused: bool,
    settled: bool,
    held: Held,
    listening: bool,
    permissions: Vec<PendingPermission>,
    grants: Grants,
}

#[derive(Debug)]
struct PendingPermission {
    token: String,
    ask: ClaudePermission,
    missing: Classes,
    answer: Option<String>,
}

#[derive(Debug)]
pub enum Outcome {
    Refused(String),
    Answered { choice: Choice, classes: Classes },
}

impl Outcome {
    pub fn report(self, store_dir: &Path) -> String {
        match self {
            Outcome::Refused(message) => message,
            Outcome::Answered { choice, classes } => confirm_answer(store_dir, choice, classes),
        }
    }
}

const NO_MATCH: &str = "No matching main chat permission request.";
const INVALID_OPTION: &str = "Invalid option; use allow_once|deny|allow_always.";

fn apply(state: &mut GateState, index: usize, choice: Choice) -> Outcome {
    let Some(pending) = state.permissions.get_mut(index) else {
        return Outcome::Refused(NO_MATCH.into());
    };
    let classes = pending.missing;
    let sent = match choice {
        Choice::AllowAlways => ALLOW_ONCE,
        other => other.option_id(),
    };
    pending.answer = Some(sent.to_string());
    if choice == Choice::AllowAlways {
        state.grants = state.grants.union(classes.into());
    }
    Outcome::Answered { choice, classes }
}

#[derive(Debug, Default)]
pub struct OutputGate {
    state: Mutex<GateState>,
}

pub(super) fn write_direct(stream: Stream, text: &str) {
    match stream {
        Stream::Stdout => {
            let mut out = io::stdout();
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }
        Stream::Stderr => {
            let mut err = io::stderr();
            let _ = err.write_all(text.as_bytes());
            let _ = err.flush();
        }
    }
}

pub fn write_to(gate: Option<&OutputGate>, stream: Stream, text: &str) {
    match gate {
        Some(gate) => gate.write(stream, text),
        None => write_direct(stream, text),
    }
}

fn emit(chunk: &Chunk) {
    write_direct(chunk.stream, &chunk.text);
}

impl OutputGate {
    pub fn new() -> OutputGate {
        OutputGate::default()
    }

    pub fn with_grants(grants: Grants) -> OutputGate {
        OutputGate {
            state: Mutex::new(GateState {
                grants,
                ..GateState::default()
            }),
        }
    }

    pub(super) fn grants(&self) -> Grants {
        self.lock().grants
    }

    pub fn listen(&self, active: bool) {
        let mut state = self.lock();
        state.listening = active;
        if !active {
            for pending in &mut state.permissions {
                pending.answer = Some("deny".into());
            }
        }
    }

    pub(super) fn request_permission(&self, ask: ClaudePermission, missing: Classes) -> bool {
        let mut state = self.lock();
        if !state.listening {
            return false;
        }
        let mut bytes = [0u8; 16];
        if getrandom::fill(&mut bytes).is_err() {
            return false;
        }
        let token = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut content = ask.content.clone();
        content.request_id = token.clone();
        content.tool = escaped_json(&content.tool);
        content.summary = escaped_json(&content.summary);
        let notice = format!(
            "\n{}\ninput: {}\n{}\n",
            blocked_notice(None, "@chat", Some(&content)),
            escaped_json(&ask.input),
            answer_hint(&token, missing)
        );
        state.permissions.push(PendingPermission {
            token,
            ask,
            missing,
            answer: None,
        });
        if state.paused {
            state.held.push(Stream::Stdout, &notice);
        } else {
            write_direct(Stream::Stdout, &notice);
        }
        true
    }

    pub fn answer_permission(&self, token: &str, option: &str) -> Outcome {
        let state = &mut *self.lock();
        let index = state
            .permissions
            .iter()
            .position(|p| state.listening && p.token == token && p.answer.is_none());
        match (index, Choice::from_option(option)) {
            (None, _) => Outcome::Refused(NO_MATCH.into()),
            (Some(_), None) => Outcome::Refused(INVALID_OPTION.into()),
            (Some(index), Some(choice)) => apply(state, index, choice),
        }
    }

    pub fn answer_oldest(&self, choice: Choice) -> Option<Outcome> {
        let state = &mut *self.lock();
        let index = state
            .permissions
            .iter()
            .position(|p| state.listening && p.answer.is_none())?;
        Some(apply(state, index, choice))
    }

    pub(super) fn permission_reply(&self, deny: bool) -> Option<String> {
        let mut state = self.lock();
        let position = state
            .permissions
            .iter()
            .position(|p| deny || p.answer.is_some())?;
        let pending = state.permissions.remove(position);
        let option = if deny || !state.listening {
            "deny"
        } else {
            pending.answer.as_deref().unwrap_or("deny")
        };
        Some(permission_response(
            &pending.ask.content.request_id,
            option,
            &pending.ask.input,
        ))
    }

    pub(super) fn cancel_permission(&self, id: Option<&str>) {
        let mut state = self.lock();
        state
            .permissions
            .retain(|p| id.is_some_and(|id| p.ask.content.request_id != id));
    }

    pub(super) fn retire_duplicate(&self, id: &str) -> bool {
        let mut state = self.lock();
        let before = state.permissions.len();
        state
            .permissions
            .retain(|pending| pending.ask.content.request_id != id);
        state.permissions.len() != before
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn is_paused(&self) -> bool {
        self.lock().paused
    }

    pub fn pause(&self) {
        let mut state = self.lock();
        state.paused = true;
        state.settled = false;
    }

    pub fn is_settled(&self) -> bool {
        self.lock().settled
    }

    pub(super) fn settle(&self) {
        self.lock().settled = true;
    }

    pub fn resume(&self) {
        let mut state = self.lock();
        state.held.take().iter().for_each(emit);
        state.paused = false;
        state.settled = false;
    }

    pub fn write_out(&self, text: &str) {
        self.write(Stream::Stdout, text);
    }

    pub fn write_err(&self, text: &str) {
        self.write(Stream::Stderr, text);
    }

    pub(super) fn write(&self, stream: Stream, text: &str) {
        let mut state = self.lock();
        if state.paused {
            state.held.push(stream, text);
        } else {
            write_direct(stream, text);
        }
    }
}

fn escaped_json(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value)
        .unwrap_or_default()
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
            .chars()
            .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::worker::{mcptools_gate, GateCall};

    fn refused(outcome: Outcome) -> bool {
        matches!(outcome, Outcome::Refused(message) if message.contains("No matching"))
    }

    fn answered(outcome: Outcome) -> bool {
        matches!(outcome, Outcome::Answered { .. })
    }

    fn ask(id: &str, input: serde_json::Value) -> ClaudePermission {
        let line = serde_json::json!({"type":"control_request","request_id":id,"request":{"subtype":"can_use_tool","tool_name":"mcp__mcptools__execute","input":input}}).to_string();
        let ralph_core::worker::ClaudeLine::Permission(ask) =
            ralph_core::worker::decode_line(&line)
        else {
            panic!()
        };
        ask
    }

    #[test]
    fn permissions_bind_input_once_and_cancel_independently() {
        let gate = OutputGate::new();
        let input = serde_json::json!({"allowWrites":true,"code":"one\n\u{1b}[2J"});
        assert!(!gate.request_permission(ask("same", input.clone()), Classes::default()));
        gate.listen(true);
        gate.pause();
        assert!(gate.request_permission(ask("same", input.clone()), Classes::default()));
        assert!(gate.request_permission(
            ask("other", serde_json::json!({"allowSpend":true})),
            Classes::default()
        ));
        let (first, second, notice) = {
            let state = gate.lock();
            (
                state.permissions[0].token.clone(),
                state.permissions[1].token.clone(),
                state.held.chunks[0].text.clone(),
            )
        };
        assert!(notice.contains("one\\n\\u001b[2J"));
        assert!(notice.contains(&format!("/answer @chat {first} allow_once")));
        gate.cancel_permission(Some("other"));
        assert!(refused(gate.answer_permission(&second, "allow_once")));
        assert!(answered(gate.answer_permission(&first, "allow_once")));
        assert!(refused(gate.answer_permission(&first, "allow_once")));
        let reply: serde_json::Value =
            serde_json::from_str(&gate.permission_reply(false).unwrap()).unwrap();
        assert_eq!(reply["response"]["response"]["updatedInput"], input);
        assert!(gate.request_permission(
            ask("same", serde_json::json!({"allowSpend":true})),
            Classes::default()
        ));
        let retry = gate.lock().permissions[0].token.clone();
        assert_ne!(first, retry);
        assert!(refused(gate.answer_permission(&first, "allow_once")));
        assert!(gate.retire_duplicate("same"));
        assert!(refused(gate.answer_permission(&retry, "allow_once")));
        assert!(gate.request_permission(
            ask("same", serde_json::json!({"allowSpend":true})),
            Classes::default()
        ));
        let retry = gate.lock().permissions[0].token.clone();
        gate.answer_permission(&retry, "allow_once");
        gate.listen(false);
        let reply: serde_json::Value =
            serde_json::from_str(&gate.permission_reply(false).unwrap()).unwrap();
        assert_eq!(reply["response"]["response"]["behavior"], "deny");
    }

    #[test]
    fn cancellation_overrides_a_recorded_approval() {
        let gate = OutputGate::new();
        gate.listen(true);
        gate.pause();
        assert!(gate.request_permission(
            ask("r", serde_json::json!({"allowWrites":true})),
            Classes::default()
        ));
        let token = gate.lock().permissions[0].token.clone();
        gate.answer_permission(&token, "allow_once");
        let reply: serde_json::Value =
            serde_json::from_str(&gate.permission_reply(true).unwrap()).unwrap();
        assert_eq!(reply["response"]["response"]["behavior"], "deny");
    }

    #[test]
    fn held_keeps_order_across_streams() {
        let mut held = Held::default();
        held.push(Stream::Stdout, "a");
        held.push(Stream::Stderr, "b");
        held.push(Stream::Stdout, "c");
        let taken = held.take();
        let seen: Vec<(Stream, &str)> = taken.iter().map(|c| (c.stream, c.text.as_str())).collect();
        assert_eq!(
            seen,
            vec![
                (Stream::Stdout, "a"),
                (Stream::Stderr, "b"),
                (Stream::Stdout, "c")
            ]
        );
    }

    #[test]
    fn paused_write_out_and_write_err_are_held_in_order() {
        let gate = OutputGate::new();
        gate.pause();
        gate.write_out("rule");
        gate.write_err("warning");
        let held = gate.lock().held.take();
        let seen: Vec<(Stream, &str)> = held.iter().map(|c| (c.stream, c.text.as_str())).collect();
        assert_eq!(
            seen,
            vec![(Stream::Stdout, "rule"), (Stream::Stderr, "warning")]
        );
    }

    #[test]
    fn take_empties_the_buffer() {
        let mut held = Held::default();
        held.push(Stream::Stdout, "a");
        assert_eq!(held.take().len(), 1);
        assert!(held.take().is_empty());
    }

    #[test]
    fn writes_while_running_are_not_held() {
        let gate = OutputGate::new();
        gate.write(Stream::Stdout, "");
        assert!(gate.lock().held.take().is_empty());
    }

    #[test]
    fn pause_unsettles_and_settle_marks_until_resume() {
        let gate = OutputGate::new();
        gate.pause();
        assert!(!gate.is_settled());
        gate.settle();
        assert!(gate.is_settled());
        gate.resume();
        assert!(!gate.is_settled());
    }

    #[test]
    fn writes_while_paused_are_held_until_resume() {
        let gate = OutputGate::new();
        gate.pause();
        assert!(gate.is_paused());
        gate.write(Stream::Stdout, "");
        gate.write(Stream::Stderr, "");
        assert_eq!(gate.lock().held.chunks.len(), 2);
        gate.resume();
        assert!(!gate.is_paused());
        assert!(gate.lock().held.take().is_empty());
    }

    const WRITES: Classes = Classes {
        writes: true,
        spend: false,
    };

    #[test]
    fn always_allow_grants_the_classes_for_later_calls() {
        let gate = OutputGate::new();
        gate.listen(true);
        let input = serde_json::json!({"allowWrites":true});
        let first = ask("one", input.clone());
        assert_eq!(
            mcptools_gate(gate.grants(), &first),
            GateCall::Prompt(WRITES)
        );
        assert!(gate.request_permission(first, WRITES));
        let token = gate.lock().permissions[0].token.clone();
        assert!(matches!(
            gate.answer_permission(&token, "allow_always"),
            Outcome::Answered {
                choice: Choice::AllowAlways,
                ..
            }
        ));
        assert_eq!(
            mcptools_gate(gate.grants(), &ask("two", input)),
            GateCall::Allow
        );
        let reply: serde_json::Value =
            serde_json::from_str(&gate.permission_reply(false).unwrap()).unwrap();
        assert_eq!(reply["response"]["response"]["behavior"], "allow");
    }

    #[test]
    fn allow_once_and_deny_leave_the_grants_alone() {
        let gate = OutputGate::new();
        gate.listen(true);
        for key in [Choice::AllowOnce, Choice::Deny] {
            assert!(
                gate.request_permission(ask("r", serde_json::json!({"allowWrites":true})), WRITES)
            );
            assert!(gate.answer_oldest(key).is_some());
            gate.permission_reply(false);
        }
        assert_eq!(gate.grants(), Grants::default());
        assert!(gate.answer_oldest(Choice::AllowOnce).is_none());
    }

    #[test]
    fn a_key_answers_the_oldest_unanswered_request() {
        let gate = OutputGate::new();
        gate.listen(true);
        assert!(gate.request_permission(ask("a", serde_json::json!({"allowWrites":true})), WRITES));
        assert!(gate.request_permission(
            ask("b", serde_json::json!({"allowSpend":true})),
            Classes::default()
        ));
        assert!(gate.answer_oldest(Choice::AllowOnce).is_some());
        assert!(gate.answer_oldest(Choice::Deny).is_some());
        assert!(gate.answer_oldest(Choice::Deny).is_none());
        let first: serde_json::Value =
            serde_json::from_str(&gate.permission_reply(false).unwrap()).unwrap();
        assert_eq!(first["response"]["response"]["behavior"], "allow");
    }

    #[test]
    fn the_notice_carries_the_hint_for_the_missing_classes() {
        let gate = OutputGate::new();
        gate.listen(true);
        gate.pause();
        assert!(gate.request_permission(ask("r", serde_json::json!({"allowWrites":true})), WRITES));
        let notice = gate.lock().held.chunks[0].text.clone();
        assert!(notice.contains("a always allow writes\n/answer @chat "));
        assert!(!notice.contains("allow_once\n"));
    }

    #[test]
    fn an_unknown_option_lists_the_valid_ones() {
        let gate = OutputGate::new();
        gate.listen(true);
        assert!(gate.request_permission(ask("r", serde_json::json!({"allowWrites":true})), WRITES));
        let token = gate.lock().permissions[0].token.clone();
        let Outcome::Refused(message) = gate.answer_permission(&token, "once") else {
            panic!()
        };
        assert!(message.contains("allow_once|deny|allow_always"));
    }
}
