use std::io::IsTerminal;

pub const GREEN: &str = "32";
pub const YELLOW: &str = "33";
pub const BOLD: &str = "1";
const DIM: &str = "2";

/// Colors only on a terminal, and not when NO_COLOR is set.
pub fn paint(on: bool, code: &str, text: &str) -> String {
    if on && std::env::var_os("NO_COLOR").is_none() { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
}

/// A message to the user on stderr; continuation lines are indented under the first.
pub fn say(msg: &str) {
    let prefix = paint(std::io::stderr().is_terminal(), DIM, "wip:");
    eprintln!("{prefix} {}", msg.replace('\n', "\n     "));
}

/// Rows as aligned columns, each as wide as its longest cell.
pub fn table(rows: &[Vec<String>]) -> Vec<String> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let width = |c: usize| rows.iter().filter_map(|r| r.get(c)).map(|s| s.chars().count()).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns).map(width).collect();
    let line = |row: &Vec<String>| {
        let cells: Vec<String> = row.iter().zip(&widths).map(|(cell, w)| format!("{cell:<w$}")).collect();
        cells.join("  ").trim_end().to_string()
    };
    rows.iter().map(line).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_columns_fit_their_longest_cell() {
        let rows = vec![
            vec!["HOST".to_string(), "BRANCH".to_string(), "SAVED".to_string()],
            vec!["a-long-hostname".to_string(), "main".to_string(), "now".to_string()],
            vec!["b".to_string(), "feature/with-a-long-name".to_string(), "2 hours ago".to_string()],
        ];
        assert_eq!(
            table(&rows),
            [
                "HOST             BRANCH                    SAVED",
                "a-long-hostname  main                      now",
                "b                feature/with-a-long-name  2 hours ago",
            ]
        );
    }

    #[test]
    fn no_color_codes_when_not_on_a_terminal() {
        assert_eq!(paint(false, GREEN, "up to date"), "up to date");
    }
}
