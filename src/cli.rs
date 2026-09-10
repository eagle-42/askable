//! The command line: turning arguments into intentions.
//!
//! Nothing here reads a file or opens a socket, so every refusal below is a
//! unit test rather than a subprocess.

use std::path::PathBuf;

pub const DEFAULT_LAST: usize = 20;
pub const DEFAULT_K: usize = 10;
pub const DEFAULT_CONFIG: &str = "askable.toml";

pub const USAGE: &str = "usage:
  askable run   --candidate NAME --corpus FILE --label TEXT
                [--judge REFERENCE.json] [--config FILE] [--k N] [--out FILE]
  askable judge REFERENCE.json CANDIDATE.json [--config FILE]
  askable tail  [--match k=v]... [--show a,b,c] [--last N]

exit code: 0 nothing to report, 1 a regression the corpus can see, 2 a mistake";

#[derive(Debug)]
pub struct RunArgs {
    pub candidate: String,
    pub label: String,
    /// A reference record to judge against as soon as the replay is done.
    ///
    /// This is the whole gesture in continuous integration: replay, compare,
    /// leave through the exit code. Two commands would mean the second one can
    /// be forgotten.
    pub judge: Option<PathBuf>,
    pub corpus: PathBuf,
    pub config: PathBuf,
    pub k: usize,
    pub out: Option<PathBuf>,
}

pub fn run_args(args: &[String]) -> Result<RunArgs, String> {
    let mut candidate = None;
    let mut label = None;
    let mut corpus = None;
    let mut a = RunArgs {
        candidate: String::new(),
        label: String::new(),
        judge: None,
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
            "--label" => label = Some(value()?),
            "--judge" => a.judge = Some(PathBuf::from(value()?)),
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
    // Not optional: a record nobody can attribute to a version is an anecdote.
    // askable cannot discover what changed, so it insists that you say it.
    a.label = label.ok_or(
        "run needs --label TEXT, saying WHICH version this measures \
(a commit, a model name, a row count). Nothing else can tell two records apart",
    )?;
    if a.k == 0 {
        return Err("--k 0 would ask for no result at all".into());
    }
    Ok(a)
}

#[derive(Debug)]
pub struct TailArgs {
    pub filters: Vec<(String, String)>,
    pub columns: Vec<String>,
    pub last: usize,
}

/// Twenty lines of argument parsing beat one more dependency for as long as the
/// surface fits in three flags.
pub fn tail_args(args: &[String]) -> Result<TailArgs, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn a_run_without_a_label_is_refused_and_says_why() {
        let e = run_args(&args("--candidate c --corpus f.json")).unwrap_err();
        assert!(e.contains("--label"), "got: {e}");
        assert!(e.contains("WHICH version"), "the message must say what for");
    }

    #[test]
    fn the_whole_ci_gesture_parses() {
        let a = run_args(&args(
            "--candidate mine --corpus bench.json --label abc123 --judge records/ref.json --k 5",
        ))
        .unwrap();
        assert_eq!(a.label, "abc123");
        assert_eq!(a.judge, Some(PathBuf::from("records/ref.json")));
        assert_eq!(a.k, 5);
        // The defaults still hold for everything not named.
        assert_eq!(a.config, PathBuf::from(DEFAULT_CONFIG));
        assert_eq!(a.out, None);
    }

    #[test]
    fn a_run_still_needs_something_to_ask_and_someone_to_ask() {
        assert!(run_args(&args("--corpus f.json --label x")).is_err());
        assert!(run_args(&args("--candidate c --label x")).is_err());
        assert!(run_args(&args("--candidate c --corpus f.json --label x --k 0")).is_err());
        assert!(run_args(&args("--candidate c --corpus f.json --label x --nope 1")).is_err());
    }

    #[test]
    fn tail_reads_its_three_flags_and_refuses_the_rest() {
        // tail_args n avait AUCUN test : sept mutations y survivaient.
        let a = tail_args(&args("--match rag=true --match k=v --show a,b --last 5")).unwrap();
        assert_eq!(a.filters.len(), 2);
        assert_eq!(a.filters[0], ("rag".into(), "true".into()));
        assert_eq!(a.filters[1], ("k".into(), "v".into()));
        assert_eq!(a.columns, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(a.last, 5);

        // With no flag, the defaults hold.
        let d = tail_args(&[]).unwrap();
        assert_eq!(d.last, DEFAULT_LAST);
        assert!(d.filters.is_empty() && d.columns.is_empty());
    }

    #[test]
    fn tail_says_what_is_wrong_rather_than_guessing() {
        // A missing value must not be invented.
        assert!(tail_args(&args("--last")).is_err());
        assert!(tail_args(&args("--match")).is_err());
        // A --match with no equals sign is not a filter.
        let e = tail_args(&args("--match rag")).unwrap_err();
        assert!(e.contains("key=value"), "got: {e}");
        // A number that is not one.
        assert!(tail_args(&args("--last plenty")).is_err());
        // An unknown flag stops here, it is not skipped in silence.
        let e = tail_args(&args("--nope 1")).unwrap_err();
        assert!(e.contains("--nope"), "got: {e}");
    }

    #[test]
    fn the_columns_are_trimmed_because_people_write_spaces() {
        // A shell passes this as ONE argument: it cannot be split on spaces
        // the way the help above does.
        let a = tail_args(&["--show".to_string(), "a, b ,c".to_string()]).unwrap();
        assert_eq!(
            a.columns,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }
}
