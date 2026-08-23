//! askable — a regression judge for ranked search.
//!
//! ```text
//! askable run  --candidate mine --corpus corpus/example.json
//! askable tail --match rag=true --show query,ms_total --last 20
//! ```
//!
//! This file holds the command line, the network and the rendering. Everything
//! that can be decided without those lives in the library, where it is tested.

use askable::candidate::{Candidate, Config};
use askable::corpus::Corpus;
use askable::record::{Meta, Record, mrr, recall_at, utc_iso};
use askable::replay::{Hit, hits_from, replay};
use askable::{Event, events};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_LAST: usize = 20;
const DEFAULT_K: usize = 10;
const DEFAULT_CONFIG: &str = "askable.toml";
/// Generous on purpose: a reranker can take seconds, and a timeout that fires
/// on a slow-but-working service would abort the run for nothing.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// Wide enough to read a sentence, narrow enough that a column of them still
/// lines up on an 80-column terminal.
const MAX_CELL: usize = 44;

const USAGE: &str = "usage:
  askable run  --candidate NAME --corpus FILE [--config FILE] [--k N] [--out FILE]
  askable tail [--match k=v]... [--show a,b,c] [--last N]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("tail") => tail(&args[1..]),
        _ => Err(USAGE.to_string()),
    };
    if let Err(e) = outcome {
        eprintln!("askable: {e}");
        std::process::exit(2);
    }
}

// ---------------------------------------------------------------- run

struct RunArgs {
    candidate: String,
    corpus: PathBuf,
    config: PathBuf,
    k: usize,
    out: Option<PathBuf>,
}

fn run_args(args: &[String]) -> Result<RunArgs, String> {
    let mut candidate = None;
    let mut corpus = None;
    let mut a = RunArgs {
        candidate: String::new(),
        corpus: PathBuf::new(),
        config: PathBuf::from(DEFAULT_CONFIG),
        k: DEFAULT_K,
        out: None,
    };
    let mut i = 0;
    while i < args.len() {
        let value = || {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} expects a value", args[i]))
        };
        match args[i].as_str() {
            "--candidate" => candidate = Some(value()?),
            "--corpus" => corpus = Some(PathBuf::from(value()?)),
            "--config" => a.config = PathBuf::from(value()?),
            "--out" => a.out = Some(PathBuf::from(value()?)),
            "--k" => {
                a.k = value()?
                    .parse()
                    .map_err(|_| "--k wants a number".to_string())?
            }
            other => return Err(format!("unknown flag `{other}`\n{USAGE}")),
        }
        i += 2;
    }
    a.candidate = candidate.ok_or("run needs --candidate NAME")?;
    a.corpus = corpus.ok_or("run needs --corpus FILE")?;
    if a.k == 0 {
        return Err("--k 0 would ask for no result at all".into());
    }
    Ok(a)
}

