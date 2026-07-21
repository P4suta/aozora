# 0009. Single source of truth for version pins

- Status: accepted; the handbook that held the canonical pin was retired
  (#549), superseding decisions 1–3 — the version single-source-of-truth now
  lives in the machine sources per
  [ADR-0038](0038-release-plz-owns-versioning-and-crates-io-publishing.md), so
  the `crates/aozora-book/src/…` references below are historical
- Date: 2026-06-15
- Deciders: @P4suta
- Tags: docs, release, versioning

## Context

The handbook documents how to depend on aozora before its v1.0
crates.io publication: by pinning a git tag. That recommended tag is a
concrete version string, and it had quietly proliferated across the
docs. At one snapshot, three different numbers were live
simultaneously:

- `welcome.md` and `bindings/rust.md` said **v0.2.x**,
- `getting-started/install.md` said **0.3.0** — even though that page
  *already declared itself the single source of truth for the pin*,
- the workspace `Cargo.toml` was at **0.4.1**.

Every release that bumps the tag has to chase down each hand-maintained
string. Some always get missed, so the docs skew — and a stale pin in a
"copy this into your `Cargo.toml`" block is actively wrong, not merely
untidy. Multiple sources of truth for one fact is the root cause; the
fix is to collapse them to one.

## Decision

1. **One canonical pin.** The pin block in
   `getting-started/install.md` (the `[dependencies]` snippet, plus the
   `cargo install --tag …` example on the same page) is the *only*
   place in the documentation where a concrete version or tag is
   written.

2. **Every other doc links, never inlines.** Any page that needs to
   point a reader at "the version to use" links to install.md's pin
   section (or to GitHub Releases' **Latest**), rather than reproducing
   a number. `bindings/rust.md` and `welcome.md` now do exactly this.

3. **Prose stays version-neutral.** Where a doc discusses versioning
   *policy* rather than a specific pin, it uses phrasing that never goes
   stale — "`vX.Y.*` patch bumps are safe", "tracks the latest
   release", "released versions track GitHub Releases" — instead of
   naming a release.

4. **`Cargo.toml` remains the machine source of truth.** The workspace
   `version` field is what actually ships; install.md's pin is
   *reconciled against it* on each release. Docs derive from the
   machine fact; they do not compete with it.

## Consequences

- **Docs stop drifting.** There is exactly one documented number, so it
  cannot disagree with itself.
- **One edit per release.** Bumping the documented pin is a single block
  in install.md, not a repo-wide search-and-replace.
- **The cost is one runbook discipline.** Correctness now depends on the
  release process performing that one update. `contrib/release.md`
  therefore carries an explicit checklist item — *on version cut, update
  install.md's pin block; confirm no other doc inlines a tag* — sitting
  with the existing `cargo set-version` / CHANGELOG / tag steps. If the
  runbook skips it, install.md goes stale; but it is now a single,
  named, easy-to-audit step rather than diffuse vigilance.
- **The guard is now mechanized.** The `version-literal-gate` recipe
  (`Justfile`) fails CI and pre-push when a `v[0-9]+\.[0-9]+\.[0-9]+`
  literal appears in any handbook page outside install.md, enforcing rule
  2 automatically. It runs as the `book-versions` CI job (gated on
  `rust || book`, so a handbook-only PR still triggers it) and in the
  always-run lane of `ci-parallel`. The runbook checklist item stays as
  the human counterpart for the one allowed pin.

## Alternatives considered

**Leave the numbers in place, fix them by hand each release.** The
status quo that produced the v0.2.x / 0.3.0 / 0.4.1 three-way skew.
Rejected: it relies on flawless manual upkeep across N files and
demonstrably fails.

**Templating / build-time substitution (inject the version into the
book at build).** mdbook can preprocess, and the version could be read
from `Cargo.toml` and spliced into every `{{version}}` placeholder.
Genuinely single-source, but it adds a preprocessor and build-time
coupling for what is, in practice, one number touched a handful of times
a year. Over-engineered for a single-author book; the link-to-one-page
rule achieves the same single-source guarantee with zero machinery.

**Make `Cargo.toml` the doc source directly (no install.md pin).** The
machine version is already canonical; could the docs just say "see
`Cargo.toml`"? Rejected for ergonomics: a reader wants a copy-pasteable
`tag = "vX.Y.Z"` snippet, and the workspace `version` is not literally
the git tag string a consumer pins (tags carry the `v` prefix and are
cut deliberately). install.md is the human-facing reconciliation of the
machine fact, which is what readers actually need.

## References

- Canonical pin: `crates/aozora-book/src/getting-started/install.md`
  (the "Rust library" pin block; self-declared single source of truth)
- Reconciled-against machine source: workspace `Cargo.toml` `version`
- Linking (no-inline) docs:
  `crates/aozora-book/src/bindings/rust.md`,
  `crates/aozora-book/src/welcome.md`
- Runbook checklist home: `crates/aozora-book/src/contrib/release.md`
  (alongside `cargo set-version` / CHANGELOG / tag)
- Related: ADR-0006 (polyglot bindings) — a `SCHEMA_VERSION` bump there
  is the *machine*-side analogue of this docs-side discipline; ADR-0004
  (lint/profile policy) for the `typos`-style gate pattern the
  `version-literal-gate` follows
