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

fn polyline(values: &[f64], width: f64, height: f64) -> String {
    let Some((min, max)) = bounds(values) else {
        return String::new();
    };
    let points: Vec<(f64, f64)> = match values {
        [only] => vec![
            (0.0, height - normalise(*only, min, max) * height),
            (width, height - normalise(*only, min, max) * height),
        ],
        _ => {
            let steps = (values.len() - 1) as f64;
            values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    (
                        index as f64 * width / steps,
                        height - normalise(*value, min, max) * height,
                    )
                })
                .collect()
        }
    };
    points
        .iter()
        .map(|(x, y)| format!("{x:.2},{y:.2}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn spark_points(values: &[f64], width: f64, height: f64) -> String {
    polyline(&finite(values), dimension(width), dimension(height))
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

pub fn spark_svg(values: &[f64], width: f64, height: f64) -> Option<String> {
    let values = finite(values);
    let (min, max) = bounds(&values)?;
    let last = values.last().copied()?;
    let (width, height) = (dimension(width), dimension(height));
    Some(format!(
        "<r-spark><svg class=\"spark\" role=\"img\" aria-label=\"min {}, max {}, last {}\" viewBox=\"0 0 {} {}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\"><polyline class=\"line\" stroke=\"currentColor\" points=\"{}\"/></svg></r-spark>",
        number(min),
        number(max),
        number(last),
        number(width),
        number(height),
        number(width),
        number(height),
        polyline(&values, width, height)
    ))
}

#[cfg(test)]
mod tests {
    use super::{number, spark_points, spark_svg};
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
