use super::*;
use ralph_core::diff_command::DiffPager;
use tempfile::TempDir;

#[test]
fn test_default_config() {
    let config = AppConfig::new();
    assert!(config.theme.name.is_none());
    assert!(!config.theme.no_background);
}

#[test]
fn chat_section_defaults() {
    assert_eq!(AppConfig::new().chat.mcp_servers, ["mcptools"]);
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(&path, "[chat]\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.chat.mcp_servers, ["mcptools"]);
}

#[test]
fn mermaid_section_defaults_and_overrides() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(&path, "[mermaid]\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.mermaid, MermaidSection::default());
    assert_eq!(config.mermaid.timeout_ms, 500);
    assert_eq!(config.mermaid.max_class_relations, 400);
    fs::write(
        &path,
        "[mermaid]\ntimeout_ms = 900\nmax_flowchart_edges = 10\n",
    )
    .unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.mermaid.timeout_ms, 900);
    assert_eq!(config.mermaid.max_flowchart_edges, 10);
    assert_eq!(config.mermaid.max_rss_mb, 256);
}

#[test]
fn changes_section_defaults_unknown_pager_and_args() {
    let defaults = AppConfig::new().changes;
    assert_eq!(defaults.pager(), DiffPager::Git);
    assert!(defaults.diff_args.is_empty());
    assert!(defaults.enabled);
    assert!(defaults.warnings().is_empty());

    let config: AppConfig =
        toml::from_str("[changes]\npager = \"delta\"\ndiff_args = [\"--stat\", \"--\"]\n").unwrap();
    assert_eq!(config.changes.pager(), DiffPager::Delta);
    assert_eq!(config.changes.diff_args, ["--stat", "--"]);
    assert!(config.changes.warnings().is_empty());

    assert!(config.changes.enabled);
    let off: AppConfig = toml::from_str("[changes]\nenabled = false\n").unwrap();
    assert!(!off.changes.enabled);

    let hunk: AppConfig = toml::from_str("[changes]\npager = \"hunk\"\n").unwrap();
    assert_eq!(hunk.changes.pager(), DiffPager::Hunk);
    assert!(hunk.changes.warnings().is_empty());

    let unknown: AppConfig = toml::from_str("[changes]\npager = \"less\"\n").unwrap();
    assert_eq!(unknown.changes.pager(), DiffPager::Git);
    assert_eq!(
        unknown.changes.warnings(),
        ["[changes] pager = \"less\" is unknown; using git"]
    );
}

#[test]
fn chat_section_parses_watch_timeout_secs() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(&path, "[chat]\nwatch_timeout_secs = 120\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.chat.watch_timeout_secs, Some(120));
    assert_eq!(AppConfig::new().chat.watch_timeout_secs, None);
}

#[test]
fn chat_section_parses_wrap_width() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(&path, "[chat]\nwrap_width = 80\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.chat.wrap_width, Some(80));
    assert_eq!(AppConfig::new().chat.wrap_width, None);
}

#[test]
fn chat_section_parses_editor() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(
        &path,
        "[chat]\neditor = [\"nvim\", \"-c\", \"startinsert\"]\n",
    )
    .unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    let argv = ["nvim", "-c", "startinsert"].map(String::from);
    assert_eq!(config.chat.editor_setting(), (Some(&argv[..]), None));
    assert_eq!(AppConfig::new().chat.editor_setting(), (None, None));
}

#[test]
fn chat_section_wrong_typed_editor_warns_and_keeps_other_keys() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    for editor in ["\"nvim\"", "[1]"] {
        fs::write(
            &path,
            format!("[chat]\neditor = {editor}\nwrap_width = 80\n"),
        )
        .unwrap();
        let config = AppConfig::load_layers(&path, None).unwrap();
        assert_eq!(config.chat.wrap_width, Some(80));
        let (argv, warning) = config.chat.editor_setting();
        assert_eq!(argv, None);
        assert!(warning.is_some());
    }
}

#[test]
fn worker_quiet_secs_defaults_and_warns_on_bad_values() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    assert_eq!(AppConfig::new().chat.worker_quiet_setting(), (300, None));
    fs::write(&path, "[chat]\nworker_quiet_secs = 45\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.chat.worker_quiet_setting(), (45, None));
    for value in ["0", "-5", "1.5", "\"x\""] {
        fs::write(&path, format!("[chat]\nworker_quiet_secs = {value}\n")).unwrap();
        let config = AppConfig::load_layers(&path, None).unwrap();
        let (secs, warning) = config.chat.worker_quiet_setting();
        assert_eq!(secs, 300);
        assert!(warning.is_some_and(|text| text.contains("positive integer")));
    }
}

