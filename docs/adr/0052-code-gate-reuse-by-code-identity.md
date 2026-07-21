# 0052. Code-gate reuse across a Release PR and its merge by recomputed code identity

- Status: accepted
- Date: 2026-07-22
- Deciders: @P4suta
- Tags: infra, ci, release, supply-chain

## Context

A release-plz Release PR changes only `[workspace.package].version`, the two
internal `aozora` / `tree-sitter-aozora` dependency pins, `Cargo.lock`'s
workspace-member versions, and `CHANGELOG.md`. The code is byte-identical to
the `main` it was cut from. Yet `release-ready` runs its heavy **code** gates —
the `quality` graph, the 30-shard `mutation` sweep, `fuzz`, `sanitizers`, and
`cross-os` — on both the Release PR (a `release-plz-*` head) and again on the
merge commit's push to `main`. That is the same expensive work, twice, on the
same code.

Neither run can simply be deleted ([ADR-0042](0042-release-ready-is-the-publish-authority.md)):
the Release-PR run is the pre-merge gate the maintainer reviews, and the
merge-commit run produces the commit-stamped artifacts and the exact
`release-ready` proof `release-plz` consumes to publish
([ADR-0051](0051-ci-and-release-ready-split-event-driven-release-fan-in-and-the-actions-concurrency-budget.md),
[ADR-0038](0038-release-plz-owns-versioning-and-crates-io-publishing.md)). What
is redundant is only the **code**-verifying gates — identical for identical
code. The **artifact**-verifying gates (`native-artifacts`, `vscode-artifacts`,
`artifacts`, `python`) are version-stamped and genuinely differ per commit.

## Decision

The merge-commit run reuses the Release-PR run's code-gate result when it can
prove, from immutable data, that the code is the same.

A new `code-proof` job (`needs: qualify`) emits `reuse`:

- **Producer** — a Release PR or a `workflow_dispatch`. It is the authority a
  later merge reuses, so it never reuses: `reuse=false`, gates run in full.
- **Consumer** — a version-bumping push to `main` (the merge commit). It finds
  the *same-repository* `release-plz-*` PR the commit merged, requires a
  completed-success `release-ready` run for that PR head, fetches
  `refs/pull/<n>/head`, and compares a **code-identity hash** of the PR head and
  the merge commit. Equal ⇒ `reuse=true`.

`scripts/code-identity-hash.sh <ref>` is that hash: the tree's blobs verbatim
(`git ls-tree`) with release-plz's version footprint — and only that —
neutralised: the root `Cargo.toml` version line and the two internal pins, every
no-`source` (workspace-local) `[[package]]` version in `Cargo.lock`, and
`CHANGELOG.md` excluded. Every other byte — a third-party dependency bump, a new
line, any real edit — stays in the hash.

The five code gates gain `needs: [qualify, code-proof]` and
`if: … run == 'true' && needs.code-proof.outputs.reuse != 'true'`. The artifact
gates are unchanged. The fan-in requires `code-proof` to have succeeded, then
accepts each code gate as `success` **or** `skipped` when `reuse == true`, while
the artifact gates must always be `success`. `release-plz`'s downstream proof
consumption is unchanged: "the merge commit's `release-ready` succeeded" now
means "artifact gates green ∧ (code gates green ∨ a proven-identical PR run was
green)", which preserves its meaning.

## Consequences

Safety is argued from three properties:

- **Fail-open to a full run.** Any doubt — the PR is not uniquely identified,
  its run is not completed-success, the hashes differ, or a `gh` call fails —
  leaves `reuse=false`, so the gates run in full. `code-proof` exits 0 in every
  such branch.
- **Fail-closed on error.** An unexpected failure fails the `code-proof` job;
  the fan-in requires `code-proof == success`, so a broken proof stops the
  release rather than publishing on an unknown basis.
- **Unforgeable and fork-safe.** The consumer honours only a PR whose head is a
  `release-plz-*` branch *in this repository* (`head.repo.full_name ==
  github.repository`), the same trust boundary as the rest of the release. The
  code identity is **recomputed from git objects** (content-addressed), never
  read from a workflow-produced payload, so a fork rewriting `release-ready.yml`
  cannot forge a hash or a "success". `refs/pull/<n>/head` persists after the
  branch is deleted at merge, so the comparison is always available.

The reused result is the heavy code gates; the commit-stamped artifacts are
still built and verified on every merge, so what `release-plz` publishes is
never reused, only the code proof behind it.

## Alternatives considered

- **GitHub Actions cache keyed on the code hash.** Impossible: cache scope is
  by branch. A `main` push can restore caches from `main` and a PR's base, but
  not from a `release-plz-*` feature branch — the direction we need. The
  Release-PR run's cache is unreadable from the merge run.
- **An in-tree marker committed by the Release PR.** Rejected: anyone with write
  access could forge it, and committing the marker changes the tree it is meant
  to identify (self-referential). It cannot prove a green run existed.
- **Trusting a hash the Release-PR run writes as an artifact.** Rejected in
  favour of recomputation: a payload can be forged by a workflow edit on an
  untrusted head; a git-object hash cannot.

## See also

- [ADR-0042](0042-release-ready-is-the-publish-authority.md)
- [ADR-0051](0051-ci-and-release-ready-split-event-driven-release-fan-in-and-the-actions-concurrency-budget.md)
- [ADR-0038](0038-release-plz-owns-versioning-and-crates-io-publishing.md)
