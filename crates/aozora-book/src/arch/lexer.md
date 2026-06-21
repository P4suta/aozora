# Lexer (sanitize → tokenize → pair → classify)

`aozora-pipeline` runs the lexer as four pure-functional stages,
each `fn(input) -> output` with no shared mutable state. The split
keeps the dominant hot path (tokenize / classify) tight, lets the
bench harness measure each stage independently, and maps every
diagnostic to a single stage boundary.

The single public entry [`lex`] drives all four stages
and lands the resulting borrowed AST inside an
`aozora_syntax::borrowed::Arena` provided by the caller. The legacy
"normalize / registry / validate" steps disappeared into a fused
walk inside `lex`; they no longer have standalone stage functions.

## Stage ordering

```mermaid
flowchart LR
    p0["sanitize"]
    p1["tokenize"]
    p2["pair"]
    p3["classify"]
    fused["lex<br/>(fused walk:<br/>normalize + registry + validate)"]

    p0 --> p1 --> p2 --> p3 --> fused
```

Each arrow carries a small data structure (sanitised text, trigger
tokens, pair events, classified spans); no stage reads back into a
previous stage's output.

| Stage | Input | Output | Responsibility |
|---|---|---|---|
| sanitize | raw `&str` | `SanitizeOutput { sanitized: &str, .. }` | BOM strip, CRLF → LF, accent decomposition, decorative-rule isolation, PUA collision pre-scan |
| tokenize | sanitised `&str` | `Iterator<Item = Token>` | SIMD trigger scan (`aozora-scan`) followed by linear tokenise into `Plain` / trigger events |
| pair | `Iterator<Token>` | `Iterator<Item = PairEvent>` | Balanced-stack pairing for all opener/closer trigrams (`｜》《`, `［］`, `〔〕`, `「」`, `《《》》`) |
| classify | `Iterator<PairEvent>` | `Iterator<Item = ClassifiedSpan>` | Full-spec Aozora classification into [`Node`] variants (ruby, bouten, gaiji, tcy, kaeriten, sashie, annotation, …) |

The orchestrator [`lex`] consumes the classify stream,
substitutes PUA sentinels into the normalised text, builds the
side-table registry that maps sentinel positions back to
classified `Node` values, and accumulates diagnostics — all
in a single fused walk over the classified-span stream.

## sanitize stage

The most varied stage by what it touches. Sub-passes (in order):