#[test]
fn chat_section_parses_inbox_alert() {
    let defaults = AppConfig::new().chat;
    assert!(defaults.inbox_alert_sound);
    assert!(!defaults.inbox_alert_notification);
    assert_eq!(defaults.inbox_alert_unknown_focus, UnknownFocus::Alert);
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(
        &path,
        "[chat]\ninbox_alert_sound = false\ninbox_alert_notification = true\ninbox_alert_unknown_focus = \"silent\"\n",
    )
    .unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert!(!config.chat.inbox_alert_sound);
    assert!(config.chat.inbox_alert_notification);
    assert_eq!(config.chat.inbox_alert_unknown_focus, UnknownFocus::Silent);
    fs::write(&path, "[chat]\ninbox_alert_unknown_focus = \"loud\"\n").unwrap();
    assert!(AppConfig::load_layers(&path, None).is_err());
}

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
fn chat_section_parses_status_powerline() {
    assert!(!AppConfig::new().chat.status_powerline);
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(&path, "[chat]\nstatus_powerline = true\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert!(config.chat.status_powerline);
}

#[test]
fn chat_section_parses_worker_base() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(&path, "[chat]\nworker_base = \"develop\"\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.chat.worker_base.as_deref(), Some("develop"));
    assert_eq!(AppConfig::new().chat.worker_base, None);
}

#[test]
fn chat_section_parses_worktree_dir() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, "[chat]\nworktree_dir = \"../wt\"\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.chat.worktree_dir.as_deref(), Some("../wt"));
    assert_eq!(AppConfig::new().chat.worktree_dir, None);
}

#[test]
fn chat_section_parses_mcp_servers() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(&path, "[chat]\nmcp_servers = [\"a\", \"b\"]\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.chat.mcp_servers, ["a", "b"]);
}

#[test]
fn chat_section_parses_skills() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(&path, "[chat]\nskills = [\"~/a\", \"/b\"]\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(
        config.chat.skills,
        [PathBuf::from("~/a"), PathBuf::from("/b")]
    );
    assert_eq!(config.chat.mcp_servers, ["mcptools"]);
}

#[test]
fn test_load_nonexistent_returns_default() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("nonexistent.toml");

    let config = AppConfig::load_layers(&path, None).unwrap();
    assert!(config.theme.name.is_none());
    assert!(!config.theme.no_background);
}

#[test]
fn test_load_empty_file() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");

    fs::write(&path, "").unwrap();

    let config = AppConfig::load_layers(&path, None).unwrap();
    assert!(config.theme.name.is_none());
}

#[test]
fn test_load_theme_name() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");

    fs::write(
        &path,
        r#"
        [theme]
        name = "Monokai Extended"
        "#,
    )
    .unwrap();

    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.theme.name, Some("Monokai Extended".to_string()));
    assert!(!config.theme.no_background);
    assert_eq!(config.theme.agent, None);
    assert_eq!(config.theme.assistant, RoleTheme::default());
}

#[test]
fn legacy_agent_key_keeps_working_with_a_warning() {
    let config: AppConfig = toml::from_str("[theme]\nagent = \"Solarized (dark)\"\n").unwrap();
    assert_eq!(
        config.theme.agent().code.as_deref(),
        Some("Solarized (dark)")
    );
    assert_eq!(
        config.theme.deprecations(),
        ["[theme] agent = \"Solarized (dark)\" is deprecated; move it to [theme.agent] code = \"Solarized (dark)\""]
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
fn test_load_theme_file_path() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");

    fs::write(
        &path,
        r#"
        [theme]
        name = "/path/to/my-theme.tmTheme"
        "#,
    )
    .unwrap();

    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(
        config.theme.name,
        Some("/path/to/my-theme.tmTheme".to_string())
    );
}

#[test]
fn test_load_no_background() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");

    fs::write(
        &path,
        r#"
        [theme]
        no_background = true
        "#,
    )
    .unwrap();

    let config = AppConfig::load_layers(&path, None).unwrap();
    assert!(config.theme.no_background);
}

#[test]
fn test_load_full_config() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");

    fs::write(
        &path,
        r#"
        [theme]
        name = "base16-eighties.dark"
        no_background = true
        "#,
    )
    .unwrap();

    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.theme.name, Some("base16-eighties.dark".to_string()));
    assert!(config.theme.no_background);
}

#[test]
fn tickets_parse_and_default_to_empty() {
    let config: AppConfig =
        toml::from_str("[tickets]\nlinear = [\"GUZ\"]\njira = [\"VP20\", \"KODA\"]\n").unwrap();
    assert_eq!(config.tickets.linear, ["GUZ"]);
    assert_eq!(config.tickets.jira, ["VP20", "KODA"]);
    assert_eq!(AppConfig::new().tickets, TicketKeys::default());
    let partial: AppConfig = toml::from_str("[tickets]\njira = [\"KODA\"]\n").unwrap();
    assert!(partial.tickets.linear.is_empty());
}

