use crate::prelude::*;
use error::{ReleaseError, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;

mod error;
mod plan;

#[derive(Debug, clap::Parser)]
#[command(
    long_about = "Cut a release: bump the shared workspace version, refresh Cargo.lock, commit, and create an annotated vX.Y.Z tag.

Nothing is pushed. The command prints the push command to run when you are ready."
)]
pub struct ReleaseCommand {
    /// New version in strict x.y.z form
    pub version: String,
}

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

pub async fn run(command: ReleaseCommand, global: crate::Global) -> Result<()> {
    let new = plan::parse_release_version(&command.version)?;
    let root = PathBuf::from(git_stdout(Path::new("."), &["rev-parse", "--show-toplevel"]).await?);
    let manifest_path = root.join("Cargo.toml");
    let manifest = tokio::fs::read_to_string(&manifest_path).await?;

    plan::ensure_newer(plan::read_workspace_version(&manifest)?, new)?;
    let rewritten = plan::set_workspace_version(&manifest, new)?;
    let tag = plan::tag_name(new);

    if !git_stdout(&root, &["status", "--porcelain"])
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

    tokio::fs::write(&manifest_path, rewritten).await?;
    run_checked(&root, "cargo", &["update", "--workspace"]).await?;
    run_checked(&root, "git", &["add", "Cargo.toml", "Cargo.lock"]).await?;
    run_checked(&root, "git", &["commit", "-m", &plan::commit_message(new)]).await?;
    run_checked(&root, "git", &["tag", "-a", &tag, "-m", &tag]).await?;

    if !global.is_silent() {
        let branch = git_stdout(&root, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
        aprintln!("{} {}", p_g("✅"), p_g(&format!("Release {tag} prepared")));
        aprintln!("Push with: {}", p_c(&plan::push_command(&branch, new)));
    }
    Ok(())
}
