# 0007. Corpus sweep strategy

- Status: accepted
- Date: 2026-04-24
- Deciders: @P4suta
- Tags: testing, parser, aozora-compat, workspace

## Context

M1 proved the parser against hand-written spec fixtures
(`spec/aozora/cases/*.json`) and one golden work
(`spec/aozora/fixtures/56656/` — 罪と罰 米川正夫訳). Those test shapes are
complementary:

- **Spec cases** exercise individual recognisers with small, targeted
  input/output pairs. Exhaustive within their narrow scope but unable
  to catch emergent interactions.
- **Golden fixture** drives the full pipeline end-to-end on one real
  1.35 MB SJIS text with ~2229 ruby readings, ~489 bracket
  annotations, forward-reference bouten, JIS X 0213 gaiji, accent
  decomposition, …. Exhaustive within one work but does not surface
  behaviour that only a different work would provoke.

Neither addresses the third axis: **behavioural stability across a
large volume of real aozora-format text**. Even with every recogniser
unit-tested and 罪と罰 passing verbatim, the parser is still unproven
against the thousands of other works the Aozora Bunko catalogue
contains. In particular:

- Edge cases in annotation composition that the 罪と罰 fixture happens
  not to exercise.
- Malformed or idiosyncratic files (incomplete annotation markers,
  encoding drift, historical typographic variants) that real-world
  corpora contain.
- Panics / unexpected errors triggered only by certain text lengths,
  nesting depths, or character combinations.

An earlier design iteration proposed `corpus.lock` — a TOML file
pinning a specific upstream mirror (`aozorahack/aozorabunko_text`) to
a specific commit with 120 works chosen by hand, each SHA256-verified.
Through planning discussion the user rejected that approach as
overfit:

> 実際の本文を使うとはいえ、そこで行うテストっていろいろあると思うんだよね。
> …別にこれが正しいやり方でもないし、今後もこのテキストを使って開発者は
> 開発を進めてもらいたいなんてこともない。…どういうテストでも都合よく
> 使えるようにいい感じの実装を用意してあると助かるよねえっていう。
> 別に青空文庫のテキストでさえあればよくて、本物であるかとか最新に追従
> しているかとかどうでもいいじゃん。それこそlockfileとかうーん、なんか
> ずれてる気が。

The constraint is: the parser should be stress-testable against *any*
representative body of aozora-format text. The specific content must
not be load-bearing. Developers bring what they have; the harness
checks invariants that hold regardless of content.

## Decision

Split the "large volume of real text" coverage into two concerns,
handled by two different shapes of test:

1. **Golden** — small, hand-curated, in-tree, exact-diff.
   `spec/aozora/fixtures/<card-id>/` with `input.sjis.txt` /
   `input.utf8.txt` / `golden.html`, driven by per-fixture integration
   tests (`tests/golden_<id>.rs`). One work per directory. Each
   addition is a deliberate decision that the work represents a
   pattern worth regression-guarding.

2. **Sweep** — trait-abstracted, externally-rooted, property-based.
   A `trait CorpusSource` in the `afm-corpus` crate yields metadata-
   free `CorpusItem { label, bytes }` values. A single sweep test
   (`crates/afm-parser/tests/corpus_sweep.rs`) consumes the trait via
   `from_env()`, iterates whatever the developer has pointed at, and
   checks invariants.

The sweep harness does NOT pin, validate, or fetch any specific
corpus. The developer provides input via `AFM_CORPUS_ROOT`. If it's
unset, the test runtime-skips (passes with a diagnostic) so CI jobs
without a corpus configured stay green.

### Invariants

Each sweep iteration runs the following checks:

| # | Check | Gate | Status |
|---|---|---|---|
| I1 | No panic on any input | Hard (assertion fails the test) | Enforced |
| I2 | No unconsumed `［＃` in rendered HTML | Soft (report-only) | Pending M1 D (paired container) |
| I3 | `parse → serialize → parse` AST equality | Hard | Pending M2-S5/S6 (serializer) |
| I4 | Generated HTML parses via html5ever | Soft → Hard | Pending M2-S4 |
| I5 | SJIS decode stable | Soft (report-only) | Enforced diagnostic |
| I6 | No lexer PUA sentinel (U+E001–U+E004) in HTML | Hard with budget | Enforced with budget 1 |
| I7 | Every `afm-*` class token is in [`AFM_CLASSES`] | Hard with budget | Enforced with budget 0 |
| I8 | No `<script` / `javascript:` / `on<event>=` markers | Hard with budget | Enforced with budget 0 |
| I9 | `strip_annotation_wrappers` is idempotent, wrappers carry `hidden`, no nesting | Hard with budget | Enforced with budget 0 |
| I10 | `<h1>`–`<h6>` bodies free of `afm-indent` / `afm-container-indent` / `afm-annotation` | Hard with budget | Enforced with budget 0 |

