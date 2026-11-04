use super::{
    branch_exists, check, clear_marker, exclude, git_value, guard, locate_worktree, repo_root,
    resolve_base_branch, run, Workers,
};
use crate::chat::cell::detach_from_terminal;
use crate::chat::memory::usage_path;
use crate::chat::read_claude_config;
use crate::config::AppConfig;
use crate::cursor_owner::{now_secs, write_file};
use crate::outcome_log;
use crate::paths::absolute_from;
use crate::private_file::{check_path, write_private, write_worker_config};
use crate::subprocess::kill_process_group;
use crate::warn::warn_if_err;
use crate::worker_changes::WorkerChanges;
use ralph_core::claude_leftovers::configured_dir;
use ralph_core::snapshot::{snapshots_enabled, WorkerSite};
use ralph_core::worker::{
    branch_name, claude_worker_settings_with_config, close_clear_paths, encode_spawn_record,
    exclude_lines, fetch_env, fetch_origin_args, fetch_ssh_command, fetch_timeout_text,
    import_servers, opencode_config, project_slug, resolve_claude_model, resolve_claude_shell,
    resolve_codex_warning, resolve_cursor_model, resolve_opencode_model, resolve_ticket,
    rollback_plan, rollback_report, session_name, spawn_base_sha, spawn_event, spawn_handoff,
    spawn_record_path, startup_state, task_dir_path, task_file_path, trust_key, with_start_warning,
    worker_command, worker_config_path, worker_mcp_args, worker_mcp_config, worktree_add_args,
    wt_switch_args, ClaudeShell, ClaudeWorkerArgs, Handoff, HandoffFiles, HarnessId, OpencodeSpawn,
    Owner, Rollback, SpawnFacts, SpawnFootprint, SpawnRecord, SpawnRequest, Startup, Ticket,
    TicketKeys, UsageSink, WorkPhase, WorkerImport, WorkerMarkers, WorktreeBase, WorktreeVia,
    ATTACHMENTS_DIR, CLAUDE_COMMAND, CODEX_COMMAND, CURSOR_AGENT, FETCH_GRACE_SECS,
    FETCH_TIMEOUT_SECS, HANDOFF_POLL, ID_OPTION, INBOX_DIR, OPENCODE_CONFIG_FILE, STORE_OPTION,
    WORKERS_DIR,
};
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const STARTUP_POLL: Duration = Duration::from_millis(250);
const FETCH_POLL: Duration = Duration::from_millis(100);

struct ImportSource {
    config_dir: Option<PathBuf>,
    names: Vec<String>,
}

fn import_source<E>(harness: HarnessId, config: Result<&AppConfig, E>) -> Option<ImportSource> {
    if harness != HarnessId::Claude {
        return None;
    }
    let fallback;
    let config = match config {
        Ok(config) => config,
        Err(_) => {
            fallback = AppConfig::default();
            &fallback
        }
    };
    Some(ImportSource {
        config_dir: config.claude.config_dir.clone(),
        names: config.chat.mcp_servers.clone(),
    })
}

impl Workers {
    pub fn spawn(&self, args: &SpawnArgs) -> io::Result<String> {
        let started = self.start(args);
        self.mark_sent(args.id, started)
    }

