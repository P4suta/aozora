# 0007. Parallel, fast pre-push pipeline

- Status: accepted
- Date: 2026-06-15
- Deciders: @P4suta
- Tags: infra, ci, dx

## Context

The pre-push gate (`lefthook` → `just ci` → `just prop-deep`) took
**9–13 minutes**. A gate that slow is a gate people route around:
`--no-verify` becomes a habit, and the "always verify before push"
culture this repo depends on quietly erodes.

Measurement showed the wall-time was **not** the gate logic. Warm, the
whole `cargo nextest` suite runs in ~1.3 s. The cost was overhead:

- **(a) Redundant recompiles.** `build`, `test`, and `coverage` each
  drove their own compile of largely the same code — three full builds
  where one would do (and `coverage`'s instrumented build is a fourth
  flavour).
- **(b) Per-gate container starts.** Every `just` target is a separate
  `docker compose run …`; a long serial chain of them pays the
  container-start tax once per gate.
- **(c) Strict sequencing.** Gates that share nothing ran one after
  another, summing their wall-times instead of overlapping.

The dominant *real* constraint is narrower than "CPU": cargo holds a
**per-`CARGO_TARGET_DIR` build lock**. Two cargo invocations against the
same `/cargo/target` cannot compile concurrently — they serialise on that
lock regardless of how many cores are free. Any parallelism has to be
drawn around that fact, not around core count.

## Decision

A new `just ci-parallel` recipe (a bash recipe) runs **every** gate
before the push — the always-verify culture is kept intact — but fast.
The design follows directly from the build-lock constraint:

1. **Collapse build+test into two compiles, not four.** `check`
   (`--all-targets`, "does it compile?") plus `coverage` (the
   instrumented test build + run + region floor) replace the
   `build`/`test`/`coverage` trio. Test execution rides the coverage
   build; there is no separate non-instrumented test compile.

2. **Background every gate that does *not* take the build lock.** Gates
   with no claim on `/cargo/target` run in a background lane so their
   wall-time hides behind the foreground cargo chain: `deny`, `audit`,
   `book-linkcheck`, `smoke-ffi` (host-side target dir), the
   playground gates (`playground-typecheck`, `playground-test` via bun),
   and the non-compiling lint gates (`fmt-check`, `typos`,
   `strict-code`).

3. **Keep the build-lock gates serial and ordered cheap→expensive.** The
   foreground cargo chain — `clippy` → `check` → `drift-gate` →
   `conformance` → `coverage` → `prop` → `udeps` → `extism-build` →
   `doc` → `corpus-sweep` — stays sequential and fail-fast. There is no
   benefit to parallelising it: every step contends on the same build
   lock anyway, so ordering them cheapest-first maximises how early a
   break aborts.

4. **Launch the 4096-case `prop-deep` sweep in the background *after*
   the foreground `prop` gate.** By then the `property_*` test binaries
   are already built, so the deep sweep reuses them — no build-lock
   contention — and its long CPU tail overlaps `udeps` / `extism-build` /
   `doc`. `SKIP_TAGS=deep` opts out.

`lefthook` pre-push now runs `signing-check` → `ci-parallel`, with
`prop-deep` folded into `ci-parallel` (it is no longer a separate
trailing step).

**CI remains the authoritative backstop.** The full matrix still runs on
every PR; `ci-parallel` is a fast *local mirror* of it, not a
replacement. That is what makes the lane split and the collapse safe: if
a corner is shaved locally, the PR matrix still catches it.

## Consequences

- **Pre-push drops from 9–13 min toward low single-digit minutes** on a
  warm cache. The foreground cargo chain is now the critical path;
  background lanes and the deep prop sweep finish underneath it.
- **One new bash recipe to maintain.** `ci-parallel` encodes the
  foreground/background split and the lock-aware ordering by hand; it
  must be kept in step with the gate set (a new gate has to be placed in
  the correct lane).
- **Failure semantics: foreground aborts fast, background self-cleans.**
  A foreground break stops the chain immediately; in-flight background
  containers are `--rm`, so they tear down on their own rather than
  needing explicit cleanup.
- **Local and CI gate lists can drift.** Because `ci-parallel` is a
  hand-rolled mirror, a gate could be added to CI but forgotten locally
  (or vice versa). The PR matrix is the safety net that makes this
  non-fatal, but the recipe is now a thing to remember to update.

## Alternatives considered

**Just make the gate faster (cache harder, prune work).** The bottleneck
was never the work — warm, the suite is ~1.3 s — it was redundant
compiles, container starts, and sequencing. Tuning the gates' internals
would not have moved a number dominated by overhead.

**Parallelise the whole pipeline naively (run every gate at once).**
Tempting, but the build lock makes it a lie: all the compiling gates
would queue on `/cargo/target` and serialise anyway, while N concurrent
`docker compose run` starts add contention. The win comes specifically
from backgrounding only the *non-locking* gates; the rest must stay
serial.

**Drop pre-push to a thin smoke check and lean on CI.** Fastest locally,
but it abandons the "verify before push" guarantee and pushes every
real failure to the slow CI loop. Rejected: the goal was to make the
*full* local gate cheap enough that nobody wants to skip it, not to
shrink what it covers.

**Change-aware gating now** (skip gates whose inputs the push range did
not touch). The biggest theoretical win, deferred — see below.

## Future work

**Change-aware gating.** A further speedup is to skip gates whose inputs
the push range did not touch — e.g. don't run `playground-*` when no
`playground/` file changed, don't run `conformance` when no
fixture/renderer changed. A design exists: a deliberately conservative
changed-paths → gates map, with an `AOZORA_CI_FULL=1` override to force
the whole pipeline. It is **not yet implemented**. The conservatism (and
the CI backstop, which always runs the full matrix) make it safe to add
incrementally later, so it is deferred rather than built now.

## References

- Recipes: `just ci-parallel`, `just ci`, `just prop` / `just prop-deep`,
  `just coverage`, `just corpus-sweep`; `lefthook.yml` pre-push
  (`signing-check` → `ci-parallel`).
- Build-lock constraint: a single `CARGO_TARGET_DIR` (`/cargo/target`)
  serialises concurrent cargo compiles — the reason the lane split is
  drawn at "takes the build lock?".
- Related: ADR-0004 (lint/profile policy — `just strict-code` and the
  rustdoc-deny `doc` gate run inside this pipeline); ADR-0005
  (corpus-sweep — the opt-in `corpus-sweep` gate, last in the foreground
  chain); ADR-0006 (polyglot bindings — `extism-build` is a foreground
  build-lock gate here).
