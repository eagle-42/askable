//! What one replay produced: one line per case, then the aggregates.
//!
//! A record is the unit the judge compares. It carries enough identity to be
//! compared honestly later — which system, which version, which corpus — because
//! a number without those three is not a measurement, it is a souvenir.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Outcome {
    pub question: String,
    pub gold: Vec<String>,
    /// 1-based rank of the first expected answer, or `null`.
    ///
    /// Absent is NOT zero and NOT 999. "Nothing right came back" and "the right
    /// answer came back first" are different facts, and a sentinel number would
    /// quietly average them together.
    pub rank: Option<usize>,
    pub returned: Vec<String>,
    /// Every numeric field the top result carried, whatever it is called.
    ///
    /// Not a fixed list: the scores that explain a failure belong to whoever
    /// emitted them, and hard-coding three names here would blind the judge to
    /// the fourth.
    pub top_numbers: BTreeMap<String, f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Meta {
    pub produced_at: String,
    pub candidate: String,
    /// Whatever identifies the version that was measured — a commit, a model
    /// name, a row count. askable cannot discover it: only the person running
    /// the replay knows what they just changed, so `run --label` asks them.
    ///
    /// `None` is allowed and visible in the verdict. A comparison between two
    /// unlabelled records is still arithmetic; it just cannot say what changed.
    #[serde(default)]
    pub candidate_version: Option<String>,
    pub url: String,
    pub corpus: String,
    pub cases: usize,
    /// Travels beside the candidate's identity on purpose: refreshing the
    /// corpus retires the previous reference, it does not extend it.
    pub corpus_fingerprint: String,
    pub k: usize,
    /// `null` when the cutoff is beyond the `k` results that were asked for.
    ///
    /// Printing recall@10 after asking for five results would report a ceiling
    /// as a measurement — and a number nobody measured is the exact shape of
    /// the failures this tool exists to catch.
    pub recall_at_1: Option<f64>,
    pub recall_at_5: Option<f64>,
    pub recall_at_10: Option<f64>,
    pub mrr: f64,
    pub seconds: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Record {
    pub meta: Meta,
    pub outcomes: Vec<Outcome>,
}

/// The rank of the first expected answer among the ids that came back.
///
/// A gold entry matches by PREFIX: a corpus is written by a human, who types a
/// short id, while a service usually returns the full one.
pub fn rank_of(returned: &[String], gold: &[String]) -> Option<usize> {
    returned
        .iter()
        .position(|id| gold.iter().any(|g| id.starts_with(g.as_str())))
        .map(|i| i + 1)
}

/// Share of cases whose expected answer landed in the first `k`.
///
/// An empty slice yields 0.0 and never divides by zero — but a caller should
/// never get there: an empty corpus is refused at load time.
pub fn recall_at(outcomes: &[Outcome], k: usize) -> f64 {
    if outcomes.is_empty() {
        return 0.0;
    }
    let hits = outcomes
        .iter()
        .filter(|o| o.rank.is_some_and(|r| r <= k))
        .count();
    hits as f64 / outcomes.len() as f64
}

/// Recall at `cutoff`, or `None` when only `k` results were ever requested.
pub fn recall_within(outcomes: &[Outcome], cutoff: usize, k: usize) -> Option<f64> {
    (cutoff <= k).then(|| recall_at(outcomes, cutoff))
}

/// Mean reciprocal rank. A case with no expected answer in reach contributes 0.
pub fn mrr(outcomes: &[Outcome]) -> f64 {
    if outcomes.is_empty() {
        return 0.0;
    }
    let s: f64 = outcomes
        .iter()
        .filter_map(|o| o.rank)
        .map(|r| 1.0 / r as f64)
        .sum::<f64>()
        // `Sum for f64` starts from NEGATIVE zero, so a corpus where nothing
        // was found printed `-0.0000` — which reads as a bug in the tool rather
        // than as a run that found nothing. Adding zero normalises the sign.
        + 0.0;
    s / outcomes.len() as f64
}

/// UTC as `YYYY-MM-DDTHH:MM:SSZ`, from seconds since the epoch.
///
/// Twenty lines of civil-calendar arithmetic instead of a date crate, for a
/// program that formats exactly one timestamp per run. The algorithm is
/// Howard Hinnant's `civil_from_days`, and the test below pins four dates
/// including a leap day.
pub fn utc_iso(unix: u64) -> String {
    let (days, secs) = (unix / 86_400, unix % 86_400);
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(rank: Option<usize>) -> Outcome {
        Outcome {
            question: "q".into(),
            gold: vec!["a".into()],
            rank,
            returned: Vec::new(),
            top_numbers: BTreeMap::new(),
        }
    }

    #[test]
    fn a_short_gold_matches_a_full_identifier() {
        let returned = vec![
            "2eebc3e2-b77c-4180".to_string(),
            "af076cc4-0000".to_string(),
        ];
        assert_eq!(rank_of(&returned, &["2eebc3e2".into()]), Some(1));
        assert_eq!(rank_of(&returned, &["af076cc4".into()]), Some(2));
        // Several golds: the FIRST one to appear wins, not the first listed.
        assert_eq!(
            rank_of(&returned, &["af076cc4".into(), "2eebc3e2".into()]),
            Some(1)
        );
        assert_eq!(rank_of(&returned, &["deadbeef".into()]), None);
    }

    #[test]
    fn a_case_out_of_reach_is_absent_not_last() {
        let all = vec![outcome(Some(1)), outcome(Some(4)), outcome(None)];
        assert!((recall_at(&all, 1) - 1.0 / 3.0).abs() < 1e-9);
        assert!((recall_at(&all, 5) - 2.0 / 3.0).abs() < 1e-9);
        // The absent case adds nothing to the MRR — it does not add 1/999.
        assert!((mrr(&all) - (1.0 + 0.25) / 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_cutoff_beyond_what_was_asked_for_is_absent() {
        let all = vec![outcome(Some(1))];
        assert_eq!(recall_within(&all, 1, 5), Some(1.0));
        assert_eq!(recall_within(&all, 5, 5), Some(1.0));
        // Asked for five, so recall@10 was never measured. Not 1.0, not 0.0.
        assert_eq!(recall_within(&all, 10, 5), None);
    }

    #[test]
    fn a_run_that_found_nothing_reports_plain_zero() {
        let none = vec![outcome(None), outcome(None)];
        let m = mrr(&none);
        assert_eq!(m, 0.0);
        assert!(
            !m.is_sign_negative(),
            "negative zero reads as a broken tool"
        );
        assert_eq!(format!("{m:.4}"), "0.0000");
    }

    #[test]
    fn the_aggregates_survive_an_empty_slice() {
        assert_eq!(recall_at(&[], 1), 0.0);
        assert_eq!(mrr(&[]), 0.0);
    }

    #[test]
    fn the_timestamp_is_right_including_a_leap_day() {
        // Four dates let three mutations survive in the day-of-month
        // computation: they all fell on nearly the same day of the month.
        // These cross months of 28, 29, 30 and 31 days, the century
        // rollover, and both ends of a year.
        for (seconds, expected) in [
            (0u64, "1970-01-01T00:00:00Z"),
            // THE PIVOT. The algorithm counts years from March 1st, and that is
            // where the century correction term switches sides. Three mutations
            // survived here: with the sign flipped, this date becomes 1970-02-29,
            // a day that does not exist. Found by searching, not by
            // guessing.
            (5_097_600, "1970-03-01T00:00:00Z"),
            (5_270_400, "1970-03-03T00:00:00Z"),
            (946_684_800, "2000-01-01T00:00:00Z"),
            (951_782_400, "2000-02-29T00:00:00Z"),
            (1_709_251_200, "2024-03-01T00:00:00Z"),
            (1_769_903_999, "2026-01-31T23:59:59Z"),
            (1_772_323_200, "2026-03-01T00:00:00Z"),
            (1_782_777_600, "2026-06-30T00:00:00Z"),
            (1_789_117_199, "2026-09-11T08:59:59Z"),
            (1_789_117_200, "2026-09-11T09:00:00Z"),
            (1_798_718_400, "2026-12-31T12:00:00Z"),
        ] {
            assert_eq!(utc_iso(seconds), expected, "for {seconds}");
        }
    }
}
