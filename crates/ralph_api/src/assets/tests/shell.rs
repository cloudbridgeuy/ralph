use super::*;

#[test]
fn the_shell_is_html() {
    assert_eq!(find("/").unwrap().content_type, "text/html; charset=utf-8");
    assert!(bytes_of("/").starts_with(b"<!doctype html>"));
}

fn nav_links(html: &str) -> Vec<(String, String)> {
    let start = html.find("<nav").unwrap();
    let end = html.find("</nav>").unwrap();
    html[start..end]
        .split("<a href=\"")
        .skip(1)
        .map(|link| {
            let (href, rest) = link.split_once("\">").unwrap();
            let (inner, _) = rest.split_once("</a>").unwrap();
            (href.to_string(), inner.to_string())
        })
        .collect()
}

fn screen_routes(js: &str) -> Vec<String> {
    let line = js
        .lines()
        .find(|l| l.starts_with("const SCREENS = ["))
        .unwrap();
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(String::from)
        .collect()
}

#[test]
fn nav_links_read_href_and_inner_html_of_each_anchor() {
    let html = "<nav>\n<a href=\"#/a\"><span class=\"n\">1</span>a</a>\n<a href=\"/b\">b</a>\n</nav><a href=\"/x\">x</a>";
    assert_eq!(
        nav_links(html),
        vec![
            ("#/a".to_string(), "<span class=\"n\">1</span>a".to_string()),
            ("/b".to_string(), "b".to_string())
        ]
    );
}

#[test]
fn screen_routes_read_the_quoted_routes_of_the_screens_line() {
    assert_eq!(
        screen_routes("x\nconst SCREENS = [\"/a\", \"/b\"];\n"),
        vec!["/a", "/b"]
    );
}

#[test]
fn every_nav_href_maps_to_a_screen_or_the_catalogue() {
    let routes = screen_routes(&script());
    assert!(!routes.is_empty());
    let links = nav_links(&shell());
    for (href, _) in &links {
        let known = href == "/catalogue"
            || href
                .strip_prefix('#')
                .is_some_and(|r| routes.iter().any(|s| s == r));
        assert!(known, "{href}");
    }
    for route in &routes {
        assert!(
            links.iter().any(|(h, _)| h == &format!("#{route}")),
            "{route}"
        );
    }
}

#[test]
fn routes_without_a_screen_are_hidden_from_the_nav() {
    let html = shell();
    let hidden = "threads";
    assert!(!html.contains(&format!("#/{hidden}")), "{hidden}");
    assert!(
        !nav_links(&html).iter().any(|(_, t)| t.ends_with(hidden)),
        "{hidden}"
    );
}

#[test]
fn nav_tabs_are_numbered_from_one_in_order() {
    let links = nav_links(&shell());
    let labels: Vec<&str> = links.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "<span class=\"n\" aria-hidden=\"true\">1</span>memory",
            "<span class=\"n\" aria-hidden=\"true\">2</span>config",
            "<span class=\"n\" aria-hidden=\"true\">3</span>workers",
            "<span class=\"n\" aria-hidden=\"true\">4</span>schedules",
            "<span class=\"n\" aria-hidden=\"true\">5</span>watches",
            "<span class=\"n\" aria-hidden=\"true\">6</span>catalogue"
        ]
    );
}

#[test]
fn the_header_carries_a_read_only_badge_and_an_empty_store_slot() {
    let html = shell();
    let header = &html[html.find("<header").unwrap()..html.find("</header>").unwrap()];
    assert!(header.contains("<span class=\"badge\">read-only</span>"));
    assert!(header.contains("<span class=\"store\" id=\"store\"></span>"));
}

#[test]
fn the_script_sets_the_store_name_as_text_from_the_threads_reply() {
    let js = script();
    assert!(js.contains("getElementById(\"store\").textContent = threads.store"));
}

#[test]
fn the_badge_and_store_use_existing_roles_only() {
    let css = css();
    let badge = rule_body(&css, ".badge").unwrap();
    assert!(badge.contains("border: 1px solid var(--r-line-strong);"));
    assert!(badge.contains("color: var(--r-muted);"));
    assert!(badge.contains("font-size: 12px;"));
    assert!(!badge.contains('#'));
    let store = rule_body(&css, ".store").unwrap();
    assert!(store.contains("color: var(--r-muted);"));
    assert!(store.contains("margin-left: auto;"));
    assert!(!store.contains("--r-faint"));
}

#[test]
fn the_script_marks_the_current_tab_and_falls_back_to_the_first_screen() {
    let js = script();
    assert!(js.contains("\"aria-current\""));
    assert!(js.contains("SCREENS.includes(state.route)"));
    assert!(!js.contains("No screens yet."));
    assert!(!shell().contains("No screens yet."));
}

#[test]
fn the_current_tab_is_ink_strong_with_an_accent_underline_and_numbers_are_muted() {
    let css = css();
    assert_declares(
        &css,
        ".nav a[aria-current=\"page\"]",
        "color: var(--r-ink-strong);",
    );
    assert_declares(&css, ".nav a[aria-current=\"page\"]", "font-weight: 600;");
    assert_declares(
        &css,
        ".nav a[aria-current=\"page\"]",
        "border-bottom-color: var(--r-accent);",
    );
    assert_declares(&css, ".nav .n", "color: var(--r-muted);");
    assert_declares(&css, ".nav .n", "margin-right: 1ch;");
}

#[test]
fn the_stylesheet_is_css() {
    assert_eq!(
        find("/assets/app.css").unwrap().content_type,
        "text/css; charset=utf-8"
    );
    assert!(!bytes_of("/assets/app.css").is_empty());
}

#[test]
fn the_font_is_the_upstream_woff2() {
    let font = find("/assets/geist-mono.woff2").unwrap();
    assert_eq!(font.content_type, "font/woff2");
    assert!(font.bytes.starts_with(b"wOF2"));
    assert_eq!(font.bytes.len(), 71_212);
}

#[test]
fn unknown_and_traversal_paths_find_nothing() {
    for path in [
        "/nope",
        "/assets/",
        "/assets/../index.html",
        "/assets/app.css/",
        "",
        "/index.html",
    ] {
        assert!(find(path).is_none(), "{path}");
    }
}

#[test]
fn every_path_is_unique_and_absolute() {
    assert!(!ASSETS.is_empty());
    for (at, asset) in ASSETS.iter().enumerate() {
        assert!(asset.path.starts_with('/'), "{}", asset.path);
        assert!(!asset.bytes.is_empty(), "{}", asset.path);
        assert!(ASSETS[at + 1..]
            .iter()
            .all(|other| other.path != asset.path));
    }
}
