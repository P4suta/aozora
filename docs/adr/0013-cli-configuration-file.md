# 0013. CLI configuration file (`.aozora.toml`)

- Status: accepted
- Date: 2026-06-21
- Deciders: @P4suta
- Tags: cli, devex

## Context

The document subcommands (`check` / `render` / `fmt` / `wire` / `pandoc`)
take the same `--encoding`, `--format`, and `--strict` flags on
every invocation. In a CI pipeline or a repeated editing loop that
repetition is friction: a project that always ships Shift_JIS, or always
wants `--format json` in CI, has to repeat itself on every
call. A project-local default would remove the boilerplate.

The risk a config file introduces is *precedence opacity*: once a setting
can come from a flag, the environment, and a file, it must be obvious
which wins.

## Decision

Add an optional **`.aozora.toml`**.

- **Discovery.** Walk up from the working directory to the filesystem
  root and use the first `.aozora.toml` found (the git / rustfmt idiom —
  project-local config without having to locate the repo root).
  `--config PATH` overrides discovery with an explicit file.
- **Precedence: flag > env > config > default.** clap owns the flag/env
  half — each document flag carries `env = "AOZORA_*"`, so clap resolves
  flag-over-env itself. The CLI then folds in the config value with
  `Option::or` and falls back to the built-in default. One resolver
  function, easy to read and to test.
- **Keys (v1):** `encoding`, `format`, `strict` (kebab-case,
  mirroring the flags). Parsed with `serde` + `toml`, both already
  workspace dependencies — **no new external crate**.
- **`deny_unknown_fields`.** A mistyped key is a hard error, not a silent
  no-op.
- **No global/XDG config in v1.** Configuration is project-scoped only.
- **Config errors exit like other input errors (code 1).** A malformed
  file, an unknown key, or a missing `--config` path surfaces through the
  same `anyhow` path as a decode failure. The documented "2 = usage"
  contract (AGENTS.md) is for clap argv parse errors; file *content*
  errors are library-level, like a bad encoding.

## Consequences

- **Less flag boilerplate.** CI and scripts set defaults once in the
  repo instead of on every call.
- **Zero new dependencies.** `serde` and `toml` were already in the
  lockfile; the CLI just opts in.
- **Precedence is explicit and tested.** `tests/config.rs` pins each
  boundary (flag beats config, env beats default, unknown key fails).
- **Typos fail loudly.** `deny_unknown_fields` turns `stict = true` into
  an error rather than a silently ignored line.
- **The flag set stays the source of truth.** Config keys are a strict
  subset of existing flags; there is no setting reachable only through
  the file, so `--help` remains the complete reference.

## Alternatives considered

**A global/XDG config (`~/.config/aozora/config.toml`).** Rejected for
v1: project-scoped config covers the motivating cases (per-repo encoding,
per-repo CI format) without machine-wide state that surprises, and
without a `dirs`/`directories` dependency. Can be layered in later
*below* the project file in the precedence chain if a need appears.

**clap's `env` feature alone.** Already used for the env tier, but
environment variables are not a persistent, version-controlled,
project-local default — that is exactly what a committed `.aozora.toml`
provides. The two compose; neither replaces the other.

**A non-TOML format (JSON / YAML).** Rejected: `toml` is already a
dependency, is the Rust-ecosystem convention (`Cargo.toml`,
`rustfmt.toml`), and reads well for a handful of flat keys.

## References

- `crates/aozora-cli/src/config.rs` — discovery, loading, the
  `ConfigFile` struct with `deny_unknown_fields`.
- `crates/aozora-cli/tests/config.rs` — precedence and error coverage.
- ADR-0008 — diagnostic rendering; the `format` key selects
  among the views defined there.
- Plan: `cli-devex-twinkling-mountain.md`.
