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

## Dependency follow-up timer (xtask, not a shell script)

Install / inspect / remove a systemd **user** timer that runs `just
deps-check` weekly. Replaces the dependabot / renovate / GitHub
Actions pattern with an entirely-local mechanism — there is no
remote CI involved at any point.

The implementation lives in `crates/aozora-xtask/src/deps.rs` and
is invoked through `just`:

```sh
just deps-timer-install     # weekly schedule activated
just deps-timer-status      # next run + recent journal entries
just deps-timer-uninstall   # schedule removed; log preserved
```

(Equivalent low-level invocation: `cargo run --release -p
aozora-xtask -- deps {install-timer|status|uninstall-timer}`.)

Cadence: Sunday 03:30 local, with a 30-minute random delay so
multiple developer machines on the same LAN don't hit `crates.io`
in lock-step. `Persistent=true` so a missed run (laptop asleep)
fires on next boot.

The unit bakes in `WorkingDirectory=$REPO_ROOT`, so the timer is
**bound to the checkout that ran the install**. Cloning the repo
elsewhere and re-running `just deps-timer-install` there is fine —
the unit name stays `aozora-deps-check.{service,timer}` and the new
install overwrites the old.

Output rolls into `$XDG_STATE_HOME/aozora/deps-check.log`
(or `~/.local/state/aozora/deps-check.log`). The lefthook
`post-merge` hook surfaces freshness via `just deps-status` so the
developer sees the report whenever they pull.

## `corpus_sweep.sh` (planned)

Reserved name for an opt-in 17 K aozora-corpus sweep that takes
2–5 min. Not yet implemented; `just corpus-sweep` (which loads the
corpus through `aozora-corpus` and walks every document) covers the
same invariants today.

## Profiling (samply)

Profiling lives in the `aozora-xtask` workspace crate and is invoked
through `just`, not as a shell script:

```sh
# Sample-profile a single corpus document.
AOZORA_CORPUS_ROOT=/path/to/corpus \
  just samply-doc 001529/files/50685_ruby_67979/50685_ruby_67979.txt
# → /tmp/aozora-doc-50685_ruby_67979.json.gz

# Sample-profile the corpus parser hot path. REPEAT defaults to 5
# parse passes after the one-time load.
AOZORA_CORPUS_ROOT=/path/to/corpus just samply-corpus
AOZORA_CORPUS_ROOT=/path/to/corpus just samply-corpus 10

# Open the resulting JSON in the Firefox-Profiler UI.
samply load /tmp/aozora-doc-50685_ruby_67979.json.gz
```

Both targets:
1. Verify `/proc/sys/kernel/perf_event_paranoid <= 1` and tell you
   the fix-up command if not.
2. Rebuild the bench example with `--profile=bench` so debug info is
   preserved (samply needs symbols; `cargo run --release` strips
   them, which is the original foot-gun).
3. Spawn `samply record --save-only --no-open` at 4 kHz.

Why on the host (not Docker): `samply` opens `perf_event_open(2)`
directly against the kernel; Docker's default seccomp profile
blocks it. The xtask binary therefore runs without the `{{_dev}}`
prefix that other Justfile targets use.
