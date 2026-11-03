use crate::worker::SERVER_NAME;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

pub const ACP_PROTOCOL_VERSION: u16 = 1;
pub const VERSION_PROBE_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcpAgent {
    pub name: &'static str,
    pub command: &'static str,
    pub tested_version: &'static str,
    pub login_method: &'static str,
}

pub const CURSOR_AGENT: AcpAgent = AcpAgent {
    name: "cursor",
    command: "agent",
    tested_version: "2026.10.14",
    login_method: "cursor_login",
};
pub const AUTH_REQUIRED_CODE: i32 = -32000;

pub fn parse_agent_version(output: &str) -> Option<&str> {
    let token = output.split_whitespace().next()?;
    let version = token.split('-').next()?;
    version.chars().next()?.is_ascii_digit().then_some(version)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionProbe {
    Exited(String),
    ExitedWithError(String),
    Unreadable(String),
    TimedOut,
    Failed(String),
}

pub fn finished_probe(success: bool, stdout: Vec<u8>) -> VersionProbe {
    match (success, String::from_utf8(stdout)) {
        (false, text) => VersionProbe::ExitedWithError(lossy(text)),
        (true, Ok(text)) => VersionProbe::Exited(text),
        (true, Err(error)) => VersionProbe::Unreadable(lossy(Err(error))),
    }
}

fn lossy(text: Result<String, std::string::FromUtf8Error>) -> String {
    text.unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

pub fn probed_version<'a>(agent: &AcpAgent, probe: &'a VersionProbe) -> Result<&'a str, String> {
    let command = agent.command;
    match probe {
        VersionProbe::Exited(output) => parse_agent_version(output)
            .ok_or_else(|| format!("could not read the version from `{command} --version`")),
        VersionProbe::ExitedWithError(_) => Err(format!(
            "could not run `{command} --version`: it exited with an error status"
        )),
        VersionProbe::Unreadable(_) => Err(format!(
            "could not read the version from `{command} --version`"
        )),
        VersionProbe::TimedOut => Err(format!(
            "`{command} --version` did not answer within {}s",
            VERSION_PROBE_LIMIT.as_secs()
        )),
        VersionProbe::Failed(error) => Err(format!("could not run `{command} --version`: {error}")),
    }
}

pub fn check_protocol(agent: &AcpAgent, version: u16) -> Result<(), String> {
    if version == ACP_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(format!(
            "{} agent speaks ACP protocol {version}; ralph supports {ACP_PROTOCOL_VERSION}.",
            agent.name
        ))
    }
}

pub fn auth_method<'a>(agent: &AcpAgent, advertised: &[&'a str]) -> Option<&'a str> {
    advertised
        .iter()
        .find(|id| **id == agent.login_method)
        .copied()
}

pub fn session_error(agent: &AcpAgent, code: i32, message: &str) -> String {
    if code == AUTH_REQUIRED_CODE {
        format!(
            "{} agent is not logged in; run `{} login` and spawn the worker again.",
            agent.name, agent.command
        )
    } else {
        format!("{} agent could not start a session: {message}", agent.name)
    }
}

pub fn acp_mcp_server(
    exe: &Path,
    id: &str,
    inbox: &Path,
    owner: &Path,
    worktree: Option<&Path>,
) -> Value {
    let mut args = vec![
        "worker-mcp".to_string(),
        "--id".to_string(),
        id.to_string(),
        "--inbox".to_string(),
        inbox.to_string_lossy().into_owned(),
        "--owner".to_string(),
        owner.to_string_lossy().into_owned(),
    ];
    if let Some(worktree) = worktree {
        args.push("--worktree".to_string());
        args.push(worktree.to_string_lossy().into_owned());
    }
    json!({
        "name": SERVER_NAME,
        "command": exe.to_string_lossy(),
        "args": args,
        "env": []
    })
}

pub fn claude_mcp_config(server: &Value) -> Value {
    json!({"mcpServers": {SERVER_NAME: {"command": server["command"], "args": server["args"]}}})
}

