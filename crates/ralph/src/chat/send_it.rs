use crate::chat::editor::open_in_editor;
use crate::chat::monitor::Monitor;
use crate::inbox_alert;
use crate::watches::{locked, OutcomeSlot};
use ralph_core::chat_hold::HoldFilter;
use ralph_core::chat_send_it::{
    approve, blocked_alert, blocked_report, check_spec, done_notice, judge_prompt, parse_run,
    plan_hash, propose_refusal, read_run, render_status, running_line, start_run, tick, write_run,
    Action, Driving, Ended, Limits, Next, RunFile, StopReason, Ticked,
};
use ralph_core::chat_status::Route;
use serde_json::Value;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const RUN_DIR: &str = "send-it";
const RUN_FILE: &str = "run.toml";
const NO_RUN: &str = "No send-it run. Start one with /send-it <instructions>.";
const NOT_PROPOSED: &str =
    "No new run plan was stored. Try /send-it again with clearer instructions.";

#[derive(Debug, Clone)]
pub(crate) struct SendIt {
    path: PathBuf,
    limits: Limits,
    refused: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Screen {
    pub(crate) text: String,
    pub(crate) shown: Option<String>,
}

impl Screen {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            shown: None,
        }
    }
}

pub(super) struct Active {
    driving: Driving,
    watched: Option<OutcomeSlot>,
}

pub(super) struct Driven {
    pub(super) active: Option<Active>,
    pub(super) route: Option<Route>,
}

impl Driven {
    fn keep(driving: Driving, route: Route, watched: Option<OutcomeSlot>) -> Self {
        Self {
            active: Some(Active { driving, watched }),
            route: Some(route),
        }
    }

    fn ended() -> Self {
        Self {
            active: None,
            route: None,
        }
    }
}

fn stopped(reason: StopReason, driving: &Driving, monitor: &Monitor) -> Driven {
    let notice = reason.notice(&driving.run);
    println!("{notice}");
    if let Some(alert) = blocked_alert(&notice, &monitor.alert) {
        inbox_alert::deliver(&alert);
    }
    monitor.term.send(&blocked_report(&notice));
    Driven::ended()
}

pub(super) fn took_over(active: &Active, monitor: &Monitor) {
    stopped(StopReason::TookOver, &active.driving, monitor);
}

impl SendIt {
    pub(crate) fn new(store_dir: &Path, limits: Limits) -> Self {
        Self {
            path: store_dir.join(RUN_DIR).join(RUN_FILE),
            limits,
            refused: Arc::default(),
        }
    }

