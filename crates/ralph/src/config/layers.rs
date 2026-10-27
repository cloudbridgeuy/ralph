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

pub fn anchor_skills(mut table: Table, dir: &Path) -> Table {
    let skills = table
        .get_mut("chat")
        .and_then(Value::as_table_mut)
        .and_then(|chat| chat.get_mut("skills"))
        .and_then(Value::as_array_mut);
    for entry in skills.into_iter().flatten() {
        if let Value::String(text) = entry {
            *text = anchored(text, dir);
        }
    }
    table
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
        let anchored = anchor_skills(
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
