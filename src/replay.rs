//! Replaying a corpus against a candidate.
//!
//! The network sits behind a closure. That is not ceremony: it is what lets the
//! rule below be tested — one failed case stops the whole run — without standing
//! up a server that fails on demand.

use crate::candidate::Candidate;
use crate::corpus::Corpus;
use crate::record::{Outcome, rank_of};
use serde_json::Value;
use std::collections::BTreeMap;

/// One result, reduced to what a judge can use.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: String,
    /// Every numeric field this result carried.
    pub numbers: BTreeMap<String, f64>,
}

/// Turns a service's JSON answer into hits.
///
/// Anything that is not a list of objects with an id is a configuration
/// mistake, and it is reported as one rather than counted as "no result" —
/// a service that answers 200 with the wrong shape would otherwise score zero
/// and look like a retrieval failure.
pub fn hits_from(body: &Value, cand: &Candidate) -> Result<Vec<Hit>, String> {
    let array = match &cand.results_at {
        Some(key) => body
            .get(key)
            .ok_or_else(|| format!("the answer has no `{key}` field"))?,
        None => body,
    };
    let array = array
        .as_array()
        .ok_or_else(|| "the answer is not a list of results".to_string())?;

    array
        .iter()
        .map(|item| {
            let id = item
                .get(&cand.id_field)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("a result carries no `{}` string", cand.id_field))?;
            let numbers = item
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
                        .collect()
                })
                .unwrap_or_default();
            Ok(Hit {
                id: id.to_string(),
                numbers,
            })
        })
        .collect()
}

/// Asks every question of the corpus and records where the right answer landed.
///
/// `fetch` receives a ready-made URL and returns the ranked hits, or an error.
/// `progress` is called with `(done, total)` after each case, because 63 cases
/// at two seconds each is two minutes of a silent terminal otherwise.
pub fn replay<F, P>(
    corpus: &Corpus,
    cand: &Candidate,
    k: usize,
    mut fetch: F,
    mut progress: P,
) -> Result<Vec<Outcome>, String>
where
    F: FnMut(&str) -> Result<Vec<Hit>, String>,
    P: FnMut(usize, usize),
{
    let total = corpus.cases.len();
    let mut out = Vec::with_capacity(total);
    for (i, case) in corpus.cases.iter().enumerate() {
        // One failure stops everything. A record missing a third of its cases
        // still computes an average, and that average looks exactly like a
        // measurement — refusing to produce one is the only honest answer.
        let hits = fetch(&cand.request_url(&case.question, k))
            .map_err(|e| format!("case {}/{} ({:?}): {e}", i + 1, total, case.question))?;
        let returned: Vec<String> = hits.iter().map(|h| h.id.clone()).collect();
        out.push(Outcome {
            rank: rank_of(&returned, &case.gold),
            top_numbers: hits.first().map(|h| h.numbers.clone()).unwrap_or_default(),
            question: case.question.clone(),
            gold: case.gold.clone(),
            returned,
        });
        progress(i + 1, total);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::Config;

    fn corpus() -> Corpus {
        Corpus::from_json(
            r#"{"name":"t","cases":[
                {"question":"one","gold":["aaa"]},
                {"question":"two","gold":["bbb"]},
                {"question":"three","gold":["ccc"]}]}"#,
        )
        .unwrap()
    }

    fn candidate() -> Candidate {
        Config::from_toml("[candidate.t]\nurl = \"http://x/s?q={query}&k={k}\"")
            .unwrap()
            .get("t")
            .unwrap()
            .clone()
    }

    fn hit(id: &str) -> Hit {
        Hit {
            id: id.into(),
            numbers: BTreeMap::new(),
        }
    }

    #[test]
    fn every_case_is_asked_and_ranked() {
        let mut seen = Vec::new();
        let out = replay(
            &corpus(),
            &candidate(),
            10,
            |url| {
                seen.push(url.to_string());
                Ok(vec![hit("zzz1"), hit("bbb2")])
            },
            |_, _| {},
        )
        .unwrap();
        assert_eq!(seen.len(), 3);
        assert!(seen[0].ends_with("q=one&k=10"));
        // Only the second case had its expected answer come back.
        assert_eq!(
            out.iter().map(|o| o.rank).collect::<Vec<_>>(),
            vec![None, Some(2), None]
        );
    }

    #[test]
    fn one_failed_case_aborts_the_whole_run() {
        let mut asked = 0;
        let err = replay(
            &corpus(),
            &candidate(),
            10,
            |_| {
                asked += 1;
                if asked == 2 {
                    Err("connection refused".into())
                } else {
                    Ok(vec![hit("aaa")])
                }
            },
            |_, _| {},
        )
        .unwrap_err();
        // It stopped AT the failure, and said which case and why.
        assert_eq!(asked, 2);
        assert!(err.contains("case 2/3"), "got: {err}");
        assert!(err.contains("connection refused"), "got: {err}");
    }

    #[test]
    fn a_wrong_shape_is_a_configuration_error_not_an_empty_result() {
        let cand = candidate();
        assert!(hits_from(&serde_json::json!({"oops": 1}), &cand).is_err());
        assert!(hits_from(&serde_json::json!([{"name": "x"}]), &cand).is_err());
        // Nested results, and the numbers of the top hit are kept as they came.
        let mut nested = cand.clone();
        nested.results_at = Some("results".into());
        let body = serde_json::json!({"results":[{"id":"aaa","score":0.5,"note":"x"}]});
        let hits = hits_from(&body, &nested).unwrap();
        assert_eq!(hits[0].id, "aaa");
        assert_eq!(hits[0].numbers.get("score"), Some(&0.5));
        assert_eq!(hits[0].numbers.get("note"), None);
    }
}
