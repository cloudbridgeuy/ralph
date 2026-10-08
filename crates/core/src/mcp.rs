use serde_json::{json, Value};

const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INVALID_REQUEST: i64 = -32600;

#[derive(Debug, Clone, PartialEq)]
pub enum Dispatch {
    Silent,
    Respond(Value),
    Call {
        id: Value,
        name: String,
        args: Value,
    },
}

pub fn string_arg(args: &Value, key: &str) -> Result<String, String> {
    match args.get(key).and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => Ok(s.to_string()),
        _ => Err(format!("Missing or empty string argument '{key}'.")),
    }
}

fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

pub fn tool_result(id: Value, text: &str, is_error: bool) -> Value {
    success(
        id,
        json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
    )
}

pub fn dispatch(server_name: &str, tools: &Value, request: &Value) -> Dispatch {
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return match request.get("id") {
            Some(id) => Dispatch::Respond(failure(id.clone(), INVALID_REQUEST, "Invalid request")),
            None => Dispatch::Silent,
        };
    };
    let Some(id) = request.get("id").cloned() else {
        return Dispatch::Silent;
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL_VERSION);
            Dispatch::Respond(success(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": server_name, "version": "0.1.0"}
                }),
            ))
        }
        "ping" => Dispatch::Respond(success(id, json!({}))),
        "tools/list" => Dispatch::Respond(success(id, json!({"tools": tools}))),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Dispatch::Respond(failure(id, INVALID_PARAMS, "Missing tool name"));
            };
            Dispatch::Call {
                id,
                name: name.to_string(),
                args: params.get("arguments").cloned().unwrap_or(Value::Null),
            }
        }
        other => Dispatch::Respond(failure(
            id,
            METHOD_NOT_FOUND,
            &format!("Method not found: {other}"),
        )),
    }
}
