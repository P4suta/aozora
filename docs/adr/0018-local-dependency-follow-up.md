# 0018. Local-only dependency follow-up — pure Rust + lefthook + systemd user timer

- Status: accepted
- Date: 2026-04-27
- Deciders: @P4suta
- Tags: tooling, dependencies, ci, architecture
- References: ADR-0002 (Docker-only execution), ADR-0009 (clean
  layered architecture)

## Context

The workspace pins many third-party crates (rayon, bumpalo, smallvec,
miette, addr2line, regex, …). Each minor / patch release of any one
of them can fix a security advisory or a perf bug we benefit from;
each major bump is an opt-in API decision. The standard solution is
**dependabot** or **renovate** — a remote service that opens PRs
with bumps, gated by the repo's CI.

We do not want that. The operational reasons:

- **No remote CI**. The repo's contract is `just ci` — every gate
  runs locally inside the dev container (ADR-0002 docker-only).
  Adding a separate remote-CI surface duplicates state, adds an
  authentication boundary (which token can dispatch which job?),
  and creates a drift class: the gates a remote bot enforces on a
  PR may differ from the gates a developer can re-run locally.
- **No remote secrets / no remote write access**. The dependency-bot
  pattern needs commit / PR / branch-write tokens. Any such token
  is a permanent attack surface; revocation cadence is a separate
  thing to remember.
- **Quiet branches still need fresh advisory checks**. A bot tied
  to PRs only fires when someone opens a PR. A timer that fires
  regardless of repo activity catches new RustSec advisories on a
  branch nobody is touching this week.
- **The dev container already owns the toolchain**. `cargo
  outdated` / `cargo audit` / `cargo deny` / `cargo upgrade` all
  live in the cargo-tools layer of the Dockerfile. Re-using that
  layer from a host-side timer is cheaper than spinning up a remote
  GitHub-Actions runner.

## Decision

A four-layer, pure-Rust, host-side mechanism:

| Layer | Component | Cadence |
|---|---|---|
| **Tooling** | `cargo-outdated` added to the Dockerfile cargo-tools layer alongside the existing `cargo-edit` / `cargo-audit` / `cargo-deny` / `cargo-udeps` / `cargo-semver-checks`. | rebuilt whenever the Dockerfile changes |
| **Recipes** | `just outdated` / `just upgrade` / `just upgrade-incompat` / `just deps-check` / `just deps-status`. All run inside the dev container via the shared `_dev` prefix; no host-side cargo is invoked. | on demand |
| **Hooks** | `lefthook.yml` → `pre-push: audit` (paired with the existing `deny`); `post-merge` / `post-checkout` → `just deps-status` (silent unless stale). | every push / pull / checkout |
| **Schedule** | Pure-Rust `xtask deps install-timer` (sources in `crates/aozora-xtask/src/deps.rs`) writes a `.service` + `.timer` pair into `$XDG_CONFIG_HOME/systemd/user/` that runs `just deps-check` weekly. Bound to the repo checkout that ran the install. | weekly (Sunday 03:30 ± 30 min jitter, `Persistent=true`) |

The `xtask` subcommand (rather than a shell script) follows the
precedent set by `xtask samply` and `xtask trace` — see ADR-0009 §
"Why pure Rust" and the `feedback_prefer_rust_over_shell.md` user
preference. There is no shell script in `scripts/` for the timer;
the unit-file rendering, `systemctl --user` invocation, and
WorkingDirectory discovery are all in Rust.

## Mechanism details

### `just deps-check`

```bash
$ just deps-check
[deps-check] 2026-04-27T06:32:11Z — outdated, audit, deny
docker compose run --rm dev cargo outdated --workspace --root-deps-only --depth 2 --exit-code 0
docker compose run --rm dev cargo audit
docker compose run --rm dev cargo deny check
[deps-check] OK — marker written to target/.deps-check.timestamp
```

Three sub-checks:

- `outdated` — what to bump. Exit always zero (this is information,
  not a gate); the developer reads the table and decides.
- `audit` — RustSec CVE database. Exit non-zero on a known-vulnerable
  transitive dep — this becomes a gate.
- `deny` — license / ban-list / source-allowlist policy from
  `deny.toml`. Same — gate, not info.

The marker file `target/.deps-check.timestamp` is written on success
so `deps-status` can summarise freshness without re-running the
slow checks. `target/` is volume-mounted from the dev container, so
host-side `just deps-status` (which runs **outside** the container)
can read it without a Docker round-trip.

### `just deps-status`

Reads `target/.deps-check.timestamp`, computes age, exits non-zero
if > 7 days. The lefthook `post-merge` hook calls this on every
`git pull` so the developer is reminded of stale gates without
having to ask. Output:

