# Changelog

All notable changes to aozora are recorded in
this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
