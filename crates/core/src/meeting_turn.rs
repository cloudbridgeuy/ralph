use crate::meeting::{MeetingEvent, MeetingReply};
use std::collections::BTreeMap;
use std::fmt;

pub const DEFAULT_MAX_TURNS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingPlan {
    pub roster: Vec<String>,
    pub first: String,
    pub models: BTreeMap<String, String>,
    pub max_turns: usize,
}

impl MeetingPlan {
    pub fn new(
        roster: Vec<String>,
        first: String,
        model_specs: &[String],
        max_turns: usize,
    ) -> Result<Self, String> {
        let mut unique: Vec<&String> = roster.iter().collect();
        unique.sort();
        unique.dedup();
        if unique.len() != roster.len() {
            return Err("--personas must not repeat a persona.".to_string());
        }
        if roster.len() < 2 {
            return Err("A meeting needs at least two personas.".to_string());
        }
        if !roster.contains(&first) {
            return Err(format!("--first '{first}' is not in --personas."));
        }
        if max_turns == 0 {
            return Err("--max-turns must be at least 1.".to_string());
        }
        let models = parse_models(model_specs, &roster)?;
        Ok(Self {
            roster,
            first,
            models,
            max_turns,
        })
    }
}

pub fn parse_models(
    specs: &[String],
    roster: &[String],
) -> Result<BTreeMap<String, String>, String> {
    let mut models = BTreeMap::new();
    for spec in specs {
        let (persona, model) = spec
            .split_once('=')
            .filter(|(p, m)| !p.is_empty() && !m.is_empty())
            .ok_or_else(|| format!("Invalid --model '{spec}'. Expected persona=model."))?;
        if !roster.iter().any(|p| p == persona) {
            return Err(format!("--model persona '{persona}' is not in --personas."));
        }
        if models
            .insert(persona.to_string(), model.to_string())
            .is_some()
        {
            return Err(format!("--model given twice for '{persona}'."));
        }
    }
    Ok(models)
}

