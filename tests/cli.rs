//! What the binary does, exercised through the binary.
//!
//! Everything else in this repository is tested one function at a time. These
//! run `askable` as a process, because the defects that reached users were all
//! at that seam: an exit code, a refusal, a file written or not written. A
//! library test cannot see any of them.
//!
//! No dependency and no network: the one test that needs a service starts a
//! four-line server on a loopback port and stops it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};

const ASKABLE: &str = env!("CARGO_BIN_EXE_askable");

fn run(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(ASKABLE)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary must be runnable")
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("no signal expected")
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// A scratch directory that removes itself, so a failing test leaves nothing.
struct Dir(std::path::PathBuf);

impl Dir {
    fn make(name: &str) -> Dir {
        let p = std::env::temp_dir().join(format!(
            "askable-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
    fn write(&self, name: &str, content: &str) -> std::path::PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A record with one outcome per rank given, all of the same corpus.
fn record(hash: &str, k: usize, ranks: &[Option<usize>]) -> String {
    let cases: Vec<String> = ranks
        .iter()
        .enumerate()
        .map(|(i, r)| {
            format!(
                r#"{{"question":"q{i}","gold":["aaaa"],"rank":{},"returned":[],"top_numbers":{{}}}}"#,
                r.map(|v| v.to_string()).unwrap_or_else(|| "null".into())
            )
        })
        .collect();
    format!(
        r#"{{"meta":{{"produced_at":"2026-01-01T00:00:00Z","candidate":"c","candidate_version":"v",
"url":"http://x/{{query}}","corpus":"t","cases":{},"corpus_fingerprint":"{hash}","k":{k},
"recall_at_1":null,"recall_at_5":null,"recall_at_10":null,"mrr":0.0,"seconds":0}},
"outcomes":[{}]}}"#,
        ranks.len(),
        cases.join(",")
    )
}

#[test]
fn no_argument_prints_the_usage_and_refuses() {
    let dir = Dir::make("usage");
    let o = run(&dir.0, &[]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("askable run"), "got: {}", err(&o));
}

#[test]
fn a_run_without_a_label_is_refused_before_anything_is_touched() {
    let dir = Dir::make("label");
    let o = run(
        &dir.0,
        &["run", "--candidate", "x", "--corpus", "nope.json"],
    );
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("--label"), "got: {}", err(&o));
    // The corpus does not even exist: the missing argument shows BEFORE the
    // disk access, so the message is about the label, not the file.
    assert!(!err(&o).contains("nope.json"), "got: {}", err(&o));
}

