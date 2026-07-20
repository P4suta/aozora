# ADR-0042: release-ready is the publish authority

- Status: Accepted
- Date: 2026-07-19
- Supersedes: ADR-0039

## Context

The ordinary CI result did not prove that a commit was publishable. Deep
fuzzing, complete mutation sweeps, native macOS and Windows tests, external
specification and corpus checks, and installation from distribution artifacts
were scheduled or documented obligations. Corpus gates could also succeed
without a corpus, and several correctness gates compared against accepted
residue rather than requiring zero defects.

The tag-driven publishers rebuilt their inputs independently. A green source
test therefore did not identify one immutable artifact set, and a retry could
publish bytes different from the first attempt.

## Decision

`release-ready` is the sole automated publish authority.

The workflow runs in full for a release-plz Release PR, for its version-changing
commit on `main`, and when explicitly dispatched. It fixes the external
specification and corpus to repository and commit identifiers, verifies both
checkouts are non-empty, and treats every decode, read, panic, internal
diagnostic, unknown fallback, render leak, structural render defect, mutation
survivor, mutation timeout, fuzz finding, sanitizer finding, native-platform
failure, packaging failure, or artifact smoke failure as a failed release.
There is no update flag, tolerance, report-only result, or missing-input success
path in this workflow.

Release artifacts carry the source commit, workspace version, wire schema
version, checksums, licenses, and generated interface files. The workflow
builds and exercises the Rust packages, CLI, C library, npm package, Python
wheel and sdist, Extism module, Go SDK, VSIX, and playground production build
before it reports success.

The `release-ready` job name is the branch-protection status for Release PRs.
The `release: approved` label records the human decision but cannot replace
that status. After the Release PR is merged, release-plz waits for
`release-ready` to succeed on the exact version-changing commit before it may
publish or create the tag. A manual release-plz dispatch has the same proof
requirement.

Local `just` recipes may omit expensive external-data work when no checkout is
configured. That convenience never participates in release authorization.

## Consequences

The release document contains no quality checklist. Humans review the version,
changelog, and automated result, then approve the Release PR. Quality work that
cannot fail `release-ready` is not a release requirement until it is automated.

Full release proof is intentionally expensive. It runs once for the Release PR
and once for the exact merge commit whose bytes publishers use. Scheduled
mutation, fuzz, and cross-platform workflows remain useful early-warning
signals but are no longer substitutes for a release result.

The repository must configure branch protection to require the
`release-ready` job on Release PRs and must retain the protected `release`
environment for the final registry approval.

## References

- [ADR-0031](0031-mutation-testing-for-assertion-strength.md)
- [ADR-0038](0038-release-plz-owns-versioning-and-crates-io-publishing.md)
- [ADR-0041](0041-public-facade-and-editable-documents.md)
- [Release process](../contrib/release.md)
