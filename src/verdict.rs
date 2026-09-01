//! Comparing two records of the same corpus, and saying whether to worry.
//!
//! The comparison is PAIRED: the same questions, asked of both, matched one by
//! one. That is not a detail. On a corpus of 63 cases, comparing two recall
//! *rates* needs a 16-point gap before the difference means anything, because a
//! rate of 70% carries a confidence interval 22 points wide. Matching the cases
//! instead throws away everything both runs agree on and looks only at the ones
//! that changed camp — where six regressions against no improvement is already
//! significant.
//!
//! The floor that follows from that is six discordant cases, and it does not
//! depend on how big the corpus is. Adding cases widens what you can measure; it
//! does not sharpen how small a change you can see.

use crate::config::Judge;
use crate::record::{Outcome, Record};

/// What changed at one cutoff.
#[derive(Debug, Clone, PartialEq)]
pub struct AtCutoff {
    pub cutoff: usize,
    pub reference_recall: f64,
    pub candidate_recall: f64,
    /// Cases the reference found and the candidate lost.
    pub regressions: Vec<String>,
    /// Cases the candidate found and the reference missed.
    pub improvements: Vec<String>,
    pub p_value: f64,
    pub significant_regression: bool,
    pub below_floor: Option<f64>,
}

impl AtCutoff {
    pub fn failed(&self) -> bool {
        self.significant_regression || self.below_floor.is_some()
    }
}

/// Two-sided exact McNemar p-value for `b` regressions and `i` improvements.
///
/// Only the discordant pairs carry information: under the hypothesis that the
/// change did nothing, each of them is a coin flip. So the p-value is the
/// binomial tail, and with no improvements at all it is simply 2 / 2^b — which
/// first drops under 5% at b = 6.
pub fn mcnemar(b: usize, i: usize) -> f64 {
    let n = b + i;
    if n == 0 {
        // Nothing changed camp. That is the strongest possible agreement, not a
        // missing measurement.
        return 1.0;
    }
    // Built up term by term rather than through factorials, which overflow long
    // before a corpus does.
    let mut term = 0.5f64.powi(n as i32);
    let mut sum = term;
    for step in 0..b.min(i) {
        term *= (n - step) as f64 / (step + 1) as f64;
        sum += term;
    }
    (2.0 * sum).min(1.0)
}

fn found(o: &Outcome, cutoff: usize) -> bool {
    o.rank.is_some_and(|r| r <= cutoff)
}