fn run(args: &[String]) -> Result<(), String> {
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
            url: cand.url.clone(),
            corpus: corpus.name.clone(),
            cases: corpus.cases.len(),
            corpus_fingerprint: corpus.fingerprint(),
            k: a.k,
            recall_at_1: recall_at(&outcomes, 1),
            recall_at_5: recall_at(&outcomes, 5),
            recall_at_10: recall_at(&outcomes, 10),
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
    report(&record);
    eprintln!("askable: record written to {}", path.display());
    Ok(())
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

/// Progress on one rewritten line, and only when someone is watching.
///
/// Redirected to a file, a carriage return every two seconds produces a log
/// nobody can read.
fn show_progress(done: usize, total: usize, started: Instant) {
    if !io::stderr().is_terminal() {
        return;
    }
    eprint!("\r{done}/{total}  {}s", started.elapsed().as_secs());
    let _ = io::stderr().flush();
}

fn write_record(record: &Record, path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let json =
        serde_json::to_string_pretty(record).map_err(|e| format!("cannot serialise: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn report(record: &Record) {
    let m = &record.meta;
    println!("candidate  {}", m.candidate);
    println!("corpus     {} ({} cases, {})", m.corpus, m.cases, m.corpus_fingerprint);
    println!("recall@1   {:.4}", m.recall_at_1);
    println!("recall@5   {:.4}", m.recall_at_5);
    println!("recall@10  {:.4}", m.recall_at_10);
    println!("mrr        {:.4}", m.mrr);
    // The judge has a floor, and it belongs next to the numbers rather than in
    // a README nobody rereads: below it, a change is invisible to this corpus.
    println!(
        "\n{} cases, {} out of reach. A difference smaller than about {} discordant \
cases is not detectable on this corpus.",
        m.cases,
        record.outcomes.iter().filter(|o| o.rank.is_none()).count(),
        discordant_floor()
    );
}

/// Smallest number of one-sided discordant cases that an exact McNemar test
/// calls significant at 5% — the resolution of the judge, in cases.
///
/// With b regressions and no improvement the two-sided exact p-value is
/// 2 / 2^b, so the floor is the first b where that drops under 0.05. It does
/// NOT depend on the size of the corpus, which is the whole reason a paired
/// comparison beats comparing two rates: adding cases widens what you can
/// measure, it does not change how small a difference you can see.
fn discordant_floor() -> usize {
    let mut b = 1usize;
    while 2.0 / 2f64.powi(b as i32) >= 0.05 {
        b += 1;
    }
    b
}

// ---------------------------------------------------------------- tail

struct TailArgs {
    filters: Vec<(String, String)>,
    columns: Vec<String>,
    last: usize,
}

/// Twenty lines of argument parsing beat one more dependency for as long as the
/// surface fits in three flags.
fn tail_args(args: &[String]) -> Result<TailArgs, String> {
    let mut o = TailArgs {
        filters: Vec::new(),
        columns: Vec::new(),
        last: DEFAULT_LAST,
    };
    let mut i = 0;
    while i < args.len() {
        let value = || {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} expects a value", args[i]))
        };
        match args[i].as_str() {
            "--match" => {
                let kv = value()?;
                let (k, v) = kv
                    .split_once('=')
                    .ok_or_else(|| format!("--match wants key=value, got `{kv}`"))?;
                o.filters.push((k.to_string(), v.to_string()));
            }
            "--show" => o.columns = value()?.split(',').map(|s| s.trim().to_string()).collect(),
            "--last" => {
                o.last = value()?
                    .parse()
                    .map_err(|_| "--last wants a number".to_string())?
            }
            other => return Err(format!("unknown flag `{other}`\n{USAGE}")),
        }
        i += 2;
    }
    Ok(o)
}

/// The columns to print: the ones asked for, or every field that shows up.
///
/// Falling back to all fields matters the first time someone runs this against
/// an unfamiliar service — you cannot name a column you have never seen.
fn columns(opts: &TailArgs, found: &[Event]) -> Vec<String> {
    if !opts.columns.is_empty() {
        return opts.columns.clone();
    }
    let mut seen: Vec<String> = Vec::new();
    for e in found {
        for k in e.fields.keys() {
            if !seen.contains(k) {
                seen.push(k.clone());
            }
        }
    }
    seen
}

fn clip(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        return s.to_string();
    }
    format!("{}…", c[..n.saturating_sub(1)].iter().collect::<String>())
}

fn render(found: &[Event], cols: &[String]) {
    // One pass to size the columns, so nothing is padded to a width the data
    // never needs.
    let mut widths: Vec<usize> = cols.iter().map(|c| c.chars().count()).collect();
    for e in found {
        for (i, c) in cols.iter().enumerate() {
            let cell = e.fields.get(c).map(String::as_str).unwrap_or("-");
            widths[i] = widths[i].max(clip(cell, MAX_CELL).chars().count());
        }
    }
    let head: Vec<String> = cols
        .iter()
        .zip(&widths)
        .map(|(c, w)| format!("{c:<w$}"))
        .collect();
    println!("{:<9} {}", "time", head.join("  "));
    println!("{}", "-".repeat(9 + head.join("  ").chars().count() + 1));
    for e in found {
        let row: Vec<String> = cols
            .iter()
            .zip(&widths)
            .map(|(c, w)| {
                // A dash, never a blank: absent and empty must not look alike.
                let cell = e.fields.get(c).map(String::as_str).unwrap_or("-");
                format!("{:<w$}", clip(cell, MAX_CELL))
            })
            .collect();
        println!("{:<9} {}", e.time, row.join("  "));
    }
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
    render(shown, &columns(&opts, shown));
    eprintln!("askable: {} matched, {} shown", found.len(), shown.len());
    Ok(())
}
