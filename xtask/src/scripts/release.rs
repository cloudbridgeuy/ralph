use crate::cli::ReleaseArgs;
use color_eyre::eyre::{eyre, Context, Result};
use duct::cmd;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const GITHUB_REPO: &str = "cloudbridgeuy/mcptools";
const WORKFLOW_FILE: &str = "release.yml";
const WORKFLOW_CHECK_INTERVAL: u64 = 30;
const WORKFLOW_TIMEOUT: u64 = 1800; // 30 minutes
const MAX_RETRIES: usize = 3;

/// Colors for terminal output
struct Colors;

impl Colors {
    fn info(msg: &str) -> String {
        format!("\x1b[0;34mINFO:\x1b[0m {msg}")
    }

    fn success(msg: &str) -> String {
        format!("\x1b[0;32mSUCCESS:\x1b[0m {msg}")
    }

    fn warning(msg: &str) -> String {
        format!("\x1b[1;33mWARNING:\x1b[0m {msg}")
    }

    fn error(msg: &str) -> String {
        format!("\x1b[0;31mERROR:\x1b[0m {msg}")
    }

    fn step(msg: &str) -> String {
        format!("\x1b[0;36m\x1b[1m==>\x1b[0m {msg}")
    }
}

fn get_project_root() -> Result<PathBuf> {
    Ok(env::current_dir()?)
}

fn check_gh_cli() -> Result<()> {
    println!("{}", Colors::info("Checking GitHub CLI availability..."));

    if cmd!("which", "gh").run().is_err() {
        return Err(eyre!(
            "GitHub CLI (gh) is required but not installed. Install it from: https://cli.github.com/"
        ));
    }
    println!("{}", Colors::info("GitHub CLI found"));

    println!("{}", Colors::info("Checking GitHub CLI authentication..."));
    if cmd!("gh", "auth", "status").run().is_err() {
        return Err(eyre!("GitHub CLI is not authenticated. Run: gh auth login"));
    }
    println!("{}", Colors::info("GitHub CLI authenticated successfully"));

    Ok(())
}

fn check_main_branch() -> Result<()> {
    println!("{}", Colors::info("Checking current git branch..."));
    let output = cmd!("git", "branch", "--show-current")
        .read()
        .context("Failed to get current branch")?;

    let current_branch = output.trim();
    if current_branch != "main" {
        return Err(eyre!(
            "You must be on the main branch to create a release. Current branch: {}",
            current_branch
        ));
    }
    println!("{}", Colors::info("On main branch ✓"));
    Ok(())
}

fn check_clean_working_dir() -> Result<()> {
    println!("{}", Colors::info("Checking working directory status..."));
    let output = cmd!("git", "status", "--porcelain")
        .read()
        .context("Failed to check git status")?;

    if !output.trim().is_empty() {
        println!(
            "{}",
            Colors::error("Working directory is not clean. Please commit or stash your changes.")
        );
        cmd!("git", "status", "--short").run()?;
        return Err(eyre!("Working directory not clean"));
    }
    println!("{}", Colors::info("Working directory is clean ✓"));
    Ok(())
}

