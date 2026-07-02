# 0023. Render-only forward emphasis on a ruby base

- Status: accepted
- Date: 2026-07-02
- Deciders: @P4suta
- Tags: notation, render, diagnostics
- Amends: [ADR-0019](0019-coupled-and-container-minimal-diff-splice.md)

## Context

A forward-reference emphasis directive `［＃「X」に傍点／罫囲み／行右小書き／二重傍線／
太字…］` styles a run named by its quoted target `X`. The classifier resolves `X`
to a preceding occurrence: byte-adjacent (`ForwardOrigin::Reclaimed`) or
interior-to-the pending plain run (`Detached` + a `Referenced` bracket). When
neither holds — `X` is a **ruby base**, an earlier line, a prior construct, or
one of several occurrences — `resolve_forward_referent` returns `Unresolvable`
(`crates/aozora-pipeline/src/lexer/classify/forward.rs`), and the directive is
recognised but declined: the leaf is kept `ForwardOrigin::Referenced` (renders
nothing, serialises the bracket alone) and a `forward_referent_not_stylable`
warning is attached.

ADR-0019 and the `ForwardOrigin` doc frame the ruby-base target as *irreducible
provenance*: `我《われ》…我［＃「我」に傍点］` cannot be pulled into a text-only forward
leaf because "bouten-over-ruby is not representable" as a nested AST node, so it
must stay `Referenced`. ADR-0019 lists the ruby-base literal among the "honestly
irreducible cases" of the minimal-diff **splice** — a coherent text-edit that
rewrites both the bracket target and the base occurrence cannot be carved from a
plain interstitial run.

