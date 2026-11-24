use crate::render::escape;

const DEFAULT_SOURCE: &str = "default";

pub fn kv(pairs: &[(&str, &str)]) -> String {
    wrap(
        "two",
        &pairs
            .iter()
            .map(|(key, value)| row(key, value, ""))
            .collect::<String>(),
    )
}

pub fn kv_inline(pairs: &[(&str, &str)]) -> String {
    wrap(
        "inline",
        &pairs
            .iter()
            .map(|(key, value)| row(key, value, ""))
            .collect::<String>(),
    )
}

pub fn kv_sourced(rows: &[(&str, &str, &str)]) -> String {
    wrap(
        "",
        &rows
            .iter()
            .map(|(key, value, source)| row(key, value, &source_cell(source)))
            .collect::<String>(),
    )
}

fn wrap(variant: &str, rows: &str) -> String {
    let attribute = if variant.is_empty() {
        String::new()
    } else {
        format!(" {variant}")
    };
    format!("<r-kv{attribute}><dl>{rows}</dl></r-kv>")
}

fn row(key: &str, value: &str, source_cell: &str) -> String {
    format!(
        "<div><dt>{}</dt><dd>{}</dd>{source_cell}</div>",
        escape(key),
        escape(value)
    )
}

fn source_cell(source: &str) -> String {
    let class = if source == DEFAULT_SOURCE {
        "src"
    } else {
        "src set"
    };
    format!("<dd class=\"{class}\">{}</dd>", escape(source))
}

#[cfg(test)]
mod tests {
    use super::{kv, kv_inline, kv_sourced};
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

    #[test]
    fn two_variant_is_marked_and_has_no_source() {
        let html = kv(&[("model", "opus")]);
        assert!(html.starts_with("<r-kv two>"), "{html}");
        assert!(!html.contains("src"), "{html}");
    }

    #[test]
    fn sourced_rows() {
        assert_golden(
            "kv_sourced",
            &kv_sourced(&[
                ("model", "opus", "default"),
                ("effort", "high", "config.toml"),
            ]),
        );
    }

    #[test]
    fn sourced_empty_list() {
        assert_golden("kv_sourced_empty", &kv_sourced(&[]));
    }

    #[test]
    fn hostile_source_is_escaped() {
        assert_golden(
            "kv_sourced_hostile",
            &kv_sourced(&[(hostile_text(), hostile_text(), hostile_text())]),
        );
    }

    #[test]
    fn only_the_exact_word_default_is_not_marked_set() {
        let html = kv_sourced(&[("a", "b", "default"), ("c", "d", "defaults")]);
        assert!(html.contains("<dd class=\"src\">default</dd>"), "{html}");
        assert!(
            html.contains("<dd class=\"src set\">defaults</dd>"),
            "{html}"
        );
    }

    #[test]
    fn inline_rows() {
        assert_golden(
            "kv_inline",
            &kv_inline(&[("turns", "12"), ("cost", "$0.41")]),
        );
    }
}
