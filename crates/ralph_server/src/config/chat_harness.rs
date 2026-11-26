use super::{AppConfig, ChatSection};
use ralph_core::chat_backend::MemoryHarness;
use ralph_core::chat_thread_model::{build_model_args, validate_thread_model};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatClaudeSection {
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatAgySection {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

pub fn default_chat_harness() -> String {
    "claude".to_string()
}

impl ChatSection {
    pub fn harness_kind(&self) -> MemoryHarness {
        MemoryHarness::parse(&self.harness).unwrap_or(MemoryHarness::Claude)
    }
}

fn model_error(name: &str, model: &str) -> Option<String> {
    (model.trim().is_empty()
        || model.trim_start().starts_with('-')
        || model.chars().any(char::is_control))
    .then(|| format!("{name} must be nonempty and must not start with an option"))
}

pub fn harness_error(chat: &ChatSection) -> Option<String> {
    let harness = MemoryHarness::parse(&chat.harness)
        .filter(|harness| matches!(harness, MemoryHarness::Claude | MemoryHarness::Agy));
    let Some(harness) = harness else {
        return Some("chat.harness must be claude or agy".to_string());
    };
    let claude = chat
        .claude
        .model
        .as_deref()
        .and_then(|model| model_error("chat.claude.model", model));
    if claude.is_some() || harness == MemoryHarness::Claude {
        return claude;
    }
    let Some(model) = chat.agy.model.as_deref() else {
        return Some("chat.agy.model must be set when chat.harness is agy".to_string());
    };
    model_error("chat.agy.model", model)
        .or_else(|| {
            matches!(model, "haiku" | "claude-haiku-5-5")
                .then(|| "chat.agy.model must be set to a Gemini model".to_string())
        })
        .or_else(|| {
            chat.agy
                .effort
                .as_deref()
                .filter(|effort| effort.is_empty() || effort.chars().any(char::is_whitespace))
                .map(|_| "chat.agy.effort must be non-empty with no whitespace".to_string())
        })
}

pub fn claude_model_args(chat: &ChatSection, model_args: Vec<String>) -> Vec<String> {
    if model_args.is_empty() {
        build_model_args(chat.claude.model.as_deref())
    } else {
        model_args
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatModel {
    Claude(Vec<String>),
    Agy {
        model: String,
        effort: Option<String>,
    },
}

fn cli_model(model_args: &[String]) -> Option<&str> {
    model_args
        .windows(2)
        .find(|pair| pair[0] == "--model")
        .map(|pair| pair[1].as_str())
}

pub fn resolve_chat_model(
    chat: &ChatSection,
    model_args: Vec<String>,
) -> Result<ChatModel, String> {
    if chat.harness_kind() != MemoryHarness::Agy {
        return Ok(ChatModel::Claude(claude_model_args(chat, model_args)));
    }
    let model = match cli_model(&model_args) {
        Some(model) if validate_thread_model(model).is_ok() => {
            return Err(format!(
                "--model {model} is a claude model; chat.harness is agy, so use a Gemini model"
            ))
        }
        Some(model) => model.to_string(),
        None => chat
            .agy
            .model
            .clone()
            .ok_or_else(|| "chat.agy.model must be set when chat.harness is agy".to_string())?,
    };
    Ok(ChatModel::Agy {
        model,
        effort: chat.agy.effort.clone(),
    })
}

pub fn ignored_extras(config: &AppConfig, harness: MemoryHarness) -> Vec<String> {
    if harness != MemoryHarness::Agy {
        return Vec::new();
    }
    let chat = &config.chat;
    [
        (
            !chat.skills.is_empty(),
            "chat.skills: plugin not loaded, slash expansion still works",
        ),
        (chat.user_settings, "chat.user_settings: not read by agy"),
        (
            config.claude.config_dir.is_some(),
            "claude.config_dir: no claude config isolation on agy",
        ),
        (true, "subagents (--agents): not available on agy"),
        (true, "attachments directory: not available on agy"),
    ]
    .into_iter()
    .filter(|(set, _)| *set)
    .map(|(_, note)| note.to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<AppConfig, String> {
        AppConfig::parse_reporting_unknown(text, "test")
            .map(|(config, _)| config)
            .map_err(|error| error.to_string())
    }

    fn refused(text: &str) -> String {
        parse(text).unwrap_err()
    }

    #[test]
    fn chat_harness_defaults_to_claude() {
        let config = parse("").unwrap();
        assert_eq!(config.chat.harness, "claude");
        assert_eq!(config.chat.harness_kind(), MemoryHarness::Claude);
    }

    #[test]
    fn unknown_chat_harness_is_refused() {
        assert!(refused("[chat]\nharness = 'codex'").contains("chat.harness must be claude or agy"));
    }

    #[test]
    fn agy_chat_needs_a_model() {
        assert!(refused("[chat]\nharness = 'agy'").contains("chat.agy.model"));
    }

    #[test]
    fn haiku_is_refused_on_agy_chat() {
        let text = "[chat]\nharness = 'agy'\n[chat.agy]\nmodel = 'haiku'";
        assert!(refused(text).contains("Gemini model"));
    }

    #[test]
    fn bad_agy_effort_is_refused() {
        let text =
            "[chat]\nharness = 'agy'\n[chat.agy]\nmodel = 'gemini-3-flash'\neffort = 'very high'";
        assert!(refused(text).contains("chat.agy.effort"));
    }

    #[test]
    fn good_agy_chat_is_accepted() {
        let text = "[chat]\nharness = 'agy'\n[chat.agy]\nmodel = 'gemini-3-flash'\neffort = 'high'";
        assert_eq!(parse(text).unwrap().chat.harness_kind(), MemoryHarness::Agy);
    }

    #[test]
    fn chat_claude_model_is_the_fallback_for_the_cli_model() {
        let config = parse("[chat.claude]\nmodel = 'opus'").unwrap();
        assert_eq!(
            claude_model_args(&config.chat, Vec::new()),
            ["--model", "opus"]
        );
        let cli = vec!["--model".to_string(), "sonnet".to_string()];
        assert_eq!(claude_model_args(&config.chat, cli.clone()), cli);
    }

    fn cli(model: &str) -> Vec<String> {
        vec!["--model".to_string(), model.to_string()]
    }

    const AGY_CHAT: &str = "[chat]\nharness = 'agy'\n[chat.agy]\nmodel = 'gemini-3-flash'\neffort = 'high'\n[chat.claude]\nmodel = 'opus'";

    #[test]
    fn claude_harness_resolves_the_claude_fallback_and_ignores_agy_settings() {
        let config =
            parse("[chat.claude]\nmodel = 'opus'\n[chat.agy]\nmodel = 'gemini-3-flash'").unwrap();
        assert_eq!(
            resolve_chat_model(&config.chat, Vec::new()),
            Ok(ChatModel::Claude(cli("opus")))
        );
        assert_eq!(
            resolve_chat_model(&config.chat, cli("sonnet")),
            Ok(ChatModel::Claude(cli("sonnet")))
        );
    }

    #[test]
    fn agy_harness_takes_the_cli_model_before_the_configured_one() {
        let config = parse(AGY_CHAT).unwrap();
        let agy = |model: &str| ChatModel::Agy {
            model: model.to_string(),
            effort: Some("high".to_string()),
        };
        assert_eq!(
            resolve_chat_model(&config.chat, cli("gemini-3-pro")),
            Ok(agy("gemini-3-pro"))
        );
        assert_eq!(
            resolve_chat_model(&config.chat, Vec::new()),
            Ok(agy("gemini-3-flash"))
        );
    }

    #[test]
    fn agy_harness_refuses_a_claude_cli_model() {
        let config = parse(AGY_CHAT).unwrap();
        for model in ["haiku", "sonnet", "opus", "claude-opus-5"] {
            let error = resolve_chat_model(&config.chat, cli(model)).unwrap_err();
            assert!(error.contains("is a claude model"), "{error}");
        }
    }

    #[test]
    fn ignored_extras_are_empty_for_claude_and_listed_for_agy() {
        let config = parse("[chat]\nuser_settings = true\nskills = ['/s']").unwrap();
        assert!(ignored_extras(&config, MemoryHarness::Claude).is_empty());
        let agy = ignored_extras(&config, MemoryHarness::Agy);
        assert_eq!(agy.len(), 4);
        assert!(agy[0].starts_with("chat.skills"));
        assert!(agy[1].starts_with("chat.user_settings"));
        let bare = ignored_extras(&AppConfig::new(), MemoryHarness::Agy);
        assert_eq!(bare.len(), 2);
    }
}