Report-only invariants surface counts and sample labels in the test
output but do not fail the test. This is a deliberate ratchet: new
recognisers land → fewer leaked markers → eventually the I2 count
reaches zero → we flip the gate to hard. The same pattern will be
used for I4 once html5ever reports zero errors across the sweep.

### Source implementations

Four `CorpusSource` impls live in `afm-corpus/src/`:

- **`InMemoryCorpus`** — explicit `Vec<CorpusItem>` or `(label, bytes)`
  pairs, for unit tests with full input control.
- **`VendoredCorpus`** — scans an `spec/aozora/fixtures/`-shaped tree
  for `<card-id>/input.sjis.txt` subfiles. Used when running the sweep
  against the in-tree goldens themselves, or as the fallback when no
  external corpus is available.
- **`FilesystemCorpus`** — recursive walkdir over any directory,
  yielding every `.txt` file. Accepts any layout (the aozorahack
  mirror's `cards/*/files/*/*.txt` scheme, a flat dump, a hand-picked
  selection). `follow_links = false` avoids symlink cycles.
- No HTTP-backed impl today; adding one is a non-breaking change to
  the trait surface if it becomes necessary.

### `from_env()` policy

`fn from_env() -> Option<Box<dyn CorpusSource>>`:

- `AFM_CORPUS_ROOT` unset → `None`.
- Variable set, path not a directory → `None` (both "absent" and
  "misconfigured" collapse to the same sweep-skip signal).
- Variable set, path is a directory → `Some(FilesystemCorpus)`
  rooted there.

Never returns a construction error: from the sweep harness's point of
view, "no corpus" is a valid state, not a failure.

### Scope exclusions

What this ADR explicitly does NOT adopt:

- **No SHA256 pinning of external content.** A pinned external corpus
  locks every contributor into a specific content set and turns
  upstream churn into busywork. The whole point of the sweep is that
  content identity doesn't matter — only volume and variety.
- **No `corpus-refresh` tool.** See above. A refresh tool implies a
  canonical corpus to refresh against; we don't have one.
- **No CI job that downloads a corpus.** If sweep coverage is needed
  in CI later, a small, legally-clean, in-tree "sweep-ci" directory
  is the right shape (5–10 PD works, vendored with explicit license
  notes, *separate* from developer corpora). That's a future ADR if
  and when the need surfaces.

### Adding Tier A golden fixtures

Tier A growth is separate from this ADR but follows the policy stated
in the plan file: super-canonical PD works, license note per fixture,
annotation diversity preferred over sheer count. Today's Tier A is
罪と罰 only; candidates include こころ / 山月記 / 銀河鉄道の夜 / …
Adding each requires `spec/aozora/fixtures/<id>/` populated with
`input.sjis.txt` / `input.utf8.txt` / `golden.html` / `README.md`
(SHA256 + author/translator death years + license note) and a
per-fixture `tests/golden_<id>.rs`. No ADR update needed for each
addition — data, not policy.

## Consequences

**Becomes easier:**

- Running the parser against a large real-world corpus — point
  `AFM_CORPUS_ROOT` at any directory of aozora-format text.
- Stress-testing after parser changes — the same sweep scales from
  1 file to 50 000.
- Adding new invariants — extend the test with another check block
  against the same iteration.
- Legal clarity — Tier A alone is vendored, each with a per-fixture
  license note. Sweep corpora live outside the repo, on developer
  machines where license questions are their responsibility.

**Becomes harder:**

- Reproducible CI sweep. Because we don't pin content, a CI sweep job
  would need its own small vendored corpus separate from the
  developer experience. We defer that until a concrete need appears.
- "Why didn't the sweep catch X?" — because sweep passes are
  content-dependent: without a specific work in the developer's local
  corpus, an edge case can go unexercised. Mitigation: keep the
  invariant list honest (no silently-skipped checks), surface counts
  loudly so discrepancies between local runs stand out.

**Non-consequences:**

- Existing golden tests (`tests/golden_56656.rs`) and spec cases
  (`tests/aozora_spec.rs`) are unaffected. Sweep is strictly additive.
- `upstream/comrak/` is not touched; the sweep test lives entirely in
  workspace-owned crates.

## Alternatives considered

**A) Pinned lockfile of 120 works with SHA256.** First draft of this
design. Rejected (see Context quote) — turns one specific corpus
mirror into load-bearing infrastructure that future developers would
have to buy into. Also conflated golden-style ground-truth checks
with stress-test volume.

**B) Vendor the full corpus in-tree.** Considered briefly. Rejected
on license grounds — `aozorahack/aozorabunko_text` README explicitly
mentions "CC-licensed works alongside PD works", mixing two licensing
regimes that afm's Apache-2.0-OR-MIT LICENSE doesn't cleanly
accommodate for re-distribution. Also the 558 MB extracted size
would bloat the repo prohibitively.

