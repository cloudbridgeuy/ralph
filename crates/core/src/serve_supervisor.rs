use crate::chat_process::SessionKey;
use crate::chat_thread_cmd::ThreadOp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Draining,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Hello { proto: u32 },
    Stop,
    Send(SessionKey),
    TurnDone(SessionKey),
    Thread(ThreadOp),
    Closed(SessionKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Open(SessionKey),
    Answer { proto: u32 },
    Refuse(String),
    Run(SessionKey),
    RunThread(ThreadOp),
    Close(SessionKey),
    Exit,
}

pub struct Supervisor {
    phase: Phase,
    open: Vec<SessionKey>,
    busy: Vec<SessionKey>,
}

impl Supervisor {
    pub fn start() -> (Self, Vec<Effect>) {
        (
            Self {
                phase: Phase::Running,
                open: vec![SessionKey::Main],
                busy: Vec::new(),
            },
            vec![Effect::Open(SessionKey::Main)],
        )
    }

    pub fn step(&mut self, input: Input) -> Vec<Effect> {
        match (self.phase, input) {
            (Phase::Running, Input::Hello { proto }) => vec![Effect::Answer { proto }],
            (Phase::Running, Input::Send(key)) if self.busy.contains(&key) => {
                vec![Effect::Refuse(format!("{key} is busy"))]
            }
            (Phase::Running, Input::Send(key)) if self.open.contains(&key) => {
                self.busy.push(key.clone());
                vec![Effect::Run(key)]
            }
            (Phase::Running, Input::Send(key)) => {
                vec![Effect::Refuse(format!("{key} is not open"))]
            }
            (_, Input::Send(_)) => vec![Effect::Refuse("ralph serve is shutting down".to_string())],
            (Phase::Running, Input::Thread(op)) => match self.busy_target(&op) {
                Some(key) => vec![Effect::Refuse(format!("{key} is busy"))],
                None => vec![Effect::RunThread(op)],
            },
            (_, Input::Thread(_)) => {
                vec![Effect::Refuse("ralph serve is shutting down".to_string())]
            }
            (_, Input::Hello { .. }) => {
                vec![Effect::Refuse("ralph serve is shutting down".to_string())]
            }
            (Phase::Running, Input::Stop) if self.open.is_empty() => {
                self.phase = Phase::Done;
                vec![Effect::Exit]
            }
            (Phase::Running, Input::Stop) => {
                self.phase = Phase::Draining;
                self.open.iter().cloned().map(Effect::Close).collect()
            }
            (_, Input::Stop) => Vec::new(),
            (_, Input::TurnDone(key)) => {
                self.busy.retain(|busy| *busy != key);
                Vec::new()
            }
            (phase, Input::Closed(key)) => {
                let before = self.open.len();
                self.open.retain(|open| *open != key);
                self.busy.retain(|busy| *busy != key);
                if phase == Phase::Draining && before != self.open.len() && self.open.is_empty() {
                    self.phase = Phase::Done;
                    vec![Effect::Exit]
                } else {
                    Vec::new()
                }
            }
        }
    }
}

