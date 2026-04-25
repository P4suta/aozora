# Scripts

On-demand tooling. PR-required gates live in the `cargo test` /
`cargo clippy` invocations checked into CI; these scripts cover the
heavier checks that belong on a nightly cron or local exploration.

## `pgo-build.sh`

Run the Profile-Guided Optimisation pipeline against the full
Aozora corpus and produce a release binary tuned for the workload.

Three required phases plus one optional fourth:

1. **instrumented build** — `cargo pgo build` compiles `aozora-cli`
   and the `profile_corpus` example with profiling instrumentation
2. **profile collection** — runs the instrumented binary three
   times against `$AOZORA_CORPUS_ROOT` to gather a stable profile
3. **optimised rebuild** — `cargo pgo optimize build` re-links with
   the collected profile baked in
4. *(optional)* **llvm-bolt post-link** — applies binary layout
   optimisation on top of the PGO output (Linux x86_64 only)

Requirements:

- `cargo install cargo-pgo`
- `rustup component add llvm-tools-preview`
- `AOZORA_CORPUS_ROOT` env var pointing at the corpus
- For phase 4: `llvm-bolt` (e.g. `sudo apt install llvm-bolt`)

Expected gain: 10-15% per LLVM project numbers; aozora-specific
measurement is in the script's final reporting block (it suggests
the `hyperfine` invocation that compares baseline vs PGO vs BOLT).

```sh
export AOZORA_CORPUS_ROOT=~/aozora-corpus/aozorabunko_text-master/cards
scripts/pgo-build.sh
```

## `sanitizers.sh`

Wrap nightly Rust + a sanitiser around `cargo test`. Three modes:

| Mode | Catches | Cost |
|---|---|---|
| `miri` | UB, alignment violations, dangling refs, data-race subset | 10–100× slower than `cargo test` |
| `tsan` | Data races (heisenbugs in concurrent code) | 2–10× slower; rebuilds std |
| `asan` | Use-after-free, double-free, OOB writes | ~3× slower; rebuilds std |

Examples:

```sh
# Default-strict run, full workspace
scripts/sanitizers.sh miri
scripts/sanitizers.sh tsan
scripts/sanitizers.sh asan

# Scope a filter to one test
scripts/sanitizers.sh tsan --filter concurrent_lsp
scripts/sanitizers.sh miri --filter property_parallel
```

The script auto-installs the `nightly` toolchain and the `miri`
component on first run. Nothing else is global state.

### When to run

- **Local pre-commit on concurrent changes** — touched
  `parallel.rs` / `backend.rs` / `segment_cache.rs`? run TSan
  before merging.
- **Nightly cron** — full TSan + Miri across the workspace.
- **Production incident triage** — Miri can sometimes reproduce a
  race more deterministically than the original conditions.

### Known limitations

- **Miri**: rejects most C dependencies (e.g. `bzip2`-sys), fails
  on raw-thread spawn in some configurations. Limit with
  `--filter`.
- **TSan**: needs `panic = "abort"` on some targets; the script
  forces `-Z build-std` which compensates.
- **ASan**: doesn't catch races, only memory issues. Pair with TSan.

## `corpus_sweep.sh` (planned)

Reserved name for an opt-in 17 K aozora-corpus sweep that takes
2–5 min. Not yet implemented; see `aozora-parser/tests/corpus_sweep.rs`
for the existing in-band test that runs when `AOZORA_CORPUS_ROOT` is
set.
