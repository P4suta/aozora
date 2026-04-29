# Architecture

This document describes how aozora is structured today: the crate
layers, the AST shape, and the safety / test / deployment policies
that the workspace gates on.

For *how to use* the parser, see [`USAGE.md`](./USAGE.md). For the
contributor flow, see [`../CONTRIBUTING.md`](../CONTRIBUTING.md).

## Overview

aozora parses 青空文庫 source text in three layers:

```
              source text  (UTF-8 or Shift_JIS)
                   │
                   ▼
   ┌──────────────────────────────────────┐
   │  Lex            (aozora-lex)         │   pure fn(&str, &Bump)
   │   sanitize → tokenize → pair → classify
   └──────────────────────────────────────┘
                   │
                   ▼   AozoraTree<'arena>  (borrowed AST)
                   │
                   ▼
   ┌──────────────────────────────────────┐
   │  Render        (aozora-render)        │   pure fn(&Tree) -> String
   │   html  /  serialize                 │
   └──────────────────────────────────────┘
                   │
                   ▼
              HTML  /  canonical 青空文庫 source
```

Every step is a pure function. Given the same input, the same `Bump`
arena, and the same compile-time configuration, the output is bit-for-bit
identical. There are no hidden global caches, thread-locals, or
`OnceCell` shortcuts in the parse path — the only state is the arena
itself and an [`Interner`](https://docs.rs/aozora-syntax) for string
deduplication.

## Crate layers

```
                     ┌──────────────┐
                     │ aozora-spec  │   shared types (Span, Diagnostic,
                     │              │   TriggerKind, PUA sentinels)
                     └──────┬───────┘
                            │
        ┌───────────────────┼───────────────────┐
        ▼                   ▼                   ▼
┌──────────────┐   ┌────────────────┐   ┌──────────────┐
│ aozora-      │   │  aozora-       │   │ aozora-veb   │
│ encoding     │   │  scan          │   │ (sorted-set) │
│ (Shift_JIS,  │   │  (SIMD scan)   │   │              │
│  外字)       │   │                │   │              │
└──────┬───────┘   └────────┬───────┘   └──────┬───────┘
       │                    │                   │
       └────────┬───────────┴───────────────────┘
                ▼
        ┌──────────────┐
        │aozora-syntax │   AST node types
        └──────┬───────┘
               │
               ▼
        ┌──────────────┐         ┌──────────────┐
        │aozora-lexer  │  ◄──────│ (tests)      │
        │ (7-phase     │         └──────────────┘
        │  classifier) │
        └──────┬───────┘
               │
               ▼
        ┌──────────────┐
        │ aozora-lex   │   pure fn(&str, &Bump) -> AozoraTree<'_>
        └──────┬───────┘
               │
               ▼
        ┌──────────────┐
        │aozora-render │   html / serialize
        └──────┬───────┘
               │
               ▼
        ┌──────────────┐
        │   aozora     │   public facade — Document, AozoraTree
        └──────┬───────┘
               │
   ┌───────────┼───────────┬─────────────┐
   ▼           ▼           ▼             ▼
aozora-cli  aozora-ffi aozora-wasm   aozora-py
```

| Crate | Role |
|---|---|
| `aozora-spec` | Single source of truth for shared types. **No internal dependencies** — every other crate may depend on it. |
| `aozora-syntax` | AST node types (`AozoraNode`, `Bouten`, `Ruby`, `Kaeriten`, `Indent`, `Sashie`, `Warichu`, …) backed by the bumpalo arena. |
| `aozora-encoding` | Shift_JIS decoding and 外字 (gaiji) lookup. Compile-time PHF tables resolve JIS X 0213 codepoints + UCS fallbacks. |
| `aozora-scan` | SIMD-friendly multi-pattern byte scanner. The only crate (besides `aozora-ffi`) that locally relaxes `unsafe_code`. |
| `aozora-veb` | Eytzinger-layout sorted-set lookup — cache-friendly binary search used by the placeholder registry. `no_std`. |
| `aozora-lexer` | Original 7-phase classifier pipeline. Spec backbone; emits the same diagnostics that the fused engine does. |
| `aozora-lex` | Streaming orchestrator over the lexer + scan. Front door for the public crate. |
| `aozora-render` | HTML and canonical-serialisation walkers. Single O(n) tree pass each; no allocation outside the output buffer. |
| `aozora` | Public facade. `Document::parse() -> AozoraTree<'_>`, `tree.to_html()`, `tree.serialize()`, `tree.diagnostics()`. |
| `aozora-cli` | The `aozora` binary (`check` / `fmt` / `render`). |
| `aozora-ffi` | C ABI driver. Opaque handles + JSON-encoded structured data. Locally relaxes `unsafe_code`; every block carries a `// SAFETY:` comment. |
| `aozora-wasm` | `wasm32-unknown-unknown` target with `wasm-bindgen` exports. |
| `aozora-py` | PyO3 binding shipped via `maturin`. |
| `aozora-bench` | Criterion + corpus-driven probes. Source of the PGO training data. |
| `aozora-corpus` | Corpus source abstraction (ZSTD-archived, BLAKE3-pinned). Dev-only. |
| `aozora-test-utils` | Shared proptest strategies. Dev-only. |
| `aozora-trace` | DWARF symbolicator + samply gecko-trace loader. Dev-only. |
| `aozora-xtask` | Host-side dev tooling (samply wrapper, trace analysis, corpus pack/unpack). Not on the `just build` path. |

## Borrowed-arena AST

`Document` owns a [`bumpalo::Bump`](https://docs.rs/bumpalo) arena and
the source `Box<str>`. `AozoraTree<'a>` borrows from both via `&self`:

```rust
let doc = aozora::Document::new(source);
let tree = doc.parse();           // AozoraTree<'_> bound to doc's lifetime
let html = tree.to_html();        // walks the borrow, allocates only the output
drop(doc);                        // releases every node in one Bump::reset
```

Every node — `AozoraNode<'src>`, `Ruby<'src>`, `Bouten<'src>`,
`Kaeriten<'src>`, … — borrows its text content directly from the
source buffer. Where the same byte sequence repeats (e.g. shared ruby
target characters), an [`Interner`](https://docs.rs/aozora-syntax)
deduplicates the references so the in-arena footprint stays close to
the source size.

There is no owned-AST mirror. Consumers that need an owned
representation walk the tree once and emit their own structure (HTML,
serialised text, IR for an editor backend).

## SIMD scan strategy

`aozora-scan` provides the byte-level multi-pattern scanner used by
the lexer's tokenize phase. Three backends ship in the crate, and the
correct one is selected at compile time per target:

- **Teddy** (the [Hyperscan](https://intel.github.io/hyperscan/)
  short-string algorithm via `aho-corasick`'s `packed::teddy`) for
  x86_64 with AVX2. Fastest on Japanese text where every codepoint
  spans 3 bytes and naive leading-byte scans saturate.
- **Structural bitmap** for stable cross-architecture fallback when
  the target lacks the SIMD width Teddy needs.
- **DFA fallback** built on `regex-automata` for portability and
  for `wasm32` builds (until `wasm-simd` lands in the workspace).

Each backend produces the same `(offset, TriggerKind)` stream; the
lexer cannot tell which one ran. Selection happens behind a
runtime-dispatched trait so a single binary can carry the SIMD path
and a portable fallback.

## Lint and safety policy

The workspace `[workspace.lints]` block in [`Cargo.toml`](../Cargo.toml)
sets the gates:

- `unsafe_code = "forbid"` at the workspace level.
- `dead_code = "deny"`, `non_ascii_idents = "deny"`.
- `clippy::pedantic`, `clippy::nursery`, `clippy::cargo` enabled.
- `rustdoc::broken_intra_doc_links = "warn"`.

Three crates locally relax `unsafe_code`:

| Crate | Reason |
|---|---|
| `aozora-ffi` | C ABI surface (`extern "C"`, raw pointers). Each block carries a `// SAFETY:` comment justifying it. `#[deny(unsafe_op_in_unsafe_fn)]` keeps the per-block hygiene. |
| `aozora-scan` | Aligned-load SIMD intrinsics. |
| `aozora-xtask` | `perf_event_open(2)` and other host-only system calls. |

`#[allow(...)]` is a last resort. Where a clippy lint genuinely
clashes with intent, the carve-out carries `reason = "..."` and is
visible in `git blame`. The `just strict-code` recipe rejects
unjustified allows, bare `TODO`s, and stray `println!`/`dbg!`
outside `build.rs` and the CLI binary.

## Test strategy

aozora aims for **C1 100% branch coverage** as a goal, not a ceiling.
Coverage alone does not catch every bug, so the same invariants are
exercised from several angles:

1. **Spec cases** under [`spec/aozora/cases/*.json`](../spec/aozora/cases/).
   Each entry pins `(input, html, serialise)` for round-trip + render
   equality. Read by both the unit tests and the property harness.
2. **Property tests** with `proptest`, generators in
   [`crates/aozora-test-utils`](../crates/aozora-test-utils). Run
   under `just prop` (CI) and `just prop-deep` (release).
3. **Corpus sweep** — every document in `AOZORA_CORPUS_ROOT` must
   parse without panicking and round-trip through `parse ∘ serialize`
   identically (after the lexer's sanitize pass).
4. **Golden fixture** at
   [`spec/aozora/fixtures/56656/`](../spec/aozora/fixtures/56656/) —
   the Japanese translation of *Crime and Punishment* (Aozora Bunko
   card 56656) is the Tier-A acceptance gate. It exercises 1000+
   ruby annotations, forward-reference bouten, JIS X 0213 gaiji, and
   accent decomposition edge cases.
5. **Fuzz harness** (`cargo fuzz`) over parse, render, and Shift_JIS
   decode targets.
6. **Sanitizers** — `just` recipes for Miri (UB on FFI / scan
   intrinsics), TSan (data races in the parallel corpus loader),
   ASan (heap correctness).

Branch coverage is measured by `cargo llvm-cov`; CI gates on the
result.

## Multi-target deployment

The same `aozora` core powers four delivery shapes:

| Shape | Crate | How it ships |
|---|---|---|
| CLI | `aozora-cli` | tar.gz / zip in GitHub Releases for linux x86_64, macos arm64, windows x86_64. |
| Rust library | `aozora` | git dependency today; crates.io publication tracks the 1.0 API freeze. |
| WASM (browser / Node) | `aozora-wasm` | `wasm-pack build --target web`; the `.wasm` artifact has a 500 KiB size budget after `wasm-opt -O3`. |
| C ABI | `aozora-ffi` | `cdylib` + `staticlib` with a `cbindgen`-generated header. Opaque-handle API, JSON-encoded structured data. |
| Python wheel | `aozora-py` | `maturin develop` / `maturin build --release`, gated on the `extension-module` feature so plain `cargo build` still succeeds without Python headers. |

All drivers share the same diagnostic schema (a JSON projection
mirrored across `aozora-ffi`, `aozora-wasm`, and `aozora-py`), so a
polyglot consumer can switch between FFI shapes without semantic
change.

## Related projects

aozora is the lower of three layers:

- **[`P4suta/aozora-tools`](https://github.com/P4suta/aozora-tools)** —
  authoring environment: formatter CLI, LSP server, tree-sitter
  grammar, VS Code extension. Consumes this crate via a git tag.
- **[`P4suta/afm`](https://github.com/P4suta/afm)** — CommonMark + GFM
  + 青空文庫記法 integrated Markdown dialect. Sits on top of this
  parser; this repository deliberately stays Markdown-free so the
  pure 青空文庫 surface area never accumulates Markdown coupling.
- **[`P4suta/aozora`](https://github.com/P4suta/aozora)** — this
  repository, the pure 青空文庫記法 parser.
