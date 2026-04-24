# 0008. Zero-parser-hook Aozora-first lexer pipeline

- Status: accepted
- Date: 2026-04-24
- Deciders: @P4suta
- Supersedes: ADR-0005 (paired block annotation container hook)
- Tags: architecture, parser, upstream-diff, lexer, pure-functional

## Context

M0/M1 built afm on comrak's extension hooks: comrak drives parsing, and when
its inline scanner hits a trigger character (`｜` / `《` / `［` / `※`) it
calls into `AozoraExtension::try_parse_inline`; ADR-0005 was about to add a
symmetric `try_start_block` dispatch for paired containers. Total upstream
diff ~33 lines, planned to grow to ~48 with ADR-0005 (remaining budget
~152/200).

Three observations forced a re-evaluation.

**Observation 1 — 17,435-work sweep (ADR-0007) surfaced systemic leaks, not
isolated bugs.** 580 files (3.3%) leak `［＃` markers through rendered HTML;
all cluster into 4 root causes: annotations nested inside ruby spans
(opaque-text extraction), bracket bodies that contain further `［＃` (naive
`find(']')`), Chinese-reading marks (`［＃二］`) between kanji in ruby
bases, and forward-ref bouten greedy over multi-quote bodies. Each is
addressable locally, but the common shape — "custom syntax embedded
inside another custom construct that was extracted as opaque text" — is
structural, not local.

**Observation 2 — hook-based extension has fundamental coupling cost.**
Every new construct requires another trigger character, another recogniser,
another dispatch tree branch. Every `upstream comrak` minor bump means
re-verifying that the dispatch points still exist and still fire at the
right time. ADR-0005's block-start hook would add *another* integration
seam. The 200-line diff budget (ADR-0001) was being burned one hook at a
time. The hook approach doesn't just cost lines — it couples afm's logic
to comrak's internal parse order.

**Observation 3 — Aozora notation is describable as a pure pre-pass.** Every
Aozora construct has clear syntactic delimiters (`｜base《reading》`,
`［＃...］`, `〔e^〕`) and, once parsed, the construct collapses to a
single logical unit (a ruby, a bouten, an annotation). There is no
"Aozora construct that requires CommonMark semantics to classify." The
two grammars are orthogonal.

These three observations together imply: Aozora parsing *wants* to happen
entirely before comrak sees the text, and the only thing comrak truly
needs to know is "here's an Aozora node at position P" at render time.

Earlier architectural sketches (plan iterations) considered:

1. **Keep hooks, pre-compute results for O(log N) lookup at hook time.**
   Moves parsing earlier but keeps parser hooks. Still ~33 upstream
   lines; still couples to comrak's inline dispatch order.
2. **Keep hooks, make them lookup-only; add block hook for ADR-0005.**
   Same as #1 but adds ~15 more upstream lines for block containers.
   Total ~48/200.

Both sketched designs left 2–3 parser hooks in comrak. User's directive
was explicit: "hook や lookup は少なければ少ないほどいい。純関数的アプローチが良い"
— fewer hooks, pure-functional approach. This ADR adopts the stricter
form: zero parser hooks.

## Decision

Invert the parse/extension relationship. afm runs a pure-functional
pre-pass (the lexer) that extracts all Aozora constructs into a side
registry, replaces them in the source with Private Use Area (PUA)
sentinel characters, and hands the normalized text to comrak as pure
CommonMark + GFM. Post-comrak, a pure-functional AST walk substitutes
sentinels with the pre-classified Aozora nodes.

### Pipeline

```
 source bytes
  │
  ▼ afm-encoding::decode (if SJIS)
  │
  ▼ afm-lexer::lex  (pure function)
  │    Phase 0 sanitize: BOM / CRLF / PUA collision pre-scan
  │    Phase 1 events:   linear trigger tokenization
  │    Phase 2 pair:     balanced-stack pairing across all delimiters
  │    Phase 3 classify: full-spec Aozora classification → AozoraNode
  │    Phase 4 normalize: text with PUA sentinels + SourceMap
  │    Phase 5 registry: sorted (pos → node) tables for O(log N) lookup
  │    Phase 6 validate: 4 invariants (V1-V4)
  │
  ▼ LexOutput { normalized, registry, sources, diagnostics }
  │
  ▼ comrak::parse_document(&normalized, &opts)   [NO HOOKS]
  │    PUA sentinels pass through as opaque text characters.
  │    Block-level sentinels (single-char lines) parse as one-char paragraphs.
  │
  ▼ comrak AST (standard CommonMark tree, PUA chars in text/paragraph nodes)
  │
  ▼ afm-parser::post_process  (pure function)
  │    Inline pass: split text nodes at sentinel positions;
  │                 insert NodeValue::Aozora(registry.lookup(pos)).
  │    Block pass:  collapse single-char sentinel paragraphs;
  │                 pair Open/Close sentinels; wrap intervening
  │                 siblings as children of the container node.
  │
  ▼ final AST (comrak tree with NodeValue::Aozora nodes in place)
  │
  ▼ render (comrak walk; Aozora arm dispatches to function pointer)
  │
  ▼ HTML
```