```
[deps-status] last check 2 days ago (2026-04-25T03:31:14Z) — fresh
```

vs. when stale (or never run):

```
[deps-status] last check 9 days ago (2026-04-18T03:31:14Z) — STALE; run 'just deps-check'
```

### `just upgrade` / `just upgrade-incompat`

Two recipes, distinguished by whether they cross semver-major boundaries:

- `just upgrade` — `cargo upgrade --workspace --pinned --recursive`
  (compat bumps only). Safe to run anytime.
- `just upgrade-incompat` — `cargo upgrade --workspace --incompatible
  allow --recursive`. Major bumps included. Must be reviewed —
  major bumps are API breaks by definition.

Both call `cargo update --workspace` afterwards so the lockfile
reflects the new ceiling. The recipe ends with a hint reminding
the developer to `just ci` before committing.

### `xtask deps install-timer`

`crates/aozora-xtask/src/deps.rs`:

- Walks up from CWD to find the workspace root (the `Cargo.toml`
  containing `[workspace]`). Hard-fails with a clear error if no
  workspace is found above the current directory.
- Writes two files:
  - `$XDG_CONFIG_HOME/systemd/user/aozora-deps-check.service` —
    oneshot unit, `WorkingDirectory={repo}`, `ExecStart=/bin/bash -c
    'just deps-check 2>&1 | tee -a "{log}"'`. `Nice=10` +
    `IOSchedulingClass=idle` so the run never steals foreground
    cycles.
  - `$XDG_CONFIG_HOME/systemd/user/aozora-deps-check.timer` —
    `OnCalendar=Sun *-*-* 03:30:00`, `Persistent=true`,
    `RandomizedDelaySec=30m`.
- `systemctl --user daemon-reload && systemctl --user enable --now
  aozora-deps-check.timer`.
- Idempotent: re-running overwrites the unit files and re-enables.

The `Persistent=true` is load-bearing for laptop workflows: a unit
that misses its window (machine asleep at 03:30 Sunday) fires on
next boot. `RandomizedDelaySec=30m` spreads multiple machines on
the same LAN so they don't hammer crates.io in lock-step.

The unit is **bound to the checkout that ran the install** via
`WorkingDirectory=`. Re-installing from a different checkout
overwrites the previous unit (same name) — this is a feature, not a
bug: developers should pick *one* canonical checkout for the timer
and keep it there.

## Why on the host (not in the dev container)

`systemctl --user` talks to the host's systemd user instance. There
is no per-user systemd inside the dev container; even if there were,
cron-style scheduling inside an ephemeral `docker compose run --rm`
container makes no sense. Same precedent as `xtask samply` —
tooling that touches the host kernel / init system runs on the
host, not in the container.

## Validation gates

- `cargo build -p aozora-xtask --release` — Rust build still passes
  with the new module.
- `xtask deps install-timer` round-trip on the developer machine
  (manual; documented as the canonical test).
- `xtask deps status` after install reports the next run.
- `xtask deps uninstall-timer` cleans both unit files; the rolling
  log file is intentionally preserved.
- `cargo clippy --workspace --all-targets --all-features -- -D
  warnings` — clean.

## Out of scope

- **Auto-PR creation**. `just upgrade` writes the bumps locally;
  the developer reviews `git diff` and commits. No bot opens
  unattended PRs. (We can revisit if multiple developers join and
  the manual cadence becomes a bottleneck.)
- **Cross-machine state sync**. Each machine has its own timer.
  The state file (`target/.deps-check.timestamp`) is per-checkout.
  This is fine: stale on one machine doesn't imply stale on
  another, and the global truth lives in the lockfile committed to
  the repo.
- **Toolchain bumps**. `RUST_VERSION` is pinned in the Dockerfile.
  Bumping it is a manual decision that goes through the normal
  edit-Dockerfile flow; no automation.
- **Rebuild of the dev container itself on a schedule**. Tempting
  (it would let the cargo-tools layer pick up a new `cargo-deny`
  release automatically), but `docker compose build` is heavy and
  the marginal benefit is small — bump the image when something
  actually breaks.

## Trade-offs

- **Cost**: zero remote infrastructure, ~1 minute of CPU on the
  developer's machine once a week.
- **Risk**: a developer who never runs `just deps-timer-install`
  doesn't get the schedule. Mitigation: the lefthook
  `post-merge` / `post-checkout` hook surfaces stale state on
  every pull / checkout, so even an un-scheduled developer is
  prompted to run `just deps-check` manually.
- **Discoverability**: nothing fires PRs at the developer. They
  have to read the journal / log themselves. We accept this — the
  `lefthook` hook + `just deps-status` provide enough discovery
  for a small team.
