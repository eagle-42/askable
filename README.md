# askable

Prints structured `tracing` logs as a table, in a terminal.

Any Rust service using [`tracing`](https://docs.rs/tracing) writes lines like:

```text
2026-09-10T14:50:14.110057Z  INFO svc: search rag=true query="two words" ms_total=4214
```

Readable enough one at a time, useless by the hundred. askable turns a stream of
them into columns you choose.

## Use

```
journalctl -u some.service -o json | askable tail --match rag=true \
    --show query,ms_total --last 20
```

- `--match k=v` keeps only events carrying that field. Repeat it to narrow.
- `--show a,b,c` picks the columns. Omit it and every field found is shown —
  useful the first time, when you do not yet know what a service emits.
- `--last N` keeps the last N. Defaults to 20.

Both `-o json` and `-o cat` are accepted: the first is what a pipeline produces,
the second is what you type by hand.

## Build

```
cargo build --release
```

## Gotchas

- **Quote any value containing a space when you emit it.** In `tracing` that
  means `field = ?value` (Debug) rather than `%value` (Display). Unquoted, a
  multi-word value runs into the next key and no parser can recover it.
- **Turn ANSI off in the emitting service** (`.with_ansi(false)`). Under systemd
  there is no terminal, but colours are still emitted by default, and then
  `journalctl -o json` returns a message full of escape sequences while
  `-o cat` still looks perfectly healthy.
- Read-only. askable opens a pipe and prints; it never writes anywhere.

## Layout

- `src/lib.rs` — the log format: parsing, filtering, nothing else. Testable
  without a terminal.
- `src/main.rs` — the command line and the rendering.
