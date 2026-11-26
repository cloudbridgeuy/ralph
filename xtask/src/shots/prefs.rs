use super::core::Theme;

pub fn user_js(theme: Theme, marionette_port: u16) -> String {
    let scheme = match theme {
        Theme::Dark => 0,
        Theme::Light => 1,
    };
    format!(
        "user_pref(\"layout.css.prefers-color-scheme.content-override\", {scheme});\n\
         user_pref(\"marionette.port\", {marionette_port});\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_js_sets_scheme_and_port() {
        let dark = user_js(Theme::Dark, 4444);
        assert!(dark.contains("prefers-color-scheme.content-override\", 0)"));
        assert!(dark.contains("marionette.port\", 4444)"));
        let light = user_js(Theme::Light, 1);
        assert!(light.contains("prefers-color-scheme.content-override\", 1)"));
    }
}