- **bom_strip** — UTF-8 BOM detection and removal at the head.
- **normalize_line_endings** — CRLF → LF in one `memchr2` pass.
- **rewrite_accent_spans** — ASCII digraph / ligature decomposition
  for [accent gaiji](../notation/gaiji.md#accent-decomposition).
- **isolate_decorative_rules** — long horizontal-rule lines (`──────────`
  patterns) get separated from neighbouring text so the tokenize
  stage's trigger scan does not split them mid-glyph.
- **scan_for_sentinel_collisions** — pre-scan for stray PUA codepoints
  (`U+E001..U+E004`); any hit emits `Diagnostic::SourceContainsPua`
  and the colliding bytes flow through verbatim (the registry has
  no entry for them, so they degrade to plain text).

Each sub-pass is independent and runs over the same buffer. The
output `SanitizeOutput` carries the rewritten text alongside any
diagnostics emitted along the way.

## tokenize stage

The hot path. SIMD multi-pattern scan from
[`aozora-scan`](scanner.md) finds every trigger byte position; a
single linear walk converts those positions into `Token` events:

```rust
pub enum Token<'src> {
    Plain(&'src str),
    Trigger(TriggerKind, Span),
}
```

The trigger scan and the tokenise loop fuse so the output stream
allocates no per-event vector — downstream stages consume the
iterator directly. See [SIMD scanner backends](scanner.md) for the
runtime backend selection.

Throughput on a typical mid-size work (`crime_and_punishment.txt`,
~600 KiB UTF-8): on the order of GB/s for the SIMD backends, which
is well above the rest of the pipeline's throughput; the tokenize
stage is essentially free at the corpus level. Concrete numbers are
pinned by `cargo bench -p aozora-bench --bench crime_and_punishment`
and the synthetic corpus bench.

## pair stage

Balanced-stack bracket matching. Walk the trigger event stream,
push openers onto a `SmallVec<[(PairKind, Span); 8]>` (inline
capacity 8 covers 99th-percentile bracket nesting in real corpus),
pop on closers, and emit a `PairEvent::Solo` / `Matched` /
`Unmatched` / `Unclosed` for every trigger.

The pair stage is also the first place [recovery semantics](error-recovery.md)
fire: stray closers and unmatched openers each emit a structured
diagnostic but never abort, so downstream consumers see a complete
event stream regardless of input wellformedness.

## classify stage

The most code-heavy stage. The classifier maps `PairEvent`s to
[`Node`] variants via a slug-canonicalised dispatch table
([`SLUGS`] / `canonicalise_slug`). Recognisers are organised per
construct family:

- Ruby (`｜青梅《おうめ》`, with implicit-base auto-glob)
- Bouten / forward-bouten (`［＃「平和」に傍点］`, with look-back target resolution)
- Tate-chu-yoko (`［＃「12」は縦中横］`)
- Gaiji (`※［＃説明、ページ-行］`)
- Kaeriten (Chinese-text reading marks)
- Illustration (illustrations)
- Indent / alignment / line-length annotations
- Section / page breaks

The recogniser dispatch is deterministic and slug-canonicalised so
prefix collisions (`ここから2字下げ` vs `ここから2字下げ、地寄せ`)
resolve via the [`SLUGS`] entry's family + arity, not by recogniser
ordering. Look-back targets (bouten / tcy) resolve against the
sanitised text in the same walk.

## Fused finishing walk

After classify, [`lex`] runs a single output-build walk
that does what was once three separate stages:

- **Normalise** — substitute each Aozora span with its PUA sentinel
  (`U+E001`/`E002`/`E003`/`E004` for inline / block-leaf / block-open
  / block-close) so the downstream CommonMark parser sees a flat
  text with single-codepoint placeholders.
- **Register** — build the [`Registry`] (an `EytzingerMap<u32, NodeRef<'src>>`,
  see [van Emde Boas / Eytzinger layout](veb.md)) keyed by sentinel
  byte position so the post-process walk can recover the borrowed-AST
  node from a normalised position in `O(log n)`.
- **Validate + diagnostics** — collect every sanitize / pair /
  classify diagnostic, sort by span, and pin stable codes
  (`aozora::lex::source_contains_pua`, `aozora::lex::unclosed_bracket`,
  …; see [diagnostics](../notation/diagnostics.md)).

Performing all three in one walk avoids three extra passes over
the (potentially MB-class) source and keeps the `Registry`'s
`EytzingerMap` build amortised.

## Why four stages, not one big function?

Three reasons.

1. **Bench-driven optimisation.** Per-stage boundaries let
   `cargo bench -p aozora-bench` measure each stage's wall time
   independently. Knowing that "this document spends 80 % of parse
   time in the classify stage" tells you where the next perf PR
   belongs. A monolithic `lex()` would force re-instrumentation in
   every PR.
2. **Spec compliance.** Each stage corresponds to a discrete
   transformation the spec describes. Spec gaps in production
   almost always land in one stage, and the
   [conformance suite](../conformance.md) can pin regression
   fixtures targeting that stage only.
3. **Composability.** `aozora-pipeline` exposes both the fused
   [`lex`] entry and the per-stage functions
   (`sanitize`, `tokenize` / `tokenize_in`, `pair` / `pair_in`,
   `classify`). Production code uses the fused entry; benchmarks
   and the [type-state Pipeline state machine](pipeline.md) use
   per-stage calls to isolate regressions.

The cost is conceptual (more API surface internal to the crate);
the win is that every perf decision in the parser has a
measurement attached.

## See also

- [Pipeline overview](pipeline.md) — how the lexer fits into the
  full parse layer.
- [SIMD scanner backends](scanner.md) — the tokenize stage's trigger scan.
- [Error recovery](error-recovery.md) — what each stage does when a
  diagnostic fires.
- [Performance → Profiling with samply](../perf/samply.md) — how to
  measure the per-stage cost on your own workload.

[`lex`]: https://docs.rs/aozora-pipeline/latest/aozora_pipeline/fn.lex.html
[`Node`]: https://docs.rs/aozora-syntax/latest/aozora_syntax/borrowed/enum.Node.html
[`SLUGS`]: https://docs.rs/aozora-spec/latest/aozora_spec/static.SLUGS.html
[`Registry`]: https://docs.rs/aozora-pipeline/latest/aozora_pipeline/struct.Registry.html