pub fn scene_prompt(roster: &[String], speaker: &str, first: &str, kickoff: &str) -> String {
    let others: Vec<&str> = roster
        .iter()
        .map(String::as_str)
        .filter(|p| *p != speaker)
        .collect();
    format!(
        "## Meeting\n\n\
         You are {speaker}, a participant in a meeting run by ralph. \
         Participants: {roster}. Other participants: {others}.\n\n\
         The human opened the meeting with this kickoff, owned by {first}:\n\n{kickoff}\n\n\
         ### Rules\n\n\
         1. Only one participant speaks at a time. You speak only when you receive a message.\n\
         2. Use the mcp__ralph-meeting__send_message tool (to, text) to pass your reply to one other participant. \
         Calling it ends your turn: stop right after it succeeds and call no other tool.\n\
         3. Call send_message at most once per turn.\n\
         4. Use mcp__ralph-meeting__ask_human (question) when you need the human. You get the answer back and keep your turn.\n\
         5. Use mcp__ralph-meeting__end_meeting (summary) when the goal is reached. It ends the meeting for everyone.\n\
         6. Every turn must finish with send_message or end_meeting.\n\
         7. Plain text in your reply is shown to the human but is never delivered to other participants.",
        roster = roster.join(", "),
        others = others.join(", "),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndMeeting(String),
    TurnCap,
    NoHandoff(String),
    Error(String),
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EndMeeting(summary) => write!(f, "meeting ended: {summary}"),
            Self::TurnCap => write!(f, "turn cap reached"),
            Self::NoHandoff(persona) => {
                write!(
                    f,
                    "{persona} ended their turn without send_message or end_meeting"
                )
            }
            Self::Error(message) => write!(f, "error: {message}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Resume { persona: String, message: String },
    Stop(StopReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reaction {
    Reply(MeetingReply),
    Sent {
        reply: MeetingReply,
        to: String,
        text: String,
    },
    AskHuman(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Handoff {
    to: String,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnState {
    speaker: String,
    turn: usize,
    max_turns: usize,
    handoff: Option<Handoff>,
    ended: Option<String>,
}

fn reply(text: &str) -> Reaction {
    Reaction::Reply(MeetingReply {
        text: text.to_string(),
    })
}

impl TurnState {
    pub fn new(first: &str, max_turns: usize) -> Self {
        Self {
            speaker: first.to_string(),
            turn: 1,
            max_turns,
            handoff: None,
            ended: None,
        }
    }

    pub fn speaker(&self) -> &str {
        &self.speaker
    }

    pub fn turn(&self) -> usize {
        self.turn
    }

    pub fn on_event(&mut self, event: &MeetingEvent) -> Reaction {
        let (MeetingEvent::SendMessage { from, .. }
        | MeetingEvent::AskHuman { from, .. }
        | MeetingEvent::EndMeeting { from, .. }) = event;
        if *from != self.speaker {
            return reply("It is not your turn. Stop now.");
        }
        match event {
            MeetingEvent::SendMessage { to, text, .. } => {
                if self.handoff.is_some() || self.ended.is_some() {
                    return reply("Your turn is already handed off. Stop now.");
                }
                self.handoff = Some(Handoff {
                    to: to.clone(),
                    text: text.clone(),
                });
                Reaction::Sent {
                    reply: MeetingReply {
                        text: "ok".to_string(),
                    },
                    to: to.clone(),
                    text: text.clone(),
                }
            }
            MeetingEvent::AskHuman { question, .. } => Reaction::AskHuman(question.clone()),
            MeetingEvent::EndMeeting { summary, .. } => {
                self.ended = Some(summary.clone());
                reply("Meeting ended. Stop now.")
            }
        }
    }

    pub fn finish_turn(&mut self, exit_code: i32) -> Step {
        if let Some(summary) = self.ended.take() {
            return Step::Stop(StopReason::EndMeeting(summary));
        }
        let Some(handoff) = self.handoff.take() else {
            return Step::Stop(if exit_code == 0 {
                StopReason::NoHandoff(self.speaker.clone())
            } else {
                StopReason::Error(format!("{} exited with code {exit_code}", self.speaker))
            });
        };
        if self.turn >= self.max_turns {
            return Step::Stop(StopReason::TurnCap);
        }
        let message = format!("Message from {}:\n\n{}", self.speaker, handoff.text);
        self.speaker = handoff.to.clone();
        self.turn += 1;
        Step::Resume {
            persona: handoff.to,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn send(from: &str, to: &str) -> MeetingEvent {
        MeetingEvent::SendMessage {
            from: from.into(),
            to: to.into(),
            text: "hi".into(),
        }
    }

    fn reply_text(reaction: Reaction) -> String {
        match reaction {
            Reaction::Reply(r) => r.text,
            other => panic!("expected reply, got {other:?}"),
        }
    }

    #[test]
    fn plan_validates_roster_first_and_models() {
        let ok = MeetingPlan::new(names(&["a", "b"]), "a".into(), &names(&["b=sonnet"]), 5);
        assert_eq!(ok.unwrap().models["b"], "sonnet");
        let bad = [
            MeetingPlan::new(names(&["a"]), "a".into(), &[], 5),
            MeetingPlan::new(names(&["a", "a"]), "a".into(), &[], 5),
            MeetingPlan::new(names(&["a", "b"]), "c".into(), &[], 5),
            MeetingPlan::new(names(&["a", "b"]), "a".into(), &names(&["c=x"]), 5),
            MeetingPlan::new(names(&["a", "b"]), "a".into(), &names(&["a"]), 5),
            MeetingPlan::new(names(&["a", "b"]), "a".into(), &names(&["a=x", "a=y"]), 5),
            MeetingPlan::new(names(&["a", "b"]), "a".into(), &[], 0),
        ];
        assert!(bad.iter().all(Result::is_err));
    }

    #[test]
    fn second_send_in_a_turn_is_refused() {
        let mut state = TurnState::new("a", 5);
        assert_eq!(
            state.on_event(&send("a", "b")),
            Reaction::Sent {
                reply: MeetingReply { text: "ok".into() },
                to: "b".into(),
                text: "hi".into(),
            }
        );
        assert!(reply_text(state.on_event(&send("a", "c"))).contains("already handed off"));
        match state.finish_turn(0) {
            Step::Resume { persona, message } => {
                assert_eq!(persona, "b");
                assert!(message.starts_with("Message from a:"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!((state.speaker(), state.turn()), ("b", 2));
    }

    #[test]
    fn event_from_non_speaker_is_refused() {
        let mut state = TurnState::new("a", 5);
        assert!(reply_text(state.on_event(&send("b", "a"))).contains("not your turn"));
        assert_eq!(
            state.finish_turn(0),
            Step::Stop(StopReason::NoHandoff("a".into()))
        );
    }

    #[test]
    fn end_meeting_wins_and_turn_cap_stops() {
        let mut state = TurnState::new("a", 5);
        state.on_event(&send("a", "b"));
        state.on_event(&MeetingEvent::EndMeeting {
            from: "a".into(),
            summary: "done".into(),
        });
        assert_eq!(
            state.finish_turn(0),
            Step::Stop(StopReason::EndMeeting("done".into()))
        );
        let mut capped = TurnState::new("a", 1);
        capped.on_event(&send("a", "b"));
        assert_eq!(capped.finish_turn(0), Step::Stop(StopReason::TurnCap));
    }

    #[test]
    fn ask_human_is_left_to_the_shell_and_failed_exit_is_error() {
        let mut state = TurnState::new("a", 5);
        let ask = MeetingEvent::AskHuman {
            from: "a".into(),
            question: "why".into(),
        };
        assert_eq!(state.on_event(&ask), Reaction::AskHuman("why".into()));
        assert!(matches!(
            state.finish_turn(1),
            Step::Stop(StopReason::Error(_))
        ));
    }

    #[test]
    fn scene_prompt_names_speaker_roster_kickoff_and_tools() {
        let prompt = scene_prompt(&names(&["a", "b", "c"]), "b", "a", "plan the launch");
        assert!(prompt.contains("You are b"));
        assert!(prompt.contains("Other participants: a, c"));
        assert!(prompt.contains("owned by a"));
        assert!(prompt.contains("plan the launch"));
        for tool in ["send_message", "ask_human", "end_meeting"] {
            assert!(prompt.contains(&format!("mcp__ralph-meeting__{tool}")));
        }
    }
}