#[test]
fn test_save_and_load() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("subdir").join("config.toml");

    let config = AppConfig {
        theme: ThemeSection {
            name: Some("Monokai Extended".to_string()),
            no_background: true,
            assistant: RoleTheme::default(),
            agent: None,
            status: StatusTheme::default(),
            input: InputTheme::default(),
        },
        ..AppConfig::new()
    };

    config.save_to(&path).unwrap();

    let loaded = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(loaded.theme.name, Some("Monokai Extended".to_string()));
    assert!(loaded.theme.no_background);
}

#[test]
fn test_parse_error() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");

    fs::write(&path, "invalid toml {{{{").unwrap();

    let result = AppConfig::load_layers(&path, None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, ConfigError::ParseConfig { .. }));
}

#[test]
fn test_unknown_fields_ignored() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");

    fs::write(
        &path,
        r#"
        [theme]
        name = "Monokai"
        unknown_field = "ignored"

        [unknown_section]
        foo = "bar"
        "#,
    )
    .unwrap();

    // Should not error on unknown fields
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.theme.name, Some("Monokai".to_string()));
}

#[test]
fn test_serialize_omits_none() {
    let config = AppConfig {
        theme: ThemeSection {
            name: None,
            no_background: false,
            assistant: RoleTheme::default(),
            agent: None,
            status: StatusTheme::default(),
            input: InputTheme::default(),
        },
        ..AppConfig::new()
    };

    let content = toml::to_string_pretty(&config).unwrap();
    // Should not contain "name" since it's None
    assert!(!content.contains("name"));
}

#[test]
fn test_config_path_contains_ralph() {
    let path = config_path();
    assert!(path.to_string_lossy().contains("ralph"));
    assert!(path.to_string_lossy().ends_with("config.toml"));
}

#[test]
fn test_default_permission_modes() {
    let config = AppConfig::new();
    assert!(config.chat.permission_mode.is_none());
    assert!(config.ask.permission_mode.is_none());
}

#[test]
fn test_load_chat_and_ask_permission_modes_separately() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");

    fs::write(
        &path,
        r#"
        [chat]
        permission_mode = "plan"

        [ask]
        permission_mode = "acceptEdits"
        "#,
    )
    .unwrap();

    let config = AppConfig::load_layers(&path, None).unwrap();
    assert_eq!(config.chat.permission_mode, Some("plan".to_string()));
    assert_eq!(config.ask.permission_mode, Some("acceptEdits".to_string()));
}

fn configured(chat: Option<&str>, ask: Option<&str>) -> AppConfig {
    let mut config = AppConfig::new();
    config.chat.permission_mode = chat.map(str::to_string);
    config.ask.permission_mode = ask.map(str::to_string);
    config
}

#[test]
fn chat_permission_mode_precedence() {
    let config = configured(Some("plan"), Some("dontAsk"));
    assert_eq!(
        config.chat_permission_mode(Some("manual"), Some("acceptEdits")),
        "manual"
    );
    assert_eq!(
        config.chat_permission_mode(None, Some("acceptEdits")),
        "acceptEdits"
    );
    assert_eq!(config.chat_permission_mode(None, Some("")), "plan");
    assert_eq!(config.chat_permission_mode(None, None), "plan");
    assert_eq!(
        configured(None, Some("dontAsk")).chat_permission_mode(None, None),
        "auto"
    );
}

#[test]
fn ask_permission_mode_precedence() {
    let config = configured(Some("plan"), Some("dontAsk"));
    assert_eq!(
        config.ask_permission_mode(Some("manual"), Some("acceptEdits")),
        "manual"
    );
    assert_eq!(
        config.ask_permission_mode(None, Some("acceptEdits")),
        "acceptEdits"
    );
    assert_eq!(config.ask_permission_mode(None, Some("")), "dontAsk");
    assert_eq!(config.ask_permission_mode(None, None), "dontAsk");
    assert_eq!(
        configured(Some("plan"), None).ask_permission_mode(None, None),
        "bypassPermissions"
    );
}

fn write_layer(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn store_layer_overrides_per_key_and_keeps_sibling_global_keys() {
    let dir = TempDir::new().unwrap();
    let global = write_layer(
        &dir,
        "g.toml",
        "[chat]\nwrap_width = 80\nworker_base = \"dev\"\n",
    );
    let store = write_layer(&dir, "s.toml", "[chat]\nwrap_width = 100\n");
    let config = AppConfig::load_layers(&global, Some(&store)).unwrap();
    assert_eq!(config.chat.wrap_width, Some(100));
    assert_eq!(config.chat.worker_base.as_deref(), Some("dev"));
}

#[test]
fn missing_layers_are_empty() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("none.toml");
    let config = AppConfig::load_layers(&missing, Some(&missing)).unwrap();
    assert_eq!(config.chat.mcp_servers, ["mcptools"]);
}

