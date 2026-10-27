use std::path::Path;
use toml::{Table, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    Global,
    Store,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::Global => "global",
            Source::Store => "store",
        }
    }
}

pub fn merge_tables(global: Table, store: Table) -> Table {
    store.into_iter().fold(global, |mut merged, (key, value)| {
        let combined = match (merged.remove(&key), value) {
            (Some(Value::Table(base)), Value::Table(over)) => {
                Value::Table(merge_tables(base, over))
            }
            (_, over) => over,
        };
        merged.insert(key, combined);
        merged
    })
}

fn anchored(entry: &str, dir: &Path) -> String {
    let path = Path::new(entry);
    if path.is_absolute() || path.starts_with("~") {
        entry.to_string()
    } else {
        dir.join(path).to_string_lossy().into_owned()
    }
}

pub fn anchor_chat_paths(mut table: Table, dir: &Path) -> Table {
    let Some(chat) = table.get_mut("chat").and_then(Value::as_table_mut) else {
        return table;
    };
    for key in ["skills", "append_prompt_files"] {
        let entries = chat.get_mut(key).and_then(Value::as_array_mut);
        for entry in entries.into_iter().flatten() {
            if let Value::String(text) = entry {
                *text = anchored(text, dir);
            }
        }
    }
    if let Some(Value::String(text)) = chat.get_mut("prompt_file") {
        *text = anchored(text, dir);
    }
    table
}

const SECURITY_KEYS: [&str; 8] = [
    "chat.permission_mode",
    "chat.mcp_servers",
    "chat.user_settings",
    "chat.prompt_file",
    "chat.append_prompt_files",
    "chat.skills",
    "codex.permissions",
    "cursor.permissions",
];

pub fn security_keys(layer: &Table) -> Vec<&'static str> {
    SECURITY_KEYS
        .into_iter()
        .filter(|key| {
            let path: Vec<String> = key.split('.').map(str::to_string).collect();
            defines(layer, &path)
        })
        .collect()
}

fn defines(table: &Table, path: &[String]) -> bool {
    match path {
        [] => false,
        [key] => table.contains_key(key),
        [key, rest @ ..] => table
            .get(key)
            .and_then(Value::as_table)
            .is_some_and(|inner| defines(inner, rest)),
    }
}

pub fn leaf_sources(
    effective: &Table,
    global: &Table,
    store: &Table,
) -> Vec<(String, Value, Source)> {
    let mut rows = Vec::new();
    collect(effective, &[], (global, store), &mut rows);
    rows
}

fn collect(
    table: &Table,
    prefix: &[String],
    layers: (&Table, &Table),
    rows: &mut Vec<(String, Value, Source)>,
) {
    for (key, value) in table {
        let path: Vec<String> = prefix.iter().cloned().chain([key.clone()]).collect();
        match value {
            Value::Table(inner) => collect(inner, &path, layers, rows),
            leaf => {
                let source = if defines(layers.1, &path) {
                    Source::Store
                } else if defines(layers.0, &path) {
                    Source::Global
                } else {
                    Source::Default
                };
                rows.push((path.join("."), leaf.clone(), source));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        text.parse().unwrap()
    }

    #[test]
    fn merge_is_per_key_and_arrays_replace() {
        let merged = merge_tables(
            table("[chat]\nwrap_width = 80\nskills = [\"a\", \"b\"]\n[ask]\nx = 1\n"),
            table("[chat]\nwrap_width = 100\nskills = [\"c\"]\n"),
        );
        assert_eq!(
            merged,
            table("[chat]\nwrap_width = 100\nskills = [\"c\"]\n[ask]\nx = 1\n")
        );
    }

    #[test]
    fn store_scalar_replaces_global_table() {
        let merged = merge_tables(table("a.b = 1\n"), table("a = 2\n"));
        assert_eq!(merged, table("a = 2\n"));
    }

    #[test]
    fn relative_skills_anchor_to_the_file_directory() {
        let anchored = anchor_chat_paths(
            table("[chat]\nskills = [\"rel/x\", \"~/y\", \"/abs/z\"]\nworktree_dir = \"wt\"\n"),
            Path::new("/cfg"),
        );
        assert_eq!(
            anchored,
            table(
                "[chat]\nskills = [\"/cfg/rel/x\", \"~/y\", \"/abs/z\"]\nworktree_dir = \"wt\"\n"
            )
        );
    }

    #[test]
    fn prompt_files_anchor_like_skills() {
        let anchored = anchor_chat_paths(
            table("[chat]\nprompt_file = \"p.md\"\nappend_prompt_files = [\"a.md\", \"~/b.md\", \"/c.md\"]\n"),
            Path::new("/cfg"),
        );
        assert_eq!(
            anchored,
            table("[chat]\nprompt_file = \"/cfg/p.md\"\nappend_prompt_files = [\"/cfg/a.md\", \"~/b.md\", \"/c.md\"]\n")
        );
        assert_eq!(
            anchor_chat_paths(table("x = 1\n"), Path::new("/cfg")),
            table("x = 1\n")
        );
    }

    #[test]
    fn security_keys_finds_each_listed_key() {
        for key in SECURITY_KEYS {
            let (section, leaf) = key.split_once('.').unwrap();
            let layer = table(&format!("[{section}]\n{leaf} = 1\n"));
            assert_eq!(security_keys(&layer), [key]);
        }
    }

    #[test]
    fn security_keys_ignores_other_keys_and_empty_tables() {
        assert!(security_keys(&Table::new()).is_empty());
        let layer = table("theme = 1\n[chat]\nwrap_width = 1\n[codex]\nmodel = \"x\"\n");
        assert!(security_keys(&layer).is_empty());
    }

    #[test]
    fn leaf_sources_name_the_winning_layer() {
        let global = table("[chat]\nwrap_width = 80\nworker_base = \"main\"\n");
        let store = table("[chat]\nwrap_width = 100\n");
        let mut effective = merge_tables(global.clone(), store.clone());
        effective
            .get_mut("chat")
            .and_then(Value::as_table_mut)
            .unwrap()
            .insert("inbox_alert_sound".into(), Value::Boolean(true));
        let sources: Vec<_> = leaf_sources(&effective, &global, &store)
            .into_iter()
            .map(|(key, _, source)| (key, source))
            .collect();
        assert_eq!(
            sources,
            [
                ("chat.inbox_alert_sound".to_string(), Source::Default),
                ("chat.worker_base".to_string(), Source::Global),
                ("chat.wrap_width".to_string(), Source::Store),
            ]
        );
    }
}
