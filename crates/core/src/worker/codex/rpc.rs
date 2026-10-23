use serde_json::{json, Value};
use std::collections::{BTreeSet, VecDeque};

const REJECTED_CODE: i64 = -32601;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    Response {
        id: u64,
        outcome: Result<Value, RpcError>,
    },
    Notification {
        method: String,
        params: Value,
    },
    ServerRequest {
        id: Value,
        method: String,
        params: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    Malformed(String),
    Unrecognized(String),
    UnknownId(u64),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(error) => write!(f, "codex sent a line that is not JSON: {error}"),
            Self::Unrecognized(line) => write!(f, "codex sent an unrecognized frame: {line}"),
            Self::UnknownId(id) => write!(f, "codex answered request {id} that was never sent"),
        }
    }
}

#[derive(Debug, Default)]
pub struct Rpc {
    last: u64,
    pending: BTreeSet<u64>,
    deferred: VecDeque<Incoming>,
}

impl Rpc {
    pub fn request(&mut self, method: &str, params: Value) -> (u64, String) {
        self.last += 1;
        self.pending.insert(self.last);
        let line = json!({"jsonrpc": "2.0", "id": self.last, "method": method, "params": params});
        (self.last, line.to_string())
    }

    pub fn settle(&mut self, id: u64, incoming: Incoming) -> Option<Result<Value, RpcError>> {
        match incoming {
            Incoming::Response {
                id: answered,
                outcome,
            } if answered == id => Some(outcome),
            other => {
                self.deferred.push_back(other);
                None
            }
        }
    }

    pub fn next_deferred(&mut self) -> Option<Incoming> {
        self.deferred.pop_front()
    }

    pub fn notification(method: &str) -> String {
        json!({"jsonrpc": "2.0", "method": method}).to_string()
    }

    pub fn receive(&mut self, line: &str) -> Result<Incoming, FrameError> {
        let frame: Value =
            serde_json::from_str(line).map_err(|e| FrameError::Malformed(e.to_string()))?;
        let method = frame.get("method").and_then(Value::as_str);
        let params = frame.get("params").cloned().unwrap_or(Value::Null);
        match (method, frame.get("id")) {
            (Some(method), Some(id)) => Ok(Incoming::ServerRequest {
                id: id.clone(),
                method: method.to_string(),
                params,
            }),
            (Some(method), None) => Ok(Incoming::Notification {
                method: method.to_string(),
                params,
            }),
            (None, Some(id)) => {
                let id = id
                    .as_u64()
                    .ok_or_else(|| FrameError::Unrecognized(line.to_string()))?;
                if !self.pending.remove(&id) {
                    return Err(FrameError::UnknownId(id));
                }
                let outcome = match frame.get("error") {
                    Some(error) => Err(RpcError {
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    }),
                    None => Ok(frame.get("result").cloned().unwrap_or(Value::Null)),
                };
                Ok(Incoming::Response { id, outcome })
            }
            (None, None) => Err(FrameError::Unrecognized(line.to_string())),
        }
    }
}

pub fn reject_server_request(id: &Value, method: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": REJECTED_CODE,
            "message": format!("ralph declines {method}: approvals are not answered")
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_get_increasing_ids_and_responses_resolve_them() {
        let mut rpc = Rpc::default();
        let (first, line) = rpc.request("initialize", json!({}));
        let (second, _) = rpc.request("model/list", json!({}));
        assert_eq!((first, second), (1, 2));
        assert!(line.contains(r#""method":"initialize""#));
        let reply = r#"{"id":1,"result":{"userAgent":"x/0.162.1 (Mac)"}}"#;
        assert_eq!(
            rpc.receive(reply),
            Ok(Incoming::Response {
                id: 1,
                outcome: Ok(json!({"userAgent": "x/0.162.1 (Mac)"}))
            })
        );
        assert_eq!(rpc.receive(reply), Err(FrameError::UnknownId(1)));
    }

    #[test]
    fn an_error_response_keeps_code_and_message() {
        let mut rpc = Rpc::default();
        rpc.request("turn/interrupt", json!({}));
        let line = r#"{"error":{"code":-32600,"message":"no active turn to interrupt"},"id":1}"#;
        assert_eq!(
            rpc.receive(line),
            Ok(Incoming::Response {
                id: 1,
                outcome: Err(RpcError {
                    code: -32600,
                    message: "no active turn to interrupt".into()
                })
            })
        );
    }

    #[test]
    fn notifications_and_server_requests_are_told_apart() {
        let mut rpc = Rpc::default();
        let status = r#"{"method":"mcpServer/startupStatus/updated","params":{"name":"ralph-worker","status":"ready"},"emittedAtMs":1}"#;
        assert!(matches!(
            rpc.receive(status),
            Ok(Incoming::Notification { method, .. }) if method == "mcpServer/startupStatus/updated"
        ));
        let ask = r#"{"id":0,"method":"mcpServer/elicitation/request","params":{"serverName":"ralph-worker"}}"#;
        let Ok(Incoming::ServerRequest { id, method, .. }) = rpc.receive(ask) else {
            panic!("not a server request");
        };
        let reply: Value = serde_json::from_str(&reject_server_request(&id, &method)).unwrap();
        assert_eq!(reply["id"], json!(0));
        assert_eq!(reply["error"]["code"], json!(-32601));
        assert!(reply.get("result").is_none());
    }

    #[test]
    fn frames_that_arrive_while_waiting_are_kept_in_arrival_order() {
        let mut rpc = Rpc::default();
        let (id, _) = rpc.request("turn/start", json!({}));
        let (other, _) = rpc.request("turn/interrupt", json!({}));
        let frames = [
            r#"{"method":"item/completed","params":{"n":1}}"#,
            r#"{"id":7,"method":"item/commandExecution/requestApproval","params":{}}"#,
            r#"{"id":2,"result":{}}"#,
            r#"{"method":"turn/completed","params":{"n":2}}"#,
            r#"{"id":1,"result":{"turn":{"id":"u"}}}"#,
        ];
        let mut settled = None;
        for frame in frames {
            let incoming = rpc.receive(frame).unwrap();
            settled = settled.or(rpc.settle(id, incoming));
        }
        assert_eq!(settled, Some(Ok(json!({"turn": {"id": "u"}}))));
        let mut replies = Vec::new();
        let methods: Vec<_> = std::iter::from_fn(|| rpc.next_deferred())
            .map(|incoming| match incoming {
                Incoming::Notification { method, .. } => method,
                Incoming::Response { id, .. } => format!("response {id}"),
                Incoming::ServerRequest { id, method, .. } => {
                    replies.push(reject_server_request(&id, &method));
                    method
                }
            })
            .collect();
        assert_eq!(
            methods,
            [
                "item/completed",
                "item/commandExecution/requestApproval",
                &format!("response {other}"),
                "turn/completed"
            ]
        );
        assert_eq!(replies.len(), 1);
        let reply: Value = serde_json::from_str(&replies[0]).unwrap();
        assert_eq!(
            (reply["id"].clone(), reply["error"]["code"].clone()),
            (json!(7), json!(-32601))
        );
        assert_eq!(rpc.next_deferred(), None);
    }

    #[test]
    fn malformed_lines_are_typed_errors() {
        let mut rpc = Rpc::default();
        assert!(matches!(
            rpc.receive("not json"),
            Err(FrameError::Malformed(_))
        ));
        assert!(matches!(
            rpc.receive(r#"{"params":{}}"#),
            Err(FrameError::Unrecognized(_))
        ));
    }
}
