# askable

A regression judge for ranked search.

You change an embedding model, a chunk size, a reranker. Recall drops four
points. Nobody notices until someone complains, weeks later. askable replays a
frozen set of questions against your system and writes down, case by case, where
the right answer landed — so the next change has something to be compared to.

It is not a dashboard. Four good terminal viewers for OpenTelemetry already
exist; this is the other half, the one that renders a verdict.

## Use

```
askable run --candidate mine --corpus corpus/example.json --label v1.2.0
```

`--label` is required. It says WHICH version this measures — a commit, a model
name, a row count. askable cannot discover it, and a record nobody can attribute
to a version is an anecdote.

```
candidate  mine
corpus     example (3 cases, 8f2a1c04e7b93d55)
recall@1   0.6667
recall@5   1.0000
recall@10  1.0000
mrr        0.8333
```

A record lands in `records/`, one object per case: the question, the answers
that count as right, the rank the right one reached, every id that came back,
and the numeric fields the top result carried.

`askable tail` is the other half of the tool: it prints structured `tracing`
logs as a table, which is where the questions people actually ask show up.

```
journalctl -u some.service -o json | askable tail --match rag=true \
    --show query,ms_total --last 20
```

## Configure

Copy `askable.example.toml` to `askable.toml`:

```toml
[candidate.mine]
url = "http://localhost:8080/search?q={query}&k={k}"
```

Any service that answers a GET with a ranked list of identifiers is a candidate.
`{query}` is replaced by the question, percent-encoded; `{k}` by how many
results to ask for. Two optional keys cover the rest: `id_field` when the
identifier is not called `id`, `results_at` when the array is nested.

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
prefix, so you can write the first eight characters of an identifier rather than
all thirty-six.

Write the questions **without** reusing the vocabulary of the documents they
should find. Otherwise you are measuring lexical overlap, not retrieval.

## What a passing run means, and what it does not

askable judges one system against one corpus, at one moment. It says: *no
regression detectable on these cases*. It does not say your search is good, or
good for anyone else.

The resolution is finite and worth knowing. Comparing two recall *rates* is
weak: on 63 cases, a 70% recall carries a 95% confidence interval 22 points
wide, and two independent rates must differ by 16 points before the difference
means anything. Comparing the **same cases one by one** is far sharper — six
cases regressing with none improving is already significant (exact McNemar,
p = 0.031). That floor of six does not depend on the size of the corpus.

So: a change smaller than about six discordant cases is invisible here,
whatever the corpus size. Structural changes — a different model, a different
chunking — move far more than that. Fine-tuning a parameter usually does not,
and askable will say nothing about it. That is a limit, not a defect.

## Refreshing the corpus

A corpus is frozen *between two refreshes*, not forever. Usage drifts; some
questions stop mattering and new ones start.

Two rules make refreshes safe:

1. **Never refresh the corpus and change the system in the same step.** Refresh,
   re-measure on the unchanged system, *then* change it. Otherwise you are
   comparing two things at once and can attribute neither.
2. **A refresh retires the previous reference.** Recall over 63 cases is not
   comparable to recall over 71. Every record carries the corpus fingerprint
   next to the candidate, so a stale comparison is visible rather than silent.

## Build

```
cargo build --release
```

Rust 2024 edition. Four dependencies: `serde`, `serde_json`, `toml`, `ureq`.

## Gotchas

- **Keep your real corpus out of the repository.** The questions alone are a
  table of contents of whatever you are searching. `askable.toml` and
  `*.private.json` are git-ignored for that reason.
- **A failed case aborts the whole run and writes nothing.** A record missing a
  third of its cases still computes an average, and that average looks exactly
  like a measurement.
- **Absent is not zero.** A case whose answer never came back has a `null` rank,
  never a sentinel. It contributes nothing to the MRR rather than contributing
  a tiny amount.
- **The replay is sequential.** Rerankers are usually behind a single model
  instance, so parallel requests serialise anyway — and a run that is not
  reproducible is not a measurement.
- For `askable tail`, the emitting service must run with `.with_ansi(false)`.
  Otherwise `journalctl -o json` returns a message full of escape sequences and
  nothing parses, while `journalctl -o cat` still looks perfectly healthy.
