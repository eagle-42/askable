//! The frozen corpus: the questions, and which answers count as right.
//!
//! It is frozen between two refreshes, not forever. Refreshing it invalidates
//! every earlier record by construction — a recall measured over 63 cases is
//! not comparable to one measured over 71 — which is why [`Corpus::fingerprint`]
//! travels with every record.

use serde::Deserialize;
use std::path::Path;

/// Shortest identifier prefix a corpus may use as an expected answer.
///
/// Four hexadecimal characters are one chance in 65 536 of matching something
/// else; anything shorter turns a coincidence into a passing case.
const MIN_GOLD: usize = 4;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Case {
    pub question: String,
    /// Every answer that counts as right.
    ///
    /// A question with one obvious answer and three acceptable ones is the
    /// normal case; scoring it out of a single id would call three correct
    /// results a failure.
    pub gold: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Corpus {
    pub name: String,
    pub cases: Vec<Case>,
}

impl Corpus {
    pub fn load(path: &Path) -> Result<Corpus, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Corpus::from_json(&raw).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_json(raw: &str) -> Result<Corpus, String> {
        let c: Corpus = serde_json::from_str(raw).map_err(|e| format!("not a corpus: {e}"))?;
        // An empty corpus would replay in no time and produce a record whose
        // every rate is a division by zero dressed up as a measurement.
        if c.cases.is_empty() {
            return Err("no case in this corpus".into());
        }
        if let Some(bad) = c.cases.iter().find(|k| k.gold.is_empty()) {
            return Err(format!("case {:?} lists no expected answer", bad.question));
        }
        // A gold matches by prefix, so a very short one matches identifiers it
        // was never meant to. That does not fail: it scores a wrong answer as
        // right and quietly inflates the headline number.
        if let Some((case, g)) = c
            .cases
            .iter()
            .find_map(|k| k.gold.iter().find(|g| g.len() < MIN_GOLD).map(|g| (k, g)))
        {
            return Err(format!(
                "case {:?}: the expected answer {g:?} is shorter than {MIN_GOLD} characters, \
and would match identifiers it does not mean",
                case.question
            ));
        }
        Ok(c)
    }

    /// A fingerprint of what the corpus ASKS, not of how the file is laid out.
    ///
    /// Reformatting the JSON must not change it, or every pretty-print would
    /// read as a refreshed corpus. Adding, removing or editing a case must.
    ///
    /// ponytail: FNV-1a, not a cryptographic hash. It answers "did the corpus
    /// change", not "is this the authentic corpus". Reach for sha2 the day the
    /// second question is the one being asked.
    pub fn fingerprint(&self) -> String {
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |bytes: &[u8]| {
            for b in bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(PRIME);
            }
            // A separator, so ["ab","c"] and ["a","bc"] do not collide.
            h ^= 0xff;
            h = h.wrapping_mul(PRIME);
        };
        for case in &self.cases {
            eat(case.question.as_bytes());
            for g in &case.gold {
                eat(g.as_bytes());
            }
        }
        format!("{h:016x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = r#"{"name":"demo","cases":[
        {"question":"where do backups live","gold":["a1b2"]},
        {"question":"how is a release cut","gold":["c3d4","e5f6"]}]}"#;

    #[test]
    fn a_corpus_reads_back_with_its_cases() {
        let c = Corpus::from_json(RAW).unwrap();
        assert_eq!(c.name, "demo");
        assert_eq!(c.cases.len(), 2);
        assert_eq!(c.cases[1].gold, vec!["c3d4", "e5f6"]);
    }

    #[test]
    fn a_gold_too_short_to_mean_anything_is_refused() {
        let tiny = r#"{"name":"x","cases":[{"question":"q","gold":["a1"]}]}"#;
        let e = Corpus::from_json(tiny).unwrap_err();
        assert!(e.contains("shorter than"), "got: {e}");
    }

    #[test]
    fn a_corpus_with_nothing_to_ask_is_refused() {
        assert!(Corpus::from_json(r#"{"name":"x","cases":[]}"#).is_err());
        let no_gold = r#"{"name":"x","cases":[{"question":"q","gold":[]}]}"#;
        assert!(Corpus::from_json(no_gold).is_err());
    }

    #[test]
    fn the_example_shipped_in_the_repository_loads() {
        // An example whose format drifted from the parser is a bug discovered
        // by the first stranger who tries it.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/example.json");
        let c = Corpus::load(&path).expect("corpus/example.json must stay loadable");
        assert!(!c.cases.is_empty());
    }

    #[test]
    fn the_fingerprint_follows_the_questions_not_the_formatting() {
        let pretty =
            serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(RAW).unwrap())
                .unwrap();
        assert_eq!(
            Corpus::from_json(RAW).unwrap().fingerprint(),
            Corpus::from_json(&pretty).unwrap().fingerprint(),
            "reformatting a file must not look like a refreshed corpus"
        );
        // Editing one character of one question must move it.
        let edited = RAW.replace("where do backups live", "where do backups sleep");
        assert_ne!(
            Corpus::from_json(RAW).unwrap().fingerprint(),
            Corpus::from_json(&edited).unwrap().fingerprint()
        );
        // So must dropping a case: the denominator changed.
        let shorter =
            r#"{"name":"demo","cases":[{"question":"where do backups live","gold":["a1b2"]}]}"#;
        assert_ne!(
            Corpus::from_json(RAW).unwrap().fingerprint(),
            Corpus::from_json(shorter).unwrap().fingerprint()
        );
    }
}
