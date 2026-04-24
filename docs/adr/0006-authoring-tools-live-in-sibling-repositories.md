# 0009. Authoring tools (formatter / LSP / editor plugins) live in sibling repositories

- Status: accepted
- Date: 2026-04-25
- Deciders: @P4suta
- Tags: ecosystem, repo-layout, release-strategy, extension-projects

## Context

afm has reached a point where its public surface is useful beyond the
single-binary `afm render` / `afm check` CLI. The Aozora-first pipeline
(ADR-0008) exposes structured artifacts that make it cheap to build
*authoring support tools* on top of the parser:

- `afm_parser::parse` returns `ParseResult { root, diagnostics,
  artifacts }` with span-annotated diagnostics carried as
  `afm_lexer::Diagnostic` + `miette::SourceSpan` — one-to-one mappable
  onto LSP `PublishDiagnostics`.
- `afm_parser::serialize` inverts the pipeline and satisfies the
  corpus sweep's I3 fixed-point invariant — byte-identical after a
  single round-trip. This is a drop-in backend for an `afm fmt`
  formatter.
- `afm_encoding::gaiji::resolve` is a pure `(Gaiji) → Resolution`
  function suitable for `textDocument/hover` on `※［＃…］` references.
- `docs/specs/aozora/` vendors the official annotation manual so the
  hover content has a stable offline source of truth.

The space is not empty but it is shallow. A 2026-04 survey of existing
authoring tools found:

- **novel-writer** (Taiyo Fujii, VS Code): regex-based syntax
  highlighting for ruby / bouten plus a vertical-writing preview and
  character counter. No structural analysis.
- **テキスト小説** (VS Code): snippet-based input assistance plus a
  vertical preview. No structural analysis.
- **縦式** (standalone editor): vertical-writing authoring, ruby /
  bouten / 縦中横 support, but not an LSP — single editor only.

None of them detect missing forward-reference targets
(`［＃「X」に傍点］` where `X` does not appear earlier in the paragraph),
surface unknown `［＃…］` annotations as warnings, offer gaiji hover,
or produce an idempotent canonical formatter. afm's existing diagnostic
machinery makes those capabilities cheap to ship — they fall out of
the lexer's `Diagnostic` variants plus one `parse ∘ serialize` round
trip — so the question is not whether to build them, but where.

Three distinct deliverables are in scope once the decision is made:

1. `afm fmt` — a CLI formatter (`parse ∘ serialize`) with
   `--check` / `--write` / stdin-stdout modes.
2. `aozora-lsp` — a `tower-lsp` server exposing
   `textDocument/publishDiagnostics`, `textDocument/formatting`,
   and `textDocument/hover` for gaiji.
3. A VS Code extension (and, by construction, any LSP-speaking
   editor: Neovim, Helix, Emacs, Zed) that launches `aozora-lsp`.

The layout decision is: do these live inside the current workspace,
or in a sibling repository?

## Decision

**Authoring tools ship in a sibling repository, tentatively
`P4suta/aozora-tools`.** The current repo (`P4suta/afm`) keeps its
current responsibilities — parser, CLI, book, corpus — and exposes
its library crates as the stable dependency surface for the sibling
project.

### Layout

```
P4suta/afm/               (this repo)
├── crates/afm-syntax     (public lib)
├── crates/afm-lexer      (public lib)
├── crates/afm-parser     (public lib)
├── crates/afm-encoding   (public lib)
├── crates/afm-cli        (afm render / check binary)
├── crates/afm-corpus     (17k-work sweep source)
├── crates/afm-book       (mdbook site)
└── crates/xtask          (dev automation)

P4suta/aozora-tools/         (sibling repo, to be created)
├── crates/aozora-fmt        (CLI: parse → serialize)
├── crates/aozora-lsp        (tower-lsp server)
└── editors/vscode/       (TypeScript client for aozora-lsp)
```

### Dependency direction

One-way: **`aozora-tools → afm`**, never the reverse. The current repo
contains no build-time, test-time, or doc-time reference to
`aozora-tools`.

### Distribution

`aozora-tools` depends on the library crates via **git dep pinned to a
tag**:

