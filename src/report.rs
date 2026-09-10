//! What a run and a verdict look like once written down.
//!
//! Every function here composes a string and returns it. A function that
//! prints cannot be asserted on, and half of what this tool says to a human
//! used to live where no test could reach it.

use crate::record::Record;
use crate::verdict::AtCutoff;
use std::fmt::Write;

pub fn report(record: &Record) -> String {
    let mut out = String::new();
    let m = &record.meta;
    let _ = writeln!(out, "candidate  {}", m.candidate);
    let _ = writeln!(
        out,
        "corpus     {} ({} cases, {})",
        m.corpus, m.cases, m.corpus_fingerprint
    );
    for (name, value) in [
        ("recall@1 ", m.recall_at_1),
        ("recall@5 ", m.recall_at_5),
        ("recall@10", m.recall_at_10),
    ] {
        match value {
            Some(v) => {
                let _ = writeln!(out, "{name}  {v:.4}");
            }
            // A dash, never a zero: this cutoff was beyond the k asked for.
            None => {
                let _ = writeln!(out, "{name}  -  (beyond the {} results requested)", m.k);
            }
        }
    }
    let _ = writeln!(out, "mrr        {:.4}", m.mrr);
    // The judge has a floor, and it belongs next to the numbers rather than in
    // a README nobody rereads: below it, a change is invisible to this corpus.
    let _ = writeln!(
        out,
        "\n{} cases, {} out of reach. A difference smaller than about {} discordant \
cases is not detectable on this corpus.",
        m.cases,
        record.outcomes.iter().filter(|o| o.rank.is_none()).count(),
        discordant_floor()
    );
    out
}

/// With b regressions and no improvement the two-sided exact p-value is
/// 2 / 2^b, so the floor is the first b where that drops under 0.05. It does
/// NOT depend on the size of the corpus, which is the whole reason a paired
/// comparison beats comparing two rates: adding cases widens what you can
/// measure, it does not change how small a difference you can see.
pub fn discordant_floor() -> usize {
    let mut b = 1usize;
    while 2.0 / 2f64.powi(b as i32) >= 0.05 {
        b += 1;
    }
    b
}

pub fn report_verdict(
    reference: &Record,
    candidate: &Record,
    at: &[AtCutoff],
    reference_path: &str,
    candidate_path: &str,
) -> String {
    let mut out = String::new();
    // An unlabelled record still compares; it just cannot say what changed, and
    // saying so is more useful than leaving the column blank.
    let label = |r: &Record| {
        r.meta
            .candidate_version
            .clone()
            .unwrap_or_else(|| "unlabelled".into())
    };
    let _ = writeln!(
        out,
        "reference  {reference_path}  {} @ {}  {}",
        reference.meta.candidate,
        label(reference),
        reference.meta.produced_at
    );
    let _ = writeln!(
        out,
        "candidate  {candidate_path}  {} @ {}  {}",
        candidate.meta.candidate,
        label(candidate),
        candidate.meta.produced_at
    );
    let _ = writeln!(
        out,
        "corpus     {} ({} cases, {}), k={}\n",
        reference.meta.corpus,
        reference.meta.cases,
        reference.meta.corpus_fingerprint,
        reference.meta.k
    );
    let _ = writeln!(
        out,
        "cutoff  reference  candidate  regressed  improved      p  verdict"
    );
    for a in at {
        let mark = if a.below_floor.is_some() {
            "under floor"
        } else if a.significant_regression {
            "REGRESSION"
        } else {
            "ok"
        };
        let _ = writeln!(
            out,
            "{:>6}  {:>9.4}  {:>9.4}  {:>9}  {:>8}  {:>5.3}  {mark}",
            a.cutoff,
            a.reference_recall,
            a.candidate_recall,
            a.regressions.len(),
            a.improvements.len(),
            a.p_value
        );
    }
    for a in at.iter().filter(|a| !a.regressions.is_empty()) {
        let _ = writeln!(out, "\ncases lost at rank {}:", a.cutoff);
        for q in &a.regressions {
            let _ = writeln!(out, "  {q}");
        }
    }
    let failed = at.iter().any(AtCutoff::failed);
    let _ = writeln!(
        out,
        "\n{}",
        if failed {
            "FAIL - this corpus can see this regression".to_string()
        } else {
            format!(
                "PASS - no regression this corpus can see. Its floor is {} discordant cases; \
anything smaller is invisible here, whatever the number of cases.",
                discordant_floor()
            )
        }
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Judge;
    use crate::record::{Meta, Outcome};
    use std::collections::BTreeMap;

    fn record(ranks: &[Option<usize>], k: usize) -> Record {
        Record {
            meta: Meta {
                produced_at: "2026-01-01T00:00:00Z".into(),
                candidate: "c".into(),
                candidate_version: Some("v1".into()),
                url: "http://x/{query}".into(),
                corpus: "bench".into(),
                cases: ranks.len(),
                corpus_fingerprint: "ffff".into(),
                k,
                recall_at_1: crate::record::recall_within(&[], 1, k),
                recall_at_5: None,
                recall_at_10: None,
                mrr: 0.5,
                seconds: 1,
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
    fn a_cutoff_beyond_k_is_a_dash_and_says_why() {
        let mut r = record(&[Some(1)], 5);
        r.meta.recall_at_1 = Some(1.0);
        r.meta.recall_at_10 = None;
        let t = report(&r);
        assert!(t.contains("recall@1   1.0000"), "got: {t}");
        // A ceiling does not present itself as a measurement.
        assert!(t.contains("recall@10  -"), "got: {t}");
        assert!(t.contains("beyond the 5 results"), "got: {t}");
    }

    #[test]
    fn the_report_says_how_many_cases_are_out_of_reach() {
        let t = report(&record(&[Some(1), None, None], 10));
        assert!(t.contains("3 cases, 2 out of reach"), "got: {t}");
        // And it recalls its own resolution, where the numbers are read.
        assert!(t.contains("6 discordant"), "got: {t}");
    }

    #[test]
    fn the_floor_is_six_and_does_not_depend_on_anything() {
        assert_eq!(discordant_floor(), 6);
    }

    #[test]
    fn a_verdict_names_what_it_lost_and_says_pass_or_fail() {
        let before = record(&[Some(1); 10], 10);
        let mut after = before.clone();
        for o in after.outcomes.iter_mut().take(7) {
            o.rank = Some(9);
        }
        let at = crate::verdict::compare(&before, &after, &Judge::default()).unwrap();
        let t = report_verdict(&before, &after, &at, "a.json", "b.json");
        assert!(t.contains("REGRESSION"), "got: {t}");
        assert!(t.contains("FAIL"), "got: {t}");
        // The actionable half: the lost cases are NAMED.
        assert!(t.contains("q0") && t.contains("q6"), "got: {t}");
        // And the identity of what is compared is at the top.
        assert!(t.contains("v1"), "the label has to show: {t}");
        assert!(t.contains("bench") && t.contains("ffff"), "got: {t}");
    }

    #[test]
    fn an_unlabelled_record_says_so_rather_than_leaving_a_blank() {
        let mut before = record(&[Some(1)], 10);
        before.meta.candidate_version = None;
        let at = crate::verdict::compare(&before, &before, &Judge::default()).unwrap();
        let t = report_verdict(&before, &before, &at, "a.json", "b.json");
        assert!(t.contains("unlabelled"), "got: {t}");
        assert!(t.contains("PASS"), "got: {t}");
    }
}
