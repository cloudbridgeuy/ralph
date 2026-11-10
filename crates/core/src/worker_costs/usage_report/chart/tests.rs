use super::*;
use crate::worker_costs::{Usd, NO_USAGE};

const HEADING: Heading = Heading {
    title: "ralph usage",
    subtitle: "test data",
};

fn est(usd: f64) -> Usd {
    Usd {
        known: Some(usd),
        estimate: true,
        ..Usd::default()
    }
}

fn group(key: &str, reported: Option<f64>, computed: Option<f64>) -> Group {
    Group {
        key: key.to_string(),
        metered: 1,
        reported: reported.map_or_else(Usd::default, Usd::reported),
        est: computed.map_or_else(Usd::default, est),
        ..Group::default()
    }
}

fn heights(svg: &str, fill: &str) -> Vec<f64> {
    svg.split("<rect")
        .filter(|rect| rect.contains(&format!("fill=\"{fill}\"")))
        .filter_map(|rect| {
            let tail = rect.split("height=\"").nth(1)?;
            tail.split('"').next()?.parse().ok()
        })
        .collect()
}

#[test]
fn days_without_usage_are_padded_to_zero() {
    let days = [
        group("2026-11-05", None, Some(1.0)),
        group("2026-11-08", None, Some(2.0)),
    ];
    let padded = pad_days(&days);
    let keys: Vec<_> = padded.iter().map(|day| day.key.as_str()).collect();
    assert_eq!(
        keys,
        ["2026-11-05", "2026-11-06", "2026-11-07", "2026-11-08"]
    );
    assert_eq!(padded[1].metered, 0);
    let svg = daily_svg(&days, &HEADING);
    assert!(svg.contains(">11-06<") && svg.contains(">11-07<"), "{svg}");
}

#[test]
fn the_daily_legend_names_the_est_series_and_colours_its_bars() {
    let days = [
        group("2026-11-05", Some(1.0), None),
        group("2026-11-06", None, Some(1.0)),
    ];
    let svg = daily_svg(&days, &HEADING);
    assert!(
        svg.contains(">est.<") && svg.contains(">reported<"),
        "{svg}"
    );
    assert_eq!(heights(&svg, REPORTED_FILL).len(), 1);
    assert_eq!(heights(&svg, EST_FILL).len(), 1);
}

#[test]
fn daily_bar_heights_scale_with_dollars() {
    let days = [
        group("2026-11-05", None, Some(1.0)),
        group("2026-11-06", None, Some(2.0)),
    ];
    let bars = heights(&daily_svg(&days, &HEADING), EST_FILL);
    assert_eq!(bars.len(), 2);
    assert!((bars[1] / bars[0] - 2.0).abs() < 1e-6, "{bars:?}");
}

#[test]
fn an_empty_daily_chart_says_so() {
    assert!(daily_svg(&[], &HEADING).contains(NO_USAGE));
}

#[test]
fn a_missing_model_keeps_its_label_in_the_bars() {
    let svg = bars_svg(&[group("(model missing)", Some(3.0), None)], &HEADING);
    assert!(svg.contains(">(model missing)<"), "{svg}");
}

#[test]
fn a_group_with_no_rows_reads_no_data_never_zero_dollars() {
    let cursor = Group {
        key: "cursor".to_string(),
        unmetered: 1,
        ..Group::default()
    };
    let svg = bars_svg(&[cursor], &HEADING);
    assert!(svg.contains(">cursor: no data<"), "{svg}");
    assert!(!svg.contains("$0"), "{svg}");
}

#[test]
fn an_all_codex_group_reads_excluded_never_a_price() {
    let codex = Group {
        key: "codex".to_string(),
        metered: 2,
        excluded: 2,
        ..Group::default()
    };
    let svg = bars_svg(&[codex], &HEADING);
    assert!(svg.contains(">codex: excluded<"), "{svg}");
    assert!(!svg.contains("$0") && !svg.contains("n/a"), "{svg}");
}

#[test]
fn bar_values_carry_est_only_for_computed_dollars() {
    let svg = bars_svg(
        &[group("a", None, Some(5.0)), group("b", Some(5.0), None)],
        &HEADING,
    );
    assert!(svg.contains(">$5.00 est. (50%)<"), "{svg}");
    assert!(svg.contains(">$5.00 (50%)<"), "{svg}");
}

#[test]
fn labels_are_xml_escaped() {
    let svg = bars_svg(&[group("<b>&\"", Some(1.0), None)], &HEADING);
    assert!(svg.contains("&lt;b&gt;&amp;&quot;"), "{svg}");
    assert!(!svg.contains("<b>"), "{svg}");
}

