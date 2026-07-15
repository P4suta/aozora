# Contributing to aozora

Thanks for wanting to help. The rules are few and strict — the guarantees
only hold when every contribution respects them.

Run `./bootstrap` on a fresh clone. It builds the dev image, installs the
git hooks, and runs the tests. After that, `just --list` is the menu and
`just ci-parallel` is the full gate — the same one that runs on push.

## Ground rules

1. **Docker-only execution.** Do not invoke `cargo` on the host. Every
   step goes through `just`, which shells into the dev container so the
   toolchain and caches stay identical across machines. (The sanitizer
   script is the one documented exception; it needs host nightly.)
2. **`unsafe_code = "forbid"` workspace-wide.** A crate that genuinely
   needs to relax it declares a `reason = "…"` carve-out and keeps
   `unsafe_op_in_unsafe_fn` denied, so every block still justifies
   itself. `just strict-code` holds the line.
3. **No silent warning suppressions.** A bare `allow(...)` is rejected.
   Fix the cause, or carve it out with a `reason`.
4. **A failing test lands first, then the fix.** `just coverage` gates on
   region coverage; the property, corpus, and mutation layers are what
   actually judge whether the tests assert anything.
5. **Prefer a loud panic to a silent fallback** on the classify and render
   paths. A wrong-but-quiet default ships wrong bytes on a green build.
   Correctness beats liveness here.

## Commits

Signed and [Conventional](https://www.conventionalcommits.org/), both
enforced by hooks. Do not reach for `--no-verify` or `LEFTHOOK=0`.

Scope is a crate name minus the `aozora-` prefix — `feat(render):`,
`perf(scan):` — and is optional for cross-cutting changes. Nothing
enforces this one, so it is on you.

If a gate blocks you, read what it printed: lefthook names the failing
command and `ci-parallel` prints `::error title=<gate>::`. They know more
than a troubleshooting page could.

## Where things live

- `docs/adr/` — why a decision was made. Read the one governing an area
  before changing what it governs.
- `docs/contrib/` — the MSRV and release runbooks.
- Everything else — `just --list`, `aozora --help`, and the code.

Report security issues privately per [SECURITY.md](./SECURITY.md); do not
open a public issue.

Contributions are dual-licensed Apache-2.0 OR MIT, matching the project.