    fn start(&self, args: &SpawnArgs) -> io::Result<String> {
        let SpawnArgs {
            id,
            task,
            harness,
            model,
            ticket,
            scheduled,
            phase,
        } = *args;
        let repo = repo_root()?;
        let project = project_slug(&repo);
        self.refuse_busy_headless(id, false)?;
        match self.owner(&project, id) {
            Owner::Missing => {}
            Owner::Mine(_) => {
                return Err(io::Error::other(format!("worker {id} is already running")))
            }
            Owner::Foreign(session) => {
                return Err(io::Error::other(format!(
                    "worker {id} is running for another chat (store {})",
                    session.store
                )))
            }
        }
        let config = AppConfig::load().map_err(|e| e.to_string());
        let (shell, shell_warning) = resolve_claude_shell(
            config
                .as_ref()
                .map(|config| config.claude.shell_setting())
                .map_err(String::as_str),
        );
        let shell_warning = shell_warning.filter(|_| harness == HarnessId::Claude);
        self.preflight(id, harness, shell).map_err(|e| {
            io::Error::new(
                e.kind(),
                with_start_warning(e.to_string(), shell_warning.as_deref()),
            )
        })?;
        let session = session_name(&project, id);
        let (worktree, worktree_dir) =
            locate_worktree(&repo, id, self.worktree_dir.as_deref(), config.as_ref())?;
        exclude(&repo, &exclude_lines(harness, &repo, &worktree_dir.path))?;
        let branch = branch_name(id);
        let created_worktree = !worktree.join(".git").exists();
        let created_branch = !branch_exists(&repo, &branch)?;
        let via = ensure_worktree(
            &repo,
            &worktree,
            &branch,
            WorktreeBase::scheduled(scheduled),
        )?;
        let branch_ref = format!("refs/heads/{branch}");
        let tip = git_value(&repo, &["rev-parse", "--verify", "--quiet", &branch_ref])
            .ok()
            .flatten();
        let merge_base = if created_branch {
            None
        } else {
            resolve_base_branch(&repo, &config).ok().and_then(|base| {
                git_value(&repo, &["merge-base", &base.branch, &branch_ref])
                    .ok()
                    .flatten()
            })
        };
        let created_at = spawn_base_sha(created_branch, tip.as_deref(), merge_base.as_deref());
        let site = snapshots_enabled(config.as_ref().ok().map(|config| config.changes.enabled))
            .then(|| self.site(id, &worktree));
        let (model, model_warning) = match harness {
            HarnessId::Opencode => {
                let (model, warning) = resolve_opencode_model(
                    model,
                    config
                        .as_ref()
                        .map(|config| config.opencode.model.as_deref())
                        .map_err(String::as_str),
                );
                (Some(model), warning)
            }
            HarnessId::Claude | HarnessId::ClaudeHeadless => resolve_claude_model(
                model,
                config
                    .as_ref()
                    .map(|config| config.claude.model.as_deref())
                    .map_err(String::as_str),
            ),
            HarnessId::Cursor => resolve_cursor_model(
                model,
                config
                    .as_ref()
                    .map(|config| config.cursor.model.as_deref())
                    .map_err(String::as_str),
            ),
            HarnessId::Codex => (
                model.map(str::to_string),
                resolve_codex_warning(
                    model,
                    config
                        .as_ref()
                        .map(|config| config.codex.model.as_deref())
                        .map_err(String::as_str),
                ),
            ),
        };
        let version_warning =
            crate::version_drift::spawn_warning(harness, &self.store_dir.join(INBOX_DIR));
        let ticket = resolve_ticket(
            id,
            ticket,
            config
                .as_ref()
                .map(|config| &config.tickets)
                .unwrap_or(&TicketKeys::default()),
        );
        let imported = match import_source(harness, config.as_ref()) {
            Some(source) => import_servers(
                &read_claude_config(source.config_dir.as_deref()),
                &source.names,
            ),
            None => WorkerImport {
                servers: Map::new(),
                warnings: Vec::new(),
            },
        };
        let mut warnings = vec![
            worktree_dir.warning.as_deref(),
            model_warning.as_deref(),
            shell_warning.as_deref(),
            version_warning.as_deref(),
        ];
        warnings.extend(
            imported
                .warnings
                .iter()
                .map(|warning| Some(warning.as_str())),
        );
        let config_dir = config.as_ref().ok().and_then(|config| {
            configured_dir(
                config.claude.config_dir.as_deref(),
                dirs::home_dir().as_deref(),
            )
        });
        let launch = Launch {
            id,
            task,
            harness,
            model: model.as_deref(),
            worktree: &worktree,
            scheduled,
            shell,
            config_dir,
            servers: imported.servers,
            site: site.as_ref(),
            session: &session,
            ticket,
            base_sha: created_at.clone(),
            phase,
            started: started_text(id, &worktree, &via, &warnings),
        };
        if let Some(changes) = site.as_ref().and_then(WorkerChanges::open) {
            changes.snapshot_turn_or_warn();
        }
        let launched = if scheduled {
            guard::apply(&repo, &worktree).and_then(|()| self.launch(&launch))
        } else {
            self.launch(&launch)
        };
        if launched.is_ok() {
            outcome_log::record(&self.store_dir, id, |key| {
                spawn_event(&SpawnFacts {
                    key,
                    ticket: launch.ticket.as_ref().map(|ticket| ticket.key.as_str()),
                    tracker: launch.ticket.as_ref().and_then(|ticket| ticket.tracker),
                    harness: launch.harness,
                    model: launch.model,
                    base_sha: launch.base_sha.as_deref(),
                    scheduled: launch.scheduled,
                    phase: launch.phase,
                })
            });
        }
        launched.map_err(|error| {
            warn_if_err(
                self.clear_changes(id, &worktree),
                "Failed to clear worker snapshots",
            );
            warn_if_err(
                clear_marker(&spawn_record_path(&self.store_dir, id)),
                "Failed to clear worker spawn record",
            );
            warn_if_err(
                clear_marker(&task_dir_path(&self.store_dir, id)),
                "Failed to clear worker task dir",
            );
            warn_if_err(
                clear_marker(&worker_config_path(&self.store_dir, id)),
                "Failed to clear worker MCP config",
            );
            let footprint = SpawnFootprint {
                created_worktree,
                created_branch,
                harness_running: self.harness_running(&launch),
                commits_past_base: created_at
                    .as_deref()
                    .and_then(|start| commits_past(&repo, start, &branch)),
            };
            let failures = roll_back(&repo, &launch, &rollback_plan(&footprint));
            io::Error::other(rollback_report(&error.to_string(), &failures))
        })
    }

