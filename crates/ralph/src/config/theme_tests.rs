use super::*;

#[test]
fn theme_status_table_loads_and_reports_warnings() {
    let config: AppConfig = toml::from_str(
        "[theme.status]\nmode_idle_bg = \"#102030\"\nmode_nap_bg = \"nope\"\nbranch_fg = \"red\"\n",
    )
    .unwrap();
    assert_eq!(config.theme.status.colors.len(), 3);
    assert_eq!(
        config.theme.warnings(),
        [
            "[theme.status] unknown key \"branch_fg\"; ignored",
            "[theme.status] mode_nap_bg = \"nope\" is not a color; using the default"
        ]
    );
    assert!(AppConfig::new().theme.warnings().is_empty());
}

#[test]
fn theme_input_table_loads_and_reports_warnings() {
    let config: AppConfig =
        toml::from_str("[theme.input]\nsuggestion = \"#102030\"\nhint = \"red\"\n").unwrap();
    assert_eq!(config.theme.input.colors.len(), 2);
    assert_eq!(
        config.theme.warnings(),
        ["[theme.input] unknown key \"hint\"; ignored"]
    );
    let bad: AppConfig = toml::from_str("[theme.input]\nsuggestion = \"nope\"\n").unwrap();
    assert_eq!(
        bad.theme.warnings(),
        ["[theme.input] suggestion = \"nope\" is not a color; using the default"]
    );
}

#[test]
fn role_tables_parse_code_and_colors() {
    let config: AppConfig = toml::from_str(
        "[theme.assistant]\ncode = \"Monokai Extended\"\nheading1 = \"red\"\n\n[theme.agent]\ncode = \"Sublime Snazzy\"\nborder = \"#00ff00\"\nbad = \"x\"\n",
    )
    .unwrap();
    assert_eq!(
        config.theme.assistant.code.as_deref(),
        Some("Monokai Extended")
    );
    let agent = config.theme.agent();
    assert_eq!(agent.code.as_deref(), Some("Sublime Snazzy"));
    assert_eq!(
        agent.colors.get("border").map(String::as_str),
        Some("#00ff00")
    );
    assert_eq!(
        config.theme.warnings(),
        ["[theme.agent] unknown key \"bad\"; ignored"]
    );
}

#[test]
fn theme_diff_table_loads_and_reports_warnings() {
    let config: AppConfig =
        toml::from_str("[theme.diff]\nadd_bg = \"#000000\"\nctx_fg = \"red\"\n").unwrap();
    assert_eq!(config.theme.diff.colors.len(), 2);
    assert_eq!(
        config.theme.warnings(),
        ["[theme.diff] unknown key \"ctx_fg\"; ignored"]
    );
    let bad: AppConfig = toml::from_str("[theme.diff]\nadd_bg = \"nope\"\n").unwrap();
    assert_eq!(
        bad.theme.warnings(),
        ["[theme.diff] add_bg = \"nope\" is not a color; using the default"]
    );
}

#[test]
fn theme_tools_table_loads_and_reports_warnings() {
    let config: AppConfig =
        toml::from_str("[theme.tools]\nname = \"#000000\"\nbold = \"red\"\n").unwrap();
    assert_eq!(config.theme.tools.colors.len(), 2);
    assert_eq!(
        config.theme.warnings(),
        ["[theme.tools] unknown key \"bold\"; ignored"]
    );
    let bad: AppConfig = toml::from_str("[theme.tools]\nname = \"nope\"\n").unwrap();
    assert_eq!(
        bad.theme.warnings(),
        ["[theme.tools] name = \"nope\" is not a color; using the default"]
    );
}

#[test]
fn theme_ui_table_loads_and_reports_warnings() {
    let config: AppConfig =
        toml::from_str("[theme.ui]\nspinner = \"#000000\"\nbold = \"red\"\n").unwrap();
    assert_eq!(config.theme.ui.colors.len(), 2);
    assert_eq!(
        config.theme.warnings(),
        ["[theme.ui] unknown key \"bold\"; ignored"]
    );
    let bad: AppConfig = toml::from_str("[theme.ui]\nwarning = \"nope\"\n").unwrap();
    assert_eq!(
        bad.theme.warnings(),
        ["[theme.ui] warning = \"nope\" is not a color; using the default"]
    );
}
