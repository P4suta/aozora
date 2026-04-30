# Rust library

The first-class binding. Full type safety, zero copy, and the
borrowed-arena AST exposed directly.

## Adding to a project

```toml
[dependencies]
aozora          = { git = "https://github.com/P4suta/aozora.git", tag = "v0.2.6" }
aozora-encoding = { git = "https://github.com/P4suta/aozora.git", tag = "v0.2.6" }
```

crates.io publication tracks the v1.0 API freeze; until then, use a
git tag.

## Surface

The public surface is small by design — three types and four
methods cover everything:

```rust
pub struct Document { /* opaque */ }
impl Document {
    pub fn new(source: String) -> Self;
    pub fn parse(&self) -> AozoraTree<'_>;
    pub fn source(&self) -> &str;
}

pub struct AozoraTree<'a> { /* borrows from Document */ }
impl<'a> AozoraTree<'a> {
    pub fn nodes(&self) -> impl Iterator<Item = AozoraNode<'a>>;
    pub fn to_html(&self) -> String;
    pub fn serialize(&self) -> String;
    pub fn diagnostics(&self) -> &[Diagnostic];
}

pub enum AozoraNode<'src> { Plain(&'src str), Ruby(Ruby<'src>), … }
```

See [Library Quickstart](../getting-started/library.md) for the
walk-through.

## Feature flags

aozora exposes one optional feature:

| Feature | Default | What it enables |
|---|---|---|
| `serde` | off | `serde::Serialize` / `Deserialize` impls on `AozoraNode`, `Diagnostic`, `Span`. Useful for downstream tools that need to ship the AST over a wire. |

The default-off policy keeps `cargo build aozora` slim — the JSON
encoders that the bindings need live in the bindings themselves
(`aozora-ffi`, `aozora-wasm`, `aozora-py`), not in the core crate.

## Error handling

Three philosophies, used consistently:

1. **Diagnostics are not errors.** `Document::parse()` always returns a
   `AozoraTree<'_>`. Per-input diagnostics live in `tree.diagnostics()`.
   Callers decide whether to treat any diagnostic as fatal.
2. **Decoding is fallible.** `aozora_encoding::sjis::decode_to_string`
   returns `Result<Cow<str>, DecodeError>`. Malformed Shift_JIS is the
   one place a function actually fails — the parser proper assumes
   UTF-8.
3. **Panics are bugs.** No `.unwrap()` on user-data paths in
   non-test code; clippy's `unwrap_used` and `expect_used` are warned
   workspace-wide. If you ever see a panic in `aozora::*`, file a
   bug.

## Thread safety

`Document` is `Send` but not `Sync` — the bumpalo arena does not
support concurrent allocation. Pass a `Document` between threads
freely; do not share `&Document` across threads.

`AozoraTree<'_>` borrows from `&Document`, so by Rust's lifetime
rules the same shape applies: a `&AozoraTree` is `Send + Sync` (it's
just `&` to immutable data), but it can't outlive its `Document`.

For *parallel* corpus processing (e.g. the corpus sweep harness
parsing 1000s of documents concurrently), each thread creates its
own `Document` from its own source. The arena resets per-`Document`,
so there's no contention point.

## MSRV policy

aozora pins **Rust 1.95.0**. The MSRV advances roughly once per
quarter, when a new stable feature is needed and the workspace
moves to it. The `msrv` job in CI gates every PR; Dependabot is
configured to *not* auto-bump the MSRV pin (manual decision).

## Public API stability

Pre-1.0: minor-version bumps may break the API. `cargo-semver-checks`
runs in CI to catch unintentional breakage between releases, so a
`v0.2.x` → `v0.2.y` upgrade is always safe; only `v0.x.y` →
`v0.x+1.y` opens the door for breaks.

Post-1.0 (planned): semver discipline. Breaking changes accumulate
on a `next` branch and ship in a major bump.

## See also

- [Library Quickstart](../getting-started/library.md)
- [Borrowed-arena AST](../arch/arena.md) — the lifetime model.
- [Reference → API](../ref/api.md) — generated rustdoc.
