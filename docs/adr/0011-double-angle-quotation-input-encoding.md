# 0011. Double-angle quotation: `≪≫` input encoding, `《》` display

- Status: accepted
- Date: 2026-06-16
- Deciders: @P4suta
- Tags: notation, lexer, render, spec

## Context

A 底本 may set a phrase inside twin angle brackets `《…》`. Those glyphs
(U+300A / U+300B) are **exactly** the ruby reading markers, so an Aozora
Bunko transcription cannot write them literally without colliding with
ruby. The official input convention is therefore to **encode** the
quotation with the lookalike math symbols `≪…≫` (U+226A / U+226B) and let
the renderer **restore** the display form `《…》`
(per the input checklist, <https://www.aozora.gr.jp/KOSAKU/textfile_checklist/>):

```text
底本 《重要》  →  transcription ≪重要≫  →  display 《重要》
```

The parser had this **backwards and misnamed**. It recognised `《《…》》`
as the *input* (a phase-1 pass merged two adjacent ruby brackets into a
6-byte `DoubleRuby` trigger), called the node `DoubleRuby`, and *rendered*
`≪重要≫` (the U+226A/U+226B math symbols) — i.e. it consumed the display
form and emitted the input form, the exact inverse of the convention, and
the spec mischaracterised it as a "double-bracket bouten" (§6.2) selected
over ruby by the leftmost-longest rule.

Corpus evidence removes any compatibility constraint: across
`P4suta/aozorabunko_text` (17,886 works) the construct does not appear in
either form, so the fix can **replace** the old behaviour rather than
dual-accept it.

## Decision

Model the construct as **`AngleQuote`**, matching the official convention:

- **Input** is `≪…≫` (U+226A / U+226B). Both are single 3-byte BMP scalars,
  so they are ordinary single-character triggers
  (`TriggerKind::AngleQuoteOpen` / `Close`) — the phase-1 double-merge is
  deleted, and because the delimiters are distinct from the ruby markers
  `《`/`》` there is no leftmost-longest tie-break to resolve.
- **Display** restores `《…》` (U+300A / U+300B) inside
  `<span class="aozora-angle-quote">《…》</span>`.
- **Serialization** emits the input form `≪…≫`; `parse ∘ serialize` is a
  fixed point.
- The node / pair / wire kind is `AngleQuote` / `angleQuote`, the CSS class
  `aozora-angle-quote`, the pandoc `Span` class `angle-quote`.
- A stray `《《…》》` in source is **not** this construct: it is two ruby
  openers and yields a `nested-ruby` diagnostic with plain-text recovery
  (the existing nested-ruby contract — `first_nested_ruby_open` no longer
  special-cases the construct). An empty `≪≫` degrades to plain text like a
  bare `《》`; an unclosed `≪` raises `unclosed-bracket`.

The sibling specification recharacterises the construct as its own family
**§6.15 二重山括弧 (double-angle quotation)**, removed from the §6.2 bouten
page, with grammar `angle-quote = ANGLE-OPEN angle-content ANGLE-CLOSE`
(`%x226A` / `%x226B`). The vendored `angle_quote` conformance vector pins
`≪重要≫ → 《重要》`.

## Consequences

- Input and output now match the official Aozora convention, and the
  construct is named for what it is (a quotation bracket, not ruby, not
  bouten).
- **Breaking** (acceptable pre-1.0, 0 corpus occurrences): input
  `《《》》`→`≪≫`, display `≪≫`→`《》`, wire `doubleRuby`→`angleQuote`, and the
  CSS / pandoc class rename. The umbrella `aozora` crate is the only
  supported surface, so the blast radius is the re-exported node type plus
  the wire string.
- The lexer is simpler: the bespoke `《《`/`》》` merge and the §5.1
  longest-match consequence both disappear.
- afm (the downstream Markdown superset) is non-breaking: it matches
  `Node` through `#[non_exhaustive]` with a `_` arm and never named
  the old variant directly.
- The specification is the enforced master: the `angle_quote` vector is
  vendored into `spec-vectors/` and held by the `just conformance` gate, so
  the parser and the spec cannot silently diverge.

## Alternatives considered

- **Dual-accept `《《》》` as input too.** Keeps any hypothetical existing
  `《《》》` source working. Rejected: it perpetuates the inverted convention,
  and the corpus has zero `《《》》` occurrences, so there is nothing to keep
  working — `《《》》` is far more useful as the (correct) `nested-ruby` signal.
- **Keep rendering `≪≫` (the math symbols) as the display form.** Minimal
  change. Rejected: restoring `《》` is the entire purpose of the
  encoding; emitting `≪≫` defeats it and surfaces the transcription
  artefact to readers.
- **Keep modelling it as a bouten variant (the old §6.2 framing).**
  Rejected: it is a quotation bracket, not emphasis; it is absent from the
  注記一覧 emphasis list, and conflating the two mischaracterises both
  families.

## References

- Plan: `~/.claude/plans/spec-first-3-6-normative-twinkling-plum.md`.
- Input convention: <https://www.aozora.gr.jp/KOSAKU/textfile_checklist/>.
- Spec §6.15 二重山括弧 (sibling `aozora-notation-spec`,
  `src/notation/angle-quote.md`).
- Corpus: `P4suta/aozorabunko_text` (0 occurrences of either form).
- [`angle-quote.md`](../../crates/aozora-cli/src/node-docs/angle-quote.md).