    fn preflight(&self, id: &str, harness: HarnessId, shell: ClaudeShell) -> io::Result<()> {
        match harness {
            HarnessId::Claude => {
                require_on_path("tmux")?;
                require_on_path(shell.program())?;
                let mut check = Command::new(shell.program());
                check
                    .args([shell.login_flags(), "command -v claude"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                detach_from_terminal(&mut check);
                let found = check.status()?.success();
                if found {
                    Ok(())
                } else {
                    Err(io::Error::other(format!(
                        "claude not found on PATH (checked in a {} login shell)",
                        shell.program()
                    )))
                }
            }
            HarnessId::Opencode => self.preflight_opencode(id),
            HarnessId::Cursor => require_on_path(CURSOR_AGENT.command),
            HarnessId::Codex => require_on_path(CODEX_COMMAND),
            HarnessId::ClaudeHeadless => require_on_path(CLAUDE_COMMAND),
        }
    }

    fn launch(&self, launch: &Launch) -> io::Result<String> {
        let Launch {
            id,
            task,
            harness,
            model,
            scheduled,
            shell,
            config_dir,
            servers,
            worktree,
            site,
            session,
            ticket,
            base_sha,
            started,
            ..
        } = launch;
        let inbox = self.store_dir.join(INBOX_DIR);
        let attachments = self.store_dir.join(ATTACHMENTS_DIR);
        fs::create_dir_all(&attachments)?;
        let markers = WorkerMarkers::new(&self.store_dir, id);
        for marker in close_clear_paths(&self.store_dir, id) {
            clear_marker(&marker)?;
        }
        self.write_spawn_record(id, *harness, ticket.clone(), base_sha.clone())?;
        let command = match harness {
            HarnessId::Claude => {
                let settings = self.write_settings(id, &markers, *site)?;
                let task_dir = self.write_task(id, task)?;
                let copied: Vec<String> = servers.keys().cloned().collect();
                worker_command(&ClaudeWorkerArgs {
                    store: &fs::canonicalize(&self.store_dir)?,
                    mcp_config: &self.write_config(id, *site, servers)?,
                    settings: &settings,
                    permission_mode: &self.permission_mode,
                    attachments: &attachments,
                    task_dir: &task_dir,
                    model: *model,
                    scheduled: *scheduled,
                    shell: *shell,
                    config_dir: config_dir.as_deref(),
                    copied: &copied,
                })
            }
            HarnessId::Opencode => {
                let model =
                    model.ok_or_else(|| io::Error::other("opencode model was not resolved"))?;
                let worktree_arg = site.map(|site| site.worktree.as_path());
                let mcp_args = worker_mcp_args(id, &inbox, worktree_arg);
                let config = opencode_config(model, &self.exe, &mcp_args, &attachments);
                fs::write(worktree.join(OPENCODE_CONFIG_FILE), config.to_string())?;
                self.spawn_opencode(launch, model)?;
                return Ok(format!(
                    "{started} It runs headless on the shared opencode server inside ralph chat (no tmux session)."
                ));
            }
            HarnessId::Cursor => {
                let warning = self.spawn_headless(
                    launch,
                    HandoffFiles::cursor(&self.store_dir, id),
                    "cursor",
                )?;
                return Ok(with_start_warning(
                    format!("{started} It runs headless inside ralph chat (no tmux session)."),
                    warning.as_deref(),
                ));
            }
            HarnessId::Codex => {
                let warning =
                    self.spawn_headless(launch, HandoffFiles::codex(&self.store_dir, id), "codex")?;
                return Ok(with_start_warning(
                    format!("{started} It runs headless inside ralph chat (no tmux session)."),
                    warning.as_deref(),
                ));
            }
            HarnessId::ClaudeHeadless => {
                let files = HandoffFiles::claude_headless(&self.store_dir, id);
                let warning = self.spawn_headless(launch, files, "claude-headless")?;
                return Ok(with_start_warning(
                    format!("{started} It runs headless inside ralph chat (no tmux session)."),
                    warning.as_deref(),
                ));
            }
        };
        run(Command::new("tmux")
            .args(["new-session", "-d", "-s", session])
            .arg("-c")
            .arg(worktree)
            .args(command))?;
        let target = format!("={session}:");
        run(Command::new("tmux")
            .args(["set-option", "-t", &target, STORE_OPTION])
            .arg(&self.store_dir))?;
        run(Command::new("tmux").args(["set-option", "-t", &target, ID_OPTION, id]))?;
        let confirmed = await_startup(session, &markers.ready)?;
        Ok(if confirmed {
            format!("{started} Watch it with: tmux attach -t {session}")
        } else {
            format!(
                "{started} Its ready signal is not confirmed yet after {}s; check it with: tmux attach -t {session}",
                STARTUP_TIMEOUT.as_secs()
            )
        })
    }

    fn write_spawn_record(
        &self,
        id: &str,
        harness: HarnessId,
        ticket: Option<Ticket>,
        base_sha: Option<String>,
    ) -> io::Result<()> {
        let path = spawn_record_path(&self.store_dir, id);
        let dir = path.parent().unwrap_or(&self.store_dir);
        fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{id}.{}.tmp", std::process::id()));
        let record = SpawnRecord::new(ticket, base_sha, harness, now_secs());
        fs::write(&tmp, encode_spawn_record(&record)?)?;
        fs::rename(tmp, path)
    }

    fn harness_running(&self, launch: &Launch) -> bool {
        match launch.harness {
            HarnessId::Claude => session_exists(launch.session).unwrap_or(true),
            HarnessId::Opencode => HandoffFiles::opencode(&self.store_dir, launch.id)
                .owner
                .exists(),
            HarnessId::Cursor => HandoffFiles::cursor(&self.store_dir, launch.id)
                .owner
                .exists(),
            HarnessId::Codex => HandoffFiles::codex(&self.store_dir, launch.id)
                .owner
                .exists(),
            HarnessId::ClaudeHeadless => HandoffFiles::claude_headless(&self.store_dir, launch.id)
                .owner
                .exists(),
        }
    }

    fn spawn_headless(
        &self,
        launch: &Launch,
        files: HandoffFiles,
        harness: &str,
    ) -> io::Result<Option<String>> {
        let id = launch.id;
        for path in files.all() {
            clear_marker(path)?;
        }
        let request = SpawnRequest {
            worktree: launch.worktree.to_path_buf(),
            exe: self.exe.clone(),
            inbox: self.store_dir.join(INBOX_DIR),
            task: launch.task.to_string(),
            at: now_secs(),
            snapshots: launch.site.is_some(),
            model: launch.model.map(str::to_string),
        };
        write_file(
            &files.spawn,
            &serde_json::to_string(&request).map_err(io::Error::other)?,
        )?;
        let ready = WorkerMarkers::new(&self.store_dir, id).ready;
        self.await_spawn(&files.spawn, &ready, &files.error, harness)?;
        let warning = fs::read_to_string(&files.warning).ok();
        clear_marker(&files.warning)?;
        Ok(warning)
    }

    fn spawn_opencode(&self, launch: &Launch, model: &str) -> io::Result<()> {
        let id = launch.id;
        let files = HandoffFiles::opencode(&self.store_dir, id);
        let preflight = HandoffFiles::opencode_preflight(&self.store_dir, id);
        let healthy = HandoffFiles::opencode_healthy(&self.store_dir, id);
        for path in files.all().into_iter().chain([&*preflight, &*healthy]) {
            clear_marker(path)?;
        }
        let request = OpencodeSpawn {
            worktree: launch.worktree.to_path_buf(),
            inbox: self.store_dir.join(INBOX_DIR),
            model: model.to_string(),
            task: launch.task.to_string(),
            at: now_secs(),
            snapshots: launch.site.is_some(),
        };
        write_file(&files.spawn, &request.render())?;
        let ready = WorkerMarkers::new(&self.store_dir, id).ready;
        self.await_spawn(&files.spawn, &ready, &files.error, "opencode")
    }

    fn preflight_opencode(&self, id: &str) -> io::Result<()> {
        let files = HandoffFiles::opencode(&self.store_dir, id);
        let preflight = HandoffFiles::opencode_preflight(&self.store_dir, id);
        let healthy = HandoffFiles::opencode_healthy(&self.store_dir, id);
        for path in [&preflight, &healthy, &files.error] {
            clear_marker(path)?;
        }
        write_file(&preflight, &now_secs().to_string())?;
        self.await_spawn(&preflight, &healthy, &files.error, "opencode")
            .and_then(|()| clear_marker(&healthy))
    }

    fn await_spawn(
        &self,
        spawn: &Path,
        ready: &Path,
        error: &Path,
        harness: &str,
    ) -> io::Result<()> {
        let started = Instant::now();
        loop {
            let failure = fs::read_to_string(error).ok();
            match spawn_handoff(ready.exists(), failure.as_deref(), started.elapsed()) {
                Handoff::Done => return Ok(()),
                Handoff::Failed(message) => {
                    clear_marker(error)?;
                    return Err(io::Error::other(message));
                }
                Handoff::TimedOut => {
                    clear_marker(spawn)?;
                    return Err(io::Error::other(format!(
                        "no ralph chat picked up the {harness} worker in time; {harness} workers start only from a running `ralph chat` with this store"
                    )));
                }
                Handoff::Pending => thread::sleep(HANDOFF_POLL),
            }
        }
    }

    pub(super) fn clear_changes(&self, id: &str, worktree: &Path) -> io::Result<()> {
        match WorkerChanges::open(&self.site(id, worktree)) {
            Some(changes) => changes.clear(),
            None => Ok(()),
        }
    }

    pub(super) fn site(&self, id: &str, worktree: &Path) -> WorkerSite {
        WorkerSite {
            store: self.store_dir.clone(),
            id: id.to_string(),
            worktree: worktree.to_path_buf(),
        }
    }

    fn write_settings(
        &self,
        id: &str,
        markers: &WorkerMarkers,
        site: Option<&WorkerSite>,
    ) -> io::Result<PathBuf> {
        let dir = self.store_dir.join(WORKERS_DIR);
        fs::create_dir_all(&dir)?;
        let usage = absolute_from(&usage_path(), &std::env::current_dir()?);
        let path = dir.join(format!("{id}.settings.json"));
        fs::write(
            &path,
            claude_worker_settings_with_config(
                &self.exe,
                markers,
                site,
                &UsageSink {
                    store: &self.store_dir,
                    worker: id,
                    usage: &usage,
                },
                Some(&self.written_store),
            )
            .to_string(),
        )?;
        Ok(path)
    }

    fn write_task(&self, id: &str, task: &str) -> io::Result<PathBuf> {
        let dir = task_dir_path(&self.store_dir, id);
        if let Some(parent) = dir.parent() {
            fs::create_dir_all(parent)?;
        }
        check_path(&dir)?;
        match fs::DirBuilder::new().mode(0o700).create(&dir) {
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
            _ => {}
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        write_private(task_file_path(&dir), task.as_bytes())?;
        Ok(dir)
    }

    fn write_config(
        &self,
        id: &str,
        site: Option<&WorkerSite>,
        servers: &Map<String, Value>,
    ) -> io::Result<PathBuf> {
        let own = worker_mcp_config(
            &self.exe,
            id,
            &self.store_dir.join(INBOX_DIR),
            site.map(|site| site.worktree.as_path()),
        );
        write_worker_config(&self.store_dir, id, servers, &own)
    }
}

struct Launch<'a> {
    id: &'a str,
    task: &'a str,
    harness: HarnessId,
    model: Option<&'a str>,
    scheduled: bool,
    shell: ClaudeShell,
    config_dir: Option<PathBuf>,
    servers: Map<String, Value>,
    worktree: &'a Path,
    site: Option<&'a WorkerSite>,
    session: &'a str,
    ticket: Option<Ticket>,
    base_sha: Option<String>,
    phase: Option<WorkPhase>,
    started: String,
}

#[derive(Clone, Copy)]
pub(crate) struct SpawnArgs<'a> {
    pub id: &'a str,
    pub task: &'a str,
    pub harness: HarnessId,
    pub model: Option<&'a str>,
    pub ticket: Option<&'a str>,
    pub scheduled: bool,
    pub phase: Option<WorkPhase>,
}

fn require_on_path(program: &str) -> io::Result<()> {
    let found = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()));
    if found {
        Ok(())
    } else {
        Err(io::Error::other(format!("{program} not found on PATH")))
    }
}

