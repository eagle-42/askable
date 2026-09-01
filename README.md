# askable

A regression judge for ranked search.

You change an embedding model, a chunk size, a reranker. Recall drops four
points. Nobody notices until someone complains, weeks later. askable replays a
frozen set of questions against your system, records case by case where the
right answer landed, and compares two such records to say whether to worry.

It is not a dashboard. Several good terminal viewers for OpenTelemetry already
exist; this is the other half, the one that renders a verdict.

## Use

```
askable run   --candidate NAME --corpus FILE --label TEXT
              [--judge REFERENCE.json] [--config FILE] [--k N] [--out FILE]
askable judge REFERENCE.json CANDIDATE.json [--config FILE]
askable tail  [--match k=v]... [--show a,b,c] [--last N]
```

Exit code `0` when there is nothing to report, `1` when the corpus can see a
regression, `2` when the question could not be asked. A pipeline only ever reads
that number.

### Replay

```
askable run --candidate mine --corpus corpus/example.json --label v1.2.0
```

```
candidate  mine
corpus     example (3 cases, 8f2a1c04e7b93d55)
recall@1   0.6667
recall@5   1.0000
recall@10  1.0000
mrr        0.8333
```

A record lands in `records/`: one object per case with the question, the answers
that count as right, the rank the right one reached, every id that came back,
and the numeric fields the top result carried.

`--label` is required. It says WHICH version this measures — a commit, a model
name, a row count. askable cannot discover it, and a record nobody can attribute
to a version is an anecdote.

### Judge

```
askable judge records/reference.json records/candidate.json
```

```
cutoff  reference  candidate  regressed  improved      p  verdict
     1     0.6984     0.5873          7         0  0.016  REGRESSION
     5     0.9524     0.9524          0         0  1.000  ok

cases lost at rank 1:
  where do backups live
  ...

FAIL - this corpus can see this regression
```

The comparison is refused rather than approximated when the two records are not
of the same corpus, were not asked for the same number of results, or do not
hold the same cases in the same order. A number that looks like a comparison and
is not one is worse than no number.

### In continuous integration

Replay and judge in one gesture, and let the exit code decide:

```
askable run --candidate mine --corpus corpus/bench.json \
    --label "$(git rev-parse --short HEAD)" \
    --judge records/reference.json
```

The record is written to disk *before* the comparison runs: a failing verdict
must not throw away the measurement that proves it.

### Read the traces

`askable tail` is where the questions people actually ask show up. It prints
structured `tracing` logs as a table:

```
journalctl -u some.service -o json | askable tail --match rag=true \
    --show query,ms_total --last 20
```

## Configure

Copy `askable.example.toml` to `askable.toml`:

```toml
[candidate.mine]
url = "http://localhost:8080/search?q={query}&k={k}"

[judge]
cutoffs = [1, 5]
alpha = 0.05
```

Any service that answers a GET with a ranked list of identifiers is a candidate.
`{query}` is replaced by the question, percent-encoded; `{k}` by how many
results to ask for. Two optional keys cover the rest: `id_field` when the
identifier is not called `id`, `results_at` when the array is nested.

The judge's strictness lives in the same file so that a FAIL can be argued with
in a diff rather than in a shell history. An optional `[[judge.floor]]` adds an
absolute recall that must hold whatever the paired test says.

## Write a corpus

```json
{
  "name": "example",
  "cases": [
    {"question": "how do I roll back the last deployment",
     "gold": ["doc-deploy-rollback"]}
  ]
}
```

`gold` lists *every* answer that counts as right — a question with one obvious
answer and three acceptable ones is the normal case. A gold entry matches by
prefix and must be at least four characters: shorter, it matches identifiers it
does not mean and scores a wrong answer as right.

Write the questions the way someone would actually type them. Use the words they
would use, including the document's own domain terms — what you must not do is
lift a *phrase*. Paraphrasing a term nobody would avoid measures your synonym
skills, not the retrieval.

## What a passing run means, and what it does not

askable judges one system against one corpus, at one moment. It says: *no
regression detectable on these cases*. It does not say your search is good, or
good for anyone else.

The resolution is finite and worth knowing. Comparing two recall *rates* is
weak: on 63 cases, a 70% recall carries a 95% confidence interval 22 points
wide, and two independent rates must differ by 16 points before the difference
means anything. Comparing the **same cases one by one** is far sharper — six
cases regressing with none improving is already significant (exact McNemar,
p = 0.031).

That floor of six cases does **not** depend on the size of the corpus. What a
bigger corpus buys is a smaller floor *in points*: six cases out of 63 is 9.5
points, out of 143 it is 4.2. Widening detects a smaller proportional effect,
never a smaller absolute one.

So a change smaller than about six discordant cases is invisible here.
Structural changes — a different model, a different chunking — move far more
than that. Fine-tuning a parameter usually does not, and askable will say
nothing about it. That is a limit, not a defect. When the expected effect is
small, read the list of named cases rather than only the exit code.

## Refreshing the corpus

A corpus is frozen *between two refreshes*, not forever. Usage drifts; some
questions stop mattering and new ones start.

Two rules make refreshes safe:

1. **Never refresh the corpus and change the system in the same step.** Refresh,
   re-measure on the unchanged system, *then* change it. Otherwise you are
   comparing two things at once and can attribute neither.
2. **A refresh retires the previous reference.** Recall over 63 cases is not
   comparable to recall over 143. Every record carries the corpus fingerprint
   next to the candidate, so a stale comparison is visible rather than silent.

The fingerprint follows the questions and the expected answers, not the layout:
reformatting the file, or adding a comment to it, leaves it unchanged.

## Build

```
cargo build --release
```

Rust 2024 edition. Four dependencies: `serde`, `serde_json`, `toml`, `ureq`. No
statistics crate — the exact McNemar bound is a binomial tail, and a binomial
tail is a loop. No date crate, no argument parser: one timestamp and six flags
do not earn one.

## Gotchas

- **Keep your real corpus out of the repository.** The questions alone are a
  table of contents of whatever you are searching. `askable.toml` and
  `*.private.json` are git-ignored for that reason.
- **A failed case aborts the whole run and writes nothing.** A record missing a
  third of its cases still computes an average, and that average looks exactly
  like a measurement.
- **Absent is not zero.** A case whose answer never came back has a `null` rank,
  never a sentinel. It contributes nothing to the MRR rather than a tiny amount.
  A recall at a cutoff beyond the `k` you asked for is `null` too, not a ceiling
  dressed up as a measurement.
- **The replay is sequential.** Rerankers usually sit behind a single model
  instance, so parallel requests serialise anyway — and a run that is not
  reproducible is not a measurement.
- For `askable tail`, the emitting service must run with `.with_ansi(false)`.
  Otherwise `journalctl -o json` returns a message full of escape sequences and
  nothing parses, while `journalctl -o cat` still looks perfectly healthy.
