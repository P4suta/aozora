# 0004. Lint profile policy

- Status: accepted
- Date: 2026-05-31
- Deciders: @P4suta
- Tags: infra, policy

> Shared policy between this repo and the sibling
> [`afm`](https://github.com/P4suta/afm) repo. afm keeps a redirect stub
> (its ADR-0006); the canonical statement is here. Both repos enforce the
> same `[workspace.lints]` and `just strict-code` gates against the same
> rationale.

## Context

A workspace this size drifts toward inconsistent lint posture: one crate
allows what another denies, warnings accumulate unread, and
`#[allow(...)]` sprinkles hide real problems. Two sibling repos sharing a
parser make the drift worse if each sets its own policy.

## Decision

A single source of truth in the root `Cargo.toml [workspace.lints]`,
inherited by every crate (`[lints] workspace = true`):

- `rust` + `clippy` at **pedantic + nursery**, with documented, narrowly
  scoped exceptions only.
- **All `rustdoc` lints = `deny`** (broken intra-doc links, bare URLs,
  invalid codeblocks, …) so documentation rot fails the build.
- **No warning suppression without a reason.** `#[allow(lint, reason =
  "…")]` is required; a bare `#[allow]` is rejected by `just strict-code`.
  `continue-on-error` / blanket `--cap-lints` are likewise banned in CI.

`just strict-code` is the enforcement gate (also part of `just ci` and
the pre-push hook). Genuine carve-outs (e.g. `large_enum_variant` for the
boxed `SpanKind::Aozora`) are allowed *with* an inline rationale.

## Consequences

- Lint posture is uniform and reviewable in one file.
- A new contributor cannot silently weaken a lint.
- Doc links can't rot unnoticed.
- Cost: pedantic + nursery occasionally flag style the author disagrees
  with; the policy says fix-or-document, never silently allow.

## Alternatives considered

- **Per-crate lint config.** Rejected: drift, and no cross-repo parity.
- **Warnings-as-warnings (not deny).** Rejected: unread warnings are
  indistinguishable from no warnings.

## References

- Root `Cargo.toml [workspace.lints]`; `just strict-code`.
- Owner convention: resolve warnings at the root, never `allow` to dodge.
