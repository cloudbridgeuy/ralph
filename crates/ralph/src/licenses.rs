use ralph_core::licenses::syntax_notices;
use std::io::{ErrorKind, Write};

pub fn bundled_syntax_notices() -> String {
    let listing = two_face::acknowledgement::listing();
    let owned: Vec<_> = listing
        .for_syntaxes()
        .iter()
        .map(|licence| (licence.rel_path.to_string_lossy(), licence.text.as_str()))
        .collect();
    let pairs: Vec<(&str, &str)> = owned
        .iter()
        .map(|(path, text)| (path.as_ref(), *text))
        .collect();
    syntax_notices(&pairs)
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    match std::io::stdout()
        .lock()
        .write_all(bundled_syntax_notices().as_bytes())
    {
        Err(err) if err.kind() != ErrorKind::BrokenPipe => Err(err.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_syntax_notices_names_typescript() {
        let out = bundled_syntax_notices();
        assert!(out.contains("\nsyntaxes/02_Extra/TypeScript/LICENSE\n"));
        assert!(out.contains("Apache License"));
    }
}
