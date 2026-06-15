# aozora — Claude Code project guide

Opening note for any Claude Code session that enters this repo: read this
file first. It is the shortest path to productive work.

## What this is

**aozora**: a pure-functional Rust parser for **青空文庫記法** (Aozora
Bunko notation) — ruby (`｜青梅《おうめ》`), bouten (`［＃「X」に傍点］`),
縦中横, 外字 references (`※［＃…、第3水準1-85-54］`), kunten / kaeriten,
indent / align-end containers (`［＃ここから2字下げ］… ［＃ここで字下げ終わり］`),
and page / section breaks.

The parser is **CommonMark-free, Markdown-free** — this repository deals
only with the 青空文庫 notation itself. The renderer emits semantic HTML5;
the lexer reports structured diagnostics; the AST is a borrowed-arena tree
that can be walked in O(n) without copying source bytes.

Hard guarantees:
- **Single front door.** Downstream consumers depend on the umbrella
  `aozora` crate alone. The build-block crates (`aozora-spec`,
  `aozora-syntax`, `aozora-pipeline`, `aozora-render`, `aozora-encoding`)
  are published to crates.io only so the umbrella can depend on them;
  they are internal/unstable, carry **no API-stability contract**, and
  are reachable through `aozora`'s curated re-exports + the `pipeline` /
  `syntax` / `render` / `encoding` / `wire` modules.
- **`#![forbid(unsafe_code)]` across the parser core.** The trigger
  scanner (`aozora-scan`) used to be the sole core exception (a
  hand-rolled per-ISA SIMD Teddy); it now delegates to the safe,
  portable `aho-corasick` packed matcher (differentially tested against
  a naive backend), so the whole parsing/rendering path is unsafe-free.
  The only `unsafe` left is at the FFI boundary (`aozora-ffi`'s C ABI,
  `#[unsafe(no_mangle)]` + raw-pointer handling) — unavoidable there and
  quarantined under `unsafe_code = "deny"`.
- **Single binary, no runtime process dependencies** for the `aozora` CLI.

## Architecture

```text
source (UTF-8 or Shift_JIS)
   │
   ▼ aozora_encoding::decode_sjis  (if SJIS)  + gaiji resolution
   │
┌──┴── aozora_pipeline::lex_into_arena (borrowed-AST) ────────────────────┐
│ Phase 0  sanitize   BOM / CRLF→LF / 〔…〕 accent decomposition /         │
│                     PUA collision scan                                   │
│ Phase 1  events     SIMD trigger-byte tokenise (aozora-scan)             │
│ Phase 2  pair       balanced-stack bracket / ruby / quote pairing        │
│ Phase 3  classify   borrowed AozoraNode<'arena> + ContainerKind, with    │
│                     Annotation{Unknown} catch-all so every `［＃…］`     │
│                     is claimed (no bare `［＃` ever leaks)                │
│                                                                          │
│ Output: BorrowedLexOutput<'arena> { normalized, registry, diagnostics }  │
└───┬──────────────────────────────────────────────────────────────────────┘
    │
    ▼  aozora_render::html / serialize / render_node::render
    │     (semantic HTML5  /  byte-exact round-trip  /  per-node writer)
    ▼
   HTML  +  structured Diagnostics
```

[`Document`] owns the source buffer plus a `bumpalo`-backed [`Arena`];
[`AozoraTree`] borrows from that arena via the `&self` lifetime returned
by [`Document::parse`]. Dropping the `Document` releases the whole tree in
one `Bump::reset`. The [`Interner`] deduplicates repeated string content.

### Workspace crates

| Crate | Responsibility |
|---|---|
| `aozora` | **The front door.** `Document` / `AozoraTree` + curated re-exports (`lex_into_arena`, `html`, `serialize`, `render_node`, `Arena`, `Diagnostic`, node types) and the `pipeline`/`syntax`/`render`/`encoding`/`wire`/`cst`/`query`/`proptest` modules. Depend on this alone. |
| `aozora-spec` | `Diagnostic` / `Span` / `NormalizedOffset`, the PUA sentinel constants, and the canonical `SLUGS` catalogue. Internal/unstable crate, now on crates.io — depend on `aozora`. |
| `aozora-syntax` | `borrowed::AozoraNode<'a>` + `Arena` + `Interner` + `BoutenKind` / `ContainerKind` / `AnnotationKind` / `SectionKind` / the accent table. Internal/unstable crate, now on crates.io — depend on `aozora`. |
| `aozora-pipeline` | `lex_into_arena` + the four lexer phases (sanitize/events/pair/classify). Internal/unstable crate, now on crates.io — depend on `aozora`. |
| `aozora-render` | `html` / `serialize` / `render_node::render` per-node writer + bouten CSS slugs. Internal/unstable crate, now on crates.io — depend on `aozora`. |
| `aozora-encoding` | Shift_JIS decode + gaiji resolution (`gaiji::Resolved`). Internal/unstable crate, now on crates.io — depend on `aozora`. |
| `aozora-scan` | Trigger-byte scanner. Delegates to the safe, portable `aho-corasick` packed matcher (std) / `NaiveScanner` (no_std); differentially tested against the naive reference. Fully `forbid(unsafe_code)`. |
| `aozora-veb` | Eytzinger-laid-out static maps used by the registry tables. |
| `aozora-cst` | Lossless rowan concrete-syntax tree (`cst` feature) for editor-grade tooling. |
| `aozora-query` | tree-sitter-flavoured query DSL over the CST (`query` feature). |
| `aozora-conformance` | Byte-identical render gate + the `fixtures/render/<case>/` corpus. |
| `aozora-corpus` | Corpus-source abstraction for the `corpus-sweep` invariant pass. |
| `aozora-proptest` | Stratified proptest generators (shared with afm via git pin). |
| `aozora-trace` | Parse profiling / flamegraph analysis tooling. |
| `aozora-bench` | criterion benches (corpus-driven + synthetic); excluded from workspace CI gates. |
| `aozora-cli` | The `aozora` binary (`render` / `check` / `pandoc` / …). |
| `aozora-wasm` | `wasm-bindgen` browser package (editor primitives). |
| `aozora-ffi` | cbindgen C ABI (`aozora.h`). |
| `aozora-extism` | Extism (WASM) plugin driver — one portable `aozora.wasm` for polyglot host SDKs (Go/Java/PHP/Ruby/…); the breadth strategy for new languages (ADR-0006). `just extism-build` / `just smoke-extism`. |
| `aozora-go` | **Go module** (not a cargo crate; in `exclude`). Host SDK over `aozora.wasm` via pure-Go wazero; wire types generated by `xtask types langs` (quicktype). `just smoke-go`. |
| `aozora-py` | PyO3 / maturin Python bindings. |
| `aozora-pandoc` | Projection to `pandoc_ast` JSON (`aozora pandoc`). |
| `aozora-book` | mdbook handbook (not a Rust crate). |
| `aozora-xtask` | Dev automation (`schema`, `types`, `conformance`, `new-adr`, …). |

