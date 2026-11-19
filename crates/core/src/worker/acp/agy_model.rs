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
    Ok(Some(chosen.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_settings_validate_advertised_model_and_effort() {
        let session = serde_json::json!({"models": {"availableModels": [{"modelId": "gemini-high"}]}, "configOptions": [{"id": "model", "options": [{"value": "gemini"}]}, {"id": "thought_level", "options": [{"value": "high"}]}]});
        assert_eq!(
            agy_selection(Some("gemini-high"), &session),
            Ok(Some("gemini-high".to_string()))
        );
        assert!(agy_selection(Some("unknown"), &session).is_err());
    }

    #[test]
    fn family_requires_a_unique_advertised_slug() {
        let session = serde_json::json!({"models":{"availableModels":[{"modelId":"flash-low"},{"modelId":"flash-high"}]},"configOptions":[{"id":"model","options":[{"value":"flash"}]}]});
        assert!(agy_selection(Some("flash"), &session).is_err());
    }
}
