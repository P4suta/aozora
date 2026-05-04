# Development loop

aozora's development workflow is built around three rules:

1. **Docker-only execution.** The host toolchain is never invoked.
2. **`just` is the entry point.** Every operation goes through a
   `just` recipe that wraps the underlying tool inside the dev
   container.
3. **Lint gates run automatically.** lefthook installs git hooks
   that run `fmt + clippy + typos` pre-commit and `test + deny`
   pre-push, so a passing local commit roughly mirrors a passing
   CI run.

## First-time setup

```sh
git clone git@github.com:P4suta/aozora.git
cd aozora
docker compose build dev        # ~5 min the first time, cached afterwards
just hooks                      # install lefthook git hooks
just test                       # confirm green
```

## Daily loop

```sh
just shell                      # drop into the dev container
just build                      # cargo build --workspace --all-targets
just test                       # workspace nextest
just lint                       # fmt + clippy + typos + strict-code
just prop                       # property-based sweep (128 cases / block)
just ci                         # full CI replica (lint + build + test + prop + deny + audit + udeps + coverage + book-build)
```

`just --list` enumerates everything available; `just --list --unsorted`
preserves the topical grouping (build → test → lint → deps → bench →
docs → release → dev-helpers).

## Watch mode (bacon)

```sh
just watch                      # default `check` job
just watch clippy
just watch test
```

Inside bacon: `t` test, `c` clippy, `d` doc, `f` failing-only,
`esc` previous job, `q` quit, `Ctrl-J` list jobs. The watcher runs
inside the dev container so file change detection works against the
bind-mounted source.

For headless usage (no TTY, e.g. piping to `tee`):

```sh
just watch-headless check       # plain output, no TUI
```

## Why Docker for everything?

Three reasons.

1. **Toolchain reproducibility.** The dev image pins
   `rust:1.95.0-bookworm` plus exact versions of `cargo-nextest`,
   `cargo-llvm-cov`, `cargo-deny`, `cargo-audit`, `cargo-udeps`,
   `cargo-semver-checks`, `cargo-fuzz`, `mdbook`, `mdbook-mermaid`,
   `lychee`, `git-cliff`, `bacon`, and `lefthook`. A fresh checkout
   on any machine produces *identical* tool behaviour.
2. **sccache hits.** The compose file mounts a named volume at
   `/workspace/.sccache` and sets `RUSTC_WRAPPER=sccache`. Across
   sessions and across branches, the cache stays warm.
3. **Host insulation.** Nothing in the workspace touches `~/.cargo`,
   `~/.rustup`, or any global state. Removing the project means
   `docker compose down -v && rm -rf aozora/`.

The two exceptions to Docker-only:

- **samply profiling.** `perf_event_open(2)` doesn't survive the
  container seccomp profile; the `samply-*` recipes invoke the host
  toolchain (see [Profiling with samply](../perf/samply.md#why-these-run-on-the-host-not-docker)).
- **Release builds.** GitHub Actions runners build the release
  binaries natively per OS (the cross-target binary needs to match
  its runner OS exactly).

## Editor / IDE setup

The repository includes a `.devcontainer/` config, so:

- **VS Code with Dev Containers extension** — "Reopen in Container"
  picks up the dev image, the rust-analyzer toolchain, and the
  `aozora-*` workspace at once. No host-side rust install needed.
- **Anything else** — point your editor's rust-analyzer at the dev
  container via `docker exec`. The cleanest approach is symlinking
  `target/` from the named volume to a host-visible path; the
  alternative is the editor's own remote-LSP support.

## sccache stats

After a build cycle, check that the cache is actually warm:

```sh
just sccache-stats
```

Healthy steady state: 80%+ hit rate during normal iteration. A
sub-50% hit rate usually means `RUSTC_WRAPPER` got defeated — the
likely culprit is a stray env override or an `[env]` in
`.cargo/config.toml`. To reset counters before a measurement window:

```sh
just sccache-zero && just clean && just build && just sccache-stats
```

## Pre-commit hooks (lefthook)

`lefthook.yml` configures:

- **pre-commit** (parallel): `fmt`, `clippy`, `typos`.
- **commit-msg**: Conventional Commits regex.
- **pre-push** (parallel): `test`, `deny`.

The hooks shell into `docker compose run --rm dev …` so they're
identical to the `just` recipes you ran manually. To skip a hook
temporarily, push from the dev container's shell directly (the
hooks attach to the host git, not the container's git).

## Why lefthook over husky / pre-commit / cargo-husky?

- **husky** — Node-only ecosystem; would force a Node dep into a
  Rust workspace.
- **pre-commit** (Python framework) — Python-only ecosystem; same
  issue inverted.
- **cargo-husky** — abandoned upstream.
- **lefthook** — single Go binary, language-neutral, parallel
  execution, ships from a small upstream that's actively maintained.
  Mainstream choice for polyglot Rust workspaces in 2026.

## Conventional commits

The `commit-msg` hook enforces:

```text
<type>(<scope>): <subject>
```

Where `<type>` ∈ `feat | fix | docs | style | refactor | perf | test | build | ci | chore | revert`,
and `<scope>` is typically a crate name without the `aozora-` prefix
(e.g. `feat(render): add aozora-tcy class hook`).

git-cliff turns these into the [CHANGELOG](release.md#changelog-generation)
on release.

## Adding a new 青空文庫 notation

End-to-end TDD flow:

1. **Spec fixture.** Add a `(input, html, serialise)` triple under
   `spec/aozora/cases/`.
2. **AST variant.** Add a borrowed-arena variant to `AozoraNode` in
   `crates/aozora-syntax/src/borrowed.rs`.
3. **Lexer test (red).** Add a case to the relevant phase test
   under `crates/aozora-pipeline/tests/`.
4. **Lexer impl (green).** Wire the recogniser into the appropriate
   phase (sanitize → events → pair → classify).
5. **Renderer.** Emit the new HTML shape in
   `crates/aozora-render/src/html.rs` and the canonical
   serialisation in `crates/aozora-render/src/serialize.rs`.
6. **Cross-layer invariants.** Extend the property test or corpus
   predicate that the new shape interacts with (escape-safety,
   round-trip, span well-formedness).

## See also

- [Testing strategy](testing.md) — what each test layer asserts.
- [Release process](release.md) — how a tag becomes a published
  release.
