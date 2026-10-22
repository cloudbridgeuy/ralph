use super::is_cursor_model;
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

pub struct CursorModelSources<'a> {
    pub spawn_model: Option<&'a str>,
    pub config_model: Result<Option<&'a str>, &'a str>,
    pub spawn_effort: Option<&'a str>,
    pub config_effort: Result<Option<&'a str>, &'a str>,
    pub spawn_fast: Option<bool>,
    pub config_fast: Result<Option<bool>, &'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCursorModel {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub fast: Option<bool>,
}

pub struct AdvertisedCursorModel<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

pub struct CursorVariantRequest<'a> {
    pub model: &'a str,
    pub reasoning_effort: Option<&'a str>,
    pub fast: Option<bool>,
}

pub fn resolve_cursor_model(
    sources: &CursorModelSources<'_>,
) -> Result<(ResolvedCursorModel, Option<String>), String> {
    let mut warnings = Vec::new();
    let (model, effort, fast) = match sources.config_model {
        Err(error) => {
            if sources.spawn_model.is_none() {
                warnings.push(format!(
                    "Warning: [cursor] model was skipped because the ralph config did not load ({error}). Used cursor's default model instead."
                ));
            }
            (
                sources.spawn_model.map(str::to_string),
                spawn_effort(sources.spawn_effort)?,
                sources.spawn_fast,
            )
        }
        Ok(config_model) => {
            let model = if let Some(model) = sources.spawn_model {
                Some(model.to_string())
            } else {
                match config_model {
                    Some(model) if is_cursor_model(model) => Some(model.to_string()),
                    Some(model) => {
                        warnings.push(format!(
                            "[cursor] model '{model}' must be non-empty with no whitespace; using cursor's default model."
                        ));
                        None
                    }
                    None => None,
                }
            };
            let effort = match sources.spawn_effort {
                Some(effort) => spawn_effort(Some(effort))?,
                None => match sources.config_effort.ok().flatten() {
                    Some(effort) if valid_effort(effort) => Some(effort.to_string()),
                    Some(effort) => {
                        warnings.push(format!(
                            "[cursor] reasoning_effort '{effort}' must be non-empty with no whitespace; ignoring it."
                        ));
                        None
                    }
                    None => None,
                },
            };
            (
                model,
                effort,
                sources.spawn_fast.or(sources.config_fast.ok().flatten()),
            )
        }
    };
    finish(model, effort, fast, warnings)
}

