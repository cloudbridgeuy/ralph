use ralph_core::chat_send_it::{
    parse_run, plan_hash, propose_refusal, read_run, write_run, Limits, RunFile,
};
use serde_json::Value;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const RUN_DIR: &str = "send-it";
const RUN_FILE: &str = "run.toml";

#[derive(Debug, Clone)]
pub(crate) struct SendIt {
    path: PathBuf,
    limits: Limits,
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
}
