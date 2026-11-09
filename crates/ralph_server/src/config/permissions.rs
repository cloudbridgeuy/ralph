pub const CHAT_DEFAULT_PERMISSION_MODE: &str = "auto";
pub const ASK_DEFAULT_PERMISSION_MODE: &str = "bypassPermissions";

pub fn resolve_permission_mode(
    flag: Option<&str>,
    env: Option<&str>,
    config: Option<&str>,
    default: &str,
) -> String {
    flag.or(env.filter(|value| !value.is_empty()))
        .or(config)
        .unwrap_or(default)
        .to_string()
}
