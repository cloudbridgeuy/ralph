use crate::render::escape;

pub fn kv(pairs: &[(&str, &str)]) -> String {
    let rows: String = pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "<div><dt>{}</dt><dd>{}</dd></div>",
                escape(key),
                escape(value)
            )
        })
        .collect();
    format!("<r-kv><dl>{rows}</dl></r-kv>")
}

#[cfg(test)]
mod tests {
    use super::kv;
    use crate::render::golden::{assert_golden, hostile_text, long_text};

    #[test]
    fn empty_list() {
        assert_golden("kv_empty", &kv(&[]));
    }

    #[test]
    fn sparse_list() {
        assert_golden("kv_sparse", &kv(&[("model", "opus"), ("turns", "12")]));
    }

    #[test]
    fn hostile_pair_is_escaped() {
        assert_golden("kv_hostile", &kv(&[(hostile_text(), hostile_text())]));
    }

    #[test]
    fn very_long_pair() {
        let long = long_text();
        assert_golden("kv_very_long", &kv(&[(&long, &long)]));
    }
}
