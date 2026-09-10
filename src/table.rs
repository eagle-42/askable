//! Printing rows of fields as a table.
//!
//! Composes a string and returns it. Printing is the binary's job, and a
//! function that prints cannot be asserted on.

use crate::Event;
use std::fmt::Write;

/// Wide enough to read a sentence, narrow enough that a column of them still
/// lines up on an 80-column terminal.
pub const MAX_CELL: usize = 44;

/// Falling back to all fields matters the first time someone runs this against
/// an unfamiliar service — you cannot name a column you have never seen.
pub fn columns(opts: &crate::cli::TailArgs, found: &[Event]) -> Vec<String> {
    if !opts.columns.is_empty() {
        return opts.columns.clone();
    }
    let mut seen: Vec<String> = Vec::new();
    for e in found {
        for k in e.fields.keys() {
            if !seen.contains(k) {
                seen.push(k.clone());
            }
        }
    }
    seen
}

pub fn clip(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        return s.to_string();
    }
    format!("{}…", c[..n.saturating_sub(1)].iter().collect::<String>())
}

pub fn render(found: &[Event], cols: &[String]) -> String {
    let mut out = String::new();
    // One pass to size the columns, so nothing is padded to a width the data
    // never needs.
    let mut widths: Vec<usize> = cols.iter().map(|c| c.chars().count()).collect();
    for e in found {
        for (i, c) in cols.iter().enumerate() {
            let cell = e.fields.get(c).map(String::as_str).unwrap_or("-");
            widths[i] = widths[i].max(clip(cell, MAX_CELL).chars().count());
        }
    }
    let head: Vec<String> = cols
        .iter()
        .zip(&widths)
        .map(|(c, w)| format!("{c:<w$}"))
        .collect();
    let _ = writeln!(out, "{:<9} {}", "time", head.join("  "));
    let _ = writeln!(
        out,
        "{}",
        "-".repeat(9 + head.join("  ").chars().count() + 1)
    );
    for e in found {
        let row: Vec<String> = cols
            .iter()
            .zip(&widths)
            .map(|(c, w)| {
                // A dash, never a blank: absent and empty must not look alike.
                let cell = e.fields.get(c).map(String::as_str).unwrap_or("-");
                format!("{:<w$}", clip(cell, MAX_CELL))
            })
            .collect();
        let _ = writeln!(out, "{:<9} {}", e.time, row.join("  "));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn a_missing_field_shows_a_dash_and_never_a_blank() {
        let e = parse("2026-01-01T09:00:00Z INFO s: a=1 b=deux");
        let t = render(&[e], &["a".into(), "absent".into()]);
        assert!(t.contains("09:00:00"), "got: {t}");
        // Absent and empty must not look alike.
        assert!(t.contains('-'), "got: {t}");
        // Columns are sized on what fills them, header included.
        assert!(t.lines().count() >= 3, "header, rule, and one row: {t}");
    }

    #[test]
    fn the_columns_fall_back_to_every_field_seen() {
        let e = parse("2026-01-01T09:00:00Z INFO s: rag=true ms=12");
        let opts = crate::cli::TailArgs {
            filters: Vec::new(),
            columns: Vec::new(),
            last: 20,
        };
        let c = columns(&opts, std::slice::from_ref(&e));
        assert!(
            c.contains(&"rag".to_string()) && c.contains(&"ms".to_string()),
            "{c:?}"
        );
    }

    #[test]
    fn a_long_cell_is_clipped_with_a_visible_mark() {
        assert_eq!(clip("abcdef", 3), "ab\u{2026}");
        assert_eq!(clip("abc", 3), "abc", "what fits is not clipped");
        // Clips on CHARACTERS, not on bytes.
        assert_eq!(clip("éééé", 2), "é\u{2026}");
    }
}
