use color_eyre::eyre::{eyre, Result};
use duct::cmd;
use std::env;
use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

/// Available git hooks
const HOOKS: &[&str] = &["pre-commit"];

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

/// Get project root directory
fn get_project_root() -> Result<PathBuf> {
    let current_dir = env::current_dir()?;
    // When running via `cargo xtask`, the current dir is already the workspace root
    // Just verify this is a git repo and return it
    Ok(current_dir)
}

/// Check if we're in a git repository
fn check_git_repo(project_root: &Path) -> Result<()> {
    let git_dir = project_root.join(".git");
    if !git_dir.exists() {
        return Err(eyre!(
            "This directory is not a git repository. Please run this command from within a git repository."
        ));
    }
    Ok(())
}

/// Check if hooks directory exists
fn check_hooks_directory(hooks_dir: &Path) -> Result<()> {
    if !hooks_dir.exists() {
        return Err(eyre!(
            "Hooks directory not found: {}. Please ensure the hooks are available in the project.",
            hooks_dir.display()
        ));
    }
    Ok(())
}

/// Create git hooks directory if it doesn't exist
fn create_git_hooks_dir(git_hooks_dir: &Path) -> Result<()> {
    if !git_hooks_dir.exists() {
        println!(
            "{}",
            Colors::info(&format!(
                "Creating git hooks directory: {}",
                git_hooks_dir.display()
            ))
        );
        fs::create_dir_all(git_hooks_dir)?;
    }
    Ok(())
}

/// Backup existing hook
fn backup_existing_hook(hook_path: &Path) -> Result<()> {
    if hook_path.exists() && !hook_path.is_symlink() {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let backup_path = hook_path.with_extension(format!("backup.{timestamp}"));
        println!(
            "{}",
            Colors::warning(&format!(
                "Backing up existing {} hook to: {}",
                hook_path.file_name().unwrap().to_string_lossy(),
                backup_path.file_name().unwrap().to_string_lossy()
            ))
        );
        fs::rename(hook_path, backup_path)?;
    }
    Ok(())
}

/// Install a specific hook
fn install_hook(hook_name: &str, hooks_dir: &Path, git_hooks_dir: &Path) -> Result<bool> {
    let source_hook = hooks_dir.join(hook_name);
    let target_hook = git_hooks_dir.join(hook_name);

    if !source_hook.exists() {
        println!(
            "{}",
            Colors::warning(&format!("Hook not found: {}", source_hook.display()))
        );
        return Ok(false);
    }

    println!(
        "{}",
        Colors::info(&format!("Installing {hook_name} hook..."))
    );

    // Backup existing hook if it exists
    backup_existing_hook(&target_hook)?;

    // Remove target if it exists (could be a broken symlink)
    if target_hook.exists() || target_hook.symlink_metadata().is_ok() {
        fs::remove_file(&target_hook)?;
    }

    // Create symlink to our hook
    unix_fs::symlink(&source_hook, &target_hook)?;

    // Make sure source is executable
    let metadata = fs::metadata(&source_hook)?;
    let mut permissions = metadata.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
    }
    fs::set_permissions(&source_hook, permissions)?;

    println!(
        "{}",
        Colors::success(&format!("✅ {hook_name} hook installed"))
    );
    Ok(true)
}

/// Install all available hooks
pub fn install_hooks() -> Result<()> {
    println!(
        "{}",
        Colors::step("MCPTOOLS DevOps CLI Git Hooks Installer")
    );
    println!();

    let project_root = get_project_root()?;
    let hooks_dir = project_root.join("scripts").join("hooks");
    let git_hooks_dir = project_root.join(".git").join("hooks");

    check_git_repo(&project_root)?;
    check_hooks_directory(&hooks_dir)?;
    create_git_hooks_dir(&git_hooks_dir)?;

    println!("{}", Colors::step("Installing git hooks..."));
    println!();

    let mut installed_count = 0;
    let mut failed_count = 0;

    for hook in HOOKS {
        match install_hook(hook, &hooks_dir, &git_hooks_dir) {
            Ok(true) => installed_count += 1,
            Ok(false) => failed_count += 1,
            Err(e) => {
                println!(
                    "{}",
                    Colors::error(&format!("Failed to install {hook}: {e}"))
                );
                failed_count += 1;
            }
        }
    }

    println!();
    println!("{}", Colors::info("Installation summary:"));
    println!(
        "{}",
        Colors::success(&format!("  • Installed: {installed_count} hooks"))
    );
    if failed_count > 0 {
        println!(
            "{}",
            Colors::warning(&format!("  • Failed: {failed_count} hooks"))
        );
    }

    println!();
    if check_installation_internal(&git_hooks_dir)? {
        println!();
        println!(
            "{}",
            Colors::success("🎉 Git hooks installation completed successfully!")
        );
        println!();
        println!("{}", Colors::info("The following hooks are now active:"));
        println!(
            "{}",
            Colors::info("  • pre-commit: Runs cargo fmt, check, and clippy")
        );
        println!();
        println!(
            "{}",
            Colors::info("You can check hook status anytime with:")
        );
        println!("{}", Colors::info("  cargo xtask hooks status"));
    } else {
        return Err(eyre!("Some hooks failed to install properly"));
    }

    Ok(())
}