fn check_ci_status() -> Result<()> {
    println!(
        "{}",
        Colors::info("Checking CI status for current commit...")
    );

    // Get current commit SHA
    let commit_sha = cmd!("git", "rev-parse", "HEAD")
        .read()
        .context("Failed to get current commit SHA")?
        .trim()
        .to_string();

    println!(
        "{}",
        Colors::info(&format!("Current commit: {}", &commit_sha[..8]))
    );

    // Check CI status using GitHub API
    let output = cmd!(
        "gh",
        "api",
        format!("/repos/{}/commits/{}/check-runs", GITHUB_REPO, commit_sha),
        "-H",
        "Accept: application/vnd.github+json"
    )
    .read()
    .context("Failed to fetch CI status from GitHub. Make sure gh CLI is authenticated.")?;

    let json: serde_json::Value =
        serde_json::from_str(&output).context("Failed to parse GitHub API response")?;

    let check_runs = json["check_runs"]
        .as_array()
        .ok_or_else(|| eyre!("Invalid GitHub API response: missing check_runs"))?;

    if check_runs.is_empty() {
        return Err(eyre!(
            "No CI checks found for commit {}. Please wait for CI to run or push the commit to trigger CI.",
            &commit_sha[..8]
        ));
    }

    // Filter relevant CI checks (Test Suite and Build Check)
    let mut relevant_checks = vec![];
    for check in check_runs {
        let name = check["name"].as_str().unwrap_or("");
        if name.starts_with("Test Suite") || name.starts_with("Build Check") {
            relevant_checks.push(check);
        }
    }

    if relevant_checks.is_empty() {
        return Err(eyre!(
            "No relevant CI checks (Test Suite, Build Check) found for commit {}",
            &commit_sha[..8]
        ));
    }

    // Check if all relevant checks passed
    let mut all_passed = true;
    let mut any_in_progress = false;

    for check in &relevant_checks {
        let name = check["name"].as_str().unwrap_or("unknown");
        let status = check["status"].as_str().unwrap_or("unknown");
        let conclusion = check["conclusion"].as_str();

        if status != "completed" {
            println!("{}", Colors::warning(&format!("⏳ {name}: {status}")));
            any_in_progress = true;
        } else if let Some(conclusion_val) = conclusion {
            if conclusion_val == "success" {
                println!("{}", Colors::success(&format!("✅ {name}: passed")));
            } else {
                println!("{}", Colors::error(&format!("❌ {name}: {conclusion_val}")));
                all_passed = false;
            }
        }
    }

    if any_in_progress {
        return Err(eyre!(
            "CI checks are still in progress. Please wait for them to complete before creating a release."
        ));
    }

    if !all_passed {
        return Err(eyre!(
            "CI checks failed for commit {}. Please fix the issues before creating a release.",
            &commit_sha[..8]
        ));
    }

    println!("{}", Colors::info("All CI checks passed ✓"));
    Ok(())
}

fn get_current_version(project_root: &Path) -> Result<String> {
    let cargo_toml = project_root.join("crates/mcptools/Cargo.toml");
    let content = fs::read_to_string(&cargo_toml).context("Failed to read Cargo.toml")?;

    for line in content.lines() {
        if line.starts_with("version = ") {
            let version = line
                .trim_start_matches("version = \"")
                .trim_end_matches('"')
                .to_string();
            return Ok(version);
        }
    }

    Err(eyre!("Could not find version in Cargo.toml"))
}

fn validate_version(version: &str) -> Result<()> {
    println!(
        "{}",
        Colors::info(&format!("Validating version format: {version}"))
    );

    let re = regex::Regex::new(
        r"^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$",
    )?;

    if !re.is_match(version) {
        return Err(eyre!(
            "Invalid version format: {}. Please use semantic versioning (e.g., 1.0.0, 1.0.0-beta.1)",
            version
        ));
    }

    println!("{}", Colors::info("Version format is valid ✓"));
    Ok(())
}

