use crate::render::escape;

fn all_digits(text: &str) -> bool {
    text.chars().all(|c| c.is_ascii_digit())
}

fn grouped_integer(text: &str) -> bool {
    let mut groups = text.split(',');
    let first = groups.next().unwrap_or("");
    let rest: Vec<&str> = groups.collect();
    if rest.is_empty() {
        return all_digits(first);
    }
    (1..=3).contains(&first.len())
        && all_digits(first)
        && rest
            .iter()
            .all(|group| group.len() == 3 && all_digits(group))
}

fn is_number(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let body = body.strip_prefix('$').unwrap_or(body);
    let body = body.strip_suffix('%').unwrap_or(body);
    let (whole, fraction) = match body.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (body, None),
    };
    let fraction_ok = fraction.is_none_or(|digits| !digits.is_empty() && all_digits(digits));
    let whole_ok =
        whole.is_empty() && fraction.is_some() || !whole.is_empty() && grouped_integer(whole);
    fraction_ok && whole_ok
}

fn numeric_columns(headers: &[&str], rows: &[Vec<&str>]) -> Vec<bool> {
    let width = rows.iter().map(Vec::len).fold(headers.len(), usize::max);
    (0..width)
        .map(|column| {
            let mut cells = rows
                .iter()
                .filter_map(|row| row.get(column))
                .filter(|cell| !cell.is_empty())
                .peekable();
            cells.peek().is_some() && cells.all(|cell| is_number(cell))
        })
        .collect()
}

fn cell(tag: &str, text: &str, numeric: bool) -> String {
    let class = if numeric { " class=\"num\"" } else { "" };
    let scope = if tag == "th" { " scope=\"col\"" } else { "" };
    format!("<{tag}{scope}{class}>{}</{tag}>", escape(text))
}

fn shell(headers: &[&str], numeric: &[bool], body: &str) -> String {
    let head: String = headers
        .iter()
        .enumerate()
        .map(|(column, text)| cell("th", text, numeric.get(column).copied().unwrap_or(false)))
        .collect();
    format!("<r-table><table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table></r-table>")
}

#[derive(Clone, Copy)]
pub enum RowMark {
    Selected,
    Open,
}

pub fn table(headers: &[&str], rows: &[Vec<&str>]) -> String {
    table_marked(headers, rows, &[])
}

pub fn table_marked(headers: &[&str], rows: &[Vec<&str>], marks: &[(usize, RowMark)]) -> String {
    let numeric = numeric_columns(headers, rows);
    let body: String = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let padding = headers.len().saturating_sub(row.len());
            let cells: String = row
                .iter()
                .copied()
                .chain(std::iter::repeat_n("", padding))
                .enumerate()
                .map(|(column, text)| cell("td", text, numeric[column]))
                .collect();
            let mark = match marks.iter().find(|(at, _)| *at == index) {
                Some((_, RowMark::Selected)) => " aria-selected=\"true\"",
                Some((_, RowMark::Open)) => " class=\"open\"",
                None => "",
            };
            format!("<tr{mark}>{cells}</tr>")
        })
        .collect();
    shell(headers, &numeric, &body)
}

pub fn table_state(headers: &[&str], inner_html: &str) -> String {
    let columns = headers.len().max(1);
    let row = format!(
        "<tr class=\"state\"><td class=\"state\" colspan=\"{columns}\">{inner_html}</td></tr>"
    );
    shell(headers, &[], &row)
}

#[cfg(test)]
mod tests {
    use super::{is_number, numeric_columns, table, table_marked, table_state, RowMark};
    use crate::render::golden::{assert_golden, hostile_text, long_text};
    use crate::render::{empty, error, spinner};

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

    #[test]
    fn numbers_allow_a_sign_a_currency_mark_separators_and_a_percent() {
        for text in ["0", "12", "-3", "+3", "1.20", "$1,200.50", "45%", "1,000"] {
            assert!(is_number(text), "{text}");
        }
    }

    #[test]
    fn comma_groups_have_three_digits_and_a_dot_needs_digits_after_it() {
        for text in ["1,234", "12,345.6", "1,234,567", "$1,000.50", ".5", "0.50"] {
            assert!(is_number(text), "{text}");
        }
        for text in [
            "1,2,3", ",,1", ",1", "1,", "1.", "1,23", "1,2345", "1,234,56", "1,234.", "1.2.3",
            "1,.5",
        ] {
            assert!(!is_number(text), "{text:?}");
        }
    }

    #[test]
    fn words_and_near_numbers_are_not_numbers() {
        for text in [
            "", " ", "d1", "1e5", "NaN", "inf", "1.2.3x", "-", "$", "12 ms", ".",
        ] {
            assert!(!is_number(text), "{text:?}");
        }
    }

    #[test]
    fn a_column_is_numeric_when_every_non_empty_cell_is_a_number() {
        let rows = vec![vec!["a", "1", ""], vec!["b", "2.5", "3"]];
        assert_eq!(
            numeric_columns(&["k", "n", "m"], &rows),
            vec![false, true, true]
        );
    }

    #[test]
    fn a_stray_number_in_a_text_column_keeps_the_column_left() {
        let rows = vec![vec!["alpha"], vec!["42"], vec!["beta"]];
        assert_eq!(numeric_columns(&["name"], &rows), vec![false]);
    }

    #[test]
    fn an_all_empty_column_is_not_numeric() {
        let rows = vec![vec![""], vec![""]];
        assert_eq!(numeric_columns(&["x"], &rows), vec![false]);
        assert_eq!(numeric_columns(&["x"], &[]), vec![false]);
    }

    #[test]
    fn cells_past_the_header_count_get_their_own_column() {
        let rows = vec![vec!["1", "2", "3"]];
        assert_eq!(numeric_columns(&["a"], &rows), vec![true, true, true]);
    }

    #[test]
    fn numeric_columns_align_right_in_head_and_body() {
        assert_golden(
            "table_numeric",
            &table(
                &["name", "cost"],
                &[vec!["opus", "1.20"], vec!["7", "$3"], vec!["sonnet", ""]],
            ),
        );
    }

    #[test]
    fn an_empty_state_row_spans_every_column() {
        assert_golden(
            "table_state_empty",
            &table_state(&["day", "turns", "cost"], &empty("No rows yet")),
        );
    }

    #[test]
    fn a_loading_state_row_holds_the_spinner() {
        assert_golden(
            "table_state_loading",
            &table_state(&["day", "turns"], &spinner("Loading rows")),
        );
    }

    #[test]
    fn an_error_state_row_holds_the_error() {
        assert_golden(
            "table_state_error",
            &table_state(&["day"], &error("could not load rows")),
        );
    }

    #[test]
    fn a_state_table_with_no_headers_still_spans_one_column() {
        let html = table_state(&[], "x");
        assert!(html.contains("colspan=\"1\""), "{html}");
        assert!(html.contains("<tbody><tr class=\"state\">"), "{html}");
    }

    #[test]
    fn a_marked_row_is_selected_or_open_and_the_others_are_plain() {
        assert_golden(
            "table_marked",
            &table_marked(
                &["name"],
                &[vec!["a"], vec!["b"], vec!["c"]],
                &[(1, RowMark::Selected), (2, RowMark::Open)],
            ),
        );
    }

    #[test]
    fn a_mark_past_the_last_row_changes_nothing() {
        let rows = [vec!["a"]];
        assert_eq!(
            table_marked(&["n"], &rows, &[(5, RowMark::Open)]),
            table(&["n"], &rows)
        );
    }
}