#[test]
fn two_identical_records_pass_and_leave_zero() {
    let dir = Dir::make("pass");
    let a = dir.write("a.json", &record("ffff", 10, &[Some(1), Some(1), Some(2)]));
    let b = dir.write("b.json", &record("ffff", 10, &[Some(1), Some(1), Some(2)]));
    let o = run(&dir.0, &["judge", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "stderr: {}", err(&o));
    assert!(out(&o).contains("PASS"), "got: {}", out(&o));
}

#[test]
fn a_regression_the_corpus_can_see_leaves_one() {
    let dir = Dir::make("fail");
    // Seven cases lost at rank 1, none gained: above the floor of six.
    let before = record("ffff", 10, &[Some(1); 10]);
    let mut ranks = [Some(1); 10];
    for r in ranks.iter_mut().take(7) {
        *r = Some(4);
    }
    let after = record("ffff", 10, &ranks);
    let a = dir.write("a.json", &before);
    let b = dir.write("b.json", &after);
    let o = run(&dir.0, &["judge", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert_eq!(code(&o), 1, "stdout: {} stderr: {}", out(&o), err(&o));
    assert!(out(&o).contains("REGRESSION"), "got: {}", out(&o));
    // The verdict NAMES the lost cases: that is the actionable half.
    assert!(out(&o).contains("q0"), "got: {}", out(&o));
}

#[test]
fn a_refreshed_corpus_is_refused_rather_than_approximated() {
    let dir = Dir::make("refused");
    let a = dir.write("a.json", &record("aaaa", 10, &[Some(1)]));
    let b = dir.write("b.json", &record("bbbb", 10, &[Some(1)]));
    let o = run(&dir.0, &["judge", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("same corpus"), "got: {}", err(&o));

    // Same corpus, but not the same number of results was asked for.
    let c = dir.write("c.json", &record("aaaa", 5, &[Some(1)]));
    let o = run(&dir.0, &["judge", a.to_str().unwrap(), c.to_str().unwrap()]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("results"), "got: {}", err(&o));
}

#[test]
fn an_unreadable_record_is_a_mistake_not_a_verdict() {
    let dir = Dir::make("unreadable");
    let a = dir.write("a.json", &record("ffff", 10, &[Some(1)]));
    let b = dir.write("b.json", "{ this is not a record }");
    let o = run(&dir.0, &["judge", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("not a record"), "got: {}", err(&o));
}

/// Serves one fixed answer to the first `how_many` requests, then stops.
fn fake_service(how_many: usize, body: &'static str) -> (u16, std::thread::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let h = std::thread::spawn(move || {
        for _ in 0..how_many {
            let Ok((mut stream, _)) = l.accept() else {
                return;
            };
            let mut buffer = [0u8; 2048];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (port, h)
}

#[test]
fn a_whole_replay_goes_through_the_network_and_writes_a_record() {
    let dir = Dir::make("replay");
    let body = r#"{"results":[{"id":"aaaa1111","score":0.9},{"id":"bbbb2222"}]}"#;
    let (port, h) = fake_service(2, body);

    dir.write(
        "askable.toml",
        &format!(
            r#"[candidate.fake]
url = "http://127.0.0.1:{port}/s?q={{query}}&k={{k}}"
results_at = "results"
"#
        ),
    );
    dir.write(
        "corpus.json",
        r#"{"name":"t","cases":[
 {"question":"the first","gold":["aaaa"]},
 {"question":"the second","gold":["zzzz"]}]}"#,
    );

    let o = run(
        &dir.0,
        &[
            "run",
            "--candidate",
            "fake",
            "--corpus",
            "corpus.json",
            "--label",
            "trial",
            "--out",
            "r.json",
        ],
    );
    let _ = h.join();
    assert_eq!(code(&o), 0, "stderr: {}", err(&o));

    let raw = std::fs::read_to_string(dir.0.join("r.json")).expect("a record must exist");
    // The first case finds its answer at rank 1, the second does not.
    assert!(raw.contains(r#""rank": 1"#), "got: {raw}");
    assert!(
        raw.contains(r#""rank": null"#),
        "absent must stay null, not 0"
    );
    // And the record carries the identity of what was measured.
    assert!(
        raw.contains(r#""candidate_version": "trial""#),
        "got: {raw}"
    );
    assert!(raw.contains(r#""cases": 2"#), "got: {raw}");
}

#[test]
fn a_service_that_refuses_aborts_the_run_and_writes_nothing() {
    let dir = Dir::make("outage");
    // A closed port: the first request fails.
    dir.write(
        "askable.toml",
        "[candidate.dead]\nurl = \"http://127.0.0.1:1/s?q={query}\"\n",
    );
    dir.write(
        "corpus.json",
        r#"{"name":"t","cases":[{"question":"only one","gold":["aaaa"]}]}"#,
    );
    let o = run(
        &dir.0,
        &[
            "run",
            "--candidate",
            "dead",
            "--corpus",
            "corpus.json",
            "--label",
            "trial",
            "--out",
            "r.json",
        ],
    );
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("case 1/1"), "got: {}", err(&o));
    // A partial record would be worse than no record at all.
    assert!(!dir.0.join("r.json").exists(), "nothing must be written");
}
