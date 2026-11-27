use super::core::Theme;

const UNROUTABLE: &str = "http://invalid.invalid/";

enum Pref {
    Bool(bool),
    Int(i64),
    Str(&'static str),
}

const PREFS: &[(&str, Pref)] = &[
    // A transient macOS overlay scrollbar made memory-dark-390 flaky; classic scrollbars keep shots stable.
    ("ui.useOverlayScrollbars", Pref::Int(0)),
    (
        "datareporting.policy.dataSubmissionEnabled",
        Pref::Bool(false),
    ),
    (
        "datareporting.healthreport.uploadEnabled",
        Pref::Bool(false),
    ),
    ("toolkit.telemetry.enabled", Pref::Bool(false)),
    ("toolkit.telemetry.unified", Pref::Bool(false)),
    ("toolkit.telemetry.archive.enabled", Pref::Bool(false)),
    ("toolkit.telemetry.server", Pref::Str(UNROUTABLE)),
    ("app.update.enabled", Pref::Bool(false)),
    ("app.update.auto", Pref::Bool(false)),
    ("app.update.service.enabled", Pref::Bool(false)),
    ("extensions.update.enabled", Pref::Bool(false)),
    ("app.update.checkInstallTime", Pref::Bool(false)),
    ("browser.shell.checkDefaultBrowser", Pref::Bool(false)),
    (
        "browser.startup.homepage_override.mstone",
        Pref::Str("ignore"),
    ),
    ("startup.homepage_welcome_url", Pref::Str("")),
    ("startup.homepage_welcome_url.additional", Pref::Str("")),
    ("browser.aboutwelcome.enabled", Pref::Bool(false)),
    ("browser.startup.page", Pref::Int(0)),
    ("browser.startup.homepage", Pref::Str("about:blank")),
    ("browser.newtabpage.enabled", Pref::Bool(false)),
    ("trailhead.firstrun.didSeeAboutWelcome", Pref::Bool(true)),
    ("browser.safebrowsing.malware.enabled", Pref::Bool(false)),
    ("browser.safebrowsing.phishing.enabled", Pref::Bool(false)),
    ("browser.safebrowsing.downloads.enabled", Pref::Bool(false)),
    ("app.normandy.enabled", Pref::Bool(false)),
    ("app.shield.optoutstudies.enabled", Pref::Bool(false)),
    ("services.settings.server", Pref::Str(UNROUTABLE)),
    ("network.captive-portal-service.enabled", Pref::Bool(false)),
    ("network.connectivity-service.enabled", Pref::Bool(false)),
    ("browser.urlbar.suggest.searches", Pref::Bool(false)),
    (
        "browser.urlbar.suggest.quicksuggest.nonsponsored",
        Pref::Bool(false),
    ),
    (
        "browser.urlbar.suggest.quicksuggest.sponsored",
        Pref::Bool(false),
    ),
    ("browser.urlbar.suggest.topsites", Pref::Bool(false)),
    ("browser.urlbar.suggest.history", Pref::Bool(false)),
    ("browser.urlbar.suggest.bookmark", Pref::Bool(false)),
    ("browser.urlbar.suggest.openpage", Pref::Bool(false)),
    ("browser.urlbar.suggest.engines", Pref::Bool(false)),
    ("browser.urlbar.suggest.trending", Pref::Bool(false)),
    ("browser.search.suggest.enabled", Pref::Bool(false)),
    ("geo.enabled", Pref::Bool(false)),
    ("browser.ping-centre.telemetry", Pref::Bool(false)),
    ("extensions.getAddons.cache.enabled", Pref::Bool(false)),
    ("browser.region.update.enabled", Pref::Bool(false)),
    ("browser.search.update", Pref::Bool(false)),
    ("network.prefetch-next", Pref::Bool(false)),
    ("network.dns.disablePrefetch", Pref::Bool(true)),
    ("network.http.speculative-parallel-limit", Pref::Int(0)),
    ("network.predictor.enabled", Pref::Bool(false)),
];

fn format_pref(name: &str, value: &Pref) -> String {
    match value {
        Pref::Bool(flag) => format!("user_pref(\"{name}\", {flag});\n"),
        Pref::Int(number) => format!("user_pref(\"{name}\", {number});\n"),
        Pref::Str(text) => format!("user_pref(\"{name}\", \"{text}\");\n"),
    }
}

pub fn user_js(theme: Theme) -> String {
    let scheme = match theme {
        Theme::Dark => 0,
        Theme::Light => 1,
    };
    let table: String = PREFS
        .iter()
        .map(|(name, value)| format_pref(name, value))
        .collect();
    format!(
        "user_pref(\"layout.css.prefers-color-scheme.content-override\", {scheme});\n\
         user_pref(\"marionette.port\", 0);\n{table}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_js_lets_firefox_pick_the_port_and_renders_every_pref_once() {
        let text = user_js(Theme::Dark);
        assert!(text.contains("user_pref(\"marionette.port\", 0);\n"));
        for (name, value) in PREFS {
            assert_eq!(text.matches(&format_pref(name, value)).count(), 1, "{name}");
        }
    }

    fn pref_lines(theme: Theme) -> Vec<String> {
        user_js(theme).lines().map(str::to_string).collect()
    }

    fn pref_names(theme: Theme) -> Vec<String> {
        pref_lines(theme)
            .iter()
            .filter_map(|line| {
                let rest = line.strip_prefix("user_pref(\"")?;
                Some(rest.split_once('"')?.0.to_string())
            })
            .collect()
    }

    fn line_for(name: &str) -> Option<String> {
        pref_lines(Theme::Dark)
            .into_iter()
            .find(|line| line.starts_with(&format!("user_pref(\"{name}\", ")))
    }

    #[test]
    fn every_pref_name_is_unique() {
        for theme in Theme::ALL {
            let names = pref_names(theme);
            let mut sorted = names.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), names.len());
            assert!(names.len() > 40);
        }
    }

    #[test]
    fn every_pref_line_is_a_valid_user_pref_call() {
        for line in pref_lines(Theme::Dark) {
            let body = line
                .strip_prefix("user_pref(\"")
                .and_then(|rest| rest.strip_suffix(");"))
                .unwrap_or_else(|| panic!("{line}"));
            let (name, value) = body.split_once("\", ").unwrap();
            assert!(!name.is_empty() && !name.contains('"'), "{line}");
            let quoted = value.len() >= 2 && value.starts_with('"') && value.ends_with('"');
            let boolean = value == "true" || value == "false";
            let number = value.parse::<i64>().is_ok();
            assert!(quoted || boolean || number, "{line}");
        }
    }

    #[test]
    fn overlay_scrollbars_are_off_so_captures_do_not_flake() {
        assert_eq!(
            line_for("ui.useOverlayScrollbars").as_deref(),
            Some("user_pref(\"ui.useOverlayScrollbars\", 0);")
        );
    }

    #[test]
    fn the_scheme_override_follows_the_theme() {
        assert!(pref_lines(Theme::Dark).contains(
            &"user_pref(\"layout.css.prefers-color-scheme.content-override\", 0);".to_string()
        ));
        assert!(pref_lines(Theme::Light).contains(
            &"user_pref(\"layout.css.prefers-color-scheme.content-override\", 1);".to_string()
        ));
    }

    #[test]
    fn only_the_unroutable_placeholder_looks_like_a_network_url() {
        for line in pref_lines(Theme::Dark) {
            let urls = line.matches("://").count();
            if urls > 0 {
                assert!(line.contains(UNROUTABLE), "{line}");
            }
        }
        assert!(line_for("toolkit.telemetry.server")
            .unwrap()
            .contains(UNROUTABLE));
        assert!(line_for("services.settings.server")
            .unwrap()
            .contains(UNROUTABLE));
    }
}
