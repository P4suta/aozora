# 0006. Polyglot bindings via an Extism wasm plugin + schema-driven type generation

- Status: accepted
- Date: 2026-06-15
- Deciders: @P4suta
- Tags: bindings, wasm, ffi, wire, distribution

## Context

aozora ships first-party bindings for three ecosystems today: Python
(`aozora-py`, PyO3), npm/browser (`aozora-wasm`, wasm-bindgen) and a C
ABI (`aozora-ffi`, cbindgen). The natural pull is to keep going —
Go, Java, PHP, Ruby, and eventually "every language". Hand-writing and
maintaining a bespoke native binding per language does not scale: each
adds its own transport glue, its own type mirrors, and — worst of all —
its own `(OS × arch)` native-build-and-publish matrix (the C ABI path
already pays this in `release.yml`).

The forces in our favour: the binding surface is already a **narrow,
serialized waist**. Every driver funnels through `aozora::json` (the
single cross-driver authority), so the data crossing any language
boundary is just *text in → a versioned JSON envelope out*
(`{ "schema_version": 1, "data": [ … ] }`). That JSON is described by a
committed JSON Schema (`xtask schema`, drift-gated). The hard part of
multi-language support — agreeing a stable, machine-described contract —
is therefore already done. What remains is (a) a transport that reaches
many languages cheaply and (b) per-language types, which a schema can
generate mechanically.

## Decision

1. **Extism is the universal transport for *new* languages.** A new
   driver crate `aozora-extism` compiles to a single portable
   `wasm32-unknown-unknown` artifact (`aozora.wasm`) exposing
   `#[plugin_fn]` entry points (`to_html`, `serialize`,
   `diagnostics_json`, `nodes_json`, `pairs_json`,
   `container_pairs_json`, `schema_version`). Each is a thin wrapper that
   delegates to `aozora::json` — **no new serialization logic** — so its
   output is byte-identical to the FFI / WASM / PyO3 drivers. Any
   language with an Extism host SDK (Go / Java / PHP / Ruby / … ~15)
   loads the same bytes.

2. **Types are generated from the existing JSON Schema, for all
   languages at once.** `xtask types` is extended with a `quicktype`
   driver that consumes the committed `schema-*.json` and emits native
   types per target language, wired into the same dump/check drift-gate
   that already guards the TypeScript `.d.ts`.

3. **Native bindings are retained where they already exist.**
   `aozora-py` (PyO3) and `aozora-wasm` (wasm-bindgen) stay native: they
   are in-process, faster, already funnel through `aozora::json`, and
   already share `SCHEMA_VERSION`. The browser playground keeps
   wasm-bindgen unconditionally (Extism is a host-side runtime, not a
   browser one). "Formalizing" Python/Node means aligning them to the
   same schema/version discipline and type-generation step — not
   migrating their transport.

4. **The dev image carries the toolchain.** binaryen's `wasm-opt`
   (pinned upstream release, not the bulk-memory-incapable apt build) is
   baked into the dev image; `just extism-build` produces and optimizes
   the artifact in Docker.

## Consequences

- **One artifact replaces the `(OS × arch)` matrix.** The `.wasm` is
  built once, on Linux, in the existing Docker image; every host SDK on
  every platform loads identical bytes. This is the central win over the
  C ABI, whose `release.yml` matrix builds a native library per target.
- **A serialization round-trip is accepted** at the Extism boundary
  (text in, JSON bytes out) versus the zero-copy borrowed AST a native
  Rust consumer sees. For a parser whose interface is already "string →
  JSON" this cost is intrinsic, not new.
- **Each host gains one runtime dependency** — the Extism runtime —
  versus a bare native library. Acceptable for the long tail of
  languages where writing a native binding is the real cost.
- **Schema-version coupling becomes explicit.** The plugin embeds
  `SCHEMA_VERSION`; each host SDK asserts it at load via the
  `schema_version` export. A `SCHEMA_VERSION` bump forces regeneration of
  every language's types (drift-gate) and a coordinated SDK release; the
  wasm release and the SDK releases are version-locked.
- **Three transport mechanisms now coexist** (native PyO3, native
  wasm-bindgen, Extism). This is deliberate: native for the hot,
  already-paid-for ecosystems and the browser; Extism for breadth.

## Alternatives considered

**Per-language native FFI bindings (one bespoke crate per language).**
Best ergonomics and performance, but multiplies maintenance by N: each
language carries its own transport glue, type mirrors, and native-build
matrix. Rejected as the thing we are explicitly trying not to do.

**C ABI everywhere (extend `aozora-ffi` to every language).** The C ABI
already reaches any language via FFI, and the per-language wrapper is
small because the surface is narrow and JSON-shaped. But every consuming
language must still ship a native library for every `(OS × arch)` pair —
the matrix `release.yml` already demonstrates. Kept for in-process,
max-performance embedders, but not chosen as the breadth strategy.

**UniFFI (Mozilla).** Generates rich, idiomatic typed bindings from a
Rust interface for Kotlin/Swift/Python/Ruby (+ third-party Go/C#).
Excellent for mobile, but Rust-specific, still native (the `(OS × arch)`
matrix remains), and narrower in language coverage than
quicktype-from-schema + Extism. Rejected as the universal strategy;
could complement it for mobile later.

## References

- Plan: `~/.claude/plans/python-npm-go-java-php-ruby-kind-iverson.md`
- Single wire authority: `crates/aozora/src/json.rs` (`SCHEMA_VERSION`,
  `serialize_*`)
- Schema artefacts + codegen: `crates/aozora-xtask/src/schema.rs`,
  `crates/aozora-xtask/src/types.rs`, `crates/aozora-book/src/json/`
- Guest plugin: `crates/aozora-extism/`, build via `just extism-build`
- Type generation: `xtask types langs` (quicktype), drift-gated; the
  combined-schema build hoists `$defs` (see `aozora::json::envelope_schema`)
- First host SDK: `crates/aozora-go/` (pure-Go wazero), `just smoke-go`
- Distribution: `.github/workflows/publish-extism-wasm.yml` (one wasm →
  release asset)
- Related: ADR-0001 (zero parser hooks), ADR-0004 (lint/profile policy)