```toml
afm-parser   = { git = "https://github.com/P4suta/afm", tag = "v0.1.0" }
afm-lexer    = { git = "https://github.com/P4suta/afm", tag = "v0.1.0" }
afm-syntax   = { git = "https://github.com/P4suta/afm", tag = "v0.1.0" }
afm-encoding = { git = "https://github.com/P4suta/afm", tag = "v0.1.0" }
```

crates.io publication remains out of scope for the current release
cycle (consistent with the v0.1.0 GitHub-Releases-only distribution
plan). Publishing to crates.io is a reversible addition that the
sibling repo can drive later if there is demand.

### API stability contract

The library crates' public surface is treated as stable-for-v0.1: a
breaking change to any of the items listed below requires a semver
major bump (v0.2.0) rather than a v0.1.x patch.

Stable surface (consumed by `aozora-tools`):

- `afm_parser::{parse, serialize, ParseResult, ParseArtifacts, Options}`
- `afm_parser::Diagnostic` (re-exported from `afm-lexer`)
- `afm_lexer::{lex, LexOutput, Diagnostic, PlaceholderRegistry}`
- `afm_lexer::{INLINE_SENTINEL, BLOCK_LEAF_SENTINEL,
  BLOCK_OPEN_SENTINEL, BLOCK_CLOSE_SENTINEL}`
- `afm_syntax::{AozoraNode, Content, Segment, SegmentRef, Span,
  ContainerKind, BoutenKind, BoutenPosition, SectionKind,
  AozoraHeadingKind, AnnotationKind, Ruby, Bouten, TateChuYoko,
  Gaiji, Indent, AlignEnd, Warichu, Keigakomi, AozoraHeading,
  HeadingHint, Sashie, Kaeriten, Annotation, DoubleRuby, Container}`
- `afm_encoding::{decode_sjis, has_utf8_bom, DecodeError}`
- `afm_encoding::gaiji::{resolve, Resolution, lookup}`

Internal modules, phase-by-phase lexer types
(`phase0_sanitize::SanitizeOutput`, …), and `afm-parser::aozora::html`
rendering internals are *not* part of the stable surface; consumers
outside this repo should not depend on them.

### Core extraction (Stage 3) is deferred

A future, more ambitious refactor would move `afm-syntax` /
`afm-lexer` / `afm-parser` / `afm-encoding` plus `upstream/comrak/`
and `spec/commonmark-*` / `spec/gfm-*` into a third repository
(tentatively `P4suta/afm-core`), leaving the current repo with
`afm-cli` + `afm-corpus` + `afm-book` + `xtask`, and having both
repos (afm and aozora-tools) depend on `afm-core`.

This is **explicitly deferred** under this ADR. Trigger conditions
before reconsideration:

1. `aozora-tools` is past its MVP and in active use;
2. at least one breaking API change (semver major bump) has shipped,
   so there is concrete evidence of what parts of the surface
   actually change;
3. a second non-`afm` consumer (beyond `aozora-tools`) is on the
   horizon, justifying the library-as-product posture.

Until those conditions hold, splitting now would force API guesses
that are better made after one real downstream consumer exists.

## Consequences

**Becomes easier:**

- Release cadence decouples. `afm` ships on its existing CommonMark
  0.31.2 + golden 56656 gate; `aozora-tools` ships on whatever testing
  discipline fits formatter / LSP work (integration-test-heavy,
  Playwright-like for the vscode client).
- The current repo's ADR constraints (ADR-0001's 200-line comrak
  diff budget, ADR-0008's zero-parser-hook pipeline, the 17k-work
  corpus sweep) no longer need to bind formatter / LSP work — those
  constraints exist to protect parser correctness and are noise for
  an editor-surface project.
- Contributor onboarding splits by skill. Parser work stays Rust +
  property testing + corpus sweep. LSP / vscode work is Rust +
  `tower-lsp` / TypeScript + `vscode-languageclient`; a contributor
  who knows the latter can skip the former.
- Precedent is uniform across the ecosystem: `taplo` (TOML LSP),
  `marksman` (Markdown LSP), `texlab` (LaTeX LSP), `rust-analyzer`
  (Rust LSP) all live in dedicated repositories separate from the
  parser or compiler they wrap.

