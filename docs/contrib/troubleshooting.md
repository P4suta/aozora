# Troubleshooting & gate recovery

Most first-run friction is environmental, not code. This chapter
collects the failures people actually hit on a fresh checkout and the
shortest path back to a green tree. If you haven't set up the
environment yet, start with [Development loop → First-time
setup](dev.md#first-time-setup); if a commit is being rejected for
signing, see [Your first PR](first-pr.md).

## First-run failures

### Docker daemon not running

Every `just` target shells into the dev container via `docker compose
run …`, so a stopped daemon makes *all* of them fail at once — usually
with `Cannot connect to the Docker daemon at unix:///var/run/docker.sock`.

Start (or restart) Docker Desktop / the `docker` service and re-run the
target. Nothing in aozora runs on the host toolchain; the daemon is a
hard prerequisite for `build`, `test`, `lint`, and `ci` alike.

### Disk full / image build fails midway

`docker compose build dev` pulls a `rust:*-bookworm` base and layers a
pinned toolchain on top. A build that dies partway through — or a
`no space left on device` from `just build` — almost always means the
Docker volume is out of room, not a Dockerfile bug.

```sh
docker system prune          # reclaim dangling images / layers / build cache
```

Keep roughly **5 GB free** for the dev image plus the named cargo /
sccache volumes. After pruning, re-run `docker compose build dev`; the
layer cache resumes from the last good step.

### Commit signing fails

Signed commits are mandatory. If a commit is rejected — or the
post-commit re-amend rolls your commit back because the signer was
unavailable — your SSH/GPG signing key isn't reachable from the
container's git context. Walk through the signing setup in
[CONTRIBUTING.md → First-time setup](https://github.com/P4suta/aozora/blob/main/CONTRIBUTING.md#first-time-setup)
and confirm `git config commit.gpgsign` is `true` with a configured
`user.signingkey`.

This is the three-layer defense working as designed: a `post-commit`
re-amend, the `signing-check` pre-push command
(`scripts/check-signed-commits.sh`), and GitHub's "require signed
commits" ruleset. **Do not weaken any layer** — the redundancy is
intentional. Fix the key, don't disable the gate.

### Hooks not firing

If `fmt` / `clippy` / `typos` aren't running on commit, or the signing
re-amend never happens, lefthook isn't installed for this clone. Hooks
live in `.git/hooks/`, which is per-clone and never committed, so a
fresh checkout always needs:

```sh
just hooks                   # (re)install the lefthook git hooks
```

Re-run it any time hooks go quiet (e.g. after `git init`-level surgery
or switching the hooks path).

### Reading lefthook output

Lefthook prints one icon per command in its post-run summary. The
non-obvious one:

- **🥊** is a **failure**, not decoration. Lefthook falls back to its
  branding glyph when the underlying tooling (a `docker compose run`,
  or a multi-step `just` recipe with background jobs) buries the real
  exit status. Treat 🥊 exactly like a plain failure mark and **scroll
  up**: the actual error line is in the command output *above* the
  summary, not in the summary itself.

Each command in `lefthook.yml` also carries a `fail_text:` hint
naming the recipe responsible, so a failing push prints both the raw
output and a pointer at what to fix.

## When a gate fails

`just ci` runs the full pipeline; the pre-push hook runs the same jobs
plus a deep property sweep. When one trips, this table maps the symptom
to its recovery recipe:

| Gate | Symptom | Recovery |
|---|---|---|
| coverage | Region coverage below the floor | `just coverage-html`, open `coverage/html/index.html`, add tests for the uncovered regions |
| clippy / fmt | `cargo fmt --check` diff or clippy denial | `just fmt` to auto-format, fix any clippy findings, then re-run `just lint` |
| drift-gate (schema) | JSON Schema is stale | `just schema` to regenerate, then commit the diff |
| drift-gate (types) | TypeScript `.d.ts` drift | `just types` to regenerate, then commit the diff |
| drift-gate (langs) | Generated host-SDK JSON types are stale | `just types-langs` to regenerate, then commit the diff |
| typos | Spelling hit | `just typos` to see every hit; fix, or add a genuine term to `typos.toml` |
| strict-code | A forbidden source pattern (see below) | `just strict-code` prints each hit as `==> forbidden: <label>` with its `file:line`; fix per the list below |
| deny / audit | License / advisory failure | Read the captured log under `/tmp` (the recipe writes the full `cargo deny` / `cargo audit` output there), then update the dependency or the `deny.toml` exception |

For the schema / types gates, the regenerate-then-commit step is the
fix — the gate only checks that the committed artefact matches what the
generator would emit, so a stale checkout fails until you regenerate
and stage it.

### strict-code violations

`just strict-code` (part of `just lint`) greps the tree for patterns the
workspace forbids, printing each as `==> forbidden: <label>` with the
offending `file:line`. The common ones and their fixes:

| Forbidden | Fix |
|---|---|
| `#[allow(...)]` without `reason = "..."` | Add a `reason`, or drop the allow and fix the lint. |
| `unsafe` outside the exempt crates (`aozora-ffi` / `-scan` / `-xtask` / fuzz) | Refactor to safe code; the exempt crates carry a crate-level `#![allow(unsafe_code, reason = …)]`. |
| A crate root missing `#![forbid(unsafe_code)]` | Add it to that crate's `lib.rs` / `main.rs`. |
| Bare `TODO` / `FIXME` / `XXX` without an issue reference | Append `#123` (or an `M<n>` milestone). |
| `println!` / `eprintln!` in a library crate | Use `tracing`; stdout/stderr printing belongs to the CLI. |
| A `comrak` import or path | Remove it — aozora is the pure-notation layer, with no markdown dependency. |
| Stale slug `koshogaki` (小書き = こがき) | Use `kogaki`. |
| Stale section-break slugs `choho` / `dan` / `spread` | Use `kaicho` / `kaidan` / `kaimihiraki`. |

The grep is the cheap last line of defence behind the type system; when
it fires, the fix is in the named file, not in the recipe.

### samply: `perf_event_open` permission denied

Host-side profiling (`just samply-*`, `just trace-*`) reads kernel
performance counters, which need `perf_event_paranoid` at `1` or lower:

```sh
cat /proc/sys/kernel/perf_event_paranoid          # want <= 1
echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid
```

`just doctor` reports this — along with Docker, the dev image, host
tools, hooks, and signing — in one read-only pass. Run it first when a
fresh environment misbehaves; it never mutates anything.

## Escape hatches

Two exist, and they are **not** interchangeable:

- **`SKIP_TAGS=deep git push`** — the *narrow* hatch. Skips only the
  tagged `deep` command (the 4096-case `prop-deep` sweep) while leaving
  `signing-check`, `ci`, and everything else in force. Use this when a
  deep-sweep regression is unrelated to your change — and file an issue
  against the failing crate so it doesn't stay hidden.
- **`LEFTHOOK=0`** — the *nuclear* hatch. Disables **all** hooks,
  including `signing-check`. An unsigned commit pushed this way is
  rejected server-side by the ruleset anyway, so you gain nothing but a
  later, more confusing failure. **Avoid it.** Reach for `SKIP_TAGS`
  instead.

## See also

- [Development loop](dev.md) — the daily `just` recipes and watch mode.
- [Your first PR](first-pr.md) — the lightweight doc-fix path.
- [Testing strategy](testing.md) — what each coverage layer asserts.
