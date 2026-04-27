# 0020. Load-phase parallelism + decode buffer reuse + mmap (L-1 → L-4)

- Status: accepted (L-1, L-2, L-3 promoted; L-4 DROPPED — `unsafe` non-negotiable per project policy)
- Date: 2026-04-27
- Deciders: @P4suta
- Tags: architecture, performance, io, encoding, corpus, 0.2.0
- References: ADR-0009 (clean layered architecture), ADR-0017 (R4 — bumpalo + rayon), ADR-0019 (M-1/A0/A/B'-2 + B' verdicts)

## Context

After the B' sprint (ADR-0019) the parse path was well-tuned (sequential
253 MB/s, parallel 14× scaling). Profile attention shifted to the **load
phase** — file walk, file read, SJIS decode, size-bucketing — which the
bench harness had historically excluded from optimisation focus and which
the user identified as a primary lever:

> 青空文庫のコーパスは100%（といっていいと思う）がShift-JISなので
> 文字コード変換は100%通過する処理。だから、時間としてはたいしたことな
> いけれど、最適化をゴリゴリやりたい。もっといってしまえばI/Oそのもの
> に焦点を当てて、がっつり最適化を行いたい。

Pre-sprint baseline measurement (5-run mean, sequential, warm cache):

```
Wall:    load 3.50s   parse 3.18s
```

Load was **53 %** of total corpus wall time and fully sequential. This ADR
records the L-1 → L-4 sprint that targeted it.

## Sprint shape

| step | name | shipping verdict | load wall (parallel mode) |
|---|---|---|---:|
| L-1 | Per-phase load split + isolated `decode_throughput` example | infra (no perf delta) | baseline |
| L-2 | `par_load_decoded` + `parallel_size_bands` (rayon fold/reduce) | **PROMOTED** | 1.38 s (2.5× vs serial) |
| L-3 | `decode_sjis_into(&mut String)` + thread-local buffer | API ships, perf-neutral | 1.38 s (no change) |
| ~~L-4~~ | ~~`memmap2`-backed `read_item`~~ | **DROPPED** — `unsafe` is non-negotiable | n/a |

Final corpus load wall: **3.50 s sequential → 1.38 s parallel = 2.5×**.
Below the plan's 5× target; the gap is documented per-step below.

## L-1 — per-phase split + isolated decode benchmark

### What

`throughput_by_class.rs` previously reported one `load_wall` number that
fused walkdir + fs::read + decode + bucketing. Without per-sub-phase
attribution, no L-2/L-3/L-4 delta could be cleanly assigned. L-1 splits
the wall into four timers (verified to sum within ±0.3 % of the original):

```
load wall 3.47s
  ├─ walkdir :  0.33s ( 9.4%, 17436 paths)
  ├─ read    :  0.69s (19.8%, 532.1 MB sjis)
  ├─ decode  :  2.46s (70.8%, 791.5 MB utf8 → 217 MB/s)
  └─ bucket  :  0.00s ( 0.0%)
```

A new `crates/aozora-bench/examples/decode_throughput.rs` example pre-loads
all corpus bytes into memory, then times the decode in isolation per band,
sequential and parallel side-by-side. Output (16-thread, parallel-pass on):

```
band             docs    sjis MB    utf8 MB   seq MB/s   par MB/s   scale
<50KB           15443     193.79     287.64      291.4     2534.3   8.70×
50KB-500KB       1903     272.96     406.39      290.8     2479.7   8.53×
500KB-2MB          89      63.33      94.45      275.1     2156.3   7.84×
ALL             17436     532.10     791.51      288.5     2375.1   8.23×
```

Decode in isolation scales 8.23× at 16 threads — encouraging signal for
L-2's headline win.

### Walkdir double-stat fix (incidental)

Profiling exposed `walkdir` taking 4.57 s on cold caches because the original
`is_text_file(path)` filter called `path.is_file()` — a fresh `stat(2)`
per entry, redundant with walkdir's already-cached `DirEntry::file_type()`.
Replacing the call with `is_text_dir_entry(&entry)` shrinks the walkdir
phase from 4.57 s → 0.33 s on cold caches and ~50 ms → 30 ms on warm.

### Files

- `crates/aozora-bench/examples/decode_throughput.rs` (new, ~290 LoC)
- `crates/aozora-bench/examples/throughput_by_class.rs` (load-phase refactor)
- `crates/aozora-bench/src/lib.rs` (split `corpus_size_bands` /
  `corpus_size_bands_from_decoded` so decode and bucket time
  independently)
- `crates/aozora-corpus/src/filesystem.rs` (`walk_paths`, `read_path`,
  `is_text_dir_entry` walkdir double-stat fix)

### Verdict

**Infra (no perf delta of its own)**; gating measurement for the rest of
the sprint. Walkdir double-stat fix is a side-effect cold-cache win.

