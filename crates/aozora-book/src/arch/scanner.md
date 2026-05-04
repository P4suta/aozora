# SIMD scanner backends

Phase 1 of the lexer is a multi-pattern byte scan: find every
occurrence of the 11 Aozora trigger characters (`｜《》＃※［］〔〕「」`)
in the source. On a typical Japanese corpus document — where every
codepoint is a 3-byte UTF-8 sequence and trigger characters appear
on the order of 1–2 % of bytes — the *scan* dominates the
*interpretation* by an order of magnitude. So this is the place
where SIMD pays for itself.

## Architecture: outer driver × inner kernel

`aozora-scan` ships a single algorithm — Hyperscan-style Teddy with
nibble LUTs — implemented once as a platform-agnostic outer driver
and plugged into per-ISA inner kernels. The split is the spine of
the crate:

- `crate::kernel::teddy` — algorithm side. Defines the const-built
  bucket LUTs (one bit per pattern; the 11 triggers fit comfortably
  in the 16-bit mask), the verify table, the `TeddyInner` trait
  every kernel implements, and `teddy_outer` — the platform-
  agnostic chunk loop + verify pass.
- `crate::arch::*` — platform side. One file per ISA; each
  implements `TeddyInner::lead_mask_chunk` using the appropriate
  16-byte LUT shuffle: `pshufb` on x86 SSSE3, `_mm256_shuffle_epi8`
  on AVX2, `vqtbl1q_u8` on NEON, `i8x16_swizzle` on WASM SIMD.

Adding a new SIMD ISA is one file under `arch/`. Adding a new
algorithm (e.g. SHIFT-OR baseline, AVX-512 64-byte chunk) is one
file under `kernel/`. The two axes never tangle.

## BackendChoice + static dispatch

[`BackendChoice`](https://docs.rs/aozora-scan/latest/aozora_scan/enum.BackendChoice.html)
is a `Copy` enum carrying one variant per inner kernel currently
compiled into the build. `BackendChoice::detect()` runs once at
process start, picks the fastest variant the host CPU supports
(cached in `OnceLock`), and the `match`-based
`BackendChoice::scan` gives **static dispatch** straight into the
monomorphised `teddy_outer<I>` instantiation. No `&dyn`, no virtual
call on the hot path.

Static dispatch is the whole point: a trait object cannot carry a
generic `S: OffsetSink` method, so a `&dyn`-based dispatcher would
force every parse to allocate a heap `Vec<u32>` and memcpy it into
the lex pipeline's bumpalo arena. The enum-and-match shape gives
us the same runtime-CPU adaptation a single binary needs without
that detour.

## Backends compiled into the build

| Variant | Target gate | Kernel size | Notes |
|---|---|---|---|
| `TeddyAvx2` | `x86_64` | 32-byte chunk | Production winner on every modern dev / CI host. `_mm256_shuffle_epi8` per-lane LUT shuffle. |
| `TeddySsse3` | `x86_64` | 16-byte chunk | Selected when AVX2 is unavailable but SSSE3 is. `_mm_shuffle_epi8` (`pshufb`). |
| `TeddyNeon` | `aarch64` | 16-byte chunk | aarch64 ABI mandates NEON, so always selected on that target. `vqtbl1q_u8`. |
| `TeddyWasm` | `wasm32` | 16-byte chunk | WASM SIMD128 baseline since 2022. `i8x16_swizzle`. |
| `ScalarTeddy` | always | 16-byte chunk, no SIMD | Pure-Rust reference; the `no_std` last-resort dispatch target and the proptest oracle for SIMD ports. |

[`NaiveScanner`](https://docs.rs/aozora-scan/latest/aozora_scan/struct.NaiveScanner.html)
(brute-force PHF walker) is `#[doc(hidden)]` — kept reachable for
the integration proptests and the bake-off bench, never the
dispatch target.

## Why a self-rolled Teddy

The previous production stack drove three external crates —
`aho_corasick::packed::teddy` (SSSE3-only), `regex_automata` (DFA),
hand-rolled simdjson-style structural bitmap (AVX2). Coverage gaps
forced redundant fallback code on every commit and the trio carried
~1.4 MB of compiled dependency surface.

Switching to a self-rolled Teddy:

- **One algorithm, four ISAs.** The outer driver is ~120 LOC; each
  ISA inner kernel is ~30 LOC. NEON / WASM SIMD ports compile
  natively rather than waiting on upstream `aho_corasick`.
- **No external SIMD deps.** `aho_corasick` and `regex_automata`
  are gone from the default dep tree. The `aozora-scan` build no
  longer pulls in `regex-automata`'s ~600 KB of state-table code.
- **One-bit-per-pattern bucket layout.** The 11 triggers fit in
  the lower 11 bits of a `u16`; we don't pay for the
  collision-verify pass Hyperscan's "fat-finger" packing requires.
- **`OffsetSink` visitor.** Every kernel writes through the same
  generic sink, so the lex pipeline's `BumpVec<'_, u32>` receives
  offsets directly from the SIMD inner loop — the legacy
  heap-allocate-then-memcpy detour is gone.

Every kernel cross-validates byte-identically against `NaiveScanner`
in proptest, both in-source (chunk-level) and in
[`tests/property_backend_equiv.rs`](https://github.com/P4suta/aozora/blob/main/crates/aozora-scan/tests/property_backend_equiv.rs)
(end-to-end across the workhorse fragment / pathological /
unicode-adversarial distributions).

## Verifying the scanner is firing

```rust
println!("{}", aozora_scan::BackendChoice::detect().name());
// "teddy-avx2" | "teddy-ssse3" | "teddy-neon" | "teddy-wasm" | "scalar-teddy"
```

Or under samply, look for one of the per-ISA inner kernels:

- `aozora_scan::arch::x86_64::lead_mask_chunk_avx2`
- `aozora_scan::arch::x86_64::lead_mask_chunk_ssse3`
- `aozora_scan::arch::aarch64::lead_mask_chunk_neon`
- `aozora_scan::arch::wasm32::lead_mask_chunk_wasm`
- `aozora_scan::kernel::teddy::ScalarTeddyKernel::lead_mask_chunk`

Their parent in the call tree is always
`aozora_scan::kernel::teddy::teddy_outer`, where the chunk loop
lives.

## See also

- [Pipeline overview](pipeline.md)
- [Four-phase lexer](lexer.md) — Phase 1 events fits in here.
