# 0038. release-plz owns versioning and crates.io publishing

- Status: accepted; the `workflow_dispatch` consequence superseded by
  [ADR-0039](0039-release-plzs-manual-trigger-stays-unguarded.md)
- Date: 2026-07-15
- Deciders: @P4suta
- Tags: release, security, ci, supply-chain

## Context

[ADR-0020](0020-release-secret-hardening-trusted-publishing.md) decided
how each registry authenticates. For crates.io it chose
`rust-lang/crates-io-auth-action`, driven by a hand-maintained publish
ladder in `publish-crates.yml`.

That workflow no longer exists. #306 adopted
[release-plz](https://release-plz.dev/) and deleted the ladder, but
recorded the change nowhere: ADR-0020 is still `accepted` and still
describes the action we do not use, and no ADR mentions release-plz at
all. Meanwhile `docs/contrib/releasing-secrets.md` was rewritten in #550
to match the code, so an accepted ADR and the runbook now contradict
each other. Accepted ADRs are never edited, so only a superseding one
closes that.

Two things forced the move away from a hand-written ladder:

- **The workspace is multi-crate and lockstep.** Every public crate
  bumps and publishes together, in dependency order, from one
  `[workspace.package].version`. The ladder encoded that order by hand.
- **A Release PR is a better gate than a local tag.** The bump and the
  CHANGELOG arrive as something reviewable, and the deliberate human act
  becomes a labelled squash-merge rather than a `git push --tags` from
  somebody's laptop.

## Decision

**release-plz solely owns the workspace version, `CHANGELOG.md`, the
crates.io publish, and the `vX.Y.Z` tag.** Humans never hand-edit a
version or hand-push a release tag.

This supersedes **only the crates.io line of ADR-0020's decision 2**.
Everything else in ADR-0020 stands: the ungated-build / gated-publish
split, the `release` environment's reviewer and ref restrictions, PyPI
and npm via their own OIDC, the OIDC-less VS Code tokens as environment
secrets, and the honest bootstrap.

1. **crates.io authenticates through release-plz's own OIDC exchange.**
   Not `crates-io-auth-action`, and no `CARGO_REGISTRY_TOKEN` — release-plz
   performs the token exchange internally, and its documentation is
   explicit that supplying either breaks it. The job carries
   `id-token: write` and nothing else for this purpose.

2. **A second environment, `release-plz`, with no required reviewer and
   a `main`-only branch policy.** ADR-0020's `release` environment
   requires an approval before a runner ever sees the credential. That is
   right for the tag-driven publishers, and wrong here: the human gate is
   the labelled squash-merge that already happened, and a second approval
   would only stall an unattended publish that the merge authorised. The
   `main`-only policy keeps the credential unreadable from a workflow on
   any other branch.

3. **A GitHub App pushes the tag, not `GITHUB_TOKEN`.** GitHub does not
   start a workflow from a ref pushed with `GITHUB_TOKEN`. `release.yml`
   triggers on `push: tags: ["v*"]`, as do the PyPI / npm / Extism
   publishers, so a `GITHUB_TOKEN` tag would publish nothing downstream.

## Consequences

**We traded a publish token for an App private key.** ADR-0020's goal
was no long-lived publish secret, and crates.io now meets it exactly —
there is no token to leak. But `RELEASE_PLZ_APP_PRIVATE_KEY` is a new
long-lived credential, and it is more powerful than what it replaced:
Contents R/W and Pull requests R/W, not "publish one crate".

This is not a trade we chose so much as one the platform imposed. Given
the tag-fires-downstream design, no other actor can push that tag. The
mitigation is scope, not elimination: the key lives on an environment
restricted to `main`, so no branch can read it, and it is the reason
that environment exists separately at all.

**The first publish of a brand-new crate still needs a token.** Trusted
publishing cannot create a crate;
[RFC 3691](https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html)
lists that only as a future possibility, and PyPI's pending publisher has
no crates.io equivalent. So ADR-0020's decision 4 continues to apply,
with the bootstrap token wired to `release-plz-release` and revoked after.
Enable crates.io's Trusted Publishing only mode per crate once an OIDC
release has actually gone green — never before, since a failed OIDC
publish would otherwise leave no way back in.

**A publisher is bound to an exact workflow filename plus environment**,
because that is what the OIDC `sub` claim matches. Every publisher
registered against the retired `publish-crates.yml` / `release` must be
deleted and re-added against `release-plz.yml` / `release-plz`; crates.io
has no edit UI. Renaming either the workflow or the environment later
breaks publishing for every crate at once.

**`release-plz.yml`'s manual trigger is live once activated.** Its
`workflow_dispatch` takes no inputs, `release-plz-release` has no event
guard, and by decision 2 the environment has no reviewer — so a dispatch
on `main` is equivalent to a push on `main`. It is currently harmless for
two reasons: release-plz is dormant until the App secrets exist, and it
is idempotent against crates.io, publishing only what is not yet
published. **Activation removes the first of those.** Recorded because
the runbook previously claimed a `dry_run` default protected this, and
no such default exists on this workflow.

`pull_request_target` and `workflow_run` are blocked by crates.io from
trusted publishing. Our `push: branches: [main]` + `workflow_dispatch`
triggers are compliant, and must stay that way.

## Alternatives considered

**Keep `crates-io-auth-action` and the hand-written ladder.** Rejected:
the ladder was a second, hand-maintained encoding of the dependency
order that `Cargo.toml` already states, and lockstep publishing across
17 crates makes it exactly the kind of thing that rots quietly. It also
still needed a human to push the tag.

**Run release-plz inside the existing `release` environment.** Rejected:
its required reviewer would block an unattended publish behind a second
approval for a merge that was already the approval, and the two
environments genuinely want different policies.

**Use `GITHUB_TOKEN` and trigger the downstream workflows some other
way** (`workflow_run`, or folding them into release-plz). Rejected:
crates.io blocks `workflow_run` from trusted publishing, and merging the
binary and wheel builds into the publish job would put them behind the
credential that publishes crates.

## References

- [ADR-0020](0020-release-secret-hardening-trusted-publishing.md) —
  superseded for crates.io only.
- [ADR-0021](0021-cli-release-stays-hand-written.md) — why `release.yml`
  stays hand-written while release-plz owns versioning.
- [ADR-0009](0009-version-single-source-of-truth.md) —
  `xtask publish check` keeps the ledger and `release-plz.toml` agreeing.
- [`docs/contrib/releasing-secrets.md`](../contrib/releasing-secrets.md)
  — the operational runbook, including activation.
- release-plz — <https://release-plz.dev/docs/github/quickstart>
- crates.io Trusted Publishing — <https://crates.io/docs/trusted-publishing>
