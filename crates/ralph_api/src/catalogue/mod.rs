use crate::render::{
    empty, error, escape, frame, kv, pill, rule, spark_svg, spinner, status, table, Icon,
    PillShape, Shape, Tone,
};
use fixture::{parse, Fixture};

mod fixture;

const TONES: [Tone; 5] = [Tone::Ok, Tone::Bad, Tone::Warn, Tone::Info, Tone::Accent];
const ICONS: [Icon; 4] = [Icon::Check, Icon::Cross, Icon::Warning, Icon::Info];

fn state(name: &str, html: &str) -> String {
    format!("<div class=\"state\"><h4>{name}</h4>{html}</div>")
}

fn group(tag: &str, states: &[(&str, String)]) -> String {
    let body: String = states
        .iter()
        .map(|(name, html)| state(name, html))
        .collect();
    format!("<div class=\"group\"><h3>{tag}</h3>{body}</div>")
}

fn text(content: &str) -> String {
    format!("<p>{}</p>", escape(content))
}

fn pairs(rows: &[(String, u64)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(name, value)| (name.clone(), value.to_string()))
        .collect()
}

fn borrowed(rows: &[(String, String)]) -> Vec<(&str, &str)> {
    rows.iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

fn first_title(fixture: &Fixture) -> &str {
    fixture.titles.first().map_or("untitled", String::as_str)
}

fn cost_rows(fixture: &Fixture) -> Vec<Vec<String>> {
    fixture
        .costs
        .iter()
        .map(|(day, turns, cost)| vec![day.clone(), turns.to_string(), cost.to_string()])
        .collect()
}

fn table_html(headers: &[&str], rows: &[Vec<String>]) -> String {
    let cells: Vec<Vec<&str>> = rows
        .iter()
        .map(|row| row.iter().map(String::as_str).collect())
        .collect();
    table(headers, &cells)
}

fn boxes(fixture: &Fixture) -> String {
    let long = &fixture.very_long_title;
    group(
        "r-box",
        &[
            ("empty", frame(Shape::Square, None, "")),
            (
                "sparse",
                frame(Shape::Round, Some(first_title(fixture)), &text("one line")),
            ),
            (
                "dense",
                frame(
                    Shape::Heavy,
                    Some("Counts"),
                    &kv(&borrowed(&pairs(&fixture.counts))),
                ),
            ),
            (
                "error",
                frame(Shape::Square, Some("Failed"), &error("could not load")),
            ),
            ("very-long", frame(Shape::Square, Some(long), &text(long))),
        ],
    )
}

fn rules() -> String {
    group("r-rule", &[("sparse", rule())])
}

fn tables(fixture: &Fixture) -> String {
    let headers = ["day", "turns", "cost"];
    let rows = cost_rows(fixture);
    let one = rows.iter().take(1).cloned().collect::<Vec<_>>();
    let long = vec![vec![fixture.very_long_title.clone(); 3]];
    group(
        "r-table",
        &[
            ("empty", table_html(&headers, &[])),
            ("sparse", table_html(&headers, &one)),
            ("dense", table_html(&headers, &rows)),
            ("error", error("could not load rows")),
            ("very-long", table_html(&headers, &long)),
        ],
    )
}

fn kvs(fixture: &Fixture) -> String {
    let long = fixture.very_long_title.as_str();
    let counts = pairs(&fixture.counts);
    let mut all = counts.clone();
    all.extend(pairs(&fixture.durations));
    group(
        "r-kv",
        &[
            ("empty", kv(&[])),
            ("sparse", kv(&borrowed(&counts)[..counts.len().min(2)])),
            ("dense", kv(&borrowed(&all))),
            ("error", error("could not load pairs")),
            ("very-long", kv(&[(long, long)])),
        ],
    )
}

fn pills(fixture: &Fixture) -> String {
    let dense: String = TONES
        .iter()
        .enumerate()
        .flat_map(|(i, tone)| {
            [PillShape::Square, PillShape::Round]
                .into_iter()
                .map(move |shape| (i, *tone, shape))
        })
        .map(|(i, tone, shape)| pill(tone, shape, Some(ICONS[i % ICONS.len()]), tone.as_str()))
        .collect();
    group(
        "r-pill",
        &[
            ("empty", pill(Tone::Info, PillShape::Square, None, "")),
            (
                "sparse",
                pill(
                    Tone::Ok,
                    PillShape::Round,
                    Some(Icon::Check),
                    first_title(fixture),
                ),
            ),
            ("dense", dense),
            (
                "error",
                pill(Tone::Bad, PillShape::Square, Some(Icon::Cross), "failed"),
            ),
            (
                "very-long",
                pill(
                    Tone::Accent,
                    PillShape::Round,
                    None,
                    &fixture.very_long_title,
                ),
            ),
        ],
    )
}

fn statuses(fixture: &Fixture) -> String {
    let dense: String = TONES
        .iter()
        .map(|tone| status(*tone, tone.as_str()))
        .collect();
    group(
        "r-status",
        &[
            ("empty", status(Tone::Warn, "")),
            ("sparse", status(Tone::Ok, first_title(fixture))),
            ("dense", dense),
            ("error", status(Tone::Bad, "disk full")),
            ("very-long", status(Tone::Info, &fixture.very_long_title)),
        ],
    )
}

fn empties(fixture: &Fixture) -> String {
    group(
        "r-empty",
        &[
            ("empty", empty("")),
            ("sparse", empty("No sessions yet")),
            ("very-long", empty(&fixture.very_long_title)),
        ],
    )
}

fn errors(fixture: &Fixture) -> String {
    group(
        "r-error",
        &[
            ("empty", error("")),
            ("error", error("could not read the store")),
            ("very-long", error(&fixture.very_long_title)),
        ],
    )
}

fn spinners() -> String {
    group("r-spinner", &[("sparse", spinner())])
}

fn sparks(fixture: &Fixture) -> String {
    let costs: Vec<f64> = fixture
        .costs
        .iter()
        .map(|(_, _, cost)| *cost as f64)
        .collect();
    let svg =
        |values: &[f64]| spark_svg(values, 160.0, 24.0).unwrap_or_else(|| empty("No cost data"));
    group(
        "r-spark",
        &[
            ("empty", svg(&[])),
            ("sparse", svg(&costs[..costs.len().min(3)])),
            ("dense", svg(&costs)),
            ("error", error("could not load the series")),
            ("very-long", svg(&costs.repeat(10))),
        ],
    )
}

fn theme_body(fixture: &Fixture) -> String {
    [
        boxes(fixture),
        rules(),
        tables(fixture),
        kvs(fixture),
        pills(fixture),
        statuses(fixture),
        empties(fixture),
        errors(fixture),
        spinners(),
        sparks(fixture),
    ]
    .concat()
}

fn column(theme: &str, body: &str) -> String {
    format!("<section data-theme=\"{theme}\" aria-label=\"{theme} theme\"><h2>{theme}</h2>{body}</section>")
}

pub(crate) fn page(fixture_json: &str) -> String {
    let body = match parse(fixture_json) {
        Ok(fixture) => theme_body(&fixture),
        Err(message) => error(&format!("catalogue fixture: {message}")),
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<meta name=\"color-scheme\" content=\"dark light\">\n<title>ralph catalogue</title>\n<link rel=\"stylesheet\" href=\"/tokens.css\">\n<link rel=\"stylesheet\" href=\"/assets/app.css\">\n</head>\n<body>\n<header class=\"bar\">\n<span class=\"brand\">ralph web</span>\n<span class=\"note\">catalogue</span>\n</header>\n<nav class=\"nav\" aria-label=\"Screens\">\n<a href=\"/\">shell</a>\n</nav>\n<main id=\"main\">\n<div class=\"columns\">\n{}\n{}\n</div>\n</main>\n</body>\n</html>\n",
        column("dark", &body),
        column("light", &body)
    )
}

#[cfg(test)]
mod tests {
    use super::page;

    const COMMITTED: &str = include_str!("../../fixtures/catalogue.json");
    const TAGS: [&str; 10] = [
        "r-box",
        "r-rule",
        "r-table",
        "r-kv",
        "r-pill",
        "r-status",
        "r-empty",
        "r-error",
        "r-spinner",
        "r-spark",
    ];

    fn html() -> String {
        page(COMMITTED)
    }

    fn markup_only(html: &str) -> String {
        let mut out = String::new();
        let mut in_tag = false;
        for c in html.chars() {
            match (in_tag, c) {
                (false, '<') => {
                    in_tag = true;
                    out.push(c);
                }
                (true, '>') => {
                    in_tag = false;
                    out.push(c);
                }
                (true, _) => out.push(c),
                (false, _) => {}
            }
        }
        out
    }

    fn colour_hits(html: &str) -> Vec<String> {
        let html = &markup_only(html);
        let mut hits = Vec::new();
        for (index, _) in html.match_indices('#') {
            let run = html[index + 1..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .count();
            if matches!(run, 3 | 4 | 6 | 8) {
                hits.push(format!("hex at {index}"));
            }
        }
        for function in ["rgb(", "rgba(", "hsl(", "hsla(", "oklch(", "color-mix("] {
            if html.contains(function) {
                hits.push(function.to_string());
            }
        }
        for attribute in ["fill", "stroke", "color", "background"] {
            for (index, _) in html.match_indices(&format!(" {attribute}=\"")) {
                let rest = &html[index + attribute.len() + 3..];
                let value = rest.split('"').next().unwrap_or("");
                if value != "none" && value != "currentColor" {
                    hits.push(format!("{attribute}={value}"));
                }
            }
        }
        hits
    }

    #[test]
    fn text_that_looks_like_a_colour_is_not_a_colour() {
        assert!(colour_hits("<p>Fix PR #1234</p>").is_empty());
        assert!(colour_hits("<p>#abc and rgb(1,2,3)</p>").is_empty());
        assert!(colour_hits("<p>a fill=\"red\" b</p>").is_empty());
    }

    #[test]
    fn a_title_like_a_colour_does_not_fail_the_page() {
        let fixture = COMMITTED.replacen(
            "\"titles\": [",
            "\"titles\": [\"Fix PR #1234\", \"#abc\", ",
            1,
        );
        assert_ne!(fixture, COMMITTED);
        assert!(page(&fixture).contains("Fix PR #1234"));
        assert_eq!(colour_hits(&page(&fixture)), Vec::<String>::new());
    }

    #[test]
    fn two_theme_columns_dark_then_light() {
        let html = html();
        let dark = html.find("<section data-theme=\"dark\"").unwrap();
        let light = html.find("<section data-theme=\"light\"").unwrap();
        assert!(dark < light);
        assert_eq!(html.matches("<section data-theme=").count(), 2);
    }

    #[test]
    fn every_primitive_is_rendered_in_both_themes() {
        let html = html();
        for tag in TAGS {
            assert!(
                html.matches(&format!("<{tag}")).count() >= 2,
                "{tag} missing"
            );
        }
    }

    #[test]
    fn every_state_is_labelled_in_both_themes() {
        let html = html();
        for state in ["empty", "sparse", "dense", "error", "very-long"] {
            assert_eq!(
                html.matches(&format!("<h4>{state}</h4>")).count() % 2,
                0,
                "{state}"
            );
            assert!(html.contains(&format!("<h4>{state}</h4>")), "{state}");
        }
    }

    #[test]
    fn the_page_has_no_inline_style() {
        assert!(!html().contains("style="));
    }

    #[test]
    fn the_page_has_no_raw_colour() {
        assert_eq!(colour_hits(&html()), Vec::<String>::new());
    }

    #[test]
    fn the_colour_scan_catches_what_it_should() {
        assert!(!colour_hits("<p title=\"#fff\">x</p>").is_empty());
        assert!(!colour_hits("<p class=\"rgb(1,2,3)\">x</p>").is_empty());
        assert!(!colour_hits("<path fill=\"red\"/>").is_empty());
        assert!(colour_hits("<path fill=\"none\" stroke=\"currentColor\"/>").is_empty());
        assert!(colour_hits("a &#39; b").is_empty());
    }

    #[test]
    fn the_page_links_the_shared_stylesheets_and_back_to_the_shell() {
        let html = html();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<link rel=\"stylesheet\" href=\"/tokens.css\">"));
        assert!(html.contains("<link rel=\"stylesheet\" href=\"/assets/app.css\">"));
        assert!(html.contains("<a href=\"/\">shell</a>"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn a_bad_fixture_gives_an_error_page_and_still_two_columns() {
        let html = page("not json");
        assert!(html.contains("<r-error role=\"alert\">"));
        assert_eq!(html.matches("<section data-theme=").count(), 2);
    }

    #[test]
    fn hostile_fixture_text_is_escaped() {
        let text = r#"{"titles":["<script>alert(1)</script>"],"longest_title":"x","very_long_title_derived":"<b onclick=\"x\">","counts":{},"durations_ms":{},"cost_series_usd":[]}"#;
        let html = page(text);
        assert!(!html.contains("<script"));
        assert!(!html.contains("<b onclick"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn an_empty_series_renders_the_empty_state_for_the_sparkline() {
        let text = r#"{"titles":[],"longest_title":null,"very_long_title_derived":"v","counts":{},"durations_ms":{},"cost_series_usd":[]}"#;
        let html = page(text);
        assert!(!html.contains("<r-spark"));
        assert!(html.contains("<r-empty>"));
    }
}
