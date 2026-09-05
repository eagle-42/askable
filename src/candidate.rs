//! A candidate: one system, at one version, reachable at one URL.
//!
//! Three fields, deliberately. Not a plugin, not an adapter per engine: any
//! service that answers a GET with a ranked list of identifiers is a candidate,
//! and the day one of them needs more than a URL template is the day to write
//! the second shape — with it in front of us, not from imagination.

use serde::Deserialize;

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

impl Candidate {
    pub(crate) fn check(&self) -> Result<(), String> {
        // Without a placeholder every case would hit the same URL and the run
        // would produce 63 identical rows that still average into a rate.
        if !self.url.contains("{query}") {
            return Err(
                "the url has no {query} placeholder, so every case would ask the same thing".into(),
            );
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

    fn candidate(url: &str) -> Candidate {
        Candidate {
            url: url.into(),
            id_field: "id".into(),
            results_at: None,
        }
    }

    #[test]
    fn a_question_is_escaped_before_it_reaches_the_url() {
        let c = candidate("http://localhost:8080/search?q={query}&n={k}");
        assert_eq!(
            c.request_url("why & how", 10),
            "http://localhost:8080/search?q=why%20%26%20how&n=10"
        );
        // Accented text is bytes, not characters, once it is encoded.
        assert_eq!(percent_encode("é"), "%C3%A9");
    }

    #[test]
    fn a_url_without_a_placeholder_is_refused() {
        assert!(
            candidate("http://localhost/search?q=hello")
                .check()
                .is_err()
        );
        assert!(
            candidate("http://localhost/search?q={query}")
                .check()
                .is_ok()
        );
    }
}
