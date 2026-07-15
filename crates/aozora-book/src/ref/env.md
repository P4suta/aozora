# Environment variables

A central reference for every env var aozora reads. Variables fall
into three groups: parser configuration, dev / bench harness, and
container plumbing.

## Parser configuration

| Variable | Read by | Effect |
|---|---|---|
| `NO_COLOR` | `aozora-cli` | If set (any value), disable ANSI colour output — but only where the colour policy is `auto`, which is what it means to leave the decision to the terminal. A decided `always` (from `--color` or `.aozora.toml`) still colours. |
| `CLICOLOR` / `CLICOLOR_FORCE` | `aozora-cli` | The other two terminal colour signals, consulted on the `auto` path only: `CLICOLOR_FORCE` (non-`0`) forces colour on, `CLICOLOR=0` forces it off. Like `NO_COLOR`, neither overrides a decided `always` / `never`. |
| `FORCE_COLOR` | `aozora-cli` (via `miette`) | Read by miette's `supports-color` backend and, on the `auto` path, **outranks all three** vars above (`FORCE_COLOR=0` disables; any other value forces colour on). Not part of aozora's own surface — documented because it is easy to mistake its effect for a bug. |
| `AOZORA_ENCODING` | `aozora-cli` | Source-encoding fallback for `-E/--encoding`: `auto` (default), `utf8`, or `sjis`. Lower precedence than the flag, higher than `.aozora.toml`. |
| `AOZORA_STRICT` | `aozora-cli` | Fallback for `check` / `lint` `--strict`: any diagnostic exits non-zero. |
| `AOZORA_FORMAT` | `aozora-cli` | Fallback for `check` / `lint` `--format`: `auto` / `human` / `json` / `short`. |
| `AOZORA_LANG` | `aozora-cli` | Language for **human messages** (the stdin guard, `--watch` banner, `explain` chrome): `en` (default) / `ja` / `zh`, or any BCP-47 tag. Precedence `--lang > AOZORA_LANG > .aozora.toml lang > LANG > en`; unknown locales fall back to `en`. Never affects machine output (json / short / codes / exit / schema) or encoding. See [ADR-0033](https://github.com/P4suta/aozora/blob/main/docs/adr/0033-cli-output-language-policy.md). |
| `LANG` | `aozora-cli` | **Lowest-priority** fallback for the human-message language only (a POSIX `ja_JP.UTF-8` value negotiates to `ja`). Outranked by `--lang` / `AOZORA_LANG` / `.aozora.toml lang`. **Not** read for source-byte encoding — see below. |
| `AOZORA_LOG` | `aozora-cli`, library opt-in | `tracing-subscriber` filter directive (e.g. `aozora_pipeline=debug,aozora_render=info`). For internal debugging; not part of the stable surface. |

## Dev / bench harness

| Variable | Read by | Effect |
|---|---|---|
| `AOZORA_CORPUS_ROOT` | `aozora-corpus`, every probe, every sample-profile recipe, the corpus sweep | Directory of 青空文庫 source files (UTF-8 or Shift_JIS). Required for any corpus-driven operation. |
| `AOZORA_PROFILE_LIMIT` | `aozora-bench` probes | Cap the number of corpus documents per probe. Useful for fast iteration; set to `100` for a sub-second sweep. |
| `AOZORA_PROFILE_REPEAT` | `samply-corpus`, `samply-render` | Number of parse / render passes per document after the one-time corpus load. Default `5`; raise to give samply enough parser-bound wall time to attach to. |
| `AOZORA_PROBE_DOC` | `pathological_probe` | Single corpus path to probe in tight per-call mode. Path is relative to `$AOZORA_CORPUS_ROOT`. |
| `AOZORA_PROPTEST_CASES` | `aozora-proptest::config` | Override default proptest case count (default `128` per block). `4096` for `just prop-deep`. |

## Container plumbing

These are set by `docker-compose.yml` and don't need manual handling
unless you're invoking cargo directly outside the dev container.

| Variable | Set by | Purpose |
|---|---|---|
| `CARGO_HOME` | compose | `/cargo/home` — registry + git deps cached on a named volume. |
| `CARGO_TARGET_DIR` | compose | `/cargo/target` — build output cached on a named volume. |
| `RUSTC_WRAPPER` | compose | `sccache` — compile cache. |
| `SCCACHE_DIR` | compose | `/cargo/sccache` — sccache backing store on a named volume. |
| `SCCACHE_CACHE_SIZE` | compose | `10G` — default cap. |
| `CARGO_INCREMENTAL` | compose | `0` — incremental compile defeats sccache; turning it off lets sccache cache the very crates we build most often. |
| `RUST_BACKTRACE` | compose | `1` — full backtraces on panic. |
| `GIT_CONFIG_*` | compose | Whitelists `/workspace` for git's "dubious ownership" check (the bind-mounted host source is a non-root UID; the container runs as root). |

## Variables we deliberately do *not* read

A few standard variables aozora intentionally ignores:

| Variable | Why ignored |
|---|---|
| `LANG` / `LC_ALL` **for encoding** | Source-byte interpretation is governed only by `--encoding` / `AOZORA_ENCODING` / `.aozora.toml encoding` / auto-detection. Locale-driven byte interpretation would make the parser non-reproducible across machines. (`LANG` *is* read as the lowest-priority **message-language** fallback — see the parser-configuration table above; `LC_ALL` is not read at all.) |
| `RUSTFLAGS` (in non-build context) | The release / bench / PGO profiles set their own flags; per-invocation `RUSTFLAGS` would defeat sccache hits for unrelated crates. |
| `CARGO_BUILD_JOBS` | Cargo's default (CPU count) is what we want. Overriding usually fights the bench harness's own parallelism control. |

## See also

- [CLI reference → Environment](cli.md#environment) — the CLI's
  per-invocation env.
- [Performance → Corpus sweeps](../perf/corpus.md) — the
  `AOZORA_CORPUS_ROOT` setup.
- [Performance → Profiling with samply](../perf/samply.md) — the
  `AOZORA_PROFILE_REPEAT` knob.
