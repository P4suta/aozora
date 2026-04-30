# Benchmarks (criterion)

aozora ships two layers of perf measurement:

- **Criterion microbenchmarks** in `crates/aozora-lex/benches/` and
  `crates/aozora-render/benches/`. Reproducible per-function timings
  with statistical confidence intervals.
- **Corpus probes** in `crates/aozora-bench/examples/`. Each probe is
  a `cargo run --release --example <name>` binary that reports
  per-band statistics across a real corpus.

## Criterion microbenchmarks

Run a specific bench:

```sh
cargo bench -p aozora-lex --bench tokenize_compare
cargo bench -p aozora-render --bench html_emit
```

Criterion writes HTML reports under `target/criterion/`. Each bench
reports throughput in MB/s, ns/byte, and a confidence interval; the
HTML reports include violin plots that surface multi-modal latency
distributions (which often indicate cache-line or page-fault
effects we'd otherwise miss).

### Why criterion over `#[bench]`

Three reasons.

1. **Statistical rigour.** `#[bench]` reports the minimum of N
   iterations; criterion fits a model and reports a confidence
   interval. The minimum is a known-bad estimator on a system with
   any noise (which is every real machine).
2. **Iteration count auto-tuning.** Criterion picks the iteration
   count to reach a target precision; `#[bench]` requires a
   hand-picked count.
3. **Stability.** `#[bench]` is unstable Rust, only works on
   nightly. Criterion is stable Rust.

## Corpus probes

Each probe under `crates/aozora-bench/examples/` reports a different
slice of the workload. All read `AOZORA_CORPUS_ROOT`; most accept
`AOZORA_PROFILE_LIMIT=N` to cap the sweep.

| Probe | Question it answers | Output shape |
|---|---|---|
| `throughput_by_class` | Per-band MB/s for `lex_into_arena` | 4-band table + p50 / p90 / p99 / max + ns/byte |
| `phase_breakdown` | Per-phase ms for sanitize / tokenize / pair / classify | per-doc latencies + top-5 worst classify / sanitize |
| `latency_histogram` | Log-bucketed latency distribution per phase | bar histogram, 10 buckets, 1 µs … 1 s |
| `pathological_probe` | Single-doc 100-iter avg per phase | tight per-call numbers; takes `AOZORA_PROBE_DOC` for any corpus path |
| `phase0_breakdown` | Per-sub-pass cost inside Phase 0 sanitize | bom_strip / crlf / rule_isolate / accent / pua_scan |
| `phase0_impact` | Does Phase 0 sub-pass firing change Phase 1 cost? | bucketed by which sub-passes fired |
| `phase3_subsystems` | Per-recogniser ms inside classify | requires `--features instrument` |
| `diagnostic_distribution` | What fraction of docs emit diagnostics? | histogram by diag count; latency-by-diag-bucket |
| `allocator_pressure` | Arena bytes / source byte ratio + intern dedup | per-doc histograms |
| `fused_vs_materialized` | Does the deforestation actually win? | per-band gap % between fused (`lex_into_arena`) and materialized (per-phase collect) |
| `intern_dedup_ratio` | How well does the interner dedup short strings? | corpus-aggregate (cache + table) / calls |
| `render_hot_path` | Per-band MB/s for HTML render | 4-band MB/s + render/parse ratio + out/in size ratio |

Each probe is invoked directly:

```sh
AOZORA_CORPUS_ROOT=… cargo run --release --example <name> -p aozora-bench
```

For `phase3_subsystems`, build with the instrumentation feature:

```sh
AOZORA_CORPUS_ROOT=… cargo run --release --features instrument \
  --example phase3_subsystems -p aozora-bench
```

## Why corpus probes *and* criterion benches?

Different questions.

- **Criterion** answers "is function `X` faster after my change?"
  on a fixed input. Microscopic, reproducible, the right tool for
  optimising a single hot loop.
- **Corpus probes** answer "is the parser faster on the *real*
  Aozora Bunko catalogue after my change?" Macroscopic, includes
  every distribution effect (small-doc dispatch overhead, large-doc
  cache pressure, gaiji-density variation). The right tool for
  validating a perf PR end-to-end.

A perf PR that wins on criterion but loses on the corpus is
suspicious — usually it's optimised the small-input path at the
cost of the large-input path. The corpus probe catches it.

## Phase 3 instrumentation caveat

`phase3-instrument` wraps every recogniser entry in a
`SubsystemGuard` that calls `Instant::now()` on construction +
drop. For the dominant inner-loop recognisers this adds enough
overhead that the **report's own timing is significantly skewed**.

Use the instrumentation to compare *relative* costs between
subsystems, not as an absolute number. For absolute numbers, run
`phase_breakdown` (no instrumentation).

## Where to look in samply

If a corpus probe regresses, sample-profile the same workload:

```sh
AOZORA_CORPUS_ROOT=… just samply-corpus 5
samply load /tmp/aozora-corpus-<ts>.json.gz
# or
just trace-rollup /tmp/aozora-corpus-<ts>.json.gz
```

The `trace-rollup` analysis groups samples into aozora's built-in
categories (Phase 0/1/2/3/4 + corpus_load + intern + alloc + …) so
a regression's category jumps out at a glance.

## See also

- [Profiling with samply](samply.md) — the trace workflow.
- [Corpus sweeps](corpus.md) — what `AOZORA_CORPUS_ROOT` should
  point at.
- [Release profile & PGO](profile.md) — the build profile that
  produces these numbers.