fn commits_past(repo: &Path, start: &str, branch: &str) -> Option<u64> {
    git_value(
        repo,
        &["rev-list", "--count", &format!("{start}..{branch}")],
    )
    .ok()
    .flatten()
    .and_then(|count| count.parse().ok())
}

fn roll_back(repo: &Path, launch: &Launch, plan: &Rollback) -> Vec<String> {
    let mut failures = Vec::new();
    if plan.remove_worktree {
        let removed = run(Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["worktree", "remove"])
            .arg(launch.worktree));
        if let Err(e) = removed {
            failures.push(format!(
                "could not remove {}: {e}",
                launch.worktree.display()
            ));
            return failures;
        }
    }
    if plan.delete_branch {
        let branch = branch_name(launch.id);
        if let Err(e) = run(Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["branch", "-D", &branch]))
        {
            failures.push(format!("could not delete branch {branch}: {e}"));
        }
    }
    failures
}

fn started_text(id: &str, worktree: &Path, via: &WorktreeVia, warnings: &[Option<&str>]) -> String {
    let started = format!(
        "Worker {id} started in {} on branch {}. {}",
        worktree.display(),
        branch_name(id),
        via.note()
    );
    warnings
        .iter()
        .flatten()
        .fold(started, |text, warning| format!("{text} {warning}"))
}

