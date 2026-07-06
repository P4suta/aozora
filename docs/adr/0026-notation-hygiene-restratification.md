# 0026. Notation-hygiene re-stratification: Tier1 purity and the render-only Tier2

- Status: accepted
- Date: 2026-07-07
- Deciders: @P4suta
- Tags: architecture, notation, linter, formatter, renderer

## Context

[ADR-0022](./0022-notation-hygiene-layer-roles.md) split the notation-hygiene
work into four roles over one catalogue, and reserved a **Tier2** degraded-form
matcher — "looser heuristics… invoked only by the opt-in renderer, never by the
parser, the default lint, or the default `fmt`" — behind a placeholder: *"It
would need its own ADR."* This is that ADR.

The #414 render campaign produced an occurrence-ranked worklist of the shapes
the parser keeps `Unknown` (`corpus/render-digest.json`'s `unknown_shapes_top`).
Two questions followed: what belongs in a Tier2 matcher, and — the sharper one,
raised in review — *is Tier2 empty because degraded forms do not occur, or
because the zero-false-positive Tier1 map and the lossless Core parser quietly
absorbed them first?* Two audits answered it:

1. **Tier1 was ~90% genuine zero-FP, but four rule-families had sedimented in**
   that are **lossy or judgment-laden**, not verified spelling near-misses:
   - `中文字、ゴシック体 → 中文字、太字` — **lossy**, and self-admitted. The parser
     *deliberately* keeps this `Unknown` to preserve the ゴシック体 spelling (only
     `、太字` is a recognised line weight; `crates/aozora-pipeline/.../classify/directive.rs`),
     and Tier1 immediately overrode that choice — laundering a spelling, the
     exact thing ADR-0022 assigns the parser to refuse. The *forward* form
     `「X」ゴシック体` (Tier1 entry) meanwhile **keeps** ゴシック体: the catalogue
     preserved it in one place and laundered it in another.
   - `ここから最後まで{N}字下げ → ここから{N}字下げ` — **lossy**: `最後まで` marks an
     indent that auto-closes at document end; the reduction erases that scope.
   - `地付き、地より{N}字アキ / 字あき → 地から{N}字上げ` and
     `行末から{N}字上で地付き → 地から{N}字上げ` — **judgment**: they fold two
     measurement vocabularies (gap-from-edge vs. raised-from-edge) via a
     typographic identity, not a spelling repair.
2. The **structural reason** these could hide: the `lint_catalogue` self-test is
   purely *syntactic* — it pins "variant parses to `Unknown`, canonical parses
   to recognised, rewrite is idempotent" but never "the canonical *means the
   same* as the variant." The invariant it guarded was a **recognition**
   invariant, not a **meaning-preservation** one, so a lossy reduction passed
   every test while discarding meaning.

(A parallel audit found the Core parser absorbs a further set of lossy
convention forms on the *default* path — `ゴシック体`/`枠囲み`/`横一列` spelling
folds, dropped styling axes, a bare `［＃縦中横］` range that contradicts the
handbook's own `tcy` page. Fixing those **changes default parser output** —
corpus/render-gate/spec-vector re-baselines and downstream coordination — so it
is tracked as its own campaign, not folded in here.)

## Decision

**Re-stratify: Tier1 is purified to genuine zero-FP; the migrated lossy /
judgment reductions become Tier2, opt-in and render-only.** Default parser
output is unchanged — these bodies were, and remain, `Unknown` by default.

1. **Tier2 is reduction-only and render/interpreter-scoped.** A separate
   `aozora_syntax::degraded::degraded_directive` catalogue — never merged into
   the Tier1 fixed map (`aozora_syntax::lint::canonical_directive`), never called
   by the parser, the default lint, the default `fmt`, or `fmt --fix-notation`.
   Each entry returns a **directly parser-recognised** spelling (the opt-in
   renderer does a single serialize→lex pass, so a Tier1-*key* output would
   re-lex to `Unknown`), and is disjoint from Tier1 and idempotent.

2. **The four sedimented families move from Tier1 to Tier2.** `canonical_directive`
   now returns `None` for them; `degraded_directive` reduces them. `fmt
   --fix-notation` and the default lint consequently stop rewriting / flagging
   them — an improvement, since those were the lossy source rewrites.

3. **Containment is a type-level invariant.** The shared `bool fix_notation` /
   `bool normalize_directives` flags are replaced by one enum
   `DirectiveNormalization { Off, Canonical, Degraded }`. `Degraded` — the only
   level that consults Tier2 — is constructed at exactly **one** ephemeral site
   (`render_html_normalized`, a throwaway buffer that is lexed and discarded).
   Every persistent-write path (`fmt --fix-notation`, `to_source_with`,
   `write_back`) can hold at most `Canonical`. Therefore a Tier2 misfire can
   reach only `render --degraded` output; it can never rewrite source.

4. **A meaning-preservation self-test axis is added** — the structural fix.
   Beyond the existing recognition/idempotency pins, `lint_catalogue` now asserts
   that Tier1 does **not** override a spelling the parser deliberately keeps
   `Unknown` (starting with `中文字、ゴシック体`), and that every Tier2 sample is
   disjoint from Tier1, render-only, and refused for editorial / compound /
   composition bodies.

## Consequences

- Tier1's zero-false-positive claim is now true under ADR-0022's *stated intent*
  ("verified near-miss… loses no meaning, no judgment"), not merely under a
  recognition-only reading.
- `render --degraded` grows entry-by-entry under the same parse-round-trip +
  idempotency + disjointness gates as Tier1, plus a refuse-list gate for
  editorial / compound / composition bodies (which stay inert **by design**).
- A lossy reduction can no longer sit in Tier1 undetected: the meaning axis
  fails first.
- `fmt --fix-notation` and the default lint no longer touch the migrated forms;
  their lossy source rewrites are gone.
- A future opt-in degraded *advisory* lint may reuse the same `degraded`
  catalogue as a second consumer — explicitly deferred.

## Alternatives considered

- **Leave the four families in Tier1 (document only).** Rejected: it keeps lossy
  source rewrites live in `fmt --fix-notation` and leaves the zero-FP claim
  overstated.
- **Build a full Tier2 matcher with a fresh (empty) catalogue.** Rejected:
  measurement showed no *net-new* safe reduction in the corpus top-40; a
  `--degraded` flag reducing nothing is speculative machinery. The value is
  re-stratifying what already exists, not inventing new reductions.
- **Key Tier2 off the shared `fix_notation` flag.** Rejected: that is exactly the
  source-corruption path — a Tier2 reduction would then persist through
  `write_back`. The enum makes the illegal state unconstructable on write paths.
- **Fix the Core lossy absorptions here too.** Deferred: they change default
  parser output (spec/corpus/cross-repo blast) and need per-form judgment; a
  separate campaign.

## References

- [ADR-0022](./0022-notation-hygiene-layer-roles.md) — the four-role split and
  the Tier1/Tier2 boundary this ADR discharges.
- `aozora_syntax::degraded::degraded_directive` / `DEGRADED_SAMPLES` (Tier2),
  `aozora_syntax::lint::canonical_directive` (Tier1),
  `aozora_render::DirectiveNormalization` / `render_html_normalized`,
  `Document::to_html_with` / `to_source_with`, `aozora render --degraded`.
- Pinned by the `lint_catalogue` gates: `every_degraded_sample_reduces`,
  `tier1_and_tier2_are_disjoint`, `degraded_reductions_are_render_only`,
  `tier1_never_overrides_parser_spelling_preservation`,
  `degraded_refuses_editorial_and_compound`.
- #414 (the render-correctness campaign) and the Core ADR-0022-compliance
  follow-up issue.
