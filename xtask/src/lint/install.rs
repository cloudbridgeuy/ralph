use crate::prelude::*;

#[derive(Debug, PartialEq, Eq)]
pub struct InstallViolation {
    pub line: usize,
    pub snippet: String,
}

fn is_exempt_path(path: &str) -> bool {
    path.ends_with("CHANGELOG.md") || path.starts_with("vendor/")
}

fn command_text(line: &str) -> &str {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix("- ").unwrap_or(trimmed);
    let trimmed = trimmed.strip_prefix("run:").unwrap_or(trimmed);
    let trimmed = trimmed.strip_prefix('$').unwrap_or(trimmed);
    trimmed.trim_start()
}

fn is_unlocked_install(line: &str) -> bool {
    let command = command_text(line);
    command.starts_with("cargo install")
        && !command.contains("--locked")
        && !command.contains("--list")
}

pub fn find_unlocked_installs(path: &str, text: &str) -> Vec<InstallViolation> {
    if is_exempt_path(path) {
        return Vec::new();
    }
    text.lines()
        .enumerate()
        .filter(|(_, line)| is_unlocked_install(line))
        .map(|(index, line)| InstallViolation {
            line: index + 1,
            snippet: line.trim().to_string(),
        })
        .collect()
}

pub async fn run_install_lock_check(global: &crate::Global) -> super::error::Result<bool> {
    if !global.is_silent() {
        aprintln!(
            "{} {}",
            p_b("🔧"),
            p_b("Checking cargo install lines for --locked...")
        );
    }

    let output = tokio::process::Command::new("git")
        .args(["ls-files", "*.md", ".github/workflows/*.yml"])
        .output()
        .await?;

    if !output.status.success() {
        aprintln!("{} Failed to list documentation files", p_r("❌"));
        return Ok(false);
    }

    let files = String::from_utf8_lossy(&output.stdout);
    let mut total = 0;
    for path in files.lines().filter(|l| !l.is_empty()) {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for violation in find_unlocked_installs(path, &text) {
            if total == 0 {
                aprintln!("{} cargo install without --locked:", p_r("❌"));
            }
            total += 1;
            aprintln!("  {}:{}: {}", p_r(path), violation.line, violation.snippet);
        }
    }

    if total == 0 {
        if !global.is_silent() {
            aprintln!("{} {}", p_g("✅"), "Every cargo install line uses --locked");
        }
        Ok(true)
    } else {
        if !global.is_silent() {
            aprintln!();
            aprintln!(
                "{} Add --locked so the install uses the versions in Cargo.lock",
                p_b("Tip:")
            );
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(path: &str, text: &str) -> Vec<usize> {
        find_unlocked_installs(path, text)
            .iter()
            .map(|v| v.line)
            .collect()
    }

    #[test]
    fn passes_locked_install() {
        let text = "cargo install --locked --path crates/ralph --root ~/.local --force\n";
        assert!(hits("README.md", text).is_empty());
    }

    #[test]
    fn flags_unlocked_install_with_line_number() {
        let text = "intro\n```bash\ncargo install --path crates/ralph --force  # note\n```\n";
        assert_eq!(hits("README.md", text), vec![3]);
    }

    #[test]
    fn flags_unlocked_workflow_step() {
        let text = "steps:\n  - run: cargo install mdbook\n";
        assert_eq!(hits(".github/workflows/docs.yml", text), vec![2]);
    }

    #[test]
    fn ignores_install_list() {
        assert!(hits("README.md", "cargo install --list\n").is_empty());
    }

    #[test]
    fn ignores_prose_without_command() {
        let text = "Run `cargo install` to put ralph on your path.\nUse cargo install-update.\n";
        assert_eq!(hits("README.md", text), Vec::<usize>::new());
    }

    #[test]
    fn skips_changelog_and_vendor() {
        let text = "cargo install --path crates/ralph\n";
        assert!(hits("CHANGELOG.md", text).is_empty());
        assert!(hits("vendor/reedline/README.md", text).is_empty());
    }
}
