# Contributing to afm

Thanks for wanting to help. afm is an active project with a very small
surface area of rules, but those rules are strict — the guarantees below
only stay true if every contribution respects them.

## Ground rules

1. **Docker-only execution** (ADR-0002). Do not invoke `cargo`, `mdbook`,
   or `playwright` on the host. Every automated step must go through
   `just <target>`, which shells into the dev container.
2. **Vendored comrak is hands-off** (ADR-0001). The fork sits at
   `upstream/comrak/` with a 200-line diff budget. If a change requires
   touching upstream, open an issue first — it almost always has a
   cleaner solution via `afm-lexer` (pre-parse) or `afm-parser`
   (post-process AST surgery), per ADR-0008.
3. **No warning suppressions.** `#[allow(...)]`, `#![allow(...)]`,
   `#[cfg_attr(..., allow(...))]`, `continue-on-error` in workflows,
   and similar escape hatches are rejected by `just strict-code`.
   Refactor the real issue instead.
4. **TDD with C1 100% branch coverage as the goal.** A failing test
   lands first, then the fix. See `feedback_tdd_c1_100_coverage.md` in
   CLAUDE.md. The CI floor is currently 96% regions (`_COV_FLOOR` in
   `Justfile`), ratcheted upward as gaps close.

## First-time setup

```sh
docker compose build dev       # ~5 min first time, cached afterward
jj git init --colocate         # if jj isn't already initialised (optional)
just hooks                     # wire lefthook pre-commit / commit-msg / pre-push
just test                      # confirm green
```

## Development loop

```sh
just watch                     # bacon watcher inside the dev container
just lint                      # fmt + clippy pedantic+nursery + typos + strict-code
just test                      # full workspace nextest
just spec-golden-56656         # 罪と罰 Tier-A acceptance gate
just coverage                  # cargo llvm-cov, fails below _COV_FLOOR
just ci                        # replica of the full CI pipeline
```

`just --list` enumerates everything available.

## Commit style

**Conventional Commits** ([v1.0.0](https://www.conventionalcommits.org/)).
The `commit-msg` hook enforces this. Accepted types: `feat`, `fix`,
`docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`,
`revert`. Scopes should be one of: `syntax`, `parser`, `lexer`,
`encoding`, `cli`, `corpus`, `book`, `comrak`, `test`, `dev`, `adr`.

A single commit should be a single logical change. Split unrelated edits.

## Architectural changes

Any decision that shapes how a whole subsystem behaves lands first as an
**Architecture Decision Record** (MADR format) under `docs/adr/`.
Scaffold one with:

```sh
just adr 'my new decision'
```

Reference the ADR in the commit body. Look at `0008-aozora-first-lexer.md`
for an example of a decision that fundamentally reshaped the pipeline.

## Pull requests

- PR title should be `<type>(<scope>): <summary>` matching the commits.
- Link any issue the PR closes (`Closes #N` in the body).
- The PR template (`.github/PULL_REQUEST_TEMPLATE.md`) walks you through
  the checklist — **keep it**. It reminds everyone (including the
  author) of the full gate: tests, coverage, ADR, `just ci`.
- CI runs `just ci` in the ci container image. The gate is the same
  one you ran locally; surprises mean either an environment mismatch
  or an ADR-boundary subtlety.

## Reporting bugs and asking for features

- **Bugs**: use the `bug_report` issue form. Minimal reproducible input
  (the shortest source text that triggers the issue) is the most
  valuable thing you can supply.
- **Features**: use the `feature_request` form. Concrete motivation
  (a real Aozora Bunko text that needs the notation, a CommonMark
  construction that would benefit, a corpus sweep hit) makes triage
  faster.
- **Questions / discussions**: prefer GitHub Discussions over issues.

## Security

Security-sensitive issues (parser crashes, memory safety concerns,
sandbox escapes) should be reported privately per `SECURITY.md` — do
**not** open a public issue.

## How to release

Releases are triggered by a git tag of the form `v<semver>`:

1. Update `CHANGELOG.md` — promote `[Unreleased]` to
   `[<version>] - YYYY-MM-DD` and add a fresh `[Unreleased]` stub.
2. Commit the changelog bump: `git commit -m "chore: release v<version>"`.
3. Tag (annotated): `git tag -a v<version> -m 'v<version>'`.
4. Push: `git push origin main v<version>`.
5. `.github/workflows/release.yml` reacts to the tag, builds release
   binaries on five targets (linux-gnu, linux-musl, macos-aarch64,
   macos-x86_64, windows-msvc), assembles tarballs with the `afm`
   binary, `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, and `README.md`,
   and uploads the archives plus `SHA256SUMS` to the GitHub Release.
6. Sanity check: download one artefact, run `sha256sum --check`, then
   `./afm --version` to confirm the embedded version matches the tag.

A dry-run is available via `workflow_dispatch` from the
[Actions tab](https://github.com/P4suta/afm/actions/workflows/release.yml) —
trigger it from `main` or a release branch before cutting the tag to
confirm the five-target matrix builds cleanly.

**ADR-0002 scope exception**: release builds run on native GitHub
Actions runners with the matching stable rustc, not inside the dev
Docker image. The Docker-only rule applies to development and CI; the
release pipeline is deliberately host-toolchain so each binary target
matches its runner OS exactly. See the leading comment in
`release.yml` for the full rationale.

## License

By contributing, you agree that your contributions are dual-licensed
under Apache-2.0 OR MIT, the same as the project.
