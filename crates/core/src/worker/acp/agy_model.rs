pub fn agy_model_base<'a>(model: &'a str, session: &'a serde_json::Value) -> &'a str {
    options(session, "model")
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item["value"].as_str())
        .filter(|base| {
            *base == model
                || model
                    .strip_prefix(base)
                    .is_some_and(|suffix| suffix.starts_with('-'))
        })
        .max_by_key(|base| base.len())
        .unwrap_or(model)
}

fn options<'a>(session: &'a serde_json::Value, id: &str) -> Option<&'a [serde_json::Value]> {
    session["configOptions"]
        .as_array()?
        .iter()
        .find(|item| item["id"] == id)?["options"]
        .as_array()
        .map(Vec::as_slice)
}

pub fn agy_selection(
    model: Option<&str>,
    effort: Option<&str>,
    session: &serde_json::Value,
) -> Result<Option<String>, String> {
    let Some(model) = model else { return Ok(None) };
    let ids: Vec<&str> = session["models"]["availableModels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["modelId"].as_str())
        .collect();
    let chosen = if ids.contains(&model) {
        model
    } else if options(session, "model")
        .is_some_and(|items| items.iter().any(|item| item["value"] == model))
    {
        let choices: Vec<&str> = ids
            .iter()
            .copied()
            .filter(|id| {
                id.strip_prefix(model)
                    .is_some_and(|tail| tail.starts_with('-'))
            })
            .filter(|id| {
                effort.is_none_or(|effort| {
                    id.strip_prefix(model)
                        .and_then(|tail| tail.strip_prefix('-'))
                        == Some(effort)
                })
            })
            .collect();
        match choices.as_slice() {
            [only] => *only,
            _ => {
                return Err(format!(
                    "agy model '{model}' requires an advertised choice: {}",
                    choices.join(", ")
                ))
            }
        }
    } else {
        return Err(format!("agy did not advertise model '{model}'"));
    };
    let base = agy_model_base(chosen, session);
    if let (Some(effort), Some(suffix)) = (
        effort,
        chosen
            .strip_prefix(base)
            .and_then(|tail| tail.strip_prefix('-')),
    ) {
        if ["low", "medium", "high", "xhigh"].contains(&suffix) && suffix != effort {
            return Err(format!(
                "agy model '{chosen}' conflicts with effort '{effort}'"
            ));
        }
    }
    Ok(Some(chosen.to_string()))
}

pub fn agy_effort_available(
    effort: Option<&str>,
    session: &serde_json::Value,
) -> Result<(), String> {
    let Some(effort) = effort else { return Ok(()) };
    if options(session, "thought_level").is_some_and(|items| {
        items.iter().any(|item| {
            item["value"] == effort
                || item["options"]
                    .as_array()
                    .is_some_and(|group| group.iter().any(|choice| choice["value"] == effort))
        })
    }) {
        Ok(())
    } else {
        Err(format!(
            "agy does not advertise effort '{effort}' for this model"
        ))
    }
}

pub fn agy_effort_confirmed(session: &serde_json::Value, model: &str, effort: &str) -> bool {
    session["configOptions"].as_array().is_some_and(|items| {
        [("model", model), ("thought_level", effort)]
            .into_iter()
            .all(|(id, value)| {
                items
                    .iter()
                    .any(|item| item["id"] == id && item["currentValue"] == value)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_settings_validate_advertised_model_and_effort() {
        let session = serde_json::json!({"models": {"availableModels": [{"modelId": "gemini-high"}]}, "configOptions": [{"id": "model", "options": [{"value": "gemini"}]}, {"id": "thought_level", "options": [{"value": "high"}]}]});
        assert_eq!(
            agy_selection(Some("gemini-high"), None, &session),
            Ok(Some("gemini-high".to_string()))
        );
        assert!(agy_selection(Some("unknown"), None, &session).is_err());
    }

    #[test]
    fn family_requires_a_unique_advertised_slug() {
        let session = serde_json::json!({"models":{"availableModels":[{"modelId":"flash-low"},{"modelId":"flash-high"}]},"configOptions":[{"id":"model","options":[{"value":"flash"}]}]});
        assert!(agy_selection(Some("flash"), None, &session).is_err());
    }

    #[test]
    fn effort_selects_a_unique_slug_and_rejects_conflicts() {
        let session = serde_json::json!({"models":{"availableModels":[{"modelId":"flash-low"},{"modelId":"flash-high"}]},"configOptions":[{"id":"model","options":[{"value":"flash"}]}]});
        assert_eq!(
            agy_selection(Some("flash"), Some("high"), &session),
            Ok(Some("flash-high".to_string()))
        );
        assert!(agy_selection(Some("flash-low"), Some("high"), &session).is_err());
    }

    #[test]
    fn effort_must_be_advertised_by_current_model() {
        let options = serde_json::json!({"configOptions":[{"id":"thought_level","options":[{"value":"low"}]}]});
        assert!(agy_effort_available(Some("high"), &options).is_err());
        assert!(agy_effort_available(Some("low"), &options).is_ok());
    }

    #[test]
    fn effort_readback_confirms_both_model_and_level() {
        let state = serde_json::json!({"configOptions":[{"id":"model","currentValue":"flash"},{"id":"thought_level","currentValue":"high"}]});
        assert!(agy_effort_confirmed(&state, "flash", "high"));
        assert!(!agy_effort_confirmed(&state, "flash", "low"));
        assert!(!agy_effort_confirmed(&state, "other", "high"));
    }
}
