# 0014. CLI watch mode and the `notify` dependency

- Status: accepted
- Date: 2026-06-21
- Deciders: @P4suta
- Tags: cli, devex, dependencies

## Context

`aozora <cmd> --watch FILE` re-runs a document subcommand whenever the
input changes — the tight inner loop for writing and proofreading. That
needs file-system change notification, which means either a new
dependency or a hand-rolled poll loop. The workspace is deliberately
dependency-averse — `cargo-deny`, `cargo-udeps`, and a license allow-list
gate every addition — so a new crate has to earn its place.

## Decision

Add **`notify`** (the de-facto Rust file-watching crate) and build a
foreground, non-daemon watch loop on top of it.

- **`notify` over a hand-rolled poll loop.** Polling wastes CPU when idle,
  adds latency when coarse, and still has to special-case atomic saves.
  `notify` wraps each platform's native mechanism (inotify / FSEvents /
  ReadDirectoryChangesW) — lower latency, no busy-wait. It is MIT-licensed
  with an MIT/Apache dependency tail (clean against `deny.toml`), and it
  is genuinely used, so `cargo-udeps` stays green. The single new direct
  dependency is justified by real, recurring user value.
- **Foreground, non-daemon.** The loop runs in the invoking process and
  ends on Ctrl-C. No background process, no PID files, no daemon
  lifecycle to manage — consistent with a single self-contained binary.
- **Watch the parent directory, filter by file name.** Editors save
  atomically by writing a temp file and renaming it over the target; a
  direct file watch misses the rename. Watching the parent and matching
  on file name catches both in-place writes and atomic saves.
- **Swallow per-run exit codes.** A diagnostic, or an `fmt --check`
  mismatch, prints and the loop keeps going — stopping on the first
  non-zero would defeat the purpose. The watch process itself exits 0
  (on Ctrl-C). Per-run failures are visible in the output, not the code.
- **Reject stdin.** `--watch -` is a usage error (exit 2): a pipe is not a
  watchable file.
- **Ctrl-C ends it.** Default SIGINT termination; no signal handler and
  thus no `ctrlc` dependency. The `notify` recv loop is interrupted and
  the process exits.

## Consequences

- **One new direct dependency.** `notify` (plus its platform-native
  tail). Isolated to `aozora-cli`; library consumers and the WASM /
  Python / FFI builds do not pull it in.
- **A fast inner loop for authors.** Edit-on-save re-renders or re-checks
  without re-invoking the binary by hand. `--watch --timing` reprints
  phase timings each iteration; `fmt --write --watch` is a format-on-save
  loop (its own write-back is coalesced by the debounce, so it does not
  spin).
- **stdout stays clean.** The re-run banner is stderr-only and TTY-gated,
  so `render --watch > out.html` keeps writing valid HTML.
- **A new lint/license surface.** `cargo-deny` and `cargo-udeps` now cover
  `notify`'s tail; that is the intended, contained cost of the feature.

## Alternatives considered

**A hand-rolled `stat`-polling loop.** No new dependency, but it busy-waits
or lags, and reimplementing reliable atomic-save detection is exactly what
`notify` already does across platforms. Rejected.

**`notify-debouncer-mini` / `notify-debouncer-full`.** They provide
debouncing for free, but at the cost of *another* crate. A ~75 ms manual
debounce (drain the channel within a window) is a few lines and keeps the
dependency tail minimal. Rejected in favour of plain `notify`.

**Shelling out to `watchexec` / `entr`.** Pushes the feature onto an
external tool the user must install separately, and loses the in-process
config / timing integration. Rejected.

**A long-running daemon / LSP-style server.** Out of scope and against the
single-binary model (see ADR-0008's rejection of an embedded server).

## References

- `crates/aozora-cli/src/watch.rs` — the watch loop, parent-dir watch,
  debounce, and name filter.
- `crates/aozora-cli/src/main.rs` — `run_watched` (stdin rejection, the
  `--watch` branch wrapping each `run_*_once`).
- ADR-0013 — the sibling CLI-DevEx config-file decision.
- Plan: `cli-devex-twinkling-mountain.md`.
