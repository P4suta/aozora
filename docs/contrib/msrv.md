# MSRV policy

aozora always supports a Rust release **at least six months old**.

The MSRV is raised only when a new stable feature is actually needed —
never merely because a new stable shipped. It is a *measured floor*, not
a mirror of whatever we happen to develop on.

This page is the only place in the handbook that names a Rust version.
Everywhere else links here, and `xtask msrv check` enforces that.

## Two versions, two jobs

|                   | file                                | what it is                              | moves when                       |
| ----------------- | ----------------------------------- | --------------------------------------- | -------------------------------- |
| toolchain channel | `rust-toolchain.toml`               | what **we** develop on — latest stable  | a new stable ships               |
| MSRV              | `Cargo.toml` → `rust-version`       | what **you** need — the public contract | we measure that it has to        |

These are deliberately **different numbers**, and that is the whole
point. The dev image, the mise host lane, and the `FROM rust:` base all
follow the *channel*, so they are free to track stable — that is how we
get new clippy lints. None of it touches the contract.

Coupling the two, as this workspace used to, quietly converts "we want a
newer clippy" into "we drop every user on an older rustc". The bump that
motivated the split raised the MSRV by a whole release with `(current
stable)` as its entire stated reason.

## Is an MSRV bump breaking?

No — and under this policy it is also *predictable*, which is what makes
that answer honest. Any toolchain from the last six months keeps working,
so "not breaking" is a guarantee you can plan around rather than a label
we assign to a change you could not see coming.

## What sets the floor today

`rustyline` — the line editor behind `aozora repl` — uses `file_lock`,
stabilised in Rust 1.89. Our own code needs 1.88 (let-chains), and no std
API newer than that appears anywhere in the workspace.

That gap is one release, so the workspace keeps a single `rust-version`
rather than declaring a lower floor per library crate. If it ever widens
enough to matter to library consumers, that trade is worth revisiting.

## Measuring it

Do not infer the floor from dependency manifests. `rustyline` declares no
`rust-version` at all, which makes it invisible both to a manifest sweep
and to Cargo's MSRV-aware resolver — and perfectly visible to the
compiler. The floor is whatever actually compiles:

```sh
# Does the declared MSRV still build? (reads the version from Cargo.toml)
just msrv-local
```

To re-measure — after dropping a dependency, or when curious whether the
floor has fallen — walk candidates downward until one fails:

```sh
rustup toolchain install 1.88.0 --profile minimal
cargo +1.88.0 check --locked --ignore-rust-version \
      --workspace --all-targets --all-features --exclude aozora-extism
cargo +1.88.0 check --locked --ignore-rust-version -p aozora-extism --all-targets
```

`--ignore-rust-version` is required: without it Cargo enforces the
declared `rust-version` before the compiler ever gets a say, so you would
only be testing the declaration.

`aozora-extism` is checked on default features only. It is
`publish = false`, and its dev-only `host-smoke` feature pulls in
wasmtime, which *declares* a higher `rust-version` than it actually
needs. A dev-only test feature does not get to set a public contract.

For the std-API side, ask clippy instead of reading code — without
touching the checked-in config:

```sh
docker compose run --rm dev bash -c '
  mkdir -p /tmp/msrv-probe
  sed "s/^msrv = .*/msrv = \"1.88.0\"/" clippy.toml > /tmp/msrv-probe/clippy.toml
  CLIPPY_CONF_DIR=/tmp/msrv-probe cargo clippy \
    --workspace --all-targets --all-features --exclude aozora-extism
'
```

`clippy::incompatible_msrv` names every std API newer than the configured
floor, with line numbers. **Confirm the lint is alive before believing a
clean run** — set `msrv` absurdly low (`1.60.0`) and check that warnings
appear. A cached run that never re-lints looks exactly like a clean one.

## Raising it

Only when a feature you need is stable and nothing older can express it.
Then:

1. Set `rust-version` in the root `Cargo.toml`.
2. Set `msrv` in `clippy.toml` to match.
3. Run `just msrv-check` — it fails unless every pin agrees and the new
   floor is still at least six months behind the channel.

The CI `msrv` job reads the version out of `Cargo.toml`, so there is
nothing to update there. Neither is `rust-toolchain.toml` involved: the
channel is not the contract.

## Enforcement

- **`just clippy-strict`** — the fast gate. `clippy::incompatible_msrv`
  names the offending line before anything is compiled at the MSRV.
- **CI `msrv` job** — compiles the workspace at the declared version,
  across every feature a crates.io consumer can turn on.
- **`just msrv-check`** (in `drift-gate`) — pins follow the right
  authority, and the six-month rule holds.

## See also

- [ADR-0034](../adr/0034-separate-toolchain-channel-from-msrv.md)
  — why the two numbers are separate, and the arithmetic behind the
  six-month rule.
- [Release process](release.md) — where the MSRV sits in the release
  contract.
