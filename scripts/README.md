# Scripts

On-demand tooling. PR-required gates live in the `cargo test` /
`cargo clippy` invocations checked into CI; these scripts cover the
heavier checks that belong on a nightly cron or local exploration.

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