/// Uninstall hooks
pub fn uninstall_hooks() -> Result<()> {
    println!(
        "{}",
        Colors::step("Uninstalling MCPTOOLS DevOps CLI Git Hooks")
    );
    println!();

    let project_root = get_project_root()?;
    let hooks_dir = project_root.join("scripts").join("hooks");
    let git_hooks_dir = project_root.join(".git").join("hooks");

    check_git_repo(&project_root)?;

    let mut removed_count = 0;

    for hook in HOOKS {
        let hook_path = git_hooks_dir.join(hook);
        let source_hook = hooks_dir.join(hook);

        if hook_path.is_symlink() {
            // Check if it points to our hook
            if let Ok(target) = fs::read_link(&hook_path) {
                if target == source_hook {
                    println!("{}", Colors::info(&format!("Removing {hook} hook...")));
                    fs::remove_file(&hook_path)?;
                    println!("{}", Colors::success(&format!("✅ {hook} hook removed")));
                    removed_count += 1;
                } else {
                    println!(
                        "{}",
                        Colors::warning(&format!(
                            "❓ {hook} exists but points to different source (skipping)"
                        ))
                    );
                }
            }
        } else if hook_path.exists() {
            println!(
                "{}",
                Colors::warning(&format!(
                    "❓ {hook} exists but is not our symlink (skipping)"
                ))
            );
        }
    }

    if removed_count > 0 {
        println!(
            "{}",
            Colors::success(&format!("Removed {removed_count} hooks"))
        );
    } else {
        println!("{}", Colors::info("No hooks to remove"));
    }

    Ok(())
}

/// Show hook status
pub fn show_status() -> Result<()> {
    let project_root = get_project_root()?;
    let hooks_dir = project_root.join("scripts").join("hooks");
    let git_hooks_dir = project_root.join(".git").join("hooks");

    check_git_repo(&project_root)?;

    println!("{}", Colors::step("Git hooks status:"));
    println!();

    for hook in HOOKS {
        let hook_path = git_hooks_dir.join(hook);
        let source_hook = hooks_dir.join(hook);

        print!("  {:<12} ", format!("{}:", hook));

        if hook_path.is_symlink() {
            if let Ok(target) = fs::read_link(&hook_path) {
                if target == source_hook {
                    println!("\x1b[0;32m✅ Installed\x1b[0m");
                } else {
                    println!("\x1b[1;33m⚠️  Installed (different source)\x1b[0m");
                }
            } else {
                println!("\x1b[0;31m❌ Broken symlink\x1b[0m");
            }
        } else if hook_path.exists() {
            println!("\x1b[1;33m❓ Exists (not our hook)\x1b[0m");
        } else {
            println!("\x1b[0;31m❌ Not installed\x1b[0m");
        }
    }

    println!();
    println!("{}", Colors::info("Available hooks:"));
    println!(
        "{}",
        Colors::info("  • pre-commit: Runs cargo fmt, check, and clippy before each commit")
    );
    println!(
        "{}",
        Colors::info("                Supports --force flag to check all Rust files")
    );

    Ok(())
}

/// Test hooks
pub fn test_hooks() -> Result<()> {
    println!("{}", Colors::step("Testing hooks..."));

    let project_root = get_project_root()?;
    let git_hooks_dir = project_root.join(".git").join("hooks");

    check_git_repo(&project_root)?;

    let pre_commit_hook = git_hooks_dir.join("pre-commit");

    if pre_commit_hook.exists() && pre_commit_hook.is_symlink() {
        println!("{}", Colors::info("Testing pre-commit hook..."));
        println!();

        println!("{}", Colors::info("Hook supports the following options:"));
        println!(
            "{}",
            Colors::info("  • Normal mode: Only checks staged .rs files")
        );
        println!(
            "{}",
            Colors::info("  • --force mode: Checks all .rs files in project")
        );
        println!("{}", Colors::info("  • --help: Shows usage information"));
        println!();

        println!("{}", Colors::info("Running pre-commit hook with --help:"));
        println!();

        match cmd!(&pre_commit_hook, "--help").run() {
            Ok(_) => {
                println!();
                println!(
                    "{}",
                    Colors::success("✅ Pre-commit hook is executable and responsive")
                );
            }
            Err(e) => {
                println!();
                println!(
                    "{}",
                    Colors::warning(&format!("⚠️  Pre-commit hook test had issues: {e}"))
                );
                println!(
                    "{}",
                    Colors::info("This might indicate code quality issues that need to be fixed")
                );
            }
        }
    } else {
        println!(
            "{}",
            Colors::error("❌ Pre-commit hook not found or not executable")
        );
    }

    Ok(())
}

/// Internal check installation (returns bool instead of exiting)
fn check_installation_internal(git_hooks_dir: &Path) -> Result<bool> {
    let mut all_good = true;

    for hook in HOOKS {
        let hook_path = git_hooks_dir.join(hook);
        if !(hook_path.is_symlink() && hook_path.exists()) {
            all_good = false;
        }
    }

    Ok(all_good)
}