#[test]
fn layer_errors_name_the_offending_file() {
    let dir = TempDir::new().unwrap();
    let global = write_layer(&dir, "g.toml", "[chat]\nwrap_width = 80\n");
    let broken = write_layer(&dir, "s.toml", "[chat\n");
    let typed = write_layer(&dir, "t.toml", "[chat]\nwrap_width = \"wide\"\n");
    let syntax = AppConfig::load_layers(&global, Some(&broken)).unwrap_err();
    assert!(syntax.to_string().contains("s.toml"));
    let typing = AppConfig::load_layers(&global, Some(&typed)).unwrap_err();
    assert!(typing.to_string().contains("t.toml"));
}

#[test]
fn relative_skills_resolve_per_file_and_store_replaces_global() {
    let dir = TempDir::new().unwrap();
    let global = write_layer(&dir, "g.toml", "[chat]\nskills = [\"gs\"]\n");
    let sub = dir.path().join("store");
    fs::create_dir(&sub).unwrap();
    let store = sub.join("config.toml");
    fs::write(&store, "[chat]\nskills = [\"ss\", \"~/h\"]\n").unwrap();
    assert_eq!(
        AppConfig::load_layers(&global, None).unwrap().chat.skills,
        [dir.path().join("gs")]
    );
    assert_eq!(
        AppConfig::load_layers(&global, Some(&store))
            .unwrap()
            .chat
            .skills,
        [sub.join("ss"), PathBuf::from("~/h")]
    );
}

fn layer_files(global: &str, store: &str) -> (TempDir, PathBuf, PathBuf) {
    let dir = TempDir::new().unwrap();
    let global_path = dir.path().join("global.toml");
    let store_path = dir.path().join("store.toml");
    fs::write(&global_path, global).unwrap();
    fs::write(&store_path, store).unwrap();
    (dir, global_path, store_path)
}

#[test]
fn broken_store_falls_back_to_global_with_a_notice() {
    let (_dir, global, store) = layer_files("[theme]\nname = \"global\"\n", "[chat\n");
    let (result, notices) = AppConfig::load_from(&global, Some(&store));
    assert_eq!(result.unwrap().theme.name.as_deref(), Some("global"));
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].key,
        format!("store-fallback:{}", store.display())
    );
    assert!(notices[0].message.contains(&store.display().to_string()));
    assert!(notices[0].message.ends_with("using the global config only"));
}

#[test]
fn broken_global_still_fails() {
    let (_dir, global, store) = layer_files("[chat\n", "[theme]\nname = \"store\"\n");
    let (result, notices) = AppConfig::load_from(&global, Some(&store));
    assert!(matches!(result, Err(ConfigError::ParseConfig { .. })));
    assert!(notices.is_empty());
}

#[test]
fn valid_store_has_no_notice() {
    let (_dir, global, store) = layer_files(
        "[theme]\nname = \"global\"\n",
        "[theme]\nname = \"store\"\n",
    );
    let (result, notices) = AppConfig::load_from(&global, Some(&store));
    assert_eq!(result.unwrap().theme.name.as_deref(), Some("store"));
    assert!(notices.is_empty());
}

#[test]
fn store_security_key_yields_a_notice() {
    let (_dir, global, store) =
        layer_files("", "[chat]\npermission_mode = \"bypassPermissions\"\n");
    let (result, notices) = AppConfig::load_from(&global, Some(&store));
    assert!(result.is_ok());
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].key, "security:chat.permission_mode");
    assert_eq!(
        notices[0].message,
        format!(
            "store config {} sets chat.permission_mode; check it before you trust this store",
            store.display()
        )
    );
}

#[test]
fn value_errors_name_the_layer_that_owns_the_merged_error() {
    let dir = TempDir::new().unwrap();
    let global = write_layer(&dir, "g.toml", "[wire.prices.m]\ninput = -1\n");
    let store = write_layer(&dir, "s.toml", "[workers]\ncache_secs = 0\n");
    let message = AppConfig::load_layers(&global, Some(&store))
        .unwrap_err()
        .to_string();
    assert!(message.contains(&global.display().to_string()));
    assert!(!message.contains(&store.display().to_string()));
    assert!(message.contains("must be zero or positive"));
}

#[test]
fn valid_layers_load_without_value_errors() {
    let dir = TempDir::new().unwrap();
    let global = write_layer(&dir, "g.toml", "[wire.prices.m]\ninput = 1\n");
    let store = write_layer(&dir, "s.toml", "[chat]\nwrap_width = 100\n");
    let config = AppConfig::load_layers(&global, Some(&store)).unwrap();
    assert_eq!(config.chat.wrap_width, Some(100));
}
