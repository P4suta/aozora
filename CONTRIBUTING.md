# Contributing to aozora

Follow the repository's verification and contribution requirements.

Run `./bootstrap` on a fresh clone.
It installs the locked native toolchain, installs the git hooks, and runs the tests.
Use `just --list` to find commands and `just ci` for the pre-push gate on Linux.
Other hosts run `just ci-repo` before pushing.
Coverage, corpus, mutation, fuzzing, and the other heavy checks run at the release gate, as [the gate split](docs/adr/fast-pull-request-gate-and-release-gate.md) records.

## Ground rules

1. **Locked native execution.**
   Install tools through `mise install --locked` and run repository operations through `just`.
   The host package manager supplies system libraries such as Clang/libclang, Valgrind, and Playwright dependencies.
2. **Workspace unsafe-code policy.**
   The workspace sets `unsafe_code = "forbid"`.
   A crate that needs an exception declares `reason = "…"` and keeps `unsafe_op_in_unsafe_fn` denied.
   Keep the required justification for each unsafe block.
3. **Fix the code a lint names.**
   Use a narrow item-level `#[expect(lint, reason = "…")]` only where a macro, a binary interface, or generated code forces an exception.
   Don't add outer `#[allow]` attributes or inner crate-wide or file-wide suppression.
4. **A failing test lands first, then the fix.**
   `release-ready` enforces region coverage, and the property, corpus, and mutation layers judge whether the tests assert anything.
5. **Reject silent fallbacks on classification and rendering paths.**
   A fallback that returns incorrect bytes can pass tests while violating the output contract.
   Prefer an explicit panic when the alternative would hide incorrect output.

## Commits

Hooks require signed commits and [Conventional Commits](https://www.conventionalcommits.org/).
Don't use `--no-verify` or `LEFTHOOK=0`.

Use the crate name without the `aozora-` prefix as the optional scope, as in `feat(render):` or `perf(scan):`.
Cross-cutting changes can omit the scope.

If a gate blocks you, re-run the named `just ci-*` suite or individual recipe.

## Where things live

- `docs/adr/` records design decisions.
  Read the governing record before changing its subject.
- `docs/contrib/` contains the Rust version support policy and release runbooks.
- `just --list`, `aozora --help`, and the code describe commands and behavior.

Report security issues privately through [SECURITY.md](./SECURITY.md).
Don't open a public issue for a vulnerability.

Contributions use the project's `Apache-2.0 OR MIT` license expression.
