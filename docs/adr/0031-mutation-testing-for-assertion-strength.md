# 0031. Mutation testing as an assertion-strength gate

- Status: accepted
- Date: 2026-07-12
- Deciders: @P4suta
- Tags: infra, ci, testing, dx

## Context

The CI coverage gate is **region coverage** (ADR-0007, `contrib/testing.md`):
it proves a line of code *ran* during the suite. It cannot prove that a
*wrong* result on that line would make some test *fail*. That gap is not
academic. The incremental re-parse span-divergence bug sat behind ~84%
region coverage — the divergent code was executed by the tests — yet no
assertion anywhere caught the wrong span. Coverage measured reach;
nothing measured catch.

`contrib/testing.md` already concedes this ("coverage is the floor, not
the ceiling"), and the property/corpus/fuzz layers each strengthen
assertions from a different angle. What was missing is a *direct*
measurement of assertion strength: given a deliberate defect, does the
suite go red?

## Decision

Adopt **cargo-mutants** as the assertion-strength tool. It is the Rust
ecosystem standard for mutation testing: nextest-native and supports
`--in-diff` for cost-bounded per-change runs. It is built from source in
the dev image — its only prebuilt releases are gnu-linked against a newer
glibc than the bookworm base provides, and no musl build exists, so a
source build against the image's own glibc is the compatible path. It
mutates each function (swap return
values, negate conditions, delete statements), rebuilds, and re-runs the
tests; a mutant that survives is a place where a real defect could ship
green.

Introduce it in **stages**, not as a day-1 blocking gate:

1. **Plumbing (this ADR).** `cargo-mutants` in the dev image, a
   root-level `mutants.toml` (nextest runner; 5x/30s hang timeout for
   mutations that unbound a parser loop; bench and fuzz excluded — no
   generated code is checked in, and test code is skipped by the tool),
   and a `just mutants [-p CRATE]` recipe that runs in a dedicated,
   incremental target dir.
2. **Measure and reinforce (report-only).** Sweep the high-ROI crates
   first (aozora-syntax, then aozora-spec), triage surviving mutants —
   write tests to kill the real gaps, `#[mutants::skip]` the
   equivalent/unreachable ones with a reason — and record a clean
   baseline. CI does **not** fail during this phase.
3. **Ratchet.** PR-scoped `cargo mutants --in-diff` (advisory, then
   blocking once a crate's baseline is clean) plus a **scheduled** weekly
   full sweep. Full mutation runs are inherently slow, so the full sweep
   is scheduled rather than per-push; the local mirror is `just mutants`.

## Consequences

- **A measurable, actionable signal for test *efficacy*, not just
  reach.** Surviving mutants point at exactly the assertions worth
  adding — the class of gap region coverage is blind to.
- **New skill to keep honest.** `#[mutants::skip]` must carry a reason;
  an un-reasoned skip is indistinguishable from hiding a real gap. The
  triage discipline (kill vs. justified-skip) is the whole value.
- **Cost is bounded by construction.** Full sweeps are scheduled and
  scoped per-crate; per-PR runs use `--in-diff`. Neither sits on the hot
  pre-push path, so the fast-pre-push guarantee (ADR-0007) is untouched.
- **Runs off the sccache lane.** Mutation rebuilds serially and reuses a
  dedicated incremental target dir; it deliberately does not share the
  main sccache'd `/cargo/target`, so the two build strategies never
  clobber each other.

## Addendum (2026-07-12): host-native lane

The staged rollout above reinforces one crate at a time, and each crate's
sweep is the slow part — cargo-mutants rebuilds once per mutant. To make
that inner loop fast enough to drive full adoption (every product crate,
not just syntax + spec), a **host-native lane** runs the same sweep outside
the container:

- `just mutants-host [-p CRATE]` runs cargo-mutants directly on the host,
  driving its `-j` parallelism. The dev image pins `CARGO_TARGET_DIR` to a
  volume shared across containers, so the container lane cannot safely run
  concurrent sweeps; a host target dir (`target/mutants-host`) can, and
  cargo-mutants' own `-j` fans a single crate's mutants across cores.
  cargo-mutants, nextest, and rust 1.96.0 all come from the existing mise
  host inner-loop manifest (`.config/mise/config.toml`).
- **Parity is the invariant that keeps this sound.** Mutant enumeration is
  a deterministic function of (source AST × cargo-mutants version) and the
  outcome is a function of the suite, so a baseline computed on the host
  holds for the CI Docker sweep **iff both run the identical cargo-mutants
  version**. cargo-mutants is therefore pinned to an exact version in *both*
  the mise manifest and the Dockerfile, and the two must move together
  (each carries a comment pointing at the other). The acceptance test is
  that `just mutants-host -p aozora-scan` reports the same survivor set as
  the container `just mutants -p aozora-scan`.

The container lane stays authoritative: `mutants.yml` and the
`mutants-in-diff` CI job run `just mutants` in the dev image (locale- and
glibc-pinned); the host lane is a developer/campaign accelerator that must
agree with it. This is scoped to the mutation loop — a broader
"develop without Docker" migration is out of scope here; the mise inner
loop already covers nextest/deny/llvm-cov/shear and this extends it.

## Alternatives considered

**Push line/branch-coverage targets higher.** Raising the coverage floor
would still only measure reach. The span bug was already *covered*;
demanding 100% branch coverage would not have surfaced it. Coverage and
mutation testing answer different questions; more of the former is not
the latter.

**`mutagen` (the older compile-time mutation crate).** Effectively
unmaintained, nightly-only, and it rewrites the AST at compile time
rather than driving cargo, which fits neither the pinned stable
toolchain nor the nextest pipeline. cargo-mutants is the maintained,
tool-driven successor.

**Make it a blocking pre-push/PR gate on day one.** A first mutation
sweep of an untuned suite surfaces many survivors, most needing triage
(equivalent mutants, integration-only coverage). Blocking immediately
would either wall off all work or pressure un-reasoned skips. Report →
reinforce → ratchet earns the gate instead of imposing it.

## References

- cargo-mutants book: <https://mutants.rs/> — config keys
  (<https://mutants.rs/config-file.html>), `--in-diff`
  (<https://mutants.rs/in-diff.html>), timeouts, skip attribute.
- Recipe / config: `just mutants`, `mutants.toml`.
- Plan: "Mutation Testing 導入プラン" (motivated by the incremental
  span-divergence bug that passed region coverage).
- Related: ADR-0005 (corpus-sweep — another always-catches layer),
  ADR-0007 (parallel pre-push — the region-coverage gate this
  complements and the build-lock constraint the recipe honours);
  `contrib/testing.md` "Coverage measurement".
