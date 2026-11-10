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
    assert!(svg.contains(">(model missing) n/a<"), "{svg}");
    assert!(!svg.contains("$0"), "{svg}");
}