impl Supervisor {
    fn busy_target(&self, op: &ThreadOp) -> Option<SessionKey> {
        let name = match op {
            ThreadOp::Merge { name, .. } | ThreadOp::Park(name) | ThreadOp::Drop(name) => name,
            _ => return None,
        };
        let key = SessionKey::Thread(name.clone());
        self.busy.contains(&key).then_some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_thread::ThreadName;

    #[test]
    fn start_opens_main() {
        let (_, effects) = Supervisor::start();
        assert_eq!(effects, vec![Effect::Open(SessionKey::Main)]);
    }

    #[test]
    fn exit_only_after_main_closed() {
        let (mut sup, _) = Supervisor::start();
        assert_eq!(sup.step(Input::Stop), vec![Effect::Close(SessionKey::Main)]);
        assert_eq!(
            sup.step(Input::Closed(SessionKey::Main)),
            vec![Effect::Exit]
        );
    }

    #[test]
    fn second_stop_is_a_no_op() {
        let (mut sup, _) = Supervisor::start();
        sup.step(Input::Stop);
        assert_eq!(sup.step(Input::Stop), Vec::new());
    }

    #[test]
    fn hello_answered_then_refused_while_draining() {
        let (mut sup, _) = Supervisor::start();
        assert_eq!(
            sup.step(Input::Hello { proto: 1 }),
            vec![Effect::Answer { proto: 1 }]
        );
        sup.step(Input::Stop);
        assert_eq!(
            sup.step(Input::Hello { proto: 1 }),
            vec![Effect::Refuse("ralph serve is shutting down".to_string())]
        );
    }

    #[test]
    fn send_to_open_main_runs_a_turn() {
        let (mut sup, _) = Supervisor::start();
        assert_eq!(
            sup.step(Input::Send(SessionKey::Main)),
            vec![Effect::Run(SessionKey::Main)]
        );
    }

    #[test]
    fn send_to_a_closed_thread_is_refused() {
        let (mut sup, _) = Supervisor::start();
        let key = SessionKey::Thread(ThreadName::parse("side").unwrap());
        assert_eq!(
            sup.step(Input::Send(key)),
            vec![Effect::Refuse("thread side is not open".to_string())]
        );
    }

    #[test]
    fn send_while_draining_is_refused() {
        let (mut sup, _) = Supervisor::start();
        sup.step(Input::Stop);
        assert_eq!(
            sup.step(Input::Send(SessionKey::Main)),
            vec![Effect::Refuse("ralph serve is shutting down".to_string())]
        );
    }

    #[test]
    fn send_to_busy_session_is_refused() {
        let (mut sup, _) = Supervisor::start();
        sup.step(Input::Send(SessionKey::Main));
        assert_eq!(
            sup.step(Input::Send(SessionKey::Main)),
            vec![Effect::Refuse("main is busy".to_string())]
        );
    }

    #[test]
    fn turn_done_frees_the_session() {
        let (mut sup, _) = Supervisor::start();
        sup.step(Input::Send(SessionKey::Main));
        sup.step(Input::TurnDone(SessionKey::Main));
        assert_eq!(
            sup.step(Input::Send(SessionKey::Main)),
            vec![Effect::Run(SessionKey::Main)]
        );
    }

    #[test]
    fn stop_while_busy_closes_after() {
        let (mut sup, _) = Supervisor::start();
        sup.step(Input::Send(SessionKey::Main));
        assert_eq!(sup.step(Input::Stop), vec![Effect::Close(SessionKey::Main)]);
        assert_eq!(sup.step(Input::TurnDone(SessionKey::Main)), Vec::new());
        assert_eq!(
            sup.step(Input::Closed(SessionKey::Main)),
            vec![Effect::Exit]
        );
    }

    fn side() -> ThreadName {
        ThreadName::parse("side").unwrap()
    }

    #[test]
    fn merge_of_busy_thread_is_refused() {
        let (mut sup, _) = Supervisor::start();
        sup.open.push(SessionKey::Thread(side()));
        sup.step(Input::Send(SessionKey::Thread(side())));
        for op in [
            ThreadOp::Merge {
                name: side(),
                view: false,
            },
            ThreadOp::Park(side()),
            ThreadOp::Drop(side()),
        ] {
            assert_eq!(
                sup.step(Input::Thread(op)),
                vec![Effect::Refuse("thread side is busy".to_string())]
            );
        }
        let pull = ThreadOp::Pull(side());
        assert_eq!(
            sup.step(Input::Thread(pull.clone())),
            vec![Effect::RunThread(pull)]
        );
    }

    #[test]
    fn draining_refuses_thread_ops() {
        let (mut sup, _) = Supervisor::start();
        sup.step(Input::Stop);
        assert_eq!(
            sup.step(Input::Thread(ThreadOp::Pull(side()))),
            vec![Effect::Refuse("ralph serve is shutting down".to_string())]
        );
    }

    #[test]
    fn closed_unknown_key_is_ignored() {
        let (mut sup, _) = Supervisor::start();
        sup.step(Input::Closed(SessionKey::Main));
        assert_eq!(sup.step(Input::Closed(SessionKey::Main)), Vec::new());
    }
}
