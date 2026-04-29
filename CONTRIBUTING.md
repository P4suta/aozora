# Contributing to aozora

Thanks for wanting to help. aozora is an active project with a small
surface area of rules, but those rules are strict — the guarantees
below only hold when every contribution respects them.

## Ground rules

1. **Docker-only execution.** Do not invoke `cargo` on the host.
   Every automated step goes through `just <target>`, which shells
   into the dev container so toolchain, sccache, and target caches
   stay reproducible across machines.
2. **`unsafe_code = "forbid"` at the workspace level.** The only
   crates that locally relax it are `aozora-ffi` (C ABI),
   `aozora-scan` (SIMD), and `aozora-xtask` (host tooling using
   `perf_event_open`). Each carries a documented `#[allow(..., reason
   = "...")]` carve-out and `#[deny(unsafe_op_in_unsafe_fn)]` so
   every `unsafe { }` block still has to justify itself.
3. **No silent warning suppressions.** `#[allow(...)]`,
   `#![allow(...)]`, `#[cfg_attr(..., allow(...))]`, and
   `continue-on-error` in workflows are rejected by `just
   strict-code`. Fix the real issue, or change the rule with a
   `reason = "..."` carve-out — never paper over it. (See user
   memory `feedback_clippy_fix_dont_allow.md`.)
4. **TDD with C1 100% branch coverage as the goal.** A failing
   test lands first, then the fix. `just coverage` measures branch
   coverage via `cargo llvm-cov`; CI gates on it.

## First-time setup

```sh
docker compose build dev       # ~5 min first time, cached afterward
just test                      # confirm green
```

## Development loop

```sh
just shell                     # drop into the dev container
just build                     # cargo build --workspace --all-targets
just test                      # workspace nextest
just lint                      # fmt + clippy pedantic+nursery + typos + strict-code
just prop                      # property-based sweep (128 cases per block)
just corpus-sweep              # invariants over AOZORA_CORPUS_ROOT
just coverage                  # cargo llvm-cov branch coverage
just ci                        # replica of the full CI pipeline

# Before a release:
just prop-deep                 # 4096 cases per block — deeper than CI
just fuzz parse_render -- -runs=10000   # cargo-fuzz smoke
```

`just --list` enumerates everything available.

### Corpus-driven tests

Set `AOZORA_CORPUS_ROOT` to a directory of 青空文庫 source files
(UTF-8 or Shift_JIS) before running `just corpus-sweep` or any
sample-profiling target:

```sh
export AOZORA_CORPUS_ROOT=$HOME/aozora-corpus
just corpus-sweep
```

The flagship in-tree fixture lives at
[`spec/aozora/fixtures/56656/`](./spec/aozora/fixtures/56656/) and is
gated as the Tier-A acceptance check. Smaller focused JSON cases sit
under [`spec/aozora/cases/`](./spec/aozora/cases/) and run from
`just test`.

## Test strategy

Each invariant is asserted from multiple angles (see user memory
`feedback_tests_from_many_angles.md`):

1. **Spec cases** under `spec/aozora/cases/*.json` — each entry pins
   `(input, html, canonical_serialise)` for round-trip + render
   equality.
2. **Property tests** under `crates/*/tests/property_*.rs` — generators
   in [`crates/aozora-test-utils`](./crates/aozora-test-utils) drive
   parse / render / round-trip invariants.
3. **Corpus sweep** via `just corpus-sweep` — every document in
   `AOZORA_CORPUS_ROOT` must (a) parse without panicking and
   (b) round-trip through `parse ∘ serialize`. The loader lives in
   `aozora-corpus`.
4. **Fuzz harness** at `crates/*/fuzz/fuzz_targets/*.rs` for
   parse + render + Shift_JIS decode paths.
5. **Sanitizers** — `just` recipes for Miri (UB), TSan (data races
   in the parallel corpus loader), ASan (heap correctness).

When you add a new invariant, land all five touchpoints in the same
PR or split them into a chain referencing the predicate.

## Lint gates

`just lint` runs four checks:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  (pedantic + nursery + cargo, gated by `[workspace.lints.clippy]` in
  the root `Cargo.toml`)
- `typos --config typos.toml`
- `just strict-code` — repo-local gates that grep for forbidden
  patterns: `println!`/`dbg!` outside of `build.rs`, bare `TODO`s
  without an issue reference, `unsafe` outside the carved-out
  crates, etc.

`just deny`, `just audit`, `just udeps`, and `just semver` cover
dependency licensing, advisories, unused deps, and SemVer breakage
respectively. CI runs all of these in `just ci`.