### Sentinel scheme

| Sentinel | Role |
|---|---|
| `U+E001` | Inline Aozora placeholder (ruby, bouten, annotation, gaiji, tcy, kaeriten) |
| `U+E002` | Block-leaf line (page break, section break, leaf indent, sashie) |
| `U+E003` | Block-open line (paired container start) |
| `U+E004` | Block-close line (paired container end) |

The lexer's Phase 0 pre-scans source for any PUA usage in `U+E000..U+F8FF`.
Any occurrence emits `Diagnostic::SourceContainsPua`; sentinels shift to
Unicode noncharacters `U+FDD0..U+FDD4` as a collision-free fallback (these
ranges are reserved by Unicode for internal application use and are never
assigned).

### Minimal upstream diff

Parser hooks go to zero. Render dispatch remains, rewritten as a function
pointer rather than a trait object. Total target: ~18 lines
(vs ~33 current, vs projected ~48 for the ADR-0005 path).

| Upstream site | Current | Target | Delta |
|---|---|---|---|
| `NodeValue::Aozora(Box<AozoraNode>)` variant + trait arms | ~7 | ~7 | 0 |
| Render dispatch arm (`html.rs`) | ~5 | ~5 | 0 |
| `Options.render_aozora: Option<fn(…)>` | 0 | ~3 | +3 |
| `xml.rs` / `cm.rs` no-op arms | ~3 | ~3 | 0 |
| Inline dispatch + trigger scanner | ~15 | 0 | **-15** |
| `ExtensionOptions.aozora` Arc field | ~3 | 0 | **-3** |
| Block dispatch (projected ADR-0005) | 0 | 0 | **-15 (never added)** |
| **Total** | **~33** | **~18** | **-15 actual, -30 vs ADR-0005 path** |

The `AozoraExtension` trait (with `try_parse_inline` / `try_start_block`)
is deleted. The only remaining extension point is the render function
pointer, which carries no state and is trivially replaceable at Options
construction.

### Post-process is a pure function

```rust
pub fn splice_aozora<'a>(
    arena: &'a Arena<'a>,
    root: &'a AstNode<'a>,
    registry: &PlaceholderRegistry,
) -> &'a AstNode<'a>
```

Deterministic: same (AST, registry) → same output. The inline pass is a
pre-order walk that splits text nodes at sentinel positions; the block
pass is a linear sibling scan that identifies single-char sentinel
paragraphs and collapses Open/Close pairs into container subtrees. Both
passes rely only on the registry and the arena; no global state, no
external I/O.

### Lexer is a pure function

```rust
pub fn lex(source: &str) -> LexOutput
```

Each of the 7 phases is itself `fn(input_i) -> input_{i+1}` with no
mutable shared state between phases. This enables:

- Independent unit testing per phase.
- Property-based fuzzing of invariants (sorted registry, non-overlapping
  spans, bidirectional source map, determinism).
- Future parallelization (Phase 1 chunk-parallel on line boundaries;
  Phase 3 per-span parallel).
- Future incremental re-parse (edit a substring, re-lex only the affected
  chunk).

### AST schema migration (Content type)

Ruby/Bouten/Warichu/AozoraHeading/TateChuYoko/Sashie.caption all migrate
their flat `Box<str>` body fields to a new `Content` type:

```rust
#[non_exhaustive]
pub enum Content {
    Plain(Box<str>),           // 99%+ fast path
    Segments(Box<[Segment]>),  // embedded Aozora constructs
}
#[non_exhaustive]
pub enum Segment {
    Text(Box<str>),
    Gaiji(Gaiji),
    Annotation(Annotation),
}
```

This lets nested Aozora constructs (e.g. gaiji marker inside a ruby
reading, annotation inside a bouten target) round-trip through the AST
without information loss — the root cause fix for sweep leak classes R1
and R3.

A new `AozoraNode::Kaeriten` variant captures Chinese-reading order marks
(`［＃一］`, `［＃レ］`, etc.) as their own node rather than an
`Annotation{Unknown}` — semantically precise and matches spec intent.

## Consequences

**Becomes easier:**

- Upstream comrak version bumps: ~18 lines to re-apply, almost all trait
  arms. The diff budget essentially stops shrinking.