## Sibling projects

- **[`P4suta/afm`](https://github.com/P4suta/afm)** — *Aozora Flavored
  Markdown*: a CommonMark + GFM superset that **consumes this parser** (it
  pins `aozora` by git rev and splices Aozora spans into comrak output via
  `render_node::render`). New 記法 / lexer / AST / renderer work lands
  HERE, not there; afm only owns the Markdown ↔ Aozora composition glue.
- **[`P4suta/aozora-tools`](https://github.com/P4suta/aozora-tools)** —
  formatter / LSP / editor plugins (ADR-0009). They build on `aozora-cst`
  / `aozora-query`.

When a proposed change is "really a parser change", it belongs here.

## Development environment

Docker is the only accepted execution surface (every `just` target runs
`docker compose run …`). Never invoke cargo / wasm-pack / mdbook on the
host. Cargo caches live in named volumes mounted at `/cargo/*` — outside
the `/workspace` bind mount — so the host working tree stays clean of
root-owned dirs.

```
just build                # cargo build (excl. aozora-bench)
just test                 # cargo nextest run
just lint                 # fmt-check + clippy + typos + strict-code + doc
just doc                  # rustdoc with all rustdoc lints = deny
just ci                   # full fail-fast pipeline (mirrors CI)
just prop / prop-deep      # 128-case / 4096-case property sweeps
just drift-gate           # schema-check + types-check (wire / .d.ts drift)
just conformance          # WPT-style must/should/may runner
just smoke-ffi            # C ABI end-to-end (host-side build)
just coverage             # cargo-llvm-cov regions floor
just playground-dev       # Vite at http://localhost:5173/aozora/playground/
just wasm-build           # aozora-wasm release pkg/
just fuzz-quick / -deep    # cargo-fuzz harnesses
just corpus-sweep         # invariant pass over $AOZORA_CORPUS_ROOT (opt-in)
just hooks                # install lefthook git hooks
```

There is **no `just check`** — use `just build` for the fastest "still
compiles?" gate (afm uses `just check`; aozora does not — don't assume it).

## Version control

- **Signed commits are mandatory** (`commit.gpgsign = true`, ssh signing).
  A three-layer defence: a post-commit re-amend, the `signing-check`
  pre-push command (`scripts/check-signed-commits.sh`), and GitHub's
  "require signed commits" ruleset.
- **`lefthook` pre-push runs `just ci`** (mirrors every CI job reachable
  from inside the dev image) + a `prop-deep` sweep. Don't push around it.
- **No force-push to `main`.** PR + merge; `main` is branch-protected.
- Conventional Commits are enforced by the `commit-msg` hook.

## Architecture Decision Records

ADRs live under `docs/adr/` (MADR format; scaffold with
`just new-adr "<title>"`). Read the one that governs an area before
touching it. Cross-repo: afm's ADRs that concern the parser core
(zero-parser-hooks, the lint/profile policy, the core extraction) are
homed here; afm keeps redirect stubs pointing at these.

## DO NOT

- **Do not let a bare `［＃` reach HTML output.** The
  `Annotation{Unknown}` catch-all + the Tier-A invariant tests guarantee
  every annotation is claimed. New 記法 extends the classifier, never the
  HTML escape hatch.
- **Do not bypass the umbrella crate.** Downstream code (incl. afm)
  consumes `aozora::…`, not `aozora-syntax` / `aozora-render` directly.
  The build-block crates are on crates.io so the umbrella can depend on
  them, but they carry no API-stability contract — depending on them
  directly forfeits the tested, versioned surface.
- **Do not add `unsafe` outside the FFI boundary.** The parser core is
  `forbid(unsafe_code)`; only `aozora-ffi`'s C ABI may carry `unsafe`.
  Reach for a safe, audited crate (as `aozora-scan` does with
  `aho-corasick`) before hand-rolling SIMD.
- **Do not suppress warnings** (`#[allow(...)]`, `continue-on-error`)
  without a `reason = "…"` and a `strict-code` exemption.
- **Do not run cargo / wasm-pack / mdbook on the host.** `just` + Docker
  only.
- **Do not pin dependency versions from memory** — verify against
  crates.io / GitHub Releases at decision time.
