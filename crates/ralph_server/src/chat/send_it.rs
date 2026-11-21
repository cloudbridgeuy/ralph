use crate::chat::ask::Ask;
use crate::chat::memory::Store;
use crate::chat::session::{Emit, Event};
use crate::warn::warn_if_err;
use crate::watches::{locked, OutcomeSlot};
use crate::wire::store::current_uid;
use ralph_core::chat_local::notice_record;
use ralph_core::chat_memory::Role;
use ralph_core::chat_send_it::template::{
    fill_template, match_template, render_templates, save_notice, template_name, template_text,
};
use ralph_core::chat_send_it::{
    approve, parse_run, plan_hash, plan_warnings, propose_refusal, read_run, render_status,
    render_status_styled, settle_check, start_run, stop_file, tick, with_reply, write_run, Action,
    Driving, Ended, Limits, RunFile, Seen, Signing, StepResult, Ticked, Trust,
};
use ralph_core::watch::WatchOutcome;
use ralph_core::worker::{hex_encode, Report};
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const NO_RUN: &str = "No send-it run. Start one with /send-it <instructions>.";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 16;
const RUN_DIR: &str = "send-it";
const RUN_FILE: &str = "run.toml";
const TEMPLATE_DIR: &str = "templates";

fn random_bytes<const N: usize>() -> Result<[u8; N], getrandom::Error> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes)?;
    Ok(bytes)
}

fn unreadable(path: &Path, error: io::Error) -> String {
    format!("Could not read {}: {error}", path.display())
}

#[derive(Debug, Clone)]
pub struct SendIt {
    log: PathBuf,
    path: PathBuf,
    templates: PathBuf,
    limits: Limits,
    key: Arc<[u8; KEY_BYTES]>,
    live: Arc<Mutex<Option<String>>>,
    refused: Arc<Mutex<Option<String>>>,
    judged: Arc<Mutex<Option<StepResult>>>,
    answered: Arc<Mutex<Vec<String>>>,
    out: Out,
}

#[derive(Clone)]
struct Out(Emit);

impl fmt::Debug for Out {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Out")
    }
}

impl SendIt {
    pub fn new(store_dir: &Path, limits: Limits, emit: Emit) -> Result<Self, getrandom::Error> {
        Ok(Self {
            key: Arc::new(random_bytes()?),
            live: Arc::default(),
            log: store_dir.to_path_buf(),
            path: store_dir.join(RUN_DIR).join(RUN_FILE),
            templates: store_dir.join(RUN_DIR).join(TEMPLATE_DIR),
            limits,
            refused: Arc::default(),
            judged: Arc::default(),
            answered: Arc::default(),
            out: Out(emit),
        })
    }

    pub fn show(&self, line: impl fmt::Display) {
        (self.out.0)(Event::Notice(line.to_string()));
    }

