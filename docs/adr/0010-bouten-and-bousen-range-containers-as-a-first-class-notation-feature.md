# 0010. Bouten and bousen range containers as a first-class notation feature

- Status: accepted
- Date: 2026-06-16
- Deciders: @P4suta
- Tags: notation, lexer, render, diagnostics

## Context

青空文庫 supports a **range form** for 傍点 (emphasis dots) and 傍線
(side lines): a run of text is wrapped between a bare opener and a
matching closer, e.g.

```text
彼は［＃傍点］必ず［＃傍点終わり］来る
本文［＃二重傍線］乙［＃二重傍線終わり］
［＃左に傍線］丙［＃左に傍線終わり］
```

(per the official 注記一覧 <https://www.aozora.gr.jp/annotation/emphasis.html>).
Until now the parser only recognised the **forward-reference** form
(`［＃「対象」に傍点］`); a bare `［＃傍点］` fell through to
`Annotation{Unknown}` and rendered as nothing. The handbook claimed a
`［＃ここから傍点］ … ［＃ここで傍点終わり］` form, but that form does **not
exist** in the spec or in the real corpus (`P4suta/aozorabunko_text`:
`［＃傍点終わり］` = 71 files, `［＃ここで傍点終わり］` = 0). The
catalogue also lists a planned `mismatched_bouten_container` diagnostic
(a 傍点 opener closed by a 傍線 closer), which requires the range form to
be modelled in the first place.

Two forces shaped the design:

1. **Inline, not block.** Every one of the 2301 range-form pairs in the
   corpus sits within a single line — the form is inline emphasis
   (`<em>…</em>`), never a block. A naive reuse of the block-container
   machinery (字下げ / 罫囲み) would wrap the run in paragraph breaks and
   shatter the line.
2. **Open/close pairing for the mismatch check.** Detecting a 点/線
   family mismatch is most naturally expressed as an open/close pairing
   over the normalizer's existing `open_stack`, which already powers
   `mismatched_container_close`.

## Decision

Model the range form as a **`ContainerKind::BoutenRange { kind, position }`**
container that pairs through the normalizer's `open_stack` (giving the
mismatch check for free) **but renders inline**:

- The classifier (`classify`) recognises bare `［＃<variant>］` /
  `［＃<variant>終わり］` (with an optional `左に` left-side prefix) and
  emits `EmitKind::BlockOpen` / `BlockClose(ContainerKind::BoutenRange…)`.
  `parse_bouten_range_body` reuses `bouten_kind_from_suffix`.
- The **normalizer** suppresses the block-leaf `\n\n` padding for
  `BoutenRange` (so the renderer keeps the markers in-paragraph) and,
  on close, compares the open/close **点/線 family** via
  `BoutenKind::is_line` — a difference fires
  `aozora::lex::mismatched_bouten_container` (Error). Same-discriminant
  same-family variant differences (白丸 vs 丸) recover silently on the
  opener's variant.
- The **HTML renderer** treats a `BoutenRange` open/close as inline
  (`ensure_in_paragraph` + `<em class="aozora-bouten-…">` / `</em>`),
  matching the forward-reference markup, instead of the block
  `before_block_emit` / `after_block_emit` path.
- The **serializer** round-trips `［＃<左に?><variant>(終わり)?］`
  byte-for-byte.

The `mismatched_bouten_container` diagnostic is scoped to the 点/線
**family** boundary, matching the catalogue's wording.

## Consequences

- A real, common notation now parses and renders correctly; the
  handbook's fictional `ここから傍点` form and `BoutenForm` AST are
  replaced with the real `［＃傍点］…［＃傍点終わり］` shape.
- `BoutenRange` is the first container that is paired like a block but
  rendered inline. The inline-vs-block decision lives in two places (the
  normalizer's padding and the HTML renderer's paragraph handling),
  keyed off the single `matches!(kind, BoutenRange { .. })` test. New
  container kinds must decide which side they fall on.
- No wire drift: `ContainerPair.kind` is an unconstrained string, so
  adding the `"boutenRange"` tag leaves the JSON schema / `.d.ts`
  unchanged.
- `鎖線` / `破線` / `黒三角傍点` range variants are not yet in
  `bouten_kind_from_suffix`, so they still fall through to
  `Annotation{Unknown}` (none appear in range form in the corpus).

## Alternatives considered

- **Inline paired annotations (warichu-style).** Emit the open/close as
  two inline `Annotation` nodes carrying the kind. Rejected: it would
  need data-carrying `AnnotationKind` variants (touching serde) and a
  bespoke pairing stack for the mismatch check, duplicating machinery the
  `open_stack` already provides.
- **A dedicated `Node` variant.** Cleaner typing, but ripples
  through every exhaustive `Node` match (render / serialize /
  visitor / cst / query) for a construct the container model already
  expresses.
- **Reuse block containers verbatim.** Simplest to wire, but the `\n\n`
  block padding breaks inline emphasis — empirically wrong for 100% of
  corpus occurrences.

## References

- Plan: `aozora-idempotent-frog.md` (Phase B diagnostics catalogue).
- 注記一覧: <https://www.aozora.gr.jp/annotation/emphasis.html>
- Corpus: `P4suta/aozorabunko_text` (range-form usage survey).
- [`diagnostics.md`](../../crates/aozora-book/src/notation/diagnostics.md),
  [`bouten.md`](../../crates/aozora-book/src/notation/bouten.md).
