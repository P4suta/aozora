# 0025. `［＃…］` is a hard pairing scope in the pair stage

- Status: accepted
- Date: 2026-07-03
- Deciders: @P4suta
- Tags: architecture, notation, parser, recovery

## Context

The pair stage (`aozora-pipeline` `lexer/pair.rs`) matches delimiters with a
single balanced stack: `［`, `「`, `《`, `≪`, `〔` all push, and a close pops
only when it equals the top. A close that does not match the top is emitted
`Unmatched` and the stack is left untouched — deliberately conservative, so a
stray `》` inside a bracket body does not derail the outer pair.

That conservatism had a corpus-wide failure mode. When a `［＃…］` directive
*body* contains an unbalanced `「` — common in real Aozora text:

- image captions: `［＃「直径六糎、線の幅は〇・二粍の円の図（fig…png、…）入る］`,
- composed-glyph gaiji descriptions: `［＃「口＋「皐」の「白」にかえて「自」、第4水準2-4-33］`,
- typo-notes quoting literal quote characters: `［＃「ござんす！」」は底本では「ござんす！「」］`,
- 字下げ directives that name a bare `「`: `［＃ここから２字下げ、ただし冒頭の「のみは１字下げ］`,

— the dangling `「` sits on top of the stack when the `］` arrives, so the `］`
is `Unmatched`, the bracket never closes, and the classifier's open frame
buffers the **entire rest of the document**. At end-of-input the never-closing
frame is replayed to plain text, so every ruby, directive and gaiji *after* the
offending `［＃` renders as literal source — a "sink". Measured impact before
this change: the sink was the dominant *source of leaked ruby volume* (render
occurrences 51,942 → 10,186, −80%, when fixed) and inflated the leaked-directive
count.

The parser is otherwise lossless: an unrecognised directive stays
`DirectiveKind::Unknown` and round-trips its raw bytes verbatim (ADR-0022), and
the splice / incremental engine depends on that. The fix must preserve every
byte-identity and coupling invariant while stopping the sink.

## Decision

**A `］` treats its bracket as a *hard pairing scope*.** When a `］` arrives and
the top of the stack is not a bracket, the pair stage finds the nearest
enclosing `［` and closes it, force-resolving every non-bracket open stacked
above that bracket as `Unclosed` (innermost-first) before emitting the
`PairClose`. The classifier, on receiving a mid-stream `Unclosed`, drops the
matching inner-stack entry — *except the frame's outermost open at position 0,
which may only be closed by a real `PairClose`* — so the directive frame closes
on its `］` and recognition runs on a bounded body.

Consequences of the "hard scope" model:

- **Balanced bodies are byte-for-byte unchanged.** When the inner `「X」` closes
  before the `］`, the top *is* the bracket and the existing fast path returns
  directly. Every forward-reference recogniser (`［＃「X」に傍点］` bouten,
  縦中横, headings, 左ルビ, …) still sees the inner `「X」` as real
  `PairOpen`/`PairClose(Quote)` events — the directive body stays *event-bearing*,
  it is not made opaque.
- **Only currently-broken sinks change behaviour**, so output can only improve:
  the leaked brackets disappear, and content after the directive classifies
  normally.
- The unwound `「` carries no `PairLink`; the directive body round-trips as raw
  `Unknown` bytes, honouring the verbatim contract.
- A genuinely never-closed `［＃…` (no `]` at all) is untouched — the fix keys
  entirely off the `]`; the EOF drain still replays it to plain.
- A `」` still cannot cross a bracket downward — only `］` gets scope power.

## Consequences

- The `Unknown`-degradation budget rose once, 1884 → 1974, as a one-time
  *undercount correction*: directives that previously sank to plain (never
  reaching the `Directive{Unknown}` counter) are now counted. Balanced
  directives are byte-identical, so the delta is purely the newly-surfaced
  unbalanced-quote directives. Recorded in `corpus/baseline.json`.
- New in-tree recovery fixtures pin the four corpus shapes plus a nested case
  (`crates/aozora-conformance/fixtures/render/directive_*_*`,
  `gaiji_composed_glyph_unbalanced_quote`, `nested_directive_unbalanced_inner_quote`).
- Byte-identity holds across the whole corpus: `corpus verbatim`, `corpus_sweep`
  (round-trip fixed point), `corpus_splice_tiling`, `corpus_incremental_merge`
  all 0-diverged with the change in place.
- No spec-vector edit is required — of 127 vendored vectors none exercises an
  unbalanced-quote directive body, so the `conformance vectors` must-gate is
  unmoved. A recovery vector may be contributed upstream to lock the contract.

## Alternatives considered

- **Make the directive body opaque to `「」` in the pair stage** (stop tokenising
  quotes between `［＃` and `］`). Rejected: nine forward-reference recognisers
  resolve the inner `「X」` through real `PairOpen`/`PairClose(Quote)` events and
  `view.links`; opacity would drop `［＃「X」に傍点］`, 縦中横, headings, 左ルビ,
  emphasis and box-enclosure to `Unknown` — a large regression — and would force
  rewriting all nine to byte-parse their targets.
- **Fix it only in the classifier** (force-close the frame on the first `]`).
  Rejected: the pair stage never delivers a `PairClose(Bracket)` for the buried
  `]` (it delivers `Unmatched`), and force-closing still leaves the dangling
  inner `Quote` on the frame's inner stack, so the sink persists. The mismatch
  is in the single-stack model and must be fixed there; the classifier change is
  the minimal companion (drop the unwound inner entry, never the outermost).

## References

- Plan: the 2026-07 ruby-leak eradication + render-correctness campaign
  (`corpus render-audit` #395, this is its Category-C fix).
- [ADR-0022](./0022-notation-hygiene-layer-roles.md) — the lossless-`Unknown`
  round-trip contract this change preserves.
- [ADR-0019](./0019-coupled-and-container-minimal-diff-splice.md) — the
  container splice model whose coupling invariants the fix keeps 0-diverged.
