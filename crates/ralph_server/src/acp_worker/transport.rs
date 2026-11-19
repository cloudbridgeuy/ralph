use crate::acp_log::{CappedLog, LoggedRead, LoggedWrite};
use agent_client_protocol::schema::v1::{InitializeRequest, InitializeResponse};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{Agent, ByteStreams, ConnectionTo};
use ralph_core::worker::{check_protocol, AcpAgent};
use std::process::{ChildStdin, ChildStdout};
use std::sync::Arc;

pub(crate) fn streams(
    stdin: ChildStdin,
    stdout: ChildStdout,
    log: Arc<CappedLog>,
) -> ByteStreams<
    blocking::Unblock<LoggedWrite<ChildStdin>>,
    blocking::Unblock<LoggedRead<ChildStdout>>,
> {
    ByteStreams::new(
        blocking::Unblock::new(LoggedWrite {
            inner: stdin,
            log: Arc::clone(&log),
            tag: "in",
        }),
        blocking::Unblock::new(LoggedRead {
            inner: stdout,
            log,
            tag: "out",
        }),
    )
}

pub(crate) async fn initialize(
    connection: &ConnectionTo<Agent>,
    agent: &AcpAgent,
) -> Result<Result<InitializeResponse, String>, agent_client_protocol::Error> {
    let init = connection
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    Ok(check_protocol(agent, init.protocol_version.as_u16()).map(|()| init))
}
