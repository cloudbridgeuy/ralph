const HEADER: &str = "ralph bundles syntax grammars curated by the bat project \
(https://github.com/sharkdp/bat) through the two-face crate.\n\
Their licence notices follow.\n\n";

const FONT_HEADER: &str = "The ralph web shell bundles fonts under their own licences.\n\
Their notices follow.\n\n";

pub fn font_notices(fonts: &[(&str, &str)]) -> String {
    if fonts.is_empty() {
        return String::new();
    }
    fonts
        .iter()
        .fold(FONT_HEADER.to_string(), |mut out, (name, text)| {
            out.push_str(&format!("{name}\n\n{}\n\n", text.trim_end()));
            out
        })
}

pub fn syntax_notices(notices: &[(&str, &str)]) -> String {
    notices
        .iter()
        .fold(HEADER.to_string(), |mut out, (path, text)| {
            out.push_str(&format!("{path}\n\n{}\n\n", text.trim_end()));
            out
        })
}

#[cfg(test)]
mod tests;
