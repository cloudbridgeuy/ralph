use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Fixture {
    pub titles: Vec<String>,
    pub very_long_title: String,
    pub counts: Vec<(String, u64)>,
    pub durations: Vec<(String, u64)>,
    pub costs: Vec<(String, u64, u64)>,
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, String> {
    value
        .get(name)
        .ok_or_else(|| format!("the fixture has no {name}"))
}

fn number_map(value: &Value, name: &str) -> Result<Vec<(String, u64)>, String> {
    let map = field(value, name)?
        .as_object()
        .ok_or_else(|| format!("{name} is not an object"))?;
    let mut pairs: Vec<(String, u64)> = map
        .iter()
        .filter_map(|(key, number)| number.as_u64().map(|n| (key.clone(), n)))
        .collect();
    pairs.sort();
    Ok(pairs)
}

fn cost_row(row: &Value) -> Option<(String, u64, u64)> {
    Some((
        row.get("day")?.as_str()?.to_string(),
        row.get("turns")?.as_u64()?,
        row.get("cost")?.as_u64()?,
    ))
}

pub(super) fn parse(text: &str) -> Result<Fixture, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if field(&value, "schema")?.as_u64() != Some(1) {
        return Err("the fixture schema is not 1".into());
    }
    let titles = field(&value, "titles")?
        .as_array()
        .ok_or("titles is not a list")?
        .iter()
        .filter_map(|title| title.as_str().map(str::to_string))
        .collect();
    let very_long_title = field(&value, "very_long_title_derived")?
        .as_str()
        .ok_or("very_long_title_derived is not text")?
        .to_string();
    let costs = field(&value, "cost_series_usd")?
        .as_array()
        .ok_or("cost_series_usd is not a list")?
        .iter()
        .filter_map(cost_row)
        .collect();
    Ok(Fixture {
        titles,
        very_long_title,
        counts: number_map(&value, "counts")?,
        durations: number_map(&value, "durations_ms")?,
        costs,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse, Fixture};

    const COMMITTED: &str = include_str!("../../fixtures/catalogue.json");

    #[test]
    fn the_committed_fixture_parses() {
        let fixture = parse(COMMITTED).unwrap();
        assert!(!fixture.titles.is_empty());
        assert_eq!(fixture.costs.len(), 16);
        assert!(fixture.very_long_title.chars().count() >= 300);
        assert!(fixture.counts.iter().any(|(name, _)| name == "turns"));
    }

    #[test]
    fn the_generator_sample_parses() {
        let fixture = parse(include_str!("../../fixtures/generator_sample.json")).unwrap();
        assert_eq!(fixture.titles.len(), 2);
        assert!(fixture.very_long_title.chars().count() >= 300);
        assert_eq!(fixture.costs.len(), 2);
        assert!(fixture
            .counts
            .iter()
            .any(|(name, n)| name == "turns" && *n == 3));
    }

    #[test]
    fn a_small_fixture_keeps_its_order() {
        let text = r#"{"schema":1,"titles":["a","b"],"very_long_title_derived":"b b","counts":{"turns":2,"naps":1},"durations_ms":{"turn_max":9},"cost_series_usd":[{"day":"2026-01-01","turns":1,"cost":5}]}"#;
        let fixture = parse(text).unwrap();
        assert_eq!(
            fixture,
            Fixture {
                titles: vec!["a".into(), "b".into()],
                very_long_title: "b b".into(),
                counts: vec![("naps".into(), 1), ("turns".into(), 2)],
                durations: vec![("turn_max".into(), 9)],
                costs: vec![("2026-01-01".into(), 1, 5)],
            }
        );
    }

    #[test]
    fn a_missing_or_unknown_schema_is_an_error() {
        let body = r#""titles":[],"very_long_title_derived":"v","counts":{},"durations_ms":{},"cost_series_usd":[]"#;
        assert!(parse(&format!("{{{body}}}"))
            .unwrap_err()
            .contains("schema"));
        let error = parse(&format!("{{\"schema\":2,{body}}}")).unwrap_err();
        assert!(error.contains("schema"), "{error}");
        assert!(parse(&format!("{{\"schema\":1,{body}}}")).is_ok());
    }

    #[test]
    fn bad_json_is_an_error() {
        assert!(parse("not json").is_err());
    }

    #[test]
    fn a_missing_field_is_an_error_that_names_it() {
        let error = parse(r#"{"schema":1,"titles":[]}"#).unwrap_err();
        assert!(error.contains("very_long_title_derived"), "{error}");
    }
}
