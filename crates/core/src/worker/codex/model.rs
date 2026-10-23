use serde_json::{json, Value};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub model: Option<String>,
    pub effort: Option<String>,
}

fn present(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn resolve_selection(
    selected_model: Option<&str>,
    config_model: Option<&str>,
    config_effort: Option<&str>,
) -> Selection {
    Selection {
        model: present(selected_model).or_else(|| present(config_model)),
        effort: present(config_effort),
    }
}

fn entry_names(entry: &Value) -> impl Iterator<Item = &str> {
    [entry["id"].as_str(), entry["model"].as_str()]
        .into_iter()
        .flatten()
}

fn supported_efforts(entry: &Value) -> Vec<String> {
    entry["supportedReasoningEfforts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|effort| effort["reasoningEffort"].as_str())
        .map(str::to_string)
        .collect()
}

pub fn check_effort(selection: &Selection, list: &Value) -> Result<(), String> {
    let Some(effort) = selection.effort.as_deref() else {
        return Ok(());
    };
    let entries = list["data"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let entry = match selection.model.as_deref() {
        Some(model) => entries
            .iter()
            .find(|entry| entry_names(entry).any(|name| name == model)),
        None => entries
            .iter()
            .find(|entry| entry["isDefault"] == json!(true)),
    };
    let Some(entry) = entry else {
        return Err(format!(
            "codex lists no model to check effort '{effort}' against"
        ));
    };
    let supported = supported_efforts(entry);
    if supported.iter().any(|candidate| candidate == effort) {
        return Ok(());
    }
    let name = entry_names(entry).next().unwrap_or("the default model");
    Err(format!(
        "codex model '{name}' does not support effort '{effort}'; supported: {}",
        supported.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Value {
        json!({"data": [
            {"id": "big", "model": "big", "isDefault": true, "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "high"}]},
            {"id": "small", "model": "small", "isDefault": false, "supportedReasoningEfforts": [{"reasoningEffort": "minimal"}]}
        ], "nextCursor": null})
    }

    fn pick(model: Option<&str>, effort: Option<&str>) -> Selection {
        Selection {
            model: model.map(str::to_string),
            effort: effort.map(str::to_string),
        }
    }

    #[test]
    fn worker_model_beats_config_model_and_blank_values_are_unset() {
        let chosen = resolve_selection(Some("a"), Some("b"), Some("high"));
        assert_eq!(chosen, pick(Some("a"), Some("high")));
        assert_eq!(
            resolve_selection(None, Some("b"), None),
            pick(Some("b"), None)
        );
        assert_eq!(
            resolve_selection(Some(" "), Some(""), Some(" ")),
            Selection::default()
        );
    }

    #[test]
    fn effort_is_checked_against_the_effective_model() {
        assert_eq!(check_effort(&pick(None, None), &list()), Ok(()));
        assert_eq!(check_effort(&pick(None, Some("high")), &list()), Ok(()));
        assert_eq!(
            check_effort(&pick(Some("small"), Some("minimal")), &list()),
            Ok(())
        );
        let error = check_effort(&pick(Some("small"), Some("high")), &list()).unwrap_err();
        assert!(error.contains("'small'") && error.contains("'high'") && error.contains("minimal"));
        let default = check_effort(&pick(None, Some("max")), &list()).unwrap_err();
        assert!(default.contains("'big'") && default.contains("low, high"));
        assert!(check_effort(&pick(None, Some("low")), &json!({"data": []})).is_err());
    }
}