    fn load(&self) -> Result<Option<RunFile>, String> {
        match fs::read_to_string(&self.path) {
            Ok(text) => read_run(&text, self.limits).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("Could not read {}: {error}", self.path.display())),
        }
    }

    fn save(&self, file: &RunFile) -> Result<(), String> {
        let text = write_run(file)?;
        self.path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&self.path, text))
            .map_err(|error| format!("Could not write {}: {error}", self.path.display()))
    }

    pub(crate) fn allowed(&self) -> Vec<Action> {
        self.load()
            .ok()
            .flatten()
            .filter(RunFile::is_approved)
            .map(|file| file.plan.allowed)
            .unwrap_or_default()
    }

    pub(crate) fn refuse(&self, tool: String) {
        *locked(&self.refused) = Some(tool);
    }

    pub(crate) fn refusal(&self) -> Option<String> {
        propose_refusal(self.load().ok().flatten().as_ref())
    }

    pub(crate) fn current_hash(&self) -> Option<String> {
        self.load().ok().flatten().map(|file| plan_hash(&file.plan))
    }

    pub(crate) fn propose(&self, args: &Value) -> Result<String, String> {
        if let Some(refusal) = self.refusal() {
            return Err(refusal);
        }
        let plan = parse_run(args, self.limits)?;
        let hash = plan_hash(&plan);
        self.save(&RunFile {
            plan,
            approved: None,
        })?;
        Ok(format!(
            "Stored run plan {hash}. Stop here: ralph shows the plan and the person approves it with /send-it go."
        ))
    }

    pub(crate) fn status(&self) -> Screen {
        match self.load() {
            Ok(Some(file)) => Screen {
                text: render_status(&file),
                shown: Some(plan_hash(&file.plan)),
            },
            Ok(None) => Screen::text(NO_RUN),
            Err(text) => Screen::text(text),
        }
    }

    pub(crate) fn proposed(&self, before: Option<String>) -> Screen {
        let screen = self.status();
        let stored_new = match &screen.shown {
            Some(hash) => before.as_ref() != Some(hash),
            None => screen.text != NO_RUN,
        };
        if stored_new {
            screen
        } else {
            Screen::text(NOT_PROPOSED)
        }
    }

    pub(crate) fn go(
        &self,
        shown: Option<&str>,
        typed_at_prompt: bool,
    ) -> (String, Option<Ticked>) {
        let file = match self.load() {
            Ok(Some(file)) => file,
            Ok(None) => return (NO_RUN.to_string(), None),
            Err(text) => return (text, None),
        };
        let approved = approve(&file, shown, typed_at_prompt)
            .and_then(|approved| self.save(&approved).map(|()| approved));
        match approved {
            Ok(approved) => (
                format!("Plan {} approved. Running it.", plan_hash(&approved.plan)),
                start_run(&approved).map(|(driving, next)| Ticked {
                    driving,
                    next,
                    met: None,
                }),
            ),
            Err(text) => (text, None),
        }
    }

    pub(super) fn tick(&self, active: &Active, ended: Ended) -> Ticked {
        let file = self.load().ok().flatten();
        let watched = active
            .watched
            .as_ref()
            .and_then(|slot| locked(slot).clone());
        let refused = locked(&self.refused).take();
        tick(
            &active.driving,
            &Ended {
                file: file.as_ref(),
                watched: watched.as_ref(),
                refused: refused.as_deref(),
                ..ended
            },
        )
    }

    pub(super) fn dispatch(&self, ticked: Ticked, monitor: &Monitor) -> Driven {
        if let Some(met) = &ticked.met {
            println!("{met}");
        }
        let plan = self.load().ok().flatten().map(|file| file.plan);
        let driving = ticked.driving;
        match ticked.next {
            Next::Send(line) => {
                if let Some(plan) = &plan {
                    println!("{}", running_line(plan, &driving));
                }
                println!("{line}");
                Driven::keep(driving, Route::Send(line), None)
            }
            Next::Judge => match plan.and_then(|plan| judge_prompt(&plan, driving.progress)) {
                Some(prompt) => {
                    println!("{prompt}");
                    Driven::keep(driving, Route::Send(prompt), None)
                }
                None => stopped(StopReason::PlanChanged, &driving, monitor),
            },
            Next::Hold(worker) => {
                Driven::keep(driving, Route::Hold(HoldFilter::Source(worker)), None)
            }
            Next::Watch(command) | Next::Check(command) => {
                let spec = check_spec(driving.progress.step, &command);
                let id = spec.id.clone();
                match monitor.watches.start(spec) {
                    Ok(slot) => {
                        Driven::keep(driving, Route::Hold(HoldFilter::Source(id)), Some(slot))
                    }
                    Err(error) => {
                        println!("{error}");
                        let step = driving.progress.step + 1;
                        stopped(StopReason::CheckFailed { step }, &driving, monitor)
                    }
                }
            }
            Next::Stop(reason) => stopped(reason, &driving, monitor),
            Next::Done => {
                if let Some(plan) = &plan {
                    println!("{}", done_notice(plan, &driving.run));
                }
                Driven::ended()
            }
            Next::Wait => Driven::ended(),
        }
    }

    pub(crate) fn off(&self) -> String {
        match fs::remove_file(&self.path) {
            Ok(()) => "send-it off: the run plan is deleted.".to_string(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => NO_RUN.to_string(),
            Err(error) => format!("Could not delete {}: {error}", self.path.display()),
        }
    }

    pub(crate) fn edit(&self, argv: &[String]) -> Screen {
        if !self.path.exists() {
            return Screen::text(NO_RUN);
        }
        let opened = open_in_editor(argv, &self.path);
        let screen = self.clear_approval();
        match opened {
            Ok(()) => screen,
            Err(error) => Screen {
                text: format!("Editor failed: {error}\n{}", screen.text),
                ..screen
            },
        }
    }

    fn clear_approval(&self) -> Screen {
        match self.load() {
            Ok(Some(file)) => match self.save(&RunFile {
                approved: None,
                ..file
            }) {
                Ok(()) => self.status(),
                Err(text) => Screen::text(text),
            },
            Ok(None) => Screen::text(NO_RUN),
            Err(text) => Screen::text(format!("{text}\nFix it with /send-it edit.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::memory::Store;
    use crate::chat::socket::{respond, Shared};
    use crate::chat::suggestion::SuggestionSlot;
    use crate::workers::Workers;
    use ralph_core::chat_memory::Trigger;
    use ralph_core::chat_socket::SocketRequest;
    use ralph_core::watch::WatchOutcome;
    use serde_json::json;
    use std::sync::Mutex;

    const LIMITS: Limits = Limits {
        max_turns: 3,
        max_usd: 1.0,
    };

    fn plan_args() -> Value {
        json!({
            "goal": "ship GUZ-1",
            "steps": [{"do": "run the checks", "criterion": {"kind": "check_passes", "command": "cargo xtask lint"}}]
        })
    }

    fn proposed(dir: &Path) -> (SendIt, String) {
        let send_it = SendIt::new(dir, LIMITS);
        send_it.propose(&plan_args()).unwrap();
        let hash = send_it.status().shown.unwrap();
        (send_it, hash)
    }

    #[test]
    fn propose_stores_the_plan_under_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = SendIt::new(dir.path(), LIMITS);
        let stored = send_it.propose(&plan_args()).unwrap();
        let text = fs::read_to_string(dir.path().join("send-it/run.toml")).unwrap();
        let file = read_run(&text, LIMITS).unwrap();
        assert_eq!(file.approved, None);
        let hash = plan_hash(&file.plan);
        assert!(
            stored.starts_with(&format!("Stored run plan {hash}.")),
            "{stored}"
        );
    }

    #[test]
    fn propose_rejects_a_step_without_a_criterion_and_stores_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = SendIt::new(dir.path(), LIMITS);
        let args = json!({"goal": "g", "steps": [{"do": "x"}]});
        assert_eq!(
            send_it.propose(&args),
            Err("Step 1 has no exit criterion. Give each step a check ralph can measure.".into())
        );
        assert!(!dir.path().join("send-it").exists());
    }

    #[test]
    fn propose_refuses_while_an_approved_plan_exists() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = SendIt::new(dir.path(), LIMITS);
        let plan = parse_run(&plan_args(), LIMITS).unwrap();
        let hash = plan_hash(&plan);
        let draft = RunFile {
            plan,
            approved: None,
        };
        send_it.save(&draft).unwrap();
        assert!(send_it.propose(&plan_args()).is_ok());
        send_it
            .save(&RunFile {
                approved: Some(hash.clone()),
                ..draft
            })
            .unwrap();
        assert_eq!(
            send_it.propose(&plan_args()),
            Err(format!(
                "Plan {hash} is approved and active. Stop it with /send-it off first."
            ))
        );
    }

    #[test]
    fn an_approved_run_blocks_a_new_proposal_until_off() {
        let dir = tempfile::tempdir().unwrap();
        let (send_it, hash) = proposed(dir.path());
        assert_eq!(
            send_it.go(Some(&hash), true).0,
            format!("Plan {hash} approved. Running it.")
        );
        let refused = send_it.propose(&plan_args()).unwrap_err();
        assert!(refused.contains("approved and active"), "{refused}");
        assert_eq!(send_it.off(), "send-it off: the run plan is deleted.");
        assert!(send_it.propose(&plan_args()).is_ok());
    }

    #[test]
    fn go_needs_the_prompt_and_the_hash_on_screen() {
        let dir = tempfile::tempdir().unwrap();
        let (send_it, hash) = proposed(dir.path());
        assert!(send_it
            .go(Some(&hash), false)
            .0
            .contains("type it at the prompt"));
        assert!(send_it.go(None, true).0.contains("/send-it status"));
        assert!(send_it
            .status()
            .text
            .ends_with("not approved. Type /send-it go to approve this plan."));
        send_it.go(Some(&hash), true);
        assert!(send_it
            .status()
            .text
            .ends_with(&format!("Plan {hash}: approved.")));
    }

    #[test]
    fn clearing_after_an_edit_drops_the_approval_and_shows_the_plan() {
        let dir = tempfile::tempdir().unwrap();
        let (send_it, hash) = proposed(dir.path());
        send_it.go(Some(&hash), true);
        let screen = send_it.clear_approval();
        assert_eq!(screen.shown, Some(hash));
        assert!(screen
            .text
            .ends_with("not approved. Type /send-it go to approve this plan."));
        fs::write(&send_it.path, "goal = ").unwrap();
        let broken = send_it.clear_approval();
        assert_eq!(broken.shown, None);
        assert!(
            broken.text.ends_with("Fix it with /send-it edit."),
            "{}",
            broken.text
        );
    }

    #[test]
    fn without_a_run_every_command_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = SendIt::new(dir.path(), LIMITS);
        let none = Screen {
            text: NO_RUN.to_string(),
            shown: None,
        };
        assert_eq!(send_it.status(), none);
        assert_eq!(send_it.clear_approval(), none);
        assert_eq!(send_it.go(Some("x"), true), (NO_RUN.to_string(), None));
        assert_eq!(send_it.off(), NO_RUN);
        assert!(NO_RUN.starts_with("No send-it run."));
    }

    #[test]
    fn a_turn_that_stores_no_new_plan_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = SendIt::new(dir.path(), LIMITS);
        assert_eq!(send_it.proposed(None).text, NOT_PROPOSED);
        let (send_it, hash) = proposed(dir.path());
        assert_eq!(send_it.proposed(Some(hash.clone())).text, NOT_PROPOSED);
        assert_eq!(send_it.proposed(None).shown, Some(hash));
    }

    const STEP_1: &str = "[send-it 1/1] run the checks · exit: `cargo xtask lint` exits 0";

    fn started(dir: &Path) -> (SendIt, Active) {
        let (send_it, hash) = proposed(dir);
        let ticked = send_it.go(Some(&hash), true).1.unwrap();
        let active = Active {
            driving: ticked.driving,
            watched: None,
        };
        (send_it, active)
    }

    fn ended() -> Ended<'static> {
        Ended {
            file: None,
            replied: true,
            asked: false,
            typed: false,
            queued: &[],
            cost_usd: 0.1,
            reply: "",
            drained: &[],
            unread: &[],
            watched: None,
            refused: None,
        }
    }

    #[test]
    fn only_an_approved_plan_lends_its_allowed_list() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = SendIt::new(dir.path(), LIMITS);
        assert_eq!(send_it.allowed(), Vec::new());
        let mut args = plan_args();
        args["allowed"] = json!([{"kind": "reply"}, {"kind": "watch"}]);
        send_it.propose(&args).unwrap();
        assert_eq!(send_it.allowed(), Vec::new());
        let hash = send_it.status().shown.unwrap();
        send_it.go(Some(&hash), true);
        assert_eq!(send_it.allowed(), vec![Action::Reply, Action::Watch]);
    }

    #[test]
    fn a_refusal_from_the_socket_stops_the_next_tick_once_and_keeps_the_approval() {
        let dir = tempfile::tempdir().unwrap();
        let (send_it, active) = started(dir.path());
        send_it.clone().refuse("spawn_worker codex".to_string());
        assert_eq!(
            send_it.tick(&active, ended()).next,
            Next::Stop(StopReason::Refused {
                tool: "spawn_worker codex".to_string()
            })
        );
        assert_eq!(
            send_it.tick(&active, ended()).next,
            Next::Check("cargo xtask lint".to_string())
        );
        assert!(send_it.load().unwrap().unwrap().is_approved());
    }

    #[test]
    fn the_socket_runs_a_listed_tool_and_a_refused_one_stops_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let shared = Shared {
            store: Store::new(dir.path().to_path_buf()),
            workers: Workers::new("/s".into(), "/bin/ralph".into(), "auto".into(), None),
            suggestions: SuggestionSlot::default(),
            send_it: SendIt::new(dir.path(), LIMITS),
        };
        let mut args = plan_args();
        args["allowed"] = json!([{"kind": "watch"}]);
        shared.send_it.propose(&args).unwrap();
        let hash = shared.send_it.status().shown.unwrap();
        let driving = shared.send_it.go(Some(&hash), true).1.unwrap().driving;
        let active = Active {
            driving,
            watched: None,
        };
        shared.store.set_pending_turn(Trigger::SendIt).unwrap();
        let call = |tool: &str, args: Value| {
            respond(
                &shared,
                &SocketRequest {
                    tool: tool.to_string(),
                    args,
                },
            )
        };
        let watched = call(
            "watch",
            json!({"id": "ci", "command": "c", "follow_up": "f"}),
        );
        assert!(!watched.is_error, "{}", watched.text);
        let spawned = call(
            "spawn_worker",
            json!({"id": "A", "task": "t", "harness": "codex"}),
        );
        assert!(spawned.is_error);
        assert!(
            spawned.text.contains("`spawn_worker codex`"),
            "{}",
            spawned.text
        );
        assert_eq!(
            shared.send_it.tick(&active, ended()).next,
            Next::Stop(StopReason::Refused {
                tool: "spawn_worker codex".to_string()
            })
        );
    }

    #[test]
    fn go_starts_only_the_approved_plan_at_step_one() {
        let dir = tempfile::tempdir().unwrap();
        let (send_it, hash) = proposed(dir.path());
        assert!(send_it.go(None, true).1.is_none());
        let ticked = send_it.go(Some(&hash), true).1.unwrap();
        let Next::Send(brief) = ticked.next else {
            panic!("{:?}", ticked.next)
        };
        assert!(brief.ends_with(STEP_1), "{brief}");
        assert!(brief.starts_with("send-it run "), "{brief}");
    }

    #[test]
    fn every_tick_reads_the_plan_file_again() {
        let dir = tempfile::tempdir().unwrap();
        let (send_it, active) = started(dir.path());
        assert_eq!(
            send_it.tick(&active, ended()).next,
            Next::Check("cargo xtask lint".to_string())
        );
        send_it.clear_approval();
        assert_eq!(
            send_it.tick(&active, ended()).next,
            Next::Stop(StopReason::PlanChanged)
        );
        send_it.off();
        assert_eq!(
            send_it.tick(&active, ended()).next,
            Next::Stop(StopReason::Off)
        );
    }

    #[test]
    fn a_tick_takes_the_outcome_the_watch_left_in_its_slot() {
        let dir = tempfile::tempdir().unwrap();
        let (send_it, active) = started(dir.path());
        let waiting = send_it.tick(&active, ended()).driving;
        for (outcome, next) in [
            (WatchOutcome::Exited(0), Next::Done),
            (
                WatchOutcome::Exited(1),
                Next::Stop(StopReason::CheckFailed { step: 1 }),
            ),
        ] {
            let active = Active {
                driving: waiting.clone(),
                watched: Some(OutcomeSlot::new(Mutex::new(Some(outcome)))),
            };
            assert_eq!(send_it.tick(&active, ended()).next, next);
        }
    }
}
