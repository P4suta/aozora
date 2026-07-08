# 0027. Core parser notation purification: decline non-canonical convention forms

- Status: accepted
- Date: 2026-07-07
- Deciders: @P4suta
- Tags: architecture, notation, parser, wire

## Context

[ADR-0022](./0022-notation-hygiene-layer-roles.md) fixed the layer roles: the
parser is **lossless and non-judgemental** — an unrecognised `［＃…］` body stays
`DirectiveKind::Unknown` and round-trips its raw bytes verbatim; any
reinterpretation lives in the linter (Tier1), formatter, or opt-in renderer
(Tier2), never in the default parse path.
[ADR-0026](./0026-notation-hygiene-restratification.md) re-stratified the Tier1
side but **explicitly deferred** a parallel finding: the Core parser itself was
**lossily absorbing a set of non-official convention forms on the default
path** — exactly ADR-0022's rejected "parser absorbs the variants" alternative.
Fixing those changes default parser output (a spec / corpus / wire blast), so it
was tracked as its own campaign. This ADR is that campaign.

The dispositions were decided from **corpus directive-frequency evidence** over
the 17,889-work `aozorabunko_text` mirror, and from general typesetting
principles — **not** from the spec's or handbook's permissiveness, which
tolerate more drift than a clean core should. The guiding telos: the **core
parser is an opinionated, limited, unified canonical vocabulary** — the one
clear way future authors are guided to write — while the surrounding layers
(Tier1 lint did-you-mean, `fmt --fix`, `--degraded` render) still accept
every corpus variant, so **nothing is lost**.

## Decision

Nine forms the parser previously absorbed are corrected:

**Promoted to first-class (was a lossy fold):**
- `ゴシック体` / `ゴチック` were folded to 太字. The corpus uses ゴシック体 (185
  occ / 25 works) and 太字 (205 works) in **disjoint** populations, and gothic
  is a distinct typeface family, not a bold weight. `ゴシック体` is now a
  **first-class `Gothic` weight** (own render class `aozora-goshikku`, own wire
  tag `gothic`) across every scope — forward, block range, line
  (`この行はゴシック体`), and the indent compound (`、ゴシック体`). The vanishing
  `ゴチック` (1 work) declines to Unknown with a Tier1 lint → ゴシック体.

**Declined to lossless Unknown (each served by an opt-in layer):**
- `枠囲み` / `枠囲い` (A2), `表罫囲み` / `ミシン罫囲み` (A3) — folded to `罫囲み`,
  erasing the spelling / rule-style. All corpus-rare (≤10 works); the core keeps
  only the canonical `罫囲み`. `枠囲み`/`枠囲い` gain a Tier1 lint → `罫囲み`.
- `は横一列` (A4) — a 縦中横 synonym; Tier1 lint → `は縦中横`.
- `は縦中横、行右/左小書き` (A5) — the small-script axis was **silently dropped**
  (a data-loss bug); now declines.
- multi-quote `「A」「B」は縦中横` (A6) — kept only the first target (a data-loss
  bug); now declines, mirroring emphasis's single-target rule.
- `傍点（白丸）` / `傍点◎` (A7) — marker-suffix spellings normalised to the
  canonical keyword; now decline, Tier1 lint → `白丸傍点` / `二重丸傍点`.
- heading close `中見出` (送り仮名-elided, A8) — Tier1 lint → `中見出し終わり`,
  matching how the structurally identical `字下げ` close okurigana is handled.
- bare `［＃縦中横］…［＃縦中横終わり］` range (B1) — a non-official corpus
  convention that opened a styling block, contradicting the handbook's own tcy
  page (which the doc now matches). 縦中横 has **no** paired-range form (spec
  §6.3); the bare markers stay verbatim Unknown and never open a block. The dead
  `RegionFormat::CombineUpright` / `RegionClose::CombineUpright` variants are
  removed (the forward `「X」は縦中横` leaf is unaffected).

**Consequences.**
- **Wire (`SCHEMA_VERSION` 1 → 2):** added the `gothic` weight tag; renamed the
  `lineBold` node kind to `lineGothic`; removed the `combineUprightRange`
  container tag. Downstream `features = ["wire"]` consumers (aozora-proof, afm)
  pin an immutable `aozora` version ([ADR-0017](./0017-ecosystem-dependency-pin-policy.md))
  and do not auto-follow — a pin bump is a deliberate step after the next
  release, coordinated via the release issue.
- **Spec realigned:** the official `aozora-notation-spec` §6.12 / §6.6 (gothic as
  its own construct) and §6.7 (enclosure named forms degrade, not fold) are
  updated, and the `bold_forward_gothic_no_referent` / `keigakomi_inline_framed`
  vectors re-vendored. The A4–A8 forms needed **no** spec prose change — their
  ABNF already admitted only the strict form and said the rest "degrades §6.14";
  the parser is merely brought into line with the spec it already had.
- **Corpus:** `unknown_total` rises 2374 → 3794 (a deliberate raise, recorded in
  `corpus/baseline.json`); render-correctness stays 0/0/0 and panic count 0.

## Alternatives considered

- **Keep the folds (respect the spec's permissiveness).** Rejected: the spec
  §6.12/§6.7 folds were written to rationalise the current lossy parser; the
  corpus and typesetting first principles say gothic ≠ bold and 表罫 ≠ 罫, so the
  spec follows the purified core, not the reverse.
- **Fold `ゴシック体` to Tier2-degraded 太字 (as the Tier1 restratification did).**
  Rejected: the corpus shows ゴシック体 is a common, distinct construct, so
  rendering it as bold is a meaning change, not a faithful degrade. Promoting it
  to first-class gothic is lossless and unifies every scope.
- **Add named `EnclosureKind` members for 枠 / 表罫 / ミシン罫.** Rejected: each
  is 1–2 works; a first-class wire enum member for corpus-vanishing styles
  bloats the core for no benefit when a lossless Unknown round-trip plus a Tier1
  lint already serves them.

## References

- [ADR-0022](./0022-notation-hygiene-layer-roles.md) (layer roles),
  [ADR-0026](./0026-notation-hygiene-restratification.md) (deferred this work at
  its §Context and Alternatives).
- Issue #435. Evidence: `aozora-pipeline/src/lexer/classify/{directive,forward}.rs`,
  `aozora-syntax/src/{format,lint,degraded}.rs`, the `lint_catalogue`
  self-test, and the `aozorabunko_text` corpus directive-frequency audit.
