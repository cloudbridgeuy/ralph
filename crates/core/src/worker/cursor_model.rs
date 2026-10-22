use super::is_cursor_model;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Keys in `~/.cursor/cli-config.json` that a model `session/set_config_option` rewrites.
pub const CURSOR_CLI_DEFAULT_KEYS: &[&str] = &[
    "model",
    "selectedModel",
    "modelParameters",
    "hasChangedDefaultModel",
    "modelSelectionHistory",
    "maxMode",
];

/// Key in `~/.cursor/acp-config.json` that the same set rewrites.
pub const CURSOR_ACP_DEFAULT_KEYS: &[&str] = &["selectedModelVariantId"];

/// Model keys saved before `session/set_config_option`, so a crash can be repaired.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CursorDefaultSnapshot {
    pub cli: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp: Option<Value>,
}

/// What the next start can do about a crash between the set and the restore.
#[derive(Debug, Clone, PartialEq)]
pub enum CursorDefaultCrash {
    /// A snapshot was saved before the set, so the saved keys can be put back.
    Repairable(CursorDefaultSnapshot),
    /// No snapshot was saved, so the files cannot be told from a change the user made.
    Undetectable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepairedCursorDefault {
    pub cli: Value,
    /// `None` when `acp-config.json` was absent before the set and should stay absent.
    pub acp: Option<Value>,
}

pub struct AdvertisedCursorModel<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

pub fn resolve_cursor_model(
    spawn_model: Option<&str>,
    config_model: Result<Option<&str>, &str>,
) -> (Option<String>, Option<String>) {
    match config_model {
        Err(error) => {
            if spawn_model.is_none() {
                return (
                    None,
                    Some(format!(
                        "Warning: [cursor] model was skipped because the ralph config did not load ({error}). Used cursor's default model instead."
                    )),
                );
            }
            (spawn_model.map(str::to_string), None)
        }
        Ok(config_model) => {
            if let Some(model) = spawn_model {
                return (Some(model.to_string()), None);
            }
            match config_model {
                Some(model) if is_cursor_model(model) => (Some(model.to_string()), None),
                Some(model) => (
                    None,
                    Some(format!(
                        "[cursor] model '{model}' must be non-empty with no whitespace; using cursor's default model."
                    )),
                ),
                None => (None, None),
            }
        }
    }
}

/// Picks the ACP `model` option value for a requested id.
/// An exact advertised value is kept. A short name selects among the advertised
/// values of that name. Cursor's ACP lists one value per name, so that value is
/// the one sent. If a name is listed more than once, the cheapest wins:
/// `fast=false`, then no `fast`, then `fast=true`, then the lowest effort
/// (`none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`). Any other
/// string is returned unchanged so the agent can reject it.
pub fn select_cursor_model(requested: &str, choices: &[AdvertisedCursorModel<'_>]) -> String {
    if choices.iter().any(|choice| choice.value == requested) {
        return requested.to_string();
    }
    let Some(name) = model_name(requested, choices) else {
        return requested.to_string();
    };
    let variants: Vec<Variant<'_>> = choices
        .iter()
        .filter(|choice| choice.name == name)
        .map(|choice| Variant {
            value: choice.value,
            params: parse_params(choice.value),
        })
        .collect();
    variants
        .iter()
        .enumerate()
        .min_by_key(|(index, variant)| {
            (
                fast_rank(&variant.params),
                effort_rank(&variant.params),
                *index,
            )
        })
        .map(|(_, variant)| variant.value.to_string())
        .unwrap_or_else(|| requested.to_string())
}

/// Copies `keys` from `before` onto `after`. A key missing from `before` is removed.
pub fn restore_json_keys(before: &Value, after: &Value, keys: &[&str]) -> Result<Value, String> {
    let (Some(before), Some(after)) = (before.as_object(), after.as_object()) else {
        return Err("cursor config was not a JSON object".to_string());
    };
    let mut restored = after.clone();
    for key in keys {
        match before.get(*key) {
            Some(value) => {
                restored.insert((*key).to_string(), value.clone());
            }
            None => {
                restored.remove(*key);
            }
        }
    }
    Ok(Value::Object(restored))
}

/// Keeps only the model-default keys. Other keys, including secrets, are dropped.
pub fn cursor_default_snapshot(
    cli: &Value,
    acp: Option<&Value>,
) -> Result<CursorDefaultSnapshot, String> {
    Ok(CursorDefaultSnapshot {
        cli: project_keys(cli, CURSOR_CLI_DEFAULT_KEYS)?,
        acp: acp
            .map(|value| project_keys(value, CURSOR_ACP_DEFAULT_KEYS))
            .transpose()?,
    })
}

pub fn cursor_default_crash(snapshot: Option<CursorDefaultSnapshot>) -> CursorDefaultCrash {
    match snapshot {
        Some(snapshot) => CursorDefaultCrash::Repairable(snapshot),
        None => CursorDefaultCrash::Undetectable,
    }
}

/// Puts the saved model keys back onto the files as a crash left them.
pub fn repair_cursor_default(
    snapshot: &CursorDefaultSnapshot,
    cli_now: &Value,
    acp_now: Option<&Value>,
) -> Result<RepairedCursorDefault, String> {
    let cli = restore_json_keys(&snapshot.cli, cli_now, CURSOR_CLI_DEFAULT_KEYS)?;
    let acp = match &snapshot.acp {
        Some(before) => Some(restore_json_keys(
            before,
            acp_now.unwrap_or(&serde_json::json!({})),
            CURSOR_ACP_DEFAULT_KEYS,
        )?),
        None => acp_now
            .map(|now| restore_json_keys(&serde_json::json!({}), now, CURSOR_ACP_DEFAULT_KEYS))
            .transpose()?,
    };
    Ok(RepairedCursorDefault { cli, acp })
}

fn project_keys(document: &Value, keys: &[&str]) -> Result<Value, String> {
    let Some(object) = document.as_object() else {
        return Err("cursor config was not a JSON object".to_string());
    };
    let mut kept = serde_json::Map::new();
    for key in keys {
        if let Some(value) = object.get(*key) {
            kept.insert((*key).to_string(), value.clone());
        }
    }
    Ok(Value::Object(kept))
}

struct Variant<'a> {
    value: &'a str,
    params: Vec<(String, String)>,
}

fn model_name<'a>(requested: &str, choices: &[AdvertisedCursorModel<'a>]) -> Option<&'a str> {
    if let Some(choice) = choices.iter().find(|choice| choice.name == requested) {
        return Some(choice.name);
    }
    choices
        .iter()
        .find(|choice| choice.value == requested || param_id(choice.value) == requested)
        .map(|choice| choice.name)
}

