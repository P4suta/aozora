# 0034. Separate the toolchain channel from the MSRV

- Status: accepted
- Date: 2026-07-15
- Deciders: @P4suta
- Tags: build, ci, dx, release

## Context

`rust-toolchain.toml`'s `channel` and the root `Cargo.toml`'s
`rust-version` held the same value, and were bumped together by policy —
the Dockerfile and `.github/dependabot.yml` both said so in prose.

They are not the same fact.

- The **channel** is what *we* develop on. It wants to track latest
  stable: that is how the workspace gets new clippy lints and new
  language features.
- The **MSRV** is what *consumers* need. Thirteen library crates are on
  crates.io, so `rust-version` decides whether someone can build aozora
  at all.

Coupling them makes those two forces pull the same lever in opposite
directions, and the developer-facing one always wins because it is the
one someone is actively wishing for. The evidence:

- The workspace declared **1.96.0** against a current stable of 1.97 —
  **N-1**. hyper and tokio support "at least six months old"; clap uses
  N-2. Nothing in the ecosystem is this aggressive.
- `dd65755`, the commit that set it, gives its entire reason as
  `(current stable)`. `rust-toolchain.toml` has only ever been touched
  twice: created at the then-stable 1.95, and bumped to 1.96. **The MSRV
  has never once moved because a feature required it.**
- `bindings/rust.md` claimed the MSRV "advances roughly once per quarter,
  when a new stable feature is needed". The 1.95 → 1.96 bump landed
  **eight days** after the previous pin. The documented policy and the
  actual practice were unrelated.
- Measured 2026-07-15, the real floor is **1.89.0** — seven releases
  below what was declared, and set by `rustyline` (the `aozora repl` line
  editor), not by anything in our own code, which needs 1.88.

Seventeen places carried the version, with no gate over them.
`dd65755`'s message asserts "All version pins move together"; it left
three README badges behind, and `d40da3f` had to sweep them later.

## Decision

**Two authorities, deliberately holding different numbers.**

| authority | file | meaning | moves when |
|---|---|---|---|
| `T` — channel | `rust-toolchain.toml` | the dev toolchain | a new stable ships |
| `M` — MSRV | `Cargo.toml` `rust-version` | the public contract | we measure that it must |

1. **`M` is measured, not declared.** It is whatever the workspace
   actually compiles at, found by building — not by summing dependency
   manifests. Today `M = 1.89.0`.
2. **Policy: support any Rust from the last six months**, and raise `M`
   only when a new stable feature is genuinely needed. This is hyper's
   shape, and the reason its "MSRV bumps are not breaking" stance is
   credible: a predictable bump is one a consumer can plan around.
3. **Every other pin follows exactly one authority.** The Dockerfile
   base, the mise host lane, and the dev image follow `T`. `clippy.toml`'s
   `msrv` and CI's `msrv` job follow `M`.
4. **Pins become derivations wherever possible.** CI's msrv job and
   `just msrv-local` read `rust-version` out of `Cargo.toml`; mise drops
   its `rust` entry and reads `rust-toolchain.toml` natively; the README
   badges become `img.shields.io/crates/msrv/aozora`. The handbook names
   a Rust version in exactly one page.
5. **`xtask msrv check` enforces what is left**, inside `drift-gate`.

**`clippy.toml`'s `msrv` follows `M`, and this is load-bearing.** Its
whole job is `clippy::incompatible_msrv`, which flags std APIs newer than
the configured floor. Point it at `T` and it has nothing left to detect —
the lint goes quietly dead while still appearing configured. Following
`M` instead turns `just clippy-strict` into the *fast* MSRV gate: it
names the offending line before CI compiles anything.

**The six-month rule is checked as `T.minor - M.minor >= 5`.** Rust ships
every six weeks, so four releases is 24 weeks (~5.5 months — short of the
promise) and five is 30 weeks (~6.9 months). Five is the smallest gap that
keeps the policy true, and being pure arithmetic it needs no release
calendar in the repo.

**The MSRV check covers every publishable feature.** The old
`cargo check --workspace --all-targets` omitted `--all-features`, so
`aozora/schema`, `aozora-syntax/serde` and friends — all of which a
crates.io consumer can turn on — were never checked at the floor.
`aozora-extism` is excluded from that lane and checked on default
features instead: it is `publish = false`, and its dev-only `host-smoke`
feature pulls in wasmtime, which *declares* `rust-version = 1.91`. A
dev-only test feature does not get to set a public contract. (Measured:
wasmtime compiles fine at 1.89 — its declaration is merely conservative —
but cargo enforces declarations, so the exclusion is required rather than
merely tidy.)

