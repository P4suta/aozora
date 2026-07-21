# 0051. CI and release-ready split, event-driven release fan-in, and the Actions concurrency budget

- Status: accepted
- Date: 2026-07-21
- Deciders: @P4suta
- Tags: infra, ci, release, supply-chain

## Context

A release commit ran two large Actions graphs at once. `ci` fanned its
Rust matrix across ~25 jobs on every push, and a `version = ` bump also
fired `release-ready` — a strict superset of ci's Rust gates plus deep
fuzz, complete mutation sweeps, native macOS/Windows tests, external
spec/corpus checks, and installation from every distribution artifact
([ADR-0042](0042-release-ready-is-the-publish-authority.md)). The two
overlapped almost entirely, so a Release-PR merge spent hundreds of
runner-minutes re-proving in `ci` what `release-ready` was proving
again, and the wide matrices contended for the org's concurrent-runner
pool.

The release-commit detection that decides all of this was copy-pasted in
three workflows (`release-ready.yml`, `release-plz.yml`, `docs.yml`):
`git diff HEAD^ -- Cargo.toml | grep -Eq '^[+-]version = '`. Two of the
downstream workflows then *busy-polled* for `release-ready` to finish —
`release-plz.yml` looped 420×60 s and `docs.yml` looped 330×60 s — each
loop holding a runner idle for up to seven and five and a half hours
respectively. No `workflow_run` trigger existed to turn a finished
`release-ready` into the downstream work directly.

crates.io Trusted Publishing constrains how the publish may be reached:
it refuses an OIDC token minted under a `workflow_run` or
`pull_request_target` trigger
([ADR-0038](0038-release-plz-owns-versioning-and-crates-io-publishing.md),
Consequences). The publish must run under `push: [main]` or
`workflow_dispatch`. Any event-driven fan-in has to respect that.

## Decision

**On a release commit, `ci` defers its Rust graph to `release-ready`,
and `release-ready` becomes a required check.** A single composite
action, `.github/actions/detect-release-commit`, is the one source of
the release-commit primitives (`version-changed`, `release-branch`);
every caller derives its own policy from it.

- `ci`'s `changes` job now emits a `release` output — true for a
  `version = ` bump on push or a `release-plz-*` PR head. Every Rust,
  playground, VS Code, and docs-drift job gains
  `&& needs.changes.outputs.release != 'true'`, so on a release commit
  they do not run: `release-ready` re-proves them and more. The
  aggregate `ci-success` still reports (skipped deps pass), and the
  three gates `release-ready` cannot run stay unconditional in `ci`
  because each needs an environment `release-ready` does not provision:
  `msrv` (the MSRV toolchain), `wasm-test` (the `wasm32-wasip1`
  target), and `pandoc-smoke` (the `pandoc` binary). The cheap gates
  that `release-ready` did not already run were folded into its
  `quality`/`artifacts` recipes — `just coverage`, `test-doc`, `shear`,
  `readme-gate`, `playground-ci` — so the deferral loses no signal.

- `release-ready` is added to the `main` branch ruleset's required
  status checks, alongside `ci-success` and the CodeQL trio. It was
  already the branch-protection gate for Release PRs by name
  ([ADR-0042](0042-release-ready-is-the-publish-authority.md)); it is
  now enforced, not merely documented.

**A new `release-fan-in.yml` replaces both busy-poll loops with one
`workflow_run` → `workflow_dispatch` hop.** It triggers on
`release-ready` completion; runs only for a successful run whose event
was `push` on `main`; re-checks via `detect-release-commit` that HEAD
actually bumps the version; and then `gh workflow run`s
`release-plz.yml` and `docs.yml` with `commit=<qualified sha>`.
`release-plz` and `docs` each re-verify `release-ready` is
completed-success for that commit once and fail fast instead of polling.

