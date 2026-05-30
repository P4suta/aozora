# 0001. Zero parser hooks — Aozora-first lexer

- Status: accepted
- Date: 2026-05-31
- Deciders: @P4suta
- Tags: architecture, parser

> Foundational thesis of the lexer. Originated as afm ADR-0008 and moved
> here when the parser core was extracted (afm ADR-0010).

## Context

青空文庫記法 and the host document format (CommonMark/GFM, in the afm
sibling) are two independent grammars that happen to share a byte stream.
The naive design weaves Aozora recognition *into* the host parser via
per-node hooks — e.g. teaching comrak about `［＃…］`. That couples the
two grammars: every host-parser upgrade risks breaking Aozora handling,
the Aozora rules are scattered across host-parser callbacks, and the
notation can no longer be parsed, tested, or rendered on its own.

## Decision

The Aozora lexer is **first-class and self-contained**: it tokenises and
classifies 青空文庫記法 against its own grammar, with **zero hooks into any
host parser**. The host format never carries Aozora-aware code. Composition
with a host format happens *after* parsing, by splicing the Aozora render
output into the host output at PUA sentinel positions — never by mutating
the host parser.

Concretely:
- `aozora-pipeline` owns the four phases (sanitize / events / pair /
  classify) and emits a borrowed `AozoraNode` tree plus PUA sentinels.
- The classifier's `Annotation::Unknown` catch-all claims every `［＃…］`,
  so a bare `［＃` can never leak to output (Tier-A invariant).
- A host integrator (afm) runs its vanilla parser, then substitutes the
  sentinels with `aozora-render::render_node::render` output.

## Consequences

- The notation is parseable, fuzzable, and renderable standalone — this
  whole repo exists because of that.
- Host parsers stay vanilla (afm pins comrak verbatim, 0-line diff).
- New 記法 extends the classifier, not a host-parser callback.
- Cost: the sentinel-splice composition layer is extra machinery the
  integrator must maintain (it lives in afm, not here).

## Alternatives considered

- **Parse-time hooks in the host parser.** Rejected: couples the grammars,
  forks the host parser, and makes the notation untestable in isolation.
- **A single fused grammar.** Rejected: every host-format upgrade becomes
  an Aozora-grammar migration; the two evolve on different clocks.

## References

- afm ADR-0001 (fork comrak, vendor in-tree), ADR-0010 (extract aozora core).
- `crates/aozora-pipeline/src/lexer/` (the four phases).
