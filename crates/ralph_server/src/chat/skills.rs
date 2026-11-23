use crate::private_file::write_private;
use crate::warn::warn;
use ralph_core::chat::{build_mcp_args, mcp_config, ImportedServers};
use ralph_core::chat_skills::{plan_links, plugin_manifest, ChatSkill};
use ralph_core::chat_socket::{mcp_config_path, skill_plugin_dir};
use ralph_core::claude_leftovers::claude_json;
use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const SKILLS_ENV: &str = "RALPH_CHAT_SKILLS";

pub fn write_mcp_config(
    store_dir: &Path,
    files_dir: &Path,
    socket: &Path,
    exe: &Path,
    imported: &ImportedServers,
) -> io::Result<Vec<String>> {
    for name in &imported.missing {
        warn(format!(
            "chat: MCP server \"{name}\" not found in ~/.claude.json; continuing without it"
        ));
    }
    let config = mcp_config(exe, store_dir, socket, &imported.servers);
    let path = write_private(mcp_config_path(files_dir), config.to_string().as_bytes())?;
    Ok(build_mcp_args(&path, &config))
}

pub fn env_skills() -> Vec<PathBuf> {
    std::env::var_os(SKILLS_ENV)
        .map(|value| {
            std::env::split_paths(&value)
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn usable_skills(dirs: Vec<PathBuf>) -> Vec<(PathBuf, String)> {
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

pub fn link_skills(found: Vec<(PathBuf, String)>) -> Vec<ChatSkill> {
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

pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    match DirBuilder::new().mode(0o700).recursive(true).create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        }
        Err(e) => Err(e),
    }
}

pub fn write_skill_plugin(files_dir: &Path, skills: &[ChatSkill]) -> io::Result<Option<PathBuf>> {
    if skills.is_empty() {
        return Ok(None);
    }
    let plugin = skill_plugin_dir(files_dir);
    let manifest_dir = plugin.join(".claude-plugin");
    let skills_dir = plugin.join("skills");
    ensure_private_dir(files_dir)?;
    ensure_private_dir(&plugin)?;
    ensure_private_dir(&manifest_dir)?;
    ensure_private_dir(&skills_dir)?;
    write_private(
        manifest_dir.join("plugin.json"),
        plugin_manifest().to_string().as_bytes(),
    )?;
    for skill in skills {
        let link = skills_dir.join(&skill.name);
        if fs::symlink_metadata(&link).is_ok() {
            fs::remove_file(&link)?;
        }
        std::os::unix::fs::symlink(&skill.dir, &link)?;
    }
    Ok(Some(plugin))
}

pub fn read_claude_config(config_dir: Option<&Path>) -> serde_json::Value {
    let Some(path) = dirs::home_dir().map(|home| claude_json(config_dir, &home)) else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn skill_plugin_lives_inside_store_files_dir_with_private_modes() {
        let root = tempfile::tempdir().unwrap();
        let files_dir = root.path().join("chat-42");
        let skill_dir = root.path().join("demo-skill");
        fs::create_dir(&skill_dir).unwrap();
        let skills = vec![ChatSkill {
            name: "demo".to_string(),
            dir: skill_dir.clone(),
            skill_md: "body".to_string(),
        }];
        let plugin = write_skill_plugin(&files_dir, &skills).unwrap().unwrap();
        assert_eq!(plugin, files_dir.join("plugin"));
        assert_eq!(
            fs::metadata(&plugin).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let manifest = plugin.join(".claude-plugin/plugin.json");
        assert_eq!(
            fs::metadata(&manifest).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_link(plugin.join("skills/demo")).unwrap(),
            skill_dir
        );
        // Rewriting over an existing plugin directory succeeds idempotently.
        assert_eq!(
            write_skill_plugin(&files_dir, &skills).unwrap(),
            Some(plugin)
        );
    }
}
