# Architecture Decision Records

This directory's `adr/` folder holds [MADR 4.0](https://adr.github.io/madr/)
Architecture Decision Records. Significant, hard-to-reverse decisions live
there; read the one that governs an area before changing what it governs. Once
accepted an ADR is never edited — it is *superseded* by a later ADR that links
back.

| ADR                                                                                  | Title                                                                 | Status   |
| ------------------------------------------------------------------------------------ | --------------------------------------------------------------------- | -------- |
| [0001](./adr/0001-zero-parser-hooks.md)                                              | Zero parser hooks — Aozora-first lexer                                | accepted |
| [0003](./adr/0003-accent-decomposition-preparse.md)                                  | Accent decomposition preparse                                         | accepted |
| [0004](./adr/0004-lint-profile-policy.md)                                            | Lint profile policy                                                   | accepted |
| [0005](./adr/0005-corpus-sweep-strategy.md)                                          | Corpus sweep strategy                                                 | accepted |
| [0006](./adr/0006-polyglot-bindings-via-extism.md)                                   | Polyglot bindings via an Extism wasm plugin + schema-driven typegen   | accepted |
| [0007](./adr/0007-parallel-pre-push-pipeline.md)                                     | Parallel, fast pre-push pipeline                                      | accepted |
| [0008](./adr/0008-diagnostic-rendering-and-agent-output.md)                          | Diagnostic rendering & the agent-facing output contract              | accepted |
| [0009](./adr/0009-version-single-source-of-truth.md)                                 | Single source of truth for version pins                               | accepted |
| [0010](./adr/0010-bouten-and-bousen-range-containers-as-a-first-class-notation-feature.md) | Bouten / bousen range containers as a first-class notation feature | accepted |
| [0011](./adr/0011-double-angle-quotation-input-encoding.md)                          | Double-angle quotation: `≪≫` input encoding, `《》` display            | accepted |
| [0012](./adr/0012-release-time-generated-cli-artefacts.md)                           | Release-time generated CLI artefacts (completions, man pages)         | accepted |
| [0013](./adr/0013-cli-configuration-file.md)                                         | CLI configuration file (`.aozora.toml`)                               | accepted |
| [0014](./adr/0014-cli-watch-mode.md)                                                 | CLI watch mode and the `notify` dependency                            | accepted |
| [0015](./adr/0015-spec-syntax-layer-boundary.md)                                     | The spec / syntax layer boundary                                      | accepted |
| [0016](./adr/0016-consolidate-tooling-into-the-aozora-monorepo.md)                   | Consolidate the editor/CLI tooling into the aozora monorepo           | accepted |
| [0017](./adr/0017-ecosystem-dependency-pin-policy.md)                                | Ecosystem dependency-pin policy                                       | accepted |
| [0018](./adr/0018-minimal-diff-splice-and-source-region-ownership.md)                | Minimal-diff splice and source-region ownership                       | superseded by [0019](./adr/0019-coupled-and-container-minimal-diff-splice.md) |
| [0019](./adr/0019-coupled-and-container-minimal-diff-splice.md)                      | Coupled and container minimal-diff splice                             | accepted |
| [0020](./adr/0020-release-secret-hardening-trusted-publishing.md)                    | Release secret hardening via Trusted Publishing and environment gates | accepted |
| [0021](./adr/0021-cli-release-stays-hand-written.md)                                 | CLI release stays hand-written (cargo-dist not adopted)               | accepted |
| [0022](./adr/0022-notation-hygiene-layer-roles.md)                                   | Notation-hygiene layer roles: parser / linter / formatter             | accepted |
| [0023](./adr/0023-render-only-forward-emphasis-on-a-ruby-base.md)                    | Render-only forward emphasis on a ruby base                           | accepted |
| [0024](./adr/0024-canonical-reference-stylesheet.md)                                 | Canonical reference stylesheet for notation presentation              | accepted |
| [0025](./adr/0025-bracket-is-a-hard-pairing-scope.md)                                | `［＃…］` is a hard pairing scope in the pair stage                     | accepted |
| [0026](./adr/0026-notation-hygiene-restratification.md)                              | Notation-hygiene re-stratification: Tier1 purity and render-only Tier2 | accepted |
| [0027](./adr/0027-core-parser-notation-purification.md)                              | Core parser notation purification: decline non-canonical forms         | accepted |
| [0028](./adr/0028-remove-dead-framed-and-invalidrubyspan-core-surfaces.md)           | Remove dead Framed and InvalidRubySpan core surfaces                   | accepted |
| [0029](./adr/0029-broken-pipe-exit-semantics.md)                                     | Broken-pipe exit semantics                                             | accepted |

## Authoring a new ADR

1. Scaffold with `just new-adr "Short imperative title"` (copies
   `adr/0000-template.md` to the next sequential number).
2. Fill in the sections; keep paragraphs short and action-oriented.
3. Add a row to the table above.
4. Reference the ADR in the commit body and open a PR. ADRs are normally
   accepted on merge; controversial ones land as `proposed` and flip to
   `accepted` once the discussion concludes.

## Numbering

`aozora` was split out of [`P4suta/afm`](https://github.com/P4suta/aozora-flavored-markdown)
(afm ADR-0010, "extract aozora core"). The parser-layer decisions that
originated on the afm side moved here and were **renumbered** into this
repo's own sequence; afm keeps redirect stubs (`NNNN-MOVED.md`) pointing
at the canonical text here. The numbering therefore starts at 0001 and
has gaps relative to afm's (and no 0002) — that is expected, not a mistake.
New aozora ADRs continue this repo's sequence.
