# 0029. Broken-pipe exit semantics

- Status: accepted
- Date: 2026-07-11
- Deciders: @P4suta
- Tags: cli, exit-codes, robustness

## Context

`aozora render big.txt | head` — a downstream reader that closes the pipe
after the first line — left the CLI reporting an error. Measured behavior:

- `aozora render big.txt | head -1` → exit **1**, stderr
  `aozora: failed to write to stdout: Broken pipe (os error 32)`
- `aozora fmt big.txt | head -1` → exit **2**, stderr
  `aozora fmt: Broken pipe (os error 32)`

This is wrong: piping into `head`, `less` (quit early), or any short-circuiting
consumer is routine, correct usage. A tool that returns non-zero and prints an
error for it is noisy in scripts and breaks `set -euo pipefail` pipelines that
are otherwise succeeding.

The mechanism: Rust's runtime resets `SIGPIPE` to `SIG_IGN` at startup, so a
write to a pipe whose reader has gone does **not** kill the process with signal
13 — it returns `io::ErrorKind::BrokenPipe` from the write. The CLI's stdout
writes (`write_all(..).context("…")?`) then propagate that up as an ordinary
`anyhow` error and the top-level `Err` arm treats it like any failure. The
formatter's `--list` / `--write` paths were worse: they used `println!`, which
**panics** on a broken-pipe write (`failed printing to stdout`).

## Decision

Treat a broken output pipe as a normal, silent success (exit **0**) in both
frontends — the `aozora` CLI and the standalone `aozora-fmt` — matching
`ripgrep` and `bat`.

- Add `aozora_fmt::is_broken_pipe(&anyhow::Error)`: walk `err.chain()` and
  return true if any link downcasts to an `io::Error` with
  `kind() == BrokenPipe`. The io::Error may be wrapped in `anyhow` context, so
  the whole chain is searched.
- `aozora-cli`'s `main` `Err` arm and `aozora-fmt`'s `run_engine` `Err` arm
  short-circuit on `is_broken_pipe` to `ExitCode::SUCCESS`, printing nothing.
- Convert the formatter's `println!` output paths (`--list`, `--write --list`,
  stdin `--list`) to `writeln!(io::stdout(), …)?` so a broken pipe becomes a
  propagated `Err` instead of a panic. The formatter's `fold_files` fold — which
  otherwise localises a per-file error so one bad file doesn't abort the batch —
  makes a single exception for a broken pipe and propagates it, since it is
  terminal for the whole run (every later stdout write would fail too).

## Consequences

- The documented exit-code contract (`0` success, `1` `--strict`/`--check`
  mismatch, `2` usage error, `3` internal diagnostic) is unchanged. A broken
  stdout pipe now folds into `0`.
- No `unsafe` and no signal-disposition change: the fix is pure error
  classification, honouring the crate-wide `#![forbid(unsafe_code)]` posture.
- Behavior is symmetric across `render` / `inspect` / `pandoc` (CLI stdout
  writes) and every `fmt` output mode (default stdout, `--list`, `--write`,
  `--diff`), so pipelines never see a mode-dependent broken-pipe outcome.

## Alternatives considered

- **Reset `SIGPIPE` to `SIG_DFL` at startup (exit 141).** The classic C idiom:
  let the OS kill the process with signal 13 on a broken pipe, which shells
  report as exit `128 + 13 = 141`. Rejected: it requires an `unsafe` libc call,
  violating `#![forbid(unsafe_code)]`; `141` is outside the tool's `0/1/2/3`
  exit contract; and a signal death is harder to reason about than a clean
  `0` for what is a successful, complete-as-far-as-the-reader-wanted run.
- **Exit `141` without the signal (map broken pipe → `ExitCode::from(141)`).**
  Rejected: it invents a fourth non-zero code purely to mimic the signal
  convention, still reads as failure to `set -e`, and gives scripts nothing a
  plain `0` doesn't.
- **Leave it as-is.** Rejected: a routine, correct pipeline should not print an
  error or return non-zero, and the `println!` panic path is a latent crash.

## References

- `ripgrep` exits 0 on a broken pipe rather than erroring
  (<https://github.com/BurntSushi/ripgrep>), motivated by
  <https://github.com/BurntSushi/ripgrep/issues/200>.
- `bat` swallows broken-pipe errors on stdout and exits 0:
  <https://github.com/sharkdp/bat/blob/master/src/bin/bat/main.rs>.
- Rust resets `SIGPIPE` to `SIG_IGN` so writes return `BrokenPipe` instead of
  killing the process: <https://github.com/rust-lang/rust/issues/62569>.
- Plan: `.claude/plans/core-calm-gray.md` (Phase 3 PR-2, finding F5).
- Evidence: `crates/aozora-cli/src/main.rs`, `crates/aozora-fmt/src/lib.rs`,
  `crates/aozora-cli/tests/smoke.rs` (the EPIPE tests).
