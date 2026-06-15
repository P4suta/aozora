# Your first PR

Not every contribution touches the parser. A typo fix, a clarified
sentence, a broken link — these are real, welcome PRs, and they ride a
much lighter path than the add-a-notation TDD flow. This chapter is
that lightweight path. For parser changes (a new 青空文庫 notation, a
lexer phase, a renderer shape), follow the full TDD flow in
[Development loop → Adding a new 青空文庫 notation](dev.md#adding-a-new-青空文庫-notation)
instead.

## Before your first commit: environment setup

If this is a brand-new checkout, get the environment standing first:

```sh
just setup            # one-shot first-time environment bootstrap
# or, equivalently:
./bootstrap
```

That builds the dev image and installs the lefthook git hooks. If
hooks ever stop firing later, re-run `just hooks` (see
[Troubleshooting → Hooks not firing](troubleshooting.md#hooks-not-firing)).

## The lightweight path (a doc / typo fix)

1. **Pick a small fix.** A typo, a stale link, an unclear sentence in
   the handbook (`crates/aozora-book/src/…`) or a top-level doc. Keep
   it to a single logical change.

2. **Branch.** `main` is branch-protected — never commit to it
   directly.

   ```sh
   git switch -c docs/fix-ruby-example-typo
   ```

3. **Edit the `.md`.** Use only your editor here; no parser code is
   involved, so there's nothing to compile.

4. **Verify locally.** Two quick gates cover doc-only changes:

   ```sh
   just typos            # spelling across the tree
   just book-build       # the mdbook handbook still builds
   ```

   `just book-build` catches broken intra-book links and bad Markdown
   that a typo check won't. Both run inside the dev container, matching
   what CI runs.

5. **Commit with a signed, Conventional Commit.** Doc changes take the
   `docs:` type:

   ```sh
   git commit -m "docs: fix ruby example in the notation chapter"
   ```

   Both requirements are enforced by hooks: the `commit-msg` hook
   rejects a non-Conventional subject, and the signing layers reject an
   unsigned commit. Scope is optional for cross-cutting doc edits; use
   one when the change is crate-local (e.g. `docs(render): …`). See
   [Conventional commits](dev.md#conventional-commits) for the accepted
   types.

6. **Push and open a PR.** The PR title mirrors the commit subject
   (`docs: …`). The [PR template](https://github.com/P4suta/aozora/blob/main/.github/PULL_REQUEST_TEMPLATE.md)
   walks the checklist — keep it. CI re-runs the same gates you ran
   locally.

If a commit is rejected for signing, or a hook misbehaves, jump to
[Troubleshooting & gate recovery](troubleshooting.md).

## The inner loop (while you iterate)

For anything beyond a one-line fix, run a watcher in a second terminal
so feedback is continuous instead of per-commit:

```sh
just watch            # default check job — recompiles on save
just watch-lint       # fmt + clippy on save
just watch-test       # nextest on save
```

The watcher runs inside the dev container, so it detects saves against
the bind-mounted source. See [Development loop → Watch mode](dev.md#watch-mode-bacon)
for the in-watcher keybindings.

## How this differs from a parser change

A doc fix is intentionally cheap. A parser change is not: it lands a
failing test first, then the fix, and extends every test layer the new
shape touches. The contrast is deliberate.

| | Doc / typo fix | New notation / parser change |
|---|---|---|
| Touches | A `.md` file | Spec fixture → AST → lexer → renderer → invariants |
| Verify | `just typos`, `just book-build` | `just test`, `just prop`, `just coverage` |
| Commit type | `docs:` | `feat:` / `fix:` / `perf:` … |
| TDD | Not applicable | Red test first, then green — required |

Both paths share the same two hard rules: **signed commits** and
**Conventional Commits**. Everything else scales with the size of the
change.

## See also

- [Development loop](dev.md) — the daily loop and the full notation
  TDD flow.
- [Troubleshooting & gate recovery](troubleshooting.md) — when a hook
  or gate blocks you.
- [Testing strategy](testing.md) — what the parser-change path is
  verifying.