#[test]
fn an_unpriced_group_reads_its_line_never_zero_dollars() {
    let unpriced = Group {
        key: "(model missing)".to_string(),
        metered: 3,
        est: Usd {
            missing: true,
            estimate: true,
            ..Usd::default()
        },
        ..Group::default()
    };
    let svg = bars_svg(&[unpriced], &HEADING);
    assert!(svg.contains(">(model missing): no price<"), "{svg}");
    assert!(!svg.contains("$0"), "{svg}");
}

fn attr<'a>(element: &'a str, name: &str) -> &'a str {
    element
        .split(&format!("{name}=\""))
        .nth(1)
        .and_then(|tail| tail.split('"').next())
        .unwrap_or_default()
}

fn right_edges(svg: &str) -> Vec<f64> {
    svg.split("<text ")
        .skip(1)
        .filter_map(|element| {
            let x: f64 = attr(element, "x").parse().ok()?;
            let size: u8 = attr(element, "font-size").parse().ok()?;
            let content = element.split('>').nth(1)?.split('<').next()?;
            let width = text_width(content, size);
            Some(match attr(element, "text-anchor") {
                "end" => x,
                "middle" => x + width / 2.0,
                _ => x + width,
            })
        })
        .collect()
}

fn day(key: &str, reported: Option<f64>, computed: Option<f64>) -> Group {
    group(key, reported, computed)
}

#[test]
fn value_labels_stay_inside_the_svg_width() {
    let groups = [
        group("GUZ-1", Some(1000.0), Some(2000.0)),
        group("GUZ-2", Some(50.0), Some(25.0)),
    ];
    let svg = bars_svg(&groups, &HEADING);
    let widest = right_edges(&svg).into_iter().fold(0.0, f64::max);
    assert!(widest <= 900.0, "right edge {widest} in {svg}");
}

#[test]
fn a_mixed_bar_reads_reported_plus_est() {
    let svg = bars_svg(&[group("a", Some(40.0), Some(12.8))], &HEADING);
    assert!(svg.contains(">$40.00 + $12.80 est. (100%)<"), "{svg}");
}

#[test]
fn an_all_reported_bar_reads_plain_dollars() {
    let svg = bars_svg(&[group("a", Some(40.0), None)], &HEADING);
    assert!(svg.contains(">$40.00 (100%)<"), "{svg}");
    assert!(!svg.contains("est. ("), "{svg}");
}

#[test]
fn an_all_est_bar_reads_dollars_est() {
    let svg = bars_svg(&[group("a", None, Some(12.8))], &HEADING);
    assert!(svg.contains(">$12.80 est. (100%)<"), "{svg}");
}

#[test]
fn daily_value_labels_are_dollars() {
    let svg = daily_svg(&[day("2026-11-05", Some(3.2), None)], &HEADING);
    assert!(svg.contains(">$3.20<"), "{svg}");
}

#[test]
fn a_day_with_no_rows_has_no_label_and_no_bar() {
    let days = [
        day("2026-11-05", Some(2.3), None),
        day("2026-11-07", Some(2.3), None),
    ];
    let svg = daily_svg(&days, &HEADING);
    assert_eq!(svg.matches(">$2.30<").count(), 2, "{svg}");
    assert_eq!(heights(&svg, REPORTED_FILL).len(), 2, "{svg}");
}

#[test]
fn a_day_whose_rows_have_no_price_reads_no_price() {
    let unpriced = Group {
        key: "2026-11-05".to_string(),
        metered: 2,
        est: Usd {
            missing: true,
            estimate: true,
            ..Usd::default()
        },
        ..Group::default()
    };
    let svg = daily_svg(&[unpriced], &HEADING);
    assert!(svg.contains(">no price<"), "{svg}");
    assert!(heights(&svg, EST_FILL).is_empty(), "{svg}");
}

fn partial(key: &str, reported: Option<f64>, computed: Option<f64>) -> Group {
    let mut group = group(key, reported, computed);
    group.est.missing = true;
    group
}

#[test]
fn a_partly_priced_bar_reads_no_price_never_a_trailing_plus() {
    let svg = bars_svg(&[partial("a", None, Some(5.0))], &HEADING);
    assert!(svg.contains(">$5.00 est. + no price (100%)<"), "{svg}");
    let mixed = bars_svg(&[partial("a", Some(5.0), Some(5.0))], &HEADING);
    assert!(
        mixed.contains(">$5.00 + $5.00 est. + no price (100%)<"),
        "{mixed}"
    );
}

#[test]
fn a_partly_priced_day_says_so_under_its_dollars() {
    let svg = daily_svg(&[partial("2026-11-05", Some(3.2), None)], &HEADING);
    assert!(
        svg.contains(">$3.20<") && svg.contains(">+ no price<"),
        "{svg}"
    );
}
