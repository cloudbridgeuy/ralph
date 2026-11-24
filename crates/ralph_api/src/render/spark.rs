fn dimension(size: f64) -> f64 {
    if size.is_finite() && size > 0.0 {
        size
    } else {
        0.0
    }
}

fn finite(values: &[f64]) -> Vec<f64> {
    values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect()
}

fn bounds(values: &[f64]) -> Option<(f64, f64)> {
    let min = values.iter().copied().reduce(f64::min)?;
    let max = values.iter().copied().reduce(f64::max)?;
    Some((min, max))
}

fn normalise(value: f64, min: f64, max: f64) -> f64 {
    let (low, high) = (min / 2.0, max / 2.0);
    let span = high - low;
    if span > 0.0 {
        ((value / 2.0 - low) / span).clamp(0.0, 1.0)
    } else {
        0.5
    }
}

fn points(values: &[f64], width: f64, height: f64) -> Vec<(f64, f64)> {
    let Some((min, max)) = bounds(values) else {
        return Vec::new();
    };
    let y = |value: f64| height - normalise(value, min, max) * height;
    match values {
        [only] => vec![(0.0, y(*only)), (width, y(*only))],
        _ => {
            let steps = (values.len() - 1) as f64;
            values
                .iter()
                .enumerate()
                .map(|(index, value)| (index as f64 * width / steps, y(*value)))
                .collect()
        }
    }
}

fn join(points: &[(f64, f64)]) -> String {
    points
        .iter()
        .map(|(x, y)| format!("{x:.2},{y:.2}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn number(value: f64) -> String {
    let value = value + 0.0;
    if value == 0.0 {
        return "0".to_string();
    }
    if (1e-3..1e9).contains(&value.abs()) {
        let fixed = format!("{value:.2}");
        return fixed
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();
    }
    format!("{value:.2e}")
}

fn bar_height(value: f64, max: f64, height: f64) -> f64 {
    if max > 0.0 {
        (value.max(0.0) / max * height).clamp(0.0, height)
    } else {
        0.0
    }
}

fn bars(values: &[f64], width: f64, height: f64) -> String {
    let max = bounds(values).map_or(0.0, |(_, max)| max);
    let slot = width / values.len() as f64;
    let gap = if slot > 2.0 { 1.0 } else { 0.0 };
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let bar = bar_height(*value, max, height);
            let class = if index + 1 == values.len() {
                "bar hi"
            } else {
                "bar"
            };
            format!(
                "<rect class=\"{class}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                number(index as f64 * slot),
                number(height - bar),
                number(slot - gap),
                number(bar)
            )
        })
        .collect()
}

fn wrap(values: &[f64], width: f64, height: f64, body: &str) -> Option<String> {
    let (min, max) = bounds(values)?;
    let last = values.last().copied()?;
    Some(format!(
        "<r-spark><svg class=\"spark\" role=\"img\" aria-label=\"min {}, max {}, last {}\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\">{body}</svg><b aria-hidden=\"true\">{}</b></r-spark>",
        number(min),
        number(max),
        number(last),
        number(width),
        number(height),
        number(width),
        number(height),
        number(last)
    ))
}

pub fn spark_bars_svg(values: &[f64], width: f64, height: f64) -> Option<String> {
    let values = finite(values);
    let (width, height) = (dimension(width), dimension(height));
    wrap(&values, width, height, &bars(&values, width, height))
}

pub fn spark_svg(values: &[f64], width: f64, height: f64) -> Option<String> {
    let values = finite(values);
    let (width, height) = (dimension(width), dimension(height));
    let line = points(&values, width, height);
    let (first, last) = (line.first()?, line.last()?);
    let area = [&[(first.0, height)][..], &line, &[(last.0, height)]].concat();
    let body = format!(
        "<polygon class=\"area\" points=\"{}\"/><line class=\"base\" x1=\"0\" y1=\"{h}\" x2=\"{w}\" y2=\"{h}\"/><polyline class=\"line\" stroke=\"currentColor\" points=\"{}\"/><circle class=\"last\" cx=\"{}\" cy=\"{}\" r=\"2\"/>",
        join(&area),
        join(&line),
        number(last.0),
        number(last.1),
        h = number(height),
        w = number(width)
    );
    wrap(&values, width, height, &body)
}