**The fan-in stays Trusted-Publishing-compliant by dispatching, never
publishing.** `release-fan-in` holds only `contents: read` +
`actions: write`; it carries no `id-token` and no registry secret. It
merely *dispatches* `release-plz.yml`, which then runs under
`workflow_dispatch` — a trigger crates.io accepts. The OIDC exchange
therefore never happens under the forbidden `workflow_run` trigger. This
is the load-bearing reason the pattern is a dispatch and not a direct
publish from the `workflow_run` handler.

**Wide matrices are capped to a concurrency budget** so a release commit
cannot monopolise the runner pool: `release-ready`'s mutation sweep
`max-parallel: 10`, its VSIX build `max-parallel: 4`, and `ci`'s
`mutants-in-diff` shards `max-parallel: 4`.

## Consequences

- A Release-PR merge runs one large graph, not two overlapping ones. The
  publish path is event-driven: no runner sits in a multi-hour `sleep`
  loop, and the hand-off is a trigger rather than a poll.
- `release-ready` is now a hard merge gate on `main`, not just on Release
  PRs. A commit that fails it cannot land, matching the fact that it is
  the publish authority.
- The manual recovery path is unchanged. `release-plz.yml` keeps its
  `workflow_dispatch` `commit` input, so an interrupted publish is still
  rearmed by hand with `gh workflow run release-plz.yml -f commit=…`; the
  fan-in is an automatic caller of the same entrypoint, not a
  replacement for it. The publication freeze
  (`.github/RELEASE_FROZEN.md` + the `release-freeze` job) still gates
  both publish jobs.
- One composite action is now on the critical path of four workflows.
  Its `git diff HEAD^` needs a checkout with `fetch-depth >= 2`; each
  caller supplies that.
- The concurrency caps trade release-commit latency for a bounded runner
  footprint. A release commit's mutation and VSIX matrices drain in
  waves rather than all at once.

## Alternatives considered

**Keep both graphs, just narrow ci with paths-filter.** A release commit
touches `Cargo.toml` and `CHANGELOG.md`, which the filter already maps to
the full Rust matrix, so path filtering alone would not have stopped the
duplication. The overlap is with `release-ready`, not with the changed
paths, so the guard has to key on "is this a release commit?".

**Fold the downstream publish/docs work into `release-ready` itself.**
Rejected on the same grounds as ADR-0038's downstream-trigger analysis:
putting the crates.io publish behind `release-ready` would run it under
that workflow's trigger and would couple the expensive proof to the
credential-bearing publish. Keeping them separate and dispatching
preserves the trigger crates.io requires.

**Trigger `release-plz` directly from `workflow_run`.** This is exactly
what crates.io forbids — an OIDC token minted under `workflow_run` is
rejected. The fan-in works around it precisely by *dispatching* rather
than being the publisher, so the publish runs under `workflow_dispatch`.

**Leave the busy-poll loops.** They worked but pinned a runner idle for
hours and re-derived state a completion event already carries. A
`workflow_run` handler is the event the polls were emulating.

## References

- [ADR-0038](0038-release-plz-owns-versioning-and-crates-io-publishing.md)
  — crates.io Trusted Publishing blocks `workflow_run` /
  `pull_request_target`; the publish must run under `push`/`dispatch`.
- [ADR-0042](0042-release-ready-is-the-publish-authority.md) —
  `release-ready` is the sole automated publish authority and the
  Release-PR branch-protection status.
- [ADR-0007](0007-parallel-pre-push-pipeline.md) — the local
  `ci-parallel` pre-push mirror, unaffected by the cloud-side split.
- [ADR-0039](0039-release-plzs-manual-trigger-stays-unguarded.md) — the
  manual `workflow_dispatch` recovery path this fan-in automates but does
  not remove.
- `.github/actions/detect-release-commit`, `.github/workflows/release-fan-in.yml`,
  `.github/rulesets/main-branch.json`.
- [Release process](../contrib/release.md).
</content>
</invoke>
