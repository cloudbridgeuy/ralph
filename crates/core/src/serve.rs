pub fn serve_store_gate(env: Option<&str>) -> Result<(), String> {
    match env.filter(|value| !value.trim().is_empty()) {
        Some(value) => Err(format!(
            "ralph serve takes its store only from --store; unset RALPH_CHAT_STORE (set to {value})"
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_or_blank_env_passes() {
        assert!(serve_store_gate(None).is_ok());
        assert!(serve_store_gate(Some("")).is_ok());
        assert!(serve_store_gate(Some("  \t")).is_ok());
    }

    #[test]
    fn a_set_env_is_refused_and_named() {
        let error = serve_store_gate(Some("/tmp/other")).unwrap_err();
        assert!(error.contains("--store"), "{error}");
        assert!(error.contains("RALPH_CHAT_STORE"), "{error}");
        assert!(error.contains("/tmp/other"), "{error}");
    }
}
