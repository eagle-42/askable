//! Reads structured `tracing` logs, as journald stores them.
//!
//! A `tracing` line looks like this:
//!
//! ```text
//! 2026-09-10T14:50:14.110057Z  INFO svc: search rag=true query="two words" ms=42
//! ```
//!
//! This module knows that shape and nothing else. It has no idea what `rag` or
//! `ms` mean, and that is the point: the fields belong to whoever emitted them.

use std::collections::BTreeMap;

/// One log line, split into its fields.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Event {
    /// Clock time, taken from the timestamp `tracing` puts in front of every
    /// message. Eight characters do not justify a date library.
    pub time: String,
    /// Everything after `key=`. Ordered, so two runs print the same columns.
    pub fields: BTreeMap<String, String>,
}

impl Event {
    /// True when every `key=value` in `filters` is present and equal.
    ///
    /// An empty filter matches everything — the caller asked for no narrowing,
    /// not for nothing.
    pub fn matches(&self, filters: &[(String, String)]) -> bool {
        filters
            .iter()
            .all(|(k, v)| self.fields.get(k).map(String::as_str) == Some(v.as_str()))
    }

    /// A field as a number, or `None` when it is missing or not one.
    ///
    /// Missing is not zero: a stage that did not run and a stage that took no
    /// time are different facts, and collapsing them hides the first.
    pub fn number(&self, key: &str) -> Option<f64> {
        self.fields.get(key)?.parse().ok()
    }
}

/// Splits a `tracing` message into fields.
///
/// The format is `key=value` separated by spaces, except that a value may be
/// quoted — and it must be, whenever it contains a space. Unquoted, a
/// multi-word value runs straight into the next key and no parser can tell
/// where it ended. So: read to the closing quote when there is one, to the next
/// space otherwise.
pub fn parse(msg: &str) -> Event {
    let c: Vec<char> = msg.chars().collect();
    let mut fields = BTreeMap::new();
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
        fields.insert(key, value);
        i = j;
    }
    Event {
        time: time_of(msg),
        fields,
    }
}

/// The clock part of the leading ISO timestamp, or a visible placeholder.
///
/// The placeholder is not an empty string on purpose: a blank column reads as
/// "no time", a row of dashes reads as "time not found", and only the second is
/// true.
pub fn time_of(msg: &str) -> String {
    msg.split('T')
        .nth(1)
        .and_then(|r| r.get(..8))
        .filter(|h| h.chars().all(|c| c.is_ascii_digit() || c == ':'))
        .unwrap_or("--:--:--")
        .to_string()
}

/// The message carried by a journald JSON line, or the line itself.
///
/// Both are accepted because both get typed: `-o json` in a pipeline, `-o cat`
/// by hand. Refusing the second would only teach people to reach for `sed`.
pub fn message_of(line: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(v) => v
            .get("MESSAGE")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string(),
        Err(_) => line.to_string(),
    }
}

/// Reads a whole stream of log lines into the events that match.
pub fn events(input: &str, filters: &[(String, String)]) -> Vec<Event> {
    input
        .lines()
        .map(|l| parse(&message_of(l)))
        .filter(|e| !e.fields.is_empty() && e.matches(filters))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = r#"2026-09-10T14:50:14.110057Z  INFO svc: search rag=true query="why does my hook not fire" project=- results=3 ms_total=4214"#;

    fn filter(k: &str, v: &str) -> Vec<(String, String)> {
        vec![(k.to_string(), v.to_string())]
    }

    #[test]
    fn a_quoted_value_is_read_whole() {
        let e = parse(LINE);
        // The original defect: unquoted, parsing stopped at the first space.
        assert_eq!(e.fields["query"], "why does my hook not fire");
        assert_eq!(e.time, "14:50:14");
        assert_eq!(e.fields["results"], "3");
    }

    #[test]
    fn a_missing_field_is_missing_not_zero() {
        let e = parse(LINE);
        assert_eq!(e.number("ms_total"), Some(4214.0));
        assert_eq!(e.number("ms_rerank"), None);
        assert_eq!(e.number("query"), None);
    }

    #[test]
    fn filters_are_all_or_nothing() {
        let e = parse(LINE);
        assert!(e.matches(&filter("rag", "true")));
        assert!(!e.matches(&filter("rag", "false")));
        assert!(!e.matches(&filter("absent", "true")));
        // No filter means no narrowing, not no result.
        assert!(e.matches(&[]));
    }

    #[test]
    fn an_escaped_quote_does_not_close_the_value() {
        // This branch existed from the first day and NO test ever went
        // through it: fourteen mutations survived. A value holding an escaped
        // quote has to be read whole, and the next key has to stay readable
        // behind it.
        let l = r#"2026-01-01T09:00:00Z INFO s: q="she said \"no\" then" ms=7"#;
        let e = parse(l);
        assert_eq!(e.fields["q"], r#"she said \"no\" then"#);
        assert_eq!(e.fields["ms"], "7", "the next key has to survive");
    }

    #[test]
    fn a_quote_at_the_very_end_does_not_run_past_the_line() {
        // An opening quote never closed: read to the end, without running
        // over or looping.
        let e = parse(r#"2026-01-01T09:00:00Z INFO s: q="never closed"#);
        assert_eq!(e.fields["q"], "never closed");
        // An empty value is still a value, not an absence.
        let v = parse(r#"2026-01-01T09:00:00Z INFO s: q="" ms=1"#);
        assert_eq!(v.fields["q"], "");
        assert_eq!(v.fields["ms"], "1");
    }

    #[test]
    fn a_bare_equals_belongs_to_no_key() {
        // `=` with no word before it makes no field, and eats nothing after.
        let e = parse("2026-01-01T09:00:00Z INFO s: = a=1");
        assert_eq!(e.fields.len(), 1);
        assert_eq!(e.fields["a"], "1");
    }

    #[test]
    fn a_stream_returns_what_it_read_and_not_a_placeholder() {
        // The tests only looked at the NUMBER of events: returning a default
        // vector went unnoticed.
        let stream = format!("{LINE}\n2026-09-10T14:51:00Z INFO svc: search rag=true results=9\n");
        let v = events(&stream, &filter("rag", "true"));
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].fields["query"], "why does my hook not fire");
        assert_eq!(v[1].fields["results"], "9");
        assert_eq!(v[1].time, "14:51:00");
    }

    #[test]
    fn a_line_without_a_timestamp_says_so() {
        assert_eq!(time_of("no timestamp here"), "--:--:--");
        assert_eq!(time_of("2026-09-10Tnonsense"), "--:--:--");
    }

    #[test]
    fn journald_json_and_plain_text_both_work() {
        let json = format!(r#"{{"MESSAGE":{}}}"#, serde_json::to_string(LINE).unwrap());
        assert_eq!(message_of(&json), LINE);
        assert_eq!(message_of(LINE), LINE);
    }

    #[test]
    fn a_stream_keeps_only_what_matches() {
        let stream = format!("{LINE}\n2026-09-10T14:51:00Z INFO svc: started\n");
        assert_eq!(events(&stream, &filter("rag", "true")).len(), 1);
        // The second line has no `key=value` at all: it is not an event.
        assert_eq!(events(&stream, &[]).len(), 1);
    }
}
