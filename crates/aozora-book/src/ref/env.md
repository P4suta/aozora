# Environment variables

A central reference for every env var aozora reads. Variables fall
into three groups: parser configuration, dev / bench harness, and
container plumbing.

## Parser configuration

| Variable | Read by | Effect |
|---|---|---|
| `NO_COLOR` | `aozora-cli` | If set (any value), disable ANSI colour output. Same as `--no-color`. Standard convention from <https://no-color.org>. |
| `AOZORA_LOG` | `aozora-cli`, library opt-in | `tracing-subscriber` filter directive (e.g. `aozora_lex=debug,aozora_render=info`). For internal debugging; not part of the stable surface. |

## Dev / bench harness

| Variable | Read by | Effect |
|---|---|---|
| `AOZORA_CORPUS_ROOT` | `aozora-corpus`, every probe, every sample-profile recipe, the corpus sweep | Directory of 青空文庫 source files (UTF-8 or Shift_JIS). Required for any corpus-driven operation. |
| `AOZORA_PROFILE_LIMIT` | `aozora-bench` probes | Cap the number of corpus documents per probe. Useful for fast iteration; set to `100` for a sub-second sweep. |
| `AOZORA_PROFILE_REPEAT` | `samply-corpus`, `samply-render` | Number of parse / render passes per document after the one-time corpus load. Default `5`; raise to give samply enough parser-bound wall time to attach to. |
| `AOZORA_PROBE_DOC` | `pathological_probe` | Single corpus path to probe in tight per-call mode. Path is relative to `$AOZORA_CORPUS_ROOT`. |
| `AOZORA_PROPTEST_CASES` | `aozora-test-utils::config` | Override default proptest case count (default `128` per block). `4096` for `just prop-deep`. |

## Container plumbing

These are set by `docker-compose.yml` and don't need manual handling
unless you're invoking cargo directly outside the dev container.

| Variable | Set by | Purpose |
|---|---|---|
| `CARGO_HOME` | compose | `/workspace/.cargo` — registry + git deps cached on a named volume. |
| `CARGO_TARGET_DIR` | compose | `/workspace/target` — build output cached on a named volume. |
| `RUSTC_WRAPPER` | compose | `sccache` — compile cache. |
| `SCCACHE_DIR` | compose | `/workspace/.sccache` — sccache backing store on a named volume. |
| `SCCACHE_CACHE_SIZE` | compose | `10G` — default cap. |
| `CARGO_INCREMENTAL` | compose | `0` — incremental compile defeats sccache; turning it off lets sccache cache the very crates we build most often. |
| `RUST_BACKTRACE` | compose | `1` — full backtraces on panic. |
| `GIT_CONFIG_*` | compose | Whitelists `/workspace` for git's "dubious ownership" check (the bind-mounted host source is a non-root UID; the container runs as root). |

## Variables we deliberately do *not* read

A few standard variables aozora intentionally ignores:

| Variable | Why ignored |
|---|---|
| `LANG` / `LC_ALL` | aozora handles its own encoding via `--encoding`. Locale-driven byte interpretation would make the parser non-reproducible across machines. |
| `RUSTFLAGS` (in non-build context) | The release / bench / PGO profiles set their own flags; per-invocation `RUSTFLAGS` would defeat sccache hits for unrelated crates. |
| `CARGO_BUILD_JOBS` | Cargo's default (CPU count) is what we want. Overriding usually fights the bench harness's own parallelism control. |

## See also

- [CLI reference → Environment](cli.md#environment) — the CLI's
  per-invocation env.
- [Performance → Corpus sweeps](../perf/corpus.md) — the
  `AOZORA_CORPUS_ROOT` setup.
- [Performance → Profiling with samply](../perf/samply.md) — the
  `AOZORA_PROFILE_REPEAT` knob.