fn param_id(value: &str) -> &str {
    value.split_once('[').map(|(id, _)| id).unwrap_or(value)
}

fn parse_params(value: &str) -> Vec<(String, String)> {
    let Some((_, rest)) = value.split_once('[') else {
        return Vec::new();
    };
    let body = rest.strip_suffix(']').unwrap_or(rest);
    if body.is_empty() {
        return Vec::new();
    }
    body.split(',')
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn fast_rank(params: &[(String, String)]) -> u8 {
    match params
        .iter()
        .find(|(key, _)| key == "fast")
        .map(|(_, value)| value.as_str())
    {
        Some("false") => 0,
        None => 1,
        Some("true") => 2,
        Some(_) => 3,
    }
}

fn effort_rank(params: &[(String, String)]) -> u8 {
    const ORDER: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
    for key in ["reasoning_effort", "effort", "reasoning"] {
        if let Some((_, value)) = params.iter().find(|(name, _)| name == key) {
            return ORDER
                .iter()
                .position(|name| *name == value)
                .map(|index| index as u8)
                .unwrap_or(7);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cursor_model_resolution_prefers_spawn_then_config_then_default() {
        let (model, warning) = resolve_cursor_model(Some("gpt-5"), Ok(Some("sonnet-4")));
        assert_eq!(model.as_deref(), Some("gpt-5"));
        assert_eq!(warning, None);
        let (model, _) = resolve_cursor_model(None, Ok(Some("sonnet-4")));
        assert_eq!(model.as_deref(), Some("sonnet-4"));
        let (model, warning) = resolve_cursor_model(None, Ok(None));
        assert_eq!(model, None);
        assert_eq!(warning, None);
        let (model, _) = resolve_cursor_model(
            None,
            Ok(Some("claude-opus-4-8[context=1m,effort=high,fast=false]")),
        );
        assert_eq!(
            model.as_deref(),
            Some("claude-opus-4-8[context=1m,effort=high,fast=false]")
        );
    }

    #[test]
    fn cursor_model_resolution_warns_on_invalid_config() {
        for bad in ["", "two words", " gpt-5"] {
            let (model, warning) = resolve_cursor_model(None, Ok(Some(bad)));
            assert_eq!(model, None);
            assert!(warning.unwrap().contains("[cursor] model"), "{bad}");
        }
        let (model, warning) = resolve_cursor_model(Some("gpt-5"), Ok(Some("")));
        assert_eq!(model.as_deref(), Some("gpt-5"));
        assert_eq!(warning, None);
    }

    #[test]
    fn cursor_model_resolution_warns_when_config_fails_to_load() {
        let (model, warning) = resolve_cursor_model(None, Err("bad toml at line 3"));
        assert_eq!(model, None);
        let warning = warning.unwrap();
        assert!(warning.contains("bad toml at line 3"));
        assert!(warning.contains("default model"));
    }

    #[test]
    fn cursor_model_resolution_ignores_config_error_when_spawn_model_given() {
        let (model, warning) = resolve_cursor_model(Some("gpt-5"), Err("boom"));
        assert_eq!(model.as_deref(), Some("gpt-5"));
        assert_eq!(warning, None);
    }

    #[test]
    fn select_keeps_an_exact_value_and_otherwise_the_cheapest_advertised_one() {
        let choices = advertised();
        assert_eq!(
            select_cursor_model(
                "grok-4.7[context=256k,reasoning_effort=high,fast=true]",
                &choices
            ),
            "grok-4.7[context=256k,reasoning_effort=high,fast=true]"
        );
        assert_eq!(
            select_cursor_model("grok-4.7", &choices),
            "grok-4.7[context=256k,reasoning_effort=low,fast=false]"
        );
        assert_eq!(select_cursor_model("not-a-model", &choices), "not-a-model");
    }

    #[test]
    fn restore_json_keys_puts_the_model_default_back() {
        let before = json!({
            "permissions": {"allow": ["Shell(ls)"]},
            "selectedModel": {"modelId": "grok-4.7"},
            "modelSelectionHistory": ["grok-4.7"]
        });
        let after = json!({
            "permissions": {"allow": ["Shell(ls)", "Shell(echo)"]},
            "selectedModel": {"modelId": "composer-2.5"},
            "modelSelectionHistory": ["composer-2.5", "grok-4.7"],
            "selectedModelVariantId": "composer-2.5[fast=true]"
        });
        let restored = restore_json_keys(&before, &after, CURSOR_CLI_DEFAULT_KEYS).unwrap();
        assert_eq!(restored["selectedModel"]["modelId"], "grok-4.7");
        assert_eq!(restored["modelSelectionHistory"][0], "grok-4.7");
        assert_eq!(restored["permissions"]["allow"][1], "Shell(echo)");
        assert_eq!(
            restored["selectedModelVariantId"],
            "composer-2.5[fast=true]"
        );
        assert!(restore_json_keys(&json!([]), &after, CURSOR_CLI_DEFAULT_KEYS).is_err());
    }

    #[test]
    fn a_missing_snapshot_cannot_be_told_from_a_user_change() {
        assert_eq!(cursor_default_crash(None), CursorDefaultCrash::Undetectable);
    }

    #[test]
    fn a_saved_snapshot_repairs_a_crash_between_set_and_restore() {
        let before_cli = json!({
            "permissions": {"allow": ["Shell(ls)"]},
            "authInfo": {"token": "secret"},
            "selectedModel": {"modelId": "grok-4.7", "parameters": [{"id": "fast", "value": "true"}]},
            "modelParameters": {"grok-4.7": [{"id": "fast", "value": "true"}]},
            "hasChangedDefaultModel": true
        });
        let before_acp = json!({
            "selectedModelVariantId": "grok-4.7[context=256k,reasoning_effort=high,fast=true]"
        });
        let snapshot = cursor_default_snapshot(&before_cli, Some(&before_acp)).unwrap();
        assert!(snapshot.cli.get("authInfo").is_none());
        assert!(snapshot.cli.get("permissions").is_none());
        let saved = serde_json::from_str::<CursorDefaultSnapshot>(
            &serde_json::to_string(&snapshot).unwrap(),
        )
        .unwrap();
        let CursorDefaultCrash::Repairable(saved) = cursor_default_crash(Some(saved)) else {
            panic!("a snapshot is repairable");
        };
        let cli_now = json!({
            "permissions": {"allow": ["Shell(ls)"]},
            "authInfo": {"token": "secret"},
            "selectedModel": {"modelId": "composer-2.5", "parameters": [{"id": "fast", "value": "true"}]},
            "modelParameters": {"composer-2.5": [{"id": "fast", "value": "true"}]},
            "hasChangedDefaultModel": true,
            "maxMode": true
        });
        let acp_now = json!({"selectedModelVariantId": "composer-2.5[fast=true]"});
        let repaired = repair_cursor_default(&saved, &cli_now, Some(&acp_now)).unwrap();
        assert_eq!(repaired.cli["selectedModel"]["modelId"], "grok-4.7");
        assert_eq!(
            repaired.cli["modelParameters"]["grok-4.7"][0]["value"],
            "true"
        );
        assert!(repaired.cli.get("maxMode").is_none());
        assert_eq!(repaired.cli["authInfo"]["token"], "secret");
        assert_eq!(
            repaired.acp.unwrap()["selectedModelVariantId"],
            "grok-4.7[context=256k,reasoning_effort=high,fast=true]"
        );
    }

    #[test]
    fn repair_leaves_an_absent_acp_file_absent() {
        let snapshot =
            cursor_default_snapshot(&json!({"selectedModel": {"modelId": "grok-4.7"}}), None)
                .unwrap();
        let repaired = repair_cursor_default(
            &snapshot,
            &json!({"selectedModel": {"modelId": "composer-2.5"}}),
            None,
        )
        .unwrap();
        assert_eq!(repaired.cli["selectedModel"]["modelId"], "grok-4.7");
        assert_eq!(repaired.acp, None);
    }

    fn advertised() -> [AdvertisedCursorModel<'static>; 3] {
        [
            AdvertisedCursorModel {
                name: "grok-4.7",
                value: "grok-4.7[context=256k,reasoning_effort=high,fast=true]",
            },
            AdvertisedCursorModel {
                name: "grok-4.7",
                value: "grok-4.7[context=256k,reasoning_effort=low,fast=false]",
            },
            AdvertisedCursorModel {
                name: "composer-2.5",
                value: "composer-2.5[fast=true]",
            },
        ]
    }

    #[test]
    fn restore_keeps_map_type() {
        let before = json!({});
        let after = json!({"selectedModel": {"modelId": "composer-2.5"}});
        let restored = restore_json_keys(&before, &after, &["selectedModel"]).unwrap();
        assert!(restored.get("selectedModel").is_none());
    }
}
