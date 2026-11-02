use crate::prelude::*;
use error::{ReleaseError, Result};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

mod error;
mod notes;
mod plan;

#[derive(Debug, clap::Parser)]
#[command(
    long_about = "Cut a release: generate release notes, bump the shared workspace version, refresh Cargo.lock, update CHANGELOG.md, commit, and create an annotated vX.Y.Z tag.

Nothing is pushed. The command prints the push command to run when you are ready."
)]
pub struct ReleaseCommand {
    /// New version in strict x.y.z form
    pub version: String,

    /// Print the generated release notes and stop without editing, writing, committing, or tagging
    #[arg(long)]
    pub dry_run: bool,

    /// Skip the Claude-written summary paragraph
    #[arg(long)]
    pub no_summary: bool,
}

const LINEAR_URL: &str = "https://api.linear.app/graphql";
const LINEAR_BATCH: usize = 50;
const SUMMARY_MODEL: &str = "claude-haiku-5-5";

async fn git(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    Ok(Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await?)
}

async fn git_stdout(root: &Path, args: &[&str]) -> Result<String> {
    let output = git(root, args).await?;
    if !output.status.success() {
        return Err(ReleaseError::CommandFailed(format!(
            "git {}",
            args.join(" ")
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn run_checked(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(ReleaseError::CommandFailed(format!(
            "{program} {}",
            args.join(" ")
        )))
    }
}

fn warn(message: &str) {
    anstream::eprintln!("{} {message}", p_y("warning:"));
}

fn post_linear(key: &str, body: &str) -> Result<String> {
    let text = ureq::post(LINEAR_URL)
        .header("Authorization", key)
        .content_type("application/json")
        .send(body)?
        .body_mut()
        .read_to_string()?;
    Ok(text)
}

async fn fetch_batch(key: &str, ids: &[u32]) -> Result<HashMap<u32, notes::Ticket>> {
    let body = serde_json::json!({ "query": notes::linear_query(ids) }).to_string();
    let key = key.to_string();
    let text = tokio::task::spawn_blocking(move || post_linear(&key, &body)).await??;
    notes::parse_linear(&text)
}

async fn fetch_tickets(ids: &[u32]) -> HashMap<u32, notes::Ticket> {
    let Some(key) = std::env::var("LINEAR_API_KEY")
        .ok()
        .filter(|key| !key.is_empty())
    else {
        warn("LINEAR_API_KEY is not set; deriving sections from commit types");
        return HashMap::new();
    };
    let mut tickets = HashMap::new();
    for batch in ids.chunks(LINEAR_BATCH) {
        match fetch_batch(&key, batch).await {
            Ok(found) => tickets.extend(found),
            Err(error) => warn(&format!(
                "Linear lookup failed ({error}); using commit types"
            )),
        }
    }
    tickets
}

async fn generate_summary(prompt: &str) -> Result<Option<String>> {
    let mut child = Command::new("claude")
        .args(["-p", "--model", SUMMARY_MODEL])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await?;
    }
    let output = child.wait_with_output().await?;
    if !output.status.success() {
        return Err(ReleaseError::CommandFailed("claude -p".to_string()));
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!text.is_empty()).then_some(text))
}

async fn summary_for(prompt: &str) -> Option<String> {
    match generate_summary(prompt).await {
        Ok(summary) => summary,
        Err(error) => {
            warn(&format!("Summary skipped ({error})"));
            None
        }
    }
}

async fn edit_notes(draft: &str) -> Result<String> {
    let file = tempfile::Builder::new().suffix(".md").tempfile()?;
    tokio::fs::write(file.path(), draft).await?;
    let (argv, _) = ralph_core::chat_draft::editor_argv(
        None,
        std::env::var("VISUAL").ok().as_deref(),
        std::env::var("EDITOR").ok().as_deref(),
    );
    let (program, args) = argv.split_first().ok_or(ReleaseError::EmptyNotes)?;
    let status = Command::new(program)
        .args(args)
        .arg(file.path())
        .status()
        .await?;
    if !status.success() {
        return Err(ReleaseError::CommandFailed(argv.join(" ")));
    }
    let edited = tokio::fs::read_to_string(file.path()).await?;
    if edited.trim().is_empty() {
        return Err(ReleaseError::EmptyNotes);
    }
    Ok(edited)
}

async fn read_changelog(path: &Path) -> Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn draft_section(root: &Path, tag: &str, no_summary: bool) -> Result<String> {
    let previous = git_stdout(root, &["describe", "--tags", "--match", "v*", "--abbrev=0"]).await?;
    let range = format!("{previous}..HEAD");
    let log = git_stdout(
        root,
        &["log", &range, "--no-merges", "--format=%H%x1f%s%x1f%b%x1e"],
    )
    .await?;
    let commits = notes::parse_log(&log);
    let ids: Vec<u32> = commits
        .iter()
        .flat_map(|commit| commit.ids.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let tickets = fetch_tickets(&ids).await;
    let grouped = notes::build_notes(&commits, &tickets);
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let summary = if no_summary {
        None
    } else {
        let list = notes::render_section(tag, &date, None, &grouped);
        summary_for(&notes::summary_prompt(&list)).await
    };
    Ok(notes::render_section(
        tag,
        &date,
        summary.as_deref(),
        &grouped,
    ))
}

pub async fn run(command: ReleaseCommand, global: crate::Global) -> Result<()> {
    let new = plan::parse_release_version(&command.version)?;
    let root = PathBuf::from(git_stdout(Path::new("."), &["rev-parse", "--show-toplevel"]).await?);
    let manifest_path = root.join("Cargo.toml");
    let manifest = tokio::fs::read_to_string(&manifest_path).await?;

    plan::ensure_newer(plan::read_workspace_version(&manifest)?, new)?;
    let rewritten = plan::set_workspace_version(&manifest, new)?;
    let tag = plan::tag_name(new);

    if !command.dry_run
        && !git_stdout(&root, &["status", "--porcelain"])
            .await?
            .is_empty()
    {
        return Err(ReleaseError::DirtyWorkingTree);
    }
    let tag_ref = format!("refs/tags/{tag}");
    if git(&root, &["rev-parse", "-q", "--verify", &tag_ref])
        .await?
        .status
        .success()
    {
        return Err(ReleaseError::TagExists(tag));
    }

    let draft = draft_section(&root, &tag, command.no_summary).await?;
    if command.dry_run {
        aprintln!("{draft}");
        return Ok(());
    }
    let section = edit_notes(&draft).await?;
    let changelog_path = root.join("CHANGELOG.md");
    let changelog =
        notes::prepend_changelog(read_changelog(&changelog_path).await?.as_deref(), &section);

    tokio::fs::write(&manifest_path, rewritten).await?;
    run_checked(&root, "cargo", &["update", "--workspace"]).await?;
    tokio::fs::write(&changelog_path, changelog).await?;
    run_checked(
        &root,
        "git",
        &["add", "Cargo.toml", "Cargo.lock", "CHANGELOG.md"],
    )
    .await?;
    run_checked(&root, "git", &["commit", "-m", &plan::commit_message(new)]).await?;
    run_checked(&root, "git", &["tag", "-a", &tag, "-m", &tag]).await?;

    if !global.is_silent() {
        let branch = git_stdout(&root, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
        aprintln!("{} {}", p_g("✅"), p_g(&format!("Release {tag} prepared")));
        aprintln!("Push with: {}", p_c(&plan::push_command(&branch, new)));
    }
    Ok(())
}