fn update_version(project_root: &Path, new_version: &str) -> Result<()> {
    println!(
        "{}",
        Colors::info(&format!(
            "Updating version to {new_version} in Cargo.toml files..."
        ))
    );

    // Update crates/mcptools/Cargo.toml
    let mcptools_cargo = project_root.join("crates/mcptools/Cargo.toml");
    let content = fs::read_to_string(&mcptools_cargo)?;
    let new_content = content
        .lines()
        .map(|line| {
            if line.starts_with("version = ") {
                format!("version = \"{new_version}\"")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&mcptools_cargo, new_content + "\n")?;

    // Update Cargo.lock
    println!("{}", Colors::info("Updating Cargo.lock..."));
    cmd!("cargo", "check", "--quiet")
        .dir(project_root)
        .run()
        .context("cargo check failed after version update")?;

    Ok(())
}

fn cleanup_tag(tag: &str, cleanup_remote: bool) -> Result<()> {
    println!("{}", Colors::warning(&format!("Cleaning up tag: {tag}")));

    // Check if local tag exists
    let local_exists = cmd!("git", "tag", "-l", tag)
        .read()
        .map(|output| !output.trim().is_empty())
        .unwrap_or(false);

    if local_exists {
        println!("{}", Colors::info(&format!("Removing local tag: {tag}")));
        let _ = cmd!("git", "tag", "-d", tag).run();
    }

    // Remove remote tag if requested
    if cleanup_remote {
        println!(
            "{}",
            Colors::info(&format!("Checking if remote tag exists: {tag}"))
        );
        let remote_exists = cmd!("git", "ls-remote", "--tags", "origin")
            .read()
            .map(|output| output.contains(&format!("refs/tags/{tag}")))
            .unwrap_or(false);

        if remote_exists {
            println!("{}", Colors::info(&format!("Removing remote tag: {tag}")));
            let _ = cmd!("git", "push", "--delete", "origin", tag).run();
        }
    }

    Ok(())
}

fn rollback_version(project_root: &Path) -> Result<()> {
    println!("{}", Colors::warning("Rolling back version changes..."));

    // Reset Cargo.toml files
    let _ = cmd!(
        "git",
        "checkout",
        "HEAD",
        "--",
        "Cargo.toml",
        "crates/mcptools/Cargo.toml",
        "Cargo.lock"
    )
    .dir(project_root)
    .run();

    Ok(())
}

fn create_and_push_tag(version: &str, monitor: bool, project_root: &Path) -> Result<()> {
    let tag = format!("v{version}");

    // Clean up any existing tag first
    cleanup_tag(&tag, true)?;

    println!(
        "{}",
        Colors::step(&format!("Creating git commit for version {version}..."))
    );
    cmd!(
        "git",
        "add",
        "Cargo.toml",
        "crates/mcptools/Cargo.toml",
        "Cargo.lock"
    )
    .dir(project_root)
    .run()?;
    cmd!(
        "git",
        "commit",
        "-m",
        &format!("chore: bump version to {version}")
    )
    .dir(project_root)
    .run()?;

    println!("{}", Colors::step(&format!("Creating git tag {tag}...")));
    cmd!(
        "git",
        "tag",
        "-a",
        &tag,
        "-m",
        &format!("Release {version}")
    )
    .dir(project_root)
    .run()?;

    println!("{}", Colors::step("Pushing changes and tag to origin..."));
    cmd!("git", "push", "origin", "main")
        .dir(project_root)
        .run()?;
    cmd!("git", "push", "origin", &tag)
        .dir(project_root)
        .run()?;

    println!(
        "{}",
        Colors::success(&format!("✅ Tag {tag} created and pushed successfully!"))
    );

    if monitor {
        println!("{}", Colors::info("Monitoring GitHub Actions workflow..."));
        println!(
            "{}",
            Colors::info(&format!(
                "You can also monitor at: https://github.com/{GITHUB_REPO}/actions"
            ))
        );

        if wait_for_workflow(&tag)? {
            println!();
            println!(
                "{}",
                Colors::success(&format!("🎉 Release {version} completed successfully!"))
            );
            Ok(())
        } else {
            Err(eyre!("GitHub Actions workflow failed"))
        }
    } else {
        println!(
            "{}",
            Colors::info(&format!(
                "Skipping workflow monitoring. Check status at: https://github.com/{GITHUB_REPO}/actions"
            ))
        );
        Ok(())
    }
}

fn wait_for_workflow(tag: &str) -> Result<bool> {
    let start_time = Instant::now();
    println!(
        "{}",
        Colors::step(&format!(
            "Monitoring GitHub Actions workflow for tag: {tag}"
        ))
    );
    println!(
        "{}",
        Colors::info(&format!(
            "Workflow timeout: {}s ({} minutes)",
            WORKFLOW_TIMEOUT,
            WORKFLOW_TIMEOUT / 60
        ))
    );

    // Wait for workflow to start
    println!("{}", Colors::info("Waiting for workflow to start..."));
    thread::sleep(Duration::from_secs(10));

    loop {
        let elapsed = start_time.elapsed().as_secs();

        if elapsed > WORKFLOW_TIMEOUT {
            println!();
            println!(
                "{}",
                Colors::error(&format!("Workflow timeout after {WORKFLOW_TIMEOUT}s"))
            );
            return Ok(false);
        }

        // Get workflow runs for this tag
        let output = cmd!(
            "gh",
            "run",
            "list",
            "--repo",
            GITHUB_REPO,
            "--workflow",
            WORKFLOW_FILE,
            "--event",
            "push",
            "--limit",
            "5",
            "--json",
            "status,conclusion,headBranch,headSha,event"
        )
        .read()
        .unwrap_or_default();

        if let Ok(runs) = serde_json::from_str::<Vec<serde_json::Value>>(&output) {
            for run in runs {
                let head_branch = run["headBranch"].as_str().unwrap_or("");

                if head_branch == tag {
                    let status = run["status"].as_str().unwrap_or("");
                    let conclusion = run["conclusion"].as_str();

                    match status {
                        "completed" => {
                            println!();
                            match conclusion {
                                Some("success") => {
                                    println!(
                                        "{}",
                                        Colors::success(
                                            "✅ GitHub Actions workflow completed successfully!"
                                        )
                                    );
                                    println!("{}", Colors::info(&format!("Release should be available at: https://github.com/{GITHUB_REPO}/releases/tag/{tag}")));
                                    return Ok(true);
                                }
                                Some(conclusion_val) => {
                                    println!(
                                        "{}",
                                        Colors::error(&format!(
                                            "❌ GitHub Actions workflow failed with conclusion: {conclusion_val}"
                                        ))
                                    );
                                    println!(
                                        "{}",
                                        Colors::error(&format!(
                                            "Check workflow logs at: https://github.com/{GITHUB_REPO}/actions"
                                        ))
                                    );
                                    return Ok(false);
                                }
                                None => {
                                    println!(
                                        "{}",
                                        Colors::warning(
                                            "⚠️  Workflow completed with unknown conclusion"
                                        )
                                    );
                                    return Ok(false);
                                }
                            }
                        }
                        "in_progress" | "queued" | "requested" | "waiting" | "pending" => {
                            print!(
                                "\r{}  ⏳ Workflow status: {} ({}s elapsed)",
                                Colors::info(""),
                                status,
                                elapsed
                            );
                            io::stdout().flush().ok();
                        }
                        _ => {
                            println!(
                                "{}",
                                Colors::warning(&format!("Unknown workflow status: {status}"))
                            );
                        }
                    }
                    break;
                }
            }
        } else {
            print!(
                "\r{}  🔍 Looking for workflow... ({}s elapsed)",
                Colors::info(""),
                elapsed
            );
            io::stdout().flush().ok();
        }

        thread::sleep(Duration::from_secs(WORKFLOW_CHECK_INTERVAL));
    }
}

fn cleanup_after_failure(version: &str, project_root: &Path) -> Result<()> {
    let tag = format!("v{version}");
    println!(
        "{}",
        Colors::error("Release process failed. Starting cleanup...")
    );

    cleanup_tag(&tag, true)?;
    rollback_version(project_root)?;

    println!(
        "{}",
        Colors::warning("Cleanup completed. You can now fix the issues and try again.")
    );
    Ok(())
}

fn confirm(prompt: &str) -> bool {
    print!("{prompt} (y/N): ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();

    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

fn retry_release(version: &str, monitor: bool, project_root: &Path) -> Result<()> {
    let mut retry_count = 0;

    while retry_count < MAX_RETRIES {
        if retry_count > 0 {
            println!(
                "{}",
                Colors::warning(&format!("Retry attempt {retry_count} of {MAX_RETRIES}"))
            );
            println!();

            if !confirm("Do you want to retry the release?") {
                println!("{}", Colors::info("Release cancelled by user."));
                return Err(eyre!("Release cancelled after failure"));
            }
        }

        match create_and_push_tag(version, monitor, project_root) {
            Ok(_) => return Ok(()),
            Err(e) => {
                retry_count += 1;
                println!(
                    "{}",
                    Colors::error(&format!("Release attempt {retry_count} failed: {e}"))
                );

                if retry_count < MAX_RETRIES {
                    cleanup_after_failure(version, project_root)?;
                    println!(
                        "{}",
                        Colors::info("Cleaned up failed release. Ready for retry.")
                    );
                    println!();
                }
            }
        }
    }

    println!(
        "{}",
        Colors::error(&format!("All {MAX_RETRIES} release attempts failed."))
    );
    cleanup_after_failure(version, project_root)?;
    Err(eyre!("Release failed after {} attempts", MAX_RETRIES))
}

fn cleanup_command(tag: &str, project_root: &Path) -> Result<()> {
    println!(
        "{}",
        Colors::step(&format!("Cleaning up failed release: {tag}"))
    );

    // Validate tag format
    let re = regex::Regex::new(
        r"^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$",
    )?;
    if !re.is_match(tag) {
        return Err(eyre!(
            "Invalid tag format: {}. Expected format: v1.0.0",
            tag
        ));
    }

    let version = tag.trim_start_matches('v');

    println!();
    println!("{}", Colors::warning("This will:"));
    println!("  • Remove local tag: {tag}");
    println!("  • Remove remote tag: {tag} (if exists)");
    println!("  • Rollback version changes (if any)");
    println!();

    if !confirm(&format!("Are you sure you want to cleanup {tag}?")) {
        println!("{}", Colors::info("Cleanup cancelled."));
        return Ok(());
    }

    cleanup_after_failure(version, project_root)?;
    println!(
        "{}",
        Colors::success(&format!("✅ Cleanup completed for {tag}"))
    );
    Ok(())
}

pub fn release(args: &ReleaseArgs) -> Result<()> {
    let project_root = get_project_root()?;

    // Handle cleanup command
    if let Some(tag) = &args.cleanup {
        check_gh_cli()?;
        return cleanup_command(tag, &project_root);
    }

    // Require version argument for release
    let version = args
        .version
        .as_ref()
        .ok_or_else(|| eyre!("Version argument required. Usage: cargo xtask release <version>"))?;

    println!(
        "{}",
        Colors::step(&format!(
            "Starting release process for version {version}..."
        ))
    );
    println!();

    // Validations
    println!("{}", Colors::step("Running pre-release checks..."));
    check_gh_cli()?;
    check_main_branch()?;
    check_clean_working_dir()?;
    check_ci_status()?;
    validate_version(version)?;

    // Get current version
    let current_version = get_current_version(&project_root)?;
    println!(
        "{}",
        Colors::info(&format!("Current version: {current_version}"))
    );
    println!("{}", Colors::info(&format!("New version: {version}")));

    // Check if version is different
    if current_version == *version {
        return Err(eyre!(
            "New version ({}) is the same as current version. Please specify a different version.",
            version
        ));
    }

    // Show what will happen
    println!();
    println!("{}", Colors::step("Release Plan:"));
    println!("  1. Update version in Cargo.toml files");
    println!("  2. Create git commit and tag v{version}");
    println!("  3. Push changes and tag to GitHub");
    println!("  4. Monitor GitHub Actions workflow");
    println!("  5. Verify release creation");
    println!("  6. If workflow fails: cleanup and offer retry");
    println!();

    // Confirm with user
    if !confirm(&format!(
        "Are you sure you want to release version {version}?"
    )) {
        println!("{}", Colors::info("Release cancelled."));
        return Ok(());
    }

    // Perform release with retry mechanism
    update_version(&project_root, version)?;
    let monitor = !args.no_monitor;
    retry_release(version, monitor, &project_root)?;

    println!();
    println!(
        "{}",
        Colors::success(&format!("🎉 Release {version} completed successfully!"))
    );
    println!(
        "{}",
        Colors::info(&format!(
            "📦 Release available at: https://github.com/{GITHUB_REPO}/releases/tag/v{version}"
        ))
    );
    println!(
        "{}",
        Colors::info("📋 Installation instructions are included in the release notes.")
    );

    // Offer to upgrade if binary is installed
    if args.auto_upgrade
        || (cmd!("which", "mcptools").run().is_ok()
            && confirm("Would you like to upgrade your local mcptools binary to the new version?"))
    {
        println!("{}", Colors::step("Testing the upgrade command..."));
        println!(
            "{}",
            Colors::info("Running 'mcptools upgrade' to verify the release works correctly")
        );

        if cmd!("mcptools", "upgrade").run().is_ok() {
            println!(
                "{}",
                Colors::success("✅ Upgrade command executed successfully!")
            );
            println!(
                "{}",
                Colors::info(&format!(
                    "Your mcptools binary has been updated to version {version}"
                ))
            );
        } else {
            println!(
                "{}",
                Colors::warning(
                    "⚠️  Upgrade command failed, but the release was created successfully"
                )
            );
            println!(
                "{}",
                Colors::info("You can still download the release manually from GitHub")
            );
        }
    } else {
        println!(
            "{}",
            Colors::info("💡 You can upgrade later by running: mcptools upgrade")
        );
    }

    Ok(())
}
