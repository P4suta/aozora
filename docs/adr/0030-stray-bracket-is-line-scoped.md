# 0030. A stray `［` is line-scoped in the pair stage

- Status: accepted
- Date: 2026-07-12
- Deciders: @P4suta
- Tags: architecture, notation, parser, recovery

## Context

[ADR-0025](./0025-bracket-is-a-hard-pairing-scope.md) made a `］` a *hard
pairing scope*: it closes the nearest enclosing `［` even when non-bracket
opens are stacked above it, so an unbalanced `「` inside a directive body can
no longer bury the `］` and leave the bracket open. That fixed the *closed*
sink — a `［＃…］` whose `］` was present but swallowed.

It left one dual case untouched, and ADR-0025 said so explicitly: "A
genuinely never-closed `［＃…` (no `]` at all) is untouched — the fix keys
entirely off the `]`; the EOF drain still replays it to plain." A `［` with
**no `］` anywhere after it** stays on the pair stack to end-of-input. The
classify stage opens a frame on that `［` and buffers every following event
into it; the frame never closes, so at EOF the whole buffer — *the entire
rest of the document* — is replayed to plain. Every ruby, heading and
directive after the stray `［` renders as literal source. This is a **leak
cascade**: notation that is individually well-formed is dragged into the
sink by one unmatched delimiter upstream.

The corpus archetype is 江戸川乱歩『影男』L548, whose line ends in a lone
trailing `［`:

```text
［＃５字下げ］［＃中見出し］空中観覧車［＃中見出し終わり］［
```

The document has a net `［` depth of 1 from that point on, and the sink
leaked 236 ruby + 108 directive + 10 bar occurrences in that one file alone.
Corpus-wide the cascade dominated the render-leak tail: fixing it dropped
visible ruby leaks 8710 → 1588 (−82 %), directive 3103 → 1472, bar 1904 →
1604.

The parser is otherwise lossless: an unmatched `［` round-trips its raw bytes
verbatim (ADR-0022), and the splice / incremental engine depends on that.
The fix must preserve every byte-identity and coupling invariant while
stopping the sink.

## Decision

**A `［＃…］` directive body does not span a line break, so a `［` still open
when a newline arrives is stray — the pair stage force-resolves it (and the
contiguous top run of stray brackets above it) as `Unclosed`, innermost-first,
*before* emitting the `Newline`.** This is the temporal dual of ADR-0025's
spatial hard scope: `］` bounds a bracket's *reach across the stack*; a line
break bounds a bracket's *reach across the document*.

On the classify side, receiving the outermost frame's `Unclosed`
(inner-stack position 0) mid-stream **abandons** the frame: the buffered
line fragment folds to plain — byte-identical to the existing EOF
replay-to-plain path — and the frame clears, so the pair events after the
line break classify normally on the *live* event stream, exactly as if the
stray `［` were not there.

Consequences of the "line scope" model:

- **A real directive is byte-for-byte unchanged.** A well-formed `［＃…］`
  closes its `］` on the same line, so no bracket is ever open at that line's
  newline; the resolution loop is a no-op for all clean input.
- **Only currently-broken sinks change behaviour**, so output can only
  improve: the leaked brackets disappear and the content after the stray `［`
  classifies normally.
- The fix **never re-classifies already-paired events** — that is unsound,
  because a buffered event's pairing reflects the stray open's stack context
  (it can pair a later `》`/`≫` against the wrong open). The tail is processed
  once, forward, off the real stream.
- After the stray `［` is line-scoped, a later `］` no longer closes it — it
  becomes a stray `Unmatched` close (folds to plain), so the bracket cannot
  reach back across the newline.
- **Only `［` gets the line scope.** A `《…》` ruby reading and a `≪…≫`
  angle-quote also never span a line, but resolving *them* at a newline would
  rewrite authorial `《…》` (used as literal quotation, a correct non-leak)
  and dialogue `「」` legitimately spans lines. Bracket is the delimiter whose
  never-closed frame drives the cascade, and the one whose single-line
  invariant is unambiguous.
- A never-closed `［` with **no** line break after it (single logical line to
  EOF) is unchanged: it still drains to plain at EOF. The line scope is
  additive.

## Consequences

- The `Unknown`-degradation budget rose once, 3794 → 3815, as a one-time
  *undercount correction* — the same mechanism as ADR-0025's 1884 → 1974:
  `［＃…］` directives that previously sank to plain (never reaching the
  `Directive{Unknown}` counter) now surface and are counted. The
  catalogue-sweep residue rose in lockstep (2389 → 2410) with **zero** change
  to the Tier1 / Tier2 matched-shape sets — no rule regressed and no match
  was lost. Recorded in `corpus/baseline.json` and
  `corpus/catalogue-coverage.json`; the render-leak baseline was ratcheted
  down to the new floor.
- Byte-identity holds across the whole corpus: `corpus verbatim`,
  `corpus_sweep` (round-trip fixed point), `corpus_splice_tiling` and
  `corpus_incremental_merge` all stay 0-diverged with the change in place.
- No spec-vector edit is required — of the 127 vendored vectors none exercises
  a stray line-crossing `［`, so the `conformance vectors` must-gate is
  unmoved. A recovery vector may be contributed upstream to lock the contract.
- A new in-tree recovery fixture pins the reduced 影男 shape
  (`crates/aozora-conformance/fixtures/render/stray_bracket_line_scope`) and
  pair / classify unit tests pin the invariant at each stage.

## Alternatives considered

- **Re-classify the never-closed frame's buffered events at EOF** (fold the
  stray open to plain, re-feed the tail through the recognisers). Rejected as
  *unsound*: the buffered events were paired with the stray `［` on the stack,
  so re-feeding them at a different stack depth pairs nested delimiters against
  the wrong opens — a `≪［《］≫［＃］》` reduction serialised a trailing `》` as
  `≫`, corrupting bytes and breaking the round-trip fixed point. Recognition
  must run once, forward, on the live stream — which is what the line-scope
  model does by bounding the frame *before* the tail is captured.
- **Fix it only in the classifier** (abandon the frame at a newline without
  the pair-stage change). Rejected: the `［` would remain on the *pair* stack,
  so a later `］` would still hard-scope-close it and a later delimiter would
  pair against it — the pair and classify stacks would desync. The scope must
  be established in the pair stage where the stack lives; the classifier's
  mid-stream abandonment is the minimal companion.

## References

- Plan: the 2026-07 v0.5.0 pre-release audit campaign (F21, "contain
  stray-bracket leak cascade").
- [ADR-0025](./0025-bracket-is-a-hard-pairing-scope.md) — the spatial hard
  scope this decision is the temporal dual of.
- [ADR-0022](./0022-notation-hygiene-layer-roles.md) — the lossless-`Unknown`
  round-trip contract this change preserves.
