//! askable - a regression judge for ranked search.
//!
//! ```text
//! askable run   --candidate mine --corpus corpus/example.json --label v1
//! askable judge records/a.json records/b.json
//! askable tail  --match rag=true --show query,ms_total --last 20
//! ```
//!
//! What is left here is what touches the world: the network, the terminal, the
//! clock, the exit code. Everything that decides or composes lives in the
//! library, where it is tested without a process.

use askable::candidate::Candidate;
use askable::cli::{DEFAULT_CONFIG, USAGE, run_args, tail_args};
use askable::config::Config;
use askable::corpus::Corpus;
use askable::events;
use askable::record::{Meta, Record, mrr, recall_within, utc_iso};
use askable::replay::{Hit, hits_from, replay};
use askable::report::{report, report_verdict};
use askable::store::{judge_settings, read_record, write_record};
use askable::table::{columns, render};
use askable::verdict::{AtCutoff, compare};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Generous on purpose: a reranker can take seconds, and a timeout that fires
/// on a slow-but-working service would abort the run for nothing.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("judge") => judge(&args[1..]),
        Some("tail") => tail(&args[1..]).map(|()| 0),
        _ => Err(USAGE.to_string()),
    };
    match outcome {
        // A verdict leaves through the exit code, because that is the only part
        // of it a pipeline reads.
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("askable: {e}");
            std::process::exit(2);
        }
    }
}

fn run(args: &[String]) -> Result<i32, String> {
    let a = run_args(args)?;
    let config = Config::load(&a.config)?;
    let cand = config.get(&a.candidate)?;
    let corpus = Corpus::load(&a.corpus)?;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into();
    let started = Instant::now();
    let outcomes = replay(
        &corpus,
        cand,
        a.k,
        |url| ask(&agent, url, cand),
        |done, total| show_progress(done, total, started),
    )?;
    if io::stderr().is_terminal() {
        eprintln!();
    }

    let record = Record {
        meta: Meta {
            produced_at: utc_iso(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| "the system clock is before 1970")?
                    .as_secs(),
            ),
            candidate: a.candidate.clone(),
            candidate_version: Some(a.label.clone()),
            url: cand.url.clone(),
            corpus: corpus.name.clone(),
            cases: corpus.cases.len(),
            corpus_fingerprint: corpus.fingerprint(),
            k: a.k,
            recall_at_1: recall_within(&outcomes, 1, a.k),
            recall_at_5: recall_within(&outcomes, 5, a.k),
            recall_at_10: recall_within(&outcomes, 10, a.k),
            mrr: mrr(&outcomes),
            seconds: started.elapsed().as_secs(),
        },
        outcomes,
    };

    let path = a.out.unwrap_or_else(|| {
        PathBuf::from("records").join(format!(
            "{}-{}.json",
            record.meta.produced_at.replace(':', ""),
            a.candidate
        ))
    });
    write_record(&record, &path)?;
    print!("{}", report(&record));
    eprintln!("askable: record written to {}", path.display());

    // The record is on disk BEFORE the verdict runs. A failing comparison must
    // not throw away the measurement that proves it.
    let Some(reference_path) = a.judge else {
        return Ok(0);
    };
    let reference = read_record(&reference_path)?;
    let at = compare(&reference, &record, &judge_settings(&a.config)?)?;
    println!();
    print!(
        "{}",
        report_verdict(
            &reference,
            &record,
            &at,
            &reference_path.display().to_string(),
            &path.display().to_string(),
        )
    );
    Ok(if at.iter().any(AtCutoff::failed) {
        1
    } else {
        0
    })
}

/// One HTTP call, turned into hits or into an error that names the case.
fn ask(agent: &ureq::Agent, url: &str, cand: &Candidate) -> Result<Vec<Hit>, String> {
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    // A non-200 is not "no result": the service refused, and pretending it
    // returned nothing would score the refusal as a retrieval failure.
    let status = response.status();
    if status != 200 {
        return Err(format!("the service answered {status}"));
    }
    let body: serde_json::Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("the answer is not JSON: {e}"))?;
    hits_from(&body, cand)
}

/// Redirected to a file, a carriage return every two seconds produces a log
/// nobody can read.
fn show_progress(done: usize, total: usize, started: Instant) {
    if !io::stderr().is_terminal() {
        return;
    }
    eprint!("\r{done}/{total}  {}s", started.elapsed().as_secs());
    let _ = io::stderr().flush();
}

fn judge(args: &[String]) -> Result<i32, String> {
    let mut files: Vec<&String> = Vec::new();
    let mut config = PathBuf::from(DEFAULT_CONFIG);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                config = PathBuf::from(args.get(i + 1).ok_or("--config expects a value")?);
                i += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(format!("unknown flag `{flag}`\n{USAGE}"));
            }
            _ => {
                files.push(&args[i]);
                i += 1;
            }
        }
    }
    let [reference_path, candidate_path] = files.as_slice() else {
        return Err(format!(
            "judge wants two records, got {}\n{USAGE}",
            files.len()
        ));
    };
    // The judge's strictness is configuration, so that a FAIL can be argued
    // with in a diff rather than in a shell history.
    let settings = judge_settings(&config)?;
    let reference = read_record(Path::new(reference_path))?;
    let candidate = read_record(Path::new(candidate_path))?;
    let at = compare(&reference, &candidate, &settings)?;

    print!(
        "{}",
        report_verdict(&reference, &candidate, &at, reference_path, candidate_path)
    );
    Ok(if at.iter().any(AtCutoff::failed) {
        1
    } else {
        0
    })
}

fn tail(args: &[String]) -> Result<(), String> {
    let opts = tail_args(args)?;
    let mut raw = String::new();
    io::stdin()
        .read_to_string(&mut raw)
        .map_err(|e| format!("cannot read stdin: {e}"))?;

    let found = events(&raw, &opts.filters);
    let start = found.len().saturating_sub(opts.last);
    let shown = &found[start..];

    if shown.is_empty() {
        // "Nothing matched" and "nothing was piped in" are different problems
        // and would otherwise print the same silent table.
        eprintln!(
            "askable: no event matched{}",
            if raw.trim().is_empty() {
                " (the input was empty)"
            } else {
                ""
            }
        );
        return Ok(());
    }
    print!("{}", render(shown, &columns(&opts, shown)));
    eprintln!("askable: {} matched, {} shown", found.len(), shown.len());
    Ok(())
}
