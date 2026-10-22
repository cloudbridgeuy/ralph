use crate::worker::SERVER_NAME;
use serde_json::{json, Value};
use std::path::Path;

pub const ACP_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcpAgent {
    pub name: &'static str,
    pub command: &'static str,
    pub tested_version: &'static str,
}

pub const CURSOR_AGENT: AcpAgent = AcpAgent {
    name: "cursor",
    command: "agent",
    tested_version: "2026.10.14",
};
const AUTH_REQUIRED_CODE: i32 = -32000;

pub fn parse_agent_version(output: &str) -> Option<&str> {
    let token = output.split_whitespace().next()?;
    let version = token.split('-').next()?;
    version.chars().next()?.is_ascii_digit().then_some(version)
}

pub fn version_warning(agent: &AcpAgent, output: &str) -> Option<String> {
    let AcpAgent {
        name,
        tested_version,
        ..
    } = agent;
    match parse_agent_version(output) {
        Some(version) if version == *tested_version => None,
        Some(other) => Some(format!(
            "{name} agent {other} differs from the tested {tested_version}; the {name} worker may misbehave."
        )),
        None => Some(format!(
            "could not read the {name} agent version from '{}'; tested with {tested_version}.",
            output.trim()
        )),
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
    fn version_warning_only_when_the_version_differs() {
        assert_eq!(version_warning(&CURSOR_AGENT, "2026.10.14-0f9edde"), None);
        assert!(version_warning(&CURSOR_AGENT, "2026.11.01-abc")
            .unwrap()
            .contains("2026.11.01"));
        assert!(version_warning(&CURSOR_AGENT, "???")
            .unwrap()
            .contains("could not read"));
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
}