pub(super) fn await_startup(session: &str, ready: &Path) -> io::Result<bool> {
    let target = format!("={session}:");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut previous: Option<String> = None;
    let mut state = Startup::Starting;
    while Instant::now() < deadline {
        if !session_exists(session)? {
            return Err(io::Error::other(format!(
                "worker exited during startup; its tmux session {session} is gone"
            )));
        }
        let pane = run(Command::new("tmux").args(["capture-pane", "-p", "-t", &target]))?;
        let text = String::from_utf8_lossy(&pane.stdout).into_owned();
        state = startup_state(&text, ready.exists());
        if state == Startup::Ready {
            return Ok(true);
        }
        if let Some(key) = trust_key(&text, previous.as_deref()) {
            run(Command::new("tmux").args(["send-keys", "-t", &target, key]))?;
        }
        previous = Some(text);
        thread::sleep(STARTUP_POLL);
    }
    if state == Startup::TrustPrompt {
        return Err(io::Error::other(format!(
            "worker is stuck on the folder-trust prompt; answer it with: tmux attach -t {session}"
        )));
    }
    Ok(false)
}

pub(super) fn session_exists(session: &str) -> io::Result<bool> {
    Ok(Command::new("tmux")
        .args(["has-session", "-t", &format!("={session}")])
        .stderr(Stdio::null())
        .status()?
        .success())
}

