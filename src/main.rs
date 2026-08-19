//! askable — reads the service's search traces.
//!
//! The service emits one `tracing` line per search. It lands in journald, where
//! nobody reads it. askable reads it back.
//!
//! ```
//! journalctl --user -u some.service -o json | askable tail --last 50
//! ```

use std::collections::HashMap;
use std::io::{self, Read};

/// One search, as the service traced it.
#[derive(Debug, Default, PartialEq)]
struct Trace {
    time: String,
    query: String,
    project: String,
    results: u64,
    top1: String,
    rerank: Option<f64>,
    ms_rerank: u64,
    ms_total: u64,
    empty: bool,
}

/// Splits a tracing message into fields.
///
/// The format is `key=value` separated by spaces, except that `query` is
/// quoted — it contains spaces, and unquoted there is no way to tell where it
/// ends. So: read to the closing quote when there is one, to the next space
/// otherwise.
fn fields(msg: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let c: Vec<char> = msg.chars().collect();
    let mut i = 0usize;
    while i < c.len() {
        if c[i] != '=' {
            i += 1;
            continue;
        }
        // Walk back to the start of the key: word characters glued to the `=`.
        let mut k = i;
        while k > 0 && (c[k - 1].is_alphanumeric() || c[k - 1] == '_') {
            k -= 1;
        }
        if k == i {
            i += 1;
            continue;
        }
        let key: String = c[k..i].iter().collect();
        let mut j = i + 1;
        let value: String = if j < c.len() && c[j] == '"' {
            j += 1;
            let start = j;
            while j < c.len() && c[j] != '"' {
                // An escaped quote does not close the value.
                if c[j] == '\\' && j + 1 < c.len() {
                    j += 1;
                }
                j += 1;
            }
            let v: String = c[start..j].iter().collect();
            j += 1;
            v
        } else {
            let start = j;
            while j < c.len() && !c[j].is_whitespace() {
                j += 1;
            }
            c[start..j].iter().collect()
        };
        out.insert(key, value);
        i = j;
    }
    out
}

/// The clock time, taken from the timestamp tracing already puts in front of
/// every message. Eight characters do not justify a date library.
fn time_of(msg: &str) -> String {
    msg.split('T')
        .nth(1)
        .and_then(|r| r.get(..8))
        .unwrap_or("--:--:--")
        .to_string()
}

fn trace_from(msg: &str) -> Option<Trace> {
    let f = fields(msg);
    // `rag=true` is the only marker; the rest of the journal is not ours.
    if f.get("rag").map(String::as_str) != Some("true") {
        return None;
    }
    let n = |k: &str| f.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    Some(Trace {
        time: time_of(msg),
        query: f.get("requete").cloned().unwrap_or_default(),
        project: f.get("projet").cloned().unwrap_or_else(|| "-".into()),
        results: n("resultats"),
        top1: f.get("top1").cloned().unwrap_or_default(),
        // Absent is not zero: with no reranking there is no judgement, and a
        // 0.0 would pass for one that never happened.
        rerank: f.get("score_rerank").and_then(|v| v.parse::<f64>().ok()),
        ms_rerank: n("ms_reclassement"),
        ms_total: n("ms_total"),
        empty: f.get("vide").map(String::as_str) == Some("true"),
    })
}

/// The message of a journald line, or the line itself when it is not JSON —
/// `journalctl -o cat` is what you type by hand, so accept it too.
fn message(line: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(v) => v
            .get("MESSAGE")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string(),
        Err(_) => line.to_string(),
    }
}

fn share(part: u64, whole: u64) -> String {
    if whole == 0 {
        "-".into()
    } else {
        format!("{}%", part * 100 / whole)
    }
}

fn clip(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        s.to_string()
    } else {
        format!("{}…", c[..n.saturating_sub(1)].iter().collect::<String>())
    }
}

fn print_table(traces: &[Trace]) {
    println!(
        "{:<9} {:<10} {:>8} {:>8} {:>8}  {}",
        "time", "project", "rerank", "total", "rerank%", "query"
    );
    println!("{}", "-".repeat(96));
    for t in traces {
        let r = match t.rerank {
            Some(v) => format!("{v:>8.2}"),
            None => format!("{:>8}", "-"),
        };
        let mark = if t.empty { " (no result)" } else { "" };
        println!(
            "{:<9} {:<10} {} {:>7}ms {:>8}  {}{}",
            t.time,
            clip(&t.project, 10),
            r,
            t.ms_total,
            share(t.ms_rerank, t.ms_total),
            clip(&t.query, 44),
            mark
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Twenty lines of argument parsing beat one more dependency for as long as
    // the surface fits in two flags.
    if args.first().map(String::as_str) != Some("tail") {
        eprintln!("usage: journalctl --user -u some.service -o json | askable tail [--last N]");
        std::process::exit(2);
    }
    let last = args
        .iter()
        .position(|a| a == "--last")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(20);

    let mut raw = String::new();
    if io::stdin().read_to_string(&mut raw).is_err() {
        eprintln!("askable: cannot read stdin");
        std::process::exit(1);
    }
    let all: Vec<Trace> = raw
        .lines()
        .filter_map(|l| trace_from(&message(l)))
        .collect();

    let start = all.len().saturating_sub(last);
    print_table(&all[start..]);
    // "No search" and "nothing read" must not look alike: an empty pipe and a
    // journal without traces would otherwise print the same silent table.
    if all.is_empty() {
        eprintln!("askable: no `rag=true` line in the input");
    } else {
        eprintln!(
            "askable: {} search(es) read, {} shown",
            all.len(),
            all.len() - start
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real line, copied from a live journal.
    const LINE: &str = r#"2026-09-10T14:50:14.110057Z  INFO svc: recherche rag=true requete="pourquoi mon hook ne se declenche pas" projet=- resultats=3 top1=2eebc3e2-b77c-4180-8b90-fe72a338fb92 score=0.014 score_cosine=0.49 score_lexical=0.0 score_rerank=-0.028 vide=false ms_embed=36 ms_vectoriel=11 ms_lexical=0 ms_reclassement=4167 ms_total=4214"#;

    #[test]
    fn a_query_with_spaces_is_read_whole() {
        let t = trace_from(LINE).expect("expected a rag=true line");
        // The original defect: unquoted, parsing stopped at the first space.
        assert_eq!(t.query, "pourquoi mon hook ne se declenche pas");
        assert_eq!(t.project, "-");
        assert_eq!(t.results, 3);
        assert_eq!(t.ms_total, 4214);
        assert_eq!(t.ms_rerank, 4167);
        assert!(!t.empty);
    }

    #[test]
    fn a_missing_score_stays_missing() {
        let without = LINE.replace(" score_rerank=-0.028", "");
        assert_eq!(trace_from(&without).unwrap().rerank, None);
        assert_eq!(trace_from(LINE).unwrap().rerank, Some(-0.028));
    }

    #[test]
    fn a_line_without_the_marker_is_ignored() {
        assert!(trace_from("2026-09-10T14:50:14Z INFO svc: index created").is_none());
    }

    #[test]
    fn the_share_never_divides_by_zero() {
        assert_eq!(share(4167, 4214), "98%");
        assert_eq!(share(0, 0), "-");
    }
}
