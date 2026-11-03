use crate::chat::editor::open_in_editor;
use ralph_core::chat_send_it::{
    approve, parse_run, plan_hash, propose_refusal, read_run, render_status, write_run, Limits,
    RunFile,
};
use serde_json::Value;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const RUN_DIR: &str = "send-it";
const RUN_FILE: &str = "run.toml";
const NO_RUN: &str = "No send-it run. Start one with /send-it <instructions>.";
const NOT_PROPOSED: &str =
    "No new run plan was stored. Try /send-it again with clearer instructions.";

#[derive(Debug, Clone)]
pub(crate) struct SendIt {
    path: PathBuf,
    limits: Limits,
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

impl SendIt {
    pub(crate) fn new(store_dir: &Path, limits: Limits) -> Self {
        Self {
            path: store_dir.join(RUN_DIR).join(RUN_FILE),
            limits,
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

    pub(crate) fn go(&self, shown: Option<&str>, typed_at_prompt: bool) -> String {
        let file = match self.load() {
            Ok(Some(file)) => file,
            Ok(None) => return NO_RUN.to_string(),
            Err(text) => return text,
        };
        let approved = approve(&file, shown, typed_at_prompt)
            .and_then(|approved| self.save(&approved).map(|()| approved));
        match approved {
            Ok(approved) => format!("Plan {} approved.", plan_hash(&approved.plan)),
            Err(text) => text,
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
    use serde_json::json;

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
            send_it.go(Some(&hash), true),
            format!("Plan {hash} approved.")
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
            .contains("type it at the prompt"));
        assert!(send_it.go(None, true).contains("/send-it status"));
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
        assert_eq!(send_it.go(Some("x"), true), NO_RUN);
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
}
