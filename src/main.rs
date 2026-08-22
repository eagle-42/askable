//! askable — prints structured `tracing` logs as a table.
//!
//! ```text
//! journalctl -u some.service -o json | askable tail --match rag=true \
//!     --show query,ms_total --last 20
//! ```
//!
//! This file holds the command line and the rendering. Everything about the log
//! format lives in the library, where it can be tested without a terminal.

use askable::{Event, events};
use std::io::{self, Read};

const DEFAULT_LAST: usize = 20;
/// Wide enough to read a sentence, narrow enough that a column of them still
/// lines up on an 80-column terminal.
const MAX_CELL: usize = 44;

struct Options {
    filters: Vec<(String, String)>,
    columns: Vec<String>,
    last: usize,
}

/// Twenty lines of argument parsing beat one more dependency for as long as the
/// surface fits in three flags.
fn parse_args(args: &[String]) -> Result<Options, String> {
    if args.first().map(String::as_str) != Some("tail") {
        return Err("usage: askable tail [--match k=v]... [--show a,b,c] [--last N]".into());
    }
    let mut o = Options {
        filters: Vec::new(),
        columns: Vec::new(),
        last: DEFAULT_LAST,
    };
    let mut i = 1;
    while i < args.len() {
        let next = || {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} expects a value", args[i]))
        };
        match args[i].as_str() {
            "--match" => {
                let kv = next()?;
                let (k, v) = kv
                    .split_once('=')
                    .ok_or_else(|| format!("--match wants key=value, got `{kv}`"))?;
                o.filters.push((k.to_string(), v.to_string()));
                i += 2;
            }
            "--show" => {
                o.columns = next()?.split(',').map(|s| s.trim().to_string()).collect();
                i += 2;
            }
            "--last" => {
                o.last = next()?
                    .parse()
                    .map_err(|_| "--last wants a number".to_string())?;
                i += 2;
            }
            other => return Err(format!("unknown flag `{other}`")),
        }
    }
    Ok(o)
}

/// The columns to print: the ones asked for, or every field that shows up.
///
/// Falling back to all fields matters the first time someone runs this against
/// an unfamiliar service — you cannot name a column you have never seen.
fn columns(opts: &Options, found: &[Event]) -> Vec<String> {
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

fn clip(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        return s.to_string();
    }
    format!("{}…", c[..n.saturating_sub(1)].iter().collect::<String>())
}

fn render(found: &[Event], cols: &[String]) {
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
    println!("{:<9} {}", "time", head.join("  "));
    println!("{}", "-".repeat(9 + head.join("  ").chars().count() + 1));
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
        println!("{:<9} {}", e.time, row.join("  "));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("askable: {e}");
            std::process::exit(2);
        }
    };

    let mut raw = String::new();
    if io::stdin().read_to_string(&mut raw).is_err() {
        eprintln!("askable: cannot read stdin");
        std::process::exit(1);
    }

    let found = events(&raw, &opts.filters);
    let start = found.len().saturating_sub(opts.last);
    let shown = &found[start..];
    let cols = columns(&opts, shown);

    if shown.is_empty() {
        // "Nothing matched" and "nothing was piped in" are different problems
        // and would otherwise print the same silent table.
        eprintln!(
            "askable: no event matched{}",
            if raw.trim().is_empty() {
                " (the input was empty)"
            } else {
                ""
            }
        );
        return;
    }
    render(shown, &cols);
    eprintln!("askable: {} matched, {} shown", found.len(), shown.len());
}
