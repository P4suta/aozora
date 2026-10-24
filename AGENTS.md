# AGENTS.md

Repository-specific requirements for agents.
Use `just --list`, `aozora --help`, and the code for commands and behavior.

## Comments

Default to no comments.
Write only the rationale the code can't express.

Never write a source coordinate, a manually counted total, or a copy of code from another location.
Name the relevant symbol so the compiler checks the reference.
`xtask lint coordinates` rejects source coordinates.

Express an invariant through an assertion, a type, a test, or a gate.
Prose alone doesn't enforce an invariant.

A comment must serve one of these purposes:

- Supply documentation that `missing_docs` requires, limited to what the reader needs.
- Explain a rationale the code can't express.
  Cite the decision record or corpus measurement.
- Supply `// SAFETY:`, a `mutants::skip` reason, or an item-level `#[expect(lint, reason = …)]` when the toolchain requires it.

## Running anything

The locked native mise environment is canonical.
Run `./bootstrap` on a fresh clone, then use `just`.
CI invokes the same fixed `just ci-*` suites.
Don't install ad hoc tool versions outside `mise.toml` and `mise.lock`.
`just ci` mirrors the fast pull request gate and is the pre-push gate on Linux.
Other hosts run `just ci-repo` before pushing.
Heavy checks run in `release-ready`, as [the gate split](docs/adr/fast-pull-request-gate-and-release-gate.md) records.

Sign commits and follow Conventional Commits.
Don't use `LEFTHOOK=0` or `--no-verify`.

## The `aozora` CLI

The CLI reads standard input and writes deterministic output with stable exit codes.
Output contains no timestamps or random values.
Every document subcommand accepts `-` for standard input.

| code | meaning |
|---|---|
| `0` | Parsing succeeded, including tolerated diagnostics |
| `1` | `--strict` and at least one diagnostic |
| `2` | Usage error, unreadable file, or decode failure |
| `3` | an `Internal` diagnostic fired: a bug in aozora, not bad input |

`aozora render FILE | head` exits `0`.
A closed pipe counts as success under ADR-0029, preserving the meaning of exit codes `1` and `2`.

`--format` defaults to `human` on a terminal and `json` when piped.
Pass `--format json` or `--format short` when parsing output.
Never parse `human` output because its layout depends on terminal width and color settings.

## Where decisions live

Read the governing record under `docs/adr/` before changing its subject.
Never edit an accepted record.
A later record supersedes an accepted record.

`docs/contrib/` contains the Rust version support policy and release runbooks.