## Coding standards

- **Borrowed-arena AST.** Parsers return `AozoraTree<'arena>` borrowed
  from the `Document`'s [`bumpalo`](https://docs.rs/bumpalo) arena.
  No owned-AST mirror exists; downstream consumers either walk the
  borrow or call `tree.serialize() / tree.to_html()`.
- **`aozora-spec` is the single source of truth.** `Span`,
  `TriggerKind`, `PairKind`, `Diagnostic`, and PUA sentinel
  codepoints live there. New shared types belong in `aozora-spec`,
  never in `aozora-lexer` or `aozora-syntax`.
- **Pure functions over mutation.** `lex_into_arena(&str, &Bump) ->
  BorrowedLexOutput<'_>` and `html::render_to_string(&str) -> String`
  are pure. Avoid hidden global state, thread-locals, or
  `OnceCell`-backed caches.
- **No comments that just restate the code.** A short `//` line is
  fine when it captures a non-obvious invariant or the *why* behind
  an unusual choice. Multi-paragraph docstrings are reserved for
  public API surfaces.

See [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) for the layered
crate boundaries this implies.

## Adding a new 青空文庫 notation

The end-to-end TDD flow is roughly:

1. **Spec fixture** — add a `(input, html, serialise)` triple under
   [`spec/aozora/cases/`](./spec/aozora/cases/).
2. **AST variant** — add a borrowed-arena variant to `AozoraNode` in
   `crates/aozora-syntax/src/borrowed.rs`.
3. **Lexer test (red)** — add a case to the relevant phase test
   under `crates/aozora-lexer/tests/`.
4. **Lexer impl (green)** — wire the recogniser into the appropriate
   phase (sanitize → tokenize → pair → classify).
5. **Renderer** — emit the new HTML shape in
   `crates/aozora-render/src/html.rs` and the canonical
   serialisation in `crates/aozora-render/src/serialize.rs`.
6. **Cross-layer invariants** — extend the property test or corpus
   predicate that the new shape interacts with (escape-safety,
   round-trip, span well-formedness).

## Commit style

**Conventional Commits** ([v1.0.0](https://www.conventionalcommits.org/)).
The `commit-msg` hook enforces this. Accepted types: `feat`, `fix`,
`docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`,
`revert`. Scopes typically map to a crate name without the
`aozora-` prefix (e.g. `feat(render): ...`, `fix(lexer): ...`,
`perf(scan): ...`).

A single commit should be a single logical change. Split unrelated
edits.

## Pull requests

- PR title should be `<type>(<scope>): <summary>` matching the commits.
- Link any issue the PR closes (`Closes #N` in the body).
- The [PR template](./.github/PULL_REQUEST_TEMPLATE.md) walks you
  through the checklist — keep it. It reminds everyone (including
  the author) of the full gate.
- CI runs `just ci` in the dev image. The gate is the same one you
  ran locally; surprises mean either an environment mismatch or a
  layer-boundary subtlety.

## Reporting bugs and asking for features

- **Bugs**: use the `bug_report` issue form. Minimal reproducible input
  (the shortest source text that triggers the issue) is the most
  valuable thing you can supply.
- **Features**: use the `feature_request` form. Concrete motivation —
  a real Aozora Bunko text that needs the notation, a corpus sweep
  hit, a downstream consumer's blocker — makes triage faster.
- **Questions / discussions**: prefer GitHub Discussions over issues.

## Security

Security-sensitive issues (parser crashes on untrusted input,
HTML-injection bypass, memory-safety concerns in the FFI driver)
should be reported privately — see [`SECURITY.md`](./SECURITY.md).
Do **not** open a public issue.

## Releases

Releases are triggered by a git tag of the form `v<semver>`:

1. Tag (annotated): `git tag -a v<version> -m 'v<version>'`.
2. Push: `git push origin main v<version>`.
3. `.github/workflows/release.yml` reacts to the tag, builds release
   binaries on three targets (linux x86_64, macos arm64, windows
   x86_64), assembles tarballs / zips with the `aozora` binary,
   `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, and `README.md`, and
   uploads the archives plus `SHA256SUMS` to the GitHub Release.
4. Sanity check: download one artefact, run `sha256sum --check`, then
   `./aozora --version` to confirm the embedded version matches the
   tag.

Release builds run on native GitHub Actions runners with the matching
stable rustc, not inside the dev Docker image — each binary target
matches its runner OS exactly. The Docker-only rule applies to
development and CI only.

## License

By contributing, you agree that your contributions are dual-licensed
under Apache-2.0 OR MIT, the same as the project.