/// Picks the ACP `model` option value for a request.
/// An exact advertised value is kept when effort and fast are unset.
/// A short name selects among the advertised values of that name.
/// Unset effort and fast pick the cheapest advertised value: `fast=false`, then no `fast`,
/// then `fast=true`, then the lowest effort (`none`, `minimal`, `low`, `medium`, `high`,
/// `xhigh`, `max`). A requested parameter value that the model does not advertise is an error.
pub fn select_cursor_model(
    request: &CursorVariantRequest<'_>,
    choices: &[AdvertisedCursorModel<'_>],
) -> Result<String, String> {
    if request.reasoning_effort.is_none() && request.fast.is_none() {
        if let Some(choice) = choices.iter().find(|choice| choice.value == request.model) {
            return Ok(choice.value.to_string());
        }
    }
    let Some(name) = model_name(request.model, choices) else {
        if request.reasoning_effort.is_some() || request.fast.is_some() {
            return Err(
                "model is not offered, so reasoning_effort and fast cannot be applied".to_string(),
            );
        }
        return Ok(request.model.to_string());
    };
    let variants: Vec<Variant> = choices
        .iter()
        .filter(|choice| choice.name == name)
        .map(|choice| Variant {
            value: choice.value,
            params: parse_params(choice.value),
        })
        .collect();
    let mut indexes: Vec<usize> = (0..variants.len()).collect();
    if let Some(effort) = request.reasoning_effort {
        indexes = require_param(&variants, &indexes, "reasoning_effort", effort)?;
    }
    if let Some(fast) = request.fast {
        let want = if fast { "true" } else { "false" };
        indexes = require_param(&variants, &indexes, "fast", want)?;
    }
    let picked = indexes
        .into_iter()
        .min_by_key(|index| {
            let variant = &variants[*index];
            (
                fast_rank(&variant.params),
                effort_rank(&variant.params),
                *index,
            )
        })
        .map(|index| variants[index].value.to_string());
    picked.ok_or_else(|| format!("model '{name}' has no advertised value"))
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

fn spawn_effort(effort: Option<&str>) -> Result<Option<String>, String> {
    match effort {
        None => Ok(None),
        Some(effort) if valid_effort(effort) => Ok(Some(effort.to_string())),
        Some(effort) => Err(format!(
            "reasoning_effort '{effort}' must be non-empty with no whitespace."
        )),
    }
}

fn valid_effort(effort: &str) -> bool {
    !effort.is_empty() && !effort.chars().any(char::is_whitespace)
}

fn finish(
    model: Option<String>,
    mut effort: Option<String>,
    mut fast: Option<bool>,
    mut warnings: Vec<String>,
) -> Result<(ResolvedCursorModel, Option<String>), String> {
    if model.is_none() && (effort.is_some() || fast.is_some()) {
        warnings.push(
            "[cursor] reasoning_effort and fast were skipped because no model is set.".to_string(),
        );
        effort = None;
        fast = None;
    }
    let warning = if warnings.is_empty() {
        None
    } else {
        Some(warnings.join(" "))
    };
    Ok((
        ResolvedCursorModel {
            model,
            reasoning_effort: effort,
            fast,
        },
        warning,
    ))
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

fn require_param(
    variants: &[Variant],
    indexes: &[usize],
    key: &str,
    want: &str,
) -> Result<Vec<usize>, String> {
    let mut offered = Vec::new();
    for index in indexes {
        if let Some((_, value)) = variants[*index].params.iter().find(|(name, _)| name == key) {
            if !offered.iter().any(|seen| seen == value) {
                offered.push(value.clone());
            }
        }
    }
    if offered.is_empty() {
        let mut keys = Vec::new();
        for index in indexes {
            for (name, _) in &variants[*index].params {
                if !keys.iter().any(|seen| seen == name) {
                    keys.push(name.clone());
                }
            }
        }
        let keys = if keys.is_empty() {
            "none".to_string()
        } else {
            keys.join(", ")
        };
        return Err(format!("{key} is not offered; offered parameters: {keys}"));
    }
    let matched: Vec<usize> = indexes
        .iter()
        .copied()
        .filter(|index| {
            variants[*index]
                .params
                .iter()
                .any(|(name, value)| name == key && value == want)
        })
        .collect();
    if matched.is_empty() {
        return Err(format!(
            "{key} '{want}' is not offered; offered: {}",
            offered.join(", ")
        ));
    }
    Ok(matched)
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

    fn sources<'a>(
        spawn_model: Option<&'a str>,
        config_model: Result<Option<&'a str>, &'a str>,
    ) -> CursorModelSources<'a> {
        CursorModelSources {
            spawn_model,
            config_model,
            spawn_effort: None,
            config_effort: Ok(None),
            spawn_fast: None,
            config_fast: Ok(None),
        }
    }

    #[test]
    fn cursor_model_resolution_prefers_spawn_then_config_then_default() {
        let (resolved, warning) =
            resolve_cursor_model(&sources(Some("gpt-5"), Ok(Some("sonnet-4")))).unwrap();
        assert_eq!(resolved.model.as_deref(), Some("gpt-5"));
        assert_eq!(warning, None);
        let (resolved, _) = resolve_cursor_model(&sources(None, Ok(Some("sonnet-4")))).unwrap();
        assert_eq!(resolved.model.as_deref(), Some("sonnet-4"));
        let (resolved, warning) = resolve_cursor_model(&sources(None, Ok(None))).unwrap();
        assert_eq!(resolved, empty());
        assert_eq!(warning, None);
        let (resolved, _) = resolve_cursor_model(&sources(
            None,
            Ok(Some("claude-opus-4-8[context=1m,effort=high,fast=false]")),
        ))
        .unwrap();
        assert_eq!(
            resolved.model.as_deref(),
            Some("claude-opus-4-8[context=1m,effort=high,fast=false]")
        );
    }

    #[test]
    fn cursor_model_resolution_warns_on_invalid_config() {
        for bad in ["", "two words", " gpt-5"] {
            let (resolved, warning) = resolve_cursor_model(&sources(None, Ok(Some(bad)))).unwrap();
            assert_eq!(resolved.model, None);
            assert!(warning.unwrap().contains("[cursor] model"), "{bad}");
        }
        let (resolved, warning) =
            resolve_cursor_model(&sources(Some("gpt-5"), Ok(Some("")))).unwrap();
        assert_eq!(resolved.model.as_deref(), Some("gpt-5"));
        assert_eq!(warning, None);
    }

    #[test]
    fn cursor_model_resolution_warns_when_config_fails_to_load() {
        let (resolved, warning) =
            resolve_cursor_model(&sources(None, Err("bad toml at line 3"))).unwrap();
        assert_eq!(resolved, empty());
        let warning = warning.unwrap();
        assert!(warning.contains("bad toml at line 3"));
        assert!(warning.contains("default model"));
    }

    #[test]
    fn cursor_model_resolution_ignores_config_error_when_spawn_model_given() {
        let (resolved, warning) =
            resolve_cursor_model(&sources(Some("gpt-5"), Err("boom"))).unwrap();
        assert_eq!(resolved.model.as_deref(), Some("gpt-5"));
        assert_eq!(warning, None);
    }

    #[test]
    fn cursor_effort_and_fast_prefer_spawn_then_config() {
        let mut input = sources(Some("grok-4.7"), Ok(Some("grok-4.7")));
        input.spawn_effort = Some("low");
        input.config_effort = Ok(Some("high"));
        input.spawn_fast = Some(false);
        input.config_fast = Ok(Some(true));
        let (resolved, warning) = resolve_cursor_model(&input).unwrap();
        assert_eq!(resolved.reasoning_effort.as_deref(), Some("low"));
        assert_eq!(resolved.fast, Some(false));
        assert_eq!(warning, None);

        input.spawn_effort = None;
        input.spawn_fast = None;
        let (resolved, _) = resolve_cursor_model(&input).unwrap();
        assert_eq!(resolved.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(resolved.fast, Some(true));
    }

    #[test]
    fn cursor_effort_without_a_model_is_skipped() {
        let mut input = sources(None, Ok(None));
        input.spawn_effort = Some("low");
        input.spawn_fast = Some(false);
        let (resolved, warning) = resolve_cursor_model(&input).unwrap();
        assert_eq!(resolved, empty());
        assert!(warning.unwrap().contains("no model is set"));
    }

    #[test]
    fn cursor_config_effort_with_whitespace_warns() {
        let mut input = sources(Some("grok-4.7"), Ok(None));
        input.config_effort = Ok(Some("low effort"));
        let (resolved, warning) = resolve_cursor_model(&input).unwrap();
        assert_eq!(resolved.reasoning_effort, None);
        assert!(warning.unwrap().contains("reasoning_effort"));
    }

    #[test]
    fn spawn_effort_with_whitespace_is_an_error() {
        let mut input = sources(Some("grok-4.7"), Ok(None));
        input.spawn_effort = Some("low effort");
        assert!(resolve_cursor_model(&input).is_err());
    }

    #[test]
    fn select_keeps_an_exact_value_and_otherwise_the_cheapest_advertised_one() {
        let choices = advertised();
        assert_eq!(
            select_cursor_model(
                &request(
                    "grok-4.7[context=256k,reasoning_effort=high,fast=true]",
                    None,
                    None
                ),
                &choices
            )
            .unwrap(),
            "grok-4.7[context=256k,reasoning_effort=high,fast=true]"
        );
        assert_eq!(
            select_cursor_model(&request("grok-4.7", None, None), &choices).unwrap(),
            "grok-4.7[context=256k,reasoning_effort=low,fast=false]"
        );
        assert_eq!(
            select_cursor_model(&request("not-a-model", None, None), &choices).unwrap(),
            "not-a-model"
        );
    }

    #[test]
    fn select_applies_offered_parameters_and_refuses_the_rest() {
        let choices = advertised();
        assert_eq!(
            select_cursor_model(&request("grok-4.7", Some("high"), Some(true)), &choices).unwrap(),
            "grok-4.7[context=256k,reasoning_effort=high,fast=true]"
        );
        let missing = select_cursor_model(&request("grok-4.7", Some("max"), Some(false)), &choices);
        assert_eq!(
            missing.unwrap_err(),
            "reasoning_effort 'max' is not offered; offered: high, low"
        );
        let no_fast = [AdvertisedCursorModel {
            name: "gemini-4-argon",
            value: "gemini-4-argon[reasoning_effort=medium]",
        }];
        let refused = select_cursor_model(&request("gemini-4-argon", None, Some(false)), &no_fast);
        assert_eq!(
            refused.unwrap_err(),
            "fast is not offered; offered parameters: reasoning_effort"
        );
        let unknown = select_cursor_model(&request("nope", Some("low"), None), &choices);
        assert!(unknown.unwrap_err().contains("not offered"));
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

    fn empty() -> ResolvedCursorModel {
        ResolvedCursorModel {
            model: None,
            reasoning_effort: None,
            fast: None,
        }
    }

    fn request<'a>(
        model: &'a str,
        reasoning_effort: Option<&'a str>,
        fast: Option<bool>,
    ) -> CursorVariantRequest<'a> {
        CursorVariantRequest {
            model,
            reasoning_effort,
            fast,
        }
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