    pub fn blocked(&self, notice: &str) {
        (self.out.0)(Event::Blocked(notice.to_string()));
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

    fn gate<T>(&self, check: impl FnOnce(Trust) -> T) -> T {
        check(Trust {
            key: self.key.as_slice(),
        })
    }

    pub fn allowed(&self) -> Option<Vec<Action>> {
        let file = self.load().ok().flatten()?;
        self.gate(|trust| file.is_approved(trust))
            .then_some(file.plan.allowed)
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
        let existing = self.load().ok().flatten();
        self.gate(|trust| propose_refusal(existing.as_ref(), trust))
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

    pub fn status(&self, width: usize) -> Screen {
        match self.load() {
            Ok(Some(file)) => self.gate(|trust| Screen {
                text: render_status(&file, width, trust),
                styled: render_status_styled(&file, width, trust),
                shown: Some(plan_hash(&file.plan)),
            }),
            Ok(None) => Screen::text(NO_RUN),
            Err(text) => Screen::text(text),
        }
    }

    pub fn proposed(&self, before: Option<String>, width: usize) -> Screen {
        let screen = self.status(width);
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

    pub fn go(
        &self,
        shown: Option<&str>,
        typed_at_prompt: bool,
        seen: &Seen,
    ) -> (String, Option<Ticked>) {
        let file = match self.load() {
            Ok(Some(file)) => file,
            Ok(None) => return (NO_RUN.to_string(), None),
            Err(text) => return (text, None),
        };
        locked(&self.answered).clear();
        let approved = self.signed_approval(&file, shown, typed_at_prompt);
        match approved {
            Ok(approved) => (
                format!("Plan {} approved. Running it.", plan_hash(&approved.plan)),
                self.gate(|trust| start_run(&approved, seen, trust)),
            ),
            Err(text) => (text, None),
        }
    }

    fn signed_approval(
        &self,
        file: &RunFile,
        shown: Option<&str>,
        typed_at_prompt: bool,
    ) -> Result<RunFile, String> {
        let nonce = hex_encode(&random_bytes::<NONCE_BYTES>().map_err(|error| error.to_string())?);
        let at = chrono::Utc::now().to_rfc3339();
        let signing = Signing {
            key: self.key.as_slice(),
            at: &at,
            peer_uid: current_uid(),
            nonce: &nonce,
        };
        let approved = approve(file, shown, typed_at_prompt, &signing)?;
        self.save(&approved)?;
        *locked(&self.live) = Some(nonce);
        Ok(approved)
    }

    pub fn settle(&self, driving: &Driving, outcome: &WatchOutcome, reports: &[Report]) -> Ticked {
        let file = self.load().ok().flatten();
        self.gate(|trust| settle_check(driving, file.as_ref(), outcome, reports, trust))
    }

    pub fn tick(&self, driving: &Driving, watched: Option<&OutcomeSlot>, ended: Ended) -> Ticked {
        let file = self.load().ok().flatten();
        let watched = watched.and_then(|slot| locked(slot).clone());
        let refused = locked(&self.refused).take();
        let judged = locked(&self.judged).take();
        let answered = std::mem::take(&mut *locked(&self.answered));
        let reply = ended.replied.then(|| self.last_reply()).flatten();
        let ticked = self.gate(|trust| {
            tick(
                driving,
                &Ended {
                    file: file.as_ref(),
                    trust,
                    watched: watched.as_ref(),
                    refused: refused.as_deref(),
                    answered: &answered,
                    step_result: judged.as_ref(),
                    ..ended
                },
            )
        });
        match reply {
            Some(reply) => with_reply(ticked, &reply),
            None => ticked,
        }
    }

    fn last_reply(&self) -> Option<String> {
        let (entries, _) = Store::new(self.log.clone()).load().ok()?;
        entries
            .into_iter()
            .rev()
            .find(|entry| entry.role == Role::Assistant)
            .map(|entry| entry.text)
    }

    pub fn edit(&self, ask: &mut dyn Ask, width: usize) -> Screen {
        if !self.path.exists() {
            return Screen::text(NO_RUN);
        }
        let opened = ask.edit(&self.path);
        let screen = self.clear_approval(width);
        match opened {
            Ok(()) => screen,
            Err(error) => Screen {
                text: format!("Editor failed: {error}\n{}", screen.text),
                styled: format!("Editor failed: {error}\n{}", screen.styled),
                ..screen
            },
        }
    }

    pub fn use_template(&self, ticket: &str, name: &str, width: usize) -> Screen {
        if let Some(refusal) = self.refusal() {
            return Screen::text(refusal);
        }
        let path = self.templates.join(format!("{name}.toml"));
        let stored = fs::read_to_string(&path)
            .map_err(|error| unreadable(&path, error))
            .and_then(|text| {
                fill_template(&text, ticket, self.limits)
                    .map_err(|error| format!("Template {name}: {error}"))
            })
            .and_then(|file| self.save(&file));
        match stored {
            Ok(()) => self.status(width),
            Err(text) => Screen::text(text),
        }
    }

    fn clear_approval(&self, width: usize) -> Screen {
        match self.load() {
            Ok(Some(file)) => match self.save(&RunFile {
                approved: None,
                ..file
            }) {
                Ok(()) => self.status(width),
                Err(text) => Screen::text(text),
            },
            Ok(None) => Screen::text(NO_RUN),
            Err(text) => Screen::text(format!("{text}\nFix it with /send-it edit.")),
        }
    }
}

pub const NOT_PROPOSED: &str =
    "No new run plan was stored. Try /send-it again with clearer instructions.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub text: String,
    pub styled: String,
    pub shown: Option<String>,
}

impl Screen {
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            styled: text.clone(),
            text,
            shown: None,
        }
    }