#[cfg(test)]
mod tests {
    use super::{dimension, finite, join, number, points, spark_bars_svg, spark_svg};

    fn spark_points(values: &[f64], width: f64, height: f64) -> String {
        join(&points(
            &finite(values),
            dimension(width),
            dimension(height),
        ))
    }
    use crate::render::golden::assert_golden;

    fn svg(values: &[f64]) -> String {
        spark_svg(values, 100.0, 20.0).unwrap()
    }

    #[test]
    fn empty_series_has_no_points_and_no_svg() {
        assert_eq!(spark_points(&[], 100.0, 20.0), "");
        assert_eq!(spark_svg(&[], 100.0, 20.0), None);
    }

    #[test]
    fn a_series_of_only_non_finite_values_is_empty() {
        let values = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
        assert_eq!(spark_points(&values, 100.0, 20.0), "");
        assert_eq!(spark_svg(&values, 100.0, 20.0), None);
    }

    #[test]
    fn one_point_draws_a_mid_line() {
        assert_eq!(spark_points(&[5.0], 100.0, 20.0), "0.00,10.00 100.00,10.00");
        assert_golden("spark_one_point", &svg(&[5.0]));
    }

    #[test]
    fn flat_series_draws_a_mid_line() {
        assert_eq!(
            spark_points(&[2.0, 2.0, 2.0], 100.0, 20.0),
            "0.00,10.00 50.00,10.00 100.00,10.00"
        );
        assert_golden("spark_flat", &svg(&[2.0, 2.0, 2.0]));
    }

    #[test]
    fn sparse_series() {
        assert_eq!(
            spark_points(&[1.0, 3.0, 2.0], 100.0, 20.0),
            "0.00,20.00 50.00,0.00 100.00,10.00"
        );
        assert_golden("spark_sparse", &svg(&[1.0, 3.0, 2.0]));
    }

    #[test]
    fn dense_series() {
        let values: Vec<f64> = (0..24).map(|i| f64::from((i * 7) % 11)).collect();
        assert_golden("spark_dense", &svg(&values));
    }

    #[test]
    fn non_finite_values_are_dropped() {
        let values = [1.0, f64::NAN, 3.0, f64::INFINITY, 2.0];
        assert_eq!(
            spark_points(&values, 100.0, 20.0),
            spark_points(&[1.0, 3.0, 2.0], 100.0, 20.0)
        );
        assert_golden("spark_nan_dropped", &svg(&values));
    }

    #[test]
    fn huge_values_do_not_overflow() {
        let values = [f64::MAX, -f64::MAX, 0.0];
        assert_eq!(
            spark_points(&values, 100.0, 20.0),
            "0.00,0.00 50.00,20.00 100.00,10.00"
        );
        assert_golden("spark_huge", &svg(&values));
    }

    #[test]
    fn every_point_stays_inside_the_box() {
        let values = [1e300, -1e300, 5.0, 1e-300, f64::MAX, -f64::MAX];
        for pair in spark_points(&values, 100.0, 20.0).split(' ') {
            let (x, y) = pair.split_once(',').unwrap();
            let (x, y): (f64, f64) = (x.parse().unwrap(), y.parse().unwrap());
            assert!(
                (0.0..=100.0).contains(&x) && (0.0..=20.0).contains(&y),
                "{pair}"
            );
        }
    }

    #[test]
    fn a_bad_box_gives_a_degenerate_but_finite_line() {
        let points = spark_points(&[1.0, 2.0], f64::NAN, -3.0);
        assert_eq!(points, "0.00,0.00 0.00,0.00");
    }

