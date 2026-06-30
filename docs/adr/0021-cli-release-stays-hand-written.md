# 0021. CLI release stays hand-written (cargo-dist not adopted)

- Status: accepted
- Date: 2026-06-30
- Deciders: @P4suta
- Tags: release, cli, ci, ecosystem

## Context

The ecosystem 足並みそろえ effort (#105) converged every *other* Rust repo's
binary release onto **cargo-dist** (`dist` 0.32.0): aozora-proof (the model),
afm (P4suta/afm#92), aozora-tools (P4suta/aozora-tools#29), afm-epub
(P4suta/afm-epub#9). Issue #108 proposed staging the same migration for
aozora's CLI binary.

Two things have since settled the question the other way:

1. **release-plz now owns versioning + crates.io.** PRs #311–#314 adopted
   release-plz (trusted publishing, ADR-0020). The original driver behind a
   tooling change — lockstep multi-crate version bumps and ordered crates.io
   publishing — is solved, natively, from `version.workspace = true`. cargo-dist
   never addressed that slice anyway.
2. **aozora's binary release is the most complex in the ecosystem**, and
   cargo-dist would be a regression for it. `release.yml` already:
   - builds the `aozora` CLI on three deliberately-chosen targets
     (ADR — "Why three release targets and not five?");
   - bundles a second package: the `aozora-ffi` cdylib (`libaozora`) plus its
     cbindgen-generated `aozora.h`;
   - generates shell completions (6 shells) and man pages at release time from
     the live clap command tree (ADR-0012), never committed;
   - mints Sigstore build-provenance attestations;
   - stamps `AOZORA_BUILD_VERSION` so `--version` prints the clean triple;
   - ships a **flat** archive (`aozora` + `LICENSE-*` + `NOTICE` + `README.md`)
     that `getting-started/install.md` and the release sanity-check document;
   - runs in a security-hardened, SHA-pinned, reviewer-gated `release`
     environment.

   cargo-dist imposes a `bin/` + `share/` archive layout, has no first-class
   clap-completion / mangen / second-package-cdylib generation, and emits a
   **hand-edit-forbidden** `release.yml` (all changes route through
   `dist-workspace.toml` + `dist generate`). Reproducing the six bespoke
   behaviours above inside that model — and re-verifying the three other
   tag-driven publishers — is real work for marginal benefit.

cargo-dist only ever replaces the **CLI-binary** slice. The multi-ecosystem
publish ladder (`publish-npm` → npm, `publish-pypi` → PyPI,
`publish-extism-wasm` → Extism) stays hand-authored regardless, so a migration
would add a second release system without retiring any.

## Decision

aozora **does not adopt cargo-dist.** The hand-written `release.yml` remains the
owner of the CLI binary build and the GitHub Release; **release-plz** owns
versioning + crates.io; the npm / PyPI / Extism publishers stay hand-authored.
This is already the shipped state (`crates/aozora-book/src/contrib/release.md`,
"Why release-plz?"). Issue #108 ("stage a cargo-dist migration for the CLI
binary") is **superseded** and closed.

## Consequences

- **Deliberate divergence from the ecosystem.** aozora is the one repo whose
  release is bespoke; the lighter-weight ecosystem repos (aozora-proof, afm,
  aozora-tools, afm-epub) use cargo-dist because their release is a plain
  single-binary. A contributor who sees those on cargo-dist and expects it here
  will not find it — this ADR is the record of why.
- **Adding a target stays a one-line `release.yml` edit**, on demand when a real
  consumer asks (per release.md, "Why three … not five?"). No installer or
  extra-target coverage is pre-emptively added.
- **Re-evaluation trigger.** If cargo-dist later grows first-class clap
  completion / mangen / second-package cdylib bundling and a
  reviewer-gated-environment story, or if `release.yml`'s maintenance cost rises
  materially, revisit this decision.

## Alternatives considered

- **Stage the CLI-binary-only migration (the #108 proposal).** Rejected: it
  reproduces six bespoke release behaviours, runs a hand-edit-forbidden
  generated workflow, and must re-verify the three independent tag-driven
  publishers — all for installers plus a couple of extra targets that
  `release.yml` can add one line at a time. release-plz already captured the
  only high-value slice (versioning + crates.io).
- **Full migration including the publish ladder.** Out of the question:
  cargo-dist has no crates.io/npm/PyPI/Extism publishing, so this would lose the
  multi-ecosystem ladder entirely.
