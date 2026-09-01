//! The configuration file: the candidates, and how strict the judge is.
//!
//! It holds a URL, so it is the one file that belongs to your infrastructure
//! rather than to this repository. `askable.toml` is git-ignored for that
//! reason, and `askable.example.toml` is what ships.

use crate::candidate::Candidate;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Floor {
    pub cutoff: usize,
    pub recall: f64,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Judge {
    /// Ranks at which a case counts as found. `1` is "did the right answer come
    /// first", `5` is "was it anywhere near the top".
    #[serde(default = "cutoffs_default")]
    pub cutoffs: Vec<usize>,
    /// Significance level of the exact McNemar test.
    #[serde(default = "alpha_default")]
    pub alpha: f64,
    /// Absolute recalls that must hold whatever the paired test says.
    ///
    /// The weak form, kept on purpose as a safety net: it cannot see a small
    /// regression, but it catches a collapse that the paired test would call
    /// "significant" long after anyone cared.
    #[serde(default)]
    pub floor: Vec<Floor>,
}

fn cutoffs_default() -> Vec<usize> {
    vec![1, 5]
}

fn alpha_default() -> f64 {
    0.05
}

impl Default for Judge {
    fn default() -> Self {
        Judge {
            cutoffs: cutoffs_default(),
            alpha: alpha_default(),
            floor: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub candidate: BTreeMap<String, Candidate>,
    /// Absent means the defaults: cutoffs 1 and 5, alpha 0.05, no floor.
    #[serde(default)]
    pub judge: Judge,
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
            cand.check()
                .map_err(|e| format!("candidate `{name}`: {e}"))?;
        }
        if c.judge.cutoffs.is_empty() {
            return Err("judge.cutoffs is empty, so there is nothing to judge".into());
        }
        if !(0.0..1.0).contains(&c.judge.alpha) {
            return Err(format!(
                "judge.alpha is {}, which is not a significance level",
                c.judge.alpha
            ));
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
        let flat = "[candidate.x]\nurl = \"http://localhost/search?q=hello\"";
        let e = Config::from_toml(flat).unwrap_err();
        assert!(e.contains("{query}"), "got: {e}");
    }

    #[test]
    fn the_judge_has_working_defaults_and_refuses_nonsense() {
        let c = Config::from_toml(RAW).unwrap();
        assert_eq!(c.judge.cutoffs, vec![1, 5]);
        assert_eq!(c.judge.alpha, 0.05);
        assert!(c.judge.floor.is_empty());

        let strict = Config::from_toml(
            "[judge]\ncutoffs = [1]\nalpha = 0.01\n\n[[judge.floor]]\ncutoff = 1\nrecall = 0.6",
        )
        .unwrap();
        assert_eq!(strict.judge.cutoffs, vec![1]);
        assert_eq!(strict.judge.floor[0].recall, 0.6);

        assert!(Config::from_toml("[judge]\ncutoffs = []").is_err());
        assert!(Config::from_toml("[judge]\nalpha = 1.5").is_err());
    }
}