**Becomes harder:**

- Atomic cross-repo changes are no longer possible: a breaking
  change to `afm_parser::parse`'s return shape requires a coordinated
  tag on `afm` + a PR on `aozora-tools` updating the pin, rather than a
  single workspace commit. Mitigation: the stable-surface contract
  above pushes back on frivolous breakage, and `aozora-tools` can
  bounce-test against `git main` in CI in addition to the tag pin.
- The API surface is now load-bearing in a way it was not before.
  A refactor that renames `ParseResult.artifacts` or reshapes
  `Diagnostic` has a concrete downstream cost. Mitigation: semver
  discipline + `cargo semver-checks` (already wired into
  `just semver`).
- Docs on the current repo must point at the sibling repo for
  authoring-tool use cases, rather than self-contained. CLAUDE.md
  gets a one-paragraph pointer; the mdbook site gets a chapter once
  `aozora-tools` ships anything visible.

**Non-consequences:**

- The current repo's `just` targets, lint profile (ADR-0006), Docker
  execution model (ADR-0002), and corpus sweep (ADR-0007) are
  unchanged.
- No existing crate is renamed, moved, or removed.
- The stable-surface contract holds whether or not `aozora-tools` is
  ever actually built; it is a standalone deliverable of this ADR.

## Alternatives considered

**A) Same workspace, new crates (`crates/aozora-fmt`, `crates/aozora-lsp`).**
Atomic updates across parser and tools; one CI pipeline; lint /
coverage gates shared. *Rejected:* conflates release cycles, forces
parser-scope ADRs (corpus sweep, comrak diff budget, CommonMark 0.31.2
compliance gate) onto work that has nothing to do with parsing
correctness, and widens the contributor surface in a way that would
slow both sides.

**B) Extract the parser first (Stage 3 now).** Split `afm-syntax`,
`afm-lexer`, `afm-parser`, `afm-encoding` plus vendored comrak into a
`P4suta/afm-core` repository *before* building `aozora-tools`. *Rejected:*
YAGNI. The API contract for a library-as-product is shaped by its
consumers; the only current consumer is `afm-cli` (which lives in the
same repo and can be changed atomically). Splitting before
`aozora-tools` exists forces API guesses that will likely be wrong, and
the `git filter-repo` migration of history + ADRs + `upstream/comrak`
is large enough to want concrete justification.

**C) Publish to crates.io instead of git deps.** Adopt crates.io as
the distribution channel for the library surface from v0.1.0.
*Rejected for now:* inconsistent with the v0.1.0 GitHub-Releases-only
distribution plan. git deps via tag provide the exact same
reproducibility guarantee for a single downstream consumer. Revisit
when there is external demand or a second consumer.

**D) Do nothing; keep afm as a parser-only project.** Ship v0.1.0 as a
library and let third parties build tooling. *Rejected:* the existing
tools (novel-writer / テキスト小説 / 縦式) have not filled the
structural-analysis gap in three years, and the infrastructure afm
already has makes shipping the tools cheap. Not building them cedes
the obvious differentiator.

## References

- ADR-0001 — fork / vendor strategy + 200-line diff budget. The
  budget is parser-scope; authoring tools are outside it.
- ADR-0002 — Docker-only execution. The sibling repo will adopt the
  same model independently.
- ADR-0007 — corpus sweep strategy. Parser-scope; authoring tools
  have their own test strategy.
- ADR-0008 — zero-parser-hook Aozora-first pipeline. Defines the
  stable API surface this ADR treats as load-bearing.
- Plan file `~/.claude/plans/flavaord-markdown-vs-code-harmonic-sonnet.md`
  — three-stage rollout (this ADR is Stage 1; `aozora-tools` creation is
  Stage 2; core extraction is Stage 3 and deferred).
- Memory `project_afm_public_release.md` — v0.1.0 GitHub-Releases
  distribution plan with crates.io publish deferred.
- Prior art: [taplo](https://github.com/tamasfe/taplo),
  [marksman](https://github.com/artempyanykh/marksman),
  [texlab](https://github.com/latex-lsp/texlab).