pub fn cursor_model_rejection(
    agent_name: &str,
    requested: &str,
    message: &str,
    data_message: Option<&str>,
) -> String {
    let detail = data_message
        .filter(|text| !text.is_empty())
        .unwrap_or(message);
    format!("{agent_name} agent rejected model '{requested}': {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_model_rejection_prefers_the_agent_data_message() {
        assert_eq!(
            cursor_model_rejection(
                "cursor",
                "composer-2.5",
                "Invalid params",
                Some("Invalid model value: composer-2.5")
            ),
            "cursor agent rejected model 'composer-2.5': Invalid model value: composer-2.5"
        );
        assert_eq!(
            cursor_model_rejection("cursor", "x", "Invalid params", Some("")),
            "cursor agent rejected model 'x': Invalid params"
        );
    }

    #[test]
    fn version_output_is_parsed_without_the_build_hash() {
        assert_eq!(
            parse_agent_version("2026.10.14-0f9edde\n"),
            Some("2026.10.14")
        );
        assert_eq!(parse_agent_version("error: nope"), None);
        assert_eq!(parse_agent_version(""), None);
    }

    #[test]
    fn a_version_probe_names_why_it_has_no_version() {
        let probe = |probe| probed_version(&CURSOR_AGENT, &probe).map(str::to_string);
        assert_eq!(
            probe(VersionProbe::Exited("2026.10.14-0f9edde\n".into())),
            Ok("2026.10.14".to_string())
        );
        assert_eq!(
            probe(VersionProbe::Exited(String::new())),
            Err("could not read the version from `agent --version`".to_string())
        );
        assert_eq!(
            probe(VersionProbe::TimedOut),
            Err("`agent --version` did not answer within 10s".to_string())
        );
        assert_eq!(
            probe(VersionProbe::Failed("No such file or directory".into())),
            Err("could not run `agent --version`: No such file or directory".to_string())
        );
    }

    #[test]
    fn a_failed_exit_is_a_failed_probe_not_a_version() {
        assert_eq!(
            finished_probe(true, b"2026.10.14\n".to_vec()),
            VersionProbe::Exited("2026.10.14\n".into())
        );
        let failed = finished_probe(false, b"2026.10.14\n".to_vec());
        assert_eq!(failed, VersionProbe::ExitedWithError("2026.10.14\n".into()));
        assert_eq!(
            probed_version(&CURSOR_AGENT, &failed),
            Err("could not run `agent --version`: it exited with an error status".to_string())
        );
    }

    #[test]
    fn version_text_that_is_not_utf8_is_unreadable_not_unrunnable() {
        assert_eq!(
            probed_version(&CURSOR_AGENT, &finished_probe(true, vec![0xff])),
            Err("could not read the version from `agent --version`".to_string())
        );
    }

    #[test]
    fn version_output_edge_cases() {
        assert_eq!(
            parse_agent_version("\n  2026.10.14-0f9edde extra\n"),
            Some("2026.10.14")
        );
        assert_eq!(parse_agent_version("2026.10.14 build"), Some("2026.10.14"));
        assert_eq!(parse_agent_version("  \n\t"), None);
        assert_eq!(parse_agent_version("-0f9edde"), None);
        assert_eq!(parse_agent_version("v2026.10.14"), None);
    }

    #[test]
    fn only_protocol_one_is_accepted() {
        assert!(check_protocol(&CURSOR_AGENT, 1).is_ok());
        assert!(check_protocol(&CURSOR_AGENT, 2)
            .unwrap_err()
            .contains("protocol 2"));
    }

    #[test]
    fn auth_required_tells_the_user_to_log_in() {
        assert!(session_error(&CURSOR_AGENT, -32000, "x").contains("agent login"));
        assert!(session_error(&CURSOR_AGENT, -32603, "boom").contains("boom"));
    }

    #[test]
    fn only_the_agent_login_method_is_ever_chosen() {
        assert_eq!(
            auth_method(&CURSOR_AGENT, &["api_key", "cursor_login"]),
            Some("cursor_login")
        );
        assert_eq!(auth_method(&CURSOR_AGENT, &["api_key"]), None);
        assert_eq!(auth_method(&CURSOR_AGENT, &[]), None);
    }

    #[test]
    fn mcp_server_entry_is_a_stdio_worker_mcp_command() {
        let entry = acp_mcp_server(
            Path::new("/bin/ralph"),
            "A",
            Path::new("/s/inbox"),
            Path::new("/s/cursor/A"),
            Some(Path::new("/w")),
        );
        assert_eq!(entry["name"], SERVER_NAME);
        assert_eq!(entry["command"], "/bin/ralph");
        assert_eq!(
            entry["args"],
            json!([
                "worker-mcp",
                "--id",
                "A",
                "--inbox",
                "/s/inbox",
                "--owner",
                "/s/cursor/A",
                "--worktree",
                "/w"
            ])
        );
        assert_eq!(entry["env"], json!([]));
    }

    #[test]
    fn mcp_server_entry_omits_the_worktree_without_one() {
        let entry = acp_mcp_server(
            Path::new("/bin/ralph"),
            "A",
            Path::new("/s/inbox"),
            Path::new("/s/cursor/A"),
            None,
        );
        assert_eq!(
            entry["args"],
            json!([
                "worker-mcp",
                "--id",
                "A",
                "--inbox",
                "/s/inbox",
                "--owner",
                "/s/cursor/A"
            ])
        );
    }

    #[test]
    fn claude_mcp_config_keys_the_command_and_args_by_server_name() {
        let server = acp_mcp_server(
            Path::new("/bin/ralph"),
            "A",
            Path::new("/s/inbox"),
            Path::new("/s/claude/A"),
            None,
        );
        assert_eq!(
            claude_mcp_config(&server),
            json!({"mcpServers": {SERVER_NAME: {
                "command": "/bin/ralph",
                "args": [
                    "worker-mcp", "--id", "A", "--inbox", "/s/inbox", "--owner", "/s/claude/A"
                ]
            }}})
        );
    }
}
