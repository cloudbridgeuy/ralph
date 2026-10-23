use super::{interrupt_reply, BlockedContent, HarnessId, TurnEnd};

mod liveness;
pub use liveness::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    Spawn,
    Prompt,
    Interrupt,
    PermAsk,
    PermYolo,
    Resume,
    Model,
    Report,
    Close,
    Heartbeat,
}

impl Feature {
    pub const ALL: [Feature; 10] = [
        Self::Spawn,
        Self::Prompt,
        Self::Interrupt,
        Self::PermAsk,
        Self::PermYolo,
        Self::Resume,
        Self::Model,
        Self::Report,
        Self::Close,
        Self::Heartbeat,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Spawn => "spawn",
            Self::Prompt => "prompt",
            Self::Interrupt => "interrupt",
            Self::PermAsk => "perm-ask",
            Self::PermYolo => "perm-yolo",
            Self::Resume => "resume",
            Self::Model => "model",
            Self::Report => "report",
            Self::Close => "close",
            Self::Heartbeat => "heartbeat",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Full,
    Partial(&'static str),
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    harness: HarnessId,
}

impl Capabilities {
    pub fn headless(&self) -> bool {
        self.harness != HarnessId::Claude
    }

    pub fn support(&self, feature: Feature) -> Support {
        use Feature::*;
        use Support::{Full, Partial};
        match (self.harness, feature) {
            (HarnessId::Claude, Model) => Full,
            (HarnessId::Claude, _) => Support::None,
            (HarnessId::Cursor, Spawn | Prompt | PermAsk | Report | Model) => Full,
            (HarnessId::Cursor, Interrupt) => Partial("outcome unknown after 30 s"),
            (HarnessId::Cursor, PermYolo) => Partial("global config, not per spawn"),
            (HarnessId::Cursor, Resume) => Partial("only if agent supports session/load"),
            (HarnessId::Cursor, Close) => Partial("cli-config.json not restored"),
            (HarnessId::Cursor, Heartbeat) => {
                Partial("ACP traffic only; exit seen by polling the child every 250 ms")
            }
            (HarnessId::Opencode, Spawn | Prompt | Resume | Model | Report | Interrupt) => Full,
            (HarnessId::Opencode, PermAsk) => {
                Partial("only prompts the server raises despite allow-*")
            }
            (HarnessId::Opencode, PermYolo) => Partial("always on"),
            (HarnessId::Opencode, Close) => Partial("no home cleanup; shared server kept"),
            (HarnessId::Opencode, Heartbeat) => {
                Partial("per-session SSE activity; one alert per shared-server death")
            }
            (HarnessId::Codex, Spawn | Prompt | Report | Model) => Full,
            (HarnessId::Codex, Interrupt) => Partial("outcome unknown after 30 s"),
            (HarnessId::Codex, PermAsk) => Partial("protocol-raised requests only"),
            (HarnessId::Codex, PermYolo) => Partial("single-request accepts only"),
            (HarnessId::Codex, Resume) => {
                Partial("needs saved history, worktree and codex login")
            }
            (HarnessId::Codex, Heartbeat) => Partial(
                "app-server frames count as activity, not progress; silent long tools can yield false Quiet; 300 s default",
            ),
            (HarnessId::Codex, Close) => Partial("global trust entry left in ~/.codex/config.toml"),
        }
    }
}

pub fn capabilities(harness: HarnessId) -> Capabilities {
    Capabilities { harness }
}

pub fn describe(caps: &Capabilities, feature: Feature) -> String {
    let prefix = format!("{} {}", caps.harness.as_str(), feature.name());
    match caps.support(feature) {
        Support::Full => format!("{prefix}: full"),
        Support::Partial(reason) => format!("{prefix}: partial, {reason}"),
        Support::None => format!("{prefix}: not supported"),
    }
}

pub fn model_refusal(harness: HarnessId) -> String {
    let accepting = HarnessId::ALL
        .into_iter()
        .filter(|candidate| capabilities(*candidate).support(Feature::Model) != Support::None)
        .map(HarnessId::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{} does not accept a model; harnesses that do: {accepting}",
        harness.as_str()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerCommand {
    Spawn(SpawnSpec),
    Prompt(String),
    Interrupt(String),
    AnswerPermission { option_id: String },
    Resume,
    Close,
}

impl WorkerCommand {
    pub fn feature(&self) -> Feature {
        match self {
            Self::Spawn(SpawnSpec { model: Some(_) }) => Feature::Model,
            Self::Spawn(_) => Feature::Spawn,
            Self::Prompt(_) => Feature::Prompt,
            Self::Interrupt(_) => Feature::Interrupt,
            Self::AnswerPermission { .. } => Feature::PermAsk,
            Self::Resume => Feature::Resume,
            Self::Close => Feature::Close,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerEvent {
    Ready,
    TurnStarted,
    TurnEnded(TurnEnd),
    PermissionAsked(BlockedContent),
    PermissionSettled,
    Reported,
    Interrupted { id: String, outcome: Option<bool> },
    Closed,
    Unsupported { feature: Feature, reason: String },
    Activity { at: u64 },
    ProcessGone { scope: Scope, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Worker,
    Server(String),
}

pub fn decide(caps: &Capabilities, command: WorkerCommand) -> Result<WorkerCommand, WorkerEvent> {
    let feature = command.feature();
    if !caps.headless() {
        return Err(WorkerEvent::Unsupported {
            feature,
            reason: format!("{} has no headless worker", caps.harness.as_str()),
        });
    }
    match caps.support(feature) {
        Support::None => Err(WorkerEvent::Unsupported {
            feature,
            reason: if feature == Feature::Model {
                model_refusal(caps.harness)
            } else {
                describe(caps, feature)
            },
        }),
        _ => Ok(command),
    }
}

pub fn unsupported_reason(harness: HarnessId, command: WorkerCommand) -> Option<String> {
    decide(&capabilities(harness), command)
        .err()
        .map(|event| match event {
            WorkerEvent::Unsupported { reason, .. } => reason,
            other => format!("{other:?}"),
        })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Starting,
    Idle,
    Running,
    Blocked,
    Closed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeadlessState {
    pub phase: Phase,
    pub last_activity: u64,
    pub liveness: Liveness,
    pub alerted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Liveness {
    #[default]
    Live,
    Quiet {
        since: u64,
    },
    Dead {
        scope: Scope,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Ready,
    Reported,
    Blocked,
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadlessEffect {
    Set(Marker),
    Clear(Marker),
    Publish(BlockedContent),
    ReplyText(String),
}

pub fn reduce(state: &HeadlessState, event: &WorkerEvent) -> (HeadlessState, Vec<HeadlessEffect>) {
    use HeadlessEffect::{Clear, Publish, ReplyText, Set};
    let at = |phase| HeadlessState {
        phase,
        ..state.clone()
    };
    let same = state.clone();
    match event {
        WorkerEvent::Ready => (at(Phase::Idle), vec![Set(Marker::Ready)]),
        WorkerEvent::TurnStarted => (at(Phase::Running), vec![Clear(Marker::Waiting)]),
        WorkerEvent::TurnEnded(_) => (at(Phase::Idle), vec![Clear(Marker::Blocked)]),
        WorkerEvent::PermissionAsked(content) => {
            (at(Phase::Blocked), vec![Publish(content.clone())])
        }
        WorkerEvent::PermissionSettled => (at(Phase::Running), vec![Clear(Marker::Blocked)]),
        WorkerEvent::Reported => (same, vec![Set(Marker::Reported)]),
        WorkerEvent::Interrupted { id, outcome } => {
            (same, vec![ReplyText(interrupt_reply(id, *outcome))])
        }
        WorkerEvent::Closed => (
            at(Phase::Closed),
            vec![
                Clear(Marker::Ready),
                Clear(Marker::Blocked),
                Clear(Marker::Waiting),
            ],
        ),
        WorkerEvent::Unsupported { reason, .. } => (same, vec![ReplyText(reason.clone())]),
        WorkerEvent::Activity { at } => {
            let fresh =
                *at > state.last_activity && !matches!(state.liveness, Liveness::Dead { .. });
            let next = if fresh {
                HeadlessState {
                    last_activity: *at,
                    liveness: Liveness::Live,
                    alerted: false,
                    ..state.clone()
                }
            } else {
                same
            };
            (next, Vec::new())
        }
        WorkerEvent::ProcessGone { scope, reason } => {
            let next = match state.liveness {
                Liveness::Dead { .. } => same,
                _ => HeadlessState {
                    liveness: Liveness::Dead {
                        scope: scope.clone(),
                        reason: reason.clone(),
                    },
                    alerted: false,
                    ..same
                },
            };
            (next, Vec::new())
        }
    }
}

pub trait HeadlessWorker: Send {
    fn capabilities(&self) -> &Capabilities;
    fn send(&mut self, command: WorkerCommand) -> Result<(), String>;
    fn poll(&mut self) -> Vec<WorkerEvent>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_cells_match_the_probed_matrix() {
        let caps = capabilities(HarnessId::Codex);
        for feature in Feature::ALL {
            let expected = match feature {
                Feature::Spawn | Feature::Prompt | Feature::Report | Feature::Model => {
                    Support::Full
                }
                Feature::Interrupt => Support::Partial("outcome unknown after 30 s"),
                Feature::PermAsk => Support::Partial("protocol-raised requests only"),
                Feature::PermYolo => Support::Partial("single-request accepts only"),
                Feature::Resume => {
                    Support::Partial("needs saved history, worktree and codex login")
                }
                Feature::Heartbeat => Support::Partial(
                    "app-server frames count as activity, not progress; silent long tools can yield false Quiet; 300 s default",
                ),
                Feature::Close => {
                    Support::Partial("global trust entry left in ~/.codex/config.toml")
                }
            };
            assert_eq!(caps.support(feature), expected, "{feature:?}");
            if let Some(command) = command_for(feature) {
                assert_eq!(unsupported_reason(HarnessId::Codex, command), None);
            }
        }
    }

    fn command_for(feature: Feature) -> Option<WorkerCommand> {
        match feature {
            Feature::Spawn => Some(WorkerCommand::Spawn(SpawnSpec { model: None })),
            Feature::Model => Some(WorkerCommand::Spawn(SpawnSpec {
                model: Some("m".into()),
            })),
            Feature::Prompt => Some(WorkerCommand::Prompt("p".into())),
            Feature::Interrupt => Some(WorkerCommand::Interrupt("i".into())),
            Feature::PermAsk => Some(WorkerCommand::AnswerPermission {
                option_id: "o".into(),
            }),
            Feature::Resume => Some(WorkerCommand::Resume),
            Feature::Close => Some(WorkerCommand::Close),
            Feature::PermYolo | Feature::Report | Feature::Heartbeat => None,
        }
    }

    fn content() -> BlockedContent {
        BlockedContent {
            tool: "bash".into(),
            summary: "ls".into(),
            options: Vec::new(),
        }
    }

    #[test]
    fn matrix_matches_decide_and_describe() {
        for harness in HarnessId::ALL {
            let caps = capabilities(harness);
            if !caps.headless() {
                continue;
            }
            for feature in Feature::ALL {
                match caps.support(feature) {
                    Support::None => {
                        assert!(describe(&caps, feature).ends_with(": not supported"));
                        if let Some(command) = command_for(feature) {
                            let err = decide(&caps, command).unwrap_err();
                            assert!(
                                matches!(err, WorkerEvent::Unsupported { feature: f, .. } if f == feature)
                            );
                        }
                    }
                    Support::Partial(reason) => {
                        assert!(describe(&caps, feature).contains(reason));
                    }
                    Support::Full => assert!(describe(&caps, feature).ends_with(": full")),
                }
            }
        }
    }

    #[test]
    fn opencode_interrupt_is_full() {
        let caps = capabilities(HarnessId::Opencode);
        assert_eq!(caps.support(Feature::Interrupt), Support::Full);
    }

    #[test]
    fn model_refusal_names_accepting_harnesses() {
        assert_eq!(
            model_refusal(HarnessId::Cursor),
            "cursor does not accept a model; harnesses that do: claude, opencode, cursor, codex"
        );
    }

    #[test]
    fn decide_accepts_a_cursor_model() {
        let spec = SpawnSpec {
            model: Some("gpt-5".into()),
        };
        assert_eq!(
            decide(
                &capabilities(HarnessId::Cursor),
                WorkerCommand::Spawn(spec.clone())
            ),
            Ok(WorkerCommand::Spawn(spec))
        );
    }

    #[test]
    fn describe_reports_cursor_model_as_full() {
        assert_eq!(
            describe(&capabilities(HarnessId::Cursor), Feature::Model),
            "cursor model: full"
        );
    }

    #[test]
    fn describe_keeps_generic_wording_for_unsupported_features() {
        assert_eq!(
            describe(&capabilities(HarnessId::Claude), Feature::Spawn),
            "claude spawn: not supported"
        );
    }

    #[test]
    fn non_headless_harnesses_reject_every_command() {
        for harness in HarnessId::ALL {
            let caps = capabilities(harness);
            assert_eq!(caps.headless(), harness != HarnessId::Claude);
            if caps.headless() {
                continue;
            }
            for feature in Feature::ALL {
                if let Some(command) = command_for(feature) {
                    assert_eq!(
                        decide(&caps, command),
                        Err(WorkerEvent::Unsupported {
                            feature,
                            reason: format!("{} has no headless worker", harness.as_str()),
                        })
                    );
                }
            }
        }
    }

    #[test]
    fn model_refusal_follows_matrix() {
        for harness in HarnessId::ALL {
            let accepts = capabilities(harness).support(Feature::Model) != Support::None;
            for asking in HarnessId::ALL {
                let listed = model_refusal(asking)
                    .split_once("harnesses that do: ")
                    .map(|(_, list)| list.split(", ").any(|name| name == harness.as_str()))
                    .unwrap_or(false);
                assert_eq!(listed, accepts);
            }
        }
    }

    #[test]
    fn reduce_maps_each_event() {
        use HeadlessEffect::*;
        let from = |phase| HeadlessState {
            phase,
            ..HeadlessState::default()
        };
        let cases = vec![
            (
                Phase::Starting,
                WorkerEvent::Ready,
                Phase::Idle,
                vec![Set(Marker::Ready)],
            ),
            (
                Phase::Idle,
                WorkerEvent::TurnStarted,
                Phase::Running,
                vec![Clear(Marker::Waiting)],
            ),
            (
                Phase::Running,
                WorkerEvent::TurnEnded(TurnEnd::Succeeded),
                Phase::Idle,
                vec![Clear(Marker::Blocked)],
            ),
            (
                Phase::Running,
                WorkerEvent::PermissionAsked(content()),
                Phase::Blocked,
                vec![Publish(content())],
            ),
            (
                Phase::Blocked,
                WorkerEvent::PermissionSettled,
                Phase::Running,
                vec![Clear(Marker::Blocked)],
            ),
            (
                Phase::Idle,
                WorkerEvent::Reported,
                Phase::Idle,
                vec![Set(Marker::Reported)],
            ),
            (
                Phase::Running,
                WorkerEvent::Interrupted {
                    id: "w".into(),
                    outcome: Some(true),
                },
                Phase::Running,
                vec![ReplyText(interrupt_reply("w", Some(true)))],
            ),
            (
                Phase::Idle,
                WorkerEvent::Closed,
                Phase::Closed,
                vec![
                    Clear(Marker::Ready),
                    Clear(Marker::Blocked),
                    Clear(Marker::Waiting),
                ],
            ),
            (
                Phase::Idle,
                WorkerEvent::Unsupported {
                    feature: Feature::Model,
                    reason: "r".into(),
                },
                Phase::Idle,
                vec![ReplyText("r".into())],
            ),
        ];
        for (start, event, phase, effects) in cases {
            assert_eq!(reduce(&from(start), &event), (from(phase), effects));
        }
    }

    struct Fake {
        caps: Capabilities,
        sent: Vec<WorkerCommand>,
        queued: Vec<WorkerEvent>,
    }

    impl HeadlessWorker for Fake {
        fn capabilities(&self) -> &Capabilities {
            &self.caps
        }
        fn send(&mut self, command: WorkerCommand) -> Result<(), String> {
            self.sent.push(command);
            Ok(())
        }
        fn poll(&mut self) -> Vec<WorkerEvent> {
            std::mem::take(&mut self.queued)
        }
    }

    #[test]
    fn drives_decide_send_poll_reduce() {
        let mut worker = Fake {
            caps: capabilities(HarnessId::Opencode),
            sent: Vec::new(),
            queued: vec![WorkerEvent::TurnStarted],
        };
        let command = decide(worker.capabilities(), WorkerCommand::Prompt("go".into())).unwrap();
        worker.send(command.clone()).unwrap();
        assert_eq!(worker.sent, vec![command]);
        let mut state = HeadlessState::default();
        for event in worker.poll() {
            state = reduce(&state, &event).0;
        }
        assert_eq!(state.phase, Phase::Running);
    }
}
