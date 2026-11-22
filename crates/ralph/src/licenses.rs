use ralph_core::licenses::{font_notices, syntax_notices};
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

fn bundled_font_notices() -> String {
    font_notices(&[
        ("Geist Mono (v1.9.2)", ralph_api::FONT_NOTICE),
        ("Geist Mono licence", ralph_api::FONT_LICENCE),
    ])
}

pub fn bundled_notices() -> String {
    format!("{}{}", bundled_syntax_notices(), bundled_font_notices())
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    match std::io::stdout()
        .lock()
        .write_all(bundled_notices().as_bytes())
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

    #[test]
    fn bundled_notices_list_syntaxes_then_the_geist_mono_licence() {
        let out = bundled_notices();
        let syntaxes = out.find("syntaxes/02_Extra/TypeScript/LICENSE").unwrap();
        let font = out.find("Geist Mono").unwrap();
        assert!(syntaxes < font);
        assert!(out.contains("SIL OPEN FONT LICENSE Version 1.1"));
        assert!(out.contains("Copyright 2026 The Geist Project Authors"));
        assert!(out.contains("v1.9.2"));
    }
}
