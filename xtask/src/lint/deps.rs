use crate::prelude::*;

pub const BANNED: &[&str] = &[
    "syntect",
    "termimad",
    "crossterm",
    "resvg",
    "reedline",
    "signal-hook",
];

pub fn banned_in(tree: &str, banned: &[&str]) -> Vec<String> {
    banned
        .iter()
        .filter(|name| {
            tree.lines()
                .any(|line| line.split_whitespace().any(|word| word == **name))
        })
        .map(|name| name.to_string())
        .collect()
}

pub async fn run_server_deps_check(global: &crate::Global) -> super::error::Result<bool> {
    if !global.is_silent() {
        aprintln!(
            "{} {}",
            p_b("🔧"),
            p_b("Checking ralph_server for terminal crates...")
        );
    }

    let output = tokio::process::Command::new("cargo")
        .args([
            "tree",
            "-p",
            "ralph_server",
            "-e",
            "normal",
            "--prefix",
            "none",
        ])
        .output()
        .await?;

    if !output.status.success() {
        aprintln!("{} Failed to run cargo tree for ralph_server", p_r("❌"));
        return Ok(false);
    }

    let found = banned_in(&String::from_utf8_lossy(&output.stdout), BANNED);
    if found.is_empty() {
        if !global.is_silent() {
            aprintln!("{} {}", p_g("✅"), "ralph_server pulls no terminal crates");
        }
        Ok(true)
    } else {
        aprintln!(
            "{} ralph_server pulls terminal crates: {}",
            p_r("❌"),
            found.join(", ")
        );
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = "ralph_server v0.1.0 (/repo/crates/ralph_server)\n├── ralph_core v0.1.0\n│   └── serde v1.0.0\n├── ureq v3.0.0\n";

    #[test]
    fn passes_a_tree_without_banned_crates() {
        assert!(banned_in(TREE, BANNED).is_empty());
    }

    #[test]
    fn names_each_banned_crate_once() {
        let tree = format!("{TREE}│   ├── crossterm v0.28.1\n└── crossterm v0.28.1 (*)\n    └── signal-hook v0.3.17\n");
        assert_eq!(banned_in(&tree, BANNED), vec!["crossterm", "signal-hook"]);
    }

    #[test]
    fn does_not_match_a_crate_that_only_shares_a_prefix() {
        let tree = "└── signal-hook-registry v1.4.2\n└── syntect-extra v0.1.0\n";
        assert!(banned_in(tree, BANNED).is_empty());
    }
}
