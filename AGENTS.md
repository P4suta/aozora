# AGENTS.md — machine contract for automation

This is the terse, machine-facing contract for agents and CI driving the
aozora repo. The prose guide for humans — the architecture and the
workspace map — lives in the handbook under `crates/aozora-book/`
(published at <https://p4suta.github.io/aozora/>); start with the
Architecture chapters [Pipeline overview](crates/aozora-book/src/arch/pipeline.md)
and [Crate map](crates/aozora-book/src/arch/crates.md) — read those first;
this file only pins the commands, outputs, and exit codes you can rely on.

## Execution surface

- **Docker only.** Every dev task runs through `just`, which wraps
  `docker compose run …`. Never invoke `cargo` / `wasm-pack` / `mdbook`
  on the host. `just` itself runs on the host (it shells into containers).
- First run on a fresh clone: `./bootstrap` (or `just setup`) — checks
  prerequisites, builds the dev image, installs hooks, runs the tests.
- `just --list` is the authoritative, self-documenting task index.

## Gates — what to run and how to read the result

Each gate exits `0` on success, non-zero on failure. Run a single gate
for a clean repro; run the whole suite before a push.

| Goal | Command | Notes |
|---|---|---|
| Does it compile? (fastest) | `just check` | `cargo check --all-targets`, sub-second warm |
| Run all tests | `just test` | nextest; `just t <FILTER>` for one test |
| Run one test / pattern | `just t <FILTER>` | nextest filterset `test(<FILTER>)` |
| Lint | `just clippy` | light; `just lint-full` for the `--all-targets` surface |
| Format check | `just fmt-check` | `just fmt` to auto-fix |
| **Full pre-push suite (fast)** | `just ci-parallel` | every gate, parallelised; the lefthook pre-push gate |
| Full pre-push suite (sequential) | `just ci` | same gates, serial — easier to read on failure |
| Wire/type drift | `just drift-gate` | regenerate with `just schema` / `just types` / `just types-langs` |
| Conformance | `just conformance` | byte-identical render gate |
| Docs build + links | `just book-build` then `just book-linkcheck` | |

`ci-parallel` prints `::error title=<gate>::` naming the gate that
failed. A foreground gate aborts immediately; background gates
(deny / audit / book-linkcheck / smoke-ffi / playground-* / fmt-check /
typos / strict-code) are reaped at the end with their captured log.
`SKIP_TAGS=deep` opts out of the 4096-case property sweep.

Pushing requires **signed commits** (the pre-push `signing-check` runs
first and is non-negotiable; do not use `LEFTHOOK=0`). Conventional
Commits are enforced by `commit-msg`.

## The `aozora` CLI — machine interface

Deterministic, stdin→stdout, stable exit codes. Prefer JSON over the
human renderer in automation.

| Want | Command | Output |
|---|---|---|
| Diagnostics as data | `aozora check --format json FILE` | `{"schemaVersion":N,"data":[…]}` on stderr; same envelope every binding emits |
| Diagnostics, one line each | `aozora check --format short FILE` | `path:offset: severity[code]: message` |
| HTML | `aozora render FILE` | semantic HTML5 on stdout |
| Pandoc AST (→ any format) | `aozora pandoc FILE` | Pandoc JSON on stdout |
| Wire JSON Schema | `aozora schema {diagnostics\|nodes\|pairs\|container-pairs}` | JSON Schema |
| Enum/wire-tag tables | `aozora kinds` | tables when stdout is a terminal, else the machine envelope `{"schemaVersion":1,"data":{nodeKinds,pairKinds,…}}` (force with `--format {human,json}`) — the typed contract behind the wire format |

`--format` defaults to `human` on a TTY and `json` when piped,
so a piped `aozora check` already yields machine output without a flag.
**Agents should pass `--format json` (or `short`) explicitly**
and never parse the `human` graphical render (it is width/colour
dependent — the only non-deterministic output surface).

### `aozora check` exit-code contract

| Code | Meaning |
|---|---|
| `0` | Parse succeeded; diagnostics may have printed but were tolerated. |
| `1` | `--strict` and at least one diagnostic. |
| `2` | Usage error (bad flag, unreadable file, decode failure). |
| `3` | An `Internal`-source diagnostic fired — a library bug, distinct from bad input. |

A reader that closes stdout early — `aozora render FILE | head` — is a normal
success: the broken pipe is swallowed and the command exits `0` with no stderr
(ADR-0029), so it never masquerades as a `1`/`2` failure in a pipeline.

Encoding is auto-detected (UTF-8 → else Shift_JIS); force with
`-E {utf8,sjis}`. Every document subcommand accepts `-` for stdin.

## Determinism

HTML, `serialize`, the wire JSON envelopes, `schema`, and `kinds` carry
no timestamps or randomness — output is stable across runs, so diffs are
clean. (`ci` / `ci-parallel` write transient logs under `/tmp` and clean
them up; that is not output.)

## See also

- [Pipeline overview](crates/aozora-book/src/arch/pipeline.md) and
  [Crate map](crates/aozora-book/src/arch/crates.md) — architecture and
  the workspace crate map (handbook, `crates/aozora-book/`).
- [`README.md`](README.md) — project overview and quickstart.
- `docs/adr/` — the decision records (e.g. ADR-0007 parallel pre-push,
  ADR-0008 diagnostic rendering & this output contract).