fn fetch_origin(repo: &Path) -> io::Result<()> {
    let _setup = guard::lock_setup();
    let ssh_command = fetch_ssh_command(
        std::env::var("GIT_SSH_COMMAND").ok().as_deref(),
        git_value(repo, &["config", "--get", "core.sshCommand"])?.as_deref(),
    );
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(fetch_origin_args())
        .envs(fetch_env(&ssh_command))
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let (sender, receiver) = mpsc::channel();
    if let Some(mut pipe) = child.stderr.take() {
        thread::spawn(move || {
            let mut text = String::new();
            let _ = io::Read::read_to_string(&mut pipe, &mut text);
            let _ = sender.send(text);
        });
    }
    let deadline = Instant::now() + Duration::from_secs(FETCH_TIMEOUT_SECS);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            kill_process_group(&mut child);
            break None;
        }
        thread::sleep(FETCH_POLL);
    };
    let text = receiver
        .recv_timeout(Duration::from_secs(FETCH_GRACE_SECS))
        .unwrap_or_default();
    match status {
        Some(status) if status.success() => Ok(()),
        Some(_) => Err(io::Error::other(text.trim().to_string())),
        None => Err(io::Error::other(fetch_timeout_text())),
    }
}

fn ensure_worktree(
    repo: &Path,
    worktree: &Path,
    branch: &str,
    base: WorktreeBase,
) -> io::Result<WorktreeVia> {
    if worktree.join(".git").exists() {
        return Ok(WorktreeVia::Existing);
    }
    if base == WorktreeBase::OriginMain {
        fetch_origin(repo)?;
    }
    let wt_error = match Command::new("wt")
        .args(wt_switch_args(
            repo,
            worktree,
            branch,
            branch_exists(repo, branch)?,
            base,
        ))
        .stdin(Stdio::null())
        .output()
    {
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        output => match output.and_then(check) {
            _ if worktree.join(".git").exists() => return Ok(WorktreeVia::Wt),
            Ok(_) => Some(format!("it did not create {}", worktree.display())),
            Err(e) => Some(e.to_string()),
        },
    };
    run(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(worktree_add_args(
            worktree,
            branch,
            branch_exists(repo, branch)?,
            base,
        )))?;
    Ok(WorktreeVia::Git { wt_error })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_config_load_falls_back_to_the_default_allow_list() {
        let source = import_source(HarnessId::Claude, Err("bad toml")).unwrap();
        assert_eq!(source.names, ["mcptools"]);
        assert_eq!(source.config_dir, None);
    }

    #[test]
    fn a_loaded_config_supplies_its_allow_list_and_config_dir() {
        let config = AppConfig::parse_reporting_unknown(
            "[chat]\nmcp_servers = [\"docs\"]\n[claude]\nconfig_dir = \"/alt\"\n",
            "c",
        )
        .unwrap()
        .0;
        let source = import_source(HarnessId::Claude, Ok::<_, String>(&config)).unwrap();
        assert_eq!(source.names, ["docs"]);
        assert_eq!(source.config_dir, Some(PathBuf::from("/alt")));
    }

    #[test]
    fn settings_pass_the_written_store_path_as_config_store() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let workers = Workers::new(link.clone(), "/bin/ralph".into(), "auto".into(), None);
        let markers = WorkerMarkers::new(&workers.store_dir, "A");
        let site = WorkerSite {
            store: workers.store_dir.clone(),
            id: "A".into(),
            worktree: dir.path().join("w"),
        };
        let path = workers.write_settings("A", &markers, Some(&site)).unwrap();
        let settings: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let command = settings["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(command.contains(&format!("--config-store '{}'", link.display())));
    }

    #[test]
    fn other_harnesses_import_nothing() {
        let config = AppConfig::new();
        for harness in [
            HarnessId::ClaudeHeadless,
            HarnessId::Codex,
            HarnessId::Cursor,
        ] {
            assert!(import_source(harness, Ok::<_, String>(&config)).is_none());
        }
        assert!(import_source(HarnessId::Opencode, Err("x")).is_none());
    }
}
