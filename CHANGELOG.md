# Changelog

All notable changes to aozora are recorded in
this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-07-21


### Added

- Unify document API and distribution contracts ([#581](https://github.com/P4suta/aozora/pull/581))
- **examples**: Promote the two recipes worth keeping ([#546](https://github.com/P4suta/aozora/pull/546))
- **syntax**: Serve tcy small-script compound ([#467](https://github.com/P4suta/aozora/pull/467))
- Notation-hygiene layers to release quality ([#453](https://github.com/P4suta/aozora/pull/453))
- **parser**: Purify Core notation vocabulary ([#437](https://github.com/P4suta/aozora/pull/437))
- **render**: Opt-in normalize-directives render ([#426](https://github.com/P4suta/aozora/pull/426))
- Render forward emphasis on a ruby base ([#384](https://github.com/P4suta/aozora/pull/384)) ([#390](https://github.com/P4suta/aozora/pull/390))
- Aozora fmt --fix-notation autofix ([#373](https://github.com/P4suta/aozora/pull/373))
- Notation-hygiene lint for near-miss directives ([#371](https://github.com/P4suta/aozora/pull/371))
- Mid-run forward splice — case (B) non-adjacent referent ([#333](https://github.com/P4suta/aozora/pull/333)) ([#363](https://github.com/P4suta/aozora/pull/363))
- Render no-referent forward heading (E1-4) ([#346](https://github.com/P4suta/aozora/pull/346))
- Render no-referent forward bouten ([#341](https://github.com/P4suta/aozora/pull/341))
- Render no-referent forward emphasis (E1-2) ([#340](https://github.com/P4suta/aozora/pull/340))
- `ForwardOrigin::SelfContained` plumbing ([#339](https://github.com/P4suta/aozora/pull/339))
- **#237**: B'3 retire SegmentedParse, single engine (#280)
- **#237**: B'3 wire owned splice into LSP (#278)
- **#237**: B'2 owned-table incremental splice (#276)
- **#237**: B'2 norm-offset mapping helper (#275)
- **#237**: B'2 minimal balanced re-lex region (#273)
- **#237**: B'1 ParseCache holds OwnedLexOutput (#272)
- **#237**: Owned-only AST, delete borrowed/arena (#271)
- **#237**: Flip public Tree API to owned AST (#269)
- **#237**: Owned read accessors + re-exports (#266)
- **#237**: Native owned lex fold + perf gate (#264)
- **#237**: Owned HTML renderer + byte gate (#262)
- **#237**: Owned to_source + byte-identity gate (#261)
- **#237**: A2 — incremental reparse via the segment cache (#257)
- **#237**: A1 — segment-cache foundation (Diagnostic::shifted + SegmentedParse + corpus gate) (#255)
- Diagnostics_text bindings + ci-timings ([#87](https://github.com/P4suta/aozora/pull/87)) ([#245](https://github.com/P4suta/aozora/pull/245))
- Compound 字下げ modifiers + 本文終わり/改行 leaf nodes ([#78](https://github.com/P4suta/aozora/pull/78)) ([#239](https://github.com/P4suta/aozora/pull/239))
- Coupled forward/heading/margin splice ([#202](https://github.com/P4suta/aozora/pull/202)) ([#236](https://github.com/P4suta/aozora/pull/236))
- Terminal splice model + container splice ([#202](https://github.com/P4suta/aozora/pull/202)) ([#235](https://github.com/P4suta/aozora/pull/235))
- Source-region splice foundation ([#234](https://github.com/P4suta/aozora/pull/234))
- Lowering pass + canonical ruby/gaiji forms ([#214](https://github.com/P4suta/aozora/pull/214))
- Coremodel purification (I5–I12) ([#207](https://github.com/P4suta/aozora/pull/207))
- **pipeline**: Compound indent line-layout ([#187](https://github.com/P4suta/aozora/pull/187))
- **cli**: Overhaul the CLI developer experience ([#156](https://github.com/P4suta/aozora/pull/156))
- **notation**: Corpus-grounded conformance — audit harness + coverage ([#114](https://github.com/P4suta/aozora/pull/114))
- **notation**: Implement all corpus-attested §6 families + de-circularise the conformance suite ([#104](https://github.com/P4suta/aozora/pull/104))
- DevEx inner-loop — §6 families, diagnostics, spec gate, polyglot bindings, AngleQuote
- **xtask**: Comment discipline + coordinate gate ([#561](https://github.com/P4suta/aozora/pull/561))
- **cli**: Add `aozora lsp` umbrella (exec-delegate) ([#523](https://github.com/P4suta/aozora/pull/523))
- **fmt**: Progress bar + batch summary for directory fmt ([#521](https://github.com/P4suta/aozora/pull/521))
- **cli**: Add `aozora tui` live editor/preview ([#520](https://github.com/P4suta/aozora/pull/520))
- **cli**: Add `aozora repl` interactive shell ([#518](https://github.com/P4suta/aozora/pull/518))
- **cli**: Add `aozora init` scaffold ([#516](https://github.com/P4suta/aozora/pull/516))
- **cli**: Add `aozora doctor` user-facing self-check ([#514](https://github.com/P4suta/aozora/pull/514))
- **cli**: Explain fuzzy "did you mean" + concept targets ([#513](https://github.com/P4suta/aozora/pull/513))
- **i18n**: Aozora-i18n crate (Fluent) + --lang resolution ([#510](https://github.com/P4suta/aozora/pull/510))
- **cli**: Unify --format + normalize CLI-local JSON envelopes ([#509](https://github.com/P4suta/aozora/pull/509))
- **cli**: Add -q/-v verbosity, tracing logging, clap styles ([#508](https://github.com/P4suta/aozora/pull/508))
- **cli**: Add XDG global config layer + per-field merge ([#507](https://github.com/P4suta/aozora/pull/507))
- **conformance**: Cross-surface parity gates ([#464](https://github.com/P4suta/aozora/pull/464))
- **cli**: --color flag and stdin anti-hang guard ([#451](https://github.com/P4suta/aozora/pull/451))
- Single-line absolute font-size directive (#329 7b) ([#351](https://github.com/P4suta/aozora/pull/351))
- この行はゴシック体 line-bold marker ([#344](https://github.com/P4suta/aozora/pull/344))
- Add channel-aware build-version stamp ([#302](https://github.com/P4suta/aozora/pull/302))
- **#237**: Pandoc reads owned lex output (#267)
- **cli**: Kinds --format json ([#233](https://github.com/P4suta/aozora/pull/233))
- **xtask**: Grammar regen drift gate ([#463](https://github.com/P4suta/aozora/pull/463))
- Consolidate the editor/CLI tooling into the monorepo ([#178](https://github.com/P4suta/aozora/pull/178))


### Build

- Separate the toolchain channel from the MSRV ([#541](https://github.com/P4suta/aozora/pull/541))


### Changed

- Replace internal pipeline API with documents ([#576](https://github.com/P4suta/aozora/pull/576))
- Collapse to 3 published crates ([#573](https://github.com/P4suta/aozora/pull/573))
- Apply the comment discipline tree-wide ([#562](https://github.com/P4suta/aozora/pull/562))
- Move the contributor docs to docs/ ([#548](https://github.com/P4suta/aozora/pull/548))
- Move the generated artefacts out of the handbook ([#547](https://github.com/P4suta/aozora/pull/547))
- **cli**: Group introspection under `aozora spec` + help groups ([#522](https://github.com/P4suta/aozora/pull/522))
- **api**: Curate umbrella re-exports ([#474](https://github.com/P4suta/aozora/pull/474))
- **api**: Drop node_at_normalized shim ([#458](https://github.com/P4suta/aozora/pull/458))
- **render**: Move lossy Tier1 forms to Tier2 ([#434](https://github.com/P4suta/aozora/pull/434))
- Drop dead owned naming across crates ([#432](https://github.com/P4suta/aozora/pull/432))
- Parametrize Framed by EnclosureKind ([#352](https://github.com/P4suta/aozora/pull/352))
- Rename incremental module, fix doc-rot ([#315](https://github.com/P4suta/aozora/pull/315))
- Drop *Wire suffix (finish wire→json) ([#177](https://github.com/P4suta/aozora/pull/177))
- Demote node_at_normalized + add ADR-0015 ([#169](https://github.com/P4suta/aozora/pull/169))
- **json**: Expose wire structs + entries ([#168](https://github.com/P4suta/aozora/pull/168))
- Single-authority wire tags ([#165](https://github.com/P4suta/aozora/pull/165))
- Overhaul public & internal API naming
- Resolve the three findings deferred in #560 ([#563](https://github.com/P4suta/aozora/pull/563))
- Retire the handbook ([#549](https://github.com/P4suta/aozora/pull/549))
- **spec**: Migrate diagnostic prose to aozora-i18n (en/ja/zh) ([#511](https://github.com/P4suta/aozora/pull/511))
- Remove dead Framed & InvalidRubySpan core surfaces ([#457](https://github.com/P4suta/aozora/pull/457))
- **diagnostics**: Unify onto aozora-spec ([#450](https://github.com/P4suta/aozora/pull/450))
- **cli**: Fmt delegates to aozora-fmt core ([#409](https://github.com/P4suta/aozora/pull/409))


### Chore

- Crate publish DX (docs.rs meta, READMEs) ([#452](https://github.com/P4suta/aozora/pull/452))
- Adopt taplo for repo-wide TOML formatting ([#308](https://github.com/P4suta/aozora/pull/308))
- **crates**: Correct 4 crate descriptions ([#428](https://github.com/P4suta/aozora/pull/428))
- **repo**: Drop archived aozora-tools refs ([#411](https://github.com/P4suta/aozora/pull/411))


### Documentation

- Shrink the published READMEs ([#552](https://github.com/P4suta/aozora/pull/552))
- Cut the READMEs down to what a reader needs ([#543](https://github.com/P4suta/aozora/pull/543))
- **crates**: Add READMEs for publishable crates ([#469](https://github.com/P4suta/aozora/pull/469))
- Sweep borrowed-arena doc-rot to owned AST ([#364](https://github.com/P4suta/aozora/pull/364))
- Drop references to the deleted borrowed engine ([#347](https://github.com/P4suta/aozora/pull/347))
- **#237**: Sweep stale arena/borrowed-AST doc-rot (#277)
- Dependency-pin ADR + host-literal recipe ([#208](https://github.com/P4suta/aozora/pull/208))
- Integrity sweep + rot-detection gates ([#175](https://github.com/P4suta/aozora/pull/175))
- Fix cli doc-rot + list AOZORA_* env vars ([#505](https://github.com/P4suta/aozora/pull/505))
- **handbook**: Regen quickstart from real runs ([#462](https://github.com/P4suta/aozora/pull/462))


### Fixed

- **parser**: Reject open incremental regions ([#590](https://github.com/P4suta/aozora/pull/590))
- **vscode**: Delete the drift, not the symptoms ([#558](https://github.com/P4suta/aozora/pull/558))
- **incremental**: Decline region-end on ruby base ([#476](https://github.com/P4suta/aozora/pull/476))
- **json**: SchemaVersion casing in wire types and docs ([#422](https://github.com/P4suta/aozora/pull/422))
- **pair**: Bracket is a hard pairing scope (Category C sink) ([#400](https://github.com/P4suta/aozora/pull/400))
- **release**: Make package publication recoverable ([#588](https://github.com/P4suta/aozora/pull/588))
- **cli**: Repair the explain/spec kinds contract ([#553](https://github.com/P4suta/aozora/pull/553))
- Point diagnostic urls at the specification ([#544](https://github.com/P4suta/aozora/pull/544))
- Gate anchors and drop hand-counted totals ([#540](https://github.com/P4suta/aozora/pull/540))
- **cli**: Make the .aozora.toml color key effective ([#524](https://github.com/P4suta/aozora/pull/524))
- **ci**: Correct release man-page subcommand list ([#504](https://github.com/P4suta/aozora/pull/504))
- **cli**: Normalize .exe in help snapshots for cross-os ([#479](https://github.com/P4suta/aozora/pull/479))
- **cli**: Reject oversize input gracefully ([#461](https://github.com/P4suta/aozora/pull/461))
- **cli**: Exit 0 quietly on stdout broken pipe ([#460](https://github.com/P4suta/aozora/pull/460))


### Performance

- **#237**: Remove owned incremental engine (#297)
- **#237**: PieceSeq compaction, no forced re-parse (#296)
- **#237**: PieceSeq-spliced diagnostics hot path (#295)
- **#237**: PieceSeq incremental region-find base (#294)
- **#237**: Incremental sanitized rope splice (#292)
- **#237**: SanitizedSrc byte-source trait (#289)
- **#237**: Recover CRLF incremental coverage (#287)
- **#237**: O(log n) incremental region-find (#285)
- **#237**: Diagnostics-only LSP hot path + lazy tree (#283)


### Tests

- **aozora**: Kill umbrella mutation survivors + arm baseline ([#495](https://github.com/P4suta/aozora/pull/495))
- **splice**: Seal coupled edit coverage ([#202](https://github.com/P4suta/aozora/pull/202)) ([#238](https://github.com/P4suta/aozora/pull/238))
- **doc**: Make the front-door + pandoc examples executed doctests ([#96](https://github.com/P4suta/aozora/pull/96))
- **cli**: Kill mutation survivors + arm baseline (Wave6a) ([#497](https://github.com/P4suta/aozora/pull/497))
- Raise workspace coverage 74.65% → 86.20% ([#136](https://github.com/P4suta/aozora/pull/136))



### Added

- **corpus / xtask**: `aozora-xtask corpus render-audit` renders every corpus
  document to HTML and reports where aozora notation control markers (`《…》`
  ruby, `｜` ruby-base, `［＃…］` directive) survive into the *visible* text of
  the output — the signature of a notation that failed to resolve (e.g. a ruby
  that never attached to its base and leaked as literal `《…》`). Report-only
  measurement; the enforcing counterpart gate follows. Legitimate `≪…≫`
  angle-quote delimiters, empty ruby `《》`, and `hidden` directive spans are
  excluded structurally, and the standard header/footer legend is stripped so
  the audit sees only the literary body.
- **render / playground**: a canonical reference stylesheet,
  `crates/aozora-render/assets/aozora-notation.css`, is now the single source of
  truth for how every `aozora-*` class is presented (theming via `--aozora-*`
  custom properties, a `.aozora-vertical` hook for 縦書き). The playground adopts
  it instead of hand-rolling its own copy, and
  `classes::canonical_stylesheet_matches_emitted_classes` pins the sheet's
  selectors to `AOZORA_CLASSES` exactly, so notation styling can no longer drift
  from the emitted classes. See ADR-0024.
- **trace**: `aozora-xtask trace <sub> --format json` emits the typed analysis
  report (`hot` / `libs` / `rollup` / `stacks` / `compare` / `flame`) as pretty
  JSON instead of the human table — scriptable and diff-friendly. Every report
  now derives `Serialize` (camelCase fields, snake_case enum tags); `cache`
  ignores the flag. Closes the long-standing `TableRenderable` "structured
  serializable form (planned)" gap.
- **cli**: a first-class **`aozora lint`** subcommand reports only the
  authoring-hygiene diagnostics (the `aozora::lint::*` namespace, gated by a
  single `Diagnostic::is_lint` authority), and `aozora lint --fix` applies the
  zero-false-positive Tier1 autofix in place through the exact path
  `fmt --fix --write` uses. `aozora fmt` reaches full parity with the standalone
  `aozora-fmt` binary — `--diff` / `--list` / `--json` / `-E`/`--encoding` /
  multi-file — by sharing one `aozora_fmt::run_engine`, so the canonical form
  and the flag vocabulary can never drift. That vocabulary is now coherent:
  source rewrite → `--fix`, read-only projection → `--normalize` / `--degraded`,
  diagnostics → `--strict` / `--diagnostic-format`. See #453 and ADR-0022.
- **cli**: a global `--color {auto,always,never}` flag (accepted after any
  subcommand) drives colour across the whole CLI via a process-wide miette hook;
  `auto` honours `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE` and stderr-TTY
  detection. See #451.
- **notation**: serve the single-target 縦中横 + 行小書き compound
  (`「X」は縦中横、行右／左小書き`) — corpus residue form 5 (~886 occurrences) —
  as a Tier2 `--degraded` (render-only) mapping that renders the primary 縦中横
  axis faithfully while dropping the secondary small-script axis (ADR-0027 A5).
  The **default parse path is unchanged** — the compound still declines to a
  lossless `Unknown`, so byte output is invariant. See #467.
- **playground**: the footer now shows the parser build identity, sourced from
  an `aozora-wasm` `version()` export (`aozora-buildstamp::VERSION`,
  wasm32-scoped) rather than a hard-coded literal (ADR-0009). See #468.
- **testing**: mutation-testing (assertion-strength) tooling — `just mutants`
  drives [cargo-mutants](https://mutants.rs/) over the workspace in a dedicated
  incremental target dir, configured by a root `mutants.toml` (nextest runner;
  bench / fuzz excluded). It measures what region coverage cannot: whether a
  *wrong* result would actually fail a test. Report-only today, introduced
  report → reinforce → ratchet. See ADR-0031.
- **ci / testing**: mutation testing is now wired into CI as an
  assertion-strength ratchet (ADR-0031 stage 3). A PR-scoped `mutants-in-diff`
  job runs `cargo mutants --in-diff` over each pull request's changed lines —
  advisory, so a surviving mutant nudges review without blocking merge — and a
  scheduled weekly `mutants` workflow sweeps `aozora-spec` (ratcheted against a
  committed `mutants-baseline.json`, opening a tracking issue if survivors rise
  above the baseline) and `aozora-syntax` (report-only until reinforced),
  uploading the full report as an artifact. Local mirror:
  `just mutants -p <crate>`.
- **testing**: `aozora-syntax` mutation-hardening (ADR-0031 stage "reinforce").
  A full sweep of the property-test-free notation crate surfaced 99 surviving
  mutants; 27 new assertion-strength unit tests close the gaps — the
  `ruby_base_class` / `is_ruby_base_char` character-class table (every Unicode
  range endpoint plus the deliberately-excluded small katakana), the
  `MarginNoteKind` / `HeadingKind` / `FontShift` / `RegionFormat` /
  `RegionClose` / `ForwardOrigin` / `Node` projections, the `accent.rs`
  dotted-letter and digraph-edit arithmetic, the `StrInterner` probe-table
  invariants (length-limit boundary, dedup-across-resize, `avg_probe_length`),
  and the `node_at_source` binary search. The handful of genuinely-equivalent
  mutants (a compile-time ASCII-pin guard, an allocation-capacity hint) carry an
  in-source `#[cfg_attr(test, mutants::skip)]` with a justification.

### Changed

- **cli / fmt**: `aozora fmt` now delegates to the shared
  `aozora_fmt::format_source_with` instead of re-implementing the
  `parse ∘ to_source` round-trip inline, so the `aozora` CLI and the standalone
  `aozora-fmt` binary can never drift on the canonical form (ADR-0016
  follow-up). Output is **byte-identical** — both paths already ran the same
  transform — the only observable change is that `--timing` collapses the former
  `parse` + `serialize` phases into one `format` phase. Also corrects stale
  `aozora-fmt` module docs (the `aozora` CLI was never a `FmtArgs` consumer and
  there is no `xtask gen-assets`) and drops the leftover "planned" note from the
  `--fix` non-canonical-directive help now that the autofix ships.
- **perf / pipeline**: the classifier's per-ruby synthetic event stream
  (`build_synth_ruby_view`) now lands in a `SmallVec<[_; 16]>` instead of two
  fresh heap `Vec`s. A ruby is ~5 events, so it stays inline — and since the
  ~200 ruby/file that dominate the corpus each paid two mallocs there, this was
  **~67 % of all owned-lex heap allocations**. Owned allocation pressure drops
  from **489.1 → 191.9 blocks/file (−61 %)** with byte-identical output and
  small-band throughput unchanged/slightly up (owned/borrowed 1.07). The
  `owned-alloc` ratchet baseline is lowered accordingly.
- **api**: ⚠ BREAKING (source-only) — a pre-1.0 naming overhaul locks the
  public and internal surface to industry conventions. Rust types drop the
  `Aozora` stutter: `AozoraNode` → `Node`, `AozoraTree` → `Tree`,
  `AozoraHeading*` → `Heading*`, `BorrowedLexOutput` → `LexOutput`; the
  textual-criticism variants adopt TEI terms (`AsIs` → `Sic`, `TextualNote` →
  `BaseTextVariant`). The JSON `kind` tags take a minimal-romaji policy —
  `tateChuYoko` → `combineUpright`, `keigakomi` → `framed`, `sashie` →
  `illustration`, `annotation` → `directive`, `sideNote` → `marginNote` (the
  five terms with no 1:1 English mapping — `ruby` / `bouten` / `gaiji` /
  `warichu` / `kaeriten` — stay romaji). The envelope field `schema_version`
  becomes `schemaVersion`, and `Tree::serialize` becomes `Tree::to_source`
  (resolving the `serialize`-vs-JSON-projection double meaning). The CLI
  `aozora wire <kind>` becomes `aozora inspect <kind>`, and the `wire` feature
  and module become `json`. Internally the numbered lexer phases are named
  `sanitize` / `tokenize` / `pair` / `classify` and `lex_into_arena` → `lex`;
  the arena-tuning surface (`ParseOptions::arena_capacity`,
  `Document::with_arena_capacity`, `arena_bytes`) is removed along with the
  borrowed/arena engine. Bindings follow: WASM methods are camelCase, and the
  Python package imports as `aozora` (`from_sjis` → `from_bytes`). **The emitted
  JSON changes only in the renamed `kind` tags** — source that names these
  symbols needs updating; the `*Wire` type-suffix drop below completes the
  `wire` → `json` migration.
- **api**: ⚠ BREAKING (source-only) — complete the `wire` → `json` migration by
  dropping the `*Wire` type suffix. `aozora::json::{SpanWire, DiagnosticWire,
  NodeWire, PairWire, ContainerPairWire, OffsetWire, SlugWire, ByteSpanWire,
  GaijiResolutionWire}` become `json::{Span, Diagnostic, Node, Pair,
  ContainerPair, Offset, Slug, ByteSpan, GaijiResolution}` (qualify by module —
  idiomatic Rust). The tag accessors `as_wire_tag` / `as_wire_str` (on
  `NodeKind` / `ContainerKind` / `PairKind` / `SlugFamily` / `Severity` /
  `DiagnosticSource`) become `as_json_tag` / `as_json_str`. Generated bindings
  follow: Go `wire_gen.go` → `json_gen.go` (top-level `AozoraWire` →
  `AozoraJSON`); TypeScript `WireEnvelope<T>` → `JsonEnvelope<T>`. **The emitted
  JSON is unchanged by this rename** — field names, `kind` / `severity` tag
  strings, and the `schemaVersion` value are all byte-identical across it, so
  JSON consumers are unaffected; only source that *names* these symbols needs
  updating. (The rename left `schemaVersion` untouched at `1`; the separate
  Gothic core purification below bumps it **1 → 2** for the 0.5.0 wire schema —
  see #437 — so a 0.5.0 envelope carries `schemaVersion: 2`.) Upgrading from
  0.4.x also picks up the earlier feature/module rename: Cargo feature
  `["wire"]` → `["json"]` and module path `aozora::wire::` → `aozora::json::`.
  This is a 0.x breaking change, so the next release is **0.5.0** (never
  0.4.2). See #176.
- **notation**: ⚠ BREAKING — 二重山括弧 is now the `≪…≫` (U+226A/U+226B) input
  encoding, rendered as `《…》` (U+300A/U+300B) — correcting a model that was
  inverted and misnamed. The node / wire kind `DoubleRuby` / `doubleRuby` is
  renamed `AngleQuote` / `angleQuote`; the CSS class `aozora-double-ruby` →
  `aozora-angle-quote`; the pandoc `Span` class `double-ruby` → `angle-quote`.
  A literal `《《…》》` in source is now a `nested-ruby` diagnostic with plain
  recovery. See ADR-0011.
- **parser / core**: ⚠ BREAKING — purify the Core notation vocabulary
  (ADR-0027). The classifier stops lossily absorbing nine non-official
  convention forms on the **default parse path**; each now declines to a
  lossless verbatim `Unknown` and is still served by a Tier1 lint / `--degraded`
  render, so nothing is dropped. **ゴシック体 is promoted to a first-class
  `Gothic` weight** — a typeface, disjoint from 太字 across the corpus — in every
  scope (`Format::Gothic` / `LineFormat::Gothic` / `NodeKind::LineGothic`; CSS
  class `aozora-goshikku`; wire tag `gothic`). The wire schema bumps
  **`schemaVersion` 1 → 2**: the tag `gothic` is added, `lineBold` is renamed
  `lineGothic`, and the `combineUprightRange` container tag is removed (縦中横
  has no paired-range form in the spec, so the dead `RegionFormat::CombineUpright`
  surface is dropped). The TypeScript-types generator now derives the version
  from `SCHEMA_VERSION` instead of a hardcoded `1`. `features = ["json"]`
  consumers pin an immutable version and follow at this release. Over the
  17,889-work corpus `unknown_total` rises 2,374 → 3,794 by design (documented
  in `corpus/baseline.json`), with render-correctness and panic counts still
  zero. Spec realigned in aozora-notation-spec#52. See ADR-0027 and #437.
- **api**: ⚠ BREAKING (source-only) — curate the `aozora` umbrella
  re-exports. The umbrella promised "insulation from internal decomposition" but
  its glob re-exports (`pub use aozora_pipeline::*` and the `syntax` / `render`
  / `encoding` / `cst` / `query` / `proptest` modules) instead leaked
  no-contract internal surface (e.g. the deleted `borrowed` module) verbatim.
  Every `pub use <crate>::*` glob is dropped in favour of named re-exports of
  only the documented stable surface, and `pub mod pipeline` (explicitly "not
  part of the stable surface") is withdrawn — its stable outputs (`LexOutput` /
  `NodeRef` / `SourceNode` / `lex` / `prewarm`) already live at the crate root.
  A consumer that needs a withdrawn internal surface should depend on the
  internal crate directly with a pinned version (the workspace's own consumers
  were switched this way). See #474.

### Removed

- **core / syntax**: ⚠ BREAKING (source-only) — removed two dead core enum
  surfaces from `aozora-syntax` that were declared and wired but never
  constructed from source. `DirectiveKind::InvalidRubySpan` (malformed ruby is
  handled by the diagnostics channel — an `empty-ruby-reading` / `nested-ruby`
  diagnostic plus lossless plain-text replay — never a typed directive) and
  `LineFormat::Framed` / the `NodeKind::Framed` (`"framed"`) node kind it
  projected to (the `罫囲み` line spelling is claimed by the paired container
  `RegionFormat::Framed` and the forward `「X」は罫囲み` by `ForwardAttr::Framed`,
  so the line scope had no source path — a byproduct of the symmetric per-scope
  enum design in #207, with an asymmetric round-trip). The live enclosure
  surfaces stay: `Format::Framed`, all of `EnclosureKind`, `ForwardAttr::Framed`,
  `RegionFormat::Framed`, and the `ContainerKind` `"framed"` tag. **The emitted
  JSON is byte-identical and `schemaVersion` stays 2** — the only wire-adjacent
  change is the generated TypeScript `NodeKind` union dropping its dead `"framed"`
  member, so only source that *names* that member needs updating. The golden
  family universe shrinks 45 → 43. See ADR-0028 and #455.
- **api**: ⚠ BREAKING (source-only) — remove the `#[deprecated]` +
  `#[doc(hidden)]` `Tree::node_at_normalized` shim (formerly
  `Document::node_at_normalized`); it had zero consumers across the workspace
  and sibling repositories. Normalized-offset lookups use
  `lex_output().registry.node_at()` directly, and source coordinates are served
  by `node_at_source`. A clean-break removal in the 0.5.0 breaking window. See
  #458.


### Fixed

- **diagnostics / fmt**: the "report this bug" URLs in the pipeline-internal
  diagnostic catalogue and the formatter's panic guard pointed at the archived
  `github.com/P4suta/aozora-tools` repo, so a user who hit one landed on a dead
  page. They now point at `github.com/P4suta/aozora/issues`.
- **notation / parser**: 振り仮名 (ruby) inside a `「…」` quote — the vast majority
  of dialogue ruby — no longer leaks as literal `《…》`. The classifier's
  stream-through path for `「…」` / `〔…〕` quotes swallowed every nested pair, so a
  `《reading》` (or `≪…≫`) inside dialogue never reached the ruby recogniser and
  replayed as plain text (`「駄目《だめ》」` rendered `「駄目《だめ》」` instead of a
  `<ruby>`). It now opens a sub-frame for the nested Ruby / AngleQuote and
  recognises it exactly as at top level; base detection is unchanged, so the
  explicit `｜base《reading》` form works too. Over the 17,889-work corpus this
  clears ruby leaks in 5,994 files (rendered-HTML ruby-leak rate 51.1 % →
  17.6 %; the remainder is gaiji-base ruby, fixed just below). Round-trip
  byte-identity, the round-trip fixed point, and source-region tiling all still
  hold. Regression fixtures: `ruby_in_quote`, `ruby_barred_in_quote`,
  `ruby_multi_in_quote`.
- **notation / parser**: 振り仮名 (ruby) on a **gaiji base** — the exact case from
  the original report, `瞳を※［＃「目＋爭」…］《みは》る` — no longer leaks as literal
  `《みは》`. A gaiji resolves to a glyph distinct from its `※［＃…］` source and is
  emitted as its own node, so the following ruby found no plain run to attach to
  and could not reach back to the already-yielded gaiji. `try_gaiji_emit` now
  defers the emit one step (`pending_ruby_base`); an immediately-adjacent `《…》`
  adopts the gaiji as a `Segment::Gaiji` base
  (`<ruby><span class="aozora-gaiji">睜</span><rt>みは</rt></ruby>`), and any
  other event / EOF flushes it as a standalone gaiji span. The serializer no
  longer injects a spurious `｜` before a gaiji base (it re-parses implicitly),
  keeping the round-trip a fixed point. Regression fixtures: `ruby_base_gaiji`,
  `ruby_base_gaiji_flushed`. Together with the quote-interior fix above, this
  legitimately forms ~50 more ruby nodes/file; combined with the synth-stream
  `SmallVec` change the net owned-allocation ratchet is **489 → 194 blocks/file
  (−60 %)** with byte-identity preserved (baseline updated).
- **go**: The Go host SDK's `Open` called a non-existent `schemaVersion`
  plugin export and failed with `unknown function: schemaVersion` on every
  use; the Extism plugin exports `schema_version` (snake_case, matching every
  other export). Pre-existing — it survived because no gate exercised the Go
  SDK at runtime (CodeQL only compiles it); now covered by `smoke-go`.
- **playground**: 縦中横 (tate-chu-yoko) no longer renders as stacked digits in
  vertical writing mode. The preview never set `text-combine-upright: all` on
  `.aozora-combine-upright` (the property was absent from the entire repo), and
  the hand-rolled 傍点 rules had drifted to class names the renderer never emits.
  Fixed at the root by the canonical reference stylesheet above; a Playwright
  test now asserts the computed `text-combine-upright` in vertical mode.
- **build**: `just playground-build` (any `vite build`) no longer fails with
  `EACCES` while emptying `dist` when the `playground-dist` /
  `playground-node-modules` named volumes carry root-owned files from an earlier
  root run. `_playground-fix-perms` normalises volume ownership to the compose
  runtime UID (guarded by a `find ! -uid` scan, so it is a no-op when clean)
  before every playground gate.
- **vscode**: the preview pane and HTML export now adopt the renderer's
  canonical stylesheet (ADR-0024) instead of hand-rolled CSS. This fixes two
  dead class names — `.aozora_gaiji` / `.aozora_tcy` (underscores the renderer
  never emits) — that silently broke gaiji highlighting and 縦中横, and styles
  every notation class for the first time. The preview follows the editor's
  light/dark theme rather than a hardcoded page colour. A new CI `vscode` job
  (tsc + biome + esbuild bundle + tests; `just vscode-ci` locally) closes the
  gap that let the drift land unnoticed — the extension was previously ungated.
- **pipeline**: a stray unmatched `［` (an opening bracket whose `］` never
  arrives) no longer desyncs the lexer and leaks every following valid ruby /
  heading / directive into the visible HTML as a cascade. Because a `［＃…］`
  directive body never spans a newline, the pair stage now force-resolves any
  still-open top-run of `［` as `Unclosed` before emitting a `Newline`, and the
  classifier abandons the frame and resumes normal classification on the live
  stream. Already-paired events are never re-classified (that would corrupt
  bytes), so `corpus verbatim` stays byte-identical — only the visible render
  improves (ruby-leak occurrences 8,710 → 1,588 over the 17,889-work corpus).
  Regression fixture: `stray_bracket_line_scope`. See ADR-0030 and #473.
- **cli**: `aozora render big.txt | head` (any downstream reader that closes the
  pipe early) no longer errors — the broken-pipe write is detected via
  `aozora_fmt::is_broken_pipe` and the CLI exits quietly with success
  (ripgrep / bat convention) instead of exit 1 (or exit 2 for `aozora-fmt`).
  The `fmt --list` / `--write` `println!` paths move to `writeln!(io::stdout())`
  so EPIPE no longer panics. See ADR-0029 and #460.
- **cli**: oversize input (> `u32::MAX` bytes) is now rejected gracefully with a
  usage error (exit 2) instead of reading the whole file and aborting with
  SIGABRT (exit 134) at the `u32` span-boundary assertion. A shared guarded
  reader checks `fs::metadata` size before reading a file, bounds stdin with a
  capped read, and re-checks after SJIS→UTF-8 decode; the message matches the
  existing Python / WASM guards. See #461.


### CI

- **ci**: a new `version-literal-gate` (the `book-versions` job) fails CI and
  pre-push when a `vX.Y.Z` literal appears in the handbook outside install.md,
  mechanizing ADR-0009's single-source-of-truth rule. Gated on `rust || book` so
  a handbook-only PR — the change most likely to introduce a stray pin — still
  runs it.
- **ci**: Gate the Go host SDK at runtime — add `smoke-go` (gofmt + go vet +
  go test against the freshly-built wasm) to `just ci` and the pre-push
  `ci-parallel`, alongside `smoke-ffi`. Previously CodeQL only compiled it.
- **conformance**: cross-surface parity gates — one committed golden (127
  render fixtures) is checked byte-identical by a thin walker on each surface
  (CLI, C FFI, Python, WASM, Go/Extism), so a binding can no longer reframe,
  re-order, or drop output undetected. See #464.
- **corpus / notation**: the notation-hygiene framework is populated against the
  17,889-work corpus. A Tier2 rule (D6) maps the count-less `下げて…字あきで`
  head-indent spellings to `地から N 字上げ` in `--degraded` render, a
  corpus-mined refuse-list keeps free-form spatial-layout decoys inert, and a
  `catalogue-sweep-gate` pins the Tier1/Tier2-matched Unknown-shape set (residue
  may only shrink; a newly matched shape fails until a human confirms a genuine
  near-miss). Golden family coverage reaches 41/45, the remaining 4 documented
  as structurally irreducible (two of them — the dead `framed` /
  `invalidRubySpan` surfaces — are then removed per the Removed section,
  shrinking the universe to 43). Default parser output stays byte-identical. See
  #456.
- **just**: repair the fuzz and semver gate recipes — the `fuzz-*` recipes
  built against a musl target the nightly image lacks (never running a single
  iteration) and referenced stale/renamed fuzz targets, and the semver recipe
  aborted on crates not yet on crates.io. All seven fuzz targets now build and
  run on the host gnu triple and the semver gate skips unpublished crates
  cleanly. See #459.
- **xtask**: a grammar regen drift gate (`conformance grammar --check`) fails
  when the committed tree-sitter `parser.c` diverges from its grammar, pinning
  `tree-sitter-cli` to 0.26.x to match the runtime dependency. See #463.
- **bench**: an instruction-count perf gate (`just perf-gate`) using
  iai-callgrind measures deterministic CPU instructions (`Ir`) over corpus-free
  vendored works plus a synthetic pathological buffer, failing on a > 10 %
  regression; wall-clock is too noisy on shared runners. Also lands a
  tree-sitter ERROR-free ratchet. See #466.
- **ci**: a weekly cross-OS behavioral test matrix runs the full test suite on
  `macos-latest` and `windows-2025` (the release runners), so platforms that
  were previously build-only are exercised at runtime. See #465.
- **release**: release builds are now `cargo auditable` and ship a CycloneDX
  SBOM (`aozora-<ver>.cdx.json`), attached to the GitHub Release and covered by
  build-provenance attestation. See #470.
- **ci**: a scheduled fuzz soak workflow runs all seven fuzz targets weekly
  (600 s each, persisted corpus cache), uploading crash artifacts and opening a
  deduplicated issue on failure. See #471.
- **xtask**: `spec-vectors check` / `sync` replace the two spec-vendor bash
  scripts, with a weekly spec-freshness workflow that opens an issue on drift.
  See #472.


### Documentation

- **repo**: scrub the remaining references to the archived `aozora-tools` repo
  (folded into this monorepo per ADR-0016) — crate READMEs, the top-level
  "Related projects" table, handbook `p4suta.github.io/aozora-tools/*` URLs, and
  the playground comments now point at `P4suta/aozora` and the in-repo
  `editors/vscode` / `crates/aozora-lsp`. ADR migration records are kept as-is.
- **handbook**: scrub the hand-maintained version literals (`v0.4.1` / `v0.5.0`)
  from `ref/api.md`, `bindings/python.md`, and the `contrib/release*.md` runbooks
  (link to install.md or use `vX.Y.Z` placeholders), leaving install.md the one
  canonical pin. ADR-0009's deferred grep-gate is now implemented.
- **handbook**: regenerate the CLI quickstart from real command runs — replacing
  a fabricated diagnostic example with actual `aozora check` output and
  correcting the document-operation count and exit-code table. A new
  `handbook_pins.rs` test asserts the fenced examples against the built binary.
  See #462.
- **crates**: add READMEs for the 14 publishable crates (three tiers: the
  `aozora` umbrella, the internal "no stability contract, use `aozora`" crates,
  and the user-facing `aozora-cli` / `aozora-pandoc`) so no crates.io page is
  empty — using crates.io-absolute URLs, with a `readme-gate` enforcing their
  presence. See #469.

## [0.4.1] - 2026-06-15

First tagged multi-channel publish (crates.io / npm / PyPI). The library API and
CLI behaviour are unchanged from 0.4.0; this release makes the internal crates
publishable and hardens the release pipeline.


### Build

- **release**: publish the whole workspace to crates.io / npm / PyPI in
  dependency-topological order — the internal crates become publishable and the
  workspace bumps to v0.4.1, with the publishing runbook documented (#74)
- **deps**: bump the rust-deps group with 6 updates (#72)
- **deps**: bump Rust 1.95.0 → 1.96.0 in the docker-base-images group (#76)


### CI

- **release**: harden the release workflow — Node24 attestation, pinned
  windows-2025 runner, and cargo network retry (#71)


### Documentation

- **release**: record the pre-1.0 code-signing deferral decision (#70)

## [0.4.0] - 2026-06-14


### Added

- **encoding**: Auto-detect source encoding; stop hard-coding Shift_JIS (#60) (#60)
- **aozora**: Add opt-in prewarm() to force parser boot off the hot path (#56) (#56)
- **aozora**: Promote render_node + Arena to the curated front door (#45) (#45)
- **playground**: Solid + CM6 + WASM playground with Docker, tests, mobile (#38) (#38)


### Build

- **deps**: Bump pyo3 from 0.28.3 to 0.29.0 (#65) (#65)
- **deps**: Bump unicode-segmentation from 1.13.2 to 1.13.3 in the rust-deps group (#64) (#64)
- **aozora**: Refresh dev tooling and crate deps to latest (#54) (#54)
- **deps**: Bump ubuntu from 24.04 to 26.04 in the docker-base-images group across 1 directory (#35) (#35)
- **aozora**: Bump playground vite to v8 (fix GHSA-4w7w-66w2-5vf9) (#52) (#52)
- **aozora**: Run dev/CI containers as a non-root user (#48) (#48)
- Dev-env hygiene — cargo caches out of the bind mount + playground self-init (#44) (#44)
- **deps**: Bump the actions-sha-bumps group with 2 updates (#29) (#29)


### CI

- Exempt CI-only rust-cache (LGPL-3.0) from dependency-review license check (#68) (#68)
- Attest release artifacts with build provenance (#66) (#66)
- **aozora**: Pin docs.yml tool versions and serialise its rustdoc build (#50) (#50)
- **aozora**: Add a PR-time rustdoc (doc) gate (#47) (#47)


### Changed

- **aozora**: Single-source the PUA sentinel set in tests (#51) (#51)
- **scan**: Pr6b — hand-rolled Teddy redesign (outer/inner split, AVX2 / SSSE3 / NEON / WASM SIMD) (#31) (#31)
- **scan**: Pr6b — hand-rolled Teddy redesign (outer/inner split, AVX2 / SSSE3 / NEON / WASM SIMD) (#31)
- **pipeline**: Pr4 — type-state field-bound pipeline + tightened gate (#27) (#27)


### Chore

- **hooks**: Wire playground gates into lefthook + persist signing-check (#40) (#40)


### Documentation

- **aozora**: Add CLAUDE.md and an ADR home (#46) (#46)
- Drop hardcoded version pins from install copy (#23) (#23)


### Fixed

- **security**: Harden parser, FFI/WASM boundaries, and CI for release (#67) (#67)
- **playground**: Tokenise fonts and define the undefined --font-sans (#63) (#63)
- **scan**: Make the no_std path compile + correct the unsafe-tree claim (#62) (#62)
- **trace**: Load samply traces that use timeDeltas, not absolute time (#58) (#58)
- **aozora**: Install bacon from source, unblocking cargo-binstall 1.19.1 (#55) (#55)
- **aozora**: Serialise cargo doc to kill the parallel rustdoc race (#49) (#49)
- **pipeline**: Forward-ref bouten/TCY consume preceding target literal (#42) (#42)
- **hooks+pipeline**: Close local-only gaps that let just ci silently fail (#41) (#41)
- **render**: Serialize is now I3-idempotent across decorative-rule boundaries (#33) (#33)


### Performance

- **scan**: Replace the unsafe SIMD Teddy with safe aho-corasick (#61) (#61)
- **spec**: Classify triggers with a direct match, not a runtime-hash phf (#59) (#59)


### Tests

- **aozora**: Add dhat heap profile + synthetic latency probe (align with afm) (#57) (#57)
- **proptest**: Close emit-symmetry / Annotation::Unknown / wire / phase1 gaps (#34) (#34)
- **fuzz**: Systematise fuzz triage workflow + fix BOM I3 fixed-point bug (#32) (#32)
- **quality**: Pr6a — docs drift cleanup + rustdoc deny + Phase D recipe + conformance 6-axis gate (#30) (#30)
- **quality**: Pr5 — coverage ratchet 73, bench drift recipes, ci corpus-sweep (#28) (#28)
- **quality**: Pr3 — snapshot tests for render html, ast pretty, cli help (#26) (#26)
- **quality**: Pr2 — property tests across 11 crates + 2 pandoc bug fixes (#25) (#25)
- **quality**: Pr1 — re-tighten rustdoc deny + expect-count calibration gate (#24) (#24)

## [0.3.0] - 2026-05-01


### CI

- **release**: Introduce release-plz for tag + CHANGELOG automation (#14) (#14)


### Fixed

- **ci**: Repair release-plz uses ref (mangled by email obfuscation) (#19) (#19)
- **ci**: Pin release-plz/action to v0.5.128 (no major float tag) (#15) (#15)
- **book**: Lychee cache + max_retries=5 to absorb pyo3.rs CI flakiness (#12) (#12)
- **book**: Replace dead Hyperscan docs URL with the GitHub repo (#10) (#10)
- **ci**: Install mdbook-mermaid assets before book build (#9) (#9)


### Performance

- **ci**: Build-itself optimisations + lefthook tightening / parallelisation (#18) (#18)
- **ci**: Project-specific deeper optimisations (bench-exclude, drift-gate, rayon, image cache, components) (#17) (#17)
- **ci**: Route in-container sccache through GitHub Actions cache backend (#16) (#16)
- **ci**: Cargo-binstall + lychee retry + xtask ci profile/precheck/act (#11) (#11)


### Release

- 0.4.0 — DX / downstream integration (Phase K-P) (#13) (#13)

## [0.2.6] - 2026-04-29


### Added

- **render**: Aozora-* class prefix flip + gaiji data attrs + wasm-opt skip (#4) (#4)


### Build

- **deps**: Bump the actions-sha-bumps group across 1 directory with 8 updates (#3) (#3)


### CI

- Drop spec/upstream-diff from ci.yml + matching Justfile recipes (post-v0.2.0 split they referenced retired aozora-parser crate), remove unused deps per cargo-udeps (insta in 4 crates, tracing in aozora-encoding, aozora-test-utils in aozora-render dev-deps, proptest in aozora-trace dev-deps), apply cargo fmt across the workspace
- **workflows**: Structurally fix Docker-in-Docker bug + Phase B-2 GHCR pull (run `just <target>` on runner host instead of `docker compose run ci just` which lacked docker client; introduce setup-dev-image composite action that pulls ghcr.io/p4suta/aozora-dev:latest with build fallback; drop CommonMark/GFM spec matrix entries that aozora doesn't ship)
- **dev-image**: Publish dev image to GHCR (ghcr.io/p4suta/aozora-dev:latest) so ci.yml can pull instead of rebuilding the 30-40min image on every commit (Phase B bootstrap, mirrors afm)
- Fix msrv tag drift, ignore dtolnay/rust-toolchain in Dependabot, pre-create cache mount targets in Dockerfile (RO bind-mount), drop ci.yml book job (no mdbook in this repo)


### Fixed

- **py**: Rename pymodule to aozora_py so maturin build accepts it (#6) (#6)
- **strict-code**: Adjust gates to match repo reality post-v0.2.0 — exempt aozora-{ffi,scan,xtask} from unsafe-forbidden gate (FFI / SIMD / dev-tooling legitimately need unsafe), accept Rust 1.81+ `#[allow(... reason=\"...\")]` documented carve-outs, skip build.rs string-literal artifacts; rephrase one bare-TODO marker in aozora-scan
- **ci**: Exclude build.rs from strict-code println! grep (cargo build-script protocol uses println!"cargo:..." by spec), drop unused criterion dev-dep from aozora-veb
- **docker-compose**: Change ci service bind mount from :ro to :cached so named-volume mountpoints (target/.cargo/.sccache) attach without read-only fs error

## [0.2.5] - 2026-04-28


### Documentation

- **readme**: Add prominent Pages/Releases links + crate-by-crate table + install/library-use sections


### Release

- Bump workspace to v0.2.5 (v0.2.4 tag landed on wrong commit, blocked from force-move by tag ruleset; v0.2.5 carries the cliff.toml repo fix to a valid release tag)
- Bump workspace to v0.2.4 + fix cliff.toml repo (was "afm", now "aozora")

## [0.2.4] - 2026-04-28


### Release

- Bump workspace to v0.2.3 + slim release matrix to 3 platforms (drop x86_64-apple-darwin Intel + x86_64-unknown-linux-musl)

## [0.2.2] - 2026-04-28


### Release

- Bump workspace to v0.2.2 + use explicit `rustup target add` in release.yml (musl build fix)

## [0.2.1] - 2026-04-28


### Documentation

- **workflow, lints**: Rewrite docs.yml for aozora layout (no mdbook, redirect index.html to /aozora/), demote rustdoc broken/private intra-doc links to warn during v0.1.0→v0.2.0 split transition, fix immediate rustdoc errors (sentinel const refs, AozoraTree path, AozoraNode legacy reference)


### Release

- Bump workspace to v0.2.1 + fix release.yml package name (aozora-cli, not afm-cli)

## [0.2.0] - 2026-04-28


### Added

- **encoding**: Single-char description fallback for gaiji lookup + canonical-case probe
- **scan, trace**: T2 SIMD scanner bake-off + aozora-trace toolkit
- **lex**: Switch lex_into_arena to BorrowedAllocator (I-2.2 Commit D)
- **lexer**: Generic phase3 over NodeAllocator (I-2.2 Commit C)
- **syntax**: BorrowedAllocator NodeAllocator impl (I-2.2 Commit B)
- **syntax**: NodeAllocator trait + OwnedAllocator impl (I-2.2 Commit A)
- **render**: Visitor trait for borrowed-AST traversal (Innovation I-10)
- **lex**: Type-state pipeline wrappers (Innovation I-3)
- **syntax, lex, aozora, drivers**: Borrowed-AST migration — interner (I-7) + Document → borrowed surface (Plan B.1.1 + B.4)
- **render**: Borrowed-AST native HTML renderers (Plan B.3)
- **syntax, lex**: Owned→borrowed bridge + arena-emitting lex API (Plan B.1+B.2)
- **lex**: Orchestrator split + falsified scan-tokenizer hypothesis (ADR-0013)
- **scan**: AVX2 SIMD backend + NEON/wasm-simd scaffolds + ADRs 11/12 + PGO pipeline
- **ffi, wasm, py, bench**: Multi-target driver crates (Move 4)
- **aozora, render, parallel**: Split aozora-parser surface — Document façade (Move 3)
- **scan, lex**: Split lex layer into aozora-scan + aozora-lex (Move 2 milestone)
- **syntax**: Add borrowed AST module — zero-copy + arena (Move 1.4, coexist mode)
- **veb**: Add aozora-veb micro-crate — Eytzinger cache-friendly search (Move 1.3)
- 0.2.0 prep — parallel/incremental groundwork + Move 1.2 spec compatibility shim
- **spec**: Extract aozora-spec as canonical truth source (Move 1.1)
- Birth aozora as comrak-free 青空文庫記法 parser
- **parser**: Promote ［＃「X」は(大|中|小)見出し］ to Markdown heading
- **book**: Afm-horizontal / afm-vertical theme stubs + class contract test (M1 Phase E)
- **test**: Corpus sweep I3 round-trip fixed-point gate (M2-S6)
- **parser**: AST → afm text serializer via registry inverse (M2-S5)
- **test**: HTML well-formed invariant I4 (M2-S4)
- **parser**: ParseResult { root, diagnostics } + CLI --strict (G2)
- **encoding**: Real gaiji UCS lookup table via phf (G1)
- **parser**: Paired-container AST wrap (F5, census floor 350→400)
- **lexer**: Double angle-bracket 《《X》》 recognition (F4)
- **lexer**: Expand kaeriten coverage to compound marks and okurigana (F3)
- **lexer**: Expanded forward-bouten kinds + position + multi-quote (F2)
- **lexer**: Ruby reading Content::Segments (F1, R1 resolved)
- **lexer**: Fold accent decomposition into Phase 0, delete preparse.rs (E2 / C5b)
- **comrak**: Render dispatch to fn pointer, delete AozoraExtension trait (D2)
- **comrak**: Remove upstream Aozora parse hooks, make dead_code a hard deny (D1)
- **parser**: Cutover parse() to lexer + post_process pipeline (E1)
- **lexer**: Classifier validation for indent/forward-ref (E1c)
- **lexer**: Pad block sentinels with blank lines for paragraph isolation (E1b)
- **parser**: Post_process block-leaf splice (D4a)
- **parser**: Post_process inline splice (D3)
- **lexer**: Wire lex() end-to-end through phases 0..6 (C-cap)
- **lexer**: Phase 6 validate — V1–V3 structural invariants (C7)
- **lexer**: Phase 5 registry — binary-search lookup API (C6)
- **lexer**: Phase 4 normalize — PUA sentinel substitution (C5a)
- **lexer**: Phase 3 classify — paired container markers (C4e)
- **lexer**: Phase 3 classify — gaiji + kaeriten (C4d)
- **lexer**: Phase 3 classify — forward TCY + sashie (C4c4)
- **lexer**: Phase 3 classify — forward-reference bouten (C4c3)
- **lexer**: Phase 3 classify — indent/alignment annotations (C4c2)
- **lexer**: Phase 3 classify — block leaf annotations (C4c1)
- **lexer**: Phase 3 classify — ruby recognition (C4b)
- **lexer**: Phase 3 classify — scaffold with span-coverage invariant (C4a)
- **lexer**: Phase 2 pair — accessors + property invariants (C3b)
- **lexer**: Phase 2 pair — symmetric balanced-stack matching (C3a)
- **lexer**: Phase 1 events — linear tokenize into trigger stream
- **lexer**: Phase 0 sanitize — BOM / CR-LF / PUA collision scan
- **syntax**: Migrate body-bearing AozoraNode fields to Content
- **syntax**: Add Content/Segment/Kaeriten schema additives
- **lexer**: Scaffold afm-lexer crate with sentinel constants
- **test**: Corpus sweep harness for invariants I1/I2/I5
- **corpus**: Implement InMemory, Vendored, and Filesystem sources
- **corpus**: Scaffold afm-corpus crate with CorpusSource trait
- **parser**: Leaf 字下げ / 地付き / 地から N 字上げ annotations
- **parser**: Recognise 縦中横 forward-ref and paired forms
- **html**: Render Bouten as semantic <em class=afm-bouten-{kind}>
- **parser**: Promote ［＃「X」に{傍点|丸傍点|…}］ to Bouten variants
- **parser**: Promote ［＃改ページ／改丁／改段／改見開き］ to typed variants
- **test+lint**: CommonMark/GFM spec runners + strict-code defensive gate
- **xtask**: Spec-refresh — vendor cmark sources, regenerate JSON fixtures
- **parser**: Preparse pass applies accent decomposition inside 〔...〕
- **parser**: Implement AfmAdapter + HTML renderer, wire comrak dep
- **syntax**: Accent decomposition table + decompose_fragment
- **comrak**: Wire Extension.aozora + inline dispatch + render hook
- **syntax**: Add AozoraExtension trait + context types


### Build

- **deny**: Pin internal path deps to version = 0.1.0 (cargo-deny wildcards gate)
- **deps**: Bump workspace crates across semver-major boundaries (incompat)
- **deps**: Bump all workspace crates to latest semver-compatible versions
- **scripts, adr-0018**: D4 — local systemd user timer for weekly deps-check + dependency policy ADR
- **lefthook**: D3 — post-merge deps-status notice + pre-push audit gate
- **just**: D2 — add deps-* recipes (outdated / upgrade / deps-check / deps-status)
- **docker**: D1 — add cargo-outdated to dev container for dependency follow-up tooling
- **xtask**: Replace shell samply scripts with the aozora-xtask crate (N7)
- Add just corpus-sweep with env-driven bind-mount


### CI

- Add release / docs / dependency-review / MSRV workflows + git-cliff automation
- Add CODEOWNERS + PR and Issue templates


### Changed

- Delete aozora_syntax::owned + OwnedAllocator + NodeAllocator (Phase F.4)
- Delete aozora_lexer owned API + hardcode BorrowedAllocator (Phase F.3)
- Delete aozora_lex owned API (Phase F.2)
- Delete aozora-parser + aozora-parallel (Phase F.1)
- I-2.2 Commit E (partial) — delete convert + parser html/serialize/test_support
- **syntax**: Owned-AST → `aozora_syntax::owned` submodule + drop top-level shim (Plan B.6 + B.6.1)
- **parser**: Extract ［＃...］ scanner + dispatcher into aozora::annotation
- **parser**: Promote strip_afm_annotations + Tier-A canary to test_support
- **syntax**: Tighten AozoraNode types per ADR-0003


### Chore

- Stop tracking .claude/ harness state
- **typos**: Exclude vendored JIS X 0213 tables from spell check
- Clippy --all-features clean (Docker CI gate) + lefthook hooks installed
- Silence all workspace warnings (41 → 0)
- **aozora, render**: Retire legacy owned-AST public surface (Plan B.5)
- Add NOTICE, code of conduct, changelog, editor/git configs
- **dev+release**: Wire dev tooling + prep repo for public release
- **test**: Corpus sweep I2 hard gate + coverage floor 94→95 (G3)
- **lint**: Document stable rustfmt policy in rustfmt.toml
- **lint**: Add hand-picked clippy::restriction lints, fold new warnings
- **lint**: Add [workspace.lints.rustdoc] block, fix fence ambiguity
- **lint**: Expand workspace.lints.rust, make Justfile yield to Cargo.toml
- **specs**: Vendor Aozora Bunko annotation spec pages for offline reference
- Bootstrap afm workspace with Docker-only dev environment


### Documentation

- **adr-0020**: L-1/L-2/L-3 sprint verdict — load wall 3.5s → 1.38s = 2.5× (L-4 mmap DROPPED — unsafe non-negotiable)
- **adr-0019**: B'-2 PROMOTE + B'-3 deferred to plan A — lazy-AST is incompatible with current Copy AozoraNode shape; +5-10 % corpus target requires full simdjson rewrite, not retrofits
- **adr-0019**: B'-1 negative result — caller-driven Interner bypass for annotation/gaiji is in noise (regression on 500K-2M band, no win on doc 50685)
- **adr-0019**: Post-A0+A drill-down — B hypothesis falsified, true hot path identified, simdjson-style 1-pass = only order-of-magnitude candidate
- **adr-0019**: Final A0/A/M-2/M-3 verdict — A0+A keep, M-2/M-3 revert
- **adr-0019, profiling**: M1-M3 modern follow-ups — measured deltas + FSM verdict
- **adr-0017, profiling**: R4 — bumpalo arena Vec + rayon parallelism case study
- **adr-0016, profiling**: I-2 deforestation reversal investigation
- **classify**: R1 — #[inline] sweep on Phase 3 hot dispatch produces no measurable gain
- PROFILING.md consolidating samply + probe methodology
- **lexer, scan**: T1 SIMD tokenizer investigation — negative result + scaffolding
- **adr**: Record 0009 (clean layered architecture) + 0010 (zero-copy + observable equivalence)
- **book**: Expand handbook skeleton and add library usage examples
- Rebrand as "Aozora Flavored Markdown" for v0.1.0 public preview
- Refresh CLAUDE.md + strip phase/task-progress markers from code
- **adr**: ADR-0008 zero-parser-hook Aozora-first lexer pipeline
- **adr**: ADR-0007 corpus sweep strategy + developer onboarding
- **adr**: Land ADR-0006 — lint profile policy and scope discipline
- **adr**: Land ADR-0004 — accent decomposition via pre-parse rewrite


### Fixed

- **corpus**: Bound untrusted archive header sizes + fsync archive finish
- **render**: Unify HTML apostrophe entity on `&#x27;`
- **parser**: Implicit-close same-family paired containers (Aozora spec)
- **lexer**: Isolate decorative ---/===/___ rules from setext headings
- **build**: Make just coverage actually run, set honest regions floor


### G.3

- Streaming type-state Pipeline canonical (I-3 restored)


### G.4

- Streaming API unit tests + integration tests


### I-2

- Full iterator-fusion deforestation across phase 1-4


### N2

- Fix O(N²) front-pop in ClassifyStream + 17-subsystem instrumentation


### N6

- Pre-size Document arena from source length


### Performance

- **corpus, xtask, bench**: L-6 — uncache (rustix fadvise) + stat dashboard + zero-copy iter; cold-cache zstd UTF-8 = 0.82s (4.27× vs L-1 seq, only 12% over warm)
- **corpus, xtask, bench**: L-5 — single-file archive (4 variants) + xtask pack + incremental + bench wiring; zstd UTF-8 = 0.73s load wall (4.79× vs L-1 seq, 5.0× end-to-end, pure safe Rust)
- **bench, corpus**: L-4-bis — physical-core rayon pool for load phase (num_cpus::get_physical avoids 16t over-subscription)
- **corpus, encoding, bench**: L-2 + L-3 — par_load_decoded + parallel_size_bands fold/reduce + decode_sjis_into buffer reuse
- **bench, corpus**: L-1 — load-phase split + decode_throughput example + walkdir double-stat fix
- **arena, bench**: B'-2 — Arena::reset_with_hint(text.len() * 4) for per-thread reuse + pathological_probe arena reuse
- **classify**: B — eliminate synthetic gaiji-body SmallVec rebuild; pass original body with bracket_open_idx=0
- **lex, classify**: Revert M-2 (Pure SoA) + M-3 (flat FSM) — A0+A baseline shows them as net regression even with Phase 1 heap reduced
- **arena, bench**: A — initial Arena capacity hint based on per-doc source size
- **lexer**: A0 — arena-allocate Phase 1 scratch Vec<u32> for trigger/newline offsets
- **classify**: M-3 — flat-state-machine Phase 3 classifier (cfg-gated, comparative)
- **lexer, lex**: M-2 — Pure SoA TokenStream + PairEventStream inter-phase storage
- **arena**: M-1 — per-thread Arena reuse via thread_local + Bump::reset
- **bench**: R4-B — rayon par_iter mode for throughput_by_class + phase_breakdown
- **lex**: R4-A — bumpalo BumpVec for inter-phase materialisation; drop heap-batch APIs
- **classify**: R3 — add classify_slice + classify_into_emit Phase 3 batch APIs
- **lexer**: R2 — Phase 1 → Vec<Token>; Phase 2 → &[Token]; drop Pipeline I generic
- **render**: Byte-level memchr scan for HTML/serialize sentinels (R1)
- **lexer**: Stream-through mode for top-level Quote / Tortoise pairs (N3)
- **lex**: Fuse phase 4 normalize with arena conversion (Innovation I-2.1)
- **pair**: Inline-store pair stack via SmallVec (Innovation I-8)
- **sanitize**: Smart phase 0 sub-pass rewrites (Plan H, +50% phase 0)
- **tokenize**: ASCII fast-path in phase 1 (Plan G)
- **classify**: Aho-Corasick anchored DFA for body annotation dispatch (Plan F)
- Validate-skip in release + memchr PUA scan in phase 0 (12-15% corpus win)
- **classify**: Aho-Corasick batch index for forward-reference precedence (8x on pathological)
- Four data-structure / API refactors targeting the lexer-parser hot path


### Tests

- **encoding, render**: Property + gatekeeper suites for SJIS / gaiji / serialize
- Multi-layer negative test enhancement (Tier A-L invariants)
- **parser**: Heading promotion + rule isolation integration, ratchet 56656 counters
- Close coverage gaps; ratchet floor 95 → 96 (Cov-Ratchet)
- **encoding**: Thicken SJIS decode + BOM + gaiji resolution coverage (T4)
- **parser**: Aozora × CommonMark block-structure interaction suite (T3)
- **parser**: Post_process + parse end-to-end invariants + proptest (T2)
- **parser**: XSS / HTML-escape invariants across every Aozora render path (T1)
- **parser**: Accept Annotation|Gaiji for gaiji fixture, reclassify uplift gap (E1d)
- **parser**: Path-parity harness for adapter vs lexer pipeline (E1a)
- **golden-56656**: Enable Tier A acceptance + regression harnesses


### Afm

- Stage 1 public API surface + gaiji close-quote serializer fix


### Release

- Bump workspace to v0.2.0 — aozora top-level facade + extended crate set

<!-- generated by git-cliff -->
