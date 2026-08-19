# askable

Reads the service's search traces and makes them readable in a terminal.

It is a private knowledge service: a Rust binary that stores notes in a graph
database and searches them with a hybrid retriever (dense + lexical) followed by
a cross-encoder reranker.

The service emits one structured `tracing` line per search — the query, the note it
found, four separate scores, and the time spent in each stage. Those lines go to
journald, where nobody reads them. askable is the missing view.

## Use

Stage 1, working today:

```
journalctl --user -u some.service -o json | askable tail --last 50
```

Stage 2, next: `askable ui` — a full-terminal cockpit.

## Build

Wherever the Rust toolchain lives:

```
cargo build --release
```

## Gotchas

- Only lines carrying `rag=true` are searches. Everything else in the journal is
  ignored.
- the service must run with `.with_ansi(false)`. Otherwise `journalctl -o json`
  returns a MESSAGE full of escape sequences and nothing parses — while
  `journalctl -o cat` still looks perfectly healthy.
- The query field is quoted on purpose. Unquoted, a multi-word query ran into
  the next field and no parser could tell where it ended.
- Read-only, always. askable never writes to the service.
