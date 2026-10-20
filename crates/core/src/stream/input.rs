use serde_json::json;

pub fn user_line(text: &str) -> String {
    json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":text}]}})
        .to_string()
}

pub fn interrupt_line(request_id: &str) -> String {
    json!({"type":"control_request","request_id":request_id,"request":{"subtype":"interrupt"}})
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_line_is_one_escaped_json_line() {
        let line = user_line("a \"b\"\nc");
        assert!(!line.contains('\n'));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["message"]["content"][0]["text"], "a \"b\"\nc");
        assert_eq!(value["message"]["role"], "user");
    }

    #[test]
    fn interrupt_line_carries_request_id() {
        let value: serde_json::Value = serde_json::from_str(&interrupt_line("int-1")).unwrap();
        assert_eq!(value["type"], "control_request");
        assert_eq!(value["request_id"], "int-1");
        assert_eq!(value["request"]["subtype"], "interrupt");
    }
}