**Dependabot may now bump the `rust` image.** The former `ignore` existed
to stop the base drifting "ahead of the MSRV". Post-separation the base
follows `T`, which is *supposed* to track stable, so the bump is a useful
signal that a new stable shipped. Dependabot cannot edit
`rust-toolchain.toml`, so `xtask msrv check` holds its PR red until a
maintainer syncs the channel in the same PR. **That red is the workflow,
not a defect** — it is what makes the un-ignore safe.

## Consequences

- **The contract stops moving for reasons consumers cannot see.** A
  clippy wish no longer costs users a rustc release.
- **`M` will now drift upward from dependencies, not from us.** 1.89 is
  set by `rustyline`. This is acceptable while it stays inside the
  six-month rule, and the gate says so out loud when it does not.
- **The floor must be re-measured, not reasoned about.** `rustyline`
  declares *no* `rust-version`, which makes it invisible to a manifest
  sweep and to Cargo's MSRV-aware resolver — and perfectly visible to the
  compiler. Any future claim about the floor has to come from a build.
  `contrib/msrv.md` carries the recipe.
- **Seventeen pins become six plus a gate.** The rest are derived or
  deleted. Nobody has to remember the list, which is the only reason the
  list was ever wrong.
- **A dependabot `rust` PR is now red by design** until its channel bump
  is completed by hand. Reviewers must know this; the Dockerfile comment
  and `contrib/release.md` say so.
- **The badge now reports the published crate, not `main`.** That is a
  behaviour change, and the right one: the badge answers "can I use this
  with my rustc", which is a question about the release. It also fixes a
  live inaccuracy — the static badge read `1.96` while the published
  0.4.1 declared `1.95.0`, so it was already wrong in the direction that
  matters.

## Alternatives considered

**Keep the coupling, fix only the prose.** Rewrite `bindings/rust.md` to
say "the MSRV tracks latest stable; pin a release tag if you need a
frozen one". Honest, and it is what the code already does — but it keeps
N-1, keeps excluding every consumer on a distro or a corporate pin for no
technical reason, and leaves seventeen ungated pins in place. It documents
the problem instead of solving it.

**Adopt clap's N-2 rule.** Mechanical, and trivially automatable: MSRV =
stable − 2, forever. Rejected because it is still a calendar, not a
measurement. It would declare 1.95 while the workspace demonstrably builds
at 1.89, excluding seven releases' worth of users to satisfy a formula, and
it re-creates the original sin in slower motion — the MSRV moving because
time passed rather than because a feature demanded it.

**Per-crate `rust-version`, so libraries can declare a lower floor than
the CLI.** Genuinely tempting while `rustyline` looked like it might drag
the library floor up a long way. Measurement killed it: our code needs
1.88 and rustyline needs 1.89 — one release, six weeks. Not worth a second
authority and a per-crate matrix in CI. Revisit if the gap ever widens
enough to matter to a real consumer.

**Make `rustyline` optional (feature-gate `aozora repl`).** Would drop
the floor to 1.88. Rejected as a six-week gain in exchange for a
default-off subcommand and a feature-matrix branch — the CLI's REPL is a
shipped feature, not an extra.

**Let `xtask msrv check` also verify the floor by compiling.** It would
have to install a second toolchain, which is precisely what keeps the CI
`msrv` job on a native runner instead of the dev image. `drift-gate` stays
offline and fast; compiling at the floor stays CI's job, mirrored locally
by `just msrv-local`.

## References

- Measurement (2026-07-15): #530. `cargo +1.88.0 check --locked
  --ignore-rust-version --workspace --all-targets --all-features
  --exclude aozora-extism` fails on `rustyline`'s `file_lock`
  (stabilised 1.89); 1.89.0 passes. `clippy::incompatible_msrv` at
  `msrv = "1.88.0"` reports zero hits across the workspace.
- Policy page: `crates/aozora-book/src/contrib/msrv.md` — the single
  place the handbook names a Rust version, plus the re-measurement recipe.
- [hyper's MSRV policy](https://hyper.rs/contrib/msrv/) — "always support
  a Rust version at least 6 months old"; "increase the MSRV responsibly,
  only when a significant new feature is needed".
- [RFC 3537](https://rust-lang.github.io/rfcs/3537-msrv-resolver.html) /
  [Cargo resolver](https://doc.rust-lang.org/cargo/reference/resolver.html)
  — resolver v3 treats `rust-version` as a *preference*, not a
  constraint, and a dependency that declares none is unconstrained.
- Related: ADR-0009 (release-tag literals live in one place — same
  instinct, different fact; `version-literal-gate` cannot cover MSRV
  because its pattern requires a `v` prefix and its scope excludes the
  READMEs), ADR-0031 (the mise host lane, whose `rust 1.96.0` reference
  predates this split and is left as the dated record it is), ADR-0007
  (the fast-pre-push guarantee `drift-gate` must not break).