    pub fn for_terminal(&self, color: bool) -> &str {
        match color {
            true => &self.styled,
            false => &self.text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ask::Ask;
    use ralph_core::chat_send_it::Seen;
    use ralph_core::chat_send_it::VerifyMode;
    use serde_json::json;
    use std::collections::HashMap;

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
        SendIt::new(dir, LIMITS, Arc::new(|_| {})).unwrap()
    }

    fn recording(dir: &Path) -> (SendIt, Arc<Mutex<Vec<Event>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let emit: Emit = Arc::new(move |event| locked(&sink).push(event));
        (SendIt::new(dir, LIMITS, emit).unwrap(), events)
    }

    #[test]
    fn show_emits_a_notice_event() {
        let dir = tempfile::tempdir().unwrap();
        let (send_it, events) = recording(dir.path());
        send_it.show("Running step 1.");
        assert_eq!(
            *locked(&events),
            vec![Event::Notice("Running step 1.".to_string())]
        );
    }

    #[test]
    fn blocked_emits_a_blocked_event_and_no_notice() {
        let dir = tempfile::tempdir().unwrap();
        let (send_it, events) = recording(dir.path());
        send_it.blocked("send-it stopped at step 2");
        assert_eq!(
            *locked(&events),
            vec![Event::Blocked("send-it stopped at step 2".to_string())]
        );
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
        let hash = plan_hash(&parse_run(&plan_args(), LIMITS).unwrap());
        send_it.propose(&plan_args()).unwrap();
        assert!(send_it.propose(&plan_args()).is_ok());
        send_it.go(
            Some(&hash),
            true,
            &Seen {
                last: &HashMap::new(),
                running: &[],
            },
        );
        assert_eq!(
            send_it.propose(&plan_args()),
            Err(format!(
                "Plan {hash} is approved and active. Stop it with /send-it off first."
            ))
        );
    }

    struct Editor(Option<&'static str>);

    impl Ask for Editor {
        fn answer(&mut self, _prompt: &str) -> Option<String> {
            None
        }

        fn edit(&mut self, _path: &Path) -> io::Result<()> {
            self.0.map_or(Ok(()), |error| Err(io::Error::other(error)))
        }
    }

    #[test]
    fn edit_clears_approval_and_reports_editor_error() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = quiet(dir.path());
        send_it.propose(&plan_args()).unwrap();
        let hash = send_it.status(80).shown.unwrap();
        let seen = Seen {
            last: &HashMap::new(),
            running: &[],
        };
        send_it.go(Some(&hash), true, &seen);
        let failed = send_it.edit(&mut Editor(Some("no editor")), 80);
        assert!(
            failed.text.starts_with("Editor failed: no editor\n"),
            "{}",
            failed.text
        );
        assert!(failed.text.contains("not approved"), "{}", failed.text);
        let fine = send_it.edit(&mut Editor(None), 80);
        assert!(fine.text.contains("not approved"), "{}", fine.text);
    }

    #[test]
    fn clearing_after_an_edit_drops_the_approval_and_shows_the_plan() {
        let dir = tempfile::tempdir().unwrap();
        let send_it = quiet(dir.path());
        send_it.propose(&plan_args()).unwrap();
        let hash = send_it.status(80).shown.unwrap();
        send_it.go(
            Some(&hash),
            true,
            &Seen {
                last: &HashMap::new(),
                running: &[],
            },
        );
        assert_eq!(
            quiet(tempfile::tempdir().unwrap().path()).clear_approval(80),
            Screen::text(NO_RUN)
        );
        let screen = send_it.clear_approval(80);
        assert_eq!(screen.shown, Some(hash.clone()));
        assert!(screen
            .text
            .ends_with(&format!("{hash} · not approved · /send-it go to approve")));
        fs::write(&send_it.path, "goal = ").unwrap();
        let broken = send_it.clear_approval(80);
        assert_eq!(broken.shown, None);
        assert!(
            broken.text.ends_with("Fix it with /send-it edit."),
            "{}",
            broken.text
        );
    }
}
