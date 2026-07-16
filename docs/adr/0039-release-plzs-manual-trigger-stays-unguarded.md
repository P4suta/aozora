# 0039. release-plz's manual trigger stays unguarded

- Status: accepted
- Date: 2026-07-16
- Deciders: @P4suta
- Tags: release, security, ci, supply-chain

## Context

[ADR-0038](0038-release-plz-owns-versioning-and-crates-io-publishing.md)
recorded that a `workflow_dispatch` of `release-plz.yml` on `main` is
equivalent to a push on `main`, and that this is "currently harmless for
two reasons: release-plz is dormant until the App secrets exist, and it is
idempotent against crates.io. **Activation removes the first of those.**"

The first reason was already false when it was written. Measured:

- The `release-plz` environment has held both `RELEASE_PLZ_APP_CLIENT_ID`
  and `RELEASE_PLZ_APP_PRIVATE_KEY` since **2026-06-29** — sixteen days
  before ADR-0038 was accepted.
- `release-plz.yml` has run **196 times**; the last five are green.
- `chore: release v0.5.0` (#313) has been open, authored by
  `app/p4suta-release-plz` and labelled `autorelease: pending`, since
  2026-06-29.

release-plz was operational the entire time. This is not a stale sentence
in a consequence: it was half of the safety argument for a button that
publishes, and the half it deferred to a future event — "activation" — had
already happened. The same claim stood in `release-plz.yml`'s own header,
in `release.md`, and in two workflow steps gated on `HAS_APP != 'true'`
that no run could reach.

So the argument has to be made again, against what is actually there.

## Decision

**The button stays: no inputs, no event guard, no reviewer.** The reason is
not the one ADR-0038 gave.

What bounds a stray **Run workflow** click is `release-plz.toml`'s
`release_always = false`: release-plz releases only from a commit
associated with a Release PR, which it decides by checking whether the
branch behind the latest commit starts with `release-plz-`. A dispatch on
an ordinary `main` commit therefore publishes nothing and tags nothing. A
dispatch on the Release-PR merge commit re-runs a publish that the merge
already authorised, and that run is idempotent against crates.io.

This is why ADR-0038's "a dispatch on `main` is equivalent to a push on
`main`" is exactly right and was never the problem: a push on `main` does
not publish either. Both are bounded by the same config line.

Neither leg of the original argument needed activation to hold, and the
surviving one does not depend on the credential state at all.

## Consequences

**The safety property is readable from the repository.** `release_always =
false` is a line in `release-plz.toml`, under review like any other. The
state ADR-0038 leaned on instead — which secrets exist on which
environment — is not in the tree, is read by no gate, and changed without
anyone noticing until #556 went and looked. A decision resting on
server-side state is one nobody can check.

**Deleting `release_always = false` re-arms every trigger at once**, not
just the button: a push on `main` would publish whatever version `main`
carries, ahead of the Release PR. Nothing enforces the line. It carries a
comment saying what it does, and this ADR is the record of what it holds
up.

**The dispatch is the only way to finish a release that fails after the
merge**, now that hand-pushing a tag is gone (ADR-0038). Guarding the
button would leave a half-published release with no route forward.

## Alternatives considered

**Guard `release-plz-release` on `github.event_name != 'workflow_dispatch'`,
the way `release.yml` guards its publish job.** Rejected: it would remove
the recovery path to protect against a click that `release_always = false`
already makes a no-op. `release.yml` needs its guard because a dispatch
there really would publish; here the equivalence to a push is the safety,
not the hazard.

**Add a `dry_run: true` input default, like the PyPI / npm publishers.**
Rejected: it protects only the one commit where a dispatch does anything —
the Release-PR merge commit — and on that commit the publish is idempotent
and is the outcome the merge asked for. The cost is real: recovery becomes
two steps with a footgun default, and both jobs must thread an input that
means nothing to either.

**Put a required reviewer on the `release-plz` environment.** Rejected,
unchanged from ADR-0038 decision 2: it would gate every unattended release
in order to gate a click, stalling the publish that the labelled
squash-merge already approved.

## References

- [ADR-0038](0038-release-plz-owns-versioning-and-crates-io-publishing.md)
  — supersedes its `workflow_dispatch` consequence, and nothing else. Its
  decisions all stand.
- release-plz `release_always` —
  <https://release-plz.dev/docs/config#the-release_always-field>
- [`docs/contrib/releasing-secrets.md`](../contrib/releasing-secrets.md) —
  the runbook's account of what stops a stray dispatch.
- #556 — the drift campaign that measured the environment.
