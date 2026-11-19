use crate::chat::ask::Notices;
use crate::chat::memory::Store;
use crate::warn::warn_if_err;
use crate::watches::locked;
use ralph_core::chat_local::notice_record;
use ralph_core::chat_memory::Role;
use ralph_core::chat_send_it::template::{
    match_template, render_templates, save_notice, template_name, template_text,
};
use ralph_core::chat_send_it::{
    parse_run, plan_hash, plan_warnings, propose_refusal, read_run, stop_file, write_run, Action,
    Limits, RunFile, StepResult,
};
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const NO_RUN: &str = "No send-it run. Start one with /send-it <instructions>.";
const RUN_DIR: &str = "send-it";
const RUN_FILE: &str = "run.toml";
const TEMPLATE_DIR: &str = "templates";

pub fn unreadable(path: &Path, error: io::Error) -> String {
    format!("Could not read {}: {error}", path.display())
}

#[derive(Debug, Clone)]
pub struct SendIt {
    pub log: PathBuf,
    pub path: PathBuf,
    pub templates: PathBuf,
    pub limits: Limits,
    pub refused: Arc<Mutex<Option<String>>>,
    pub judged: Arc<Mutex<Option<StepResult>>>,
    pub answered: Arc<Mutex<Vec<String>>>,
    out: Out,
}

#[derive(Clone)]
struct Out(Notices);

impl fmt::Debug for Out {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Out")
    }
}

impl SendIt {
    pub fn new(store_dir: &Path, limits: Limits, notices: Notices) -> Self {
        Self {
            log: store_dir.to_path_buf(),
            path: store_dir.join(RUN_DIR).join(RUN_FILE),
            templates: store_dir.join(RUN_DIR).join(TEMPLATE_DIR),
            limits,
            refused: Arc::default(),
            judged: Arc::default(),
            answered: Arc::default(),
            out: Out(notices),
        }
    }

    pub fn show(&self, line: impl fmt::Display) {
        (self.out.0)(&line.to_string());
    }

    pub fn load(&self) -> Result<Option<RunFile>, String> {
        match fs::read_to_string(&self.path) {
            Ok(text) => read_run(&text, self.limits).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(unreadable(&self.path, error)),
        }
    }

    pub fn save(&self, file: &RunFile) -> Result<(), String> {
        let text = write_run(file)?;
        self.path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&self.path, text))
            .map_err(|error| format!("Could not write {}: {error}", self.path.display()))
    }

    pub fn allowed(&self) -> Option<Vec<Action>> {
        self.load()
            .ok()
            .flatten()
            .filter(RunFile::is_approved)
            .map(|file| file.plan.allowed)
    }

    pub fn refuse(&self, tool: String) {
        *locked(&self.refused) = Some(tool);
    }

    pub fn answer(&self, worker: String) {
        locked(&self.answered).push(worker);
    }

    pub fn record(&self, result: StepResult) {
        *locked(&self.judged) = Some(result);
    }

    pub fn refusal(&self) -> Option<String> {
        propose_refusal(self.load().ok().flatten().as_ref())
    }

    pub fn current_hash(&self) -> Option<String> {
        self.load().ok().flatten().map(|file| plan_hash(&file.plan))
    }

    pub fn propose(&self, args: &Value) -> Result<String, String> {
        if let Some(refusal) = self.refusal() {
            return Err(refusal);
        }
        let plan = parse_run(args, self.limits)?;
        let hash = plan_hash(&plan);
        let warnings = plan_warnings(&plan);
        self.save(&RunFile {
            plan,
            approved: None,
            stopped: None,
        })?;
        let stored = format!(
            "Stored run plan {hash}. Stop here: ralph shows the plan and the person approves it with /send-it go."
        );
        Ok(std::iter::once(stored)
            .chain(warnings)
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn off(&self) -> String {
        match fs::remove_file(&self.path) {
            Ok(()) => "send-it off: the run plan is deleted.".to_string(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => NO_RUN.to_string(),
            Err(error) => format!("Could not delete {}: {error}", self.path.display()),
        }
    }

    pub fn save_template(&self, name: &str, ticket: Option<&str>) -> String {
        let saved = template_name(name).and_then(|name| {
            let file = self.load()?.ok_or(NO_RUN)?;
            let text = template_text(&file.plan, ticket)?;
            let path = self.templates.join(format!("{name}.toml"));
            let replaced = path.exists();
            fs::create_dir_all(&self.templates)
                .and_then(|()| fs::write(&path, &text))
                .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
            Ok(save_notice(name, ticket, &text, replaced))
        });
        saved.unwrap_or_else(|text| text)
    }

    pub fn template_names(&self) -> Result<Vec<String>, String> {
        let entries = match fs::read_dir(&self.templates) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(unreadable(&self.templates, error)),
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .filter_map(|path| Some(path.file_stem()?.to_str()?.to_string()))
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn templates(&self) -> String {
        self.template_names()
            .map_or_else(|text| text, |names| render_templates(&names))
    }

    pub fn template_use(&self, text: &str) -> Option<(String, String)> {
        match_template(text, &self.template_names().ok()?)
    }

    pub fn release(
        &self,
        hash: &str,
        stopped_at: Option<usize>,
    ) -> Result<Option<RunFile>, String> {
        match self
            .load()?
            .and_then(|file| stop_file(&file, hash, stopped_at))
        {
            Some(released) => self.save(&released).map(|()| Some(released)),
            None => Ok(None),
        }
    }

    pub fn log_notice(&self, notice: &str) {
        warn_if_err(
            Store::new(self.log.clone()).append(Role::User, &notice_record(notice)),
            "Failed to record the send-it notice",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_send_it::VerifyMode;
    use serde_json::json;

    const LIMITS: Limits = Limits {
        max_turns: 3,
        max_usd: 1.0,
        verify: VerifyMode::Off,
    };

    fn plan_args() -> Value {
        json!({
            "goal": "ship GUZ-1",
            "steps": [{"do": "run the checks", "criterion": {"kind": "check_passes", "command": "cargo xtask lint"}}]
        })
    }

    fn quiet(dir: &Path) -> SendIt {
        SendIt::new(dir, LIMITS, Arc::new(|_| {}))
    }

    #[test]
    fn propose_stores_the_plan_under_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = quiet(dir.path());
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
    fn propose_adds_the_warning_line_after_the_stored_line_for_a_worker_done_plan_without_reply() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = quiet(dir.path());
        let mut args = plan_args();
        args["steps"] =
            json!([{"do": "wait", "criterion": {"kind": "worker_done", "worker": "E1"}}]);
        let stored = send_it.propose(&args).unwrap();
        let lines: Vec<&str> = stored.lines().collect();
        assert!(lines[0].starts_with("Stored run plan "), "{stored}");
        assert_eq!(lines.len(), 2, "{stored}");
        assert!(
            lines[1].starts_with("Warning: step 1 waits for worker E1"),
            "{stored}"
        );
        let clean = quiet(tempfile::tempdir().unwrap().path());
        assert!(!clean.propose(&plan_args()).unwrap().contains("Warning:"));
    }

    #[test]
    fn propose_rejects_a_step_without_a_criterion_and_stores_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = quiet(dir.path());
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
        let send_it = quiet(dir.path());
        let plan = parse_run(&plan_args(), LIMITS).unwrap();
        let hash = plan_hash(&plan);
        let draft = RunFile {
            plan,
            approved: None,
            stopped: None,
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
