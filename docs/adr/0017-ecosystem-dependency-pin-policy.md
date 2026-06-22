# 0017. Ecosystem dependency-pin policy

- Status: accepted
- Date: 2026-06-23
- Deciders: @P4suta
- Tags: release, versioning, ecosystem, dependencies

## Context

The 2026-06 ecosystem alignment (#101) established how every downstream consumer
(afm, aozora-proof, afm-epub, the parked scaffolds) depends on the `aozora`
parser — but the convention was never written down, so a new consumer has
nothing to copy.

Three forces shape the rule:

1. **Pre-publish, drift had to be impossible.** Before `aozora` was on
   crates.io, consumers pinned a git reference. A *movable* reference — a branch,
   or a tag that can be re-cut — lets the parser silently change underneath a
   consumer that never touched its manifest. The alignment therefore converted
   every consumer from tags to the immutable commit `rev` `a53c632`.
2. **Publishable consumers cannot use a git source.** `cargo publish` rejects a
   crate that depends on a `git = …` source, so afm and aozora-proof — which
   publish to crates.io themselves — must depend on a crates.io **version**, not
   a rev. A crates.io release is itself immutable, so this preserves force (1).
3. **The umbrella is the contract.** `aozora` decomposes into many internal
   build-block crates (`aozora-syntax`, `aozora-pipeline`, …). Consumers that
   reached into those members would couple to an internal decomposition that is
   free to change; the `aozora` façade is the only supported surface
   (cf. aozora-tools#26, ADR-0016).

This pairs with [ADR-0009](./0009-version-single-source-of-truth.md) (one
canonical version pin inside this repo) — that ADR governs *our* internal
single-source-of-truth; this one governs how *external* repos pin us.

## Decision

1. **Pin by an immutable reference, never a movable one.** A downstream consumer
   depends on `aozora` by either:
   - a **crates.io version** (`aozora = { version = "X.Y.Z" }`) — the form
     **publishable** consumers must use (afm, aozora-proof), and the default once
     a release exists; or
   - an **immutable git commit `rev`** (`aozora = { git = "…", rev = "<sha>" }`) —
     for non-publishable consumers, and for any consumer that must track an
     unreleased commit.

   Never a git **tag** or **branch**: a tag can be re-cut and a branch moves.
2. **Internal dev-only crates are path-only.** Workspace-internal,
   `publish = false` crates (test-support, proptest helpers, fuzz harnesses) are
   referenced by `path` with **no** version requirement, so `cargo publish`
   strips them from the published manifest.
3. **External consumers depend on the `aozora` umbrella façade**, never its
   internal member crates.

A version bump is therefore always a deliberate, reviewed edit to the consumer's
manifest (`cargo update -p aozora` + a pin bump), coordinated through the release
issue (e.g. #99) — never an implicit follow of a moved tag.

## Consequences

- **Easier:** a consumer's parser version cannot drift without a manifest change
  that shows up in review; new consumers have one documented convention to copy;
  internal decomposition stays free to change behind the façade.
- **Harder:** each intentional sync is manual (one pin bump per consumer,
  coordinated across the ecosystem) rather than automatic — accepted, because the
  alternative is silent, uncoordinated breakage (exactly what the v0.4.1 → v0.5.0
  AngleQuote freeze, #99, exists to avoid).

## Alternatives considered

- **Movable git tag (`tag = "vX.Y.Z"`).** The original state. Rejected: a tag can
  be force-re-cut, so the "pin" is not actually immutable — the drift this ADR
  exists to forbid.
- **Git branch (`branch = "main"`).** Rejected: tracks HEAD, so every consumer
  rebuilds against whatever landed last, with no coordination point.
- **No written policy.** Rejected for the same reason ADR-0009 rejected it for
  version strings: an unwritten convention proliferates into inconsistent,
  undiscoverable practice across repos.

## References

- Issues: #100 (this ADR), #101 (2026-06 ecosystem alignment), #99 (v0.5.0
  release + coordinated downstream migration).
- Related ADRs: [0009](./0009-version-single-source-of-truth.md) (internal
  version single-source-of-truth), [0016](./0016-consolidate-tooling-into-the-aozora-monorepo.md)
  (umbrella-façade consolidation).
- Alignment PRs: P4suta/afm#88, P4suta/aozora-proof#16.
