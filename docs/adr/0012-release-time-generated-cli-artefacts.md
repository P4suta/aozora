# 0012. Release-time generated CLI artefacts (completions, man pages)

- Status: accepted
- Date: 2026-06-20
- Deciders: @P4suta
- Tags: cli, release, devex

## Context

`aozora completions <shell>` (and the forthcoming `aozora man`) produce
shell-completion scripts and man pages. Two questions follow: where do
the generated files come from, and do we commit them to the repo?

The repo already has a strong pattern for generated artefacts. The wire
JSON Schema (`xtask schema`) and the per-language wire types
(`xtask types`) are generated, **committed** under
`crates/aozora-book/src/json/` and `…/types/`, and **drift-gated** — CI
fails if a committed copy diverges from the live types (`just
drift-gate`). That gate earns its keep because those artefacts cross a
*serialization boundary* (Rust structs → JSON Schema → other-language
types): the committed copy is a published contract, and drift in it is a
real, externally-visible bug.

Completion scripts and man pages are different in kind. They are a
verbatim, mechanical projection of the clap `Command` tree — no schema,
no cross-language contract, no consumer that pins them as an interface.

## Decision

Completion scripts and man pages are **generated on demand from the
binary and never committed**. `aozora completions <shell>` (and
`aozora man`) render them from the live clap command tree at runtime.
Release tarballs ship them under `completions/` (and `man/`), generated
by the just-built binary during the release job (`release.yml`). There
is no committed copy and therefore no drift gate.

## Consequences

- **Zero drift surface.** Because the scripts are derived from the same
  `Command` the binary parses with, they cannot disagree with the
  installed flags — there is nothing to keep in sync, so nothing to
  gate. A new subcommand or flag is reflected the next time the binary
  runs.
- **No generated artefact in code review.** Completion scripts are large
  and noisy; not committing them keeps diffs about behaviour, not
  generated text.
- **Release-time cost.** The release job runs the binary once per shell
  to emit the scripts. Safe on every target leg: each leg builds for its
  own runner OS, so the binary is runnable there. The generation is
  fail-loud (no `|| true`).
- **Divergence from the schema/types pattern is intentional.** A
  contributor who sees the drift gate for wire artefacts and expects one
  here will not find it; this ADR is the record of why.

## Alternatives considered

**Commit the scripts and drift-gate them, like the wire schema.**
Mechanically possible (`xtask completions dump` + a `check`). Rejected:
it adds a gate with no signal. The wire gate guards a published
cross-language contract; completion scripts are a private, verbatim view
of flags the binary already owns. A committed copy would be pure noise
that breaks on every flag change for no external benefit.

**Generate completions in `build.rs` into `OUT_DIR`.** A common pattern.
Rejected: it bakes generation into every build (slower inner loop) and
hides the scripts in `target/`, where the release job would have to dig
them out per target. An explicit `completions` subcommand is
discoverable, testable, and usable by end users directly (e.g.
`aozora completions zsh > ~/.zfunc/_aozora`).

## References

- `crates/aozora-cli/src/completions.rs` — the generator subcommand.
- `.github/workflows/release.yml` — the "Assemble archive" step that
  emits `completions/` into the tarball.
- Contrast: the wire schema / types drift gates — `just drift-gate`,
  `xtask schema`, `xtask types`; the committed-and-gated pattern this ADR
  deliberately does *not* follow.
- Plan: `cli-devex-effervescent-fox.md`.
