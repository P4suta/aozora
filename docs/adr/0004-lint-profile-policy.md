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

## Addendum (2026-06-15): inner-loop check + clippy placement

- Status: accepted
- Date: 2026-06-15

The single-source-of-truth principle above is unchanged: **which** lints
fire is still owned solely by the root `Cargo.toml [workspace.lints]`. What
changed is *where* and *how heavily* the gates run, to stop the developer
paying full `--all-targets` cost on every commit. Two refinements:

**1. `just check` now exists — the sub-second compile gate.** It runs
`cargo check --workspace --exclude aozora-bench --all-targets` as the
fastest "still compiles?" inner-loop signal, mirroring `bacon`'s default
job and the MSRV CI job. This **reverses** the former "there is no `just
check`" guidance (CLAUDE.md updated to match; note `afm` already has its
own `check`). `just build` is retained as the heavier `--all-targets` gate
that *also links binaries*; `check` is the type-check-only fast path you
keep in the loop, `build` is the "does it actually link" confirmation.

**2. Clippy placement rebalanced.** The per-commit `lefthook` hook now runs
the **light** `just clippy` (`--lib --bins --tests`, `aozora-bench`
excluded); CI's lint matrix runs the authoritative **`just clippy-strict`**
(`--all-targets --all-features`, including bench/example targets) as the
backstop. Previously the heavy `clippy-strict` ran on *every* commit.
Rationale: commit latency is paid constantly, by the developer; the
`--all-targets` surface mostly catches bench/example-only regressions and
is paid once, in parallel, by CI — never blocking a commit. `just
lint-full` runs the thorough surface locally before a release for those who
want the backstop without waiting on CI.

The lint *set* is identical across all three (`check` / light `clippy` /
`clippy-strict`); only the compiled target surface differs. `just
strict-code` (the `#[allow(reason = …)]` / `continue-on-error` gate above)
is untouched and still part of `just ci` + pre-push.

This mirrors the breadth-vs-cost reasoning in
[ADR-0006](0006-polyglot-bindings-via-extism.md) (pay the broad,
rarely-regressing surface once in CI, keep the hot path cheap).
