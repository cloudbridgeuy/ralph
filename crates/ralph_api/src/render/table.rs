use crate::render::escape;

pub fn table(headers: &[&str], rows: &[Vec<&str>]) -> String {
    let head: String = headers
        .iter()
        .map(|text| format!("<th scope=\"col\">{}</th>", escape(text)))
        .collect();
    let body: String = rows
        .iter()
        .map(|row| {
            let padding = headers.len().saturating_sub(row.len());
            let cells: String = row
                .iter()
                .copied()
                .chain(std::iter::repeat_n("", padding))
                .map(|text| format!("<td>{}</td>", escape(text)))
                .collect();
            format!("<tr>{cells}</tr>")
        })
        .collect();
    format!("<r-table><table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table></r-table>")
}
#[cfg(test)]
mod tests {
    use super::table;
    use crate::render::golden::{assert_golden, hostile_text, long_text};

    #[test]
    fn no_columns_and_no_rows() {
        assert_golden("table_empty", &table(&[], &[]));
    }

    #[test]
    fn headers_without_rows() {
        assert_golden("table_headers_only", &table(&["name", "cost"], &[]));
    }

    #[test]
    fn one_row() {
        assert_golden(
            "table_sparse",
            &table(&["name", "cost"], &[vec!["opus", "1.20"]]),
        );
    }

    #[test]
    fn many_rows() {
        let cells: Vec<Vec<String>> = (0..12)
            .map(|i| vec![format!("d{i}"), (i * 3).to_string(), format!("{i}.50")])
            .collect();
        let rows: Vec<Vec<&str>> = cells
            .iter()
            .map(|row| row.iter().map(String::as_str).collect())
            .collect();
        assert_golden("table_dense", &table(&["day", "turns", "cost"], &rows));
    }

    #[test]
    fn short_rows_are_padded_and_long_rows_keep_every_cell() {
        assert_golden(
            "table_ragged",
            &table(&["a", "b", "c"], &[vec!["1"], vec!["1", "2", "3", "4"]]),
        );
    }

    #[test]
    fn hostile_cells_are_escaped() {
        assert_golden(
            "table_hostile",
            &table(&[hostile_text()], &[vec![hostile_text()]]),
        );
    }

    #[test]
    fn very_long_cells() {
        let long = long_text();
        assert_golden(
            "table_very_long",
            &table(&[&long, "x"], &[vec![&long, &long]]),
        );
    }
}
