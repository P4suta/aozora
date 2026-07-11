# aozora-veb

A cache-friendly sorted-set lookup (Eytzinger layout, `no_std`) used on
the [aozora][repo] lexer's hot path, where a branch-predictable,
cache-line-dense binary search beats a `HashSet` probe.

**Internal implementation crate.** It carries no independent stability
contract — the API can change in any release. It exists to serve the
[aozora][repo] pipeline; application code should depend on the umbrella
[`aozora`][crate] crate instead.

- 📦 [crates.io/crates/aozora][crate]
- 📖 [API reference (docs.rs)][docs]
- 📚 [Handbook][book] — notation reference, architecture, bindings

Part of the [aozora][repo] workspace. Dual-licensed Apache-2.0 OR MIT.

[crate]: https://crates.io/crates/aozora
[docs]: https://docs.rs/aozora
[book]: https://p4suta.github.io/aozora/
[repo]: https://github.com/P4suta/aozora
