# ADR-0007: Concurrency test strategy — what we adopt and what we
explicitly skip

## Status

Accepted (2026-04-25).

## Context

`aozora-parser`'s 3-layer parallelisation (`parse_parallel` +
segment-aware merge, ≥ 512 KB threshold, batched rayon par_iter)
introduced a non-trivial concurrency surface. Bugs in concurrent
code typically don't appear in unit tests; they surface under load,
specific scheduling, or rare interleavings, and become difficult to
reproduce post-hoc. The failure mode we most want to avoid is "the
LSP went weird in production, no one can repro, the logs say
nothing".

To prevent that we need a test strategy that:

1. Catches typical concurrency bugs in CI.
2. Lets a third party reading the logs reconstruct what happened.
3. Stays understandable six months later when the original author
   has forgotten the details.
4. Each test answers "what does this guard?" in one sentence.

This ADR records *which tools and techniques we adopted* and —
just as importantly — *which we deliberately chose not to* so the
choices don't get re-litigated on every refactor.

## Decision

### Adopted

| Layer | Technique | Where |
|---|---|---|
| **Stress** | `std::thread::spawn × N × K` | `aozora-parser/tests/concurrent_stress.rs` |
| **Property** | proptest with deterministic boundary cases + 1-thread pool + repetition checks | `aozora-parser/tests/property_parallel.rs` |
| **Bug-pattern regression** | Named tests with `Invariant: ... | Reproduces: ...` doc comments | `aozora-tools/aozora-lsp/tests/concurrency_regressions.rs` |
| **Schedule exploration** | Shuttle randomized scheduler over `Arc<DashMap<…>>` lifecycle | `aozora-tools/aozora-lsp/tests/shuttle_doc_state.rs` (gated `--features shuttle-tests`) |
| **Sanitiser scaffolding** | TSan / Miri / ASan via `scripts/sanitizers.sh` | On-demand + nightly cron |

### Explicitly NOT adopted

#### Loom

Loom exhaustively explores every interleaving of code that uses
`std::sync` primitives. Its sweet spot is verifying *custom
sync primitives*: a hand-rolled Mutex, a wait-free queue, a
lock-free channel.

We don't have those. Our concurrency surface is built from:

- `dashmap::DashMap` (extensively model-checked by maintainers)
- `rayon::par_iter` (battle-tested in production at scale)
- `tokio` async runtime (idem)
- `ahash::RandomState` (per-instance state, no cross-thread sharing)

Loom would explore mostly library-internal schedules without finding
bugs in our code. Adopting it would add CI time and a complex
dependency for low marginal value. **If a future refactor introduces
a custom sync primitive, revisit this decision.**

#### `cargo-fuzz`

Fuzzing is the right tool for input-shape bugs. We get equivalent
coverage from proptest (which already drives the
`property_parallel.rs` and `property_incremental.rs` harnesses) at
much lower operational cost — fuzz needs continuous-execution
infrastructure to find bugs, and a one-shot `cargo fuzz run` for 60
seconds finds nothing proptest hasn't.

If we ever build infrastructure to run fuzzers continuously (a
public corpus, CI cron with hours of fuzz time, OSS-Fuzz integration),
we can scaffold harnesses then. **Until then, adding empty fuzz
targets is worse than nothing**: they need maintenance, give a false
sense of coverage, and bit-rot.

#### OpenTelemetry / Prometheus push exporters

LSP is a client-resident process, not a long-running service. The
typical OTLP / Prom pull endpoints assume scrape-based observability;
LSP would need `did_close`-time push, which is what `Metrics::snapshot()`
already does via `tracing::info!`.

Future work can add a feature-flagged exporter without changing the
recording sites because `MetricsSnapshot` is `Serialize`.

## Consequences

### Positive

- The concurrency-test surface is named and bounded. New
  contributors can scan `tests/concurrency_regressions.rs` and read
  the `Invariant: ...` doc comments to learn what's already covered.
- Future refactors that add a custom sync primitive trigger an
  explicit revisit of "should we add Loom?" rather than a vague
  "we should probably check this somehow".
- TSan / Miri / ASan are wired up but opt-in, so PR latency stays
  low while still letting incident triage reach for them.

### Negative

- We accept the risk of a subtle interleaving bug that only Loom
  would have found. Our judgement is that this risk is small (we
  use only off-the-shelf primitives) and the cost of mitigating it
  via Loom is high (custom interleaving harnesses for primitives
  we didn't write).
- The Shuttle test depends on a feature flag (`shuttle-tests`) so
  default `cargo test` doesn't exercise it. Nightly cron must run
  explicitly with the feature.

## Verification

```sh
# PR-required (default cargo test)
cargo test --workspace
cargo clippy --workspace --tests --all-targets -- -D warnings

# Nightly cron
AOZORA_STRESS_K=10000 cargo test --test concurrent_stress
AOZORA_LSP_STRESS_K=10000 cargo test --test concurrent_lsp
cargo test -p aozora-lsp --features shuttle-tests --test shuttle_doc_state
scripts/sanitizers.sh tsan --filter concurrent
scripts/sanitizers.sh miri --filter property
```

## References

- `aozora-tools/docs/adr/0001-shuttle-segment-cache.md` — companion
  ADR describing what specifically Shuttle is set up to verify in
  the LSP backend.
- Plan file (off-tree, in author's home directory) — the original
  design conversation that produced this strategy.