**C) HTTP-fetched corpus with in-memory caching.** Makes CI
self-sufficient but introduces network dependency into tests and
still implies a canonical upstream. Rejected for the same reasons as
the lockfile, plus the additional flakiness budget of network calls.
Left as a non-breaking future addition via a new `CorpusSource` impl
if the need appears.

**D) Merge sweep into the existing `just corpus` target.** The
Justfile already has a `corpus` recipe that shells into `xtask
corpus-test` (stubbed). Rejected because the two things are
different: `corpus` was designed around the lockfile approach and the
stub remains in place for archaeological reference. Renaming it
would either break future Justfile users or leave a confusing
synonym. `just corpus-sweep` stands on its own; if the stubbed
`corpus` target gets repurposed later that's a separate change.

## References

- `crates/afm-corpus/src/lib.rs` — trait and `from_env()`.
- `crates/afm-corpus/src/{in_memory,vendored,filesystem}.rs` — impls.
- `crates/afm-parser/tests/corpus_sweep.rs` — the sole consumer today.
- `Justfile::corpus-sweep` — bind-mount + env bridge from host to
  container.
- `docs/CORPUS.md` — developer onboarding (how to set up a corpus dir,
  ZIP extraction, git-clone flow, troubleshooting).
- ADR-0001 — vendored-upstream policy that the legal/scope boundary
  here extends to data as well as code.
- ADR-0006 — lint profile scope (same "workspace boundary" idea
  applied to coverage/lint).
- Memory: `feedback_parser_corpus_property_sweep.md`.

## Amendment 2026-04-24 — I6–I10 (negative invariants)

### Rationale

The "must-never-be" coverage gap identified during 2026-04 (see plan
`github-lexical-mist.md`) argued for pinning each bad-output shape as
a predicate that runs uniformly across unit tests, proptests, the
corpus sweep, and the fuzz harness. I6–I10 extend this ADR's sweep
contract with the five shape predicates exported from
`afm_parser::test_support`:

- **I6 — no lexer PUA sentinel leak.** U+E001–U+E004 are lexer-internal
  markers that `post_process` must consume; one surviving to HTML
  indicates a paired-container pass missed an opener / closer.
- **I7 — CSS class contract.** Every `afm-*` class token emitted must
  be in the pinned `AFM_CLASSES` list (or a legitimate `afm-X-N`
  numeric-suffix form). Catches renderer additions that ship unstyled
  markup.
- **I8 — XSS markers.** Literal `<script`, `javascript:`-in-attribute,
  and `on<event>=` handlers. Caught only inside tag bodies so
  annotation-wrapper text content does not false-positive (see the
  tag-context-aware detectors in `test_support`).
- **I9 — annotation wrapper shape.** `strip_annotation_wrappers` must
  be idempotent, every wrapper must close, every wrapper must carry
  `hidden`.
- **I10 — heading integrity.** `<h1>`–`<h6>` bodies must not contain
  `afm-indent`, `afm-container-indent`, or `afm-annotation` class
  tokens. Proptest version of commit 7f5463a's fixed regression.

### Rollout protocol

The "report-only → observe → ratchet" pattern documented for I2 / I4
extends to I6–I10. Landing plan:

1. **Phase 4a** — add each invariant as report-only (stderr count, no
   assertion). Committed.
2. **Phase 4b** — run the sweep against representative corpora to
   observe baseline counts. Committed observations:
   - `spec/aozora/fixtures/` (in-tree vendored, 56656 SJIS + UTF-8):
     - I6: **1 leak** in 56656 SJIS — U+E003 (BLOCK_OPEN_SENTINEL)
       survives to a bare `<p>\u{E003}</p>` paragraph around offset
       1.6 MB in the rendered HTML. The post-process paired-container
       pass is missing a close-side pairing for at least one container
       shape in the 罪と罰 source. Tracked as a follow-up bug; initial
       budget accommodates the known leak.
     - I7–I10: **0** occurrences.
   - No external `AFM_CORPUS_ROOT` corpus was measured during this
     landing; developers with a 17 k-work corpus should re-run the
     sweep and ratchet the budgets to the observed values before
     merging their own follow-up PRs.
3. **Phase 4c** — promote to hard gate with `AFM_CORPUS_I6_BUDGET` =
   1, I7–I10 = 0 by default. Same `AFM_CORPUS_*_BUDGET` env-var
   override convention as I2 / I3 / I4.

The budget is an upper bound, not a floor — ratcheting down to zero
is the long-term goal as each latent bug is fixed.

### Predicate source of truth

Every I6–I10 predicate is implemented in
`crates/afm-parser/src/test_support.rs` (the [Invariant catalog]
table in the module docs). The same predicates back the
`property_html_shape.rs` / `property_xss_prevention.rs` /
`property_heading_integrity.rs` / `property_fixpoint.rs` property
tests — so the corpus sweep and the proptest suite assert identical
contracts from different angles.
