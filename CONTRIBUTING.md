# Contributing to aozora

Thanks for wanting to help. The rules are few and strict — the guarantees
only hold when every contribution respects them.

Run `./bootstrap` on a fresh clone.
It installs the locked native toolchain, installs the git hooks, and runs the tests.
After that, `just --list` is the menu, and `just ci` is the pre-push gate on Linux.
Other hosts run `just ci-repo` before pushing.
Coverage, corpus, mutation, fuzzing, and the other heavy checks run at the release gate, as [the gate split](docs/adr/fast-pull-request-gate-and-release-gate.md) records.

## Ground rules

1. **Locked native execution.** Install tools only through `mise install
   --locked` and run repository operations through `just`. OS libraries such
   as Clang/libclang, Valgrind, and Playwright dependencies come from the host
   package manager.
2. **`unsafe_code = "forbid"` workspace-wide.** A crate that genuinely
   needs to relax it declares a `reason = "…"` carve-out and keeps
   `unsafe_op_in_unsafe_fn` denied, so every block still justifies
   itself. Compiler lints hold the line.
3. **Fix the code a lint names.**
   Use a narrow item-level `#[expect(lint, reason = "…")]` only where a macro, an ABI, or generated code forces an exception.
   New outer `#[allow]` attributes are rejected; do not add inner crate-wide or file-wide suppression.
4. **A failing test lands first, then the fix.**
   `release-ready` enforces region coverage, and the property, corpus, and mutation layers judge whether the tests assert anything.
5. **Prefer a loud panic to a silent fallback** on the classify and render
   paths. A wrong-but-quiet default ships wrong bytes on a green build.
   Correctness beats liveness here.

## Commits

Signed and [Conventional](https://www.conventionalcommits.org/), both
enforced by hooks. Do not reach for `--no-verify` or `LEFTHOOK=0`.

Scope is a crate name minus the `aozora-` prefix — `feat(render):`,
`perf(scan):` — and is optional for cross-cutting changes. Nothing
enforces this one, so it is on you.

If a gate blocks you, re-run the named `just ci-*` suite or individual recipe.

## Where things live

- `docs/adr/` — why a decision was made. Read the one governing an area
  before changing what it governs.
- `docs/contrib/` — the MSRV and release runbooks.
- Everything else — `just --list`, `aozora --help`, and the code.

Report security issues privately per [SECURITY.md](./SECURITY.md); do not
open a public issue.

Contributions are dual-licensed Apache-2.0 OR MIT, matching the project.
