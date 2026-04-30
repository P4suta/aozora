# SIMD scanner backends

Phase 1 of the lexer is a multi-pattern byte scan: find every
occurrence of the seven trigger bytes (`｜《》※［］　`) in the
source. On a typical Japanese corpus document — where every
codepoint is a 3-byte UTF-8 sequence and no trigger byte appears
more than once per kilobyte — the *scan* dominates the *interpretation*
by an order of magnitude. So this is the place where SIMD pays for
itself.

`aozora-scan` ships three backends, one of which is selected per
target at compile time:

| Backend | Target | Throughput (corpus) | Selection |
|---|---|---|---|
| Teddy | x86_64 + AVX2 | ~12 GB/s | first choice when AVX2 is available |
| Hoehrmann DFA | portable | ~3.5 GB/s | x86_64 fallback, native arm64, etc. |
| Memchr-multi | wasm32 | ~1.2 GB/s | wasm32 until the SIMD proposal lands |

Each backend produces the same `(offset, TriggerKind)` stream; the
lexer cannot tell which one ran. Selection happens behind a
runtime-dispatched trait so a single binary can carry both the SIMD
fast path and a portable fallback.

## Backend 1: Teddy (Hyperscan-style packed)

Teddy is the small-string multi-pattern algorithm from Intel's
[Hyperscan](https://intel.github.io/hyperscan/). The
`aho-corasick` crate ships a `packed::teddy` implementation that
aozora calls into directly.

**Why Teddy here:**

- The trigger set is small (7 patterns) and short (1 char each in
  full-width form, 3 bytes in UTF-8). Teddy's regime is *exactly*
  N small patterns where N ≤ 64 — ours has 7.
- The patterns share no common prefix structure (they're distinct
  full-width punctuation), so a Boyer-Moore-style suffix table
  doesn't help.
- AVX2 lets Teddy compare 32 bytes per cycle against the packed
  shuffle table, and our patterns fit cleanly into that lane width.

**Why not just memchr-multi (the obvious upgrade):**

`memchr3` does scan for up to 3 bytes simultaneously — but our
trigger set is 7 patterns × 3 bytes = 21 raw bytes, which would
require seven separate `memchr` passes (one per pattern). Each pass
streams the whole source. Teddy does one pass for all seven
patterns. The arithmetic favours Teddy by ~3.5×.

**Why not memchr's own packed-pattern path:**

`memchr` does have a packed multi-pattern API now, but it tops out
at ~5 GB/s on our workload because it goes through a generic 16-byte
SSE2 lane. Teddy's AVX2 32-byte lane — combined with `aho-corasick`'s
shuffle-table compilation — wins on the corpus by 2.5×.

## Backend 2: Hoehrmann-style multi-pattern DFA

For targets that lack AVX2 (older x86_64, native arm64 on some
runners, Alpine builds) the fallback is a byte-DFA built by
`regex-automata`'s `dense::Builder`. Hoehrmann's design — single-byte
transitions, no backtracking, table-driven — gives `O(1)` per byte
with no SIMD requirement.

**Why Hoehrmann-style over Aho-Corasick textbook NFA:**

Aho-Corasick at runtime is an NFA with failure transitions; each
mismatched byte may walk a chain of failure links before consuming
the next input byte. Hoehrmann compiles those failure links into
the dense table at build time, so every byte consumes exactly one
table lookup. For a small pattern set that fits in cache, the dense
table is faster than the NFA representation by 2×.

**Why a DFA over a hand-rolled state machine:**

`regex-automata` gives us a battle-tested table compiler with
correctness guarantees (panics from malformed transitions are
impossible) and the same crate handles the build-time DFA →
serialised-table flow if we ever want to ship the table as a static
asset. Hand-rolling buys nothing here — the patterns are small
enough that the compiler-emitted code generation isn't the bottleneck.

## Backend 3: memchr-multi (wasm32)

`wasm32-unknown-unknown` doesn't yet have AVX2 (and even after
`wasm-simd` lands, the lane width is 16 bytes — which would put it
between Teddy and the DFA). Until the workspace targets `wasm-simd`,
the wasm build uses `memchr`'s portable multi-pattern path:

- `memchr3` for the three single-byte open / close triggers,
- a follow-up scan for the multi-byte `｜《》※［］` UTF-8
  sequences (these expand to 3-byte each).

Throughput is lower (~1.2 GB/s) but the WASM bundle stays small —
no need to ship a Teddy table or a `regex-automata` DFA in the
500 KiB-budgeted wasm artifact.

## Backend selection

```rust
pub fn best_scanner_name() -> &'static str {
    if is_x86_feature_detected!("avx2") {
        "teddy"
    } else if cfg!(target_arch = "wasm32") {
        "memchr-multi"
    } else {
        "hoehrmann-dfa"
    }
}
```

Runtime detection (not compile-time `cfg!`) so a single x86_64
binary works on AVX2-less CPUs without recompilation.

The dispatch goes through a `&'static dyn Scanner` trait object;
the indirect call is hoisted out of the inner loop in the lexer's
Phase 2, so the trait dispatch is paid once per `Document::parse`,
not per byte.

## Why a runtime dispatch over per-target binaries?

Two reasons.

1. **Distribution.** Shipping one binary that adapts to its host is
   simpler than shipping `aozora-x86_64-avx2` and `aozora-x86_64`
   separately. The release pipeline only has to manage three
   archives (linux-gnu, darwin-arm64, windows-msvc), not six.
2. **Container portability.** `docker run --platform linux/amd64`
   on an arm64 Mac (Rosetta) lands on x86_64 *without* AVX2 —
   runtime detection picks the DFA backend silently. A
   compile-time-only build would crash with `SIGILL` on first
   trigger byte.

The cost is a single indirect call per parse; the win is that the
distribution surface stays minimal.

## Verifying the scanner is firing

```rust
println!("{}", aozora_scan::best_scanner_name());
// "teddy" | "hoehrmann-dfa" | "memchr-multi"
```

Or under samply, look for one of:

- `aozora_scan::backends::teddy::scan_offsets` — Teddy is firing.
- `aozora_scan::backends::dfa::scan_offsets` — Hoehrmann fallback.
- `memchr::arch::*::scan` — memchr's own internal SIMD; the
  scalar / wasm path is firing.

See [Performance → Profiling with samply](../perf/samply.md) for
the full workflow.

## See also

- [Pipeline overview](pipeline.md)
- [Seven-phase lexer](lexer.md) — Phase 1 fits in here.
