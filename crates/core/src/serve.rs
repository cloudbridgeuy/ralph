pub fn serve_store_gate(env: Option<&str>) -> Result<(), String> {
    match env.filter(|value| !value.trim().is_empty()) {
        Some(value) => Err(format!(
            "ralph serve takes its store only from --store; unset RALPH_CHAT_STORE (set to {value})"
        )),
        None => Ok(()),
    }
}

pub fn serve_refusal(store: &std::path::Path, holder: Option<u32>) -> String {
    let by = holder
        .map(|pid| format!(" by pid {pid}"))
        .unwrap_or_default();
    format!(
        "ralph serve: store {} is in use{by}; stop that chat or serve first",
        store.display()
    )
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

    #[test]
    fn the_refusal_names_store_holder_and_command() {
        let message = serve_refusal(std::path::Path::new("/s/x"), Some(42));
        assert!(message.contains("ralph serve"), "{message}");
        assert!(message.contains("/s/x"), "{message}");
        assert!(message.contains("pid 42"), "{message}");
        assert!(!serve_refusal(std::path::Path::new("/s/x"), None).contains("pid"));
    }
}
