# AGENTS.md

Contracts an agent cannot derive from the tree. Everything else is in
`just --list`, `aozora --help`, and the code.

## Running anything

Docker only. `just` wraps `docker compose run`; never call `cargo` /
`wasm-pack` on the host. `./bootstrap` on a fresh clone. `just
ci-parallel` is the full gate and runs on every push — it prints
`::error title=<gate>::` naming whichever failed.

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
