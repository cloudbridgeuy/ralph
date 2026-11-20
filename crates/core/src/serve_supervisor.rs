use crate::chat_process::SessionKey;

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
    Closed(SessionKey),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Open(SessionKey),
    Answer { proto: u32 },
    Refuse(String),
    Close(SessionKey),
    Exit,
}

pub struct Supervisor {
    phase: Phase,
    open: Vec<SessionKey>,
}

impl Supervisor {
    pub fn start() -> (Self, Vec<Effect>) {
        (
            Self {
                phase: Phase::Running,
                open: vec![SessionKey::Main],
            },
            vec![Effect::Open(SessionKey::Main)],
        )
    }

    pub fn step(&mut self, input: Input) -> Vec<Effect> {
        match (self.phase, input) {
            (Phase::Running, Input::Hello { proto }) => vec![Effect::Answer { proto }],
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
            (phase, Input::Closed(key)) => {
                let before = self.open.len();
                self.open.retain(|open| *open != key);
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn closed_unknown_key_is_ignored() {
        let (mut sup, _) = Supervisor::start();
        sup.step(Input::Closed(SessionKey::Main));
        assert_eq!(sup.step(Input::Closed(SessionKey::Main)), Vec::new());
    }
}