- Testing parser logic: the lexer is a pure function with no comrak
  dependency. Unit test with literal strings, property-test with arbitrary
  generators. No arena, no options, no extension setup.
- Swapping the CommonMark backend: if we ever move from comrak to
  pulldown-cmark or similar, only the render dispatch needs reintegration.
  Parser logic and AST structure are upstream-agnostic.
- Incremental features: new Aozora constructs are a classifier entry in
  Phase 3, not a new hook + trigger character + upstream dispatch.
- Round-trip / LSP / incremental re-parse (future): the
  `(normalized, registry, SourceMap)` triple is the perfect IR for all three.

**Becomes harder:**

- PUA discipline: sentinel choice must not collide with source content.
  Mitigation is a Phase 0 pre-scan + a noncharacter fallback; both are
  cheap and structural. But it's an invariant that didn't exist before.
- Post-process bugs can corrupt the AST in ways comrak alone never
  would. Mitigation: phase 6 invariant checks (in lexer output) plus
  post_process property tests (sentinel-free post-state, container
  children count matches registry). The surgery is shallow (split text
  nodes, pair sibling paragraphs) but it is surgery.
- Block-level layout where CommonMark containers (list items, blockquotes)
  intervene between Open and Close sentinels needs careful handling:
  splice must stop at the first non-sibling ancestor rather than walking
  through comrak containers. Tested explicitly in Phase D4.

**Non-consequences:**

- Existing tests survive almost unchanged: corpus sweep is invariant-based
  (shape-agnostic), golden 56656 tests HTML output, spec fixtures compare
  HTML. Only `property_ruby` and `golden_56656::tier_a_ruby_recognition_floor`
  need small rewrites for the Content migration.
- The Aozora AST node enum (`AozoraNode`) gains a new variant (`Kaeriten`)
  and all content-bearing nodes swap to `Content`, but the enum structure
  itself is unchanged.

## Alternatives considered

**A) Keep hook-based extension; make hooks O(1) lookups.** The lexer would
still run first and build a SpanMap; `try_parse_inline` and
`try_start_block` would become pure lookup. Upstream diff stays at ~33–48
lines. *Rejected:* still couples to comrak's dispatch timing and still
costs ~15 lines per future construct family (if we add more character
triggers). User's directive is unambiguous: fewer hooks is strictly
better.

**B) Keep upstream comrak's NodeValue untouched; render via raw HTML.**
Emit Aozora constructs as raw `<span>`/`<div>` HTML blocks during
post-process so comrak's existing HTML passthrough handles them.
*Rejected:* loses structured AST fidelity. Round-trip serialization
(planned task #30) becomes parse-HTML-back-into-tree, which defeats the
purpose. Also makes accessibility / semantic HTML harder because
rendering decisions need to be made in post-process before comrak sees
them.

**C) Fork comrak harder: add a first-class "placeholder" node type with
opaque `Box<dyn Any>` payload.** Pushes the variant cost onto upstream
(one variant for the framework, not per-extension). *Rejected:*
violates the "minimal upstream diff" principle even more than the
current situation. Opaque Any erases type information; debugging is
hostile.

**D) ADR-0005's paired-container hook (block dispatch addition).** The
previous in-progress design. *Rejected as superseded:* the block splice
in post_process achieves the same paired-container semantics without
adding upstream dispatch. ADR-0005 assumed option B from its alternatives
section ("post-process the AST") was unworkable because comrak's
container parsing would have already split the inner content. This ADR
dodges that concern: by pre-labeling the paired-block markers as
single-char sentinel LINES, comrak parses each as an isolated Paragraph.
The content between is still comrak-parsed naturally; post-process only
pairs the marker paragraphs and moves the siblings between them, no
reconstruction of comrak's block logic needed.

## References

- ADR-0001 — fork / vendor strategy + 200-line diff budget; the forcing
  function that makes zero-hook attractive.
- ADR-0003 — afm-parser architecture; designed the `AozoraExtension`
  trait that this ADR deletes.
- ADR-0005 — paired block annotation container hook; superseded by this
  ADR. The paired-container semantics move from block dispatch into
  post_process.
- ADR-0006 — lint profile; the workspace-lints scope discipline extends
  naturally to the new `afm-lexer` crate.
- ADR-0007 — corpus sweep strategy; provided the 17k-work empirical
  evidence that exposed the structural leak classes and motivated this
  ADR.
- Plan file `~/.claude/plans/compiled-discovering-scroll.md` — 24-commit
  migration sequence (phases A–G).
- Memory: `feedback_pre_process_over_hooks.md` — the pattern generalized
  for future parser-wrapping projects.