    #[test]
    fn svg_has_no_inline_style_and_no_raw_colour() {
        let html = svg(&[1.0, 2.0, 3.0]);
        assert!(!html.contains("style="), "{html}");
        assert!(!html.contains('#'), "{html}");
        assert!(html.contains("stroke=\"currentColor\""), "{html}");
        assert!(html.contains("role=\"img\""), "{html}");
    }

    #[test]
    fn the_line_has_a_baseline_an_area_an_accent_last_point_and_a_bold_last_value() {
        let html = svg(&[1.0, 3.0, 2.0]);
        assert!(html.contains("<line class=\"base\""), "{html}");
        assert!(html.contains("<polygon class=\"area\""), "{html}");
        assert!(
            html.contains("<circle class=\"last\" cx=\"100\" cy=\"10\""),
            "{html}"
        );
        assert!(
            html.ends_with("<b aria-hidden=\"true\">2</b></r-spark>"),
            "{html}"
        );
        assert_golden("spark_last_value", &html);
    }

    #[test]
    fn a_flat_series_puts_the_last_point_on_the_mid_line_without_nan() {
        let html = svg(&[2.0, 2.0, 2.0]);
        assert!(html.contains("cy=\"10\""), "{html}");
        assert!(!html.contains("NaN") && !html.contains("inf"), "{html}");
    }

    #[test]
    fn a_single_value_ends_its_line_at_the_right_edge() {
        let html = svg(&[5.0]);
        assert!(html.contains("cx=\"100\" cy=\"10\""), "{html}");
        assert!(
            html.ends_with("<b aria-hidden=\"true\">5</b></r-spark>"),
            "{html}"
        );
    }

    #[test]
    fn the_last_value_uses_the_short_number_format() {
        assert!(svg(&[1.0, 1.5]).ends_with("<b aria-hidden=\"true\">1.5</b></r-spark>"));
        assert!(svg(&[1.0, -0.0]).ends_with("<b aria-hidden=\"true\">0</b></r-spark>"));
        assert!(svg(&[1.0, 1234567.0]).ends_with("<b aria-hidden=\"true\">1234567</b></r-spark>"));
        assert!(svg(&[1.0, 1e-9]).ends_with("<b aria-hidden=\"true\">1.00e-9</b></r-spark>"));
    }

    #[test]
    fn bars_mark_the_last_one_and_print_the_last_value() {
        let html = spark_bars_svg(&[1.0, 3.0, 2.0], 100.0, 20.0).unwrap();
        assert_eq!(html.matches("<rect class=\"bar\"").count(), 2, "{html}");
        assert_eq!(html.matches("<rect class=\"bar hi\"").count(), 1, "{html}");
        assert!(
            html.ends_with("<b aria-hidden=\"true\">2</b></r-spark>"),
            "{html}"
        );
        assert_golden("spark_bars", &html);
    }

    #[test]
    fn bars_for_zero_and_negative_counts_have_no_height_and_no_nan() {
        let html = spark_bars_svg(&[0.0, -4.0, 0.0], 100.0, 20.0).unwrap();
        assert!(!html.contains("NaN") && !html.contains("inf"), "{html}");
        assert_eq!(html.matches("height=\"0\"").count(), 3, "{html}");
    }

    #[test]
    fn bars_drop_non_finite_values_and_reject_an_empty_series() {
        assert_eq!(spark_bars_svg(&[], 100.0, 20.0), None);
        assert_eq!(spark_bars_svg(&[f64::NAN], 100.0, 20.0), None);
        assert_eq!(
            spark_bars_svg(&[1.0, f64::NAN, 2.0], 100.0, 20.0),
            spark_bars_svg(&[1.0, 2.0], 100.0, 20.0)
        );
    }

    #[test]
    fn numbers_are_short_and_signless_at_zero() {
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(12.0), "12");
        assert_eq!(number(1.5), "1.5");
        assert_eq!(number(-2.25), "-2.25");
        assert_eq!(number(1234567.0), "1234567");
        assert_eq!(number(f64::MAX), "1.80e308");
        assert_eq!(number(1e-9), "1.00e-9");
    }
}