That reasoning is correct for **editing**, but it conflated two questions. "Can a
text-edit coherently rewrite both copies?" and "Can we *render* the emphasis over
the base?" have different answers. The corpus attests ~60 works where a forward
emphasis names a ruby base on the same line — the single highest-value renderable
slice of the declined set, dominated by 罫囲み / 行右小書き / 二重傍線 / 太字 (not 傍点;
the `mixed_ruby_bouten` fixture's `青梅に傍点` shape is synthetic). This ADR
revisits only the render/diagnose question for the unique-ruby-base sub-case; the
splice decline stands.

## Decision

Add a **render-only** decoration to the already-emitted ruby, rather than nest
emphasis inside it (which the AST cannot express — `ForwardFormatOwned.target`
is a `ContentRange` over `ContentOwned{Plain|Segments}` and holds no `Ruby`).

### Model

`RubyOwned` gains `base_emphasis: Option<ForwardAttr>` (default `None`).
`ForwardAttr` is `Copy` and carries the full forward attribute
(`Framed`, `SmallScript`, `Bouten { kind, position }`, `Bold`, …) with no
interned body, so `RubyOwned` stays `Copy` — no `Box`/`Id`, the inline cluster
is preserved, and the `assert_copy::<RubyOwned>()` pin still holds. Because the
field rides on the ruby, not on `ForwardAttr`, downstream pins on `ForwardAttr`
are untouched.

### Lower (not classify)

The classifier is a streaming iterator: it hands the `Ruby` to the consumer
before it classifies the later directive, so it cannot reach back and mutate the
emitted node. The decoration is instead a post-classification phase in the
`lower_spans` lowering pass (`crates/aozora-pipeline/src/pipeline.rs`), which
already reaches back and mutates spans in place (the `promote_headings`
precedent). Over the lowered span list, for each `ForwardOrigin::Referenced`
forward leaf whose attribute decorates a whole run, the phase resolves the
target text and scans the look-back for a `Ruby` whose base text equals it. When
the match is **unique** it sets that ruby's `base_emphasis` and returns the
directive's span so the pipeline builder suppresses that directive's
`forward_referent_not_stylable` warning. The forward leaf is still emitted as
`ForwardOrigin::Referenced`, unchanged.

Uniqueness is load-bearing, not "nearest ruby wins": the target must match
**exactly one** preceding ruby base **and no** preceding plain-text run anywhere
in the look-back. A plain copy that precedes the ruby — cross-line, or same-line
before the ruby, out of the classifier's post-node reset window — is a competing
referent, so the phase declines and keeps the honest warning (this is why a
single-kanji target that also appears as ordinary prose, e.g. 露 / 蝶 / 花, stays
declined). Two region-local rubies with the same base, a prior-construct or
`、`-joined multi-target, and attributes that address sub-characters
(`AccentDot`, `Accent`, needing the target to *be* a Latin letter) or split the
target (`Fraction`) all stay declined too.

### Render

`render_ruby_owned` wraps the base render in the attribute's emphasis element,
placed **inside** `<ruby>`, before the base's `<rp>`, so the emphasis marks the
base glyphs and the `<rt>` reading stays outside it. The wrapper is derived by
reusing `render_format_owned` over a synthetic `ForwardOrigin::SelfContained`
leaf on the base, so **every** attribute kind wraps identically —
傍点 → `<em class="aozora-bouten …">`, 罫囲み → framed `<span>`, 行右小書き / 太字 /
二重傍線 / 文字サイズ → their own elements — with no bouten special-case. The
separate `Referenced` forward leaf still renders nothing, so exactly one styled
copy exists (no #228 double-render).

### Serialize / splice / incremental invariants

`base_emphasis` is never read by `to_source`. The bracket bytes come from the
`Referenced` leaf, which serialises `［＃「X」は…］` verbatim; the ruby serialises
`X《…》`. Serialize output is byte-identical (`corpus verbatim`: 0 divergence over
17,889 docs), and the render-gate fixed point holds. Because the forward leaf
stays `Referenced`, ADR-0019's splice model is unchanged: `classify_node_ref`
still classifies a `base_emphasis` ruby `(Ruby, Direct)`, so source-region
splicing and the identity-verbatim / no-Opaque-region tiling gates are
untouched, and the ruby-base forward **edit** stays declined `Unverifiable`.

The one new coupling is a cross-node *render* dependency: the ruby now renders
emphasis because a directive elsewhere set `base_emphasis`. The incremental
re-parse engine gates on serialize byte-identity, which cannot observe a
render-only field, so the region-reuse guard is made explicit:
`node_forbids_region_reuse` forbids reusing a ruby's region while
`base_emphasis.is_some()`. An edit that removes or retargets the directive
without intersecting the ruby therefore falls back to a full parse rather than
reuse a cached ruby with stale emphasis. `.is_some()` reads only the ruby's
`Copy` Option tag, so the store-free diagnostics-only hot path stays sound.

## Consequences

- The dominant, on-line ruby-base forward emphasis now renders its emphasis
  (~50 corpus occurrences — `forward_referent_not_stylable` drops from 211 to
  161) with no false warning, attr-agnostically across 罫囲み / 行右小書き /
  二重傍線 / 太字 / 傍点 / 文字サイズ.
- No new `NodeKind`/`DirectiveKind`; `RubyOwned` stays `Copy`; the node table,
  pairs, container-pairs, serialize output, and the round-trip fixed point are
  all byte-identical. Only `to_html` and the diagnostics envelope change, and
  only for the resolved unique-ruby-base case. The corpus `Unknown` total is
  unchanged.
- The render gate gains three fixtures: `mixed_ruby_bouten` (updated
  `expected.html` gains the base `<em>`, `expected.diagnostics.json` drops the
  warning, all other axes byte-identical), `ruby_base_framed_forward` (a real
  罫囲み-over-ruby-base, proving the attr-agnostic render), and two decline
  guards — `ruby_base_bouten_ambiguous` (two same-base rubies) and
  `ruby_base_plus_preceding_plain` (a plain copy before the ruby) — pinning that
  a non-unique / competing referent STAYS declined.
- The declined set shrinks to its genuinely irreducible remainder: ambiguous
  multi-occurrence, cross-line, prior-construct, and `、`-joined multi-target
  targets, plus every declined splice **edit** of ADR-0019.

## Alternatives considered

- **Mutate the ruby in the classifier.** Impossible: the streaming classifier
  yields the ruby to the consumer before it classifies the later directive, so
  it holds no `&mut` to reach back. The `lower_spans` post-pass is the correct
  seam.
- **Nest a bouten node around the ruby.** Impossible: `ForwardFormatOwned.target`
  cannot hold a `Ruby`, and widening it would reintroduce a boxed, non-`Copy`
  target for a single render case.
- **Wrap the whole `<ruby>` in the emphasis.** Rejected: it emphasises the `<rt>`
  reading as well as the base; the emphasis belongs to the base glyphs only, so
  it must sit inside `<ruby>` around the base.
- **Bouten-only decoration.** Rejected: the real ruby-base population is
  dominated by 罫囲み / 行右小書き / 二重傍線 / 太字, so a bouten-only field would
  miss ~80% of the addressable cases. The `Option<ForwardAttr>` field with a
  reused whole-run render is attr-agnostic.
- **Apply to the nearest preceding ruby without a uniqueness / preceding-plain
  check.** Rejected: it silently styles the wrong run in the ambiguous case; the
  honest-decline discipline of ADR-0019 requires declining a non-unique or
  competing referent.
- **Classify the ruby non-`Direct` when `base_emphasis.is_some()`.** Rejected for
  the splice model (it would break the ADR-0019 tiling gates); the cross-node
  dependency is instead encoded on the incremental *reuse* guard alone.

## References

- `crates/aozora-syntax/src/owned/payload.rs` (`RubyOwned.base_emphasis`),
  `crates/aozora-syntax/src/alloc_owned.rs` (`ruby` / `left_ruby`),
  `crates/aozora-syntax/src/format.rs` (`ForwardAttr`, `ForwardOrigin`).
- `crates/aozora-pipeline/src/pipeline.rs`
  (`lower_spans`, `decorate_ruby_bases`, the diagnostic `retain`).
- `crates/aozora-render/src/render_node_owned.rs`
  (`render_ruby_owned`, `render_format_owned`).
- `crates/aozora/src/incremental.rs` (`node_forbids_region_reuse`).
- `crates/aozora-conformance/fixtures/render/mixed_ruby_bouten/`,
  `.../ruby_base_framed_forward/`, `.../ruby_base_bouten_ambiguous/`,
  `.../ruby_base_plus_preceding_plain/`,
  `crates/aozora-conformance/tests/render_gate.rs`.
- ADR-0019 (splice: ruby-base edit stays declined — amended here for render),
  ADR-0010 (bouten/bousen range containers), ADR-0003 (forward provenance).
- Issue #384 (this work), #202 (splice core).
