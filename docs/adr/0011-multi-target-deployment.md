# 0011. Multi-target deployment: WASM + C ABI + Python

- Status: accepted
- Date: 2026-04-26
- Deciders: @P4suta
- Tags: architecture, distribution, ffi, wasm, python, 0.2.0

## Context

`aozora` 0.1 was Rust-native only: the parser shipped as
`aozora-parser` on crates.io and the only consumer surface was
"Rust binary calls `aozora_parser::parse(&str)`." Three real-world
deployment paths were therefore blocked:

1. **Browser embedding** — Aozora viewer apps (HTML5 readers,
   sentence-by-sentence study tools, kana annotators) want to parse
   directly in the page rather than round-tripping to a server.
2. **Python data-science integration** — corpus linguistics
   workflows live in pandas / spaCy / Sudachi pipelines that already
   speak Python; making researchers shell out to a Rust binary is
   friction.
3. **Polyglot embedding** — Ruby web apps, Node tools, Go CLIs, JVM
   indexers that want the same parsed-Aozora output without a
   per-language fork of the recogniser.

The 0.2.0 architecture (ADR-0009) designs the surface layer to
support these paths as **first-class siblings** of the native CLI,
not as afterthoughts.

## Decision

Ship four driver crates in `crates/aozora-{cli,wasm,ffi,py}/`,
each a thin wrapper over the public meta crate `aozora` whose
[`Document`] / [`AozoraTree`] surface matches across all of them.

| crate         | host runtime         | distribution         | gating                                        |
|---------------|----------------------|----------------------|-----------------------------------------------|
| `aozora-cli`  | native               | `cargo install`      | always                                        |
| `aozora-wasm` | browser / Node / WASI| `wasm-pack` → npm    | `wasm-bindgen` deps gated on `cfg(target_arch = "wasm32")` |
| `aozora-ffi`  | C ABI consumers      | release `.so`/`.dylib` + `cbindgen` header | always (cdylib + staticlib `crate-type`)      |
| `aozora-py`   | Python 3.x           | `maturin build` → PyPI | `pyo3` dep behind `extension-module` feature  |

Crucially:

- **All four drivers re-use the same `diagnostics_json_view`
  projection** (originally defined in `aozora-wasm`, mirrored by
  `aozora-ffi`, called by `aozora-py`). Polyglot consumers see one
  diagnostic schema across hosts.
- **`unsafe` is locally relaxed** in `aozora-ffi` (C ABI requires
  it) and `aozora-scan/backends/avx2.rs` (SIMD intrinsics). All
  other crates retain the workspace `unsafe_code = "forbid"`.
- **Build matrix is independent** — host (`x86_64`, `aarch64`),
  WASM (`simd128` + no-simd fallback), Python (3.11+), and C ABI
  (gcc / msvc) each succeed without each other's tooling.
- The C ABI smoke test (`crates/aozora-ffi/tests/c_smoke/`) runs as
  part of the workspace verification gate; WASM and Python smokes
  are CI-only because they need extra toolchain installation.

## Consequences

**Easier**:
- Adding a new driver (e.g., `aozora-jni` for JVM, `aozora-erlang`
  for BEAM) is one new crate that follows the established pattern.
- A Python user `pip install aozora` and a JS user
  `npm install aozora-wasm` see identical parse output for the same
  source.
- The same diagnostic schema simplifies polyglot pipelines that
  collect diagnostics from multiple hosts (e.g., a JS frontend +
  Python backend that both parse the same Aozora file).
- WASM bundle size budget (≤ 500 KiB after `wasm-opt -O3
  --enable-simd`) is enforced by the verification plan.

**Harder / accepted cost**:
- CI matrix grows: native build × native test × WASM build × WASM
  test × Python build × Python test × FFI smoke. Mitigated by the
  fact that the host-build path catches ~95% of regressions
  before any cross-target work runs.
- Each driver crate adds a small `serde_json` dep (the diagnostics
  projection). Acceptable: the driver crates are end-of-pipe
  consumers, never libraries other libraries depend on.
- The `unsafe_code` quarantine documentation needs to be honoured
  in code review. Made enforceable by per-crate `[lints.rust]`
  overrides in `Cargo.toml` rather than ad-hoc `#![allow]`
  attributes scattered through source files.

## Alternatives considered

- **Single mega-crate exposing all surfaces via cargo features**:
  rejected — feature combinations explode (5 features → 32
  combinations to test) and consumers transitively depend on
  tooling they don't use (e.g., a WASM consumer pulls in pyo3-build
  configuration just because the feature exists).
- **Code generation (e.g., uniffi)**: uniffi would auto-generate
  the Python + Swift + Kotlin bindings from a UDL definition.
  Attractive but locks the API into uniffi's idiom set; rejected
  for 0.2.0 because each driver is small enough (200-400 LOC) that
  hand-writing them keeps the API per-host idiomatic. Reserve
  re-evaluation for 0.3.x if the maintenance cost grows.
- **WASM-only via "wasm runs everywhere" (wasmtime, wasmer)**:
  rejected — the embedding-cost overhead of a WASM runtime in a
  Python or C process is a 5-50× perf tax. Native FFI / PyO3 keeps
  the parse cost at ~96 MB/s per core, which is the entire point
  of writing it in Rust.

## References

- Plan file: `/home/yasunobu/.claude/plans/jazzy-jingling-gizmo.md`
  — Move 4 deliverables and per-crate verification gates
- ADR-0009 (Clean layered architecture) — surface-layer design
  this ADR's drivers consume
- ADR-0010 (Zero-copy AST + observable equivalence) — the data
  shape every driver crate hands to its host language
- `crates/aozora-ffi/tests/c_smoke/run.sh` — reference C consumer
  exercising the full handle lifecycle
