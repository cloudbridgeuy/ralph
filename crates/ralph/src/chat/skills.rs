use crate::chat::memory::Store;
use crate::warn::warn;
use ralph_core::chat::{mcp_config, select_servers};
use ralph_core::chat_skills::{plan_links, plugin_manifest, ChatSkill};
use std::io::{self};
use std::path::{Path, PathBuf};

const SKILLS_ENV: &str = "RALPH_CHAT_SKILLS";

pub(super) fn write_mcp_config(
    store: &Store,
    store_dir: &Path,
    socket: &Path,
    exe: &Path,
    server_names: &[String],
) -> io::Result<PathBuf> {
    let claude_config = read_claude_config();
    let imported = select_servers(&claude_config, server_names);
    for name in &imported.missing {
        warn(format!(
            "chat: MCP server \"{name}\" not found in ~/.claude.json; continuing without it"
        ));
    }
    store.write_mcp_config(&mcp_config(exe, store_dir, socket, &imported.servers))
}

pub(super) fn env_skills() -> Vec<PathBuf> {
    std::env::var_os(SKILLS_ENV)
        .map(|value| {
            std::env::split_paths(&value)
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn usable_skills(dirs: Vec<PathBuf>) -> Vec<(PathBuf, String)> {
    dirs.into_iter()
        .filter_map(|dir| {
            let dir = std::path::absolute(&dir).unwrap_or(dir);
            match std::fs::read_to_string(dir.join("SKILL.md")) {
                Ok(skill_md) => Some((dir, skill_md)),
                Err(e) => {
                    warn(format!(
                        "chat: skipping skill {}: cannot read SKILL.md ({e})",
                        dir.display()
                    ));
                    None
                }
            }
        })
        .collect()
}

pub(super) fn link_skills(found: Vec<(PathBuf, String)>) -> Vec<ChatSkill> {
    let dirs: Vec<PathBuf> = found.iter().map(|(dir, _)| dir.clone()).collect();
    let plan = plan_links(&dirs);
    for dir in &plan.dropped {
        warn(format!(
            "chat: skipping skill {}: name already taken by an earlier skill",
            dir.display()
        ));
    }
    plan.links
        .into_iter()
        .filter_map(|(name, dir)| {
            let skill_md = found
                .iter()
                .find(|(found_dir, _)| *found_dir == dir)?
                .1
                .clone();
            Some(ChatSkill {
                name,
                dir,
                skill_md,
            })
        })
        .collect()
}

pub(super) fn write_skill_plugin(skills: &[ChatSkill]) -> io::Result<Option<tempfile::TempDir>> {
    if skills.is_empty() {
        return Ok(None);
    }
    let plugin = tempfile::tempdir()?;
    let manifest_dir = plugin.path().join(".claude-plugin");
    let skills_dir = plugin.path().join("skills");
    std::fs::create_dir_all(&manifest_dir)?;
    std::fs::create_dir_all(&skills_dir)?;
    std::fs::write(
        manifest_dir.join("plugin.json"),
        plugin_manifest().to_string(),
    )?;
    for skill in skills {
        std::os::unix::fs::symlink(&skill.dir, skills_dir.join(&skill.name))?;
    }
    Ok(Some(plugin))
}

pub(super) fn read_claude_config() -> serde_json::Value {
    let Some(path) = dirs::home_dir().map(|home| home.join(".claude.json")) else {
        return serde_json::json!({});
    };
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            warn(format!("Failed to parse {}: {e}", path.display()));
            serde_json::json!({})
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => {
            warn(format!("Failed to read {}: {e}", path.display()));
            serde_json::json!({})
        }
    }
}
