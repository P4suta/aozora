# AGENTS.md

Contracts an agent cannot derive from the tree. Everything else is in
`just --list`, `aozora --help`, and the code.

## Comments

Default to none — the code is the source of truth, and a comment that
restates it is drift waiting to happen (#553 shipped a table whose comment
named a key it did not use, and nobody was checking). Write only the *why*
the code cannot show.

Never write a coordinate (`file.rs:417`), a hand-counted total ("14 kinds"),
or a copy of code that lives elsewhere: each rots the moment its referent
moves. Name a symbol instead — the compiler keeps that honest — and `xtask
lint coordinates` rejects the coordinate form.

State an invariant as a check — an `assert!`, a type, a test, a gate — never
as prose telling the next reader to uphold it. Prose is never re-read against
the code.

So a comment earns its place only as one of: a doc `missing_docs` requires
(trimmed to what a reader needs), a load-bearing *why* that is underivable
(cite the ADR or the corpus measurement), or something the toolchain demands
(`// SAFETY:`, a `mutants::skip` reason, an `#[allow(reason = …)]`).

## Running anything

The locked native mise environment is canonical. Run `./bootstrap` on a
fresh clone, then use `just`; CI invokes the same fixed `just ci-*` suites.
Do not install ad-hoc tool versions around `mise.toml` and `mise.lock`.
`just ci` is the local pre-push gate.

Commits must be signed and Conventional. Do not reach for `LEFTHOOK=0`
or `--no-verify`.

## The `aozora` CLI

Deterministic, stdin→stdout, stable exit codes. Output carries no
timestamps or randomness, so diffs are clean. Every document subcommand
takes `-` for stdin.

| code | meaning |
|---|---|
| `0` | parsed; diagnostics may have printed but were tolerated |
| `1` | `--strict` and at least one diagnostic |
| `2` | usage error — bad flag, unreadable file, decode failure |
| `3` | an `Internal` diagnostic fired: a bug in aozora, not bad input |

`aozora render FILE | head` exits `0`. A closed pipe is success, not
failure (ADR-0029), so it never masquerades as a `1`/`2`.

**`--format` defaults to `human` on a TTY and `json` when piped**, so a
piped `aozora check` already yields machine output. Pass `--format json`
(or `short`) explicitly anyway, and **never parse the `human` render** —
it is width- and colour-dependent, the one non-deterministic surface.

## Where decisions live

`docs/adr/` — read the ADR governing an area before changing it. Once
accepted, an ADR is never edited; a later one supersedes it.

`docs/contrib/` — development loop, testing strategy, release runbooks.
