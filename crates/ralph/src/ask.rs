//! Ask command — thin handler delegating to the shared invocation engine.
//!
//! Builds an `InvocationConfig` with `agent: None` and calls `invoke()`.

use crate::invoke;

/// Environment variable for permission mode override.
const RALPH_PERMISSION_MODE_ENV: &str = "RALPH_PERMISSION_MODE";

/// Resolve permission mode from multiple sources with precedence.
///
/// Resolution order (highest priority first):
/// 1. CLI flag (passed as `cli_value`)
/// 2. Environment variable (`RALPH_PERMISSION_MODE`)
/// 3. Config file (`~/.config/ralph/config.toml` under `[ask]` section)
/// 4. Default value (`bypassPermissions`)
pub fn resolve_permission_mode(cli_value: Option<&str>) -> String {
    if let Some(mode) = cli_value {
        return mode.to_string();
    }

    if let Ok(env_value) = std::env::var(RALPH_PERMISSION_MODE_ENV) {
        if !env_value.is_empty() {
            return env_value;
        }
    }

    if let Ok(config) = crate::config::AppConfig::load() {
        if let Some(config_value) = config.ask.permission_mode {
            return config_value;
        }
    }

    invoke::DEFAULT_PERMISSION_MODE.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_permission_mode_cli_explicit() {
        let result = resolve_permission_mode(Some("acceptEdits"));
        assert_eq!(result, "acceptEdits");
    }

    #[test]
    fn test_resolve_permission_mode_cli_explicit_default() {
        let result = resolve_permission_mode(Some(invoke::DEFAULT_PERMISSION_MODE));
        assert_eq!(result, invoke::DEFAULT_PERMISSION_MODE);
    }

    #[test]
    fn test_resolve_permission_mode_none_returns_default() {
        let result = resolve_permission_mode(None);
        assert!(!result.is_empty());
    }
}
