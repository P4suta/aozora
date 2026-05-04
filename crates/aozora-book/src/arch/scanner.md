# SIMD scanner backends

Phase 1 of the lexer is a multi-pattern byte scan: find every
occurrence of the 11 Aozora trigger characters (`｜《》＃※［］〔〕「」`)
in the source. On a typical Japanese corpus document — where every
codepoint is a 3-byte UTF-8 sequence and trigger characters appear
on the order of 1–2 % of bytes — the *scan* dominates the
*interpretation* by an order of magnitude. So this is the place
where SIMD pays for itself.

`aozora-scan` ships several backends; one is selected per host at
runtime via CPU feature detection:

| Backend | Target / requirements | Role |
|---|---|---|
| Teddy | `feature = "std"` and SSSE3-capable host (built lazily via `aho_corasick::packed`) | First choice — Hyperscan-style fingerprint matcher |
| Structural bitmap (AVX2) | x86_64 with AVX2 | Production fallback when Teddy can't build but AVX2 is present |
| Hoehrmann DFA | `feature = "std"`, universal | Universal SIMD-free fallback (`regex_automata` dense byte DFA) |
| Naive (PHF walker) | always | `no_std` last resort and proptest cross-validation reference |

Each backend produces the same byte-offset stream; the lexer cannot
tell which one ran. Selection happens behind a runtime-dispatched
trait so a single binary can carry both the SIMD fast path and a
portable fallback. Cross-validation against the naive scanner is
pinned by proptest in every backend module.

## Backend 1: Teddy (Hyperscan-style packed)

Teddy is the small-string multi-pattern algorithm from Intel's
[Hyperscan](https://github.com/intel/hyperscan). The `aho-corasick`
crate ships a `packed::teddy` implementation that aozora calls into
directly.

**Why Teddy here:**

- The trigger set is small (11 patterns) and short (3 bytes each in
  UTF-8). Teddy's regime is *exactly* `N` small patterns where
  `N ≤ 64` — ours has 11.
- The patterns share no common prefix structure (they are distinct
  full-width punctuation), so a Boyer-Moore-style suffix table does
  not help.
- SSSE3's `pshufb` lets Teddy compare 16 bytes per cycle against the
  packed shuffle table; AVX2 widens that to 32 bytes per cycle when
  available.

**Why not just memchr-multi (the obvious upgrade):**

`memchr3` does scan for up to 3 bytes simultaneously — but our
trigger set has 4 distinct (lead, mid) byte pairs spanning 11 full
trigrams, which would require multiple memchr passes (one per lead
byte) followed by per-position trigram verification. Each pass
streams the whole source. Teddy does one pass for all 11 patterns.

## Backend 2: Structural bitmap (AVX2)

For x86_64 hosts that have AVX2 but where Teddy cannot build (no
SSSE3 feature exposed, or the runtime decides Teddy is not viable),
the production fallback is a simdjson-style two-byte (lead × mid)
candidate filter:

- `_mm256_cmpeq_epi8` for every distinct lead byte and every
  distinct mid byte.
- `OR`-fold the per-byte masks, `AND` adjacent windows to produce a
  candidate mask for the lead+mid 2-gram.
- `_mm256_movemask_epi8` projects the 32-byte chunk to a 32-bit
  candidate mask; Kernighan extraction yields the per-position
  offsets, each verified against the trigram PHF.

The two-byte filter is a strict superset of correct hits, so the
PHF verify ensures byte-identical output to Teddy.

## Backend 3: Hoehrmann-style multi-pattern DFA

For targets that lack AVX2 / SSSE3 (older x86_64, native arm64 on
some runners, Alpine builds without SSSE3 exposed) the universal
fallback is a byte-DFA built by `regex-automata`'s `dense::Builder`.
Hoehrmann's design — single-byte transitions, no backtracking,
table-driven — gives `O(1)` per byte with no SIMD requirement.

**Why Hoehrmann-style over Aho-Corasick textbook NFA:**

Aho-Corasick at runtime is an NFA with failure transitions; each
mismatched byte may walk a chain of failure links before consuming
the next input byte. Hoehrmann compiles those failure links into the
dense table at build time, so every byte consumes exactly one table
lookup. For a small pattern set that fits in cache, the dense table
is faster than the NFA representation.

## Backend 4: Naive (PHF walker)

`no_std`-clean walker that visits every byte and consults a
[`phf::Map`] at every potential trigger lead. Slower than the SIMD
backends but useful in two contexts:

- as the `cfg(not(feature = "std"))` last resort, so callers without
  an allocator still get a working scanner;
- as the **cross-validation reference**: every other backend runs a
  proptest comparing its output against this scanner over the
  Aozora-shaped input distribution.

## Backend selection

Dispatch order (best to worst), runtime-detected and cached via
`OnceLock`:

1. **Teddy** — built once and cached. Returns `None` on hosts
   without SSSE3, in which case we fall through.
2. **Structural bitmap** — `x86_64` + AVX2 only.
3. **Hoehrmann DFA** — universal SIMD-free fallback.
4. **Naive** — `no_std` last resort.

Runtime detection (not compile-time `cfg!`) so a single x86_64
binary works on AVX2-less CPUs without recompilation.

## Why a runtime dispatch over per-target binaries?

Two reasons.

1. **Distribution.** Shipping one binary that adapts to its host is
   simpler than shipping `aozora-x86_64-avx2` and `aozora-x86_64`
   separately. The release pipeline only has to manage three
   archives (linux-gnu, darwin-arm64, windows-msvc), not six.
2. **Container portability.** `docker run --platform linux/amd64`
   on an arm64 Mac (Rosetta) lands on x86_64 *without* AVX2 —
   runtime detection picks the next backend silently. A
   compile-time-only build would crash with `SIGILL` on first
   trigger byte.

The cost is a single indirect call per parse; the win is that the
distribution surface stays minimal.

## Verifying the scanner is firing

```rust
println!("{}", aozora_scan::best_scanner_name());
// "teddy" | "structural-bitmap" | "dfa" | "naive"
```

Or under samply, look for one of:

- `aozora_scan::backends::teddy::scan_offsets` — Teddy is firing.
- `aozora_scan::backends::structural_bitmap::scan_offsets` — AVX2
  fallback firing.
- `aozora_scan::backends::dfa::scan_offsets` — Hoehrmann fallback.
- `aozora_scan::naive::scan_offsets` — `no_std` last resort firing.

See [Performance → Profiling with samply](../perf/samply.md) for the
full workflow.

## See also

- [Pipeline overview](pipeline.md)
- [Four-phase lexer](lexer.md) — Phase 1 events fits in here.