/// Compares two records, or says why they cannot be compared.
///
/// Refusing is the point. Two records of different corpora, or asked for a
/// different number of results, produce a number that looks exactly like a
/// comparison and is not one.
pub fn compare(
    reference: &Record,
    candidate: &Record,
    judge: &Judge,
) -> Result<Vec<AtCutoff>, String> {
    if reference.meta.corpus_fingerprint != candidate.meta.corpus_fingerprint {
        return Err(format!(
            "these records are not of the same corpus ({} vs {}). A refresh retires the previous \
reference rather than extending it: re-measure the reference on the new corpus first",
            reference.meta.corpus_fingerprint, candidate.meta.corpus_fingerprint
        ));
    }
    if reference.meta.k != candidate.meta.k {
        return Err(format!(
            "one record asked for {} results and the other for {}; ranks beyond the smaller of \
the two were never measured",
            reference.meta.k, candidate.meta.k
        ));
    }
    if reference.outcomes.len() != candidate.outcomes.len() {
        return Err("the two records do not hold the same number of cases".into());
    }
    if let Some((a, b)) = reference
        .outcomes
        .iter()
        .zip(&candidate.outcomes)
        .find(|(a, b)| a.question != b.question)
    {
        return Err(format!(
            "the cases are not in the same order: {:?} against {:?}",
            a.question, b.question
        ));
    }

    Ok(judge
        .cutoffs
        .iter()
        .map(|&cutoff| {
            let pairs = reference.outcomes.iter().zip(&candidate.outcomes);
            let (mut regressions, mut improvements) = (Vec::new(), Vec::new());
            for (a, b) in pairs {
                match (found(a, cutoff), found(b, cutoff)) {
                    (true, false) => regressions.push(a.question.clone()),
                    (false, true) => improvements.push(a.question.clone()),
                    // Agreement carries no information either way.
                    _ => {}
                }
            }
            let n = reference.outcomes.len() as f64;
            let rate =
                |r: &Record| r.outcomes.iter().filter(|o| found(o, cutoff)).count() as f64 / n;
            let candidate_recall = rate(candidate);
            let p_value = mcnemar(regressions.len(), improvements.len());
            AtCutoff {
                cutoff,
                reference_recall: rate(reference),
                candidate_recall,
                significant_regression: regressions.len() > improvements.len()
                    && p_value < judge.alpha,
                below_floor: judge
                    .floor
                    .iter()
                    .find(|f| f.cutoff == cutoff && candidate_recall < f.recall)
                    .map(|f| f.recall),
                p_value,
                regressions,
                improvements,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Floor;
    use crate::record::Meta;
    use std::collections::BTreeMap;

    fn record(ranks: &[Option<usize>], fingerprint: &str, k: usize) -> Record {
        Record {
            meta: Meta {
                produced_at: "2026-01-01T00:00:00Z".into(),
                candidate: "c".into(),
                candidate_version: None,
                url: "http://x/{query}".into(),
                corpus: "t".into(),
                cases: ranks.len(),
                corpus_fingerprint: fingerprint.into(),
                k,
                recall_at_1: None,
                recall_at_5: None,
                recall_at_10: None,
                mrr: 0.0,
                seconds: 0,
            },
            outcomes: ranks
                .iter()
                .enumerate()
                .map(|(i, r)| Outcome {
                    question: format!("q{i}"),
                    gold: vec!["aaaa".into()],
                    rank: *r,
                    returned: Vec::new(),
                    top_numbers: BTreeMap::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn six_regressions_and_no_improvement_is_the_floor() {
        // The number the whole design rests on: five is not enough, six is.
        assert!((mcnemar(5, 0) - 0.0625).abs() < 1e-12);
        assert!((mcnemar(6, 0) - 0.03125).abs() < 1e-12);
        assert!(mcnemar(5, 0) > 0.05 && mcnemar(6, 0) < 0.05);
    }

    #[test]
    fn agreement_and_symmetry_hold() {
        // Nothing changed camp: the strongest agreement, not a missing number.
        assert_eq!(mcnemar(0, 0), 1.0);
        // A change that moves as many cases each way says nothing.
        assert_eq!(mcnemar(3, 3), 1.0);
        // The test does not care which side it is asked from.
        assert!((mcnemar(7, 2) - mcnemar(2, 7)).abs() < 1e-12);
        // And it never returns a probability above one.
        for b in 0..12 {
            for i in 0..12 {
                let p = mcnemar(b, i);
                assert!((0.0..=1.0).contains(&p), "p={p} for b={b} i={i}");
            }
        }
    }

    #[test]
    fn a_real_regression_is_caught_and_named() {
        let before = record(&[Some(1); 10], "f", 10);
        let mut after = before.clone();
        for o in after.outcomes.iter_mut().take(6) {
            o.rank = Some(3);
        }
        let at = compare(&before, &after, &Judge::default()).unwrap();
        let at1 = &at[0];
        assert_eq!(at1.regressions.len(), 6);
        assert_eq!(at1.improvements.len(), 0);
        assert!(at1.significant_regression && at1.failed());
        // Naming them is the actionable half of the verdict.
        assert!(at1.regressions.contains(&"q0".to_string()));
        // At cutoff 5 nothing moved: rank 3 is still inside the top five.
        assert!(!at[1].failed());
        assert_eq!(at[1].regressions.len(), 0);
    }

    #[test]
    fn a_change_under_the_floor_is_not_called_a_regression() {
        let before = record(&[Some(1); 10], "f", 10);
        let mut after = before.clone();
        after.outcomes[0].rank = Some(4);
        let at = compare(&before, &after, &Judge::default()).unwrap();
        // One case moved. Real, but below what this corpus can resolve.
        assert_eq!(at[0].regressions.len(), 1);
        assert!(!at[0].significant_regression);
        assert!(!at[0].failed());
    }

    #[test]
    fn an_absolute_floor_still_catches_a_collapse() {
        let before = record(&[Some(1); 10], "f", 10);
        let after = record(&[None; 10], "f", 10);
        let judge = Judge {
            floor: vec![Floor {
                cutoff: 1,
                recall: 0.5,
            }],
            ..Judge::default()
        };
        let at = compare(&before, &after, &judge).unwrap();
        assert_eq!(at[0].below_floor, Some(0.5));
        assert!(at[0].failed());
    }

    #[test]
    fn records_that_cannot_be_compared_are_refused() {
        let a = record(&[Some(1)], "aaa", 10);
        // A refreshed corpus: different fingerprint.
        let e = compare(&a, &record(&[Some(1)], "bbb", 10), &Judge::default()).unwrap_err();
        assert!(e.contains("same corpus"), "got: {e}");
        // A different k: ranks past the smaller one were never measured.
        let e = compare(&a, &record(&[Some(1)], "aaa", 5), &Judge::default()).unwrap_err();
        assert!(e.contains("results"), "got: {e}");
        // Same fingerprint, cases reordered by hand.
        let mut shuffled = record(&[Some(1), Some(1)], "aaa", 10);
        let two = record(&[Some(1), Some(1)], "aaa", 10);
        shuffled.outcomes[0].question = "elsewhere".into();
        assert!(compare(&two, &shuffled, &Judge::default()).is_err());
    }
}