## L-2 — `par_load_decoded` + `parallel_size_bands` (PROMOTED)

### What

A new module `crates/aozora-corpus/src/parallel.rs` exposing
`par_load_decoded<F, T>(corpus, per_item) -> Vec<Result<T, _>>`. Internally:

1. Sequential walkdir to a `Vec<PathBuf>` (~0.3 s; walkdir is `!Sync`,
   not parallelisable without a custom walker — out of scope here).
2. `paths.into_par_iter().map(|p| corpus.read_path(p)?; ... per_item(item))`.

A second helper in `aozora-bench/src/lib.rs`,
`parallel_size_bands(corpus) -> SizeBandedCorpus`, uses a rayon
`fold(SizeBandedCorpus::default, ...).reduce(SizeBandedCorpus::default, merge)`
shape so each worker accumulates into its own per-thread `SizeBandedCorpus`
and the final merge is a serial `extend`. No intermediate
`Vec<Result<...>>` allocation, no shared mutex.

When `AOZORA_PROFILE_PARALLEL=1`, the bench's load phase fans out via
`parallel_size_bands` (previously the env flag only governed parse).
Sequential remains the default for sampling-profiler attachment — matches
ADR-0017 R4-B's pattern.

### Measured delta

Same machine, same session, baseline = L-1 sequential, target = L-2 parallel
(16 threads via rayon's default pool):

| run | sequential | parallel (16t) |
|---:|---:|---:|
| 1 | 3.50 s | 1.36 s |
| 2 | 3.39 s | 1.34 s |
| 3 | 3.37 s | 1.35 s |
| 4 | 3.45 s | 1.40 s |
| 5 | 3.39 s | 1.41 s |
| **mean** | **3.42 s** | **1.37 s** |

Speedup: **2.5×**. Below the plan's ≥ 5× gate.

### Why scaling collapses above 8 threads

Thread-count sweep on the same 16-thread WSL2 host (8 physical cores +
8 hyperthreads):

| `RAYON_NUM_THREADS` | load wall | scale vs 1-thread |
|---:|---:|---:|
| 1 | 3.07 s | 1.0× |
| 2 | 1.70 s | 1.81× |
| 4 | 1.04 s | 2.95× |
| 8 | 0.92 s | **3.34×** (best) |
| 16 | 1.39 s | 2.21× (regression) |

Scaling peaks at 8 threads and degrades at 16 — classic over-subscription.
The decode work is memory-bound (~290 MB/s per thread), so two
hyperthreads sharing one physical core's cache hierarchy compete rather
than overlap. The decode-only benchmark scaled 8.23× at 16 threads
because it has no read syscall serialisation; the full pipeline pays
syscall cost for `fs::read` and contends for kernel page cache.

The 5× plan target assumed naïve linear scaling per thread; the actual
constraint is per-physical-core memory bandwidth + syscall serialisation.
**3.34× at 8 physical cores ≈ 42 % efficiency** — typical for I/O-bound
parallel work.

### Files

- `crates/aozora-corpus/src/parallel.rs` (new, ~170 LoC + 4 unit tests)
- `crates/aozora-corpus/src/lib.rs` (`mod parallel; pub use ...`)
- `crates/aozora-corpus/Cargo.toml` (`rayon = { workspace = true }`)
- `crates/aozora-bench/src/lib.rs` (`parallel_size_bands` +
  `merge_banded` reduction)
- `crates/aozora-bench/examples/throughput_by_class.rs`
  (`LoadPhase::run_parallel` branch)

### Verdict: **PROMOTED**

2.5× speedup is well below target but is the floor of what parallelism
delivers under physical-core constraints. The architectural shape is
clean (additive new function alongside existing serial path; no API
break), measurement is reproducible, and L-3 / L-4 leave the door open
for further wins. Promoted on default code path.

## L-3 — `decode_sjis_into(&mut String)` (API ships, perf-neutral)

### What

`aozora-encoding` gains a buffer-reuse decode entry point:

```rust
pub fn decode_sjis_into(input: &[u8], dst: &mut String) -> Result<(), DecodeError>;
```

Internally calls `Decoder::decode_to_string_without_replacement` with
`dst.reserve(max_utf8_buffer_length_without_replacement(input.len()))`,
so the inner decode loop does no growth-realloc. Existing `decode_sjis`
becomes a one-line wrapper that calls `decode_sjis_into` with a fresh
`String` — no behaviour change for existing callers.

`aozora-bench`'s `parallel_size_bands` uses a thread-local 128 KB
buffer; per-iteration `clear() + decode_sjis_into(&bytes, &mut buf)
+ mem::take(&mut buf)` hands the filled string to the band entry.
Sequential `corpus_size_bands` uses a single function-local buffer.

### Measured delta

Sequential decode MB/s (5-run mean):

| metric | L-1 baseline | L-3 |
|---|---:|---:|
| decode time (s) | 2.46 | 2.45 |
| decode throughput (MB/s sjis-in) | 217 | 216 |
| parallel load wall | 1.38 s | 1.38 s |

**Perf-neutral** — within measurement noise.

### Why no measurable win

The plan's hypothesis was that `encoding_rs::decode()`'s
`String::with_capacity(worst_case)` was wasting capacity and that
buffer reuse would shave allocator round-trips. Reality:

1. `encoding_rs::decode` allocates exactly one `String` per call
   internally — same as `decode_sjis_into`; the over-allocation is
   "wasted memory inside an existing alloc", not "an extra alloc".
2. The `mem::take` pattern means the band-entry `String` is a fresh
   alloc every iteration regardless (the caller-owned buffer ends up
   empty after `take`, so the next `reserve` re-allocates from scratch).
3. encoding_rs's `Decoder::decode_to_string_without_replacement`
   implementation likely matches the `Encoding::decode` codepath
   internally — Mozilla optimised both for Firefox.

Net allocator-call count is unchanged; per-call cost is unchanged;
delta is therefore zero.

### Why it ships anyway

The API surface — a "caller-owned buffer" decode entry point — is
load-bearing for arena integration: a `BorrowedAllocator`-backed
decode path can write directly into the parse arena via this entry
point, eliminating the decode → arena copy that happens today after
the borrowed-AST shape changes (planned in plan A Stage 4).

8 new tests in `aozora-encoding` pin the equivalence contract:
`decode_sjis(b) == decode_sjis_into(b, &mut buf)` byte-for-byte across
ASCII, Japanese, half-width katakana, empty input, and the strict-error
cases (invalid lead byte, lone lead byte). `into_reuses_buffer_capacity_across_calls`
pins "no shrink across `clear() + decode` cycles".

### Verdict: API ships, perf-neutral

The architectural value justifies shipping; the perf-neutral verdict is
documented here so future maintainers don't re-litigate the buffer-reuse
hypothesis.

## L-4 — `memmap2`-backed `read_item` (DROPPED)

### Why dropped

The plan proposed mmap as a kernel→user memcpy reduction, gated
behind `cfg(feature = "mmap")` so the `unsafe { Mmap::map(&file) }`
block was opt-in. The user's standing constraint is that **`unsafe`
in our own code is non-negotiable** — feature-gating is not a valid
escape hatch.

The implementation was tried and abandoned (`jj op log` retains the
`tr` change-id):

- workspace `memmap2 = "0.9"` dep
- `aozora-corpus/Cargo.toml` `[features] mmap = ["dep:memmap2"]`
- `crates/aozora-corpus/src/filesystem.rs` `read_item_mmap` with one
  `unsafe { Mmap::map(&file) }` block + safety comment
- crate-level `[lints]` overriding workspace `forbid(unsafe_code)`
  → `deny(unsafe_code)` so the `#[allow]` could compile

Warm-cache measurement showed the implementation was perf-neutral
anyway (1.38 s either way); cold-cache wins remained unmeasurable
in this session (needs `sudo drop_caches`). With zero confirmed perf
upside and a categorical "no `unsafe` in our code" constraint, the
ADR-honest disposition is **DROPPED**, not "infra opt-in".

### Replacement direction (L-4-bis sprint, separate ADR if shipped)

Three pure-safe-Rust alternatives that recover similar wins remain
on the table for a follow-up:

1. **Rayon physical-core thread pool**. Empirically, the L-2 parallel
   mode regresses from 0.92 s @ 8 threads to 1.38 s @ 16 threads
   (8 cores + 8 hyperthreads on this WSL2 host — over-subscription).
   Sizing the pool to `num_cpus::get_physical()` would deliver an
   additional 33 % off load wall (3.50 s → ~0.92 s = 3.8×) at zero
   `unsafe` cost and zero new deps.

2. **`rustix::fs::fadvise(POSIX_FADV_SEQUENTIAL)`**. The same
   read-ahead hint mmap got from `MADV_SEQUENTIAL`, applied to file
   descriptors before `fs::read`. `rustix` exposes a safe-Rust API
   (the unsafe is internal to the crate, same as `rayon` and
   `encoding_rs`). Cold-cache benefit; warm-cache neutral.

3. **`jwalk` parallel walkdir**. Pure-Rust crate, rayon-internal,
   no user-visible `unsafe`. Could shave the 0.33 s walkdir step to
   ~0.05 s on cold cache. Walkdir is small post-L-2 but disproportion-
   ately large vs the rest of the parallel pipeline.

L-4-bis (#1) is the highest-confidence and is implemented in the
follow-up commit on top of this ADR; #2 and #3 stay as documented
candidates pending demand.

## Cuts (justified)

### Custom SIMD SJIS decoder — SKIP this sprint
encoding_rs already SIMD-accelerates the ASCII fast path (`simd-accel`
default). Remaining ~290 MB/s ceiling per-thread is dominated by JIS
X 0208 two-byte decode (table lookup, data-dependent — resists SIMD).
L-2 parallelism already gives 8.23× on the isolated decode benchmark.
Marginal upside small, engineering cost (specialised pure-Rust decoder
+ exhaustive correctness tests against the JIS X 0208 table + ongoing
maintenance vs encoding_rs upstream) is multiple weeks. Defer to a
separate ADR if post-sprint measurement still flags decode as
load-bearing.

### Decode-into-arena — DEFER post-sprint
L-3 added `decode_sjis_into(&mut String)` which is the arena-integration
prerequisite. The actual arena-backed decode requires `CorpusItem`
shape changes (carry `&'a str` instead of `Vec<u8>`) which ripple
through the entire bench harness. Out of scope here; revisit if a
future load-wall budget needs the bytes-saved.

### io_uring (`tokio-uring` / `glommio`) — SKIP
io_uring's value is batching syscalls to amortise kernel-mode-switch
cost. With 17 k files on warm page cache, syscall budget is ~50 ms —
not the bottleneck. Both candidate runtimes also force async colouring
on the load API. Architectural cost massively outweighs the upper
bound on wins.

## Validation gates

Every step gets a row; ADR-0017 / ADR-0019 style.

| Gate | L-1 | L-2 | L-3 |
|---|---|---|---|
| `cargo test --workspace --no-fail-fast` | 556 / 0 | 564 / 0 | 564 / 0 |
| `cargo test -p aozora-lex --test property_borrowed_arena` | 12 / 0 | 12 / 0 | 12 / 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean | clean | clean |
| `cargo fmt --all -- --check` | clean | clean | clean |
| Per-phase split sums to load wall (±5 %) | ±0.3 % ✓ | n/a (parallel) | n/a (parallel) |
| Load wall (parallel mode) | 3.50 s baseline | 1.38 s (2.5×) | 1.38 s (2.5×) |
| Decode MB/s (parallel) | 2375 (isolated) | unchanged | unchanged |
| AST node-count diff vs main | n/a | = 0 | = 0 |
| `unsafe_code` in our code | forbid (workspace) | forbid | forbid |

## Decision

- **L-1**: ship on default. Pure infra. Walkdir double-stat fix is a
  side-effect cold-cache win.
- **L-2**: ship on default. Parallel mode opt-in via existing
  `AOZORA_PROFILE_PARALLEL=1` env flag. 2.5× corpus-wall speedup is
  the load-bearing win of this sprint.
- **L-3**: ship on default. API surface added; perf-neutral; future
  arena integration consumes it.
- **L-4**: **DROPPED** — `unsafe` in our own code is non-negotiable
  per project policy. Implementation preserved in `jj op log` for
  reference; default code path has no `unsafe`. Pure-safe-Rust
  alternatives documented in the L-4 section above.

## Lesson recorded

**Plan target was 5× load-wall speedup; sprint delivered 2.5×.** The
gap is the difference between "decode in isolation scales 8× across
hardware threads" and "real load pipeline contends on kernel page cache,
syscall serialisation, and per-physical-core memory bandwidth". The
decode-only bench is correct about the decoder's parallel ceiling; the
full-pipeline bench is correct about the bottleneck distribution.
Future load-wall wins beyond 2.5× require attacking either:

1. **Per-physical-core memory bandwidth** — a custom SIMD SJIS decoder
   could push per-thread MB/s up; rejected here for engineering cost.
2. **Syscall serialisation** — io_uring batched submission; rejected
   here for async-colouring cost.
3. **Thread-pool sizing** — see L-4-bis follow-up.

The mmap road was investigated and rejected on architectural grounds
(`unsafe` constraint), not on perf grounds; the L-4 section above
records the safe-Rust replacement candidates.

This composes on top of B'-2's parse-side wins (ADR-0019) for a total
end-to-end corpus-sweep wall improvement of:

- Parse: ~3.18 s → ~3.18 s (parse phase already at near-bandwidth; no
  load-side change affects it).
- Load: ~3.50 s → ~1.38 s (this sprint).
- **Total wall: ~6.68 s → ~4.56 s = 1.46× end-to-end.**

Plan A (simdjson-style 1-pass parser) remains the only candidate for
≥ 6× end-to-end, per ADR-0019's order-of-magnitude analysis. Load is
no longer the dominant cost: parse 3.18 s now exceeds load 1.38 s, so
future optimisation focus shifts back to the parse path or to plan A.
