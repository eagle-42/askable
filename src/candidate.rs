//! A candidate: one system, at one version, reachable at one URL.
//!
//! Three fields, deliberately. Not a plugin, not an adapter per engine: any
//! service that answers a GET with a ranked list of identifiers is a candidate,
//! and the day one of them needs more than a URL template is the day to write
//! the second shape — with it in front of us, not from imagination.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Candidate {
    /// The whole request, with `{query}` and `{k}` left to fill in.
    ///
    /// Keeping the URL whole rather than splitting host, path and parameter
    /// names means a new service is configured, not coded.
    pub url: String,
    /// Which field of a result carries its identifier.
    #[serde(default = "id_field_default")]
    pub id_field: String,
    /// Where the array of results sits, when it is not the whole body.
    pub results_at: Option<String>,
}

fn id_field_default() -> String {
    "id".into()
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub candidate: BTreeMap<String, Candidate>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Config::from_toml(&raw).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_toml(raw: &str) -> Result<Config, String> {
        let c: Config = toml::from_str(raw).map_err(|e| format!("not a config: {e}"))?;
        for (name, cand) in &c.candidate {
            cand.check().map_err(|e| format!("candidate `{name}`: {e}"))?;
        }
        Ok(c)
    }

    /// A candidate by name, or an error that says what IS there.
    ///
    /// Listing the known names costs one line and saves the round trip where
    /// someone reopens the file to find out they wrote `search-dev`.
    pub fn get(&self, name: &str) -> Result<&Candidate, String> {
        self.candidate.get(name).ok_or_else(|| {
            let known: Vec<&str> = self.candidate.keys().map(String::as_str).collect();
            if known.is_empty() {
                "no candidate is configured".to_string()
            } else {
                format!("no candidate `{name}`; this file has: {}", known.join(", "))
            }
        })
    }
}

impl Candidate {
    fn check(&self) -> Result<(), String> {
        // Without a placeholder every case would hit the same URL and the run
        // would produce 63 identical rows that still average into a rate.
        if !self.url.contains("{query}") {
            return Err("the url has no {query} placeholder, so every case would ask the same thing".into());
        }
        if self.id_field.is_empty() {
            return Err("id_field is empty".into());
        }
        Ok(())
    }

    pub fn request_url(&self, question: &str, k: usize) -> String {
        self.url
            .replace("{query}", &percent_encode(question))
            .replace("{k}", &k.to_string())
    }
}

/// Percent-encodes everything that is not unreserved, per RFC 3986.
///
/// A corpus question is free text — spaces, apostrophes, accents — and pasting
/// it raw into a query string is how a run dies on case 4 with a 400.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = r#"
[candidate.demo]
url = "http://localhost:8080/search?q={query}&n={k}"

[candidate.other]
url = "http://localhost:9090/find?text={query}"
id_field = "uuid"
results_at = "results"
"#;

    #[test]
    fn a_candidate_needs_only_a_url() {
        let c = Config::from_toml(RAW).unwrap();
        let demo = c.get("demo").unwrap();
        assert_eq!(demo.id_field, "id", "the common case should need no line");
        assert_eq!(demo.results_at, None);
        assert_eq!(c.get("other").unwrap().id_field, "uuid");
    }

    #[test]
    fn an_unknown_name_says_what_is_there() {
        let c = Config::from_toml(RAW).unwrap();
        let e = c.get("nope").unwrap_err();
        assert!(e.contains("demo") && e.contains("other"), "got: {e}");
    }

    #[test]
    fn a_url_without_a_placeholder_is_refused_at_load() {
        let flat = r#"[candidate.x]
url = "http://localhost/search?q=hello""#;
        let e = Config::from_toml(flat).unwrap_err();
        assert!(e.contains("{query}"), "got: {e}");
    }

    #[test]
    fn a_question_is_escaped_before_it_reaches_the_url() {
        let c = Config::from_toml(RAW).unwrap();
        let url = c.get("demo").unwrap().request_url("why & how", 10);
        assert_eq!(url, "http://localhost:8080/search?q=why%20%26%20how&n=10");
        // Accented text is bytes, not characters, once it is encoded.
        assert_eq!(percent_encode("é"), "%C3%A9");
    }
}
